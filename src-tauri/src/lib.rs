pub mod audit;
pub mod backup;
pub mod backup_diff;
pub mod backup_store;
mod commands;
pub mod content_update;
pub mod dependencies;
pub mod disk;
pub mod error;
pub mod extensions;
pub mod import;
pub mod java;
pub mod log_assistant;
mod logger;
pub mod migration;
pub mod minecraft;
pub mod models;
pub mod modrinth;
pub mod monitor;
pub mod network;
pub mod perf;
pub mod player_stats;
pub mod rcon;
pub mod remote;
pub mod remote_backup;
pub mod resources;
pub mod retention;
pub mod schedule;
pub mod secrets;
pub mod server;
pub mod storage;
pub mod supervision;
pub mod tunnel;
pub mod update;
pub mod upgrade;
pub mod upnp;
pub mod world_map;
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
            perf::spawn(manager.clone());
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
            commands::upgrade_info,
            commands::upgrade_preview,
            commands::upgrade_apply,
            commands::upgrade_rollback,
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
            commands::create_live_backup,
            commands::list_backups,
            commands::compare_backups,
            commands::backup_store_info,
            commands::backup_retention,
            commands::set_backup_retention,
            commands::preview_backup_cleanup,
            commands::apply_backup_cleanup,
            commands::remote_backup_info,
            commands::save_remote_backup,
            commands::test_remote_backup,
            commands::upload_backup_remote,
            commands::world_preview,
            commands::world_map,
            commands::world_diff,
            commands::world_tracks,
            commands::sample_world_tracks,
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
            commands::server_crash_report,
            commands::disk_report,
            commands::clean_disk,
            commands::log_assistant_preview,
            commands::analyze_log,
            commands::open_guide,
            commands::modrinth_search,
            commands::modrinth_versions,
            commands::modrinth_install,
            commands::modpack_install,
            commands::modpack_info,
            commands::modpack_update_check,
            commands::modpack_update,
            commands::modpack_rollback,
            commands::content_list,
            commands::content_remove,
            commands::content_remove_file,
            commands::content_check_updates,
            commands::content_update,
            commands::content_rollback,
            commands::content_set_pinned,
            commands::list_tasks,
            commands::create_task,
            commands::update_task,
            commands::delete_task,
            commands::run_task_now,
            commands::preview_task_schedule,
            commands::task_calendar,
            commands::task_history,
            commands::monitor_metrics,
            commands::monitor_history,
            commands::monitor_alerts,
            commands::clear_monitor_alerts,
            commands::monitor_rules,
            commands::save_monitor_rules,
            commands::perf_report,
            commands::perf_save_config,
            commands::perf_sample_now,
            commands::perf_jfr_tools,
            commands::perf_jfr_start,
            commands::perf_jfr_stop,
            commands::perf_delete_profile,
            commands::perf_heap_info,
            commands::resources_report,
            commands::resources_plan,
            commands::resources_start_queue,
            commands::resources_queue_state,
            commands::resources_cancel_queue,
            commands::resources_apply_port,
            commands::supervision_info,
            commands::supervision_preview,
            commands::supervision_install,
            commands::supervision_remove,
            commands::update_check,
            commands::update_download,
            commands::update_open,
            commands::migration_export_preview,
            commands::migration_export,
            commands::migration_pick_export,
            commands::migration_preview_archive,
            commands::migration_import_archive,
            commands::migration_pick_archive,
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
            commands::pick_storage_folder,
            commands::change_server_storage,
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
