//! In-place Minecraft version upgrades for an installed server.
//!
//! The server must be stopped. EasyLauncher creates a safety backup of the
//! world when one exists, downloads the new server JAR to a staging file,
//! verifies it through the install pipeline, then swaps it into place while
//! keeping the previous JAR for a single-step rollback. A failed download or
//! checksum never touches the installed JAR.
use crate::{
    error::{AppError, Result},
    models::{Backup, BackupReason, Server},
    server::{
        installer::{self, ServerInstaller},
        manager::Manager,
    },
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    path::Path,
    sync::{atomic::Ordering as AtomicOrdering, Arc},
};
use tokio::task;

const JAR: &str = "server.jar";
const STAGE: &str = "server.upgrade.jar";
const PREVIOUS: &str = "server.jar.previous";
const DISCARDED: &str = "server.jar.rolledback";
const MARKER: &str = "upgrade.json";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPoint {
    pub version: String,
    pub artifact: Option<String>,
    pub upgraded_at: String,
    pub safety_backup_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub current_version: String,
    pub installed: bool,
    pub rollback: Option<RollbackPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub current_version: String,
    pub target_version: String,
    pub required_java: u32,
    pub java_available: bool,
    /// Runtime the upgrade will pin; `None` when no exact-major runtime exists.
    pub java_path: Option<String>,
    pub downgrade: bool,
    pub artifact: String,
    pub content_jars: usize,
    pub world_exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub from_version: String,
    pub to_version: String,
    pub safety_backup_id: Option<String>,
    pub artifact: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackOutcome {
    pub version: String,
    pub artifact: Option<String>,
    pub safety_backup_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    previous_version: String,
    #[serde(default)]
    previous_artifact: Option<String>,
    #[serde(default)]
    previous_required_java: Option<u32>,
    #[serde(default)]
    previous_java_path: Option<String>,
    #[serde(default)]
    safety_backup_id: Option<String>,
    upgraded_at: String,
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (2..=4).contains(&parts.len())
        && version.len() <= 16
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.bytes().all(|c| c.is_ascii_digit())
        })
}

fn parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Numeric release comparison, so `1.21.10` sorts after `1.21.9`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a, b) = (parts(a), parts(b));
    for index in 0..a.len().max(b.len()) {
        match a
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&b.get(index).copied().unwrap_or(0))
        {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

pub fn is_downgrade(current: &str, target: &str) -> bool {
    compare_versions(target, current) == Ordering::Less
}

fn validate_target(current: &str, target: &str) -> Result<()> {
    if !valid_version(target) {
        return Err(AppError::user("Choose a stable Minecraft release.")
            .with_code("upgrade.invalidVersion"));
    }
    if target == current {
        return Err(AppError::user("This server already uses this version.")
            .with_code("upgrade.sameVersion"));
    }
    Ok(())
}

fn read_marker(m: &Manager, id: &str) -> Result<Option<Marker>> {
    let dir = m.dir(id)?;
    let path = storage::child(&dir, MARKER)?;
    if !path.is_file() {
        return Ok(None);
    }
    storage::no_links(&path)?;
    let marker: Marker = serde_json::from_slice(&fs::read(&path)?)?;
    // A marker without the saved JAR cannot perform a rollback.
    if !valid_version(&marker.previous_version) || !storage::child(&dir, PREVIOUS)?.is_file() {
        return Ok(None);
    }
    Ok(Some(marker))
}

pub fn info(m: &Manager, id: &str) -> Result<Info> {
    let server = m.metadata(id)?;
    Ok(Info {
        current_version: server.minecraft_version,
        installed: server.installed,
        rollback: read_marker(m, id)?.map(|marker| RollbackPoint {
            version: marker.previous_version,
            artifact: marker.previous_artifact,
            upgraded_at: marker.upgraded_at,
            safety_backup_id: marker.safety_backup_id,
        }),
    })
}

async fn select_java(
    m: &Manager,
    server: &Server,
    required: u32,
) -> Result<Option<crate::java::detector::JavaRuntime>> {
    if let Some(path) = &server.java_path {
        if let Ok(runtime) = crate::java::detector::inspect(path).await {
            if runtime.major == required {
                return Ok(Some(runtime));
            }
        }
    }
    Ok(crate::java::detector::detect(None, Some(&m.java))
        .await
        .into_iter()
        .find(|runtime| runtime.major == required))
}

fn world_exists(m: &Manager, id: &str) -> Result<bool> {
    let dir = m.dir(id)?;
    Ok(crate::backup::world_names(m, id)?
        .iter()
        .any(|name| dir.join(name).is_dir()))
}

fn count_content(dir: &Path) -> usize {
    ["mods", "plugins"]
        .iter()
        .map(|name| {
            fs::read_dir(dir.join(name))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|entry| {
                            entry
                                .file_type()
                                .map(|kind| kind.is_file())
                                .unwrap_or(false)
                                && entry
                                    .path()
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .map(|ext| ext.eq_ignore_ascii_case("jar"))
                                    .unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0)
        })
        .sum()
}

pub async fn preview(m: Arc<Manager>, id: String, target: String) -> Result<Preview> {
    let server = m.metadata(&id)?;
    validate_target(&server.minecraft_version, &target)?;
    let source = m.settings()?.download_source;
    let client = m.client();
    let artifact = ServerInstaller::resolve(&client, &target, server.server_type, source).await?;
    let java = select_java(&m, &server, artifact.java).await?;
    let dir = m.dir(&id)?;
    Ok(Preview {
        current_version: server.minecraft_version.clone(),
        target_version: target.clone(),
        required_java: artifact.java,
        java_available: java.is_some(),
        java_path: java.map(|runtime| runtime.path),
        downgrade: is_downgrade(&server.minecraft_version, &target),
        artifact: artifact.label,
        content_jars: count_content(&dir),
        world_exists: world_exists(&m, &id)?,
    })
}

/// Move the installed JAR aside and install the verified staging JAR.
fn swap_jars(dir: &Path) -> Result<()> {
    let jar = storage::child(dir, JAR)?;
    let previous = storage::child(dir, PREVIOUS)?;
    let stage = storage::child(dir, STAGE)?;
    storage::no_links(&jar)?;
    storage::no_links(&previous)?;
    storage::no_links(&stage)?;
    if !jar.is_file() {
        return Err(AppError::user(
            "The installed server JAR is missing. Reinstall the server before upgrading.",
        )
        .with_code("upgrade.noJar"));
    }
    if !stage.is_file() {
        return Err(AppError::user(
            "The downloaded upgrade was not found. Try again.",
        ));
    }
    if previous.exists() {
        fs::remove_file(&previous)?;
    }
    fs::rename(&jar, &previous)?;
    if let Err(error) = fs::rename(&stage, &jar) {
        let _ = fs::rename(&previous, &jar);
        return Err(error.into());
    }
    Ok(())
}

/// Best-effort reversal used when the upgrade cannot be recorded.
fn restore_upgraded_jar(dir: &Path) {
    let jar = storage::child(dir, JAR).ok();
    let previous = storage::child(dir, PREVIOUS).ok();
    if let (Some(jar), Some(previous)) = (jar, previous) {
        if previous.is_file() {
            let _ = fs::remove_file(&jar);
            let _ = fs::rename(&previous, &jar);
        }
    }
}

fn begin_rollback(dir: &Path) -> Result<()> {
    let jar = storage::child(dir, JAR)?;
    let previous = storage::child(dir, PREVIOUS)?;
    let discarded = storage::child(dir, DISCARDED)?;
    storage::no_links(&jar)?;
    storage::no_links(&previous)?;
    if !previous.is_file() {
        return Err(
            AppError::user("No previous server JAR is saved. Nothing to roll back.")
                .with_code("upgrade.noRollback"),
        );
    }
    if discarded.exists() {
        fs::remove_file(&discarded)?;
    }
    fs::rename(&jar, &discarded)?;
    if let Err(error) = fs::rename(&previous, &jar) {
        let _ = fs::rename(&discarded, &jar);
        return Err(error.into());
    }
    Ok(())
}

fn finish_rollback(dir: &Path) {
    if let Ok(discarded) = storage::child(dir, DISCARDED) {
        let _ = fs::remove_file(discarded);
    }
}

fn undo_rollback(dir: &Path) {
    let jar = storage::child(dir, JAR).ok();
    let previous = storage::child(dir, PREVIOUS).ok();
    let discarded = storage::child(dir, DISCARDED).ok();
    if let (Some(jar), Some(previous), Some(discarded)) = (jar, previous, discarded) {
        if jar.is_file() {
            let _ = fs::rename(&jar, &previous);
        }
        if discarded.is_file() {
            let _ = fs::rename(&discarded, &jar);
        }
    }
}

pub async fn apply(
    m: Arc<Manager>,
    id: String,
    target: String,
    confirmed: bool,
    allow_downgrade: bool,
) -> Result<Outcome> {
    if !confirmed {
        return Err(
            AppError::user("Confirm the upgrade before replacing the server JAR.")
                .with_code("upgrade.notConfirmed"),
        );
    }
    m.ready()?;
    let r = m.runtime(&id)?;
    let _gate = r.gate.lock().await;
    r.idle()?;
    let mut server = m.metadata(&id)?;
    if !server.installed {
        return Err(AppError::user("Install this server before upgrading it.")
            .with_code("upgrade.notInstalled"));
    }
    validate_target(&server.minecraft_version, &target)?;
    if is_downgrade(&server.minecraft_version, &target) && !allow_downgrade {
        return Err(AppError::user(
            "This target is older than the current version. Confirm the downgrade risk first.",
        )
        .with_code("upgrade.downgradeUnconfirmed"));
    }
    let source = m.settings()?.download_source;
    let client = m.client();
    let artifact = ServerInstaller::resolve(&client, &target, server.server_type, source).await?;
    let java = select_java(&m, &server, artifact.java)
        .await?
        .ok_or_else(|| {
            AppError::user(format!(
                "Java {} is required but no matching runtime was found. Install it in app settings, then retry.",
                artifact.java
            ))
            .with_code("upgrade.javaMissing")
        })?;
    let dir = m.dir(&id)?;
    let safety = if world_exists(&m, &id)? {
        let manager = m.clone();
        let server_id = id.clone();
        let from = server.minecraft_version.clone();
        Some(
            task::spawn_blocking(move || -> Result<Backup> {
                let backup = crate::backup::create(&manager, &server_id, BackupReason::Safety)?;
                crate::backup::set_label(
                    &manager,
                    &server_id,
                    &backup.id,
                    Some(format!("before upgrade {from}")),
                )
            })
            .await
            .map_err(|error| {
                AppError::new("Could not create the pre-upgrade safety backup.", error)
            })??,
        )
    } else {
        None
    };
    let stage = storage::child(&dir, STAGE)?;
    storage::no_links(&stage)?;
    let _ = fs::remove_file(&stage);
    r.cancelled.store(false, AtomicOrdering::Relaxed);
    m.line(
        &r,
        "system",
        &format!(
            "Upgrading server from {} to {target}…",
            server.minecraft_version
        ),
    );
    if let Err(error) = installer::download(
        &client,
        &artifact,
        &stage,
        &r.cancelled,
        |progress| m.emit("download-progress", progress),
        &id,
    )
    .await
    {
        let _ = fs::remove_file(&stage);
        m.line(&r, "system", &format!("Upgrade failed: {}", error.message));
        return Err(error);
    }
    if let Err(error) = swap_jars(&dir) {
        let _ = fs::remove_file(&stage);
        m.line(&r, "system", &format!("Upgrade failed: {}", error.message));
        return Err(error);
    }
    let marker = Marker {
        previous_version: server.minecraft_version.clone(),
        previous_artifact: server.artifact.clone(),
        previous_required_java: server.required_java,
        previous_java_path: server.java_path.clone(),
        safety_backup_id: safety.as_ref().map(|backup| backup.id.clone()),
        upgraded_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(error) = storage::write_json(&storage::child(&dir, MARKER)?, &marker) {
        restore_upgraded_jar(&dir);
        return Err(error);
    }
    server.minecraft_version = target.clone();
    server.artifact = Some(artifact.label.clone());
    server.required_java = Some(artifact.java);
    server.java_path = Some(java.path.clone());
    server.installed = true;
    if let Err(error) = storage::save_server(&m.servers, &server) {
        let _ = fs::remove_file(storage::child(&dir, MARKER)?);
        restore_upgraded_jar(&dir);
        return Err(error);
    }
    m.logger.log(
        "INFO",
        &format!(
            "Upgraded server {id} from {} to {target}",
            marker.previous_version
        ),
    );
    m.line(
        &r,
        "system",
        &format!(
            "Upgrade complete. Server is now {target}; the previous JAR is kept for rollback."
        ),
    );
    Ok(Outcome {
        from_version: marker.previous_version,
        to_version: target,
        safety_backup_id: safety.map(|backup| backup.id),
        artifact: artifact.label,
    })
}

pub async fn rollback(m: Arc<Manager>, id: String, confirmed: bool) -> Result<RollbackOutcome> {
    if !confirmed {
        return Err(
            AppError::user("Confirm the version rollback first.").with_code("upgrade.notConfirmed")
        );
    }
    m.ready()?;
    let r = m.runtime(&id)?;
    let _gate = r.gate.lock().await;
    r.idle()?;
    let marker = read_marker(&m, &id)?.ok_or_else(|| {
        AppError::user("No previous server JAR is saved. Nothing to roll back.")
            .with_code("upgrade.noRollback")
    })?;
    let mut server = m.metadata(&id)?;
    let dir = m.dir(&id)?;
    task::spawn_blocking({
        let dir = dir.clone();
        move || begin_rollback(&dir)
    })
    .await
    .map_err(|error| AppError::new("The rollback task failed.", error))??;
    let previous = marker.previous_version.clone();
    server.minecraft_version = previous.clone();
    server.artifact = marker.previous_artifact.clone();
    server.required_java = marker.previous_required_java;
    if marker.previous_java_path.is_some() {
        server.java_path = marker.previous_java_path;
    }
    server.installed = true;
    if let Err(error) = storage::save_server(&m.servers, &server) {
        task::spawn_blocking({
            let dir = dir.clone();
            move || undo_rollback(&dir)
        })
        .await
        .ok();
        return Err(error);
    }
    let _ = fs::remove_file(storage::child(&dir, MARKER)?);
    task::spawn_blocking({
        let dir = dir.clone();
        move || finish_rollback(&dir)
    })
    .await
    .ok();
    m.logger
        .log("INFO", &format!("Rolled back server {id} to {previous}"));
    m.line(
        &r,
        "system",
        &format!("Rolled back to server {previous}. Worlds are unchanged."),
    );
    Ok(RollbackOutcome {
        version: previous,
        artifact: marker.previous_artifact,
        safety_backup_id: marker.safety_backup_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, content: &[u8]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(content).unwrap();
    }

    #[test]
    fn version_order_is_numeric() {
        assert!(is_downgrade("1.21.4", "1.20.6"));
        assert!(!is_downgrade("1.21.9", "1.21.10"));
        assert!(!is_downgrade("1.21.4", "1.21.5"));
        assert_eq!(compare_versions("26.1", "26.1.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.21.4", "1.21.4"), Ordering::Equal);
        assert!(valid_version("1.21.4"));
        assert!(valid_version("26.1"));
        assert!(!valid_version("1.21.4.0.1"));
        assert!(!valid_version("1.21.x"));
        assert!(!valid_version(""));
    }
    #[test]
    fn upgrade_and_rollback_keep_the_previous_jar() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        write(&dir.join(JAR), b"old server");
        write(&dir.join(STAGE), b"new server");
        swap_jars(dir).unwrap();
        assert_eq!(fs::read(dir.join(JAR)).unwrap(), b"new server");
        assert_eq!(fs::read(dir.join(PREVIOUS)).unwrap(), b"old server");
        assert!(!dir.join(STAGE).exists());

        begin_rollback(dir).unwrap();
        assert_eq!(fs::read(dir.join(JAR)).unwrap(), b"old server");
        assert!(!dir.join(PREVIOUS).exists());
        finish_rollback(dir);
        assert!(!dir.join(DISCARDED).exists());
    }
    #[test]
    fn marker_roundtrip_and_invalid_versions() {
        let marker = Marker {
            previous_version: "1.21.4".into(),
            previous_artifact: Some("Vanilla 1.21.4".into()),
            previous_required_java: Some(21),
            previous_java_path: None,
            safety_backup_id: Some("backup-id".into()),
            upgraded_at: "now".into(),
        };
        let text = serde_json::to_string(&marker).unwrap();
        let parsed: Marker = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.previous_version, "1.21.4");
        assert_eq!(parsed.safety_backup_id.as_deref(), Some("backup-id"));
        assert!(!valid_version("../1.21"));
    }
}
