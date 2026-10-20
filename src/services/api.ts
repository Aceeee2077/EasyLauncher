import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  ServerView,
  Server,
  CreateServer,
  Snapshot,
  ConsoleLine,
  JavaRuntime,
  ManagedJava,
  Version,
  SystemInfo,
  Settings,
  Backup,
  RestoreOutcome,
  ServerType,
  LaunchInfo,
  HealthReport,
  DiagnosisReport,
  ProjectPage,
  ProjectVersion,
  ContentEntry,
  ContentListing,
  ModpackReport,
  ScheduledTask,
  TaskInput,
  ServerMetrics,
  MetricsSample,
  ServerAlert,
  AlertRules,
  WebConsoleInfo,
  NetworkReport,
  NetworkCheck,
  ImportPreview,
  JvmPreset,
  DependencyReport,
  PlayerStats,
  BackupDiff,
  TunnelConfig,
  TunnelInfo,
} from "../types";
type Contract = {
  tunnel_info: [{ id: string }, TunnelInfo];
  tunnel_start: [{ id: string; config: TunnelConfig }, TunnelInfo];
  tunnel_stop: [{ id: string }, TunnelInfo];
  log_assistant_preview: [
    { id: string },
    { source: string | null; excerpt: string },
  ];
  analyze_log: [
    {
      id: string;
      endpoint: string;
      model: string;
      key: string;
      excerpt: string;
      language: string;
    },
    string,
  ];
  save_share_card: [{ bytes: number[] }, string];
  open_share_cards: [undefined, void];
  check_dependencies: [{ id: string; remote: boolean }, DependencyReport];
  player_stats: [{ id: string }, PlayerStats];
  compare_backups: [
    { id: string; beforeId: string; afterId: string },
    BackupDiff,
  ];
  list_servers: [undefined, ServerView[]];
  create_server: [{ input: CreateServer }, Server];
  delete_server: [{ id: string; confirmed: boolean }, void];
  install_server: [{ id: string }, void];
  cancel_install: [{ id: string }, void];
  start_server: [{ id: string }, void];
  stop_server: [{ id: string }, void];
  restart_server: [{ id: string }, void];
  get_server_status: [{ id: string }, Snapshot];
  launch_command: [{ id: string }, LaunchInfo];
  send_console_command: [{ id: string; command: string }, void];
  console_history: [{ id: string }, ConsoleLine[]];
  clear_console: [{ id: string }, void];
  get_server_properties: [{ id: string }, Record<string, string>];
  get_raw_properties: [{ id: string }, string];
  update_server_properties: [
    { id: string; properties?: Record<string, string>; raw?: string },
    void,
  ];
  update_resources: [
    { id: string; min: number; max: number; javaPath: string },
    void,
  ];
  update_auto_restart: [{ id: string; enabled: boolean }, Server];
  update_jvm: [{ id: string; preset: JvmPreset; flags: string[] }, Server];
  detect_java: [{ custom?: string }, JavaRuntime[]];
  list_managed_java: [undefined, ManagedJava[]];
  install_java: [{ major: number }, ManagedJava];
  delete_java: [{ id: string; confirmed: boolean }, void];
  validate_java: [{ path: string; required: number }, JavaRuntime];
  java_requirement: [{ version: string; software: ServerType }, number];
  list_versions: [{ software: ServerType }, Version[]];
  system_info: [undefined, SystemInfo];
  get_settings: [undefined, Settings];
  save_settings: [{ settings: Settings }, void];
  create_backup: [{ id: string }, Backup];
  list_backups: [{ id: string }, Backup[]];
  restore_backup: [
    { id: string; backupId: string; confirmed: boolean },
    RestoreOutcome,
  ];
  set_backup_label: [
    { id: string; backupId: string; label: string | null },
    Backup,
  ];
  delete_backup: [{ id: string; backupId: string; confirmed: boolean }, void];
  player_action: [{ id: string; player: string; action: string }, void];
  latest_log: [{ id: string }, string];
  list_content: [{ id: string }, string[]];
  open_folder: [{ id?: string; kind: string }, void];
  server_health: [{ id: string }, HealthReport];
  server_diagnosis: [{ id: string }, DiagnosisReport];
  open_guide: [{ kind: string }, void];
  modrinth_search: [
    {
      query: string;
      projectType: string;
      gameVersion?: string;
      loader?: string;
      offset?: number;
    },
    ProjectPage,
  ];
  modrinth_versions: [
    { projectId: string; gameVersion?: string; loader?: string },
    ProjectVersion[],
  ];
  modrinth_install: [
    { id: string; projectId: string; versionId: string; kind: string },
    ContentEntry,
  ];
  modpack_install: [
    { id: string; projectId: string; versionId: string },
    ModpackReport,
  ];
  content_list: [{ id: string }, ContentListing];
  content_remove: [{ id: string; projectId: string }, void];
  content_remove_file: [{ id: string; kind: string; name: string }, void];
  list_tasks: [{ id: string }, ScheduledTask[]];
  create_task: [{ id: string; input: TaskInput }, ScheduledTask];
  update_task: [
    { id: string; taskId: string; input: TaskInput },
    ScheduledTask,
  ];
  delete_task: [{ id: string; taskId: string }, void];
  run_task_now: [{ id: string; taskId: string }, void];
  monitor_metrics: [{ id: string }, ServerMetrics | null];
  monitor_history: [{ id: string }, MetricsSample[]];
  monitor_alerts: [{ id: string }, ServerAlert[]];
  clear_monitor_alerts: [{ id: string }, void];
  monitor_rules: [{ id: string }, AlertRules];
  save_monitor_rules: [{ id: string; rules: AlertRules }, AlertRules];
  web_console_info: [undefined, WebConsoleInfo | null];
  start_web_console: [undefined, WebConsoleInfo];
  stop_web_console: [undefined, void];
  network_report: [{ id: string }, NetworkReport];
  network_map: [{ id: string; externalPort?: number }, NetworkReport];
  network_unmap: [{ id: string }, NetworkReport];
  network_check: [{ id: string }, NetworkCheck];
  firewall_allow: [{ port: number }, void];
  pick_server_folder: [undefined, string | null];
  preview_import: [{ path: string }, ImportPreview];
  import_server: [
    {
      path: string;
      name: string;
      minecraftVersion: string;
      eulaAccepted: boolean;
    },
    Server,
  ];
};
export const desktop = isTauri();
export async function call<K extends keyof Contract>(
  command: K,
  ...args: Contract[K][0] extends undefined ? [] : [Contract[K][0]]
): Promise<Contract[K][1]> {
  if (!desktop)
    throw {
      message: "Open the desktop application to manage servers.",
      details:
        "Run npm run tauri dev. A normal browser has no access to the Rust backend.",
    };
  return invoke<Contract[K][1]>(
    command,
    args[0] as Record<string, unknown> | undefined,
  );
}
export const brand = {
  name: "EasyLauncher",
  version: "0.1.0",
  tagline: "A home for your next world.",
};
