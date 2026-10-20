use crate::{
    error::{AppError, Result},
    models::{Server, ServerType},
    server::manager::Manager,
    storage,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha512};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

const API_BASE: &str = "https://api.modrinth.com/v2";
const PAGE_SIZE: u64 = 20;
pub(crate) const MAX_CONTENT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OVERRIDE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const INDEX_FILE: &str = "content.json";
const PACK_RECORD: &str = "modpack.json";
const PACK_PENDING: &str = "modpack-pending.json";
const PACK_PENDING_FILE: &str = "modpack-pending.mrpack";
const PACK_ROLLBACK: &str = "modpack-rollback.json";
const PACK_BACKUP_DIR: &str = "modpack-backups";
const MAX_DIFF_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHit {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub follows: u64,
    pub categories: Vec<String>,
    pub client_side: String,
    pub server_side: String,
    pub project_type: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPage {
    pub hits: Vec<ProjectHit>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionFile {
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub primary: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub channel: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub downloads: u64,
    pub date_published: String,
    pub files: Vec<VersionFile>,
    pub dependencies: Vec<ProjectDependency>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct ProjectDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub dependency_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEntry {
    pub file: String,
    pub project_id: String,
    pub title: String,
    pub version_id: String,
    pub version_number: String,
    pub kind: String,
    pub installed_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRollback {
    pub project_id: String,
    pub version_number: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentListing {
    pub entries: Vec<ContentEntry>,
    pub unmanaged: Vec<String>,
    pub pinned: Vec<String>,
    pub rollbacks: Vec<ContentRollback>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentProgress {
    pub id: String,
    pub stage: String,
    pub file: String,
    pub completed: u32,
    pub total: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackReport {
    pub name: String,
    pub version: String,
    pub files: u32,
    pub reused: u32,
    pub skipped: u32,
    pub overrides: u32,
    pub minecraft: String,
    pub loader: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledPackFile {
    pub path: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledPack {
    #[serde(default)]
    pub schema_version: u32,
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version: String,
    pub minecraft: String,
    pub loader: String,
    #[serde(default)]
    pub managed: Vec<InstalledPackFile>,
    #[serde(default)]
    pub overrides: Vec<InstalledPackFile>,
    pub installed_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackInfo {
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    pub version: String,
    pub minecraft: String,
    pub loader: String,
    pub files: u32,
    pub overrides: u32,
    pub installed_at: String,
    pub can_rollback: bool,
    pub rollback_version: Option<String>,
}
impl From<&InstalledPack> for ModpackInfo {
    fn from(pack: &InstalledPack) -> Self {
        Self {
            project_id: pack.project_id.clone(),
            version_id: pack.version_id.clone(),
            name: pack.name.clone(),
            version: pack.version.clone(),
            minecraft: pack.minecraft.clone(),
            loader: pack.loader.clone(),
            files: pack.managed.len() as u32,
            overrides: pack.overrides.len() as u32,
            installed_at: pack.installed_at.clone(),
            can_rollback: false,
            rollback_version: None,
        }
    }
}
/// A preview computed from the actual pack archive, so the file lists match
/// what an update would change rather than guessing from version metadata.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackUpdatePreview {
    pub project_id: String,
    pub name: String,
    pub from_version: String,
    pub to_version_id: String,
    pub to_version: String,
    pub channel: String,
    pub changelog: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: u32,
    pub overrides_changed: u32,
    pub truncated: bool,
    pub minecraft: String,
    pub loader: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackUpdateOutcome {
    pub name: String,
    pub from_version: String,
    pub to_version: String,
    pub files: u32,
    pub reused: u32,
    pub skipped: u32,
    pub overrides: u32,
    pub backup: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackRollbackOutcome {
    pub name: String,
    pub restored_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingPack {
    version_id: String,
    version: String,
    channel: String,
    changelog: String,
    sha1: Option<String>,
    sha512: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackRollback {
    backup: String,
    from_version: String,
    to_version: String,
    created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackBackup {
    previous: InstalledPack,
    /// Files copied to `files/` so a rollback can put them back.
    restore: Vec<String>,
    /// Files introduced by the update, removed again on rollback.
    added: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentIndex {
    #[serde(default)]
    pub(crate) entries: Vec<ContentEntry>,
}

#[derive(Debug, Deserialize)]
struct RawHit {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    follows: u64,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    client_side: String,
    #[serde(default)]
    server_side: String,
    #[serde(default)]
    project_type: String,
}
#[derive(Debug, Deserialize)]
struct RawSearch {
    #[serde(default)]
    hits: Vec<RawHit>,
    #[serde(default)]
    total: u64,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    limit: u64,
}
#[derive(Debug, Default, Deserialize)]
struct RawHashes {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}
#[derive(Debug, Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    hashes: RawHashes,
    #[serde(default)]
    primary: bool,
}
#[derive(Debug, Deserialize)]
struct RawVersion {
    #[serde(default)]
    dependencies: Vec<ProjectDependency>,
    #[serde(default)]
    id: String,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_number: String,
    #[serde(default, rename = "version_type")]
    channel: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    changelog: String,
    #[serde(default)]
    files: Vec<RawFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackIndex {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_id: String,
    #[serde(default)]
    files: Vec<PackFile>,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    hashes: RawHashes,
    #[serde(default)]
    env: Option<PackEnv>,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    file_size: u64,
}
#[derive(Debug, Default, Deserialize)]
struct PackEnv {
    #[serde(default)]
    server: Option<String>,
}
#[derive(Debug)]
struct PackOverride {
    path: String,
    entry: String,
    sha1: String,
}
#[derive(Debug)]
struct PackLayout {
    pack: PackIndex,
    managed: Vec<PackFile>,
    overrides: Vec<PackOverride>,
    skipped: u32,
}

pub async fn search(
    client: &reqwest::Client,
    query: &str,
    project_type: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
    offset: u64,
) -> Result<ProjectPage> {
    if !matches!(project_type, "mod" | "plugin" | "modpack") {
        return Err(AppError::user("Unsupported Modrinth project type."));
    }
    let facets = facets(project_type, game_version, loader);
    let response = client
        .get(format!("{API_BASE}/search"))
        .query(&[
            ("query", query.trim().to_string()),
            ("limit", PAGE_SIZE.to_string()),
            ("offset", offset.min(5000).to_string()),
            ("index", "relevance".to_string()),
            ("facets", facets),
        ])
        .send()
        .await?
        .error_for_status()?;
    let raw: RawSearch = response.json().await?;
    Ok(map_search(raw))
}

fn map_search(raw: RawSearch) -> ProjectPage {
    ProjectPage {
        hits: raw
            .hits
            .into_iter()
            .filter(|h| !h.project_id.is_empty() && !h.slug.is_empty())
            .map(|h| ProjectHit {
                id: h.project_id,
                slug: h.slug,
                title: h.title,
                description: h.description,
                author: h.author,
                icon_url: h.icon_url.filter(|u| !u.is_empty()),
                downloads: h.downloads,
                follows: h.follows,
                categories: h.categories,
                client_side: h.client_side,
                server_side: h.server_side,
                project_type: h.project_type,
            })
            .collect(),
        total: raw.total,
        offset: raw.offset,
        limit: raw.limit.max(PAGE_SIZE),
    }
}

pub async fn versions(
    client: &reqwest::Client,
    project_id: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<ProjectVersion>> {
    validate_project_id(project_id)?;
    let mut request = client.get(format!("{API_BASE}/project/{project_id}/version"));
    if let Some(game) = game_version.filter(|g| !g.is_empty()) {
        request = request.query(&[("game_versions", json!([game]).to_string())]);
    }
    if let Some(loader) = loader.filter(|l| !l.is_empty()) {
        request = request.query(&[("loaders", json!([loader]).to_string())]);
    }
    let raw: Vec<RawVersion> = request.send().await?.error_for_status()?.json().await?;
    Ok(map_versions(raw))
}

fn map_versions(raw: Vec<RawVersion>) -> Vec<ProjectVersion> {
    raw.into_iter()
        .filter(|v| !v.id.is_empty())
        .map(|v| ProjectVersion {
            dependencies: v.dependencies,
            id: v.id,
            project_id: v.project_id,
            name: v.name,
            version_number: v.version_number,
            channel: v.channel,
            game_versions: v.game_versions,
            loaders: v.loaders,
            downloads: v.downloads,
            date_published: v.date_published,
            files: v
                .files
                .into_iter()
                .filter(|f| !f.url.is_empty())
                .map(|f| VersionFile {
                    filename: f.filename,
                    url: f.url,
                    size: f.size,
                    sha1: f.hashes.sha1,
                    sha512: f.hashes.sha512,
                    primary: f.primary,
                })
                .collect(),
        })
        .collect()
}

pub async fn version_dependencies(
    client: &reqwest::Client,
    id: &str,
) -> Result<Vec<ProjectDependency>> {
    validate_project_id(id)?;
    let raw: RawVersion = client
        .get(format!("{API_BASE}/version/{id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(raw.dependencies)
}
/// Full version metadata needed by update checks: declared dependencies, the
/// author's changelog and the publication date.
pub async fn version_detail(
    client: &reqwest::Client,
    id: &str,
) -> Result<(Vec<ProjectDependency>, String, String)> {
    validate_project_id(id)?;
    let raw: RawVersion = client
        .get(format!("{API_BASE}/version/{id}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok((
        raw.dependencies,
        raw.changelog.trim().to_string(),
        raw.date_published,
    ))
}
#[derive(Debug, Default, Deserialize)]
struct RawProject {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
}
/// Resolve display names for dependency project identifiers in one request.
/// Missing or unknown projects are simply absent from the map.
pub async fn project_titles(
    client: &reqwest::Client,
    ids: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut titles = BTreeMap::new();
    if ids.is_empty() {
        return Ok(titles);
    }
    if ids.len() > 128 {
        return Err(AppError::user("Resolve at most 128 projects at a time."));
    }
    for id in ids {
        validate_project_id(id)?;
    }
    let response = client
        .get(format!("{API_BASE}/projects"))
        .query(&[("ids", serde_json::to_string(ids)?)])
        .send()
        .await?
        .error_for_status()?;
    let raw: Vec<RawProject> = response.json().await?;
    for project in raw {
        if !project.id.is_empty() && !project.title.trim().is_empty() {
            titles.insert(project.id, project.title);
        }
    }
    Ok(titles)
}

/// Map a locally installed JAR to its Modrinth version using its SHA-1 hash.
/// The hash is the only stable link for manually installed files, whose names
/// and Fabric IDs need not match a Modrinth project slug.
pub async fn version_by_hash(client: &reqwest::Client, hash: &str) -> Result<ProjectVersion> {
    version_by_hash_opt(client, hash)
        .await?
        .ok_or_else(|| AppError::user("Modrinth did not return a project for this file hash."))
}
/// Same lookup, but an unlisted file is `None` rather than an error. Most
/// manually installed JARs are not published on Modrinth.
pub async fn version_by_hash_opt(
    client: &reqwest::Client,
    hash: &str,
) -> Result<Option<ProjectVersion>> {
    if !(32..=64).contains(&hash.len()) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::user("The file hash is not a valid SHA-1 value."));
    }
    let response = client
        .get(format!("{API_BASE}/version_file/{hash}?algorithm=sha1"))
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let raw: RawVersion = response.error_for_status()?.json().await?;
    if raw.project_id.is_empty() || raw.id.is_empty() {
        return Ok(None);
    }
    Ok(map_versions(vec![raw]).into_iter().next())
}

pub async fn install(
    m: Arc<Manager>,
    id: String,
    project_id: String,
    version_id: String,
    kind: String,
) -> Result<ContentEntry> {
    m.ready()?;
    validate_project_id(&project_id)?;
    let r = m.runtime(&id)?;
    let _guard = r.gate.lock().await;
    r.idle()?;
    let server = m.metadata(&id)?;
    if !server.installed {
        return Err(AppError::user(
            "Install the server first, then add mods or plugins.",
        ));
    }
    let (folder, loader) = target(&server, &kind)?;
    let root = m.dir(&id)?;
    let content_dir = storage::child(&root, folder)?;
    fs::create_dir_all(&content_dir)?;
    let client = m.client();
    let available = versions(&client, &project_id, Some(&server.minecraft_version), None).await?;
    let version = available
        .into_iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            AppError::user(
                "This version is no longer listed for your Minecraft version. Refresh the list and try again.",
            )
        })?;
    if !compatible(&version, loader) {
        return Err(AppError::user(format!(
            "This download is not compatible with a {} server.",
            server_name(server.server_type)
        )));
    }
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| AppError::user("This version has no downloadable file."))?;
    if file.size > MAX_CONTENT_BYTES {
        return Err(AppError::user(
            "This file is larger than the 128 MB content limit.",
        ));
    }
    let name = if file.filename.trim().is_empty() {
        format!("{project_id}.jar")
    } else {
        file.filename.clone()
    };
    if !safe_component(&name) || !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(AppError::user(
            "Modrinth returned an unsafe file name. Try another version.",
        ));
    }
    let dest = content_dir.join(&name);
    storage::no_links(&dest)?;
    progress(
        &m,
        &id,
        "download",
        &name,
        0,
        1,
        0,
        (file.size > 0).then_some(file.size),
    );
    download_file(
        &client,
        &file.url,
        &dest,
        file.sha512.as_deref(),
        file.sha1.as_deref(),
        MAX_CONTENT_BYTES,
        |d, t| progress(&m, &id, "download", &name, 0, 1, d, t),
    )
    .await?;
    let entry = ContentEntry {
        file: name.clone(),
        project_id: project_id.clone(),
        title: if version.name.trim().is_empty() {
            project_id.clone()
        } else {
            version.name.clone()
        },
        version_id: version.id.clone(),
        version_number: version.version_number.clone(),
        kind: kind.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut index = load_index(&root);
    let previous: Vec<String> = index
        .entries
        .iter()
        .filter(|e| e.project_id == project_id)
        .map(|e| e.file.clone())
        .collect();
    index.entries.retain(|e| e.project_id != project_id);
    index.entries.push(entry.clone());
    save_index(&root, &index)?;
    for old in previous {
        if old != name && safe_component(&old) {
            let _ = fs::remove_file(content_dir.join(&old));
        }
    }
    progress(&m, &id, "done", &name, 1, 1, 0, Some(0));
    m.logger.log(
        "INFO",
        &format!("Installed {kind} {} into server {id}", entry.title),
    );
    Ok(entry)
}

pub fn listing(m: &Manager, id: &str) -> Result<ContentListing> {
    let server = m.metadata(id)?;
    let Some(folder) = server.server_type.content_folder() else {
        return Ok(ContentListing {
            entries: Vec::new(),
            unmanaged: Vec::new(),
            pinned: Vec::new(),
            rollbacks: Vec::new(),
        });
    };
    let root = m.dir(id)?;
    let content_dir = storage::child(&root, folder)?;
    let mut index = load_index(&root);
    let mut managed = Vec::new();
    for entry in index.entries.drain(..) {
        let path = content_dir.join(&entry.file);
        if safe_component(&entry.file) && path.is_file() {
            managed.push(entry);
        }
    }
    index.entries = managed.clone();
    save_index(&root, &index)?;
    let mut unmanaged = Vec::new();
    for e in fs::read_dir(&content_dir)?.flatten() {
        storage::no_links(&e.path())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if !e.path().is_file() || !name.to_ascii_lowercase().ends_with(".jar") {
            continue;
        }
        if !managed.iter().any(|m| m.file.eq_ignore_ascii_case(&name)) {
            unmanaged.push(name);
        }
    }
    managed.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    unmanaged.sort_by_key(|n| n.to_lowercase());
    let pinned = crate::content_update::pins(&root);
    let rollbacks = crate::content_update::rollbacks(&root)
        .into_iter()
        .filter(|record| {
            managed
                .iter()
                .any(|entry| entry.project_id == record.project_id)
        })
        .map(|record| ContentRollback {
            project_id: record.project_id,
            version_number: record.previous_version_number,
            updated_at: record.updated_at,
        })
        .collect();
    Ok(ContentListing {
        entries: managed,
        unmanaged,
        pinned,
        rollbacks,
    })
}

pub async fn remove(m: Arc<Manager>, id: String, project_id: String) -> Result<()> {
    validate_project_id(&project_id)?;
    let r = m.runtime(&id)?;
    let _guard = r.gate.lock().await;
    r.idle()?;
    let server = m.metadata(&id)?;
    let folder = folder_for(&server)?;
    let root = m.dir(&id)?;
    let content_dir = storage::child(&root, folder)?;
    let mut index = load_index(&root);
    let mut removed = 0u32;
    index.entries.retain(|e| {
        if e.project_id != project_id {
            return true;
        }
        if safe_component(&e.file) {
            let path = content_dir.join(&e.file);
            if storage::no_links(&path).is_ok() {
                let _ = fs::remove_file(path);
                removed += 1;
            }
        }
        false
    });
    if removed == 0 {
        return Err(AppError::user(
            "This mod or plugin is no longer tracked. Remove its file manually from the content folder.",
        ));
    }
    save_index(&root, &index)?;
    Ok(())
}

pub async fn remove_file(m: Arc<Manager>, id: String, kind: String, name: String) -> Result<()> {
    let r = m.runtime(&id)?;
    let _guard = r.gate.lock().await;
    r.idle()?;
    let server = m.metadata(&id)?;
    let (folder, _) = target(&server, &kind)?;
    if !safe_component(&name) || !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(AppError::user(
            "Only JAR files inside the managed content folder can be removed.",
        ));
    }
    let root = m.dir(&id)?;
    let content_dir = storage::child(&root, folder)?;
    let path = content_dir.join(&name);
    storage::no_links(&path)?;
    let mut index = load_index(&root);
    let tracked = index
        .entries
        .iter()
        .any(|e| e.file.eq_ignore_ascii_case(&name));
    if !path.is_file() && !tracked {
        return Err(AppError::user("This file has already been removed."));
    }
    if path.is_file() {
        fs::remove_file(&path)?;
    }
    index
        .entries
        .retain(|e| !e.file.eq_ignore_ascii_case(&name));
    save_index(&root, &index)?;
    Ok(())
}

pub async fn install_modpack(
    m: Arc<Manager>,
    id: String,
    project_id: String,
    version_id: String,
) -> Result<ModpackReport> {
    m.ready()?;
    validate_project_id(&project_id)?;
    let r = m.runtime(&id)?;
    let _guard = r.gate.lock().await;
    r.idle()?;
    let server = m.metadata(&id)?;
    if !server.installed {
        return Err(AppError::user(
            "Install the server software first, then apply a modpack.",
        ));
    }
    if server.server_type.plugins() {
        return Err(AppError::user(
            "Plugin servers use plugins. Create a Fabric server to install modpacks.",
        ));
    }
    let root = m.dir(&id)?;
    if let Some(existing) = read_installed_pack(&root) {
        if existing.project_id != project_id {
            return Err(AppError::user(format!(
                "The modpack {} is already installed. EasyLauncher manages one modpack per server; create a new server to use a different pack.",
                existing.name
            )));
        }
        if existing.version_id != version_id {
            return Err(AppError::user(
                "A different version of this modpack is installed. Use \"Check modpack updates\" in Installed content so files are backed up and can be rolled back.",
            ));
        }
    }
    let client = m.client();
    let available = versions(&client, &project_id, None, None).await?;
    let version = available
        .into_iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            AppError::user("This modpack version is no longer available. Refresh and try again.")
        })?;
    let file = version
        .files
        .iter()
        .find(|f| f.primary && f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        .or_else(|| {
            version
                .files
                .iter()
                .find(|f| f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        })
        .ok_or_else(|| AppError::user("This modpack version has no .mrpack file."))?;
    if file.size > MAX_PACK_BYTES {
        return Err(AppError::user(
            "This modpack is larger than the 512 MB safety limit.",
        ));
    }
    let temp = root.join(format!(".pack-{}.mrpack", uuid::Uuid::new_v4()));
    storage::no_links(&temp)?;
    progress(
        &m,
        &id,
        "download",
        &file.filename,
        0,
        1,
        0,
        (file.size > 0).then_some(file.size),
    );
    let downloaded = download_file(
        &client,
        &file.url,
        &temp,
        file.sha512.as_deref(),
        file.sha1.as_deref(),
        MAX_PACK_BYTES,
        |d, t| progress(&m, &id, "download", &file.filename, 0, 1, d, t),
    )
    .await;
    if let Err(e) = downloaded {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    progress(&m, &id, "verify", &file.filename, 1, 1, 0, Some(0));
    let result = install_pack(&m, &id, &server, &root, &temp, &client, &version).await;
    let _ = fs::remove_file(&temp);
    result
}

async fn install_pack(
    m: &Manager,
    id: &str,
    server: &Server,
    root: &Path,
    temp: &Path,
    client: &reqwest::Client,
    project_version: &ProjectVersion,
) -> Result<ModpackReport> {
    let layout = pack_layout(temp)?;
    let (loader, minecraft) = validate_pack(server, &layout.pack)?;
    let report = apply_pack(m, id, server, root, temp, client).await?;
    let record = installed_record(
        &project_version.project_id,
        &project_version.id,
        &project_version.version_number,
        &report.name,
        minecraft,
        loader,
        &layout,
    );
    save_installed_pack(root, &record)?;
    // A fresh or repair install starts a new baseline, so any rollback left
    // over from an older update no longer applies.
    cleanup_pending(root);
    if let Some(marker) = read_rollback(root) {
        remove_backup_dir(root, &marker.backup);
    }
    if let Ok(path) = storage::child(root, PACK_ROLLBACK) {
        let _ = fs::remove_file(path);
    }
    Ok(report)
}

async fn apply_pack(
    m: &Manager,
    id: &str,
    server: &Server,
    root: &Path,
    temp: &Path,
    client: &reqwest::Client,
) -> Result<ModpackReport> {
    let handle = fs::File::open(temp)?;
    let mut archive = zip::ZipArchive::new(handle)
        .map_err(|e| AppError::new("This file is not a valid .mrpack archive.", e))?;
    let mut raw = Vec::new();
    {
        let index = archive
            .by_name("modrinth.index.json")
            .map_err(|_| AppError::user("This .mrpack is missing modrinth.index.json."))?;
        index.take(MAX_INDEX_BYTES + 1).read_to_end(&mut raw)?;
    }
    if raw.len() as u64 > MAX_INDEX_BYTES {
        return Err(AppError::user(
            "The modpack index is larger than the 8 MB safety limit.",
        ));
    }
    let pack: PackIndex = serde_json::from_slice(&raw).map_err(|e| {
        AppError::new(
            "The modpack index could not be parsed. The file may be corrupt.",
            e,
        )
    })?;
    if pack.format_version != 1 {
        return Err(AppError::user(
            "This modpack format is not supported by EasyLauncher yet.",
        ));
    }
    let loader = detect_loader(&pack.dependencies)?;
    match (server.server_type, loader) {
        (ServerType::Fabric, _) => {}
        (ServerType::Vanilla, "fabric") => {
            return Err(AppError::user(
                "This modpack requires Fabric. Create a Fabric server with the same Minecraft version and apply it there.",
            ))
        }
        (ServerType::Vanilla, _) => {}
        (ServerType::Paper | ServerType::Purpur, _) => {
            return Err(AppError::user(
                "Plugin servers cannot run modpacks. Use Fabric or Vanilla.",
            ))
        }
    }
    let minecraft = pack
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| AppError::user("This modpack does not declare a Minecraft version."))?;
    if minecraft != server.minecraft_version {
        return Err(AppError::user(format!(
            "This modpack targets Minecraft {minecraft}, but this server runs {}. Create a {minecraft} server first.",
            server.minecraft_version
        )));
    }
    let mut report = ModpackReport {
        name: if pack.name.trim().is_empty() {
            "Modpack".into()
        } else {
            pack.name.clone()
        },
        version: pack.version_id.clone(),
        files: 0,
        reused: 0,
        skipped: 0,
        overrides: 0,
        minecraft,
        loader: loader.into(),
    };
    let total = pack
        .files
        .iter()
        .filter(|f| server_supported(f) && !protected(&f.path))
        .count() as u32;
    let mut completed = 0u32;
    for file in &pack.files {
        if !server_supported(file) || protected(&file.path) {
            report.skipped += 1;
            continue;
        }
        if file.file_size > MAX_PACK_FILE_BYTES {
            report.skipped += 1;
            continue;
        }
        let url = file
            .downloads
            .iter()
            .find(|u| u.starts_with("https://"))
            .ok_or_else(|| {
                AppError::user("The modpack contains a file without a secure download.")
            })?;
        let dest = safe_join(root, &file.path)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        progress(
            m,
            id,
            "download",
            &file.path,
            completed,
            total,
            0,
            Some(file.file_size),
        );
        download_file(
            client,
            url,
            &dest,
            file.hashes.sha512.as_deref(),
            file.hashes.sha1.as_deref(),
            MAX_PACK_FILE_BYTES,
            |d, t| progress(m, id, "download", &file.path, completed, total, d, t),
        )
        .await?;
        completed += 1;
        report.files += 1;
    }
    let mut budget = MAX_OVERRIDE_BYTES;
    let mut overrides = 0u32;
    for prefix in ["overrides/", "server-overrides/"] {
        let count = archive
            .file_names()
            .filter(|n| n.starts_with(prefix))
            .count() as u32;
        let mut done = 0u32;
        for i in 0..archive.len() {
            let (name, is_dir) = {
                let entry = archive
                    .by_index(i)
                    .map_err(|e| AppError::new("The modpack archive could not be read.", e))?;
                (entry.name().to_string(), entry.is_dir())
            };
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() || is_dir {
                continue;
            }
            if protected(relative) {
                report.skipped += 1;
                continue;
            }
            let dest = safe_join(root, relative)?;
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut entry = archive
                .by_index(i)
                .map_err(|e| AppError::new("The modpack archive could not be read.", e))?;
            if let Some(parent) = dest.parent() {
                storage::no_links(parent)?;
            }
            let mut out = fs::File::create(&dest)?;
            let copied = std::io::copy(&mut entry.by_ref().take(budget + 1), &mut out)?;
            if copied > budget {
                drop(out);
                let _ = fs::remove_file(&dest);
                return Err(AppError::user(
                    "The modpack overrides are larger than the 512 MB safety limit.",
                ));
            }
            out.sync_all()?;
            budget -= copied;
            overrides += 1;
            done += 1;
            progress(
                m,
                id,
                "overrides",
                relative,
                done,
                count,
                copied,
                Some(copied),
            );
        }
    }
    report.overrides = overrides;
    progress(
        m,
        id,
        "done",
        &report.name,
        report.files,
        report.files,
        0,
        Some(0),
    );
    m.logger.log(
        "INFO",
        &format!(
            "Applied modpack {} {} to server {id}",
            report.name, report.version
        ),
    );
    Ok(report)
}

/// Reads the pack index and every server override once. The resulting layout is
/// used for install records, update diffs and extraction, so all three agree on
/// which paths belong to the pack.
fn pack_layout(path: &Path) -> Result<PackLayout> {
    let handle = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(handle)
        .map_err(|e| AppError::new("This file is not a valid .mrpack archive.", e))?;
    let mut raw = Vec::new();
    {
        let index = archive
            .by_name("modrinth.index.json")
            .map_err(|_| AppError::user("This .mrpack is missing modrinth.index.json."))?;
        index.take(MAX_INDEX_BYTES + 1).read_to_end(&mut raw)?;
    }
    if raw.len() as u64 > MAX_INDEX_BYTES {
        return Err(AppError::user(
            "The modpack index is larger than the 8 MB safety limit.",
        ));
    }
    let mut pack: PackIndex = serde_json::from_slice(&raw).map_err(|e| {
        AppError::new(
            "The modpack index could not be parsed. The file may be corrupt.",
            e,
        )
    })?;
    if pack.format_version != 1 {
        return Err(AppError::user(
            "This modpack format is not supported by EasyLauncher yet.",
        ));
    }
    let mut skipped = 0u32;
    let mut overrides: BTreeMap<String, PackOverride> = BTreeMap::new();
    let mut budget = MAX_OVERRIDE_BYTES;
    for prefix in ["overrides/", "server-overrides/"] {
        for index in 0..archive.len() {
            let (name, is_dir) = {
                let entry = archive
                    .by_index(index)
                    .map_err(|e| AppError::new("The modpack archive could not be read.", e))?;
                (entry.name().to_string(), entry.is_dir())
            };
            let Some(relative) = name.strip_prefix(prefix) else {
                continue;
            };
            if relative.is_empty() || is_dir {
                continue;
            }
            if protected(relative) {
                skipped += 1;
                continue;
            }
            let relative = safe_relative(relative)?;
            let mut entry = archive
                .by_index(index)
                .map_err(|e| AppError::new("The modpack archive could not be read.", e))?;
            let mut hasher = sha1::Sha1::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut size = 0u64;
            loop {
                let read = entry.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                size += read as u64;
                if size > budget {
                    return Err(AppError::user(
                        "The modpack overrides are larger than the 512 MB safety limit.",
                    ));
                }
                hasher.update(&buffer[..read]);
            }
            budget -= size;
            overrides.insert(
                relative.to_ascii_lowercase(),
                PackOverride {
                    path: relative,
                    entry: name,
                    sha1: format!("{:x}", hasher.finalize()),
                },
            );
        }
    }
    let mut managed: BTreeMap<String, PackFile> = BTreeMap::new();
    for mut file in std::mem::take(&mut pack.files) {
        if !server_supported(&file) || protected(&file.path) || file.file_size > MAX_PACK_FILE_BYTES
        {
            skipped += 1;
            continue;
        }
        if !file.downloads.iter().any(|url| url.starts_with("https://")) {
            return Err(AppError::user(
                "The modpack contains a file without a secure download.",
            ));
        }
        let relative = safe_relative(&file.path)?;
        let key = relative.to_ascii_lowercase();
        if overrides.contains_key(&key) || managed.contains_key(&key) {
            skipped += 1;
            continue;
        }
        file.path = relative;
        managed.insert(key, file);
    }
    Ok(PackLayout {
        pack,
        managed: managed.into_values().collect(),
        overrides: overrides.into_values().collect(),
        skipped,
    })
}

/// Narrow candidates using the loader tags Modrinth publishes for packs.
/// Empty tags are accepted because older packs may omit them, and the actual
/// archive is validated after download.
fn pack_version_matches_loader(version: &ProjectVersion, loader: &str) -> bool {
    let tags: Vec<String> = version
        .loaders
        .iter()
        .map(|l| l.to_ascii_lowercase())
        .collect();
    if loader == "fabric" {
        tags.is_empty() || tags.iter().any(|tag| tag == "fabric")
    } else {
        !tags
            .iter()
            .any(|tag| matches!(tag.as_str(), "fabric" | "quilt" | "forge" | "neoforge"))
    }
}

fn validate_pack(server: &Server, pack: &PackIndex) -> Result<(&'static str, String)> {
    let loader = detect_loader(&pack.dependencies)?;
    match (server.server_type, loader) {
        (ServerType::Fabric, _) => {}
        (ServerType::Vanilla, "fabric") => {
            return Err(AppError::user(
                "This modpack requires Fabric. Create a Fabric server with the same Minecraft version and apply it there.",
            ))
        }
        (ServerType::Vanilla, _) => {}
        (ServerType::Paper | ServerType::Purpur, _) => {
            return Err(AppError::user(
                "Plugin servers cannot run modpacks. Use Fabric or Vanilla.",
            ))
        }
    }
    let minecraft = pack
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| AppError::user("This modpack does not declare a Minecraft version."))?;
    if minecraft != server.minecraft_version {
        return Err(AppError::user(format!(
            "This modpack targets Minecraft {minecraft}, but this server runs {}. Create a {minecraft} server first.",
            server.minecraft_version
        )));
    }
    Ok((loader, minecraft))
}

fn read_installed_pack(root: &Path) -> Option<InstalledPack> {
    let path = storage::child(root, PACK_RECORD).ok()?;
    storage::no_links(&path).ok()?;
    let pack: InstalledPack = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (pack.schema_version <= 1 && !pack.project_id.is_empty()).then_some(pack)
}

fn save_installed_pack(root: &Path, pack: &InstalledPack) -> Result<()> {
    storage::write_json(&storage::child(root, PACK_RECORD)?, pack)
}

pub fn pack_info(m: &Manager, id: &str) -> Result<Option<ModpackInfo>> {
    let root = m.dir(id)?;
    let rollback = read_rollback(&root);
    Ok(read_installed_pack(&root).map(|pack| {
        let mut info = ModpackInfo::from(&pack);
        if let Some(marker) = rollback {
            info.can_rollback = true;
            info.rollback_version = Some(marker.from_version);
        }
        info
    }))
}

/// Compare any hash the two records share. SHA-512 is preferred, but SHA-1 is
/// the only hash Modrinth guarantees, so either match counts.
fn hash_pair_match(
    a1: Option<&str>,
    a512: Option<&str>,
    b1: Option<&str>,
    b512: Option<&str>,
) -> bool {
    let same = |a: Option<&str>, b: Option<&str>| match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    };
    same(a512, b512) || same(a1, b1)
}

/// Hash a local file and compare it with the expected download hashes. A file
/// without a recorded hash can never be reused, because its content is unknown.
fn file_matches_hashes(path: &Path, sha1: Option<&str>, sha512: Option<&str>) -> bool {
    if sha1.is_none() && sha512.is_none() {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut h1 = sha1::Sha1::new();
    let mut h512 = Sha512::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                h1.update(&buffer[..read]);
                h512.update(&buffer[..read]);
            }
            Err(_) => return false,
        }
    }
    let one = format!("{:x}", h1.finalize());
    let five = format!("{:x}", h512.finalize());
    sha1.map(|s| s.eq_ignore_ascii_case(&one)).unwrap_or(true)
        && sha512
            .map(|s| s.eq_ignore_ascii_case(&five))
            .unwrap_or(true)
}

fn installed_record(
    project_id: &str,
    version_id: &str,
    version: &str,
    name: &str,
    minecraft: String,
    loader: &str,
    layout: &PackLayout,
) -> InstalledPack {
    InstalledPack {
        schema_version: 1,
        project_id: project_id.into(),
        version_id: version_id.into(),
        name: if name.trim().is_empty() {
            "Modpack".into()
        } else {
            name.into()
        },
        version: version.into(),
        minecraft,
        loader: loader.into(),
        managed: layout
            .managed
            .iter()
            .map(|file| InstalledPackFile {
                path: file.path.clone(),
                sha1: file.hashes.sha1.clone(),
                sha512: file.hashes.sha512.clone(),
            })
            .collect(),
        overrides: layout
            .overrides
            .iter()
            .map(|file| InstalledPackFile {
                path: file.path.clone(),
                sha1: Some(file.sha1.clone()),
                sha512: None,
            })
            .collect(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn push_diff(list: &mut Vec<String>, path: &str, truncated: &mut bool) {
    if list.len() < MAX_DIFF_ENTRIES {
        list.push(path.into());
    } else {
        *truncated = true;
    }
}

fn pack_hashes(
    installed: &InstalledPack,
) -> BTreeMap<String, (String, Option<String>, Option<String>)> {
    let mut map = BTreeMap::new();
    for file in installed.managed.iter().chain(installed.overrides.iter()) {
        map.insert(
            file.path.to_ascii_lowercase(),
            (file.path.clone(), file.sha1.clone(), file.sha512.clone()),
        );
    }
    map
}

fn layout_hashes(
    layout: &PackLayout,
) -> BTreeMap<String, (String, Option<String>, Option<String>)> {
    let mut map = BTreeMap::new();
    for file in &layout.managed {
        map.insert(
            file.path.to_ascii_lowercase(),
            (
                file.path.clone(),
                file.hashes.sha1.clone(),
                file.hashes.sha512.clone(),
            ),
        );
    }
    for file in &layout.overrides {
        map.insert(
            file.path.to_ascii_lowercase(),
            (file.path.clone(), Some(file.sha1.clone()), None),
        );
    }
    map
}

fn diff_pack(
    installed: &InstalledPack,
    layout: &PackLayout,
    version: &ProjectVersion,
    changelog: String,
    minecraft: String,
    loader: &str,
) -> ModpackUpdatePreview {
    let old = pack_hashes(installed);
    let new = layout_hashes(layout);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut truncated = false;
    let mut unchanged = 0u32;
    for (key, (path, one, five)) in &new {
        match old.get(key) {
            None => push_diff(&mut added, path, &mut truncated),
            Some((_, old_one, old_five))
                if hash_pair_match(
                    one.as_deref(),
                    five.as_deref(),
                    old_one.as_deref(),
                    old_five.as_deref(),
                ) =>
            {
                unchanged += 1;
            }
            Some(_) => push_diff(&mut changed, path, &mut truncated),
        }
    }
    for (key, (path, _, _)) in &old {
        if !new.contains_key(key) {
            push_diff(&mut removed, path, &mut truncated);
        }
    }
    let mut overrides_changed = 0u32;
    for file in &layout.overrides {
        let key = file.path.to_ascii_lowercase();
        let same = old
            .get(&key)
            .map(|(_, one, five)| {
                hash_pair_match(Some(&file.sha1), None, one.as_deref(), five.as_deref())
            })
            .unwrap_or(false);
        if !same {
            overrides_changed += 1;
        }
    }
    for file in &installed.overrides {
        if !new.contains_key(&file.path.to_ascii_lowercase()) {
            overrides_changed += 1;
        }
    }
    ModpackUpdatePreview {
        project_id: installed.project_id.clone(),
        name: if layout.pack.name.trim().is_empty() {
            installed.name.clone()
        } else {
            layout.pack.name.clone()
        },
        from_version: installed.version.clone(),
        to_version_id: version.id.clone(),
        to_version: version.version_number.clone(),
        channel: version.channel.clone(),
        changelog,
        added,
        removed,
        changed,
        unchanged,
        overrides_changed,
        truncated,
        minecraft,
        loader: loader.into(),
    }
}

fn cleanup_pending(root: &Path) {
    for name in [PACK_PENDING, PACK_PENDING_FILE] {
        if let Ok(path) = storage::child(root, name) {
            let _ = fs::remove_file(path);
        }
    }
}

fn read_rollback(root: &Path) -> Option<PackRollback> {
    let path = storage::child(root, PACK_ROLLBACK).ok()?;
    storage::no_links(&path).ok()?;
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn remove_backup_dir(root: &Path, relative: &str) {
    let Ok(backups) = safe_join(root, PACK_BACKUP_DIR) else {
        return;
    };
    let Ok(dir) = safe_join(root, relative) else {
        return;
    };
    if dir.starts_with(&backups) && dir != backups {
        let _ = fs::remove_dir_all(dir);
    }
}

fn copy_atomic(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
        storage::no_links(parent)?;
    }
    storage::no_links(dest)?;
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let part = dest.with_file_name(format!(".{name}.{}.part", uuid::Uuid::new_v4()));
    storage::no_links(&part)?;
    if let Err(e) = fs::copy(src, &part) {
        let _ = fs::remove_file(&part);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(&part, dest) {
        let _ = fs::remove_file(&part);
        return Err(e.into());
    }
    Ok(())
}

fn restore_backup(root: &Path, dir: &Path, backup: &PackBackup) -> Result<()> {
    for path in &backup.added {
        let dest = safe_join(root, path)?;
        if dest.is_file() {
            fs::remove_file(&dest)?;
        }
    }
    let files = safe_join(dir, "files")?;
    for path in &backup.restore {
        let src = safe_join(&files, path)?;
        if src.is_file() {
            copy_atomic(&src, &safe_join(root, path)?)?;
        }
    }
    save_installed_pack(root, &backup.previous)
}

fn read_pack_backup(root: &Path, marker: &PackRollback) -> Result<(PathBuf, PackBackup)> {
    let backups = safe_join(root, PACK_BACKUP_DIR)?;
    let dir = safe_join(root, &marker.backup)?;
    if dir == backups || !dir.starts_with(&backups) {
        return Err(AppError::user(
            "The modpack rollback backup path is invalid.",
        ));
    }
    storage::no_links(&dir)?;
    let bytes = fs::read(dir.join("backup.json"))
        .map_err(|_| AppError::user("The modpack rollback backup is missing."))?;
    let backup: PackBackup = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::new("The modpack rollback backup is unreadable.", e))?;
    Ok((dir, backup))
}

/// Fetch the newest compatible pack version, stage and verify its archive, and
/// compute the file-level difference against the installed record. Nothing in
/// the server directory is changed by a preview.
pub async fn preview_update(m: Arc<Manager>, id: String) -> Result<Option<ModpackUpdatePreview>> {
    m.ready()?;
    let runtime = m.runtime(&id)?;
    let _guard = runtime.gate.lock().await;
    runtime.idle()?;
    let server = m.metadata(&id)?;
    if !server.installed {
        return Err(AppError::user(
            "Install the server software first, then apply a modpack.",
        ));
    }
    let root = m.dir(&id)?;
    let installed = read_installed_pack(&root)
        .ok_or_else(|| AppError::user("Install a Modrinth modpack before checking for updates."))?;
    cleanup_pending(&root);
    let client = m.client();
    let loader = (installed.loader == "fabric").then_some("fabric");
    let available = versions(
        &client,
        &installed.project_id,
        Some(&server.minecraft_version),
        loader,
    )
    .await?;
    let installed_date = version_detail(&client, &installed.version_id)
        .await
        .ok()
        .map(|(_, _, date)| date)
        .unwrap_or_default();
    let mut candidates: Vec<ProjectVersion> = available
        .into_iter()
        .filter(|v| v.id != installed.version_id)
        .filter(|v| pack_version_matches_loader(v, &installed.loader))
        .filter(|v| installed_date.is_empty() || v.date_published > installed_date)
        .filter(|v| {
            v.files
                .iter()
                .any(|f| f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        })
        .collect();
    candidates.sort_by(|a, b| {
        let a_release = a.channel == "release";
        let b_release = b.channel == "release";
        b_release
            .cmp(&a_release)
            .then_with(|| b.date_published.cmp(&a.date_published))
    });
    let Some(version) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let file = version
        .files
        .iter()
        .find(|f| f.primary && f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        .or_else(|| {
            version
                .files
                .iter()
                .find(|f| f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        })
        .ok_or_else(|| AppError::user("This modpack version has no .mrpack file."))?;
    if file.size > MAX_PACK_BYTES {
        return Err(AppError::user(
            "This modpack is larger than the 512 MB safety limit.",
        ));
    }
    let changelog = version_detail(&client, &version.id)
        .await
        .map(|(_, changelog, _)| changelog.chars().take(20_000).collect::<String>())
        .unwrap_or_default();
    let staged = storage::child(&root, PACK_PENDING_FILE)?;
    storage::no_links(&staged)?;
    let _ = fs::remove_file(&staged);
    progress(
        &m,
        &id,
        "download",
        &file.filename,
        0,
        1,
        0,
        (file.size > 0).then_some(file.size),
    );
    let downloaded = download_file(
        &client,
        &file.url,
        &staged,
        file.sha512.as_deref(),
        file.sha1.as_deref(),
        MAX_PACK_BYTES,
        |d, t| progress(&m, &id, "download", &file.filename, 0, 1, d, t),
    )
    .await;
    if let Err(e) = downloaded {
        let _ = fs::remove_file(&staged);
        return Err(e);
    }
    let result = (|| -> Result<ModpackUpdatePreview> {
        let layout = pack_layout(&staged)?;
        let (pack_loader, minecraft) = validate_pack(&server, &layout.pack)?;
        if pack_loader != installed.loader {
            return Err(AppError::user(
                "This modpack update changes the mod loader. Install it on a new server instead of updating in place.",
            ));
        }
        let preview = diff_pack(
            &installed,
            &layout,
            &version,
            changelog,
            minecraft,
            pack_loader,
        );
        let pending = PendingPack {
            version_id: version.id.clone(),
            version: version.version_number.clone(),
            channel: version.channel.clone(),
            changelog: preview.changelog.clone(),
            sha1: file.sha1.clone(),
            sha512: file.sha512.clone(),
        };
        storage::write_json(&storage::child(&root, PACK_PENDING)?, &pending)?;
        Ok(preview)
    })();
    if result.is_err() {
        cleanup_pending(&root);
    }
    result.map(Some)
}

/// Download only the files a pack version actually needs. Files whose hashes
/// match both the previous record and the bytes on disk are reused untouched.
async fn apply_layout(
    m: &Manager,
    id: &str,
    root: &Path,
    archive_path: &Path,
    layout: &PackLayout,
    client: &reqwest::Client,
    previous: Option<&InstalledPack>,
) -> Result<(u32, u32, u32)> {
    let total = layout.managed.len() as u32;
    let mut completed = 0u32;
    let mut files = 0u32;
    let mut reused = 0u32;
    for file in &layout.managed {
        let url = file
            .downloads
            .iter()
            .find(|u| u.starts_with("https://"))
            .ok_or_else(|| {
                AppError::user("The modpack contains a file without a secure download.")
            })?;
        let dest = safe_join(root, &file.path)?;
        let can_reuse = previous
            .and_then(|old| {
                old.managed
                    .iter()
                    .find(|entry| entry.path.eq_ignore_ascii_case(&file.path))
            })
            .map(|old| {
                hash_pair_match(
                    file.hashes.sha1.as_deref(),
                    file.hashes.sha512.as_deref(),
                    old.sha1.as_deref(),
                    old.sha512.as_deref(),
                ) && file_matches_hashes(
                    &dest,
                    file.hashes.sha1.as_deref(),
                    file.hashes.sha512.as_deref(),
                )
            })
            .unwrap_or(false);
        if can_reuse {
            reused += 1;
            completed += 1;
            progress(m, id, "reuse", &file.path, completed, total, 0, Some(0));
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        progress(
            m,
            id,
            "download",
            &file.path,
            completed,
            total,
            0,
            Some(file.file_size),
        );
        download_file(
            client,
            url,
            &dest,
            file.hashes.sha512.as_deref(),
            file.hashes.sha1.as_deref(),
            MAX_PACK_FILE_BYTES,
            |d, t| progress(m, id, "download", &file.path, completed, total, d, t),
        )
        .await?;
        completed += 1;
        files += 1;
    }
    let total_overrides = layout.overrides.len() as u32;
    if total_overrides > 0 {
        let handle = fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(handle)
            .map_err(|e| AppError::new("This file is not a valid .mrpack archive.", e))?;
        let mut done = 0u32;
        for file in &layout.overrides {
            let dest = safe_join(root, &file.path)?;
            let can_reuse = previous
                .and_then(|old| {
                    old.overrides
                        .iter()
                        .find(|entry| entry.path.eq_ignore_ascii_case(&file.path))
                })
                .map(|old| {
                    hash_pair_match(
                        Some(&file.sha1),
                        None,
                        old.sha1.as_deref(),
                        old.sha512.as_deref(),
                    ) && file_matches_hashes(&dest, Some(&file.sha1), None)
                })
                .unwrap_or(false);
            if !can_reuse {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut entry = archive.by_name(&file.entry).map_err(|e| {
                    AppError::new("The staged modpack changed while it was being read.", e)
                })?;
                let name = dest
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("override");
                let part = dest.with_file_name(format!(".{name}.{}.part", uuid::Uuid::new_v4()));
                storage::no_links(&part)?;
                let mut out = fs::File::create(&part)?;
                let copied =
                    std::io::copy(&mut entry.by_ref().take(MAX_OVERRIDE_BYTES + 1), &mut out)?;
                if copied > MAX_OVERRIDE_BYTES {
                    drop(out);
                    let _ = fs::remove_file(&part);
                    return Err(AppError::user(
                        "The modpack overrides are larger than the 512 MB safety limit.",
                    ));
                }
                out.sync_all()?;
                drop(out);
                fs::rename(&part, &dest)?;
            }
            done += 1;
            progress(
                m,
                id,
                "overrides",
                &file.path,
                done,
                total_overrides,
                0,
                Some(0),
            );
        }
    }
    Ok((files, reused, total_overrides))
}

/// Apply the staged update. Files that the update replaces or removes are
/// copied to a per-update backup first; a failure restores them immediately.
pub async fn update_modpack(
    m: Arc<Manager>,
    id: String,
    confirmed: bool,
) -> Result<ModpackUpdateOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm the modpack update before applying it.",
        ));
    }
    m.ready()?;
    let runtime = m.runtime(&id)?;
    let _guard = runtime.gate.lock().await;
    runtime.idle()?;
    let server = m.metadata(&id)?;
    if !server.installed {
        return Err(AppError::user(
            "Install the server software first, then apply a modpack.",
        ));
    }
    let root = m.dir(&id)?;
    let installed = read_installed_pack(&root)
        .ok_or_else(|| AppError::user("Install a Modrinth modpack before checking for updates."))?;
    let pending_path = storage::child(&root, PACK_PENDING)?;
    storage::no_links(&pending_path)?;
    let pending: PendingPack = serde_json::from_slice(
        &fs::read(&pending_path)
            .map_err(|_| AppError::user("Check for a modpack update before applying one."))?,
    )
    .map_err(|e| AppError::new("The staged modpack update is unreadable. Check again.", e))?;
    if pending.version_id == installed.version_id {
        cleanup_pending(&root);
        return Err(AppError::user(
            "The staged update is the version already installed.",
        ));
    }
    let staged = storage::child(&root, PACK_PENDING_FILE)?;
    storage::no_links(&staged)?;
    if !file_matches_hashes(&staged, pending.sha1.as_deref(), pending.sha512.as_deref()) {
        cleanup_pending(&root);
        return Err(AppError::user(
            "The staged modpack download is missing or does not match its checksum. Check for updates again.",
        ));
    }
    let layout = pack_layout(&staged)?;
    let (loader, minecraft) = validate_pack(&server, &layout.pack)?;
    if loader != installed.loader {
        cleanup_pending(&root);
        return Err(AppError::user(
            "This modpack update changes the mod loader. Install it on a new server instead of updating in place.",
        ));
    }
    let old = pack_hashes(&installed);
    let new = layout_hashes(&layout);
    let backup_rel = format!("{PACK_BACKUP_DIR}/{}", uuid::Uuid::new_v4());
    let backup_dir = safe_join(&root, &backup_rel)?;
    fs::create_dir_all(&backup_dir)?;
    let prepared = (|| -> Result<PackBackup> {
        let mut restore = Vec::new();
        let mut added = Vec::new();
        for (key, (path, sha1, sha512)) in &old {
            let same = new
                .get(key)
                .map(|(_, one, five)| {
                    hash_pair_match(
                        one.as_deref(),
                        five.as_deref(),
                        sha1.as_deref(),
                        sha512.as_deref(),
                    )
                })
                .unwrap_or(false);
            if same {
                continue;
            }
            let src = safe_join(&root, path)?;
            if src.is_file() {
                copy_atomic(&src, &safe_join(&backup_dir.join("files"), path)?)?;
                restore.push(path.clone());
            }
        }
        for (key, (path, _, _)) in &new {
            if old.contains_key(key) {
                continue;
            }
            // A manually placed file can share a path with a file the update
            // introduces. Preserve it so a rollback restores it instead of
            // deleting it as an update-added file.
            let dest = safe_join(&root, path)?;
            if dest.is_file() {
                copy_atomic(&dest, &safe_join(&backup_dir.join("files"), path)?)?;
                restore.push(path.clone());
            } else {
                added.push(path.clone());
            }
        }
        let backup = PackBackup {
            previous: installed.clone(),
            restore,
            added,
        };
        storage::write_json(&backup_dir.join("backup.json"), &backup)?;
        Ok(backup)
    })();
    let backup = match prepared {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_dir_all(&backup_dir);
            return Err(error);
        }
    };
    let previous_marker = read_rollback(&root);
    let client = m.client();
    let apply = async {
        for (key, (path, _, _)) in &old {
            if !new.contains_key(key) {
                let dest = safe_join(&root, path)?;
                if dest.is_file() {
                    fs::remove_file(&dest)?;
                }
            }
        }
        let (files, reused, overrides) =
            apply_layout(&m, &id, &root, &staged, &layout, &client, Some(&installed)).await?;
        let record = installed_record(
            &installed.project_id,
            &pending.version_id,
            &pending.version,
            &layout.pack.name,
            minecraft,
            loader,
            &layout,
        );
        save_installed_pack(&root, &record)?;
        Ok::<(u32, u32, u32), AppError>((files, reused, overrides))
    }
    .await;
    let (files, reused, overrides) = match apply {
        Ok(values) => values,
        Err(error) => {
            if let Err(restore_error) = restore_backup(&root, &backup_dir, &backup) {
                return Err(AppError::user(format!(
                    "The modpack update failed and the automatic rollback also failed. The backup is kept at {backup_rel}. Update error: {error}. Rollback error: {restore_error}"
                )));
            }
            let _ = fs::remove_dir_all(&backup_dir);
            cleanup_pending(&root);
            return Err(error);
        }
    };
    let marker = PackRollback {
        backup: backup_rel.clone(),
        from_version: installed.version.clone(),
        to_version: pending.version.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(error) = storage::write_json(&storage::child(&root, PACK_ROLLBACK)?, &marker) {
        return Err(AppError::user(format!(
            "The modpack update was applied, but its rollback marker could not be saved. The backup is kept at {backup_rel}. {error}"
        )));
    }
    if let Some(previous) = previous_marker {
        remove_backup_dir(&root, &previous.backup);
    }
    cleanup_pending(&root);
    m.logger.log(
        "INFO",
        &format!(
            "Updated modpack {} {} -> {} on server {id}",
            installed.name, installed.version, pending.version
        ),
    );
    Ok(ModpackUpdateOutcome {
        name: installed.name,
        from_version: installed.version,
        to_version: pending.version,
        files,
        reused,
        skipped: layout.skipped,
        overrides,
        backup: backup_rel,
    })
}

/// Restore the single most recent modpack update. The marker and its backup
/// are removed afterwards, so an update is rolled back at most once.
pub async fn rollback_modpack(
    m: Arc<Manager>,
    id: String,
    confirmed: bool,
) -> Result<ModpackRollbackOutcome> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm the modpack rollback before applying it.",
        ));
    }
    m.ready()?;
    let runtime = m.runtime(&id)?;
    let _guard = runtime.gate.lock().await;
    runtime.idle()?;
    let root = m.dir(&id)?;
    let marker = read_rollback(&root)
        .ok_or_else(|| AppError::user("There is no modpack update to roll back."))?;
    let (dir, backup) = read_pack_backup(&root, &marker)?;
    restore_backup(&root, &dir, &backup)?;
    let _ = fs::remove_file(storage::child(&root, PACK_ROLLBACK)?);
    let _ = fs::remove_dir_all(&dir);
    cleanup_pending(&root);
    m.logger.log(
        "INFO",
        &format!(
            "Rolled back modpack {} {} -> {} on server {id}",
            backup.previous.name, marker.to_version, backup.previous.version
        ),
    );
    Ok(ModpackRollbackOutcome {
        name: backup.previous.name,
        restored_version: backup.previous.version,
    })
}

fn facets(project_type: &str, game_version: Option<&str>, loader: Option<&str>) -> String {
    let mut groups: Vec<Vec<String>> = vec![vec![format!("project_type:{project_type}")]];
    if let Some(game) = game_version.filter(|g| !g.is_empty()) {
        groups.push(vec![format!("versions:{game}")]);
    }
    if let Some(loader) = loader.filter(|l| !l.is_empty()) {
        groups.push(vec![format!("categories:{loader}")]);
    }
    if project_type != "modpack" {
        groups.push(vec![
            "server_side:required".into(),
            "server_side:optional".into(),
        ]);
    }
    serde_json::to_string(&groups).unwrap_or_else(|_| "[]".into())
}

pub(crate) fn validate_project_id(project_id: &str) -> Result<()> {
    if project_id.is_empty()
        || project_id.len() > 64
        || !project_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AppError::user("Invalid Modrinth project identifier."));
    }
    Ok(())
}

fn server_name(software: ServerType) -> &'static str {
    software.label()
}

pub(crate) fn folder_for(server: &Server) -> Result<&'static str> {
    server.server_type.content_folder().ok_or_else(|| {
        AppError::user(
            "Vanilla servers do not load mods or plugins. Create a Paper, Purpur or Fabric server.",
        )
    })
}

fn target(server: &Server, kind: &str) -> Result<(&'static str, &'static str)> {
    let folder = folder_for(server)?;
    let expected = if folder == "plugins" { "plugin" } else { "mod" };
    if kind != expected {
        return Err(AppError::user(format!(
            "{} servers accept {} files, not {} files.",
            server_name(server.server_type),
            expected,
            kind
        )));
    }
    Ok((folder, folder_for_loader(server.server_type)))
}

fn folder_for_loader(software: ServerType) -> &'static str {
    software.content_loader().unwrap_or("fabric")
}

pub(crate) fn compatible(version: &ProjectVersion, loader: &str) -> bool {
    match loader {
        "paper" => version.loaders.iter().any(|l| {
            l.eq_ignore_ascii_case("paper")
                || l.eq_ignore_ascii_case("spigot")
                || l.eq_ignore_ascii_case("bukkit")
                || l.eq_ignore_ascii_case("purpur")
        }),
        _ => version
            .loaders
            .iter()
            .any(|l| l.eq_ignore_ascii_case(loader)),
    }
}

fn server_supported(file: &PackFile) -> bool {
    file.env
        .as_ref()
        .and_then(|e| e.server.as_deref())
        .map(|s| s != "unsupported")
        .unwrap_or(true)
}

fn detect_loader(dependencies: &BTreeMap<String, String>) -> Result<&'static str> {
    if dependencies.contains_key("fabric-loader") {
        Ok("fabric")
    } else if dependencies.contains_key("quilt-loader") {
        Err(AppError::user(
            "Quilt modpacks are not supported yet. Use the official Quilt installer for now.",
        ))
    } else if dependencies.contains_key("neoforge") {
        Err(AppError::user(
            "NeoForge modpacks are not supported yet. EasyLauncher currently applies Fabric and Vanilla modpacks.",
        ))
    } else if dependencies.contains_key("forge") {
        Err(AppError::user(
            "Forge modpacks are not supported yet. EasyLauncher currently applies Fabric and Vanilla modpacks.",
        ))
    } else {
        Ok("vanilla")
    }
}

fn protected(relative: &str) -> bool {
    let first = relative
        .replace('\\', "/")
        .split('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        first.as_str(),
        "world"
            | "world_nether"
            | "world_the_end"
            | "logs"
            | "backups"
            | "server.jar"
            | "eula.txt"
            | "server.properties"
            | "content.json"
            | "modpack.json"
            | "modpack-pending.json"
            | "modpack-pending.mrpack"
            | "modpack-rollback.json"
            | "modpack-backups"
            | "player-stats.json"
            | "tunnel.json"
            | "minehost.json"
            | "usercache.json"
    )
}

pub(crate) fn safe_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.len() > 255 {
        return false;
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/')
    }) {
        return false;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    ![
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
}

fn safe_relative(relative: &str) -> Result<String> {
    if relative.is_empty()
        || relative.len() > 512
        || relative.starts_with('/')
        || relative.starts_with('\\')
    {
        return Err(AppError::user(
            "The modpack contains an unsafe file path and was rejected.",
        ));
    }
    let normalized = relative.replace('\\', "/");
    let mut depth = 0;
    for part in normalized.split('/') {
        if !safe_component(part) {
            return Err(AppError::user(
                "The modpack contains an unsafe file path and was rejected.",
            ));
        }
        depth += 1;
    }
    if depth == 0 {
        return Err(AppError::user(
            "The modpack contains an unsafe file path and was rejected.",
        ));
    }
    Ok(normalized)
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let normalized = safe_relative(relative)?;
    let mut path = root.to_path_buf();
    storage::no_links(&path)?;
    for part in normalized.split('/') {
        path.push(part);
        storage::no_links(&path)?;
    }
    Ok(path)
}

pub(crate) fn load_index(root: &Path) -> ContentIndex {
    let Ok(path) = storage::child(root, INDEX_FILE) else {
        return ContentIndex::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn save_index(root: &Path, index: &ContentIndex) -> Result<()> {
    storage::write_json(&storage::child(root, INDEX_FILE)?, index)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn progress(
    m: &Manager,
    id: &str,
    stage: &str,
    file: &str,
    completed: u32,
    total: u32,
    downloaded: u64,
    total_bytes: Option<u64>,
) {
    m.emit(
        "content-progress",
        ContentProgress {
            id: id.into(),
            stage: stage.into(),
            file: file.into(),
            completed,
            total,
            downloaded_bytes: downloaded,
            total_bytes,
        },
    );
}

pub(crate) async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha512: Option<&str>,
    sha1: Option<&str>,
    max: u64,
    progress: impl Fn(u64, Option<u64>),
) -> Result<u64> {
    if !url.starts_with("https://") {
        return Err(AppError::user("Only secure HTTPS downloads are allowed."));
    }
    storage::no_links(dest)?;
    if let Some(parent) = dest.parent() {
        storage::no_links(parent)?;
    }
    let name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let part = dest.with_file_name(format!("{name}.{}.part", uuid::Uuid::new_v4()));
    let mut last = AppError::user("The download failed. Please retry.");
    for attempt in 1..=3u32 {
        let result: Result<u64> = async {
            let response = client.get(url).send().await?.error_for_status()?;
            let total = response.content_length();
            if total.map(|n| n > max).unwrap_or(false) {
                return Err(AppError::user(
                    "This file is larger than the allowed safety limit.",
                ));
            }
            let mut h512 = Sha512::new();
            let mut h1 = sha1::Sha1::new();
            let mut file = tokio::fs::File::create(&part).await?;
            let mut stream = response.bytes_stream();
            let mut bytes = 0u64;
            let mut tick = Instant::now();
            loop {
                let chunk = tokio::time::timeout(Duration::from_secs(30), stream.next())
                    .await
                    .map_err(|_| AppError::user("The download stalled. Please retry."))?;
                let Some(chunk) = chunk else { break };
                let chunk = chunk?;
                bytes += chunk.len() as u64;
                if bytes > max {
                    return Err(AppError::user(
                        "This file is larger than the allowed safety limit.",
                    ));
                }
                h512.update(&chunk);
                h1.update(&chunk);
                file.write_all(&chunk).await?;
                if tick.elapsed() > Duration::from_millis(150) {
                    progress(bytes, total);
                    tick = Instant::now();
                }
            }
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            if bytes == 0 {
                return Err(AppError::user(
                    "The downloaded file was empty. Please retry.",
                ));
            }
            if let Some(expected) = sha512 {
                if !format!("{:x}", h512.finalize()).eq_ignore_ascii_case(expected) {
                    return Err(AppError::user(
                        "Download checksum verification failed. The file was discarded.",
                    ));
                }
            }
            if let Some(expected) = sha1 {
                if !format!("{:x}", h1.finalize()).eq_ignore_ascii_case(expected) {
                    return Err(AppError::user(
                        "Download checksum verification failed. The file was discarded.",
                    ));
                }
            }
            tokio::fs::rename(&part, dest).await?;
            progress(bytes, Some(bytes));
            Ok(bytes)
        }
        .await;
        match result {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                last = e;
                let _ = tokio::fs::remove_file(&part).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(400 * attempt as u64)).await;
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn file_names_are_restricted() {
        for name in [
            "",
            ".",
            "..",
            "../evil.jar",
            "mods/evil.jar",
            "mods\\evil.jar",
            "C:evil.jar",
            "evil.jar.",
            "evil.jar ",
            "CON.jar",
            "nul",
            "bad<name>.jar",
        ] {
            assert!(!safe_component(name), "should reject {name:?}");
        }
        for name in [
            "fabric-api-0.92.2+1.20.1.jar",
            "Sodium 0.5.11.jar",
            "server-icon (1).png",
            "data_1-2.jar",
        ] {
            assert!(safe_component(name), "should accept {name:?}");
        }
    }

    #[test]
    fn joining_rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for path in [
            "",
            "../escape.txt",
            "mods/../../escape.txt",
            "mods//escape.txt",
            "/absolute.txt",
            "C:/escape.txt",
            "mods/CON.jar",
        ] {
            assert!(safe_join(root, path).is_err(), "should reject {path:?}");
        }
        let ok = safe_join(root, "mods/config/settings.json").unwrap();
        assert_eq!(ok, root.join("mods").join("config").join("settings.json"));
    }

    #[test]
    fn facets_form_groups() {
        let value: Value =
            serde_json::from_str(&facets("mod", Some("1.20.1"), Some("fabric"))).unwrap();
        assert_eq!(value[0][0], "project_type:mod");
        assert_eq!(value[1][0], "versions:1.20.1");
        assert_eq!(value[2][0], "categories:fabric");
        assert_eq!(value[3][0], "server_side:required");
        assert_eq!(value[3][1], "server_side:optional");
        let pack: Value = serde_json::from_str(&facets("modpack", None, None)).unwrap();
        assert_eq!(pack.as_array().unwrap().len(), 1);
    }

    #[test]
    fn search_response_is_mapped() {
        let raw: RawSearch = serde_json::from_value(json!({
            "hits": [{
                "project_id": "AABBCCDD",
                "slug": "sodium",
                "title": "Sodium",
                "description": "Rendering engine",
                "author": "CaffeineMC",
                "icon_url": "",
                "downloads": 100,
                "follows": 5,
                "categories": ["optimization"],
                "client_side": "required",
                "server_side": "unsupported",
                "project_type": "mod"
            }, {"project_id": "", "slug": "broken"}],
            "total": 42, "offset": 0, "limit": 20
        }))
        .unwrap();
        let page = map_search(raw);
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].id, "AABBCCDD");
        assert!(page.hits[0].icon_url.is_none());
        assert_eq!(page.hits[0].server_side, "unsupported");
        assert_eq!(page.total, 42);
    }

    #[test]
    fn version_response_is_mapped() {
        let raw: Vec<RawVersion> = serde_json::from_value(json!([{
            "id": "ver1",
            "dependencies": [{"project_id":"library","version_id":"library-v1","dependency_type":"required"}],
            "name": "Fabric API 0.92",
            "version_number": "0.92.2+1.20.1",
            "version_type": "release",
            "game_versions": ["1.20.1"],
            "loaders": ["fabric"],
            "downloads": 10,
            "date_published": "2024-01-01T00:00:00Z",
            "files": [{
                "filename": "fabric-api.jar",
                "url": "https://cdn.modrinth.com/data/fabric-api.jar",
                "size": 123,
                "hashes": {"sha1": "aa", "sha512": "bb"},
                "primary": true
            }, {"filename": "sources.jar", "url": "", "size": 1, "primary": false}]
        }]))
        .unwrap();
        let version = &map_versions(raw)[0];
        assert_eq!(version.channel, "release");
        assert_eq!(version.loaders, vec!["fabric"]);
        assert_eq!(version.files[0].sha512.as_deref(), Some("bb"));
        assert_eq!(version.files.len(), 1);
        assert_eq!(
            version.dependencies[0].project_id.as_deref(),
            Some("library")
        );
        let serialized = serde_json::to_value(&version.dependencies[0]).unwrap();
        assert_eq!(serialized["dependencyType"], "required");
        assert_eq!(serialized["versionId"], "library-v1");
    }

    #[test]
    fn pack_loader_detection() {
        let fabric = BTreeMap::from([
            ("minecraft".into(), "1.20.1".into()),
            ("fabric-loader".into(), "0.15.0".into()),
        ]);
        assert_eq!(detect_loader(&fabric).unwrap(), "fabric");
        let vanilla = BTreeMap::from([("minecraft".into(), "1.20.1".into())]);
        assert_eq!(detect_loader(&vanilla).unwrap(), "vanilla");
        for unsupported in ["forge", "neoforge", "quilt-loader"] {
            let deps = BTreeMap::from([(unsupported.into(), "1".into())]);
            assert!(
                detect_loader(&deps).is_err(),
                "{unsupported} should be rejected"
            );
        }
    }

    #[test]
    fn pack_entries_respect_server_env() {
        let pack: PackIndex = serde_json::from_value(json!({
            "formatVersion": 1,
            "name": "Fixture",
            "versionId": "1.0.0",
            "dependencies": {"minecraft": "1.20.1", "fabric-loader": "0.15.0"},
            "files": [
                {"path": "mods/a.jar", "hashes": {"sha1": "aa"}, "downloads": ["https://cdn.example/a.jar"], "fileSize": 10},
                {"path": "mods/client-only.jar", "hashes": {}, "env": {"client": "required", "server": "unsupported"}, "downloads": ["https://cdn.example/b.jar"], "fileSize": 10}
            ]
        })).unwrap();
        assert_eq!(pack.format_version, 1);
        assert!(server_supported(&pack.files[0]));
        assert!(!server_supported(&pack.files[1]));
        assert_eq!(pack.files[0].hashes.sha1.as_deref(), Some("aa"));
    }

    #[test]
    fn protected_paths() {
        assert!(protected("player-stats.json"));
        assert!(protected("tunnel.json"));
        assert!(protected("server.jar"));
        assert!(protected("server.properties"));
        assert!(protected("world/level.dat"));
        assert!(protected("backups/2024.zip"));
        assert!(protected("logs/latest.log"));
        assert!(protected("modpack.json"));
        assert!(protected("modpack-pending.mrpack"));
        assert!(protected("modpack-backups/abc/files/mod.jar"));
        assert!(protected("modpack-rollback.json"));
        assert!(!protected("mods/fabric-api.jar"));
        assert!(!protected("config/settings.json"));
    }

    fn test_pack(dir: &Path, index: Value, overrides: &[(&str, &str)]) -> PathBuf {
        use std::io::Write as _;
        let path = dir.join("fixture.mrpack");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file(
            "modrinth.index.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(index.to_string().as_bytes()).unwrap();
        for (name, body) in overrides {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn sha1_hex(body: &str) -> String {
        let mut hash = sha1::Sha1::new();
        hash.update(body.as_bytes());
        format!("{:x}", hash.finalize())
    }

    #[test]
    fn pack_layout_filters_managed_files_and_merges_overrides() {
        let temp = tempfile::tempdir().unwrap();
        let index = json!({
            "formatVersion": 1,
            "name": "Fixture Pack",
            "versionId": "1.1.0",
            "dependencies": {"minecraft": "1.20.1", "fabric-loader": "0.15.0"},
            "files": [
                {"path": "mods/a.jar", "hashes": {"sha1": "aa"}, "downloads": ["https://cdn.example/a.jar"], "fileSize": 10},
                {"path": "mods/client.jar", "hashes": {}, "env": {"server": "unsupported"}, "downloads": ["https://cdn.example/c.jar"], "fileSize": 10},
                {"path": "mods/override.jar", "hashes": {"sha1": "bb"}, "downloads": ["https://cdn.example/o.jar"], "fileSize": 10},
                {"path": "config/from-managed.json", "hashes": {"sha1": "cc"}, "downloads": ["https://cdn.example/m.json"], "fileSize": 10},
                {"path": "world/level.dat", "hashes": {"sha1": "dd"}, "downloads": ["https://cdn.example/w.dat"], "fileSize": 10}
            ]
        });
        let path = test_pack(
            temp.path(),
            index,
            &[
                ("overrides/config/settings.json", "base"),
                ("overrides/mods/override.jar", "override"),
                ("server-overrides/config/settings.json", "server"),
                ("server-overrides/mods/override.jar", "server-override"),
                ("overrides/logs/latest.log", "ignored"),
            ],
        );
        let layout = pack_layout(&path).unwrap();
        assert_eq!(layout.pack.name, "Fixture Pack");
        assert_eq!(layout.managed.len(), 2);
        let managed: Vec<&str> = layout.managed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(managed, vec!["config/from-managed.json", "mods/a.jar"]);
        assert_eq!(layout.skipped, 4);
        let overrides: BTreeMap<&str, &str> = layout
            .overrides
            .iter()
            .map(|f| (f.path.as_str(), f.sha1.as_str()))
            .collect();
        assert_eq!(overrides.len(), 2);
        let server = sha1_hex("server");
        let server_override = sha1_hex("server-override");
        assert_eq!(
            overrides.get("config/settings.json").copied(),
            Some(server.as_str())
        );
        assert_eq!(
            overrides.get("mods/override.jar").copied(),
            Some(server_override.as_str())
        );
    }

    #[test]
    fn pack_diff_classifies_paths() {
        let installed = InstalledPack {
            schema_version: 1,
            project_id: "pack".into(),
            version_id: "old".into(),
            name: "Pack".into(),
            version: "1.0".into(),
            minecraft: "1.20.1".into(),
            loader: "fabric".into(),
            managed: vec![
                InstalledPackFile {
                    path: "mods/a.jar".into(),
                    sha1: Some("aa".into()),
                    sha512: None,
                },
                InstalledPackFile {
                    path: "mods/b.jar".into(),
                    sha1: Some("bb".into()),
                    sha512: None,
                },
                InstalledPackFile {
                    path: "mods/old.jar".into(),
                    sha1: Some("oo".into()),
                    sha512: None,
                },
            ],
            overrides: vec![InstalledPackFile {
                path: "config/settings.json".into(),
                sha1: Some("x".into()),
                sha512: None,
            }],
            installed_at: "2026-01-01T00:00:00Z".into(),
        };
        let version = ProjectVersion {
            id: "new".into(),
            project_id: "pack".into(),
            name: "1.1".into(),
            version_number: "1.1".into(),
            channel: "release".into(),
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
            downloads: 0,
            date_published: "2026-02-01T00:00:00Z".into(),
            files: vec![],
            dependencies: vec![],
        };
        let layout = PackLayout {
            pack: serde_json::from_value(json!({
                "formatVersion": 1,
                "name": "Pack",
                "versionId": "1.1",
                "dependencies": {"minecraft": "1.20.1", "fabric-loader": "0.15.0"}
            }))
            .unwrap(),
            managed: vec![
                serde_json::from_value(json!({"path": "mods/a.jar", "hashes": {"sha1": "aa"}, "downloads": ["https://cdn.example/a.jar"]})).unwrap(),
                serde_json::from_value(json!({"path": "mods/b.jar", "hashes": {"sha1": "bb-new"}, "downloads": ["https://cdn.example/b.jar"]})).unwrap(),
                serde_json::from_value(json!({"path": "mods/c.jar", "hashes": {"sha1": "cc"}, "downloads": ["https://cdn.example/c.jar"]})).unwrap(),
            ],
            overrides: vec![
                PackOverride {
                    path: "config/settings.json".into(),
                    entry: "server-overrides/config/settings.json".into(),
                    sha1: "y".into(),
                },
                PackOverride {
                    path: "config/new.json".into(),
                    entry: "server-overrides/config/new.json".into(),
                    sha1: "z".into(),
                },
            ],
            skipped: 0,
        };
        let preview = diff_pack(
            &installed,
            &layout,
            &version,
            String::new(),
            "1.20.1".into(),
            "fabric",
        );
        assert_eq!(preview.added, vec!["config/new.json", "mods/c.jar"]);
        assert_eq!(preview.removed, vec!["mods/old.jar"]);
        assert_eq!(preview.changed, vec!["config/settings.json", "mods/b.jar"]);
        assert_eq!(preview.unchanged, 1);
        assert_eq!(preview.overrides_changed, 2);
        assert!(!preview.truncated);
        assert_eq!(preview.from_version, "1.0");
        assert_eq!(preview.to_version, "1.1");
    }

    #[test]
    fn rollback_restores_files_and_record() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/a.jar"), b"new").unwrap();
        fs::write(root.join("mods/added.jar"), b"added").unwrap();
        fs::create_dir_all(root.join("modpack-backups/one/files/mods")).unwrap();
        fs::write(root.join("modpack-backups/one/files/mods/a.jar"), b"old").unwrap();
        let previous = InstalledPack {
            schema_version: 1,
            project_id: "pack".into(),
            version_id: "old".into(),
            name: "Pack".into(),
            version: "1.0".into(),
            minecraft: "1.20.1".into(),
            loader: "fabric".into(),
            managed: vec![],
            overrides: vec![],
            installed_at: "2026-01-01T00:00:00Z".into(),
        };
        let backup = PackBackup {
            previous,
            restore: vec!["mods/a.jar".into()],
            added: vec!["mods/added.jar".into()],
        };
        restore_backup(root, &root.join("modpack-backups/one"), &backup).unwrap();
        assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"old");
        assert!(!root.join("mods/added.jar").exists());
        let record = read_installed_pack(root).unwrap();
        assert_eq!(record.version, "1.0");
    }

    #[test]
    fn pack_candidates_follow_the_installed_loader() {
        let version = |loaders: Vec<&str>| ProjectVersion {
            id: "v".into(),
            project_id: "pack".into(),
            name: "v".into(),
            version_number: "v".into(),
            channel: "release".into(),
            game_versions: vec!["1.20.1".into()],
            loaders: loaders.into_iter().map(Into::into).collect(),
            downloads: 0,
            date_published: String::new(),
            files: vec![],
            dependencies: vec![],
        };
        assert!(pack_version_matches_loader(
            &version(vec!["fabric"]),
            "fabric"
        ));
        assert!(pack_version_matches_loader(&version(vec![]), "fabric"));
        assert!(!pack_version_matches_loader(
            &version(vec!["forge"]),
            "fabric"
        ));
        assert!(pack_version_matches_loader(
            &version(vec!["minecraft"]),
            "vanilla"
        ));
        assert!(pack_version_matches_loader(&version(vec![]), "vanilla"));
        assert!(!pack_version_matches_loader(
            &version(vec!["neoforge"]),
            "vanilla"
        ));
    }

    #[test]
    fn local_file_hash_matching_is_strict() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mod.jar");
        fs::write(&path, b"payload").unwrap();
        let one = sha1_hex("payload");
        assert!(file_matches_hashes(&path, Some(&one), None));
        assert!(!file_matches_hashes(&path, None, None));
        assert!(!file_matches_hashes(&path, Some("deadbeef"), None));
        assert!(hash_pair_match(
            Some(&one),
            None,
            Some(&one.to_uppercase()),
            None
        ));
        assert!(!hash_pair_match(None, None, Some(&one), None));
    }

    #[test]
    fn project_ids_are_restricted() {
        let long = "a".repeat(65);
        for id in ["", "../x", "a/b", "a?b", "a b", long.as_str()] {
            assert!(validate_project_id(id).is_err(), "should reject {id:?}");
        }
        for id in ["AABBCCDD", "fabric-api", "sodium_2"] {
            assert!(validate_project_id(id).is_ok(), "should accept {id:?}");
        }
    }
}
