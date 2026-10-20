pub mod audit;
pub mod backup;
pub mod backup_diff;
mod commands;
pub mod dependencies;
pub mod disk;
pub mod error;
pub mod extensions;
pub mod import;
pub mod java;
pub mod log_assistant;
mod logger;
pub mod minecraft;
pub mod models;
pub mod modrinth;
pub mod monitor;
pub mod network;
pub mod player_stats;
pub mod rcon;
pub mod remote;
pub mod schedule;
pub mod secrets;
pub mod server;
pub mod storage;
pub mod tunnel;
pub mod upnp;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager as _;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let closing = Arc::new(AtomicBool::new(false));
    let closing_event = closing.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            let manager = server::manager::Manager::new(root, app.handle().clone())?;
            schedule::spawn(manager.clone());
            monitor::spawn(manager.clone());
            player_stats::spawn(manager.clone());
            app.manage(remote::WebConsole::new(manager.clone()));
            app.manage(manager);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_servers,
            commands::save_share_card,
            commands::open_share_cards,
            commands::create_server,
            commands::delete_server,
            commands::install_server,
            commands::cancel_install,
            commands::start_preflight,
            commands::start_server,
            commands::stop_server,
            commands::restart_server,
            commands::get_server_status,
            commands::launch_command,
            commands::send_console_command,
            commands::console_history,
            commands::clear_console,
            commands::get_server_properties,
            commands::get_raw_properties,
            commands::update_server_properties,
            commands::preview_properties_template,
            commands::apply_properties_template,
            commands::update_resources,
            commands::update_auto_restart,
            commands::update_jvm,
            commands::detect_java,
            commands::list_managed_java,
            commands::install_java,
            commands::delete_java,
            commands::validate_java,
            commands::java_requirement,
            commands::list_versions,
            commands::system_info,
            commands::get_settings,
            commands::save_settings,
            commands::audit_list,
            commands::audit_clear,
            commands::secret_status,
            commands::secret_save,
            commands::secret_load,
            commands::secret_delete,
            commands::create_backup,
            commands::list_backups,
            commands::compare_backups,
            commands::check_dependencies,
            commands::player_stats,
            commands::restore_backup,
            commands::set_backup_label,
            commands::delete_backup,
            commands::player_action,
            commands::rcon_info,
            commands::rcon_enable,
            commands::rcon_disable,
            commands::rcon_execute,
            commands::rcon_players,
            commands::latest_log,
            commands::list_content,
            commands::open_folder,
            commands::server_health,
            commands::server_diagnosis,
            commands::disk_report,
            commands::clean_disk,
            commands::log_assistant_preview,
            commands::analyze_log,
            commands::open_guide,
            commands::modrinth_search,
            commands::modrinth_versions,
            commands::modrinth_install,
            commands::modpack_install,
            commands::content_list,
            commands::content_remove,
            commands::content_remove_file,
            commands::list_tasks,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
            commands::run_task_now,
            commands::monitor_metrics,
            commands::monitor_history,
            commands::monitor_alerts,
            commands::clear_monitor_alerts,
            commands::monitor_rules,
            commands::save_monitor_rules,
            commands::web_console_info,
            commands::start_web_console,
            commands::stop_web_console,
            commands::network_report,
            commands::tunnel_info,
            commands::tunnel_start,
            commands::tunnel_stop,
            commands::network_map,
            commands::network_unmap,
            commands::network_check,
            commands::firewall_allow,
            commands::pick_server_folder,
            commands::preview_import,
            commands::import_server
        ])
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if closing_event.swap(true, Ordering::SeqCst) {
                    return;
                }
                let app = window.app_handle().clone();
                {
                    use tauri::Emitter;
                    let _ = app.emit("app-closing", ());
                }
                let manager = app.state::<Arc<server::manager::Manager>>().inner().clone();
                let flag = closing_event.clone();
                tauri::async_runtime::spawn(async move {
                    match manager.shutdown().await {
                        Ok(()) => app.exit(0),
                        Err(e) => {
                            use tauri::Emitter;
                            manager
                                .logger
                                .log("ERROR", &format!("Shutdown failed: {e}"));
                            let _ = app.emit("app-error", e);
                            manager.shutting_down.store(false, Ordering::SeqCst);
                            flag.store(false, Ordering::SeqCst);
                        }
                    }
                });
            }
        })
        .run(tauri::generate_context!())
        .expect(
            "EasyLauncher could not start. Check application permissions and WebView2 installation.",
        );
}
