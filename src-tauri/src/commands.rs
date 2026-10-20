use crate::{
    error::{AppError, Result},
    models::*,
    server::manager::{read_limited, Manager},
};
use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Arc},
};
use tauri::State;
type Managed<'a> = State<'a, Arc<Manager>>;
fn audit(m: &Manager, action: &str, id: &str, detail: impl ToString) {
    let name = m.metadata(id).ok().map(|server| server.name);
    crate::audit::record(m, action, Some(id), name.as_deref(), detail);
}
fn server_name(m: &Manager, id: &str) -> Option<String> {
    m.metadata(id).ok().map(|server| server.name)
}
#[tauri::command]
pub async fn tunnel_info(m: Managed<'_>, id: String) -> Result<crate::tunnel::Info> {
    crate::tunnel::info(&m, &id).await
}
#[tauri::command]
pub async fn tunnel_start(
    m: Managed<'_>,
    id: String,
    config: crate::tunnel::Config,
) -> Result<crate::tunnel::Info> {
    m.ensure_writable()?;
    let info = crate::tunnel::start(&m, &id, config).await?;
    audit(&m, "tunnel.start", &id, "");
    Ok(info)
}
#[tauri::command]
pub async fn tunnel_stop(m: Managed<'_>, id: String) -> Result<crate::tunnel::Info> {
    // Read-only mode still allows stopping, never starting, network services.
    let info = crate::tunnel::stop(&m, &id).await?;
    audit(&m, "tunnel.stop", &id, "");
    Ok(info)
}
#[tauri::command]
pub async fn log_assistant_preview(
    m: Managed<'_>,
    id: String,
) -> Result<crate::log_assistant::Preview> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::log_assistant::preview(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not prepare the log excerpt.", e))?
}
#[tauri::command]
pub async fn analyze_log(
    m: Managed<'_>,
    id: String,
    endpoint: String,
    model: String,
    key: String,
    excerpt: String,
    language: String,
) -> Result<String> {
    crate::log_assistant::analyze(&m, &id, &endpoint, &model, &key, &excerpt, &language).await
}
#[tauri::command]
pub async fn save_share_card(m: Managed<'_>, bytes: Vec<u8>) -> Result<String> {
    if bytes.len() > 4 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(AppError::user(
            "The share card must be a PNG smaller than 4 MB.",
        ));
    }
    let root = m.root.clone();
    tokio::task::spawn_blocking(move || {
        let dir = crate::storage::child(&root, "share-cards")?;
        std::fs::create_dir_all(&dir)?;
        let path = crate::storage::child(&dir, &format!("{}.png", uuid::Uuid::new_v4()))?;
        crate::storage::atomic_write(&path, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| AppError::new("Could not save the share card.", e))?
}
#[tauri::command]
pub async fn disk_report(m: Managed<'_>, id: String) -> Result<crate::disk::DiskReport> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::disk::report(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not inspect disk usage.", e))?
}
#[tauri::command]
pub async fn clean_disk(
    m: Managed<'_>,
    id: String,
    targets: Vec<String>,
    confirmed: bool,
) -> Result<crate::disk::DiskCleanupResult> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    let server_id = id.clone();
    let scan = targets.clone();
    let result =
        tokio::task::spawn_blocking(move || crate::disk::clean(&manager, &id, targets, confirmed))
            .await
            .map_err(|e| AppError::new("Could not clean the selected files.", e))??;
    audit(
        &m,
        "disk.clean",
        &server_id,
        format!(
            "{} files, {} bytes; {}",
            result.removed_files,
            result.freed_bytes,
            scan.join(",")
        ),
    );
    Ok(result)
}
#[tauri::command]
pub fn open_share_cards(m: Managed<'_>) -> Result<()> {
    let dir = crate::storage::child(&m.root, "share-cards")?;
    std::fs::create_dir_all(&dir)?;
    open::that(dir).map_err(|e| AppError::new("Could not open the share card folder.", e))
}
#[tauri::command]
pub async fn compare_backups(
    m: Managed<'_>,
    id: String,
    before_id: String,
    after_id: String,
) -> Result<crate::backup_diff::Diff> {
    crate::backup::compare(m.inner().clone(), id, before_id, after_id).await
}
#[tauri::command]
pub async fn check_dependencies(
    m: Managed<'_>,
    id: String,
    remote: bool,
) -> Result<crate::dependencies::Report> {
    crate::dependencies::check(m.inner().clone(), id, remote).await
}
#[tauri::command]
pub async fn player_stats(m: Managed<'_>, id: String) -> Result<crate::player_stats::Store> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::player_stats::report(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read player history.", e))?
}
#[tauri::command]
pub async fn list_servers(m: Managed<'_>) -> Result<Vec<ServerView>> {
    m.list()
}
#[tauri::command]
pub async fn create_server(m: Managed<'_>, input: CreateServer) -> Result<Server> {
    m.ensure_writable()?;
    let created = m.create(input).await?;
    crate::audit::record(
        &m,
        "server.create",
        Some(&created.id),
        Some(&created.name),
        format!(
            "{} {}",
            created.minecraft_version,
            created.server_type.label()
        ),
    );
    Ok(created)
}
#[tauri::command]
pub async fn delete_server(m: Managed<'_>, id: String, confirmed: bool) -> Result<()> {
    m.ensure_writable()?;
    let name = server_name(&m, &id);
    m.delete(&id, confirmed).await?;
    crate::audit::record(&m, "server.delete", Some(&id), name.as_deref(), "");
    Ok(())
}
#[tauri::command]
pub async fn install_server(m: Managed<'_>, id: String) -> Result<()> {
    m.ensure_writable()?;
    m.install(&id).await?;
    audit(&m, "server.install", &id, "");
    Ok(())
}
#[tauri::command]
pub async fn cancel_install(m: Managed<'_>, id: String) -> Result<()> {
    m.ensure_writable()?;
    m.runtime(&id)?.cancelled.store(true, Ordering::Relaxed);
    audit(&m, "server.cancelInstall", &id, "");
    Ok(())
}
#[tauri::command]
pub fn upgrade_info(m: Managed<'_>, id: String) -> Result<crate::upgrade::Info> {
    crate::upgrade::info(m.inner(), &id)
}
#[tauri::command]
pub async fn upgrade_preview(
    m: Managed<'_>,
    id: String,
    target: String,
) -> Result<crate::upgrade::Preview> {
    crate::upgrade::preview(m.inner().clone(), id, target).await
}
#[tauri::command]
pub async fn upgrade_apply(
    m: Managed<'_>,
    id: String,
    target: String,
    confirmed: bool,
    allow_downgrade: bool,
) -> Result<crate::upgrade::Outcome> {
    m.ensure_writable()?;
    let result = crate::upgrade::apply(
        m.inner().clone(),
        id.clone(),
        target,
        confirmed,
        allow_downgrade,
    )
    .await;
    match &result {
        Ok(outcome) => audit(
            &m,
            "server.upgrade",
            &id,
            format!("{} -> {}", outcome.from_version, outcome.to_version),
        ),
        Err(error) => audit(&m, "server.upgrade.failed", &id, &error.code),
    }
    result
}
#[tauri::command]
pub async fn upgrade_rollback(
    m: Managed<'_>,
    id: String,
    confirmed: bool,
) -> Result<crate::upgrade::RollbackOutcome> {
    m.ensure_writable()?;
    let result = crate::upgrade::rollback(m.inner().clone(), id.clone(), confirmed).await;
    match &result {
        Ok(outcome) => audit(&m, "server.upgrade.rollback", &id, &outcome.version),
        Err(error) => audit(&m, "server.upgrade.rollbackFailed", &id, &error.code),
    }
    result
}
/// Critical pre-flight findings that should stop a start attempt before Java launches.
#[tauri::command]
pub async fn start_preflight(
    m: Managed<'_>,
    id: String,
) -> Result<Vec<crate::server::diagnostics::HealthCheck>> {
    let report = crate::server::diagnostics::health(m.inner().as_ref(), &id).await?;
    Ok(report
        .checks
        .into_iter()
        .filter(|check| check.level == "critical")
        .collect())
}
#[tauri::command]
pub async fn start_server(m: Managed<'_>, id: String, force: Option<bool>) -> Result<()> {
    m.ensure_writable()?;
    if !force.unwrap_or(false) {
        let report = crate::server::diagnostics::health(m.inner().as_ref(), &id).await?;
        if report.checks.iter().any(|check| check.level == "critical") {
            return Err(crate::error::AppError::user(
                "The pre-flight check found blocking issues. Review them, then start anyway if you accept the risk.",
            )
            .with_code("server.preflightBlocked"));
        }
    }
    m.start(&id).await?;
    audit(&m, "server.start", &id, "");
    Ok(())
}
#[tauri::command]
pub async fn stop_server(m: Managed<'_>, id: String) -> Result<()> {
    // Read-only mode still allows stopping, so a running server can always be
    // shut down; starting and restarting stay blocked.
    m.stop(&id).await?;
    audit(&m, "server.stop", &id, "");
    Ok(())
}
#[tauri::command]
pub async fn restart_server(m: Managed<'_>, id: String) -> Result<()> {
    m.ensure_writable()?;
    m.restart(&id).await?;
    audit(&m, "server.restart", &id, "");
    Ok(())
}
#[tauri::command]
pub async fn get_server_status(m: Managed<'_>, id: String) -> Result<Snapshot> {
    Ok(m.runtime(&id)?.snapshot())
}
#[tauri::command]
pub async fn send_console_command(m: Managed<'_>, id: String, command: String) -> Result<()> {
    m.ensure_writable()?;
    m.send(&id, &command).await?;
    audit(
        &m,
        "console.command",
        &id,
        command.lines().next().unwrap_or(""),
    );
    Ok(())
}
#[tauri::command]
pub async fn launch_command(m: Managed<'_>, id: String) -> Result<LaunchInfo> {
    m.launch_info(&id)
}
#[tauri::command]
pub async fn console_history(m: Managed<'_>, id: String) -> Result<Vec<ConsoleLine>> {
    Ok(m.runtime(&id)?
        .console
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect())
}
#[tauri::command]
pub async fn clear_console(m: Managed<'_>, id: String) -> Result<()> {
    let runtime = m.runtime(&id)?;
    m.clear_console(&runtime);
    Ok(())
}
#[tauri::command]
pub async fn get_server_properties(m: Managed<'_>, id: String) -> Result<BTreeMap<String, String>> {
    Ok(m.properties(&id)?.values)
}
#[tauri::command]
pub async fn get_raw_properties(m: Managed<'_>, id: String) -> Result<String> {
    Ok(std::fs::read_to_string(crate::storage::child(
        &m.dir(&id)?,
        "server.properties",
    )?)?)
}
#[tauri::command]
pub async fn update_server_properties(
    m: Managed<'_>,
    id: String,
    properties: Option<BTreeMap<String, String>>,
    raw: Option<String>,
) -> Result<()> {
    m.ensure_writable()?;
    m.save_properties(&id, properties, raw).await?;
    audit(&m, "properties.update", &id, "");
    Ok(())
}
#[tauri::command]
pub async fn preview_properties_template(
    m: Managed<'_>,
    id: String,
    template: String,
) -> Result<Vec<crate::minecraft::properties::PropertyChange>> {
    m.preview_properties_template(&id, &template)
}
#[tauri::command]
pub async fn apply_properties_template(m: Managed<'_>, id: String, template: String) -> Result<()> {
    m.ensure_writable()?;
    m.apply_properties_template(&id, &template).await?;
    audit(&m, "properties.template", &id, &template);
    Ok(())
}
#[tauri::command]
pub async fn update_resources(
    m: Managed<'_>,
    id: String,
    min: u32,
    max: u32,
    java_path: String,
) -> Result<()> {
    m.ensure_writable()?;
    m.update_resources(&id, min, max, java_path).await?;
    audit(&m, "server.resources", &id, format!("{min}-{max} MB"));
    Ok(())
}
#[tauri::command]
pub async fn update_auto_restart(m: Managed<'_>, id: String, enabled: bool) -> Result<Server> {
    m.ensure_writable()?;
    let server = m.update_auto_restart(&id, enabled).await?;
    audit(&m, "server.autoRestart", &id, enabled.to_string());
    Ok(server)
}
#[tauri::command]
pub async fn update_jvm(
    m: Managed<'_>,
    id: String,
    preset: JvmPreset,
    flags: Vec<String>,
) -> Result<Server> {
    m.ensure_writable()?;
    let server = m.update_jvm(&id, preset, flags).await?;
    audit(&m, "server.jvm", &id, format!("{preset:?}"));
    Ok(server)
}
#[tauri::command]
pub async fn detect_java(
    m: Managed<'_>,
    custom: Option<String>,
) -> Result<Vec<crate::java::detector::JavaRuntime>> {
    Ok(crate::java::detector::detect(custom.or(m.settings()?.java_path), Some(&m.java)).await)
}
#[tauri::command]
pub async fn list_managed_java(m: Managed<'_>) -> Result<Vec<crate::java::managed::ManagedJava>> {
    Ok(crate::java::managed::list(&m.java))
}
#[tauri::command]
pub async fn install_java(m: Managed<'_>, major: u32) -> Result<crate::java::managed::ManagedJava> {
    m.ready()?;
    m.ensure_writable()?;
    let runtime = crate::java::managed::install(m.inner().as_ref(), major).await?;
    crate::audit::record(&m, "java.install", None, None, format!("Java {major}"));
    Ok(runtime)
}
#[tauri::command]
pub async fn delete_java(m: Managed<'_>, id: String, confirmed: bool) -> Result<()> {
    m.ready()?;
    m.ensure_writable()?;
    crate::java::managed::delete(m.inner().as_ref(), &id, confirmed).await?;
    crate::audit::record(&m, "java.delete", None, None, &id);
    Ok(())
}
#[tauri::command]
pub async fn validate_java(
    path: String,
    required: u32,
) -> Result<crate::java::detector::JavaRuntime> {
    crate::java::detector::validate(&path, required).await
}
#[tauri::command]
pub async fn java_requirement(
    m: Managed<'_>,
    version: String,
    software: ServerType,
) -> Result<u32> {
    let source = m.settings()?.download_source;
    crate::minecraft::versions::requirement(&m.client(), &version, software, source).await
}
#[tauri::command]
pub async fn list_versions(
    m: Managed<'_>,
    software: ServerType,
    snapshots: Option<bool>,
) -> Result<Vec<crate::minecraft::versions::Version>> {
    let source = m.settings()?.download_source;
    crate::server::software::versions(
        &m.client(),
        software,
        source,
        snapshots.unwrap_or(false) && software.supports_snapshots(),
    )
    .await
}
#[tauri::command]
pub async fn system_info(m: Managed<'_>) -> Result<serde_json::Value> {
    let (total, available, recommended) = crate::server::manager::memory_info();
    Ok(
        serde_json::json!({"totalMb":total,"availableMb":available,"recommendedMb":recommended,"storagePath":m.servers,"downloadPath":"Downloads are streamed to each managed server as server.jar.part; no persistent cache.","logsPath":m.root.join("logs"),"lanAddress":crate::network::lan_address()}),
    )
}
#[tauri::command]
pub async fn get_settings(m: Managed<'_>) -> Result<Settings> {
    m.settings()
}
#[tauri::command]
pub async fn save_settings(m: Managed<'_>, settings: Settings) -> Result<()> {
    let before = m.settings()?.read_only;
    m.save_settings(settings.clone()).await?;
    if before != settings.read_only {
        crate::audit::record(
            &m,
            "settings.readOnly",
            None,
            None,
            if settings.read_only {
                "enabled"
            } else {
                "disabled"
            },
        );
    }
    Ok(())
}
#[tauri::command]
pub fn audit_list(m: Managed<'_>, limit: Option<u32>) -> Result<Vec<crate::audit::Entry>> {
    crate::audit::list(&m, limit.unwrap_or(100) as usize)
}
#[tauri::command]
pub fn audit_clear(m: Managed<'_>) -> Result<()> {
    m.ensure_writable()?;
    crate::audit::clear(&m)?;
    crate::audit::record(&m, "audit.clear", None, None, "cleared by user");
    Ok(())
}
#[tauri::command]
pub fn secret_status(m: Managed<'_>) -> Result<crate::secrets::Status> {
    crate::secrets::status(&m)
}
#[tauri::command]
pub fn secret_save(m: Managed<'_>, name: String, value: String) -> Result<()> {
    m.ensure_writable()?;
    crate::secrets::save(&m, &name, &value)?;
    crate::audit::record(&m, "secret.save", None, None, &name);
    Ok(())
}
#[tauri::command]
pub fn secret_load(m: Managed<'_>, name: String) -> Result<Option<String>> {
    crate::secrets::load(&m, &name)
}
#[tauri::command]
pub fn secret_delete(m: Managed<'_>, name: String) -> Result<()> {
    m.ensure_writable()?;
    crate::secrets::delete(&m, &name)?;
    crate::audit::record(&m, "secret.delete", None, None, &name);
    Ok(())
}
#[tauri::command]
pub async fn create_backup(m: Managed<'_>, id: String) -> Result<Backup> {
    m.ensure_writable()?;
    let backup =
        crate::backup::create_backup(m.inner().clone(), id.clone(), BackupReason::Manual).await?;
    audit(&m, "backup.create", &id, &backup.id);
    Ok(backup)
}
/// Back up a running server through RCON without a restart.
#[tauri::command]
pub async fn create_live_backup(m: Managed<'_>, id: String) -> Result<Backup> {
    m.ensure_writable()?;
    let result = crate::backup::create_consistent_backup(
        m.inner().clone(),
        id.clone(),
        BackupReason::Manual,
    )
    .await;
    match &result {
        Ok(backup) => audit(&m, "backup.live", &id, &backup.id),
        Err(error) => audit(&m, "backup.live.failed", &id, &error.code),
    }
    result
}
#[tauri::command]
pub async fn list_backups(m: Managed<'_>, id: String) -> Result<Vec<Backup>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::backup::list(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read backups.", e))?
}
#[tauri::command]
pub async fn restore_backup(
    m: Managed<'_>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<RestoreOutcome> {
    m.ensure_writable()?;
    let outcome =
        crate::backup::restore_backup(m.inner().clone(), id.clone(), backup_id.clone(), confirmed)
            .await?;
    audit(
        &m,
        "backup.restore",
        &id,
        format!(
            "{backup_id} (safety: {})",
            outcome.safety_backup_id.as_deref().unwrap_or("none")
        ),
    );
    Ok(outcome)
}
#[tauri::command]
pub async fn set_backup_label(
    m: Managed<'_>,
    id: String,
    backup_id: String,
    label: Option<String>,
) -> Result<Backup> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    let server_id = id.clone();
    let backup = tokio::task::spawn_blocking(move || {
        crate::backup::set_label(&manager, &id, &backup_id, label)
    })
    .await
    .map_err(|e| AppError::new("Could not update the backup label.", e))??;
    audit(&m, "backup.label", &server_id, &backup.id);
    Ok(backup)
}
#[tauri::command]
pub async fn delete_backup(
    m: Managed<'_>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<()> {
    m.ensure_writable()?;
    crate::backup::delete_backup(m.inner().clone(), id.clone(), backup_id.clone(), confirmed)
        .await?;
    audit(&m, "backup.delete", &id, &backup_id);
    Ok(())
}
#[tauri::command]
pub async fn player_action(
    m: Managed<'_>,
    id: String,
    player: String,
    action: String,
) -> Result<()> {
    m.ensure_writable()?;
    if !crate::server::process::valid_player(&player) {
        return Err(AppError::user(
            "Player names must be 1–16 letters, numbers or underscores.",
        ));
    }
    let prefix = match action.as_str() {
        "op" => "op",
        "deop" => "deop",
        "kick" => "kick",
        "ban" => "ban",
        "pardon" => "pardon",
        "whitelistAdd" => "whitelist add",
        "whitelistRemove" => "whitelist remove",
        _ => return Err(AppError::user("Unknown player action.")),
    };
    m.send(&id, &format!("{prefix} {player}")).await?;
    audit(&m, "player.action", &id, format!("{action} {player}"));
    Ok(())
}
#[tauri::command]
pub async fn rcon_info(m: Managed<'_>, id: String) -> Result<crate::rcon::RconInfo> {
    crate::rcon::info(m.inner().as_ref(), &id)
}
#[tauri::command]
pub async fn rcon_enable(
    m: Managed<'_>,
    id: String,
    port: Option<u16>,
) -> Result<crate::rcon::RconInfo> {
    m.ensure_writable()?;
    let info = crate::rcon::enable(m.inner(), &id, port).await?;
    audit(&m, "rcon.enable", &id, info.port.to_string());
    Ok(info)
}
#[tauri::command]
pub async fn rcon_disable(m: Managed<'_>, id: String) -> Result<crate::rcon::RconInfo> {
    m.ensure_writable()?;
    let info = crate::rcon::disable(m.inner(), &id).await?;
    audit(&m, "rcon.disable", &id, "");
    Ok(info)
}
#[tauri::command]
pub async fn rcon_execute(m: Managed<'_>, id: String, command: String) -> Result<String> {
    m.ensure_writable()?;
    let output = crate::rcon::execute(m.inner(), &id, &command).await?;
    audit(&m, "rcon.execute", &id, &command);
    Ok(output)
}
#[tauri::command]
pub async fn rcon_players(m: Managed<'_>, id: String) -> Result<Vec<String>> {
    crate::rcon::players(m.inner(), &id).await
}
#[tauri::command]
pub async fn latest_log(m: Managed<'_>, id: String) -> Result<String> {
    read_limited(&crate::storage::child(
        &crate::storage::child(&m.dir(&id)?, "logs")?,
        "latest.log",
    )?)
}
#[tauri::command]
pub async fn list_content(m: Managed<'_>, id: String) -> Result<Vec<String>> {
    let s = m.metadata(&id)?;
    let Some(name) = s.server_type.content_folder() else {
        return Ok(Vec::new());
    };
    let dir = crate::storage::child(&m.dir(&id)?, name)?;
    let mut jars = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        crate::storage::no_links(&e.path())?;
        if e.path().extension().and_then(|s| s.to_str()) == Some("jar") {
            jars.push(e.file_name().to_string_lossy().into());
        }
    }
    Ok(jars)
}
#[tauri::command]
pub async fn open_folder(m: Managed<'_>, id: Option<String>, kind: String) -> Result<()> {
    let p = match kind.as_str() {
        "storage" => m.servers.clone(),
        "appLogs" => m.root.join("logs"),
        "java" => m.java.clone(),
        "server" | "mods" | "plugins" | "logs" | "backups" => {
            let d = m.dir(
                id.as_deref()
                    .ok_or_else(|| AppError::user("Select a server first."))?,
            )?;
            if kind == "server" {
                d
            } else {
                crate::storage::child(&d, &kind)?
            }
        }
        _ => return Err(AppError::user("Unknown managed folder.")),
    };
    crate::storage::no_links(&p)?;
    open::that_detached(p).map_err(|e| AppError::new("Could not open the managed folder.", e))?;
    Ok(())
}
#[tauri::command]
pub async fn server_health(
    m: Managed<'_>,
    id: String,
) -> Result<crate::server::diagnostics::HealthReport> {
    crate::server::diagnostics::health(m.inner().as_ref(), &id).await
}
#[tauri::command]
pub async fn server_diagnosis(
    m: Managed<'_>,
    id: String,
) -> Result<crate::server::diagnostics::DiagnosisReport> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::server::diagnostics::diagnose(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not analyze server logs.", e))?
}
#[tauri::command]
pub async fn open_guide(kind: String) -> Result<()> {
    let url = match kind.as_str() {
        "eula" => "https://www.minecraft.net/eula",
        "java" => "https://adoptium.net/temurin/releases/",
        _ => return Err(AppError::user("Unknown guide.")),
    };
    open::that_detached(url)?;
    Ok(())
}
#[tauri::command]
pub async fn modrinth_search(
    m: Managed<'_>,
    query: String,
    project_type: String,
    game_version: Option<String>,
    loader: Option<String>,
    offset: Option<u64>,
) -> Result<crate::modrinth::ProjectPage> {
    crate::modrinth::search(
        &m.client(),
        &query,
        &project_type,
        game_version.as_deref(),
        loader.as_deref(),
        offset.unwrap_or(0),
    )
    .await
}
#[tauri::command]
pub async fn modrinth_versions(
    m: Managed<'_>,
    project_id: String,
    game_version: Option<String>,
    loader: Option<String>,
) -> Result<Vec<crate::modrinth::ProjectVersion>> {
    crate::modrinth::versions(
        &m.client(),
        &project_id,
        game_version.as_deref(),
        loader.as_deref(),
    )
    .await
}
#[tauri::command]
pub async fn modrinth_install(
    m: Managed<'_>,
    id: String,
    project_id: String,
    version_id: String,
    kind: String,
) -> Result<crate::modrinth::ContentEntry> {
    m.ensure_writable()?;
    let server_id = id.clone();
    let entry =
        crate::modrinth::install(m.inner().clone(), id, project_id, version_id, kind).await?;
    audit(
        &m,
        "content.install",
        &server_id,
        format!("{} {}", entry.title, entry.version_number),
    );
    Ok(entry)
}
#[tauri::command]
pub async fn modpack_install(
    m: Managed<'_>,
    id: String,
    project_id: String,
    version_id: String,
) -> Result<crate::modrinth::ModpackReport> {
    m.ensure_writable()?;
    let server_id = id.clone();
    let report =
        crate::modrinth::install_modpack(m.inner().clone(), id, project_id, version_id).await?;
    audit(
        &m,
        "modpack.install",
        &server_id,
        format!("{} {}", report.name, report.version),
    );
    Ok(report)
}
#[tauri::command]
pub fn modpack_info(m: Managed<'_>, id: String) -> Result<Option<crate::modrinth::ModpackInfo>> {
    crate::modrinth::pack_info(m.inner().as_ref(), &id)
}
#[tauri::command]
pub async fn modpack_update_check(
    m: Managed<'_>,
    id: String,
) -> Result<Option<crate::modrinth::ModpackUpdatePreview>> {
    m.ensure_writable()?;
    crate::modrinth::preview_update(m.inner().clone(), id).await
}
#[tauri::command]
pub async fn modpack_update(
    m: Managed<'_>,
    id: String,
    confirmed: bool,
) -> Result<crate::modrinth::ModpackUpdateOutcome> {
    m.ensure_writable()?;
    let server_id = id.clone();
    let outcome = crate::modrinth::update_modpack(m.inner().clone(), id, confirmed).await?;
    audit(
        &m,
        "modpack.update",
        &server_id,
        format!("{} -> {}", outcome.from_version, outcome.to_version),
    );
    Ok(outcome)
}
#[tauri::command]
pub async fn modpack_rollback(
    m: Managed<'_>,
    id: String,
    confirmed: bool,
) -> Result<crate::modrinth::ModpackRollbackOutcome> {
    m.ensure_writable()?;
    let server_id = id.clone();
    let outcome = crate::modrinth::rollback_modpack(m.inner().clone(), id, confirmed).await?;
    audit(
        &m,
        "modpack.rollback",
        &server_id,
        format!("{} {}", outcome.name, outcome.restored_version),
    );
    Ok(outcome)
}
#[tauri::command]
pub async fn content_list(m: Managed<'_>, id: String) -> Result<crate::modrinth::ContentListing> {
    crate::modrinth::listing(m.inner().as_ref(), &id)
}
#[tauri::command]
pub async fn content_remove(m: Managed<'_>, id: String, project_id: String) -> Result<()> {
    m.ensure_writable()?;
    crate::modrinth::remove(m.inner().clone(), id.clone(), project_id.clone()).await?;
    audit(&m, "content.remove", &id, &project_id);
    Ok(())
}
#[tauri::command]
pub async fn content_remove_file(
    m: Managed<'_>,
    id: String,
    kind: String,
    name: String,
) -> Result<()> {
    m.ensure_writable()?;
    crate::modrinth::remove_file(m.inner().clone(), id.clone(), kind, name.clone()).await?;
    audit(&m, "content.removeFile", &id, &name);
    Ok(())
}
#[tauri::command]
pub async fn content_check_updates(
    m: Managed<'_>,
    id: String,
) -> Result<crate::content_update::UpdateReport> {
    crate::content_update::check(m.inner().clone(), id).await
}
#[tauri::command]
pub async fn content_update(
    m: Managed<'_>,
    id: String,
    project_id: String,
    version_id: String,
    confirmed: bool,
) -> Result<crate::content_update::UpdateOutcome> {
    m.ensure_writable()?;
    let result = crate::content_update::apply(
        m.inner().clone(),
        id.clone(),
        project_id,
        version_id,
        confirmed,
    )
    .await;
    match &result {
        Ok(outcome) => audit(
            &m,
            "content.update",
            &id,
            format!(
                "{} {} -> {}",
                outcome.title, outcome.from_version, outcome.to_version
            ),
        ),
        Err(error) => audit(&m, "content.update.failed", &id, &error.code),
    }
    result
}
#[tauri::command]
pub async fn content_rollback(
    m: Managed<'_>,
    id: String,
    project_id: String,
    confirmed: bool,
) -> Result<crate::content_update::RollbackOutcome> {
    m.ensure_writable()?;
    let result =
        crate::content_update::rollback(m.inner().clone(), id.clone(), project_id, confirmed).await;
    match &result {
        Ok(outcome) => audit(
            &m,
            "content.rollback",
            &id,
            format!("{} {}", outcome.title, outcome.version),
        ),
        Err(error) => audit(&m, "content.rollback.failed", &id, &error.code),
    }
    result
}
#[tauri::command]
pub fn content_set_pinned(
    m: Managed<'_>,
    id: String,
    project_id: String,
    pinned: bool,
) -> Result<()> {
    m.ensure_writable()?;
    crate::content_update::set_pinned(m.inner(), &id, &project_id, pinned)
}
#[tauri::command]
pub async fn list_tasks(m: Managed<'_>, id: String) -> Result<Vec<crate::schedule::Task>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::list(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read scheduled tasks.", e))?
}
#[tauri::command]
pub async fn create_task(
    m: Managed<'_>,
    id: String,
    input: crate::schedule::TaskInput,
) -> Result<crate::schedule::Task> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    let server_id = id.clone();
    let task = tokio::task::spawn_blocking(move || crate::schedule::create(&manager, &id, input))
        .await
        .map_err(|e| AppError::new("Could not save the scheduled task.", e))??;
    audit(
        &m,
        "task.create",
        &server_id,
        format!("{:?} {}", task.action, task.id),
    );
    Ok(task)
}
#[tauri::command]
pub async fn update_task(
    m: Managed<'_>,
    id: String,
    task_id: String,
    input: crate::schedule::TaskInput,
) -> Result<crate::schedule::Task> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    let server_id = id.clone();
    let audit_id = task_id.clone();
    let task = tokio::task::spawn_blocking(move || {
        crate::schedule::update(&manager, &id, &task_id, input)
    })
    .await
    .map_err(|e| AppError::new("Could not save the scheduled task.", e))??;
    audit(&m, "task.update", &server_id, &audit_id);
    Ok(task)
}
#[tauri::command]
pub async fn delete_task(m: Managed<'_>, id: String, task_id: String) -> Result<()> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    let server_id = id.clone();
    let audit_id = task_id.clone();
    tokio::task::spawn_blocking(move || crate::schedule::delete(&manager, &id, &task_id))
        .await
        .map_err(|e| AppError::new("Could not delete the scheduled task.", e))??;
    audit(&m, "task.delete", &server_id, &audit_id);
    Ok(())
}
#[tauri::command]
pub async fn run_task_now(m: Managed<'_>, id: String, task_id: String) -> Result<()> {
    m.ensure_writable()?;
    crate::schedule::run_now(m.inner(), &id, &task_id).await?;
    audit(&m, "task.run", &id, &task_id);
    Ok(())
}
#[tauri::command]
pub fn preview_task_schedule(
    input: crate::schedule::TaskInput,
    count: Option<u32>,
) -> Result<Vec<String>> {
    crate::schedule::preview(&input, count)
}
#[tauri::command]
pub async fn task_calendar(
    m: Managed<'_>,
    id: String,
    days: Option<u32>,
) -> Result<Vec<crate::schedule::CalendarEntry>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::calendar(&manager, &id, days))
        .await
        .map_err(|e| AppError::new("Could not build the task calendar.", e))?
}
#[tauri::command]
pub async fn task_history(
    m: Managed<'_>,
    id: String,
    task_id: Option<String>,
) -> Result<Vec<crate::schedule::HistoryEntry>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::history(&manager, &id, task_id.as_deref()))
        .await
        .map_err(|e| AppError::new("Could not read task history.", e))?
}
#[tauri::command]
pub async fn monitor_metrics(
    m: Managed<'_>,
    id: String,
) -> Result<Option<crate::monitor::ServerMetrics>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::latest(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read server metrics.", e))?
}
#[tauri::command]
pub async fn monitor_history(
    m: Managed<'_>,
    id: String,
) -> Result<Vec<crate::monitor::MetricsSample>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::history(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read the metrics history.", e))?
}
#[tauri::command]
pub async fn monitor_alerts(
    m: Managed<'_>,
    id: String,
) -> Result<Vec<crate::monitor::ServerAlert>> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::alerts(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read server alerts.", e))?
}
#[tauri::command]
pub async fn clear_monitor_alerts(m: Managed<'_>, id: String) -> Result<()> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::clear_alerts(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not clear server alerts.", e))?
}
#[tauri::command]
pub async fn monitor_rules(m: Managed<'_>, id: String) -> Result<crate::monitor::AlertRules> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::rules(&manager, &id))
        .await
        .map_err(|e| AppError::new("Could not read alert rules.", e))?
}
#[tauri::command]
pub async fn save_monitor_rules(
    m: Managed<'_>,
    id: String,
    rules: crate::monitor::AlertRules,
) -> Result<crate::monitor::AlertRules> {
    m.ensure_writable()?;
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::monitor::save_rules(&manager, &id, rules))
        .await
        .map_err(|e| AppError::new("Could not save alert rules.", e))?
}
type Remote<'a> = State<'a, Arc<crate::remote::WebConsole>>;
#[tauri::command]
pub fn web_console_info(r: Remote<'_>) -> Option<crate::remote::WebConsoleInfo> {
    r.info()
}
#[tauri::command]
pub async fn start_web_console(
    m: Managed<'_>,
    r: Remote<'_>,
) -> Result<crate::remote::WebConsoleInfo> {
    m.ensure_writable()?;
    r.start().await
}
#[tauri::command]
pub fn stop_web_console(r: Remote<'_>) {
    r.stop();
}
#[tauri::command]
pub async fn network_report(m: Managed<'_>, id: String) -> Result<crate::upnp::NetworkReport> {
    crate::upnp::report(m.inner().as_ref(), &id).await
}
#[tauri::command]
pub async fn network_map(
    m: Managed<'_>,
    id: String,
    external_port: Option<u16>,
) -> Result<crate::upnp::NetworkReport> {
    m.ensure_writable()?;
    let report = crate::upnp::map(m.inner().as_ref(), &id, external_port).await?;
    audit(
        &m,
        "network.map",
        &id,
        report
            .mapping
            .as_ref()
            .map(|mapping| format!("{}->{}", mapping.external_port, mapping.internal_port))
            .unwrap_or_default(),
    );
    Ok(report)
}
#[tauri::command]
pub async fn network_unmap(m: Managed<'_>, id: String) -> Result<crate::upnp::NetworkReport> {
    m.ensure_writable()?;
    let report = crate::upnp::unmap(m.inner().as_ref(), &id).await?;
    audit(&m, "network.unmap", &id, "");
    Ok(report)
}
#[tauri::command]
pub async fn network_check(m: Managed<'_>, id: String) -> Result<crate::network::NetworkCheck> {
    let server = m.metadata(&id)?;
    let port = server.port;
    let (port_free, firewall_rule) = tokio::task::spawn_blocking(move || {
        (
            crate::network::port_free(port),
            crate::network::firewall_rule(port),
        )
    })
    .await
    .map_err(|e| AppError::new("Could not check the network ports.", e))?;
    Ok(crate::network::NetworkCheck {
        port,
        lan_ip: crate::network::lan_address(),
        port_free,
        firewall_rule,
        rule_name: crate::network::rule_name(port),
    })
}
#[tauri::command]
pub fn firewall_allow(m: Managed<'_>, port: u16) -> Result<()> {
    m.ensure_writable()?;
    crate::network::allow_firewall(port)?;
    crate::audit::record(&m, "network.firewall", None, None, port.to_string());
    Ok(())
}
#[tauri::command]
pub async fn pick_server_folder() -> Result<Option<String>> {
    tokio::task::spawn_blocking(crate::import::pick_folder)
        .await
        .map_err(|e| AppError::new("The folder picker stopped unexpectedly.", e))?
}
#[tauri::command]
pub async fn preview_import(path: String) -> Result<crate::import::ImportPreview> {
    tokio::task::spawn_blocking(move || crate::import::preview(&path))
        .await
        .map_err(|e| AppError::new("The folder scan stopped unexpectedly.", e))?
}
#[tauri::command]
pub async fn import_server(
    m: Managed<'_>,
    path: String,
    name: String,
    minecraft_version: String,
    eula_accepted: bool,
) -> Result<Server> {
    m.ensure_writable()?;
    let server =
        crate::import::import_folder(m.inner(), path, name, minecraft_version, eula_accepted)
            .await?;
    crate::audit::record(
        &m,
        "server.import",
        Some(&server.id),
        Some(&server.name),
        "",
    );
    Ok(server)
}
