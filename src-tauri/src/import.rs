use crate::{
    error::{AppError, Result},
    minecraft::properties::Properties,
    models::{Server, ServerType, Status},
    server::manager::{memory_info, validate_memory, Manager, Runtime},
    storage,
};
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub path: String,
    pub suggested_name: String,
    pub minecraft_version: String,
    pub version_source: String,
    pub server_type: ServerType,
    pub port: u16,
    pub jar: String,
    pub jar_rename_needed: bool,
    pub eula_accepted: bool,
    pub size_bytes: u64,
    pub file_count: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub copied_files: u64,
    pub total_files: u64,
}

#[derive(Default)]
struct Scan {
    size: u64,
    files: u64,
    jars: Vec<(String, u64)>,
    recovery_skipped: bool,
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn root_skip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".part")
        || lower.starts_with("restore-")
        || matches!(
            lower.as_str(),
            "minehost.json" | "network.json" | "tasks.json" | "monitor.json"
        )
}

fn scan_dir(dir: &Path, root: bool, scan: &mut Scan) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if root && root_skip(&name) {
            if name.to_ascii_lowercase().starts_with("restore-") {
                scan.recovery_skipped = true;
            }
            continue;
        }
        let path = entry.path();
        storage::no_links(&path)?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            scan_dir(&path, false, scan)?;
        } else if kind.is_file() {
            let len = entry.metadata()?.len();
            scan.size = scan.size.saturating_add(len);
            scan.files += 1;
            if root && name.to_ascii_lowercase().ends_with(".jar") {
                scan.jars.push((name, len));
            }
        } else {
            return Err(AppError::user(
                "This folder contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}

fn contains(name: &str, needle: &str) -> bool {
    name.to_ascii_lowercase().contains(needle)
}

fn is_named_server(name: &str) -> bool {
    name.eq_ignore_ascii_case("server.jar")
}

fn is_fabric(name: &str) -> bool {
    contains(name, "fabric-server")
}

fn is_paper(name: &str) -> bool {
    contains(name, "paper") || contains(name, "purpur") || contains(name, "spigot")
}

fn is_forge(name: &str) -> bool {
    contains(name, "forge")
}

pub(crate) fn select_jar(jars: &[(String, u64)]) -> Option<String> {
    let candidates: Vec<&(String, u64)> = jars
        .iter()
        .filter(|(name, _)| !contains(name, "installer"))
        .collect();
    for check in [
        is_named_server as fn(&str) -> bool,
        is_fabric,
        is_paper,
        is_forge,
    ] {
        if let Some((name, _)) = candidates.iter().find(|(name, _)| check(name)) {
            return Some(name.clone());
        }
    }
    candidates
        .iter()
        .max_by_key(|(_, size)| *size)
        .map(|(name, _)| name.clone())
}

fn is_minecraft_version(raw: &str) -> bool {
    let parts: Vec<&str> = raw.split('.').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.is_empty()) {
        return false;
    }
    let mut nums = Vec::with_capacity(parts.len());
    for part in parts {
        match part.parse::<u32>() {
            Ok(n) => nums.push(n),
            Err(_) => return false,
        }
    }
    match nums[0] {
        1 => (1..=99).contains(&nums[1]),
        major if major >= 26 => nums[1] <= 99,
        _ => false,
    }
}

pub(crate) fn version_from_text(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut best: Option<&str> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let raw = text[start..i].trim_matches('.');
            if is_minecraft_version(raw) && best.map(|b| raw.len() > b.len()).unwrap_or(true) {
                best = Some(raw);
            }
        } else {
            i += 1;
        }
    }
    best.map(str::to_string)
}

fn read_head(path: &Path, limit: u64) -> Option<String> {
    storage::no_links(path).ok()?;
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn version_from_log(root: &Path) -> Option<String> {
    let text = read_head(&root.join("logs").join("latest.log"), 256 * 1024)?;
    const MARKER: &str = "starting minecraft server version";
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(pos) = lower.find(MARKER) {
            if let Some(v) = version_from_text(&line[pos + MARKER.len()..]) {
                return Some(v);
            }
        }
    }
    None
}

fn version_from_jar(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    storage::no_links(path).ok()?;
    let file = fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for index in 0..archive.len().min(4096) {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().to_string();
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with("version.json") || name.matches('/').count() > 1 {
            continue;
        }
        let mut bytes = Vec::new();
        if (&mut entry)
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .is_err()
        {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let raw = value
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("name").and_then(|v| v.as_str()));
        if let Some(v) = raw.and_then(version_from_text) {
            return Some(v);
        }
    }
    None
}

fn version_from_dir_names(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    names.iter().find_map(|name| version_from_text(name))
}

fn version_from_libraries(root: &Path) -> Option<String> {
    version_from_dir_names(
        &root
            .join("libraries")
            .join("net")
            .join("minecraft")
            .join("server"),
    )
}

fn detect_version(root: &Path, jar: Option<&str>) -> (String, String) {
    if let Ok(bytes) = fs::read(root.join("version_history.json")) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(v) = value
                .get("currentVersion")
                .and_then(|v| v.as_str())
                .and_then(version_from_text)
            {
                return (v, "versionHistory".into());
            }
        }
    }
    if let Some(v) = version_from_log(root) {
        return (v, "latestLog".into());
    }
    if let Some(name) = jar {
        if let Some(v) = version_from_jar(&root.join(name)) {
            return (v, "serverJar".into());
        }
    }
    if let Some(v) = version_from_libraries(root) {
        return (v, "libraries".into());
    }
    if let Some(v) = version_from_dir_names(&root.join("versions")) {
        return (v, "versions".into());
    }
    if let Some(name) = jar {
        if let Some(v) = version_from_text(name) {
            return (v, "jarName".into());
        }
    }
    (String::new(), "unknown".into())
}

fn has_jar(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with(".jar")
            })
        })
        .unwrap_or(false)
}

fn detect_type(root: &Path, jar: Option<&str>) -> ServerType {
    let libraries = root.join("libraries");
    if libraries.join("net").join("fabricmc").is_dir() || jar.map(is_fabric).unwrap_or(false) {
        return ServerType::Fabric;
    }
    if root.join("purpur.yml").is_file()
        || jar
            .map(|name| name.to_ascii_lowercase().contains("purpur"))
            .unwrap_or(false)
    {
        return ServerType::Purpur;
    }
    if libraries.join("io").join("papermc").is_dir()
        || libraries
            .join("com")
            .join("mojang")
            .join("paperclip")
            .is_dir()
        || root.join("version_history.json").is_file()
        || jar.map(is_paper).unwrap_or(false)
        || has_jar(&root.join("plugins"))
    {
        return ServerType::Paper;
    }
    ServerType::Vanilla
}

fn is_modded(root: &Path, jar: Option<&str>, software: ServerType) -> bool {
    let libraries = root.join("libraries");
    if libraries.join("net").join("minecraftforge").is_dir()
        || libraries.join("net").join("neoforged").is_dir()
        || jar.map(is_forge).unwrap_or(false)
    {
        return true;
    }
    software == ServerType::Vanilla && has_jar(&root.join("mods"))
}

fn read_properties(path: &Path) -> Option<Properties> {
    storage::no_links(path).ok()?;
    let bytes = fs::read(path).ok()?;
    Some(Properties::parse(&String::from_utf8_lossy(&bytes)))
}

fn eula_accepted(root: &Path) -> bool {
    let path = root.join("eula.txt");
    if storage::no_links(&path).is_err() {
        return false;
    }
    fs::read_to_string(path)
        .map(|text| {
            Properties::parse(&text)
                .values
                .get("eula")
                .map(|v| v.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn unsafe_world(properties: &Option<Properties>) -> bool {
    properties
        .as_ref()
        .and_then(|p| p.values.get("level-name"))
        .map(|name| crate::minecraft::properties::validate_world(name).is_err())
        .unwrap_or(false)
}

pub fn preview(path: &str) -> Result<ImportPreview> {
    let raw = PathBuf::from(path.trim());
    if !raw.is_absolute() {
        return Err(AppError::user(
            "Choose the full path to an existing server folder.",
        ));
    }
    storage::no_links(&raw)?;
    let root = raw.canonicalize().map_err(|_| {
        AppError::user("This folder could not be opened. Check that it still exists.")
    })?;
    storage::no_links(&root)?;
    if !root.is_dir() {
        return Err(AppError::user("Choose a folder, not a file."));
    }
    let mut scan = Scan::default();
    scan_dir(&root, true, &mut scan)?;
    let jar = select_jar(&scan.jars);
    let (version, version_source) = detect_version(&root, jar.as_deref());
    let software = detect_type(&root, jar.as_deref());
    let properties = read_properties(&root.join("server.properties"));
    let port = properties
        .as_ref()
        .and_then(|p| p.values.get("server-port"))
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(25565);
    let accepted = eula_accepted(&root);
    let jar_rename_needed = jar
        .as_deref()
        .map(|name| name != "server.jar")
        .unwrap_or(false);
    let mut warnings = Vec::new();
    if jar.is_none() {
        warnings.push("noJar".to_string());
    }
    if version.is_empty() {
        warnings.push("unknownVersion".to_string());
    }
    if is_modded(&root, jar.as_deref(), software) {
        warnings.push("moddedLauncher".to_string());
    }
    if scan.size > 4 * 1024 * 1024 * 1024 {
        warnings.push("largeFolder".to_string());
    }
    if !accepted {
        warnings.push("eulaMissing".to_string());
    }
    if unsafe_world(&properties) {
        warnings.push("unsafeWorld".to_string());
    }
    if scan.recovery_skipped {
        warnings.push("recoverySkipped".to_string());
    }
    let folder_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim();
    let suggested_name = if folder_name.is_empty() {
        "Imported server".to_string()
    } else {
        folder_name.chars().take(64).collect()
    };
    Ok(ImportPreview {
        path: display_path(&root),
        suggested_name,
        minecraft_version: version,
        version_source,
        server_type: software,
        port,
        jar_rename_needed,
        jar: jar.unwrap_or_default(),
        eula_accepted: accepted,
        size_bytes: scan.size,
        file_count: scan.files,
        warnings,
    })
}

fn number(properties: &mut Properties, key: &str, min: u32, max: u32, default: u32) {
    let valid = properties
        .values
        .get(key)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|n| (min..=max).contains(&n))
        .unwrap_or(false);
    if !valid {
        properties.set(key, &default.to_string());
    }
}

fn boolean(properties: &mut Properties, key: &str, default: bool) {
    let value = properties
        .values
        .get(key)
        .map(|v| v.trim().to_ascii_lowercase());
    match value.as_deref() {
        Some("true") | Some("false") => properties.set(key, value.as_deref().unwrap_or("false")),
        _ => properties.set(key, if default { "true" } else { "false" }),
    }
}

fn choice(properties: &mut Properties, key: &str, map: &[(&str, &str)], default: &str) {
    let value = properties
        .values
        .get(key)
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let chosen = map
        .iter()
        .find(|(from, _)| *from == value)
        .map(|(_, to)| *to)
        .unwrap_or(default);
    properties.set(key, chosen);
}

/// Returns whether a dot-separated numeric Minecraft version is usable.
pub(crate) fn valid_version(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.contains('.')
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value.bytes().all(|c| c.is_ascii_digit() || c == b'.')
}

/// Repairs values that would block EasyLauncher's own start-up validation while
/// keeping every unrelated property the pack author wrote.
pub(crate) fn sanitize_properties(raw: Option<&str>, port: u16) -> Result<Properties> {
    let mut p = Properties::parse(
        raw.unwrap_or("# Imported by EasyLauncher. Unknown properties are preserved.\n"),
    );
    // Never expose remote control channels and never bind a fixed interface.
    p.set("enable-rcon", "false");
    p.set("enable-query", "false");
    p.set("enable-management-server", "false");
    p.set("server-ip", "");
    number(&mut p, "server-port", 1, 65535, port as u32);
    number(&mut p, "max-players", 1, 1000, 20);
    number(&mut p, "spawn-protection", 0, 10000, 16);
    number(&mut p, "view-distance", 2, 32, 10);
    number(&mut p, "simulation-distance", 2, 32, 10);
    boolean(&mut p, "pvp", true);
    boolean(&mut p, "online-mode", true);
    boolean(&mut p, "allow-flight", false);
    boolean(&mut p, "white-list", false);
    choice(
        &mut p,
        "gamemode",
        &[
            ("survival", "survival"),
            ("creative", "creative"),
            ("adventure", "adventure"),
            ("spectator", "spectator"),
            ("0", "survival"),
            ("1", "creative"),
            ("2", "adventure"),
            ("3", "spectator"),
        ],
        "survival",
    );
    choice(
        &mut p,
        "difficulty",
        &[
            ("peaceful", "peaceful"),
            ("easy", "easy"),
            ("normal", "normal"),
            ("hard", "hard"),
            ("0", "peaceful"),
            ("1", "easy"),
            ("2", "normal"),
            ("3", "hard"),
        ],
        "normal",
    );
    let world_ok = p
        .values
        .get("level-name")
        .map(|name| crate::minecraft::properties::validate_world(name).is_ok())
        .unwrap_or(false);
    if !world_ok {
        p.set("level-name", "world");
    }
    p.validate()?;
    Ok(p)
}

struct CopyState {
    total_bytes: u64,
    total_files: u64,
    copied_bytes: u64,
    copied_files: u64,
    last: Instant,
}
impl CopyState {
    fn new(total_bytes: u64, total_files: u64) -> Self {
        Self {
            total_bytes,
            total_files,
            copied_bytes: 0,
            copied_files: 0,
            last: Instant::now(),
        }
    }
    fn tick(&mut self, bytes: u64, manager: &Manager) {
        self.copied_bytes = self.copied_bytes.saturating_add(bytes);
        self.copied_files += 1;
        if self.last.elapsed() >= Duration::from_millis(120) {
            self.last = Instant::now();
            manager.emit("import-progress", self.progress());
        }
    }
    fn progress(&self) -> ImportProgress {
        ImportProgress {
            copied_bytes: self.copied_bytes,
            total_bytes: self.total_bytes,
            copied_files: self.copied_files,
            total_files: self.total_files,
        }
    }
}

fn copy_dir(
    from: &Path,
    to: &Path,
    root: bool,
    state: &mut CopyState,
    manager: &Manager,
) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if root && root_skip(&name) {
            continue;
        }
        let path = entry.path();
        storage::no_links(&path)?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&path, &target, false, state, manager)?;
        } else if kind.is_file() {
            let copied = fs::copy(&path, &target)?;
            state.tick(copied, manager);
        } else {
            return Err(AppError::user(
                "This folder contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn rename_jar(dir: &Path, jar: &str) -> Result<()> {
    if jar == "server.jar" {
        return Ok(());
    }
    let from = dir.join(jar);
    let to = dir.join("server.jar");
    storage::no_links(&from)?;
    if jar.eq_ignore_ascii_case("server.jar") {
        // Case-only rename cannot be done atomically on case-insensitive file systems.
        let temp = dir.join("server.jar.rename");
        fs::rename(&from, &temp)?;
        fs::rename(&temp, &to)?;
        return Ok(());
    }
    if to.exists() {
        fs::remove_file(&to)?;
    }
    fs::rename(&from, &to)?;
    Ok(())
}

pub async fn import_folder(
    manager: &Arc<Manager>,
    path: String,
    name: String,
    minecraft_version: String,
    eula_accepted: bool,
) -> Result<Server> {
    manager.ready()?;
    if !eula_accepted {
        return Err(AppError::user(
            "Please read and explicitly accept the Minecraft EULA before importing a server.",
        ));
    }
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::user(
            "Choose a server name between 1 and 64 characters.",
        ));
    }
    let inspected = tokio::task::spawn_blocking({
        let path = path.clone();
        move || preview(&path)
    })
    .await
    .map_err(|e| AppError::new("The folder scan stopped unexpectedly.", e))??;
    if inspected.jar.is_empty() {
        return Err(AppError::user(
            "No runnable server JAR was found in the folder root. Add server.jar (or a Paper/Fabric jar) and try again.",
        ));
    }
    let version = {
        let requested = minecraft_version.trim();
        let value = if requested.is_empty() {
            inspected.minecraft_version.clone()
        } else {
            requested.to_string()
        };
        if !valid_version(&value) {
            return Err(AppError::user(
                "Enter the Minecraft version used by this server, for example 1.20.4.",
            ));
        }
        value
    };
    let required = crate::java::version::required_java(&version, inspected.server_type, None);
    let settings = manager.settings()?;
    let runtimes = crate::java::detector::detect(settings.java_path, Some(&manager.java)).await;
    let java_path = runtimes
        .iter()
        .find(|j| j.major == required)
        .map(|j| j.path.clone());
    let (total, available, recommended) = memory_info();
    let cap = total
        .saturating_sub(1024)
        .min(total.saturating_mul(80) / 100);
    let max = (recommended as u64)
        .max(1024)
        .min(cap)
        .min(available.saturating_sub(512))
        .max(512) as u32;
    let min = (max / 2).max(256).min(max);
    validate_memory(min, max)?;
    let source = PathBuf::from(&inspected.path);
    if let Ok(servers) = manager.servers.canonicalize() {
        if servers.starts_with(&source) {
            return Err(AppError::user(
                "The EasyLauncher data folder cannot be imported into itself.",
            ));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let server = Server {
        schema_version: 1,
        id: id.clone(),
        name,
        minecraft_version: version,
        server_type: inspected.server_type,
        memory_min_mb: min,
        memory_max_mb: max,
        jvm_preset: crate::models::JvmPreset::Balanced,
        jvm_flags: Vec::new(),
        port: inspected.port,
        created_at: now.clone(),
        last_started_at: None,
        java_path,
        eula_accepted_at: Some(now),
        installed: true,
        auto_restart: false,
        required_java: Some(required),
        artifact: Some(inspected.jar.clone()),
    };
    let dir = manager.dir(&id)?;
    let servers = manager.servers.clone();
    let owner = Arc::clone(manager);
    fs::create_dir(&dir)?;
    let result = tokio::task::spawn_blocking({
        let dir = dir.clone();
        let source = source.clone();
        let info = inspected.clone();
        let server = server.clone();
        move || -> Result<Server> {
            let outcome = (|| -> Result<Server> {
                let mut state = CopyState::new(info.size_bytes, info.file_count);
                copy_dir(&source, &dir, true, &mut state, &owner)?;
                owner.emit("import-progress", state.progress());
                rename_jar(&dir, &info.jar)?;
                let raw = fs::read(source.join("server.properties"))
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
                let properties = sanitize_properties(raw.as_deref(), info.port)?;
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
                storage::save_server(&servers, &server)?;
                Ok(server)
            })();
            if outcome.is_err() {
                let _ = fs::remove_dir_all(&dir);
            }
            outcome
        }
    })
    .await
    .map_err(|e| AppError::new("The import stopped unexpectedly.", e))??;
    manager.runtimes.lock().unwrap().insert(
        id.clone(),
        Arc::new(Runtime::new(&id, Status::Stopped, None)),
    );
    manager.logger.log(
        "INFO",
        &format!("Imported server {id} from {}", inspected.path),
    );
    Ok(result)
}

#[cfg(windows)]
fn pick(description: &str, allow_new: bool) -> Result<Option<String>> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let script = format!(
        r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = '{}'
$dialog.ShowNewFolderButton = ${}
$result = $dialog.ShowDialog()
if ($result -eq [System.Windows.Forms.DialogResult]::OK) {{
  [Console]::Out.WriteLine($dialog.SelectedPath)
}}
"#,
        description.replace('\'', "''"),
        if allow_new { "true" } else { "false" }
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-STA",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
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
pub fn pick_folder() -> Result<Option<String>> {
    pick("Select the existing Minecraft server folder", false)
}

#[cfg(windows)]
pub fn pick_storage_folder() -> Result<Option<String>> {
    pick(
        "Select an empty folder for EasyLauncher server storage",
        true,
    )
}

#[cfg(not(windows))]
pub fn pick_folder() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(not(windows))]
pub fn pick_storage_folder() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        for (text, expected) in [
            ("minecraft 1.20.4", Some("1.20.4")),
            ("mc.1.21", Some("1.21")),
            ("paper-1.20.4-496.jar", Some("1.20.4")),
            ("server-1.8.9.jar", Some("1.8.9")),
            ("Starting minecraft server version 1.16.5", Some("1.16.5")),
            ("26.1-pre1", Some("26.1")),
        ] {
            assert_eq!(version_from_text(text).as_deref(), expected, "{text}");
        }
        for text in ["20.4", "java 21.0.2", "1", "1..20", "1.20.4.1"] {
            assert_eq!(version_from_text(text), None, "{text}");
        }
    }

    #[test]
    fn property_repairs() {
        let p = sanitize_properties(
            Some(
                "server-port=0\nmax-players=5000\nspawn-protection=-1\nview-distance=99\ngamemode=0\ndifficulty=hard\nlevel-name=../bad\npvp=TRUE\nonline-mode=FALSE\nallow-flight=yes\nwhite-list=TRUE\nenable-query=true\nserver-ip=1.2.3.4\nmotd=Hello\n",
            ),
            25565,
        )
        .unwrap();
        assert_eq!(p.values["server-port"], "25565");
        assert_eq!(p.values["max-players"], "20");
        assert_eq!(p.values["spawn-protection"], "16");
        assert_eq!(p.values["view-distance"], "10");
        assert_eq!(p.values["gamemode"], "survival");
        assert_eq!(p.values["difficulty"], "hard");
        assert_eq!(p.values["level-name"], "world");
        assert_eq!(p.values["pvp"], "true");
        assert_eq!(p.values["online-mode"], "false");
        assert_eq!(p.values["allow-flight"], "false");
        assert_eq!(p.values["white-list"], "true");
        assert_eq!(p.values["enable-query"], "false");
        assert_eq!(p.values["server-ip"], "");
        assert_eq!(p.values["motd"], "Hello");
    }

    #[test]
    fn preview_finds_jar_version_and_problems() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        fs::write(root.join("server.jar"), b"not a zip").unwrap();
        fs::write(
            root.join("server.properties"),
            "server-port=25566\nenable-rcon=true\nserver-ip=0.0.0.0\ngamemode=0\nmax-players=5000\nlevel-name=../bad\n",
        )
        .unwrap();
        fs::write(root.join("eula.txt"), "eula=true\n").unwrap();
        fs::create_dir(root.join("logs")).unwrap();
        fs::write(
            root.join("logs/latest.log"),
            "[12:00:00] [Server thread/INFO]: Starting minecraft server version 1.20.4\n",
        )
        .unwrap();
        fs::create_dir(root.join("mods")).unwrap();
        fs::write(root.join("mods/example.jar"), b"mod").unwrap();
        let p = preview(root.to_str().unwrap()).unwrap();
        assert_eq!(p.minecraft_version, "1.20.4");
        assert_eq!(p.version_source, "latestLog");
        assert_eq!(p.port, 25566);
        assert_eq!(p.jar, "server.jar");
        assert!(!p.jar_rename_needed);
        assert!(p.eula_accepted);
        assert_eq!(p.file_count, 5);
        assert!(p.warnings.iter().any(|w| w == "unsafeWorld"));
        assert!(p.warnings.iter().any(|w| w == "moddedLauncher"));
    }

    #[test]
    fn preview_rejects_missing_folder() {
        assert!(preview("relative/path").is_err());
        assert!(preview("Z:\\this\\does\\not\\exist\\anywhere").is_err());
    }
}
