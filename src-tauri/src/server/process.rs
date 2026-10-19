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

impl Manager {
    pub async fn start(self: &Arc<Self>, id: &str) -> Result<()> {
        self.ready()?;
        let r = self.runtime(id)?;
        let _g = r.gate.lock().await;
        self.start_locked(id, &r).await
    }
    async fn start_locked(self: &Arc<Self>, id: &str, r: &Arc<Runtime>) -> Result<()> {
        self.ready()?;
        r.idle()?;
        if r.child.lock().await.is_some() {
            return Err(AppError::user("This server already has a managed process."));
        }
        let mut s = self.metadata(id)?;
        if !s.installed {
            return Err(AppError::user("Install the server files before starting."));
        }
        if s.eula_accepted_at.is_none() {
            return Err(AppError::user("Accept the Minecraft EULA before starting."));
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
            ));
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
            ));
        }
        for e in std::fs::read_dir(&dir)?.flatten() {
            if e.file_name().to_string_lossy().starts_with("restore-") {
                return Err(AppError::user("This server has preserved restore recovery files. Open its folder and recover or remove the restore folder before starting."));
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
                        {
                            let mut snap = runtime.snapshot.lock().unwrap();
                            snap.exit_code = exit.code();
                            snap.pid = None;
                            snap.players.clear();
                            let expected = snap.status == Status::Stopping && exit.success();
                            snap.status = if expected {
                                Status::Stopped
                            } else {
                                Status::Crashed
                            };
                            snap.message = Some(if expected {
                                "Server stopped safely.".into()
                            } else {
                                format!("Server exited unexpectedly (exit code {}). Open the console or latest log for details.",exit.code().map(|s|s.to_string()).unwrap_or("unknown".into()))
                            });
                        }
                        // Publish final state only after the handle is reaped and streams drained.
                        *runtime.child.lock().await = None;
                        m.emit_status(&runtime);
                        m.logger.log(
                            "INFO",
                            &format!("Java exited for {}: {exit}", runtime.snapshot().id),
                        );
                        if runtime.snapshot().status == Status::Crashed {
                            let manager = m.clone();
                            let server_id = runtime.snapshot().id;
                            match tokio::task::spawn_blocking(move || {
                                super::diagnostics::diagnose(&manager, &server_id)
                            })
                            .await
                            {
                                Ok(Ok(report)) => m.emit("server-diagnosis", report),
                                Ok(Err(e)) => {
                                    m.logger.log("WARN", &format!("Diagnosis failed: {e}"))
                                }
                                Err(e) => m
                                    .logger
                                    .log("WARN", &format!("Diagnosis task failed: {e}")),
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
        self.stop_locked(id, &r).await?;
        self.start_locked(id, &r).await
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        self.shutting_down.store(true, Ordering::SeqCst);
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
        Ok(())
    }
}
/// The single source of truth for the server JVM arguments.
pub fn launch_args(s: &Server) -> Vec<String> {
    vec![
        format!("-Xms{}M", s.memory_min_mb),
        format!("-Xmx{}M", s.memory_max_mb),
        "-Dfile.encoding=UTF-8".into(),
        "-jar".into(),
        "server.jar".into(),
        "nogui".into(),
    ]
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
        let _ = m.transition(
            r,
            Status::Running,
            Some("Minecraft is ready for connections.".into()),
        );
    }
    if let Some((name, joined)) = player_event(text) {
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
    let (_, body) = text.split_once("]: ")?;
    for (suffix, joined) in [(" joined the game", true), (" left the game", false)] {
        if let Some(name) = body.strip_suffix(suffix) {
            if valid_player(name) {
                return Some((name.into(), joined));
            }
        }
    }
    None
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
    fn launch_arguments_match_startup() {
        let s = Server {
            schema_version: 1,
            id: "fixture".into(),
            name: "Fixture".into(),
            minecraft_version: "1.21.4".into(),
            server_type: crate::models::ServerType::Vanilla,
            memory_min_mb: 1024,
            memory_max_mb: 4096,
            port: 25565,
            created_at: "2026-01-01".into(),
            last_started_at: None,
            java_path: None,
            eula_accepted_at: None,
            installed: true,
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
}
