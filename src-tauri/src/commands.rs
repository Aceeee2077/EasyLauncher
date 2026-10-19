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
#[tauri::command]
pub async fn list_servers(m: Managed<'_>) -> Result<Vec<ServerView>> {
    m.list()
}
#[tauri::command]
pub async fn create_server(m: Managed<'_>, input: CreateServer) -> Result<Server> {
    m.create(input).await
}
#[tauri::command]
pub async fn delete_server(m: Managed<'_>, id: String, confirmed: bool) -> Result<()> {
    m.delete(&id, confirmed).await
}
#[tauri::command]
pub async fn install_server(m: Managed<'_>, id: String) -> Result<()> {
    m.install(&id).await
}
#[tauri::command]
pub async fn cancel_install(m: Managed<'_>, id: String) -> Result<()> {
    m.runtime(&id)?.cancelled.store(true, Ordering::Relaxed);
    Ok(())
}
#[tauri::command]
pub async fn start_server(m: Managed<'_>, id: String) -> Result<()> {
    m.start(&id).await
}
#[tauri::command]
pub async fn stop_server(m: Managed<'_>, id: String) -> Result<()> {
    m.stop(&id).await
}
#[tauri::command]
pub async fn restart_server(m: Managed<'_>, id: String) -> Result<()> {
    m.restart(&id).await
}
#[tauri::command]
pub async fn get_server_status(m: Managed<'_>, id: String) -> Result<Snapshot> {
    Ok(m.runtime(&id)?.snapshot())
}
#[tauri::command]
pub async fn send_console_command(m: Managed<'_>, id: String, command: String) -> Result<()> {
    m.send(&id, &command).await
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
    m.runtime(&id)?.console.lock().unwrap().clear();
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
    m.save_properties(&id, properties, raw).await
}
#[tauri::command]
pub async fn update_resources(
    m: Managed<'_>,
    id: String,
    min: u32,
    max: u32,
    java_path: String,
) -> Result<()> {
    m.update_resources(&id, min, max, java_path).await
}
#[tauri::command]
pub async fn update_auto_restart(m: Managed<'_>, id: String, enabled: bool) -> Result<Server> {
    m.update_auto_restart(&id, enabled).await
}
#[tauri::command]
pub async fn detect_java(
    m: Managed<'_>,
    custom: Option<String>,
) -> Result<Vec<crate::java::detector::JavaRuntime>> {
    Ok(crate::java::detector::detect(custom.or(m.settings()?.java_path), Some(&m.java)).await)
}
#[tauri::command]
pub async fn list_managed_java(
    m: Managed<'_>,
) -> Result<Vec<crate::java::managed::ManagedJava>> {
    Ok(crate::java::managed::list(&m.java))
}
#[tauri::command]
pub async fn install_java(
    m: Managed<'_>,
    major: u32,
) -> Result<crate::java::managed::ManagedJava> {
    m.ready()?;
    crate::java::managed::install(m.inner().as_ref(), major).await
}
#[tauri::command]
pub async fn delete_java(m: Managed<'_>, id: String, confirmed: bool) -> Result<()> {
    m.ready()?;
    crate::java::managed::delete(m.inner().as_ref(), &id, confirmed).await
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
) -> Result<Vec<crate::minecraft::versions::Version>> {
    let source = m.settings()?.download_source;
    crate::minecraft::versions::list(&m.client(), software, source).await
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
    m.save_settings(settings).await
}
#[tauri::command]
pub async fn create_backup(m: Managed<'_>, id: String) -> Result<Backup> {
    crate::backup::create_backup(m.inner().clone(), id, BackupReason::Manual).await
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
    crate::backup::restore_backup(m.inner().clone(), id, backup_id, confirmed).await
}
#[tauri::command]
pub async fn set_backup_label(
    m: Managed<'_>,
    id: String,
    backup_id: String,
    label: Option<String>,
) -> Result<Backup> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::backup::set_label(&manager, &id, &backup_id, label))
        .await
        .map_err(|e| AppError::new("Could not update the backup label.", e))?
}
#[tauri::command]
pub async fn delete_backup(
    m: Managed<'_>,
    id: String,
    backup_id: String,
    confirmed: bool,
) -> Result<()> {
    crate::backup::delete_backup(m.inner().clone(), id, backup_id, confirmed).await
}
#[tauri::command]
pub async fn player_action(
    m: Managed<'_>,
    id: String,
    player: String,
    action: String,
) -> Result<()> {
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
    m.send(&id, &format!("{prefix} {player}")).await
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
    let name = match s.server_type {
        ServerType::Paper => "plugins",
        ServerType::Fabric => "mods",
        ServerType::Vanilla => return Ok(Vec::new()),
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
    crate::modrinth::install(m.inner().clone(), id, project_id, version_id, kind).await
}
#[tauri::command]
pub async fn modpack_install(
    m: Managed<'_>,
    id: String,
    project_id: String,
    version_id: String,
) -> Result<crate::modrinth::ModpackReport> {
    crate::modrinth::install_modpack(m.inner().clone(), id, project_id, version_id).await
}
#[tauri::command]
pub async fn content_list(m: Managed<'_>, id: String) -> Result<crate::modrinth::ContentListing> {
    crate::modrinth::listing(m.inner().as_ref(), &id)
}
#[tauri::command]
pub async fn content_remove(
    m: Managed<'_>,
    id: String,
    project_id: String,
) -> Result<()> {
    crate::modrinth::remove(m.inner().clone(), id, project_id).await
}
#[tauri::command]
pub async fn content_remove_file(
    m: Managed<'_>,
    id: String,
    kind: String,
    name: String,
) -> Result<()> {
    crate::modrinth::remove_file(m.inner().clone(), id, kind, name).await
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
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::create(&manager, &id, input))
        .await
        .map_err(|e| AppError::new("Could not save the scheduled task.", e))?
}
#[tauri::command]
pub async fn update_task(
    m: Managed<'_>,
    id: String,
    task_id: String,
    input: crate::schedule::TaskInput,
) -> Result<crate::schedule::Task> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::update(&manager, &id, &task_id, input))
        .await
        .map_err(|e| AppError::new("Could not save the scheduled task.", e))?
}
#[tauri::command]
pub async fn delete_task(m: Managed<'_>, id: String, task_id: String) -> Result<()> {
    let manager = m.inner().clone();
    tokio::task::spawn_blocking(move || crate::schedule::delete(&manager, &id, &task_id))
        .await
        .map_err(|e| AppError::new("Could not delete the scheduled task.", e))?
}
#[tauri::command]
pub async fn run_task_now(m: Managed<'_>, id: String, task_id: String) -> Result<()> {
    crate::schedule::run_now(m.inner(), &id, &task_id).await
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
pub async fn start_web_console(r: Remote<'_>) -> Result<crate::remote::WebConsoleInfo> {
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
    crate::upnp::map(m.inner().as_ref(), &id, external_port).await
}
#[tauri::command]
pub async fn network_unmap(m: Managed<'_>, id: String) -> Result<crate::upnp::NetworkReport> {
    crate::upnp::unmap(m.inner().as_ref(), &id).await
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
pub fn firewall_allow(port: u16) -> Result<()> {
    crate::network::allow_firewall(port)
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
    crate::import::import_folder(m.inner(), path, name, minecraft_version, eula_accepted).await
}
