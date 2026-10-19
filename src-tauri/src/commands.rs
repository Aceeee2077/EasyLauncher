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
    crate::backup::create_backup(m.inner().clone(), id).await
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
) -> Result<()> {
    crate::backup::restore_backup(m.inner().clone(), id, backup_id, confirmed).await
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
