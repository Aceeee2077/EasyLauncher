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
  HealthCheck,
  PropertyChange,
  PropertyTemplate,
  DiagnosisReport,
  DiskReport,
  DiskCleanupResult,
  ProjectPage,
  ProjectVersion,
  ContentEntry,
  ContentListing,
  ContentUpdateReport,
  ContentUpdateOutcome,
  ContentRollbackOutcome,
  ModpackReport,
  ModpackInfo,
  ModpackUpdatePreview,
  ModpackUpdateOutcome,
  ModpackRollbackOutcome,
  ScheduledTask,
  TaskInput,
  TaskCalendarEntry,
  TaskHistoryEntry,
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
  BackupStoreInfo,
  RetentionPolicy,
  RetentionPreview,
  RetentionOutcome,
  RemoteConfig,
  RemoteInfo,
  RemoteSecretsInput,
  RemoteUploadOutcome,
  WorldPreview,
  WorldMap,
  WorldDiff,
  WorldTrack,
  TunnelConfig,
  TunnelInfo,
  RconInfo,
  AuditEntry,
  SecretStatus,
  UpgradeInfo,
  UpgradePreview,
  UpgradeOutcome,
  UpgradeRollbackOutcome,
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
  start_preflight: [{ id: string }, HealthCheck[]];
  start_server: [{ id: string; force?: boolean }, void];
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
  preview_properties_template: [
    { id: string; template: PropertyTemplate },
    PropertyChange[],
  ];
  apply_properties_template: [
    { id: string; template: PropertyTemplate },
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
  upgrade_info: [{ id: string }, UpgradeInfo];
  upgrade_preview: [{ id: string; target: string }, UpgradePreview];
  upgrade_apply: [
    { id: string; target: string; confirmed: boolean; allowDowngrade: boolean },
    UpgradeOutcome,
  ];
  upgrade_rollback: [{ id: string; confirmed: boolean }, UpgradeRollbackOutcome];
  delete_java: [{ id: string; confirmed: boolean }, void];
  validate_java: [{ path: string; required: number }, JavaRuntime];
  java_requirement: [{ version: string; software: ServerType }, number];
  list_versions: [
    { software: ServerType; snapshots?: boolean },
    Version[],
  ];
  system_info: [undefined, SystemInfo];
  get_settings: [undefined, Settings];
  save_settings: [{ settings: Settings }, void];
  audit_list: [{ limit?: number }, AuditEntry[]];
  audit_clear: [undefined, void];
  secret_status: [undefined, SecretStatus];
  secret_save: [{ name: string; value: string }, void];
  secret_load: [{ name: string }, string | null];
  secret_delete: [{ name: string }, void];
  create_backup: [{ id: string }, Backup];
  create_live_backup: [{ id: string }, Backup];
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
  backup_store_info: [{ id: string }, BackupStoreInfo];
  backup_retention: [{ id: string }, RetentionPolicy];
  set_backup_retention: [
    { id: string; policy: RetentionPolicy },
    RetentionPolicy,
  ];
  preview_backup_cleanup: [{ id: string }, RetentionPreview];
  apply_backup_cleanup: [
    { id: string; confirmed: boolean },
    RetentionOutcome,
  ];
  remote_backup_info: [{ id: string }, RemoteInfo];
  save_remote_backup: [
    { id: string; config: RemoteConfig; credentials: RemoteSecretsInput },
    RemoteInfo,
  ];
  test_remote_backup: [{ id: string }, void];
  upload_backup_remote: [
    { id: string; backupId: string; confirmed: boolean },
    RemoteUploadOutcome,
  ];
  world_preview: [{ id: string; backupId: string }, WorldPreview];
  world_map: [
    {
      id: string;
      backupId: string;
      world: string;
      dimension: string;
      regionX: number;
      regionZ: number;
      scale: number;
    },
    WorldMap,
  ];
  world_diff: [
    { id: string; beforeId: string; afterId: string },
    WorldDiff,
  ];
  world_tracks: [{ id: string }, WorldTrack[]];
  sample_world_tracks: [{ id: string }, WorldTrack[]];
  player_action: [{ id: string; player: string; action: string }, void];
  rcon_info: [{ id: string }, RconInfo];
  rcon_enable: [{ id: string; port?: number }, RconInfo];
  rcon_disable: [{ id: string }, RconInfo];
  rcon_execute: [{ id: string; command: string }, string];
  rcon_players: [{ id: string }, string[]];
  latest_log: [{ id: string }, string];
  list_content: [{ id: string }, string[]];
  open_folder: [{ id?: string; kind: string }, void];
  server_health: [{ id: string }, HealthReport];
  server_diagnosis: [{ id: string }, DiagnosisReport];
  disk_report: [{ id: string }, DiskReport];
  clean_disk: [
    { id: string; targets: string[]; confirmed: boolean },
    DiskCleanupResult,
  ];
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
  modpack_info: [{ id: string }, ModpackInfo | null];
  modpack_update_check: [{ id: string }, ModpackUpdatePreview | null];
  modpack_update: [
    { id: string; confirmed: boolean },
    ModpackUpdateOutcome,
  ];
  modpack_rollback: [
    { id: string; confirmed: boolean },
    ModpackRollbackOutcome,
  ];
  content_list: [{ id: string }, ContentListing];
  content_remove: [{ id: string; projectId: string }, void];
  content_remove_file: [{ id: string; kind: string; name: string }, void];
  content_check_updates: [{ id: string }, ContentUpdateReport];
  content_update: [
    { id: string; projectId: string; versionId: string; confirmed: boolean },
    ContentUpdateOutcome,
  ];
  content_rollback: [
    { id: string; projectId: string; confirmed: boolean },
    ContentRollbackOutcome,
  ];
  content_set_pinned: [
    { id: string; projectId: string; pinned: boolean },
    void,
  ];
  list_tasks: [{ id: string }, ScheduledTask[]];
  create_task: [{ id: string; input: TaskInput }, ScheduledTask];
  update_task: [
    { id: string; taskId: string; input: TaskInput },
    ScheduledTask,
  ];
  delete_task: [{ id: string; taskId: string }, void];
  run_task_now: [{ id: string; taskId: string }, void];
  preview_task_schedule: [{ input: TaskInput; count?: number }, string[]];
  task_calendar: [
    { id: string; days?: number },
    TaskCalendarEntry[],
  ];
  task_history: [
    { id: string; taskId?: string },
    TaskHistoryEntry[],
  ];
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
