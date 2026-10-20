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
const MAX_CONTENT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OVERRIDE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
const INDEX_FILE: &str = "content.json";

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
pub struct ContentListing {
    pub entries: Vec<ContentEntry>,
    pub unmanaged: Vec<String>,
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
    pub skipped: u32,
    pub overrides: u32,
    pub minecraft: String,
    pub loader: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentIndex {
    #[serde(default)]
    entries: Vec<ContentEntry>,
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
    let folder = match server.server_type {
        ServerType::Vanilla => {
            return Ok(ContentListing {
                entries: Vec::new(),
                unmanaged: Vec::new(),
            })
        }
        ServerType::Paper => "plugins",
        ServerType::Fabric => "mods",
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
    Ok(ContentListing {
        entries: managed,
        unmanaged,
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
    if server.server_type == ServerType::Paper {
        return Err(AppError::user(
            "Paper servers use plugins. Create a Fabric server to install modpacks.",
        ));
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
    let root = m.dir(&id)?;
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
    let result = apply_pack(&m, &id, &server, &root, &temp, &client).await;
    let _ = fs::remove_file(&temp);
    result
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
        (ServerType::Paper, _) => {
            return Err(AppError::user(
                "Paper servers cannot run modpacks. Use Fabric or Vanilla.",
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

fn validate_project_id(project_id: &str) -> Result<()> {
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
    match software {
        ServerType::Vanilla => "Vanilla",
        ServerType::Paper => "Paper",
        ServerType::Fabric => "Fabric",
    }
}

fn folder_for(server: &Server) -> Result<&'static str> {
    match server.server_type {
        ServerType::Paper => Ok("plugins"),
        ServerType::Fabric => Ok("mods"),
        ServerType::Vanilla => Err(AppError::user(
            "Vanilla servers do not load mods or plugins. Create a Paper or Fabric server.",
        )),
    }
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
    match software {
        ServerType::Paper => "paper",
        _ => "fabric",
    }
}

fn compatible(version: &ProjectVersion, loader: &str) -> bool {
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
            | "player-stats.json"
            | "tunnel.json"
            | "minehost.json"
            | "usercache.json"
    )
}

fn safe_component(name: &str) -> bool {
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

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.len() > 512
        || relative.starts_with('/')
        || relative.starts_with('\\')
    {
        return Err(AppError::user(
            "The modpack contains an unsafe file path and was rejected.",
        ));
    }
    let mut path = root.to_path_buf();
    storage::no_links(&path)?;
    let normalized = relative.replace('\\', "/");
    let mut depth = 0;
    for part in normalized.split('/') {
        if !safe_component(part) {
            return Err(AppError::user(
                "The modpack contains an unsafe file path and was rejected.",
            ));
        }
        path.push(part);
        depth += 1;
        storage::no_links(&path)?;
    }
    if depth == 0 {
        return Err(AppError::user(
            "The modpack contains an unsafe file path and was rejected.",
        ));
    }
    Ok(path)
}

fn load_index(root: &Path) -> ContentIndex {
    let Ok(path) = storage::child(root, INDEX_FILE) else {
        return ContentIndex::default();
    };
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_index(root: &Path, index: &ContentIndex) -> Result<()> {
    storage::write_json(&storage::child(root, INDEX_FILE)?, index)
}

#[allow(clippy::too_many_arguments)]
fn progress(
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

async fn download_file(
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
        assert!(!protected("mods/fabric-api.jar"));
        assert!(!protected("config/settings.json"));
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
