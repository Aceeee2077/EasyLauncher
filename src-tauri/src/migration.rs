//! Portable server archives for moving a server between machines.
//!
//! Export writes a ZIP with a small JSON manifest at the root followed by the
//! server folder. App-only state (logs, backups unless requested, schedules,
//! analytics, rollback pointers, tunnels and remote backup staging) is left
//! behind. Import validates every entry name, refuses links and extraction
//! beyond fixed limits, then re-applies the same safety repairs as folder
//! import. Archives are integrity-checked by the ZIP CRC but are not
//! authenticated; only import archives you created or trust.

use crate::{
    error::{AppError, Result},
    import,
    models::{Server, ServerType, Status},
    server::manager::{memory_info, validate_memory, Manager, Runtime},
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub const MANIFEST: &str = "easylauncher-export.json";
const FORMAT: &str = "easylauncher-server";
const SCHEMA: u32 = 1;
const MAX_ENTRIES: u64 = 250_000;
const MAX_UNCOMPRESSED: u64 = 64 * 1024 * 1024 * 1024;
const MAX_SINGLE: u64 = 16 * 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 512 * 1024;
const MAX_PATH: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestServer {
    pub name: String,
    pub minecraft_version: String,
    pub server_type: ServerType,
    #[serde(default = "default_min")]
    pub memory_min_mb: u32,
    #[serde(default = "default_max")]
    pub memory_max_mb: u32,
    #[serde(default)]
    pub jvm_preset: crate::models::JvmPreset,
    #[serde(default)]
    pub jvm_flags: Vec<String>,
    pub port: u16,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default)]
    pub required_java: Option<u32>,
    #[serde(default)]
    pub artifact: Option<String>,
}
fn default_min() -> u32 {
    1024
}
fn default_max() -> u32 {
    2048
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub schema: u32,
    pub created_at: String,
    pub app_version: String,
    pub server: ManifestServer,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub include_backups: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub backups_bytes: u64,
    pub backups_files: u64,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub include_backups: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePreview {
    pub path: String,
    pub name: String,
    pub minecraft_version: String,
    pub server_type: ServerType,
    pub port: u16,
    pub suggested_port: u16,
    pub files: u64,
    pub size_bytes: u64,
    pub archive_bytes: u64,
    pub include_backups: bool,
    pub created_at: String,
    pub app_version: String,
    pub required_java: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProgress {
    pub bytes: u64,
    pub total_bytes: u64,
    pub files: u64,
    pub total_files: u64,
}

struct Progress<'a> {
    manager: &'a Manager,
    bytes: u64,
    total_bytes: u64,
    files: u64,
    total_files: u64,
    last: Instant,
}
impl<'a> Progress<'a> {
    fn new(manager: &'a Manager, total_bytes: u64, total_files: u64) -> Self {
        Self {
            manager,
            bytes: 0,
            total_bytes,
            files: 0,
            total_files,
            last: Instant::now(),
        }
    }
    fn tick(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.last.elapsed() >= Duration::from_millis(120) {
            self.last = Instant::now();
            self.emit();
        }
    }
    fn done_file(&mut self) {
        self.files += 1;
    }
    fn emit(&self) {
        self.manager.emit(
            "migration-progress",
            MigrationProgress {
                bytes: self.bytes,
                total_bytes: self.total_bytes,
                files: self.files,
                total_files: self.total_files,
            },
        );
    }
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn zip_error(e: zip::result::ZipError) -> AppError {
    AppError::new("The server archive could not be read safely.", e)
        .with_code("migration.invalidArchive")
}

fn user(code: &str, message: &str) -> AppError {
    AppError::user(message).with_code(code)
}

fn root_state(name: &str, include_backups: bool) -> RootEntry {
    let lower = name.to_ascii_lowercase();
    if lower == "backups" {
        return if include_backups {
            RootEntry::Include
        } else {
            RootEntry::Backups
        };
    }
    if [
        "easylauncher-export.json",
        "minehost.json",
        "monitor.json",
        "perf-history.json",
        "player-stats.json",
        "world-tracks.json",
        "tasks.json",
        "task-history.json",
        "upgrade.json",
        "content-rollback.json",
        "modpack-rollback.json",
        "modpack-pending.json",
        "modpack-pending.mrpack",
        "tunnel.json",
        "network.json",
        "retention.json",
        "backup.json",
        "secrets.json",
        "settings.json",
        "server.jar.previous",
        "server.upgrade.jar",
    ]
    .contains(&lower.as_str())
        || lower.ends_with(".part")
        || lower.starts_with("restore-")
        || lower.starts_with("deleted-")
    {
        return RootEntry::Skip;
    }
    if [
        "logs",
        "crash-reports",
        "remote-backups",
        "content-backups",
        "modpack-backups",
        "profiles",
        "updates",
    ]
    .contains(&lower.as_str())
    {
        return RootEntry::Skip;
    }
    RootEntry::Include
}

enum RootEntry {
    Include,
    Backups,
    Skip,
}

struct Collected {
    files: Vec<(PathBuf, u64)>,
    bytes: u64,
    backups_bytes: u64,
    backups_files: u64,
}

fn collect(
    dir: &Path,
    rel: &Path,
    root: bool,
    include_backups: bool,
    out: &mut Collected,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = match entry.file_type() {
            Ok(k) => k,
            Err(_) => continue,
        };
        let path = entry.path();
        storage::no_links(&path)?;
        let child = rel.join(&name);
        if root {
            match root_state(&name, include_backups) {
                RootEntry::Skip => continue,
                RootEntry::Backups => {
                    count_tree(&path, &mut out.backups_bytes, &mut out.backups_files)?;
                    continue;
                }
                RootEntry::Include => {}
            }
        }
        if kind.is_dir() {
            collect(&path, &child, false, include_backups, out)?;
        } else if kind.is_file() {
            let len = entry.metadata()?.len();
            out.bytes = out.bytes.saturating_add(len);
            out.files.push((child, len));
        } else {
            return Err(user(
                "migration.specialFile",
                "The server folder contains a special file that cannot be exported.",
            ));
        }
    }
    Ok(())
}

/// The archive must not carry remote-control credentials or a fixed bind
/// address. Import applies the same repairs, but redacting at export time also
/// keeps the password out of the ZIP itself.
fn redact_properties(text: &str) -> String {
    let mut properties = crate::minecraft::properties::Properties::parse(text);
    properties.set("enable-rcon", "false");
    properties.set("enable-query", "false");
    properties.set("enable-management-server", "false");
    properties.set("server-ip", "");
    properties.values.remove("rcon.password");
    properties.serialize()
}

fn count_tree(dir: &Path, bytes: &mut u64, files: &mut u64) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        storage::no_links(&entry.path())?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            count_tree(&entry.path(), bytes, files)?;
        } else if kind.is_file() {
            *bytes = bytes.saturating_add(entry.metadata()?.len());
            *files += 1;
        }
    }
    Ok(())
}

fn zip_name(rel: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            return Err(user(
                "migration.invalidName",
                "The server folder contains a path that cannot be exported.",
            ));
        };
        let text = part.to_str().ok_or_else(|| {
            user(
                "migration.invalidName",
                "The server folder contains a file name that is not valid Unicode.",
            )
        })?;
        if !safe_component(text) {
            return Err(user(
                "migration.invalidName",
                "The server folder contains a file name that cannot be stored in a portable archive.",
            ));
        }
        parts.push(text.to_string());
    }
    Ok(parts.join("/"))
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn safe_component(part: &str) -> bool {
    if part.is_empty()
        || part == "."
        || part == ".."
        || part.len() > 160
        || part.contains(['/', '\\', ':', '\0'])
        || part.ends_with(['.', ' '])
    {
        return false;
    }
    let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    !RESERVED.contains(&stem.as_str())
}

fn safe_zip_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > MAX_PATH
        || name.contains('\0')
        || name.contains('\\')
        || name.contains(':')
        || name.starts_with('/')
        || name.contains("//")
    {
        return Err(user(
            "migration.unsafeEntry",
            "The archive contains an unsafe path and was rejected.",
        ));
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() {
        return Ok(());
    }
    for part in trimmed.split('/') {
        if !safe_component(part) {
            return Err(user(
                "migration.unsafeEntry",
                "The archive contains an unsafe path and was rejected.",
            ));
        }
    }
    Ok(())
}

fn valid_manifest(server: &ManifestServer) -> Result<()> {
    let name = server.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(user(
            "migration.invalidManifest",
            "The archive names the server with an empty or oversized name.",
        ));
    }
    if !import::valid_version(&server.minecraft_version) {
        return Err(user(
            "migration.invalidManifest",
            "The archive does not contain a valid Minecraft version.",
        ));
    }
    if server.memory_min_mb < 256
        || server.memory_max_mb < server.memory_min_mb
        || server.memory_max_mb > 1_048_576
    {
        return Err(user(
            "migration.invalidManifest",
            "The archive contains an invalid memory configuration.",
        ));
    }
    if server.port == 0 {
        return Err(user(
            "migration.invalidManifest",
            "The archive contains an invalid server port.",
        ));
    }
    if server.jvm_flags.len() > 64 {
        return Err(user(
            "migration.invalidManifest",
            "The archive contains too many JVM flags.",
        ));
    }
    if !server.jvm_flags.is_empty() {
        crate::server::process::validate_jvm_flags(&server.jvm_flags).map_err(|e| {
            user(
                "migration.invalidManifest",
                &format!("The archive contains invalid JVM flags: {}", e.message),
            )
        })?;
    }
    if let Some(java) = server.required_java {
        if !(8..=99).contains(&java) {
            return Err(user(
                "migration.invalidManifest",
                "The archive contains an invalid Java requirement.",
            ));
        }
    }
    if let Some(artifact) = &server.artifact {
        if !safe_component(artifact) || !artifact.to_ascii_lowercase().ends_with(".jar") {
            return Err(user(
                "migration.invalidManifest",
                "The archive contains an invalid startup file name.",
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ArchiveInfo {
    path: PathBuf,
    manifest: Manifest,
    entries: Vec<(String, u64)>,
    total_bytes: u64,
    archive_bytes: u64,
    artifact: String,
}

fn inspect(path: &Path) -> Result<ArchiveInfo> {
    storage::no_links(path)?;
    let metadata = fs::metadata(path).map_err(|_| {
        user(
            "migration.missingArchive",
            "The archive could not be opened. Check that the file still exists.",
        )
    })?;
    if !metadata.is_file() {
        return Err(user(
            "migration.missingArchive",
            "Choose an EasyLauncher archive file, not a folder.",
        ));
    }
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() as u64 > MAX_ENTRIES {
        return Err(user(
            "migration.limit",
            "The archive contains too many entries to import safely.",
        ));
    }
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut manifest = None;
    let mut total_bytes = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let name = entry.name().to_string();
        safe_zip_name(&name)?;
        if entry
            .unix_mode()
            .map(|mode| mode & 0o170000 == 0o120000)
            .unwrap_or(false)
        {
            return Err(user(
                "migration.unsafeEntry",
                "The archive contains a link and was rejected.",
            ));
        }
        let key = if cfg!(windows) {
            name.to_ascii_lowercase()
        } else {
            name.clone()
        };
        if !seen.insert(key) {
            return Err(user(
                "migration.unsafeEntry",
                "The archive contains duplicate paths and was rejected.",
            ));
        }
        if entry.is_dir() {
            continue;
        }
        let size = entry.size();
        if size > MAX_SINGLE {
            return Err(user(
                "migration.limit",
                "The archive contains a file larger than the 16 GB import limit.",
            ));
        }
        total_bytes = total_bytes.saturating_add(size);
        if total_bytes > MAX_UNCOMPRESSED {
            return Err(user(
                "migration.limit",
                "The archive expands beyond the 64 GB import limit.",
            ));
        }
        if name == MANIFEST {
            if size > MAX_MANIFEST {
                return Err(user(
                    "migration.invalidManifest",
                    "The archive manifest is too large.",
                ));
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(MAX_MANIFEST + 1)
                .read_to_end(&mut bytes)?;
            let parsed: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
                AppError::new("The archive manifest could not be read.", e)
                    .with_code("migration.invalidManifest")
            })?;
            manifest = Some(parsed);
            continue;
        }
        entries.push((name, size));
    }
    let manifest = manifest.ok_or_else(|| {
        user(
            "migration.invalidManifest",
            "This archive does not contain an EasyLauncher export manifest.",
        )
    })?;
    if manifest.format != FORMAT || manifest.schema > SCHEMA {
        return Err(user(
            "migration.unsupported",
            "This archive was created by a different or newer EasyLauncher version.",
        ));
    }
    valid_manifest(&manifest.server)?;
    let jars: Vec<(String, u64)> = entries
        .iter()
        .filter(|(name, _)| !name.contains('/') && name.to_ascii_lowercase().ends_with(".jar"))
        .map(|(name, size)| (name.clone(), *size))
        .collect();
    let artifact = manifest
        .server
        .artifact
        .as_ref()
        .filter(|name| jars.iter().any(|(jar, _)| jar.eq_ignore_ascii_case(name)))
        .cloned()
        .or_else(|| {
            jars.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("server.jar"))
                .map(|(name, _)| name.clone())
        })
        .or_else(|| import::select_jar(&jars))
        .ok_or_else(|| {
            user(
                "migration.noJar",
                "The archive does not contain a runnable server JAR in its root.",
            )
        })?;
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        manifest,
        entries,
        total_bytes,
        archive_bytes: metadata.len(),
        artifact,
    })
}

fn collect_server(m: &Manager, id: &str, include_backups: bool) -> Result<Collected> {
    let dir = m.dir(id)?;
    if !dir.is_dir() {
        return Err(user(
            "server.notFound",
            "This server could not be found. Refresh your server list.",
        ));
    }
    let mut out = Collected {
        files: Vec::new(),
        bytes: 0,
        backups_bytes: 0,
        backups_files: 0,
    };
    collect(&dir, Path::new(""), true, include_backups, &mut out)?;
    out.files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn export_preview(m: &Manager, id: &str) -> Result<ExportPreview> {
    let server = m.metadata(id)?;
    let collected = collect_server(m, id, false)?;
    let running = {
        let runtime = m.runtime(id)?;
        let snapshot = runtime.snapshot();
        snapshot.status.active() || snapshot.status == Status::Installing
    };
    Ok(ExportPreview {
        id: id.to_string(),
        name: server.name,
        size_bytes: collected.bytes,
        file_count: collected.files.len() as u64,
        backups_bytes: collected.backups_bytes,
        backups_files: collected.backups_files,
        running,
    })
}

fn destination(raw: &str, server_dir: &Path, app_root: &Path) -> Result<PathBuf> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(user(
            "migration.badPath",
            "Choose where the archive should be saved.",
        ));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(user(
            "migration.badPath",
            "Choose a full path for the archive.",
        ));
    }
    let name = path.file_name().ok_or_else(|| {
        user(
            "migration.badPath",
            "The archive path must include a file name ending in .zip.",
        )
    })?;
    let file_name = name.to_string_lossy();
    if !file_name.to_ascii_lowercase().ends_with(".zip") {
        return Err(user(
            "migration.badExtension",
            "Save the archive with a .zip extension.",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        user(
            "migration.badPath",
            "Choose an existing folder for the archive.",
        )
    })?;
    let parent = parent.canonicalize().map_err(|_| {
        user(
            "migration.badPath",
            "The destination folder does not exist or cannot be opened.",
        )
    })?;
    if !parent.is_dir() {
        return Err(user(
            "migration.badPath",
            "Choose an existing folder for the archive.",
        ));
    }
    let dest = parent.join(name);
    if dest.exists() {
        return Err(user(
            "migration.exists",
            "That archive already exists. Choose another name.",
        ));
    }
    let server_dir = server_dir
        .canonicalize()
        .unwrap_or_else(|_| server_dir.to_path_buf());
    let app_root = app_root
        .canonicalize()
        .unwrap_or_else(|_| app_root.to_path_buf());
    if dest.starts_with(&server_dir) || dest.starts_with(&app_root) {
        return Err(user(
            "migration.insideApp",
            "Save the archive outside the EasyLauncher data folder.",
        ));
    }
    Ok(dest)
}

pub fn export(
    m: &Arc<Manager>,
    id: &str,
    raw_path: &str,
    include_backups: bool,
) -> Result<ExportResult> {
    let server = m.metadata(id)?;
    let runtime = m.runtime(id)?;
    let snapshot = runtime.snapshot();
    if snapshot.status.active() || snapshot.status == Status::Installing {
        return Err(user(
            "migration.notStopped",
            "Stop the server and wait for the current operation to finish before exporting it.",
        ));
    }
    let dir = m.dir(id)?;
    let dest = destination(raw_path, &dir, &m.root)?;
    let collected = collect_server(m, id, include_backups)?;
    let total_bytes = collected.bytes;
    let total_files = collected.files.len() as u64;
    let manifest = Manifest {
        format: FORMAT.into(),
        schema: SCHEMA,
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        server: ManifestServer {
            name: server.name.clone(),
            minecraft_version: server.minecraft_version.clone(),
            server_type: server.server_type,
            memory_min_mb: server.memory_min_mb,
            memory_max_mb: server.memory_max_mb,
            jvm_preset: server.jvm_preset,
            jvm_flags: server.jvm_flags.clone(),
            port: server.port,
            auto_restart: server.auto_restart,
            required_java: server.required_java,
            artifact: server.artifact.clone(),
        },
        files: total_files,
        bytes: total_bytes,
        include_backups,
    };
    let parent = dest.parent().expect("destination has a parent");
    let temp = parent.join(format!(".easylauncher-{}.part", uuid::Uuid::new_v4()));
    let result = (|| -> Result<ExportResult> {
        let file = File::create(&temp)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        zip.start_file(MANIFEST, options).map_err(zip_error)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        let mut progress = Progress::new(m, total_bytes, total_files);
        for (relative, size) in &collected.files {
            let name = zip_name(relative)?;
            let source = dir.join(relative);
            storage::no_links(&source)?;
            let per_file = if *size > u32::MAX as u64 {
                options.large_file(true)
            } else {
                options
            };
            zip.start_file(name, per_file).map_err(zip_error)?;
            if relative == Path::new("server.properties") {
                let bytes = fs::read(&source)?;
                let redacted = redact_properties(&String::from_utf8_lossy(&bytes));
                zip.write_all(redacted.as_bytes())?;
                progress.tick(redacted.len() as u64);
            } else {
                let mut input = File::open(&source)?;
                let mut buffer = vec![0u8; 256 * 1024];
                loop {
                    let read = input.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    zip.write_all(&buffer[..read])?;
                    progress.tick(read as u64);
                }
            }
            progress.done_file();
        }
        let file = zip.finish().map_err(zip_error)?;
        file.sync_all()?;
        progress.emit();
        fs::rename(&temp, &dest)?;
        Ok(ExportResult {
            path: display_path(&dest),
            bytes: total_bytes,
            files: total_files,
            include_backups,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(result?)
}

pub fn preview_archive(m: &Manager, raw: &str) -> Result<ArchivePreview> {
    let path = checked_archive_path(raw)?;
    let info = inspect(&path)?;
    let server = &info.manifest.server;
    let used: BTreeSet<u16> = m
        .list()?
        .into_iter()
        .filter(|view| view.server.installed)
        .map(|view| view.server.port)
        .collect();
    let occupied = used.contains(&server.port) || !crate::network::port_free(server.port);
    let suggested = if occupied {
        crate::resources::suggest_port(
            &used,
            server.port.saturating_add(1).max(25565),
            crate::network::port_free,
        )
        .unwrap_or(server.port)
    } else {
        server.port
    };
    let mut warnings = Vec::new();
    if occupied {
        warnings.push("portInUse".to_string());
    }
    if info.manifest.include_backups {
        warnings.push("includesBackups".to_string());
    }
    Ok(ArchivePreview {
        path: display_path(&info.path),
        name: server.name.clone(),
        minecraft_version: server.minecraft_version.clone(),
        server_type: server.server_type,
        port: server.port,
        suggested_port: suggested,
        files: info.entries.len() as u64,
        size_bytes: info.total_bytes,
        archive_bytes: info.archive_bytes,
        include_backups: info.manifest.include_backups,
        created_at: info.manifest.created_at.clone(),
        app_version: info.manifest.app_version.clone(),
        required_java: crate::java::version::required_java(
            &server.minecraft_version,
            server.server_type,
            server.required_java,
        ),
        warnings,
    })
}

fn checked_archive_path(raw: &str) -> Result<PathBuf> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(user(
            "migration.badPath",
            "Choose an EasyLauncher archive to import.",
        ));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(user(
            "migration.badPath",
            "Choose the full path to an EasyLauncher archive.",
        ));
    }
    Ok(path)
}

fn extract(info: &ArchiveInfo, dir: &Path, m: &Manager) -> Result<()> {
    let file = File::open(&info.path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() as u64 > MAX_ENTRIES {
        return Err(user(
            "migration.limit",
            "The archive contains too many entries to import safely.",
        ));
    }
    let mut progress = Progress::new(m, info.total_bytes, info.entries.len() as u64);
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let name = entry.name().to_string();
        safe_zip_name(&name)?;
        if name == MANIFEST {
            continue;
        }
        let relative = Path::new(&name);
        let out = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if entry.size() > MAX_SINGLE {
            return Err(user(
                "migration.limit",
                "The archive contains a file larger than the 16 GB import limit.",
            ));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        storage::no_links(&out)?;
        let mut output = File::create(&out)?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > MAX_UNCOMPRESSED {
                return Err(user(
                    "migration.limit",
                    "The archive expands beyond the 64 GB import limit.",
                ));
            }
            output.write_all(&buffer[..read])?;
            progress.tick(read as u64);
        }
        progress.done_file();
        output.sync_all()?;
    }
    progress.emit();
    Ok(())
}

fn archive_server(
    info: &ArchiveInfo,
    name: &str,
    version: &str,
    port: u16,
    java_path: Option<String>,
) -> Result<Server> {
    let manifest = &info.manifest.server;
    let required = crate::java::version::required_java(version, manifest.server_type, None);
    let (total, available, _) = memory_info();
    let ceiling = total
        .saturating_sub(1024)
        .min(total.saturating_mul(80) / 100)
        .min(available.saturating_sub(512))
        .min(u32::MAX as u64) as u32;
    let max = manifest.memory_max_mb.clamp(512, ceiling.max(512));
    let min = manifest.memory_min_mb.clamp(256, max);
    validate_memory(min, max)?;
    Ok(Server {
        schema_version: 1,
        id: String::new(),
        name: name.to_string(),
        minecraft_version: version.to_string(),
        server_type: manifest.server_type,
        memory_min_mb: min,
        memory_max_mb: max,
        jvm_preset: manifest.jvm_preset,
        jvm_flags: manifest.jvm_flags.clone(),
        port,
        created_at: chrono::Utc::now().to_rfc3339(),
        last_started_at: None,
        java_path,
        eula_accepted_at: Some(chrono::Utc::now().to_rfc3339()),
        installed: true,
        auto_restart: manifest.auto_restart,
        required_java: Some(required),
        artifact: Some("server.jar".into()),
    })
}

pub async fn import_archive(
    m: &Arc<Manager>,
    raw: String,
    name: String,
    minecraft_version: String,
    port: u16,
    eula_accepted: bool,
) -> Result<Server> {
    m.ready()?;
    if !eula_accepted {
        return Err(user(
            "migration.eula",
            "Please read and explicitly accept the Minecraft EULA before importing an archive.",
        ));
    }
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(user(
            "migration.badName",
            "Choose a server name between 1 and 64 characters.",
        ));
    }
    if port == 0 {
        return Err(user(
            "migration.badPort",
            "Choose a server port between 1 and 65535.",
        ));
    }
    let path = checked_archive_path(&raw)?;
    let inspected = tokio::task::spawn_blocking({
        let path = path.clone();
        move || inspect(&path)
    })
    .await
    .map_err(|e| AppError::new("The archive scan stopped unexpectedly.", e))??;
    let version = {
        let requested = minecraft_version.trim();
        let value = if requested.is_empty() {
            inspected.manifest.server.minecraft_version.clone()
        } else {
            requested.to_string()
        };
        if !import::valid_version(&value) {
            return Err(user(
                "migration.badVersion",
                "Enter the Minecraft version used by this server, for example 1.20.4.",
            ));
        }
        value
    };
    let settings = m.settings()?;
    let runtimes = crate::java::detector::detect(settings.java_path, Some(&m.java)).await;
    let required =
        crate::java::version::required_java(&version, inspected.manifest.server.server_type, None);
    let java_path = runtimes
        .iter()
        .find(|runtime| runtime.major == required)
        .map(|runtime| runtime.path.clone());
    let mut server = archive_server(&inspected, &name, &version, port, java_path)?;
    let id = uuid::Uuid::new_v4().to_string();
    server.id = id.clone();
    let dir = m.dir(&id)?;
    fs::create_dir(&dir)?;
    let owner = Arc::clone(m);
    let info = ArchiveInfo {
        path: inspected.path.clone(),
        manifest: inspected.manifest.clone(),
        entries: inspected.entries.clone(),
        total_bytes: inspected.total_bytes,
        archive_bytes: inspected.archive_bytes,
        artifact: inspected.artifact.clone(),
    };
    let server = tokio::task::spawn_blocking({
        let dir = dir.clone();
        let owner = Arc::clone(&owner);
        let server = server.clone();
        move || -> Result<Server> {
            let outcome = (|| -> Result<Server> {
                extract(&info, &dir, &owner)?;
                import::rename_jar(&dir, &info.artifact)?;
                let raw = fs::read(dir.join("server.properties"))
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
                let mut properties = import::sanitize_properties(raw.as_deref(), server.port)?;
                properties.set("server-port", &server.port.to_string());
                properties.validate()?;
                storage::atomic_write(
                    &storage::child(&dir, "server.properties")?,
                    properties.serialize().as_bytes(),
                )?;
                storage::atomic_write(
                    &storage::child(&dir, "eula.txt")?,
                    b"# Explicitly accepted in EasyLauncher\neula=true\n",
                )?;
                for name in ["logs", "backups", "mods", "plugins"] {
                    fs::create_dir_all(storage::child(&dir, name)?)?;
                }
                storage::save_server(&owner.servers, &server)?;
                Ok(server)
            })();
            if outcome.is_err() {
                let _ = fs::remove_dir_all(&dir);
            }
            outcome
        }
    })
    .await
    .map_err(|e| AppError::new("The archive import stopped unexpectedly.", e))??;
    m.runtimes.lock().unwrap().insert(
        id.clone(),
        Arc::new(Runtime::new(&id, Status::Stopped, None)),
    );
    m.logger.log(
        "INFO",
        &format!(
            "Imported server {id} from archive {}",
            display_path(&inspected.path)
        ),
    );
    Ok(server)
}

fn suggested_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('-').trim();
    if cleaned.is_empty() {
        "EasyLauncher-server".into()
    } else {
        format!("EasyLauncher-{cleaned}")
    }
}

#[cfg(windows)]
fn file_dialog(script: &str) -> Result<Option<String>> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-STA",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000);
    let output = command.output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .map(str::to_string))
}

#[cfg(windows)]
pub fn pick_export_path(name: &str) -> Result<Option<String>> {
    let file_name = suggested_file_name(name);
    let script = format!(
        r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dialog = New-Object System.Windows.Forms.SaveFileDialog
$dialog.Title = 'Save the EasyLauncher server archive'
$dialog.Filter = 'EasyLauncher server archive (*.zip)|*.zip|All files (*.*)|*.*'
$dialog.DefaultExt = 'zip'
$dialog.AddExtension = $true
$dialog.OverwritePrompt = $true
$dialog.FileName = '{file_name}.zip'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{
  [Console]::Out.WriteLine($dialog.FileName)
}}
"#
    );
    file_dialog(&script)
}

#[cfg(windows)]
pub fn pick_archive() -> Result<Option<String>> {
    const SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Choose an EasyLauncher server archive'
$dialog.Filter = 'EasyLauncher server archive (*.zip)|*.zip|All files (*.*)|*.*'
$dialog.CheckFileExists = $true
$dialog.Multiselect = $false
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
  [Console]::Out.WriteLine($dialog.FileName)
}
"#;
    file_dialog(SCRIPT)
}

#[cfg(not(windows))]
pub fn pick_export_path(_name: &str) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(not(windows))]
pub fn pick_archive() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::JvmPreset;

    fn fixture(m: &Manager) -> Server {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = m.dir(&id).unwrap();
        fs::create_dir_all(dir.join("world/region")).unwrap();
        fs::write(dir.join("paper.jar"), b"jar fixture").unwrap();
        fs::write(
            dir.join("server.properties"),
            "server-port=25566\nserver-ip=0.0.0.0\nenable-rcon=true\nrcon.password=secret\nmotd=Portable\nlevel-name=world\n",
        )
        .unwrap();
        fs::write(dir.join("eula.txt"), "eula=true\n").unwrap();
        fs::write(dir.join("world/level.dat"), b"level").unwrap();
        fs::write(dir.join("world/region/r.0.0.mca"), b"region").unwrap();
        fs::write(dir.join("content.json"), b"[]").unwrap();
        fs::create_dir_all(dir.join("logs")).unwrap();
        fs::write(dir.join("logs/latest.log"), b"log").unwrap();
        fs::create_dir_all(dir.join("backups/old")).unwrap();
        fs::write(dir.join("backups/old/snapshot.zip"), b"backup").unwrap();
        fs::write(dir.join("perf-history.json"), b"[]").unwrap();
        fs::write(dir.join("player-stats.json"), b"{}").unwrap();
        let server = Server {
            schema_version: 1,
            id: id.clone(),
            name: "Portable".into(),
            minecraft_version: "1.20.4".into(),
            server_type: ServerType::Paper,
            memory_min_mb: 1024,
            memory_max_mb: 2048,
            jvm_preset: JvmPreset::Balanced,
            jvm_flags: Vec::new(),
            port: 25566,
            created_at: "2026-01-01T00:00:00Z".into(),
            last_started_at: None,
            java_path: None,
            eula_accepted_at: Some("2026-01-01T00:00:00Z".into()),
            installed: true,
            auto_restart: false,
            required_java: Some(17),
            artifact: Some("paper.jar".into()),
        };
        storage::save_server(&m.servers, &server).unwrap();
        m.runtimes.lock().unwrap().insert(
            id.clone(),
            Arc::new(Runtime::new(&id, Status::Stopped, None)),
        );
        server
    }

    #[test]
    fn archive_names_are_strictly_validated() {
        for name in [
            "easylauncher-export.json",
            "world/region/r.0.0.mca",
            "mods/Example Mod.jar",
            "目录/文件.txt",
            "empty/",
        ] {
            assert!(safe_zip_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "/absolute",
            "../escape",
            "world/../../escape",
            "a\\b",
            "C:/drive",
            "a//b",
            "./file",
            "CON/file",
            "trailing./file",
            "bad:name",
        ] {
            assert!(safe_zip_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn export_preview_excludes_app_state_and_reports_backups() {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::open(temp.path().join("app")).unwrap();
        let server = fixture(&m);
        let preview = export_preview(&m, &server.id).unwrap();
        assert_eq!(preview.name, "Portable");
        assert!(!preview.running);
        assert!(preview.backups_bytes > 0);
        assert_eq!(preview.backups_files, 1);
        assert!(preview.file_count >= 5);
    }

    #[tokio::test]
    async fn export_and_import_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::open(temp.path().join("app")).unwrap();
        let server = fixture(&m);
        let destination = temp.path().join("Portable.zip");
        let result = export(&m, &server.id, destination.to_str().unwrap(), false).unwrap();
        assert_eq!(result.files, 6);
        assert!(destination.is_file());

        let info = inspect(&destination).unwrap();
        let names: Vec<&str> = info.entries.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"world/level.dat"));
        assert!(names.contains(&"paper.jar"));
        assert!(!names.iter().any(|n| n.starts_with("logs/")));
        assert!(!names.iter().any(|n| n.starts_with("backups/")));
        assert!(!names.contains(&"perf-history.json"));
        assert!(!names.contains(&"player-stats.json"));
        assert!(!names.contains(&"minehost.json"));
        assert_eq!(info.artifact, "paper.jar");
        let mut archive = zip::ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        let mut properties = String::new();
        archive
            .by_name("server.properties")
            .unwrap()
            .read_to_string(&mut properties)
            .unwrap();
        assert!(!properties.contains("rcon.password"));
        assert!(properties.contains("enable-rcon=false"));
        assert!(properties.contains("server-ip=\n"));
        assert!(properties.contains("motd=Portable"));

        let preview = preview_archive(&m, destination.to_str().unwrap()).unwrap();
        assert_eq!(preview.minecraft_version, "1.20.4");
        assert_eq!(preview.server_type, ServerType::Paper);
        assert_eq!(preview.port, 25566);
        assert!(!preview.include_backups);

        let old_count = m.list().unwrap().len();
        let imported = import_archive(
            &m,
            destination.to_str().unwrap().into(),
            "Imported".into(),
            String::new(),
            25599,
            true,
        )
        .await
        .unwrap();
        assert_eq!(imported.name, "Imported");
        assert_eq!(imported.port, 25599);
        assert_eq!(imported.minecraft_version, "1.20.4");
        assert_eq!(imported.artifact.as_deref(), Some("server.jar"));
        assert_eq!(m.list().unwrap().len(), old_count + 1);
        let dir = m.dir(&imported.id).unwrap();
        assert!(dir.join("server.jar").is_file());
        assert!(!dir.join("paper.jar").exists());
        assert_eq!(fs::read(dir.join("world/level.dat")).unwrap(), b"level");
        assert!(!dir.join("logs/latest.log").exists());
        assert!(!dir.join("backups/old/snapshot.zip").exists());
        let properties = crate::minecraft::properties::Properties::parse(
            &fs::read_to_string(dir.join("server.properties")).unwrap(),
        );
        assert_eq!(properties.values["server-port"], "25599");
        assert_eq!(properties.values["server-ip"], "");
        assert_eq!(properties.values["enable-rcon"], "false");
        assert_eq!(properties.values["motd"], "Portable");
        assert_eq!(
            fs::read_to_string(dir.join("eula.txt"))
                .unwrap()
                .contains("eula=true"),
            true
        );
        assert_eq!(
            fs::read_to_string(dir.join("minehost.json"))
                .unwrap()
                .contains("\"Imported\""),
            true
        );
    }

    #[tokio::test]
    async fn backups_can_be_included_explicitly() {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::open(temp.path().join("app")).unwrap();
        let server = fixture(&m);
        let destination = temp.path().join("with-backups.zip");
        let result = export(&m, &server.id, destination.to_str().unwrap(), true).unwrap();
        assert!(result.include_backups);
        let info = inspect(&destination).unwrap();
        assert!(info
            .entries
            .iter()
            .any(|(name, _)| name == "backups/old/snapshot.zip"));
        assert_eq!(info.manifest.files, result.files);
        assert_eq!(info.manifest.bytes, result.bytes);
    }

    #[test]
    fn export_is_refused_while_running_and_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::open(temp.path().join("app")).unwrap();
        let server = fixture(&m);
        let destination = temp.path().join("existing.zip");
        fs::write(&destination, b"existing").unwrap();
        let error = export(&m, &server.id, destination.to_str().unwrap(), false).unwrap_err();
        assert_eq!(error.code, "migration.exists");

        m.runtimes.lock().unwrap().insert(
            server.id.clone(),
            Arc::new(Runtime::new(&server.id, Status::Running, None)),
        );
        let error = export(
            &m,
            &server.id,
            temp.path().join("running.zip").to_str().unwrap(),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "migration.notStopped");
    }

    #[test]
    fn unsafe_and_invalid_archives_are_rejected() {
        use zip::write::SimpleFileOptions;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.zip");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let error = inspect(&path).unwrap_err();
        assert_eq!(error.code, "migration.unsafeEntry");

        let path = temp.path().join("manifest.zip");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(MANIFEST, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"format":"easylauncher-server","schema":1,"createdAt":"x","appVersion":"1","server":{"name":"Bad","minecraftVersion":"not-a-version","serverType":"paper","port":25565}}"#)
            .unwrap();
        zip.start_file("server.jar", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"jar").unwrap();
        zip.finish().unwrap();
        let error = inspect(&path).unwrap_err();
        assert_eq!(error.code, "migration.invalidManifest");
    }
}
