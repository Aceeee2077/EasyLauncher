//! Opt-in acceptance harness that downloads and runs a real Minecraft server.
//!
//! These tests are ignored by default because they need network access, a
//! Java 21+ runtime and several hundred MB of disk. Run one software target:
//!
//! ```text
//! cargo test --test minecraft_e2e -- --ignored --nocapture
//! E2E_SOFTWARE=paper E2E_MC_VERSION=1.21.4 cargo test --test minecraft_e2e -- --ignored --nocapture
//! ```
//!
//! `E2E_SOFTWARE` accepts `vanilla`, `paper` or `fabric`. The harness creates a
//! throwaway app directory, downloads the real server artifact, accepts the
//! EULA only inside that temporary directory, waits for the authoritative
//! readiness line, runs a console command, stops the server, then exercises
//! backup and restore. Nothing outside the temporary directory is touched.

use easylauncher_lib::{
    backup,
    models::{BackupReason, CreateServer, ServerType, Status},
    server::manager::Manager,
};
use std::{path::Path, sync::Arc, time::Duration};

const READY_TIMEOUT: Duration = Duration::from_secs(300);
const STOP_TIMEOUT: Duration = Duration::from_secs(90);

fn env(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
}

fn console_text(m: &Arc<Manager>, id: &str) -> String {
    m.runtime(id)
        .map(|runtime| {
            runtime
                .console
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

async fn wait_until(label: &str, timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if ready() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            eprintln!("Timed out after {timeout:?} waiting for {label}");
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn zip_magic(path: &Path) -> bool {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && &magic == b"PK\x03\x04"
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "downloads and runs a real Minecraft server; pass --ignored to run"]
async fn real_server_lifecycle() {
    let version = env("E2E_MC_VERSION", "1.21.4");
    let software = match env("E2E_SOFTWARE", "vanilla").as_str() {
        "vanilla" => ServerType::Vanilla,
        "paper" => ServerType::Paper,
        "fabric" => ServerType::Fabric,
        other => panic!("E2E_SOFTWARE must be vanilla, paper or fabric, got {other}"),
    };
    let label = format!("{software:?} {version}");
    eprintln!("== real server lifecycle: {label} ==");

    let temp = tempfile::tempdir().expect("temporary app directory");
    let m = Manager::open(temp.path().join("app")).expect("open manager");

    // The application intentionally requires the exact Java major a release
    // targets, so a newer system runtime is not accepted. Reuse a matching
    // runtime when one exists, otherwise install a managed Temurin build.
    let required = easylauncher_lib::minecraft::versions::requirement(
        &m.client(),
        &version,
        software,
        m.settings().expect("settings").download_source,
    )
    .await
    .expect("resolve required Java version");
    let mut runtimes = easylauncher_lib::java::detector::detect(None, Some(&m.java)).await;
    if !runtimes.iter().any(|runtime| runtime.major == required) {
        eprintln!("No Java {required} found; installing a managed runtime …");
        easylauncher_lib::java::managed::install(&m, required)
            .await
            .expect("install managed Java");
        runtimes = easylauncher_lib::java::detector::detect(None, Some(&m.java)).await;
    }
    let java = runtimes
        .into_iter()
        .find(|runtime| runtime.major == required)
        .unwrap_or_else(|| panic!("Java {required} is required for {label}"));
    eprintln!("Using Java {} at {}", java.major, java.path);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let created = m
        .create(CreateServer {
            name: format!("E2E {label}"),
            minecraft_version: version.clone(),
            server_type: software,
            memory_min_mb: 512,
            memory_max_mb: 1024,
            port,
            max_players: 5,
            game_mode: "creative".into(),
            difficulty: "peaceful".into(),
            online_mode: false,
            java_path: java.path.clone(),
            eula_accepted: true,
        })
        .await
        .expect("create server");
    let id = created.id.clone();
    let dir = m.dir(&id).expect("server directory");
    assert!(!created.installed, "a new server starts uninstalled");
    let eula = std::fs::read_to_string(dir.join("eula.txt")).expect("eula.txt");
    assert!(eula.contains("eula=true"), "EULA was not written: {eula}");

    eprintln!("Installing {label} …");
    m.install(&id).await.expect("install real server");
    let installed = m.metadata(&id).expect("metadata");
    assert!(installed.installed, "install did not persist");
    let jar = dir.join("server.jar");
    assert!(jar.is_file(), "server.jar missing");
    assert!(zip_magic(&jar), "server.jar is not a JAR archive");
    let size = std::fs::metadata(&jar).expect("jar metadata").len();
    assert!(size > 1_000_000, "server.jar is suspiciously small: {size}");

    eprintln!("Starting {label} on 127.0.0.1:{port} …");
    m.start(&id).await.expect("start server");
    let ready = wait_until(&format!("{label} readiness"), READY_TIMEOUT, || {
        m.runtime(&id)
            .map(|runtime| runtime.snapshot().status == Status::Running)
            .unwrap_or(false)
    })
    .await;
    assert!(
        ready,
        "{label} never reported readiness. Console tail:\n{}",
        console_text(&m, &id)
    );
    let snapshot = m.runtime(&id).expect("runtime").snapshot();
    assert!(snapshot.pid.is_some(), "no Java PID after readiness");
    assert_eq!(snapshot.status, Status::Running);

    let accepts_connections = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map(|result| result.is_ok())
    .unwrap_or(false);
    assert!(
        accepts_connections,
        "port {port} did not accept connections"
    );

    m.send(&id, "list").await.expect("send list command");
    let listed = wait_until("the list command output", Duration::from_secs(30), || {
        console_text(&m, &id).contains("There are 0 of a max of 5 players online")
    })
    .await;
    assert!(
        listed,
        "the server did not answer `list`. Console tail:\n{}",
        console_text(&m, &id)
    );

    eprintln!("Stopping {label} …");
    m.stop(&id).await.expect("stop server");
    let stopped = wait_until("a clean stop", STOP_TIMEOUT, || {
        m.runtime(&id)
            .map(|runtime| runtime.snapshot().status == Status::Stopped)
            .unwrap_or(false)
    })
    .await;
    assert!(
        stopped,
        "{label} did not stop cleanly. Console tail:\n{}",
        console_text(&m, &id)
    );
    let final_state = m.runtime(&id).expect("runtime").snapshot();
    assert_eq!(final_state.exit_code, Some(0), "unexpected exit code");
    assert!(
        final_state.exit_reason.is_none(),
        "stop was reported as a crash"
    );

    eprintln!("Creating a backup of {label} …");
    let backup = backup::create_backup(m.clone(), id.clone(), BackupReason::Manual)
        .await
        .expect("create backup");
    assert!(
        backup.worlds.contains(&"world".to_string()),
        "world not backed up"
    );
    assert!(backup.size_bytes > 0, "empty backup");
    assert!(dir.join("backups").join(&backup.id).join("world").is_dir());

    let marker = dir.join("world").join("e2e-restore-marker.txt");
    std::fs::write(&marker, b"changed after backup").expect("write marker");
    let outcome = backup::restore_backup(m.clone(), id.clone(), backup.id.clone(), true)
        .await
        .expect("restore backup");
    assert!(
        !marker.exists(),
        "restore left a file that was created after the backup"
    );
    assert!(
        dir.join("world").join("level.dat").is_file(),
        "restore did not bring the world back"
    );
    let safety_id = outcome
        .safety_backup_id
        .expect("safety backup before restore");
    assert!(
        dir.join("backups")
            .join(&safety_id)
            .join("world")
            .join("e2e-restore-marker.txt")
            .is_file(),
        "the safety backup did not capture the pre-restore world"
    );

    eprintln!("Checking crash recovery for {label} …");
    m.update_auto_restart(&id, true)
        .await
        .expect("enable crash recovery");
    m.start(&id).await.expect("start after restore");
    assert!(
        wait_until("readiness before the crash", READY_TIMEOUT, || {
            m.runtime(&id)
                .map(|runtime| runtime.snapshot().status == Status::Running)
                .unwrap_or(false)
        })
        .await,
        "{label} did not become ready before the crash test. Console tail:\n{}",
        console_text(&m, &id)
    );
    {
        let runtime = m.runtime(&id).expect("runtime");
        let mut child = runtime.child.lock().await;
        child
            .as_mut()
            .expect("live Java process")
            .start_kill()
            .expect("kill Java");
    }
    assert!(
        wait_until("crash detection", Duration::from_secs(60), || {
            m.runtime(&id)
                .map(|runtime| {
                    matches!(
                        runtime.snapshot().status,
                        Status::Crashed | Status::Restarting
                    )
                })
                .unwrap_or(false)
        })
        .await,
        "the forced crash was not detected. Console tail:\n{}",
        console_text(&m, &id)
    );
    assert!(
        wait_until("automatic restart", READY_TIMEOUT, || {
            m.runtime(&id)
                .map(|runtime| runtime.snapshot().status == Status::Running)
                .unwrap_or(false)
        })
        .await,
        "{label} did not recover after the crash. Console tail:\n{}",
        console_text(&m, &id)
    );
    m.stop(&id).await.expect("stop after crash recovery");
    assert!(
        wait_until("a clean stop after recovery", STOP_TIMEOUT, || {
            m.runtime(&id)
                .map(|runtime| runtime.snapshot().status == Status::Stopped)
                .unwrap_or(false)
        })
        .await,
        "{label} did not stop after crash recovery. Console tail:\n{}",
        console_text(&m, &id)
    );

    eprintln!("Deleting {label} …");
    m.delete(&id, true).await.expect("delete server");
    assert!(!dir.exists(), "server directory survived deletion");
    eprintln!("== {label} lifecycle passed ==");
}
