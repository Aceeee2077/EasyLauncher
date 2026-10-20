use crate::{
    error::{AppError, Result},
    logger::Logger,
    minecraft::properties::Properties,
    models::*,
    storage,
};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex, RwLock,
    },
};
use tauri::{AppHandle, Emitter};
use tokio::{
    process::{Child, ChildStdin},
    sync::Mutex as AsyncMutex,
};

pub struct Runtime {
    pub gate: AsyncMutex<()>,
    pub snapshot: Mutex<Snapshot>,
    pub console: Mutex<VecDeque<ConsoleLine>>,
    pub child: AsyncMutex<Option<Child>>,
    pub stdin: AsyncMutex<Option<ChildStdin>>,
    pub cancelled: AtomicBool,
    pub restart_attempts: AtomicU32,
    pub sequence: std::sync::atomic::AtomicU64,
}
impl Runtime {
    pub(crate) fn new(id: &str, status: Status, message: Option<String>) -> Self {
        Self {
            gate: AsyncMutex::new(()),
            snapshot: Mutex::new(Snapshot {
                id: id.into(),
                status,
                pid: None,
                started_at: None,
                exit_code: None,
                exit_reason: None,
                message,
                players: Vec::new(),
            }),
            console: Mutex::new(VecDeque::new()),
            child: AsyncMutex::new(None),
            stdin: AsyncMutex::new(None),
            cancelled: AtomicBool::new(false),
            restart_attempts: AtomicU32::new(0),
            sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn idle(&self) -> Result<()> {
        if self.snapshot().status.active() || self.snapshot().status == Status::Installing {
            Err(AppError::user(
                "Stop the server and wait for the current operation to finish first.",
            ))
        } else {
            Ok(())
        }
    }
}
pub struct Manager {
    pub process_group: super::job::ProcessGroup,
    pub root: PathBuf,
    pub servers: PathBuf,
    pub java: PathBuf,
    app: Option<AppHandle>,
    client: RwLock<reqwest::Client>,
    pub logger: Logger,
    pub runtimes: Mutex<HashMap<String, Arc<Runtime>>>,
    pub monitors: Mutex<HashMap<String, Arc<Mutex<crate::monitor::MonitorState>>>>,
    pub player_stats: Mutex<HashMap<String, crate::player_stats::Store>>,
    pub tunnels: AsyncMutex<HashMap<String, crate::tunnel::Runtime>>,
    pub shutting_down: AtomicBool,
    pub settings_gate: AsyncMutex<()>,
    pub java_gate: AsyncMutex<()>,
    console_pending: Mutex<HashMap<String, Vec<ConsoleLine>>>,
    console_notify: tokio::sync::Notify,
}
impl Manager {
    pub fn new(root: PathBuf, app: AppHandle) -> Result<Arc<Self>> {
        Self::initialize(root, Some(app))
    }
    /// Open a manager without a Tauri application handle.
    ///
    /// Window notifications and downloads progress events are skipped. Used by
    /// the headless acceptance harness and by embedders that do not run Tauri.
    #[doc(hidden)]
    pub fn open(root: PathBuf) -> Result<Arc<Self>> {
        Self::initialize(root, None)
    }
    fn initialize(root: PathBuf, app: Option<AppHandle>) -> Result<Arc<Self>> {
        fs::create_dir_all(&root)?;
        storage::no_links(&root)?;
        let servers = storage::child(&root, "servers")?;
        fs::create_dir_all(&servers)?;
        let java = storage::child(&root, "java")?;
        fs::create_dir_all(&java)?;
        let logger = Logger::new(storage::child(&root, "logs")?);
        let saved = load_settings(&root).unwrap_or_default();
        let client = match build_client(saved.proxy.as_deref()) {
            Ok(client) => client,
            Err(e) => {
                logger.log(
                    "WARN",
                    &format!("Ignoring the saved proxy setting at startup: {e}"),
                );
                build_client(None)?
            }
        };
        let m = Arc::new(Self {
            process_group: super::job::ProcessGroup::new()?,
            root,
            servers,
            java,
            app,
            client: RwLock::new(client),
            logger,
            runtimes: Mutex::new(HashMap::new()),
            monitors: Mutex::new(HashMap::new()),
            player_stats: Mutex::new(HashMap::new()),
            tunnels: AsyncMutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
            settings_gate: AsyncMutex::new(()),
            java_gate: AsyncMutex::new(()),
            console_pending: Mutex::new(HashMap::new()),
            console_notify: tokio::sync::Notify::new(),
        });
        m.logger.log(
            "INFO",
            "EasyLauncher started. Persisted process states are never trusted.",
        );
        for entry in fs::read_dir(&m.servers)?.flatten() {
            if entry.path().is_dir() {
                if let Ok(s) = storage::read_server(&entry.path()) {
                    let status = if s.installed {
                        Status::Stopped
                    } else {
                        Status::InstallationFailed
                    };
                    m.runtimes
                        .lock()
                        .unwrap()
                        .insert(s.id.clone(), Arc::new(Runtime::new(&s.id, status, None)));
                } else {
                    m.logger.log(
                        "WARN",
                        &format!("Skipped invalid metadata at {}", entry.path().display()),
                    );
                }
            }
        }
        if m.app.is_some() {
            Self::spawn_console_flusher(&m);
        }
        Ok(m)
    }
    fn spawn_console_flusher(m: &Arc<Self>) {
        let weak = Arc::downgrade(m);
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(current) = weak.upgrade() else { break };
                current.console_notify.notified().await;
                drop(current);
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                let Some(current) = weak.upgrade() else { break };
                let batches = {
                    let mut pending = current
                        .console_pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    std::mem::take(&mut *pending)
                };
                for (id, lines) in batches {
                    if !lines.is_empty() {
                        current.emit("server-console-batch", ConsoleBatch { id, lines });
                    }
                }
            }
        });
    }
    pub fn ready(&self) -> Result<()> {
        if self.shutting_down.load(Ordering::SeqCst) {
            Err(AppError::user(
                "EasyLauncher is shutting down. Please wait for the servers to stop.",
            ))
        } else {
            Ok(())
        }
    }
    pub fn client(&self) -> reqwest::Client {
        self.client
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn runtime(&self, id: &str) -> Result<Arc<Runtime>> {
        storage::validate_id(id)?;
        self.runtimes
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| {
                AppError::user("This server could not be found. Refresh your server list.")
                    .with_code("server.notFound")
            })
    }
    pub fn dir(&self, id: &str) -> Result<PathBuf> {
        storage::managed(&self.servers, id)
    }
    pub fn metadata(&self, id: &str) -> Result<Server> {
        storage::read_server(&self.dir(id)?)
    }
    pub fn emit_status(&self, r: &Runtime) {
        self.emit("server-status", r.snapshot());
    }
    pub fn emit<T: serde::Serialize + Clone>(&self, name: &str, payload: T) {
        if let Some(app) = &self.app {
            let _ = app.emit(name, payload);
        }
    }
    /// Publishes a user-facing notification that stays useful after the page changes.
    pub fn notice(&self, kind: &str, id: &str, detail: Option<String>) {
        let server_name = self
            .metadata(id)
            .map(|s| s.name)
            .unwrap_or_else(|_| id.to_string());
        self.emit(
            "notice",
            serde_json::json!({
                "kind": kind,
                "serverId": id,
                "serverName": server_name,
                "detail": detail,
                "at": chrono::Utc::now().to_rfc3339(),
            }),
        );
    }
    pub fn transition(&self, r: &Runtime, status: Status, message: Option<String>) -> Result<()> {
        {
            let mut s = r.snapshot.lock().unwrap();
            if s.status != status && !s.status.allows(status) {
                return Err(AppError::user(format!(
                    "Cannot change server state from {:?} to {:?}.",
                    s.status, status
                )));
            }
            s.status = status;
            s.message = message;
        }
        self.emit_status(r);
        Ok(())
    }
    pub fn line(&self, r: &Runtime, stream: &str, text: &str) {
        let line = ConsoleLine {
            id: r.snapshot().id,
            sequence: r.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            stream: stream.into(),
            text: text.chars().take(16384).collect(),
        };
        {
            let mut history = r.console.lock().unwrap_or_else(|e| e.into_inner());
            history.push_back(line.clone());
            while history.len() > 1500 {
                history.pop_front();
            }
        }
        if self.app.is_some() {
            {
                let mut pending = self
                    .console_pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let queue = pending.entry(line.id.clone()).or_default();
                if queue.len() >= 1500 {
                    queue.drain(..queue.len() - 1499);
                }
                queue.push(line);
            }
            self.console_notify.notify_one();
        }
    }
    pub fn clear_console(&self, r: &Runtime) {
        r.console.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.console_pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&r.snapshot().id);
    }
    pub fn list(&self) -> Result<Vec<ServerView>> {
        let ids: Vec<_> = self.runtimes.lock().unwrap().keys().cloned().collect();
        let mut list = Vec::new();
        for id in ids {
            if let Ok(server) = self.metadata(&id) {
                list.push(ServerView {
                    server,
                    runtime: self.runtime(&id)?.snapshot(),
                });
            }
        }
        list.sort_by(|a, b| a.server.created_at.cmp(&b.server.created_at));
        Ok(list)
    }
    pub fn properties(&self, id: &str) -> Result<Properties> {
        let p = storage::child(&self.dir(id)?, "server.properties")?;
        Ok(Properties::parse(&fs::read_to_string(p)?))
    }
    pub async fn save_properties(
        &self,
        id: &str,
        values: Option<std::collections::BTreeMap<String, String>>,
        raw: Option<String>,
    ) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _gate = r.gate.lock().await;
        r.idle()?;
        let mut p = self.properties(id)?;
        if let Some(raw) = raw {
            if raw.len() > 1024 * 1024 {
                return Err(AppError::user("Configuration must be smaller than 1 MB."));
            }
            p = Properties::parse(&raw);
        }
        if let Some(values) = values {
            for (k, v) in values {
                p.set(&k, &v);
            }
        }
        p.validate()?;
        let mut s = self.metadata(id)?;
        s.port = p
            .values
            .get("server-port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(s.port);
        storage::atomic_write(
            &storage::child(&self.dir(id)?, "server.properties")?,
            p.serialize().as_bytes(),
        )?;
        storage::save_server(&self.servers, &s)?;
        Ok(())
    }
    /// Show what a built-in template would change without touching the file.
    pub fn preview_properties_template(
        &self,
        id: &str,
        template: &str,
    ) -> Result<Vec<crate::minecraft::properties::PropertyChange>> {
        let pairs = crate::minecraft::properties::template(template)?;
        Ok(crate::minecraft::properties::preview(
            &self.properties(id)?,
            &pairs,
        ))
    }
    /// Apply only the template's whitelisted keys, leaving comments and other keys intact.
    pub async fn apply_properties_template(&self, id: &str, template: &str) -> Result<()> {
        let pairs = crate::minecraft::properties::template(template)?;
        let values = pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.save_properties(id, Some(values), None).await
    }
    pub async fn create(self: &Arc<Self>, input: CreateServer) -> Result<Server> {
        self.ready()?;
        if !input.eula_accepted {
            return Err(AppError::user(
                "Please read and explicitly accept the Minecraft EULA before creating a server.",
            ));
        }
        if input.name.trim().is_empty() || input.name.chars().count() > 64 {
            return Err(AppError::user(
                "Choose a server name between 1 and 64 characters.",
            ));
        }
        let minecraft_version = input.minecraft_version.trim();
        if minecraft_version.is_empty()
            || minecraft_version.len() > 64
            || !minecraft_version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_' | b' '))
        {
            return Err(AppError::user(
                "Select a Minecraft version from the list. Releases and snapshots are supported.",
            ));
        }
        validate_memory(input.memory_min_mb, input.memory_max_mb)?;
        let client = self.client();
        let required = crate::minecraft::versions::requirement(
            &client,
            minecraft_version,
            input.server_type,
            self.settings()?.download_source,
        )
        .await?;
        let java = crate::java::detector::validate(&input.java_path, required).await?;
        self.ready()?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let s = Server {
            schema_version: 1,
            id: id.clone(),
            name: input.name.trim().into(),
            minecraft_version: minecraft_version.into(),
            server_type: input.server_type,
            memory_min_mb: input.memory_min_mb,
            memory_max_mb: input.memory_max_mb,
            jvm_preset: JvmPreset::Balanced,
            jvm_flags: Vec::new(),
            port: input.port,
            created_at: now.clone(),
            last_started_at: None,
            java_path: Some(java.path),
            eula_accepted_at: Some(now),
            installed: false,
            auto_restart: false,
            required_java: Some(required),
            artifact: None,
        };
        let mut p =
            Properties::parse("# Managed by EasyLauncher. Unknown properties are preserved.\n");
        for (k, v) in [
            ("server-port", input.port.to_string()),
            ("max-players", input.max_players.to_string()),
            ("gamemode", input.game_mode),
            ("difficulty", input.difficulty),
            ("online-mode", input.online_mode.to_string()),
            ("motd", s.name.clone()),
            ("level-name", "world".into()),
            ("pvp", "true".into()),
            ("spawn-protection", "16".into()),
            ("view-distance", "10".into()),
            ("simulation-distance", "10".into()),
            ("allow-flight", "false".into()),
            ("white-list", "false".into()),
            ("enable-rcon", "false".into()),
        ] {
            p.set(k, &v);
        }
        p.validate()?;
        let dir = self.dir(&id)?;
        fs::create_dir(&dir)?;
        let result = (|| {
            storage::save_server(&self.servers, &s)?;
            storage::atomic_write(
                &storage::child(&dir, "server.properties")?,
                p.serialize().as_bytes(),
            )?;
            storage::atomic_write(
                &storage::child(&dir, "eula.txt")?,
                b"# Explicitly accepted in EasyLauncher\neula=true\n",
            )?;
            for name in ["logs", "backups", "mods", "plugins"] {
                fs::create_dir(storage::child(&dir, name)?)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
        self.runtimes.lock().unwrap().insert(
            id.clone(),
            Arc::new(Runtime::new(&id, Status::Stopped, None)),
        );
        self.logger.log("INFO", &format!("Created server {id}"));
        Ok(s)
    }
    pub async fn install(self: &Arc<Self>, id: &str) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _gate = r.gate.lock().await;
        self.ready()?;
        r.idle()?;
        let mut s = self.metadata(id)?;
        if s.installed {
            return Err(AppError::user("This server is already installed.")
                .with_code("server.alreadyInstalled"));
        }
        r.cancelled.store(false, Ordering::Relaxed);
        self.transition(
            &r,
            Status::Installing,
            Some("Resolving server artifact…".into()),
        )?;
        let client = self.client();
        let source = self.settings()?.download_source;
        let result: Result<()> = tokio::select! { _=super::installer::cancelled(&r.cancelled)=>Err(AppError::user("Installation cancelled. You can retry from the server card.").with_code("server.installCancelled")), result=async {
            let artifact = super::installer::ServerInstaller::resolve(
                &client,
                &s.minecraft_version,
                s.server_type,
                source,
            )
            .await?;
            crate::java::detector::validate(s.java_path.as_deref().unwrap_or(""), artifact.java)
                .await?;
            let dest = storage::child(&self.dir(id)?, "server.jar")?;
            super::installer::download(
                &client,
                &artifact,
                &dest,
                &r.cancelled,
                |p| {
                    self.emit("download-progress", p);
                },
                id,
            )
            .await?;
            s.installed = true;
            s.artifact = Some(artifact.label);
            s.required_java = Some(artifact.java);
            storage::save_server(&self.servers, &s)?;
            Ok(())
        }=>result };
        match result {
            Ok(()) => {
                self.transition(
                    &r,
                    Status::Stopped,
                    Some("Installation complete. Ready to start.".into()),
                )?;
                self.logger.log("INFO", &format!("Installed server {id}"));
                Ok(())
            }
            Err(e) => {
                if let Ok(path) = storage::child(&self.dir(id)?, "server.jar.part") {
                    let _ = fs::remove_file(path);
                }
                self.logger.log("ERROR", &format!("Installation {id}: {e}"));
                self.transition(&r, Status::InstallationFailed, Some(e.message.clone()))?;
                Err(e)
            }
        }
    }
    pub async fn delete(&self, id: &str, confirmed: bool) -> Result<()> {
        self.ready()?;
        if !confirmed {
            return Err(AppError::user("Confirm permanent deletion first."));
        }
        let r = self.runtime(id)?;
        let _gate = r.gate.lock().await;
        r.idle()?;
        let dir = self.dir(id)?;
        if crate::tunnel::info(self, id).await?.running {
            return Err(AppError::user(
                "Stop this server's frpc tunnel before deleting the server.",
            ));
        }
        storage::validate_tree(&dir)?;
        // Tombstone rename removes the server from the catalogue before recursive removal.
        let trash = storage::child(&self.servers, &format!("deleted-{id}"))?;
        fs::rename(&dir, &trash)?;
        self.runtimes.lock().unwrap().remove(id);
        self.monitors.lock().unwrap().remove(id);
        self.player_stats.lock().unwrap().remove(id);
        self.tunnels.lock().await.remove(id);
        fs::remove_dir_all(trash)?;
        self.logger.log("INFO", &format!("Deleted server {id}"));
        Ok(())
    }
    pub fn settings(&self) -> Result<Settings> {
        load_settings(&self.root)
    }
    /// Rejects state-changing commands while read-only mode is on. Settings
    /// remain editable so the mode can be turned off again.
    pub fn ensure_writable(&self) -> Result<()> {
        if self.settings()?.read_only {
            Err(AppError::user(
                "Read-only mode is on. Turn it off in app settings to make changes.",
            )
            .with_code("app.readOnly"))
        } else {
            Ok(())
        }
    }
    pub async fn save_settings(&self, mut s: Settings) -> Result<()> {
        let _g = self.settings_gate.lock().await;
        if let Some(p) = &s.java_path {
            if !p.trim().is_empty() {
                crate::java::detector::inspect(p).await?;
            }
        }
        s.proxy = match s.proxy.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => {
                reqwest::Proxy::all(p).map_err(|_| {
                    AppError::user(
                        "Enter a valid HTTP or HTTPS proxy such as http://127.0.0.1:7890.",
                    )
                })?;
                Some(p.to_string())
            }
            _ => None,
        };
        let client = build_client(s.proxy.as_deref())?;
        storage::write_json(&storage::child(&self.root, "settings.json")?, &s)?;
        *self.client.write().unwrap_or_else(|e| e.into_inner()) = client;
        Ok(())
    }
    pub async fn update_auto_restart(&self, id: &str, enabled: bool) -> Result<Server> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        self.ready()?;
        let mut s = self.metadata(id)?;
        s.auto_restart = enabled;
        storage::save_server(&self.servers, &s)?;
        self.logger.log(
            "INFO",
            &format!(
                "Crash recovery {} for {id}",
                if enabled { "enabled" } else { "disabled" }
            ),
        );
        Ok(s)
    }
    pub async fn update_resources(
        &self,
        id: &str,
        min: u32,
        max: u32,
        java_path: String,
    ) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        r.idle()?;
        validate_memory(min, max)?;
        let mut s = self.metadata(id)?;
        let client = self.client();
        let required = crate::minecraft::versions::requirement(
            &client,
            &s.minecraft_version,
            s.server_type,
            self.settings()?.download_source,
        )
        .await?;
        let java = crate::java::detector::validate(&java_path, required).await?;
        s.memory_min_mb = min;
        s.memory_max_mb = max;
        s.java_path = Some(java.path);
        s.required_java = Some(required);
        storage::save_server(&self.servers, &s)
    }
    pub async fn update_jvm(
        &self,
        id: &str,
        preset: JvmPreset,
        flags: Vec<String>,
    ) -> Result<Server> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        r.idle()?;
        let mut s = self.metadata(id)?;
        let custom = if preset == JvmPreset::Custom {
            crate::server::process::validate_jvm_flags(&flags)?;
            flags
        } else {
            Vec::new()
        };
        s.jvm_preset = preset;
        s.jvm_flags = custom;
        storage::save_server(&self.servers, &s)?;
        self.logger
            .log("INFO", &format!("JVM preset for {id}: {preset:?}"));
        Ok(s)
    }
}
fn load_settings(root: &Path) -> Result<Settings> {
    let p = storage::child(root, "settings.json")?;
    if p.exists() {
        Ok(serde_json::from_slice(&fs::read(p)?)?)
    } else {
        Ok(Settings::default())
    }
}
pub fn build_client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent("EasyLauncher/0.1.0")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .https_only(true);
    if let Some(proxy) = proxy.map(str::trim).filter(|p| !p.is_empty()) {
        let proxy = reqwest::Proxy::all(proxy).map_err(|_| {
            AppError::user("Enter a valid HTTP or HTTPS proxy such as http://127.0.0.1:7890.")
        })?;
        builder = builder.proxy(proxy);
    }
    Ok(builder.build()?)
}
pub fn memory_info() -> (u64, u64, u32) {
    let mut s = sysinfo::System::new();
    s.refresh_memory();
    let total = s.total_memory() / 1024 / 1024;
    let available = s.available_memory() / 1024 / 1024;
    let rec = (total / 4).clamp(512, 4096) as u32;
    (total, available, rec)
}
pub fn validate_memory(min: u32, max: u32) -> Result<()> {
    let (total, available, _) = memory_info();
    if min < 256 || max < min || max as u64 > total.saturating_sub(1024).min(total * 80 / 100) {
        return Err(AppError::user("Choose at least 256 MB and keep maximum RAM above minimum. Leave at least 1 GB and 20% of system RAM for other apps.").with_code("server.memoryInvalid"));
    }
    if max as u64 > available.saturating_sub(512) {
        return Err(AppError::user("There is not enough available memory for this maximum. Close other applications or lower the RAM limit.").with_code("server.memoryLow"));
    }
    Ok(())
}
pub fn read_limited(path: &Path) -> Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    storage::no_links(path)?;
    let mut f = fs::File::open(path)?;
    let size = f.metadata()?.len();
    if size > 256 * 1024 {
        f.seek(SeekFrom::End(-256 * 1024))?;
    }
    let mut bytes = Vec::new();
    f.take(256 * 1024).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into())
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    fn setup(java: String) -> (tempfile::TempDir, Arc<Manager>, String) {
        let temp = tempfile::tempdir().unwrap();
        let m = Manager::initialize(temp.path().join("app"), None).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let dir = m.dir(&id).unwrap();
        fs::create_dir(&dir).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let s = Server {
            schema_version: 1,
            id: id.clone(),
            name: "Subprocess fixture".into(),
            minecraft_version: "1.21.4".into(),
            server_type: ServerType::Vanilla,
            memory_min_mb: 256,
            memory_max_mb: 512,
            jvm_preset: JvmPreset::Balanced,
            jvm_flags: Vec::new(),
            port,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_started_at: None,
            java_path: Some(java),
            eula_accepted_at: Some("fixture-only".into()),
            installed: true,
            auto_restart: false,
            required_java: Some(21),
            artifact: Some("TEST FIXTURE — no Minecraft code".into()),
        };
        storage::save_server(&m.servers, &s).unwrap();
        fs::write(
            dir.join("server.properties"),
            format!("server-port={port}\nlevel-name=world\nunknown-key=preserve-me\n"),
        )
        .unwrap();
        fs::write(dir.join("eula.txt"), "# Test fixture only\neula=true").unwrap();
        fs::write(dir.join("server.jar"), b"test fixture marker").unwrap();
        fs::create_dir(dir.join("backups")).unwrap();
        fs::create_dir_all(dir.join("world/region")).unwrap();
        fs::write(dir.join("world/region/data"), b"original world").unwrap();
        m.runtimes.lock().unwrap().insert(
            id.clone(),
            Arc::new(Runtime::new(&id, Status::Stopped, None)),
        );
        (temp, m, id)
    }
    async fn wait(m: &Manager, id: &str, status: Status) {
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                if m.runtime(id).unwrap().snapshot().status == status {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn backup_restore_properties_and_reopen() {
        let (temp, m, id) = setup("unused-in-this-test".into());
        let dir = m.dir(&id).unwrap();
        let backup = crate::backup::create_backup(
            m.clone(),
            id.clone(),
            crate::models::BackupReason::Manual,
        )
        .await
        .unwrap();
        fs::write(dir.join("world/region/data"), b"changed world").unwrap();
        assert!(
            crate::backup::restore_backup(m.clone(), id.clone(), backup.id.clone(), false)
                .await
                .is_err()
        );
        crate::backup::restore_backup(m.clone(), id.clone(), backup.id.clone(), true)
            .await
            .unwrap();
        assert_eq!(
            fs::read(dir.join("world/region/data")).unwrap(),
            b"original world"
        );
        let snapshots = crate::backup::list(&m, &id).unwrap();
        assert_eq!(snapshots.len(), 2);
        let safety = snapshots.iter().find(|b| b.safety).unwrap();
        let recovered = temp.path().join("safety-restore");
        crate::backup_store::restore(
            &dir.join("backups"),
            &dir.join("backups").join(&safety.id),
            &recovered,
        )
        .unwrap();
        assert_eq!(
            fs::read(recovered.join("world/region/data")).unwrap(),
            b"changed world"
        );
        fs::remove_dir_all(recovered).unwrap();
        let mut changes = std::collections::BTreeMap::new();
        changes.insert("max-players".into(), "12".into());
        m.save_properties(&id, Some(changes), None).await.unwrap();
        assert_eq!(
            m.properties(&id).unwrap().values["unknown-key"],
            "preserve-me"
        );
        let reopened = Manager::initialize(m.root.clone(), None).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 1);
        assert_eq!(
            reopened.runtime(&id).unwrap().snapshot().status,
            Status::Stopped
        );
        crate::backup::delete_backup(m.clone(), id.clone(), backup.id, true)
            .await
            .unwrap();
        assert_eq!(crate::backup::list(&m, &id).unwrap().len(), 1);
    }
    #[tokio::test]
    async fn live_backup_rejects_a_stopped_server_or_missing_rcon() {
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let err =
            crate::backup::create_consistent_backup(m.clone(), id.clone(), BackupReason::Manual)
                .await
                .unwrap_err();
        assert_eq!(err.code, "backup.notRunning");
        // Pretend Java is ready; no process is launched and RCON stays disabled.
        m.runtimes.lock().unwrap().insert(
            id.clone(),
            Arc::new(Runtime::new(&id, Status::Running, None)),
        );
        let err =
            crate::backup::create_consistent_backup(m.clone(), id.clone(), BackupReason::Manual)
                .await
                .unwrap_err();
        assert_eq!(err.code, "backup.rconRequired");
    }
    #[tokio::test]
    async fn upgrade_rejects_unconfirmed_same_version_and_downgrade() {
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let err = crate::upgrade::apply(m.clone(), id.clone(), "1.21.5".into(), false, false)
            .await
            .unwrap_err();
        assert_eq!(err.code, "upgrade.notConfirmed");
        let err = crate::upgrade::apply(m.clone(), id.clone(), "1.21.4".into(), true, true)
            .await
            .unwrap_err();
        assert_eq!(err.code, "upgrade.sameVersion");
        let err = crate::upgrade::apply(m.clone(), id.clone(), "1.20.1".into(), true, false)
            .await
            .unwrap_err();
        assert_eq!(err.code, "upgrade.downgradeUnconfirmed");
    }
    #[tokio::test]
    async fn insights_dependencies_diff_and_jvm_persist() {
        use std::io::Write;
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let dir = m.dir(&id).unwrap();
        m.update_jvm(&id, JvmPreset::Aikar, vec![]).await.unwrap();
        assert!(m
            .launch_info(&id)
            .unwrap()
            .args
            .contains(&"-XX:+UseG1GC".into()));
        let a = crate::backup::create_backup(m.clone(), id.clone(), BackupReason::Manual)
            .await
            .unwrap();
        fs::write(dir.join("world/region/data"), b"modified world").unwrap();
        fs::write(dir.join("world/new"), b"new file").unwrap();
        let b = crate::backup::create_backup(m.clone(), id.clone(), BackupReason::Manual)
            .await
            .unwrap();
        fs::remove_file(dir.join("backups").join(&a.id).join("files.json")).unwrap();
        let diff = crate::backup::compare(m.clone(), id.clone(), a.id.clone(), b.id)
            .await
            .unwrap();
        assert_eq!((diff.added, diff.modified), (1, 1));
        crate::player_stats::observe(&m, &id, "Alex", true).unwrap();
        crate::player_stats::observe(&m, &id, "Alex", true).unwrap();
        crate::player_stats::finish(&m, &id).unwrap();
        assert_eq!(
            crate::player_stats::report(&m, &id).unwrap().players["Alex"].visits,
            1
        );
        let reopened = Manager::initialize(m.root.clone(), None).unwrap();
        assert_eq!(reopened.metadata(&id).unwrap().jvm_preset, JvmPreset::Aikar);
        assert_eq!(
            crate::player_stats::report(&reopened, &id)
                .unwrap()
                .sessions
                .len(),
            1
        );
        let mut server = m.metadata(&id).unwrap();
        server.server_type = ServerType::Fabric;
        storage::save_server(&m.servers, &server).unwrap();
        fs::create_dir_all(dir.join("mods")).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(dir.join("mods/test.jar")).unwrap());
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"id":"example","version":"1","depends":{"missing":"*"}}"#)
            .unwrap();
        zip.finish().unwrap();
        let report = crate::dependencies::check(m.clone(), id.clone(), false)
            .await
            .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "missing" && i.target == "missing"));
        assert!(!report.remote_checked);
        m.delete(&id, true).await.unwrap();
        assert!(m.player_stats.lock().unwrap().is_empty());
        assert!(!dir.exists());
    }
    #[test]
    fn diagnosis_ignores_prior_run_and_reads_early_stderr() {
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let reports = m.dir(&id).unwrap().join("crash-reports");
        fs::create_dir(&reports).unwrap();
        let report = reports.join("old.txt");
        fs::write(&report, b"java.lang.OutOfMemoryError").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&report)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        let runtime = m.runtime(&id).unwrap();
        runtime.snapshot.lock().unwrap().started_at = Some(chrono::Utc::now().to_rfc3339());
        m.line(&runtime, "stderr", "Unrecognized VM option 'UnknownFlag'");
        let diagnosis = crate::server::diagnostics::diagnose(&m, &id).unwrap();
        assert_eq!(diagnosis.source.as_deref(), Some("console"));
        assert!(diagnosis.findings.iter().any(|f| f.id == "jvmOptions"));
        assert!(!diagnosis.findings.iter().any(|f| f.id == "outOfMemory"));
    }
    #[tokio::test]
    async fn log_assistant_uses_reviewed_excerpt_without_persisting_key() {
        use std::io::{Read, Write};
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.contains("Bearer fixture-secret"));
            assert!(request.contains("reviewed excerpt only"));
            let response = br#"{"choices":[{"message":{"content":"Fixture diagnosis"}}]}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).unwrap();
            socket.write_all(response).unwrap();
        });
        let text = crate::log_assistant::analyze(
            &m,
            &id,
            &endpoint,
            "fixture-model",
            "fixture-secret",
            "reviewed excerpt only",
            "en",
        )
        .await
        .unwrap();
        assert_eq!(text, "Fixture diagnosis");
        worker.join().unwrap();
        let settings = serde_json::to_string(&m.settings().unwrap()).unwrap();
        assert!(!settings.contains("fixture-secret"));
    }
    #[tokio::test]
    #[ignore = "Requires explicitly built test-only subprocess; never runs Minecraft"]
    async fn managed_frpc_fixture_lifecycle() {
        let fixture = std::env::var("EASYLAUNCHER_TEST_PROCESS")
            .expect("Build the test-only process fixture first");
        let (_temp, m, id) = setup("unused-in-this-test".into());
        let dir = m.root.join("tunnel-fixture");
        fs::create_dir(&dir).unwrap();
        let exe = dir.join(if cfg!(windows) { "frpc.exe" } else { "frpc" });
        fs::copy(fixture, &exe).unwrap();
        let config_path = dir.join("frpc.toml");
        fs::write(&config_path, b"# No real frp service; fixture only").unwrap();
        let config = crate::tunnel::Config {
            executable: exe.to_string_lossy().into(),
            config_path: config_path.to_string_lossy().into(),
            address: "fixture.invalid:25565".into(),
        };
        let started = crate::tunnel::start(&m, &id, config.clone()).await.unwrap();
        assert!(started.running);
        assert!(started.pid.is_some());
        assert!(crate::tunnel::start(&m, &id, config.clone()).await.is_err());
        assert!(m.delete(&id, true).await.is_err());
        assert!(!crate::tunnel::stop(&m, &id).await.unwrap().running);
        assert_eq!(
            crate::tunnel::info(&m, &id).await.unwrap().config.address,
            config.address
        );
        crate::tunnel::start(&m, &id, config).await.unwrap();
        m.shutdown().await.unwrap();
        assert!(!crate::tunnel::info(&m, &id).await.unwrap().running);
    }
    #[tokio::test]
    #[ignore = "Requires explicitly built test-only subprocess; never runs Minecraft"]
    async fn subprocess_lifecycle_without_minecraft() {
        let java = std::env::var("EASYLAUNCHER_TEST_PROCESS")
            .expect("Build easylauncher-process-fixture and provide its absolute path");
        let (_temp, m, id) = setup(java);
        let (a, b) = tokio::join!(m.start(&id), m.start(&id));
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "exactly one duplicate start must succeed"
        );
        wait(&m, &id, Status::Running).await;
        assert!(crate::backup::create_backup(
            m.clone(),
            id.clone(),
            crate::models::BackupReason::Manual
        )
        .await
        .is_err());
        assert!(m.send(&id, "say Hello\nstop").await.is_err());
        m.send(&id, "say Hello").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if m.runtime(&id)
                    .unwrap()
                    .console
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|l| l.stream == "stdout" && l.text.contains("Received: say Hello"))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        m.restart(&id).await.unwrap();
        wait(&m, &id, Status::Running).await;
        m.send(&id, "fixture-crash").await.unwrap();
        wait(&m, &id, Status::Crashed).await;
        assert_eq!(m.runtime(&id).unwrap().snapshot().exit_code, Some(7));
        m.start(&id).await.unwrap();
        wait(&m, &id, Status::Running).await;
        m.send(&id, "stop").await.unwrap();
        wait(&m, &id, Status::Stopped).await;
        m.start(&id).await.unwrap();
        wait(&m, &id, Status::Running).await;
        m.shutdown().await.unwrap();
        assert_eq!(m.runtime(&id).unwrap().snapshot().status, Status::Stopped);
        assert!(m.start(&id).await.is_err());
    }
}
