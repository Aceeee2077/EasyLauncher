use super::manager::{validate_memory, Manager, Runtime};
use crate::{
    error::{AppError, Result},
    models::{LaunchInfo, Server, Status},
    storage,
};
use std::{
    process::Stdio,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::Command,
};

const MAX_AUTO_RESTARTS: u32 = 5;
const RESTART_DELAYS: [u64; 5] = [5, 10, 20, 40, 60];

impl Manager {
    pub async fn start(self: &Arc<Self>, id: &str) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        r.restart_attempts.store(0, Ordering::Relaxed);
        self.start_locked(id, &r).await
    }
    async fn start_locked(self: &Arc<Self>, id: &str, r: &Arc<Runtime>) -> Result<()> {
        self.ready()?;
        // The crash supervisor already owns this runtime while it waits to restart.
        if r.snapshot().status != Status::Restarting {
            r.idle()?;
        }
        if r.child.lock().await.is_some() {
            return Err(AppError::user("This server already has a managed process.")
                .with_code("server.alreadyRunning"));
        }
        let mut s = self.metadata(id)?;
        if s.jvm_preset == crate::models::JvmPreset::Custom {
            validate_jvm_flags(&s.jvm_flags)?;
        }
        if !s.installed {
            return Err(AppError::user("Install the server files before starting.")
                .with_code("server.notInstalled"));
        }
        if s.eula_accepted_at.is_none() {
            return Err(AppError::user("Accept the Minecraft EULA before starting.")
                .with_code("server.eulaNotAccepted"));
        }
        validate_memory(s.memory_min_mb, s.memory_max_mb)?;
        let dir = self.dir(id)?;
        let eula = std::fs::read_to_string(storage::child(&dir, "eula.txt")?)?;
        if !crate::minecraft::properties::Properties::parse(&eula)
            .values
            .get("eula")
            .map(|s| s == "true")
            .unwrap_or(false)
        {
            return Err(AppError::user(
                "The Minecraft EULA has not been accepted in this server's eula.txt.",
            )
            .with_code("server.eulaNotAccepted"));
        }
        let properties = self.properties(id)?;
        properties.validate()?;
        let port = properties
            .values
            .get("server-port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(s.port);
        crate::network::check_port(port)?;
        let required = s.required_java.unwrap_or_else(|| {
            crate::java::version::required_java(&s.minecraft_version, s.server_type, None)
        });
        let j =
            crate::java::detector::validate(s.java_path.as_deref().unwrap_or(""), required).await?;
        let jar = storage::child(&dir, "server.jar")?;
        if !jar.is_file() {
            return Err(AppError::user(
                "The server JAR is missing. Re-create the server or restore its files.",
            )
            .with_code("server.jarMissing"));
        }
        for e in std::fs::read_dir(&dir)?.flatten() {
            if e.file_name().to_string_lossy().starts_with("restore-") {
                return Err(AppError::user("This server has preserved restore recovery files. Open its folder and recover or remove the restore folder before starting.").with_code("server.restorePending"));
            }
        }
        let checked_dir = dir.clone();
        tokio::task::spawn_blocking(move || storage::validate_tree(&checked_dir))
            .await
            .map_err(|e| AppError::new("Could not validate server files.", e))??;
        self.ready()?;
        // Persist before spawning so a write failure cannot leave an untracked process.
        s.last_started_at = Some(chrono::Utc::now().to_rfc3339());
        s.port = port;
        storage::save_server(&self.servers, &s)?;
        let mut command = Command::new(j.path);
        command
            .args(launch_args(&s))
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|e| {
            AppError::new(
                "Could not start Java. Check the configured runtime and folder permissions.",
                e,
            )
            .with_code("server.javaStartFailed")
        })?;
        self.process_group.attach(&child)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        {
            let mut snap = r.snapshot.lock().unwrap();
            snap.pid = child.id();
            snap.started_at = s.last_started_at;
            snap.exit_code = None;
            snap.exit_reason = None;
            snap.players.clear();
        }
        *r.stdin.lock().await = stdin;
        *r.child.lock().await = Some(child);
        self.transition(
            r,
            Status::Starting,
            Some("Java started. Waiting for Minecraft to finish loading…".into()),
        )?;
        self.logger.log("INFO", &format!("Started Java for {id}"));
        let mut readers = Vec::new();
        if let Some(out) = stdout {
            readers.push(tauri::async_runtime::spawn(read_output(
                self.clone(),
                r.clone(),
                out,
                "stdout",
            )));
        }
        if let Some(out) = stderr {
            readers.push(tauri::async_runtime::spawn(read_output(
                self.clone(),
                r.clone(),
                out,
                "stderr",
            )));
        }
        let m = self.clone();
        let runtime = r.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let result = {
                    let mut child = runtime.child.lock().await;
                    match child.as_mut() {
                        Some(c) => c.try_wait(),
                        None => break,
                    }
                };
                match result {
                    Ok(Some(exit)) => {
                        for reader in readers.drain(..) {
                            let _ = tokio::time::timeout(Duration::from_secs(3), reader).await;
                        }
                        *runtime.stdin.lock().await = None;
                        let recent: Vec<String> = runtime
                            .console
                            .lock()
                            .unwrap()
                            .iter()
                            .rev()
                            .take(80)
                            .map(|line| line.text.clone())
                            .collect();
                        {
                            let mut snap = runtime.snapshot.lock().unwrap();
                            snap.exit_code = exit.code();
                            snap.pid = None;
                            snap.players.clear();
                            let expected = snap.status == Status::Stopping && exit.success();
                            snap.exit_reason = if expected {
                                None
                            } else {
                                Some(attribute_exit(exit.code(), &recent))
                            };
                            snap.status = if expected {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                            snap.message = Some(if expected {
                                "Server stopped safely.".into()
                            } else {
                                format!(
                                    "Server exited unexpectedly (exit code {}). {}",
                                    exit.code()
                                        .map(|c| c.to_string())
                                        .unwrap_or_else(|| "unknown".into()),
                                    snap.exit_reason.clone().unwrap_or_default()
                                )
                            });
                        }
                        if let Err(e) = crate::player_stats::finish(&m, &runtime.snapshot().id) {
                            m.logger
                                .log("WARN", &format!("Could not finish player sessions: {e}"));
                        }
                        // Publish final state only after the handle is reaped and streams drained.
                        *runtime.child.lock().await = None;
                        m.emit_status(&runtime);
                        m.logger.log(
                            "INFO",
                            &format!("Java exited for {}: {exit}", runtime.snapshot().id),
                        );
                        if runtime.snapshot().status == Status::Crashed {
                            let crashed_id = runtime.snapshot().id;
                            let auto = m
                                .metadata(&crashed_id)
                                .map(|s| s.auto_restart)
                                .unwrap_or(false);
                            if auto && !m.shutting_down.load(Ordering::SeqCst) {
                                let attempt =
                                    runtime.restart_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                                if attempt <= MAX_AUTO_RESTARTS {
                                    let delay = restart_delay(attempt);
                                    let _ = m.transition(
                                        &runtime,
                                        Status::Restarting,
                                        Some(restart_message(attempt, delay)),
                                    );
                                    spawn_auto_restart(
                                        m.clone(),
                                        runtime.clone(),
                                        crashed_id.clone(),
                                        attempt,
                                    );
                                } else {
                                    m.logger.log(
                                        "WARN",
                                        &format!(
                                            "Crash recovery limit reached for {crashed_id}; waiting for manual start"
                                        ),
                                    );
                                }
                            }
                            let manager = m.clone();
                            let server_id = crashed_id;
                            match tokio::task::spawn_blocking(move || {
                                super::diagnostics::diagnose(&manager, &server_id)
                            })
                            .await
                            {
                                Ok(Ok(report)) => m.emit("server-diagnosis", report),
                                Ok(Err(e)) => {
                                    m.logger.log("WARN", &format!("Diagnosis failed: {e}"))
                                }
                                Err(e) => {
                                    m.logger.log("WARN", &format!("Diagnosis task failed: {e}"))
                                }
                            }
                        }
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        m.logger
                            .log("ERROR", &format!("Could not inspect Java process: {e}"));
                    }
                }
            }
        });
        Ok(())
    }
    /// Read-only view of the exact Java invocation used to start this server.
    pub fn launch_info(&self, id: &str) -> Result<LaunchInfo> {
        let s = self.metadata(id)?;
        let dir = self.dir(id)?;
        let java_path = s
            .java_path
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "java".into());
        let args = launch_args(&s);
        let command = std::iter::once(quote(&java_path))
            .chain(args.iter().map(|a| quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(LaunchInfo {
            java_path,
            working_directory: dir.display().to_string(),
            args,
            command,
            jar_present: dir.join("server.jar").is_file(),
            installed: s.installed,
        })
    }
    pub async fn send(&self, id: &str, command: &str) -> Result<()> {
        let r = self.runtime(id)?;
        let state = r.snapshot().status;
        if !matches!(state, Status::Starting | Status::Running | Status::Stopping) {
            return Err(AppError::user(
                "Start the server before sending console commands.",
            ));
        }
        if command.trim().is_empty() || command.len() > 4096 || command.contains(['\n', '\r', '\0'])
        {
            return Err(AppError::user(
                "Enter one console command, up to 4096 characters.",
            ));
        }
        let mut stdin = r.stdin.lock().await;
        let pipe = stdin
            .as_mut()
            .ok_or_else(|| AppError::user("The server's command input is no longer available."))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            pipe.write_all(command.trim().as_bytes()).await?;
            pipe.write_all(b"\n").await?;
            pipe.flush().await
        })
        .await
        .map_err(|_| AppError::user("The server did not accept the command in time."))??;
        self.line(&r, "input", command);
        if command.trim().trim_start_matches('/') == "stop"
            && matches!(r.snapshot().status, Status::Starting | Status::Running)
        {
            self.transition(
                &r,
                Status::Stopping,
                Some("Saving worlds and stopping safely…".into()),
            )?;
        }
        Ok(())
    }
    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<()> {
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        self.stop_locked(id, &r).await
    }
    async fn stop_locked(&self, id: &str, r: &Arc<Runtime>) -> Result<()> {
        if r.snapshot().status == Status::Restarting {
            self.transition(
                r,
                Status::Stopped,
                Some("Automatic restart cancelled.".into()),
            )?;
            self.logger
                .log("INFO", &format!("Cancelled crash recovery for {id}"));
            return Ok(());
        }
        if !r.snapshot().status.active() {
            return Ok(());
        }
        if r.snapshot().status != Status::Stopping {
            self.transition(
                r,
                Status::Stopping,
                Some("Saving worlds and stopping safely…".into()),
            )?;
            if let Err(e) = self.send(id, "stop").await {
                self.logger.log("WARN", &e.to_string());
            }
        }
        for _ in 0..180 {
            if r.child.lock().await.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        self.line(r,"warning","Graceful shutdown exceeded 45 seconds. Forcing Java to exit; recent world changes may be lost.");
        self.logger
            .log("WARN", &format!("Forced shutdown for {id}"));
        if let Some(child) = r.child.lock().await.as_mut() {
            child.start_kill()?;
        }
        for _ in 0..40 {
            if r.child.lock().await.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(AppError::user(
            "Java has not exited yet. Wait and check the console before closing EasyLauncher.",
        ))
    }
    pub async fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        r.restart_attempts.store(0, Ordering::Relaxed);
        self.stop_locked(id, &r).await?;
        self.start_locked(id, &r).await
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        self.shutting_down.store(true, Ordering::SeqCst);
        let tunnel_error = crate::tunnel::stop_all(self).await.err();
        let ids: Vec<String> = self.runtimes.lock().unwrap().keys().cloned().collect();
        let mut jobs = Vec::new();
        for id in ids {
            let m = self.clone();
            if let Ok(r) = m.runtime(&id) {
                r.cancelled.store(true, Ordering::Relaxed);
            }
            jobs.push(tauri::async_runtime::spawn(
                async move { m.stop(&id).await },
            ));
        }
        for job in jobs {
            job.await
                .map_err(|e| AppError::new("A server shutdown task failed.", e))??;
        }
        if let Some(error) = tunnel_error {
            return Err(error);
        }
        Ok(())
    }
}
/// The single source of truth for the server JVM arguments.
pub fn launch_args(s: &Server) -> Vec<String> {
    let mut args = vec![
        format!("-Xms{}M", s.memory_min_mb),
        format!("-Xmx{}M", s.memory_max_mb),
    ];
    args.extend(preset_flags(s.jvm_preset, &s.jvm_flags, s.memory_max_mb));
    args.extend([
        "-Dfile.encoding=UTF-8".into(),
        "-jar".into(),
        "server.jar".into(),
        "nogui".into(),
    ]);
    args
}
/// Flags injected by the selected preset. Memory limits stay user-controlled.
pub fn preset_flags(
    preset: crate::models::JvmPreset,
    custom: &[String],
    max_mb: u32,
) -> Vec<String> {
    use crate::models::JvmPreset::*;
    match preset {
        Balanced => Vec::new(),
        Aikar => aikar_flags(max_mb),
        Paper => paper_flags(max_mb),
        Custom => custom
            .iter()
            .filter(|flag| valid_jvm_flag(flag))
            .cloned()
            .collect(),
    }
}
/// Aikar's community-standard G1 tuning, with the large-heap variant above 12 GB.
fn aikar_flags(max_mb: u32) -> Vec<String> {
    let large = max_mb >= 12 * 1024;
    vec![
        "-XX:+UseG1GC".into(),
        "-XX:+ParallelRefProcEnabled".into(),
        "-XX:MaxGCPauseMillis=200".into(),
        "-XX:+UnlockExperimentalVMOptions".into(),
        "-XX:+DisableExplicitGC".into(),
        "-XX:+AlwaysPreTouch".into(),
        format!("-XX:G1NewSizePercent={}", if large { 40 } else { 30 }),
        format!("-XX:G1MaxNewSizePercent={}", if large { 50 } else { 40 }),
        format!("-XX:G1HeapRegionSize={}", if large { "16M" } else { "8M" }),
        format!("-XX:G1ReservePercent={}", if large { 15 } else { 20 }),
        format!(
            "-XX:InitiatingHeapOccupancyPercent={}",
            if large { 20 } else { 15 }
        ),
        "-XX:G1HeapWastePercent=5".into(),
        "-XX:G1MixedGCCountTarget=4".into(),
        "-XX:G1MixedGCLiveThresholdPercent=90".into(),
        "-XX:G1RSetUpdatingPauseTimePercent=5".into(),
        "-XX:SurvivorRatio=32".into(),
        "-XX:+PerfDisableSharedMem".into(),
        "-XX:MaxTenuringThreshold=1".into(),
        "-Dusing.aikars.flags=https://mcflags.emc.gs".into(),
        "-Daikars.new.flags=true".into(),
    ]
}
/// Aikar's flags with a tighter pause target for Paper's lower-latency tick loop.
fn paper_flags(max_mb: u32) -> Vec<String> {
    aikar_flags(max_mb)
        .into_iter()
        .map(|flag| match flag.as_str() {
            "-XX:MaxGCPauseMillis=200" => "-XX:MaxGCPauseMillis=130".into(),
            "-XX:G1NewSizePercent=30" => "-XX:G1NewSizePercent=20".into(),
            "-XX:G1MaxNewSizePercent=40" => "-XX:G1MaxNewSizePercent=35".into(),
            "-XX:G1NewSizePercent=40" => "-XX:G1NewSizePercent=30".into(),
            "-XX:G1MaxNewSizePercent=50" => "-XX:G1MaxNewSizePercent=45".into(),
            _ => flag,
        })
        .collect()
}
pub fn valid_jvm_flag(flag: &str) -> bool {
    (2..=128).contains(&flag.len())
        && flag.starts_with('-')
        && flag.bytes().all(|b| b.is_ascii_graphic())
}
/// Reject malformed or dangerous custom flags before they are persisted.
pub fn validate_jvm_flags(flags: &[String]) -> Result<()> {
    if flags.is_empty() {
        return Err(AppError::user(
            "Add at least one JVM flag for the custom preset.",
        ));
    }
    if flags.len() > 64 {
        return Err(AppError::user("Use at most 64 custom JVM flags."));
    }
    for flag in flags {
        if !valid_jvm_flag(flag) {
            return Err(AppError::user(format!(
                "“{flag}” is not a valid JVM flag. Each flag must start with '-' and contain no spaces."
            )));
        }
        let name = flag.split('=').next().unwrap_or(flag);
        let reserved = matches!(
            name,
            "-jar" | "-cp" | "-classpath" | "-m" | "-version" | "--version" | "-help" | "--help"
        ) || [
            "-Xms",
            "-Xmx",
            "-Xss",
            "-Xbootclasspath",
            "-javaagent",
            "-agentlib",
            "-agentpath",
            "-XX:OnError",
            "-XX:OnOutOfMemoryError",
            "-XX:Flags",
            "-XX:VMOptionsFile",
            "-XX:CompileCommand",
            "-Djava.",
            "-Djavax.",
            "-Dsun.",
            "--class-path",
            "--module",
            "--source",
            "-splash",
            "--patch-module",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix));
        if reserved {
            return Err(AppError::user(format!(
                "{name} is managed by EasyLauncher and cannot be set here."
            )));
        }
        if !flag.starts_with("-XX:") {
            return Err(AppError::user("Custom tuning currently supports only -XX: JVM options. Memory and launch options are managed separately."));
        }
    }
    Ok(())
}
fn quote(arg: &str) -> String {
    if arg.contains([' ', '\t']) {
        format!("\"{arg}\"")
    } else {
        arg.into()
    }
}
async fn read_output<R: AsyncRead + Unpin>(
    m: Arc<Manager>,
    r: Arc<Runtime>,
    reader: R,
    stream: &str,
) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        let buf = match reader.fill_buf().await {
            Ok(b) if !b.is_empty() => b,
            _ => break,
        };
        let count = buf
            .iter()
            .position(|c| *c == b'\n')
            .map(|n| n + 1)
            .unwrap_or(buf.len());
        line.extend_from_slice(&buf[..count]);
        reader.consume(count);
        if line.last() == Some(&b'\n') || line.len() >= 16384 {
            let text = String::from_utf8_lossy(&line).trim_end().to_string();
            observe(&m, &r, &text, stream);
            line.clear();
        }
    }
    if !line.is_empty() {
        observe(&m, &r, &String::from_utf8_lossy(&line), stream);
    }
}
fn observe(m: &Manager, r: &Runtime, text: &str, stream: &str) {
    m.line(r, stream, text);
    // Only trust readiness from the server logger, never chat or a frontend click.
    if readiness(text) && r.snapshot().status == Status::Starting {
        // A full startup proves the crash loop is over.
        r.restart_attempts.store(0, Ordering::Relaxed);
        let _ = m.transition(
            r,
            Status::Running,
            Some("Minecraft is ready for connections.".into()),
        );
    }
    if let Some((name, joined)) = player_event(text) {
        if let Err(e) = crate::player_stats::observe(m, &r.snapshot().id, &name, joined) {
            m.logger
                .log("WARN", &format!("Could not save player history: {e}"));
        }
        let mut snap = r.snapshot.lock().unwrap();
        if joined {
            if !snap.players.contains(&name) {
                snap.players.push(name);
            }
        } else {
            snap.players.retain(|p| p != &name);
        }
        drop(snap);
        m.emit_status(r);
    }
}
fn readiness(text: &str) -> bool {
    text.split_once("]: ")
        .map(|(prefix, body)| {
            prefix.contains("/INFO")
                && body.starts_with("Done (")
                && body.contains("For help, type")
        })
        .unwrap_or(false)
}
pub fn valid_player(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn player_event(text: &str) -> Option<(String, bool)> {
    let (prefix, body) = text.split_once("]: ")?;
    if !prefix.ends_with("/INFO") {
        return None;
    }
    for (suffix, joined) in [(" joined the game", true), (" left the game", false)] {
        if let Some(name) = body.strip_suffix(suffix) {
            if valid_player(name) {
                return Some((name.into(), joined));
            }
        }
    }
    None
}
/// Exponential backoff between crash recovery attempts.
fn restart_delay(attempt: u32) -> u64 {
    let index = attempt.saturating_sub(1) as usize;
    RESTART_DELAYS
        .get(index)
        .copied()
        .unwrap_or(*RESTART_DELAYS.last().unwrap())
}
fn restart_message(attempt: u32, seconds: u64) -> String {
    format!(
        "Crash detected. Restarting automatically in {seconds}s (attempt {attempt}/{MAX_AUTO_RESTARTS})…"
    )
}
/// Explain why Java exited, preferring concrete log evidence over the exit code.
fn attribute_exit(code: Option<i32>, recent: &[String]) -> String {
    let logs = recent.join("\n").to_lowercase();
    for (needle, reason) in [
        (
            "outofmemoryerror",
            "Java ran out of memory. Increase the maximum RAM in server settings.",
        ),
        (
            "could not reserve enough space",
            "Java could not reserve the requested heap. Lower the memory limit or use 64-bit Java.",
        ),
        (
            "unsupportedclassversionerror",
            "The server needs a newer Java version. Change the Java runtime in server settings.",
        ),
        (
            "invalid or corrupt jarfile",
            "server.jar is corrupt or incomplete. Reinstall the server files.",
        ),
        (
            "noclassdeffounderror",
            "A required library is missing. Reinstall the server or remove the incompatible mod.",
        ),
        (
            "failed to bind to port",
            "The server port is already in use. Change the port or stop the conflicting program.",
        ),
        (
            "address already in use",
            "The server port is already in use. Change the port or stop the conflicting program.",
        ),
    ] {
        if logs.contains(needle) {
            return reason.into();
        }
    }
    match code {
        None => "The Java process ended without an exit code. It may have been terminated externally."
            .into(),
        Some(0) => {
            "Java reported a clean exit before Minecraft became ready. Check the console output."
                .into()
        }
        Some(1) => "Java exited with code 1. Check the first error in the console.".into(),
        Some(2) => "Java could not load the server. Check for missing or corrupt files.".into(),
        Some(3) => {
            "Java could not allocate enough memory. Lower the maximum RAM or close other applications."
                .into()
        }
        Some(9) | Some(137) => {
            "The process was killed, likely by the operating system due to memory pressure.".into()
        }
        Some(143) => "The process was terminated before Minecraft finished stopping.".into(),
        Some(-1073741819) => {
            "Java crashed with an access violation (0xC0000005). Update Java or remove incompatible mods."
                .into()
        }
        Some(-1073740791) => {
            "Java crashed with a stack buffer overrun (0xC0000409). Update Java or check native libraries."
                .into()
        }
        Some(-1073741515) => {
            "A required DLL is missing (0xC0000135). Reinstall the Java runtime.".into()
        }
        Some(code) => format!("Java exited with code {code}. Check the console for details."),
    }
}
fn spawn_auto_restart(m: Arc<Manager>, r: Arc<Runtime>, id: String, attempt: u32) {
    tauri::async_runtime::spawn(auto_restart(m, r, id, attempt));
}
async fn auto_restart(m: Arc<Manager>, r: Arc<Runtime>, id: String, attempt: u32) {
    let mut remaining = restart_delay(attempt);
    while remaining > 0 {
        if m.shutting_down.load(Ordering::SeqCst) || r.snapshot().status != Status::Restarting {
            return;
        }
        let _ = m.transition(
            &r,
            Status::Restarting,
            Some(restart_message(attempt, remaining)),
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        remaining -= 1;
    }
    let _g = r.gate.lock().await;
    if m.shutting_down.load(Ordering::SeqCst) || r.snapshot().status != Status::Restarting {
        return;
    }
    m.logger.log(
        "INFO",
        &format!("Crash recovery restarting {id} (attempt {attempt}/{MAX_AUTO_RESTARTS})"),
    );
    if let Err(e) = m.start_locked(&id, &r).await {
        let _ = m.transition(
            &r,
            Status::Crashed,
            Some(format!("Automatic restart failed: {}", e.message)),
        );
        m.logger
            .log("WARN", &format!("Crash recovery failed for {id}: {e}"));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_log_events() {
        assert!(readiness(
            "[12:00:00] [Server thread/INFO]: Done (1.00s)! For help, type \"help\""
        ));
        assert!(!readiness(
            "[12:00:00] [Server thread/INFO]: <Alex> Done (1.00s)! For help, type help"
        ));
        for n in ["Steve", "player_1"] {
            assert!(valid_player(n));
        }
        for n in ["Steve\nstop", "@a", "name space", ""] {
            assert!(!valid_player(n));
        }
        assert_eq!(
            player_event("[12:00:00] [Server thread/INFO]: Steve joined the game"),
            Some(("Steve".into(), true))
        );
        assert!(
            player_event("[12:00] [Server thread/INFO]: <Alex> Steve joined the game").is_none()
        );
    }
    #[test]
    fn restart_backoff_and_exit_attribution() {
        assert_eq!(restart_delay(1), 5);
        assert_eq!(restart_delay(2), 10);
        assert_eq!(restart_delay(3), 20);
        assert_eq!(restart_delay(4), 40);
        assert_eq!(restart_delay(5), 60);
        assert_eq!(restart_delay(9), 60);
        assert!(restart_message(2, 10).contains("10s"));
        assert!(restart_message(2, 10).contains("2/5"));
        assert!(attribute_exit(
            Some(7),
            &["java.lang.OutOfMemoryError: Java heap space".into()]
        )
        .contains("out of memory"));
        assert!(attribute_exit(Some(-1073741819), &[]).contains("0xC0000005"));
        assert!(attribute_exit(None, &[]).contains("without an exit code"));
        assert!(attribute_exit(Some(143), &[]).contains("terminated"));
    }
    #[test]
    fn launch_arguments_match_startup() {
        let s = Server {
            schema_version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            minecraft_version: "1.21.4".into(),
            server_type: crate::models::ServerType::Vanilla,
            memory_min_mb: 1024,
            memory_max_mb: 4096,
            jvm_preset: crate::models::JvmPreset::Balanced,
            jvm_flags: Vec::new(),
            port: 25565,
            created_at: "2026-01-01".into(),
            last_started_at: None,
            java_path: None,
            eula_accepted_at: None,
            installed: true,
            auto_restart: false,
            required_java: Some(21),
            artifact: None,
        };
        assert_eq!(
            launch_args(&s),
            [
                "-Xms1024M",
                "-Xmx4096M",
                "-Dfile.encoding=UTF-8",
                "-jar",
                "server.jar",
                "nogui"
            ]
        );
        assert_eq!(
            quote("C:/Program Files/Java/bin/java.exe"),
            "\"C:/Program Files/Java/bin/java.exe\""
        );
        assert_eq!(quote("-jar"), "-jar");
    }
    #[test]
    fn jvm_presets_tune_the_args() {
        use crate::models::JvmPreset;
        let mut s = Server {
            schema_version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            minecraft_version: "1.21.4".into(),
            server_type: crate::models::ServerType::Vanilla,
            memory_min_mb: 2048,
            memory_max_mb: 8192,
            jvm_preset: JvmPreset::Aikar,
            jvm_flags: Vec::new(),
            port: 25565,
            created_at: "2026-01-01".into(),
            last_started_at: None,
            java_path: None,
            eula_accepted_at: None,
            installed: true,
            auto_restart: false,
            required_java: Some(21),
            artifact: None,
        };
        let aikar = launch_args(&s);
        assert!(aikar.contains(&"-XX:+UseG1GC".into()));
        assert!(aikar.contains(&"-XX:G1HeapRegionSize=8M".into()));
        assert_eq!(aikar.last().unwrap(), "nogui");
        assert!(aikar.contains(&"-jar".into()));

        s.memory_max_mb = 16384;
        let large = launch_args(&s);
        assert!(large.contains(&"-XX:G1HeapRegionSize=16M".into()));
        assert!(large.contains(&"-XX:InitiatingHeapOccupancyPercent=20".into()));

        s.jvm_preset = JvmPreset::Paper;
        let paper = launch_args(&s);
        assert!(paper.contains(&"-XX:MaxGCPauseMillis=130".into()));
        assert!(paper.contains(&"-XX:G1MaxNewSizePercent=45".into()));

        s.jvm_preset = JvmPreset::Custom;
        s.jvm_flags = vec!["-XX:+UseZGC".into(), "-XX:ZCollectionInterval=5".into()];
        let custom = launch_args(&s);
        assert!(custom.contains(&"-XX:+UseZGC".into()));
        assert!(custom.contains(&"-Xms2048M".into()));
    }
    #[test]
    fn custom_flags_are_validated() {
        assert!(validate_jvm_flags(&["-XX:+UseG1GC".into()]).is_ok());
        assert!(validate_jvm_flags(&[]).is_err());
        assert!(validate_jvm_flags(&["XX:+UseG1GC".into()]).is_err());
        assert!(validate_jvm_flags(&["-XX:+Use G1GC".into()]).is_err());
        assert!(validate_jvm_flags(&["-Xmx4G".into()]).is_err());
        assert!(validate_jvm_flags(&["-jar".into()]).is_err());
        assert!(validate_jvm_flags(&["-javaagent:evil.jar".into()]).is_err());
        assert!(validate_jvm_flags(&["-XX:OnError=calc".into()]).is_err());
        assert!(validate_jvm_flags(&["-XX:VMOptionsFile=evil".into()]).is_err());
        assert!(validate_jvm_flags(&["--class-path=evil".into()]).is_err());
        assert!(validate_jvm_flags(&["-Djava.system.class.loader=evil".into()]).is_err());
        assert!(!valid_jvm_flag("UseG1GC"));
        assert!(!valid_jvm_flag("-XX:Use G1GC"));
    }
}
