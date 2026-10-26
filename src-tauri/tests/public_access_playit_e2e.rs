//! Opt-in end-to-end check for public access through playit.gg.
//!
//! This starts a real Minecraft server and the real playit agent against a
//! throwaway app directory. It needs network access, a matching Java runtime
//! and several hundred MB of disk space:
//!
//! ```text
//! E2E_JAVA="C:\path\to\java.exe" E2E_MC_VERSION=26.3 \
//!   cargo test --test public_access_playit_e2e -- --ignored --nocapture
//! ```
//!
//! The test prints `CLAIM_URL=...` as soon as playit creates the one-time
//! authorization link. Open it in a browser and accept; the harness then waits
//! for the public address to pass a Minecraft status probe. Temporary files,
//! including the agent key, are removed afterwards. Nothing outside the
//! temporary directory is touched.

use easylauncher_lib::{
    models::{CreateServer, ServerType, Status},
    playit, public_access,
    server::manager::Manager,
};
use std::{sync::Arc, time::Duration};

const READY_TIMEOUT: Duration = Duration::from_secs(300);
const CLAIM_TIMEOUT: Duration = Duration::from_secs(60);
const ONLINE_TIMEOUT: Duration = Duration::from_secs(900);

fn env(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.into())
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
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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

async fn resolve_java(m: &Arc<Manager>, version: &str, software: ServerType) -> String {
    let required = easylauncher_lib::minecraft::versions::requirement(
        &m.client(),
        version,
        software,
        m.settings().expect("settings").download_source,
    )
    .await
    .expect("resolve required Java version");

    if let Ok(path) = std::env::var("E2E_JAVA") {
        if !path.trim().is_empty() {
            let runtime = easylauncher_lib::java::detector::validate(&path, required)
                .await
                .expect("E2E_JAVA must match the release's Java major");
            eprintln!("Using E2E_JAVA {} at {}", runtime.major, runtime.path);
            return runtime.path;
        }
    }

    let mut runtimes = easylauncher_lib::java::detector::detect(None, Some(&m.java)).await;
    if !runtimes.iter().any(|runtime| runtime.major == required) {
        eprintln!("No Java {required} found; installing a managed runtime …");
        easylauncher_lib::java::managed::install(m, required)
            .await
            .expect("install managed Java");
        runtimes = easylauncher_lib::java::detector::detect(None, Some(&m.java)).await;
    }
    let java = runtimes
        .into_iter()
        .find(|runtime| runtime.major == required)
        .unwrap_or_else(|| panic!("Java {required} is required"));
    eprintln!("Using Java {} at {}", java.major, java.path);
    java.path
}

/// Remove the tunnel this run created so it does not linger in the account.
/// This is best effort: the public address check has already passed by then.
async fn delete_tunnel(m: &Manager, id: &str) {
    let Some(key) = easylauncher_lib::secrets::load(m, "playit.agent").unwrap_or(None) else {
        return;
    };
    let path = m.root.join("playit.json");
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Ok(config) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    let Some(tunnel) = config["servers"][id]["tunnel_id"].as_str() else {
        return;
    };
    let client = reqwest::Client::new();
    let result = client
        .post("https://api.playit.gg/tunnels/delete")
        .header("Authorization", format!("Agent-Key {key}"))
        .json(&serde_json::json!({"tunnel_id": tunnel}))
        .timeout(Duration::from_secs(15))
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {
            eprintln!("Removed test tunnel {tunnel} from the playit account");
        }
        Ok(response) => eprintln!(
            "Could not remove test tunnel {tunnel}: HTTP {}. Remove it from the playit dashboard if it lingers.",
            response.status()
        ),
        Err(error) => eprintln!("Could not remove test tunnel {tunnel}: {error}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "starts a real Minecraft server and needs browser authorization for playit; pass --ignored to run"]
async fn playit_public_access() {
    let version = env("E2E_MC_VERSION", "26.3");
    let software = ServerType::Vanilla;
    eprintln!("== playit public access: vanilla {version} ==");

    let temp = tempfile::tempdir().expect("temporary app directory");
    let m = Manager::open(temp.path().join("app")).expect("open manager");

    let java_path = resolve_java(&m, &version, software).await;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let created = m
        .create(CreateServer {
            name: format!("Playit E2E {version}"),
            minecraft_version: version.clone(),
            server_type: software,
            memory_min_mb: 512,
            memory_max_mb: 1024,
            port,
            max_players: 5,
            game_mode: "creative".into(),
            difficulty: "peaceful".into(),
            online_mode: false,
            java_path,
            eula_accepted: true,
        })
        .await
        .expect("create server");
    let id = created.id.clone();

    // Select playit and start the workers before installing so the one-time
    // authorization link is available while the server downloads.
    let selection = public_access::configure(&m, &id, "playit", "tcp", None, None, None)
        .await
        .expect("configure public access");
    eprintln!(
        "Public access configured: provider={} phase={}",
        selection.provider, selection.phase
    );
    playit::spawn(m.clone());
    public_access::spawn(m.clone());

    let mut claim = None;
    let claim_deadline = tokio::time::Instant::now() + CLAIM_TIMEOUT;
    while tokio::time::Instant::now() < claim_deadline {
        if let Ok(info) = public_access::info(&m, &id).await {
            if info.claim_url != claim {
                claim = info.claim_url.clone();
                if let Some(url) = &claim {
                    eprintln!("CLAIM_URL={url}");
                }
            }
            if claim.is_some() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if claim.is_none() {
        eprintln!("playit did not create an authorization link");
    }

    eprintln!("Installing vanilla {version} …");
    let mut failure = None;
    if let Err(error) = m.install(&id).await {
        failure = Some(format!("install failed: {}", error.message));
    }

    if failure.is_none() {
        eprintln!("Starting vanilla {version} on 127.0.0.1:{port} …");
        if let Err(error) = m.start(&id).await {
            failure = Some(format!("start failed: {}", error.message));
        } else if !wait_until("server readiness", READY_TIMEOUT, || {
            m.runtime(&id)
                .map(|runtime| runtime.snapshot().status == Status::Running)
                .unwrap_or(false)
        })
        .await
        {
            failure = Some(format!(
                "server never became ready. Console tail:\n{}",
                console_text(&m, &id)
            ));
        }
    }

    if failure.is_none() {
        eprintln!("Waiting for playit to authorize and verify the public address …");
        let started = tokio::time::Instant::now();
        let deadline = started + ONLINE_TIMEOUT;
        let mut last = String::new();
        loop {
            let info = public_access::info(&m, &id).await;
            let line = match &info {
                Ok(info) => format!(
                    "phase={} verified={} address={} message={}",
                    info.phase,
                    info.verified,
                    info.address.as_deref().unwrap_or("-"),
                    info.message.as_deref().unwrap_or("-")
                ),
                Err(error) => format!("info error: {}", error.message),
            };
            if line != last {
                eprintln!("[{:?}] {line}", started.elapsed());
                last = line;
            }
            match info {
                Ok(info) if info.phase == "online" && info.verified => {
                    println!(
                        "PUBLIC_ADDRESS={}",
                        info.address.as_deref().unwrap_or_default()
                    );
                    eprintln!("playit verified the public Minecraft endpoint");
                    break;
                }
                Ok(info) if info.phase == "error" => {
                    failure = Some(format!(
                        "playit reported an error: {}",
                        info.message.unwrap_or_default()
                    ));
                    break;
                }
                Err(error) => {
                    failure = Some(format!("public access info failed: {}", error.message));
                    break;
                }
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                failure = Some(format!(
                    "public endpoint was not verified within {ONLINE_TIMEOUT:?} (last: {last})"
                ));
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    if failure.is_none() {
        eprintln!("Stopping the server to check tunnel lifecycle …");
        let _ = m.stop(&id).await;
        let followed = wait_until(
            "the tunnel to leave online",
            Duration::from_secs(90),
            || {
                m.runtime(&id)
                    .map(|runtime| runtime.snapshot().status == Status::Stopped)
                    .unwrap_or(false)
            },
        )
        .await;
        if !followed {
            failure = Some("server did not stop after the public check".into());
        } else {
            let leave_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while tokio::time::Instant::now() < leave_deadline {
                if public_access::info(&m, &id)
                    .await
                    .map(|info| info.phase != "online")
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    } else {
        let _ = m.stop(&id).await;
    }

    delete_tunnel(&m, &id).await;
    public_access::shutdown(&m).await;
    playit::shutdown(&m).await;
    let _ = m.delete(&id, true).await;

    if let Some(reason) = failure {
        panic!("playit public access test failed: {reason}");
    }
    eprintln!("== playit public access passed ==");
}
