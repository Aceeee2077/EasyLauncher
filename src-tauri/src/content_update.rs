use crate::{
    error::{AppError, Result},
    models::ServerType,
    server::manager::Manager,
    storage,
};
use futures_util::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const PINS_FILE: &str = "content-pins.json";
const ROLLBACK_FILE: &str = "content-rollback.json";
const BACKUP_DIR: &str = "content-backups";
const MAX_PROJECTS: usize = 128;
const CONCURRENCY: usize = 4;
const CHECK_DEADLINE: Duration = Duration::from_secs(45);
const MAX_CHANGELOG_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReport {
    /// One row per managed project, most actionable first.
    pub entries: Vec<UpdateEntry>,
    /// False when at least one project could not be checked.
    pub complete: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEntry {
    pub project_id: String,
    pub title: String,
    pub file: String,
    pub kind: String,
    pub current_version_id: String,
    pub current_version: String,
    pub latest_version_id: Option<String>,
    pub latest_version: Option<String>,
    /// `update`, `upToDate`, `pinned`, `unavailable` or `error`.
    pub status: String,
    pub channel: Option<String>,
    pub changelog: Option<String>,
    /// Stable warning codes translated by the frontend.
    pub warnings: Vec<String>,
    pub dependencies: Vec<DependencyImpact>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyImpact {
    pub project_id: String,
    pub title: Option<String>,
    pub dependency_type: String,
    /// `added`, `changed`, `removed`, `missing` or `incompatible`.
    pub status: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    pub project_id: String,
    pub title: String,
    pub from_version: String,
    pub to_version: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackOutcome {
    pub project_id: String,
    pub title: String,
    pub version: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Pins {
    #[serde(default)]
    projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RollbackRecord {
    pub(crate) project_id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) saved_file: String,
    pub(crate) previous_file: String,
    pub(crate) previous_version_id: String,
    pub(crate) previous_version_number: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RollbackStore {
    #[serde(default)]
    records: Vec<RollbackRecord>,
}

pub(crate) fn pins(root: &Path) -> Vec<String> {
    let Ok(path) = storage::child(root, PINS_FILE) else {
        return Vec::new();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Pins>(&bytes).ok())
        .map(|pins| pins.projects)
        .unwrap_or_default()
}

fn save_pins(root: &Path, projects: &[String]) -> Result<()> {
    let mut projects = projects.to_vec();
    projects.sort();
    projects.dedup();
    storage::write_json(&storage::child(root, PINS_FILE)?, &Pins { projects })
}

pub(crate) fn rollbacks(root: &Path) -> Vec<RollbackRecord> {
    let Ok(path) = storage::child(root, ROLLBACK_FILE) else {
        return Vec::new();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RollbackStore>(&bytes).ok())
        .map(|store| {
            let Ok(backup_dir) = storage::child(root, BACKUP_DIR) else {
                return Vec::new();
            };
            store
                .records
                .into_iter()
                .filter(|record| {
                    crate::modrinth::safe_component(&record.saved_file)
                        && crate::modrinth::safe_component(&record.previous_file)
                        && crate::modrinth::validate_project_id(&record.project_id).is_ok()
                        && backup_dir.join(&record.saved_file).is_file()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn rollback_store(root: &Path) -> RollbackStore {
    let Ok(path) = storage::child(root, ROLLBACK_FILE) else {
        return RollbackStore::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_rollback_store(root: &Path, store: &RollbackStore) -> Result<()> {
    storage::write_json(&storage::child(root, ROLLBACK_FILE)?, store)
}

fn forget_rollback(root: &Path, project_id: &str) {
    let mut store = rollback_store(root);
    let before = store.records.len();
    store
        .records
        .retain(|record| record.project_id != project_id);
    if store.records.len() != before {
        let _ = save_rollback_store(root, &store);
    }
}

pub fn set_pinned(m: &Manager, id: &str, project_id: &str, pinned: bool) -> Result<()> {
    crate::modrinth::validate_project_id(project_id)?;
    let root = m.dir(id)?;
    let index = crate::modrinth::load_index(&root);
    if !index
        .entries
        .iter()
        .any(|entry| entry.project_id == project_id)
    {
        return Err(
            AppError::user("Only content installed by EasyLauncher can be pinned.")
                .with_code("contentUpdate.notManaged"),
        );
    }
    let mut projects = pins(&root);
    if pinned {
        if !projects.iter().any(|project| project == project_id) {
            projects.push(project_id.to_string());
        }
    } else {
        projects.retain(|project| project != project_id);
    }
    projects.sort();
    projects.dedup();
    save_pins(&root, &projects)
}

/// Newest release, falling back to the newest version of any channel. Modrinth
/// returns versions in publication order, but compare the timestamp explicitly
/// so an older beta cannot win over a newer release.
pub(crate) fn choose_latest(
    versions: &[crate::modrinth::ProjectVersion],
) -> Option<&crate::modrinth::ProjectVersion> {
    versions
        .iter()
        .filter(|version| version.channel.eq_ignore_ascii_case("release"))
        .max_by(|a, b| a.date_published.cmp(&b.date_published))
        .or_else(|| {
            versions
                .iter()
                .max_by(|a, b| a.date_published.cmp(&b.date_published))
        })
}

fn relevant(dependency: &crate::modrinth::ProjectDependency) -> bool {
    matches!(
        dependency.dependency_type.as_str(),
        "required" | "incompatible"
    )
}

/// Compare the dependency declarations of the installed and candidate versions.
/// Only changes and problems are returned; unchanged declarations stay quiet.
pub(crate) fn dependency_impact(
    current: &[crate::modrinth::ProjectDependency],
    target: &[crate::modrinth::ProjectDependency],
    installed: &BTreeMap<String, String>,
) -> (Vec<DependencyImpact>, Vec<String>) {
    let current_map: BTreeMap<&str, (&str, Option<&str>)> = current
        .iter()
        .filter(|dependency| relevant(dependency))
        .filter_map(|dependency| {
            dependency.project_id.as_deref().map(|id| {
                (
                    id,
                    (
                        dependency.dependency_type.as_str(),
                        dependency.version_id.as_deref(),
                    ),
                )
            })
        })
        .collect();
    let target_ids: BTreeSet<&str> = target
        .iter()
        .filter(|dependency| relevant(dependency))
        .filter_map(|dependency| dependency.project_id.as_deref())
        .collect();
    let mut warnings = BTreeSet::new();
    let mut impact = Vec::new();
    for dependency in target.iter().filter(|dependency| relevant(dependency)) {
        let Some(project_id) = dependency.project_id.as_deref() else {
            continue;
        };
        let installed_version = installed.get(project_id);
        let mut status = match current_map.get(project_id) {
            None => "added",
            Some((previous_type, previous_version)) => {
                let changed = *previous_type != dependency.dependency_type.as_str()
                    || dependency.version_id.as_deref().map_or(false, |required| {
                        Some(required) != *previous_version
                            && installed_version.map(String::as_str) != Some(required)
                    });
                if !changed {
                    continue;
                }
                "changed"
            }
        };
        if dependency.dependency_type == "required" && installed_version.is_none() {
            warnings.insert("missingDependency".to_string());
            status = "missing";
        } else if dependency.dependency_type == "incompatible" && installed_version.is_some() {
            warnings.insert("incompatibleDependency".to_string());
            status = "incompatible";
        }
        impact.push(DependencyImpact {
            project_id: project_id.to_string(),
            title: None,
            dependency_type: dependency.dependency_type.clone(),
            status: status.to_string(),
            installed: installed_version.is_some(),
        });
    }
    for (project_id, (dependency_type, _)) in &current_map {
        if *dependency_type != "incompatible" && !target_ids.contains(project_id) {
            impact.push(DependencyImpact {
                project_id: (*project_id).to_string(),
                title: None,
                dependency_type: (*dependency_type).to_string(),
                status: "removed".into(),
                installed: installed.contains_key(*project_id),
            });
        }
    }
    impact.sort_by(|a, b| {
        a.project_id
            .to_lowercase()
            .cmp(&b.project_id.to_lowercase())
    });
    (impact, warnings.into_iter().collect())
}

fn truncate_changelog(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= MAX_CHANGELOG_CHARS {
        return Some(text.to_string());
    }
    let mut shortened: String = text.chars().take(MAX_CHANGELOG_CHARS).collect();
    shortened.push('…');
    Some(shortened)
}

enum Job {
    Done(UpdateEntry, Option<String>),
    Failed(String, String),
}

async fn check_one(
    client: reqwest::Client,
    entry: crate::modrinth::ContentEntry,
    minecraft: String,
    loader: &'static str,
    pinned: BTreeSet<String>,
    installed: BTreeMap<String, String>,
) -> Job {
    let mut base = UpdateEntry {
        project_id: entry.project_id.clone(),
        title: entry.title.clone(),
        file: entry.file.clone(),
        kind: entry.kind.clone(),
        current_version_id: entry.version_id.clone(),
        current_version: entry.version_number.clone(),
        latest_version_id: None,
        latest_version: None,
        status: "upToDate".into(),
        channel: None,
        changelog: None,
        warnings: Vec::new(),
        dependencies: Vec::new(),
    };
    if pinned.contains(&entry.project_id) {
        base.status = "pinned".into();
        return Job::Done(base, None);
    }
    let versions =
        match crate::modrinth::versions(&client, &entry.project_id, Some(&minecraft), None).await {
            Ok(versions) => versions,
            Err(error) => return Job::Failed(entry.title.clone(), error.message),
        };
    let compatible: Vec<_> = versions
        .into_iter()
        .filter(|version| crate::modrinth::compatible(version, loader))
        .collect();
    let Some(latest) = choose_latest(&compatible).cloned() else {
        base.status = "unavailable".into();
        base.warnings.push("noCompatible".into());
        return Job::Done(base, None);
    };
    if latest.id == entry.version_id {
        return Job::Done(base, None);
    }
    base.status = "update".into();
    base.latest_version_id = Some(latest.id.clone());
    base.latest_version = Some(latest.version_number.clone());
    base.channel = Some(latest.channel.clone());
    if !latest.channel.eq_ignore_ascii_case("release") {
        base.warnings.push("prerelease".into());
    }
    let mut problem = None;
    let (target_dependencies, target_date) =
        match crate::modrinth::version_detail(&client, &latest.id).await {
            Ok((dependencies, changelog, date)) => {
                base.changelog = truncate_changelog(&changelog);
                (dependencies, date)
            }
            Err(_) => {
                // The version list already carried the declarations; only the
                // changelog is lost, so keep the update actionable.
                problem = Some(format!("{}: version details unavailable", entry.title));
                (latest.dependencies.clone(), latest.date_published.clone())
            }
        };
    let current = crate::modrinth::version_detail(&client, &entry.version_id).await;
    let (current_dependencies, current_date) = match current {
        Ok((dependencies, _, date)) => (dependencies, Some(date)),
        Err(_) => {
            base.warnings.push("currentUnavailable".into());
            (Vec::new(), None)
        }
    };
    if let Some(current_date) = current_date {
        if !target_date.is_empty() && target_date < current_date {
            base.warnings.push("olderThanInstalled".into());
        }
    }
    let (dependencies, warnings) =
        dependency_impact(&current_dependencies, &target_dependencies, &installed);
    base.dependencies = dependencies;
    base.warnings.extend(warnings);
    Job::Done(base, problem)
}

/// Query Modrinth for every managed project and describe the update it has,
/// if any. Nothing is downloaded or changed.
pub async fn check(m: Arc<Manager>, id: String) -> Result<UpdateReport> {
    let server = m.metadata(&id)?;
    if server.server_type == ServerType::Vanilla {
        return Ok(UpdateReport {
            entries: Vec::new(),
            complete: true,
            errors: Vec::new(),
        });
    }
    let listing = crate::modrinth::listing(&m, &id)?;
    if listing.entries.len() > MAX_PROJECTS {
        return Err(
            AppError::user("Check at most 128 managed projects at a time.")
                .with_code("contentUpdate.tooMany"),
        );
    }
    let loader: &'static str = if server.server_type == ServerType::Paper {
        "paper"
    } else {
        "fabric"
    };
    let pinned: BTreeSet<String> = listing.pinned.iter().cloned().collect();
    let installed: BTreeMap<String, String> = listing
        .entries
        .iter()
        .map(|entry| (entry.project_id.clone(), entry.version_id.clone()))
        .collect();
    let client = m.client();
    let minecraft = server.minecraft_version.clone();
    let checks = stream::iter(listing.entries.into_iter().map(|entry| {
        let client = client.clone();
        let minecraft = minecraft.clone();
        let pinned = pinned.clone();
        let installed = installed.clone();
        async move { check_one(client, entry, minecraft, loader, pinned, installed).await }
    }))
    .buffer_unordered(CONCURRENCY);
    tokio::pin!(checks);
    let deadline = tokio::time::Instant::now() + CHECK_DEADLINE;
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut complete = true;
    loop {
        let next = match tokio::time::timeout_at(deadline, checks.next()).await {
            Ok(value) => value,
            Err(_) => {
                complete = false;
                errors.push("Update check timed out after 45 seconds.".into());
                break;
            }
        };
        let Some(job) = next else {
            break;
        };
        match job {
            Job::Done(entry, problem) => {
                if let Some(problem) = problem {
                    complete = false;
                    errors.push(problem);
                }
                entries.push(entry);
            }
            Job::Failed(title, message) => {
                complete = false;
                errors.push(format!("{title}: {message}"));
            }
        }
    }
    let mut dependency_ids: Vec<String> = entries
        .iter()
        .flat_map(|entry| entry.dependencies.iter().map(|dep| dep.project_id.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_PROJECTS)
        .collect();
    dependency_ids.dedup();
    if !dependency_ids.is_empty() {
        if let Ok(titles) = crate::modrinth::project_titles(&client, &dependency_ids).await {
            for entry in &mut entries {
                for dependency in &mut entry.dependencies {
                    dependency.title = titles.get(&dependency.project_id).cloned();
                }
            }
        }
    }
    entries.sort_by_key(|entry| (rank(&entry.status), entry.title.to_lowercase()));
    Ok(UpdateReport {
        entries,
        complete,
        errors,
    })
}

fn rank(status: &str) -> u8 {
    match status {
        "update" => 0,
        "upToDate" => 1,
        "pinned" => 2,
        "unavailable" => 3,
        _ => 4,
    }
}

/// Move the installed file aside and put the verified staging file in place.
/// The installed file is only removed after the new one is in place, and is
/// restored when the swap fails.
pub(crate) fn swap_content_file(
    dir: &Path,
    old_name: &str,
    new_name: &str,
    stage: &Path,
) -> Result<()> {
    let old = dir.join(old_name);
    let new = dir.join(new_name);
    let parked = dir.join(format!(".{}.old", uuid::Uuid::new_v4()));
    storage::no_links(&old)?;
    storage::no_links(&new)?;
    storage::no_links(&parked)?;
    fs::rename(&old, &parked)?;
    if let Err(error) = fs::rename(stage, &new) {
        let _ = fs::rename(&parked, &old);
        return Err(error.into());
    }
    let _ = fs::remove_file(&parked);
    Ok(())
}

/// Download, verify and install a newer version of one managed project,
/// keeping the replaced JAR under `content-backups` for a single-step rollback.
pub async fn apply(
    m: Arc<Manager>,
    id: String,
    project_id: String,
    version_id: String,
    confirmed: bool,
) -> Result<UpdateOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Review the changelog and dependencies, then confirm the update.",
        )
        .with_code("contentUpdate.notConfirmed"));
    }
    m.ready()?;
    crate::modrinth::validate_project_id(&project_id)?;
    crate::modrinth::validate_project_id(&version_id)?;
    let runtime = m.runtime(&id)?;
    let _gate = runtime.gate.lock().await;
    runtime.idle()?;
    let server = m.metadata(&id)?;
    if !server.installed {
        return Err(
            AppError::user("Install the server software before updating content.")
                .with_code("server.notInstalled"),
        );
    }
    let folder = crate::modrinth::folder_for(&server)?;
    let kind = if folder == "plugins" { "plugin" } else { "mod" };
    let loader: &'static str = if server.server_type == ServerType::Paper {
        "paper"
    } else {
        "fabric"
    };
    let root = m.dir(&id)?;
    let content_dir = storage::child(&root, folder)?;
    let mut index = crate::modrinth::load_index(&root);
    let position = index
        .entries
        .iter()
        .position(|entry| entry.project_id == project_id)
        .ok_or_else(|| {
            AppError::user("This project is no longer tracked. Refresh the list before updating.")
                .with_code("contentUpdate.notManaged")
        })?;
    let entry = index.entries[position].clone();
    if !crate::modrinth::safe_component(&entry.file) {
        return Err(AppError::user(
            "The tracked file name is invalid. Remove and reinstall this project.",
        )
        .with_code("contentUpdate.fileMissing"));
    }
    let old_path = content_dir.join(&entry.file);
    storage::no_links(&old_path)?;
    if !old_path.is_file() {
        return Err(AppError::user(
            "The installed file is missing. Refresh the list before updating.",
        )
        .with_code("contentUpdate.fileMissing"));
    }
    let client = m.client();
    let available =
        crate::modrinth::versions(&client, &project_id, Some(&server.minecraft_version), None)
            .await?;
    let version = available
        .into_iter()
        .find(|version| version.id == version_id)
        .ok_or_else(|| {
            AppError::user(
                "This version is no longer listed for your Minecraft version. Refresh and try again.",
            )
            .with_code("contentUpdate.versionGone")
        })?;
    if !crate::modrinth::compatible(&version, loader) {
        return Err(AppError::user(format!(
            "This version is not compatible with a {} server.",
            server_type_name(server.server_type)
        ))
        .with_code("contentUpdate.incompatible"));
    }
    if version.id == entry.version_id {
        return Err(AppError::user("This project is already at this version.")
            .with_code("contentUpdate.sameVersion"));
    }
    let file = version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| AppError::user("This version has no downloadable file."))?;
    if file.size > crate::modrinth::MAX_CONTENT_BYTES {
        return Err(
            AppError::user("This file is larger than the 128 MB content limit.")
                .with_code("contentUpdate.tooLarge"),
        );
    }
    let name = if file.filename.trim().is_empty() {
        format!("{project_id}.jar")
    } else {
        file.filename.clone()
    };
    if !crate::modrinth::safe_component(&name) || !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(
            AppError::user("Modrinth returned an unsafe file name. Try another version.")
                .with_code("contentUpdate.unsafeName"),
        );
    }
    let dest = content_dir.join(&name);
    storage::no_links(&dest)?;
    if dest.exists() && !name.eq_ignore_ascii_case(&entry.file) {
        return Err(AppError::user(
            "A different file with this name already exists in the content folder.",
        )
        .with_code("contentUpdate.nameTaken"));
    }
    let stage = content_dir.join(format!(".{}.update", uuid::Uuid::new_v4()));
    storage::no_links(&stage)?;
    crate::modrinth::progress(
        &m,
        &id,
        "download",
        &name,
        0,
        1,
        0,
        (file.size > 0).then_some(file.size),
    );
    let downloaded = crate::modrinth::download_file(
        &client,
        &file.url,
        &stage,
        file.sha512.as_deref(),
        file.sha1.as_deref(),
        crate::modrinth::MAX_CONTENT_BYTES,
        |downloaded, total| {
            crate::modrinth::progress(&m, &id, "download", &name, 0, 1, downloaded, total)
        },
    )
    .await;
    if let Err(error) = downloaded {
        let _ = fs::remove_file(&stage);
        return Err(error);
    }
    crate::modrinth::progress(&m, &id, "verify", &name, 1, 1, 0, Some(0));
    // Keep a copy before touching the installed file. A failed snapshot aborts
    // the update with the old installation untouched.
    let backup_dir = storage::child(&root, BACKUP_DIR)?;
    let prepared = (|| -> Result<PathBuf> {
        fs::create_dir_all(&backup_dir)?;
        let saved_file = format!("{}.jar", uuid::Uuid::new_v4());
        let backup_path = backup_dir.join(&saved_file);
        storage::no_links(&backup_path)?;
        if let Err(error) = fs::copy(&old_path, &backup_path) {
            let _ = fs::remove_file(&backup_path);
            return Err(error.into());
        }
        Ok(backup_path)
    })();
    let backup_path = match prepared {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&stage);
            return Err(error);
        }
    };
    let saved_file = backup_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let mut store = rollback_store(&root);
    store
        .records
        .retain(|record| record.project_id != project_id);
    store.records.push(RollbackRecord {
        project_id: project_id.clone(),
        title: entry.title.clone(),
        kind: kind.into(),
        saved_file,
        previous_file: entry.file.clone(),
        previous_version_id: entry.version_id.clone(),
        previous_version_number: entry.version_number.clone(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    });
    if let Err(error) = save_rollback_store(&root, &store) {
        let _ = fs::remove_file(&stage);
        let _ = fs::remove_file(&backup_path);
        return Err(error);
    }
    if let Err(error) = swap_content_file(&content_dir, &entry.file, &name, &stage) {
        let _ = fs::remove_file(&stage);
        let _ = fs::remove_file(&backup_path);
        forget_rollback(&root, &project_id);
        return Err(error);
    }
    index.entries[position] = crate::modrinth::ContentEntry {
        file: name.clone(),
        project_id: project_id.clone(),
        title: entry.title.clone(),
        version_id: version.id.clone(),
        version_number: version.version_number.clone(),
        kind: kind.into(),
        installed_at: entry.installed_at.clone(),
    };
    if let Err(error) = crate::modrinth::save_index(&root, &index) {
        // Put the previous installation back; the rollback record would only
        // point at the version that is already installed again.
        let _ = fs::remove_file(&dest);
        let _ = fs::copy(&backup_path, &old_path);
        let _ = fs::remove_file(&backup_path);
        forget_rollback(&root, &project_id);
        return Err(error);
    }
    crate::modrinth::progress(&m, &id, "done", &name, 1, 1, 0, Some(0));
    m.logger.log(
        "INFO",
        &format!(
            "Updated {kind} {} from {} to {} on server {id}",
            entry.title, entry.version_number, version.version_number
        ),
    );
    Ok(UpdateOutcome {
        project_id,
        title: entry.title,
        from_version: entry.version_number,
        to_version: version.version_number,
        file: name,
    })
}

/// Restore the JAR that `apply` saved, then delete the saved copy. The world,
/// configuration and every other project stay untouched.
pub async fn rollback(
    m: Arc<Manager>,
    id: String,
    project_id: String,
    confirmed: bool,
) -> Result<RollbackOutcome> {
    if !confirmed {
        return Err(AppError::user("Confirm the rollback first.")
            .with_code("contentUpdate.rollbackNotConfirmed"));
    }
    m.ready()?;
    crate::modrinth::validate_project_id(&project_id)?;
    let runtime = m.runtime(&id)?;
    let _gate = runtime.gate.lock().await;
    runtime.idle()?;
    let server = m.metadata(&id)?;
    let folder = crate::modrinth::folder_for(&server)?;
    let root = m.dir(&id)?;
    let content_dir = storage::child(&root, folder)?;
    let mut store = rollback_store(&root);
    let record = store
        .records
        .iter()
        .find(|record| record.project_id == project_id)
        .cloned()
        .ok_or_else(|| {
            AppError::user("There is no saved version to roll back to for this project.")
                .with_code("contentUpdate.noRollback")
        })?;
    if !crate::modrinth::safe_component(&record.saved_file)
        || !crate::modrinth::safe_component(&record.previous_file)
    {
        return Err(AppError::user("The saved rollback record is invalid.")
            .with_code("contentUpdate.noRollback"));
    }
    let backup_path = storage::child(&root, BACKUP_DIR)?.join(&record.saved_file);
    storage::no_links(&backup_path)?;
    if !backup_path.is_file() {
        return Err(AppError::user(
            "The saved copy is missing, so this project cannot be rolled back.",
        )
        .with_code("contentUpdate.noRollback"));
    }
    let mut index = crate::modrinth::load_index(&root);
    let position = index
        .entries
        .iter()
        .position(|entry| entry.project_id == project_id);
    // Park the currently installed file, whatever its name. The update may
    // have kept the same file name, so this is what makes room for the restore.
    let mut parked: Option<(PathBuf, PathBuf)> = None;
    if let Some(position) = position {
        let current = index.entries[position].file.clone();
        if crate::modrinth::safe_component(&current) {
            let path = content_dir.join(&current);
            storage::no_links(&path)?;
            if path.is_file() {
                let temporary = content_dir.join(format!(".{}.rollback", uuid::Uuid::new_v4()));
                storage::no_links(&temporary)?;
                fs::rename(&path, &temporary)?;
                parked = Some((temporary, path));
            }
        }
    }
    let target = content_dir.join(&record.previous_file);
    storage::no_links(&target)?;
    if target.exists() {
        if let Some((temporary, original)) = &parked {
            let _ = fs::rename(temporary, original);
        }
        return Err(AppError::user(
            "An unexpected file already uses the previous file name. Remove it manually and retry.",
        )
        .with_code("contentUpdate.targetExists"));
    }
    if let Err(error) = fs::copy(&backup_path, &target) {
        if let Some((temporary, original)) = &parked {
            let _ = fs::rename(temporary, original);
        }
        return Err(error.into());
    }
    if let Some(position) = position {
        index.entries[position].file = record.previous_file.clone();
        index.entries[position].version_id = record.previous_version_id.clone();
        index.entries[position].version_number = record.previous_version_number.clone();
    } else {
        index.entries.push(crate::modrinth::ContentEntry {
            file: record.previous_file.clone(),
            project_id: project_id.clone(),
            title: record.title.clone(),
            version_id: record.previous_version_id.clone(),
            version_number: record.previous_version_number.clone(),
            kind: record.kind.clone(),
            installed_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    if let Err(error) = crate::modrinth::save_index(&root, &index) {
        let _ = fs::remove_file(&target);
        if let Some((temporary, original)) = &parked {
            let _ = fs::rename(temporary, original);
        }
        return Err(error);
    }
    if let Some((temporary, _)) = &parked {
        let _ = fs::remove_file(temporary);
    }
    store
        .records
        .retain(|record| record.project_id != project_id);
    if let Err(error) = save_rollback_store(&root, &store) {
        // The files are consistent again; a stale record only offers the same
        // rollback twice, so report the write failure instead of pretending.
        return Err(error);
    }
    let _ = fs::remove_file(&backup_path);
    crate::modrinth::progress(&m, &id, "done", &record.previous_file, 1, 1, 0, Some(0));
    m.logger.log(
        "INFO",
        &format!(
            "Rolled back {} to {} on server {id}",
            record.title, record.previous_version_number
        ),
    );
    Ok(RollbackOutcome {
        project_id,
        title: record.title,
        version: record.previous_version_number,
    })
}

fn server_type_name(software: ServerType) -> &'static str {
    match software {
        ServerType::Vanilla => "Vanilla",
        ServerType::Paper => "Paper",
        ServerType::Fabric => "Fabric",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modrinth::{ProjectDependency, ProjectVersion, VersionFile};

    fn version(id: &str, number: &str, channel: &str, date: &str) -> ProjectVersion {
        ProjectVersion {
            id: id.into(),
            project_id: "project".into(),
            name: number.into(),
            version_number: number.into(),
            channel: channel.into(),
            game_versions: vec!["1.21.4".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: date.into(),
            files: vec![VersionFile {
                filename: format!("{id}.jar"),
                url: format!("https://cdn.modrinth.com/data/{id}.jar"),
                size: 10,
                sha1: None,
                sha512: None,
                primary: true,
            }],
            dependencies: Vec::new(),
        }
    }

    fn dep(project: &str, version: Option<&str>, kind: &str) -> ProjectDependency {
        ProjectDependency {
            project_id: Some(project.into()),
            version_id: version.map(str::to_string),
            dependency_type: kind.into(),
        }
    }

    #[test]
    fn latest_prefers_the_newest_release() {
        let versions = vec![
            version("a", "1.0", "release", "2025-01-01T00:00:00Z"),
            version("b", "2.0-beta", "beta", "2026-01-01T00:00:00Z"),
            version("c", "1.1", "release", "2025-06-01T00:00:00Z"),
        ];
        assert_eq!(choose_latest(&versions).unwrap().id, "c");
        let only_beta = vec![
            version("a", "1.0", "release", "2024-01-01T00:00:00Z"),
            version("b", "2.0-beta", "beta", "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(choose_latest(&only_beta).unwrap().id, "a");
        let no_release = vec![version("b", "2.0", "alpha", "2026-01-01T00:00:00Z")];
        assert_eq!(choose_latest(&no_release).unwrap().id, "b");
        assert!(choose_latest(&[]).is_none());
    }

    #[test]
    fn dependency_changes_are_classified() {
        let current = vec![
            dep("kept", Some("1"), "required"),
            dep("changed", Some("1"), "required"),
            dep("removed", Some("1"), "required"),
            dep("same", Some("1"), "required"),
        ];
        let target = vec![
            dep("kept", Some("1"), "required"),
            dep("changed", Some("2"), "required"),
            dep("added", Some("1"), "required"),
            dep("optional", Some("1"), "optional"),
            dep("same", Some("1"), "required"),
            dep("blocked", None, "incompatible"),
        ];
        let mut installed = BTreeMap::new();
        installed.insert("kept".into(), "1".into());
        installed.insert("changed".into(), "1".into());
        installed.insert("removed".into(), "1".into());
        installed.insert("same".into(), "1".into());
        installed.insert("blocked".into(), "9".into());
        let (impact, warnings) = dependency_impact(&current, &target, &installed);
        let status = |project: &str| {
            impact
                .iter()
                .find(|entry| entry.project_id == project)
                .map(|entry| entry.status.as_str())
        };
        assert_eq!(status("kept"), None);
        assert_eq!(status("same"), None);
        assert_eq!(status("optional"), None);
        assert_eq!(status("changed"), Some("changed"));
        assert_eq!(status("added"), Some("missing"));
        assert_eq!(status("removed"), Some("removed"));
        assert_eq!(status("blocked"), Some("incompatible"));
        assert!(warnings.contains(&"missingDependency".to_string()));
        assert!(warnings.contains(&"incompatibleDependency".to_string()));
    }

    #[test]
    fn present_required_dependency_is_not_flagged() {
        let target = vec![dep("fabric-api", None, "required")];
        let installed = BTreeMap::from([("fabric-api".to_string(), "v1".to_string())]);
        let (impact, warnings) = dependency_impact(&[], &target, &installed);
        assert_eq!(impact.len(), 1);
        assert_eq!(impact[0].status, "added");
        assert!(impact[0].installed);
        assert!(warnings.is_empty());
    }

    #[test]
    fn changelogs_are_bounded() {
        assert!(truncate_changelog("  ").is_none());
        assert_eq!(truncate_changelog(" hello ").as_deref(), Some("hello"));
        let long = "x".repeat(MAX_CHANGELOG_CHARS + 10);
        let cut = truncate_changelog(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_CHANGELOG_CHARS + 1);
    }

    #[test]
    fn swapping_content_handles_renamed_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fs::write(dir.join("old.jar"), b"old").unwrap();
        fs::write(dir.join(".stage"), b"new").unwrap();
        swap_content_file(dir, "old.jar", "new.jar", &dir.join(".stage")).unwrap();
        assert!(!dir.join("old.jar").exists());
        assert_eq!(fs::read(dir.join("new.jar")).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    }

    fn record(saved: &str) -> RollbackRecord {
        RollbackRecord {
            project_id: "project".into(),
            title: "Project".into(),
            kind: "mod".into(),
            saved_file: saved.into(),
            previous_file: "old.jar".into(),
            previous_version_id: "v1".into(),
            previous_version_number: "1.0".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn rollback_store_roundtrip_and_filtering() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let store = RollbackStore {
            records: vec![
                record("saved.jar"),
                record("../escape.jar"),
                record("missing.jar"),
            ],
        };
        save_rollback_store(root, &store).unwrap();
        fs::create_dir_all(root.join(BACKUP_DIR)).unwrap();
        fs::write(root.join(BACKUP_DIR).join("saved.jar"), b"old").unwrap();
        let loaded = rollbacks(root);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].saved_file, "saved.jar");
    }

    #[test]
    fn pins_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        assert!(pins(root).is_empty());
        save_pins(root, &["b".into(), "a".into(), "b".into()]).unwrap();
        assert_eq!(pins(root), vec!["a", "b"]);
    }
}
