export type ServerType = "vanilla" | "paper" | "fabric";
export type Status =
  | "stopped"
  | "installing"
  | "installationFailed"
  | "starting"
  | "running"
  | "stopping"
  | "restarting"
  | "crashed";
export interface Snapshot {
  id: string;
  status: Status;
  pid: number | null;
  startedAt: string | null;
  exitCode: number | null;
  exitReason: string | null;
  message: string | null;
  players: string[];
}
export type JvmPreset = "balanced" | "aikar" | "paper" | "custom";
export type PropertyTemplate = "survival" | "creative" | "performance";
export interface PropertyChange {
  key: string;
  before: string | null;
  after: string;
}
export interface Server {
  id: string;
  name: string;
  minecraftVersion: string;
  serverType: ServerType;
  memoryMinMb: number;
  memoryMaxMb: number;
  jvmPreset: JvmPreset;
  jvmFlags: string[];
  port: number;
  createdAt: string;
  lastStartedAt: string | null;
  javaPath: string | null;
  eulaAcceptedAt: string | null;
  installed: boolean;
  autoRestart: boolean;
  requiredJava: number | null;
  artifact: string | null;
}
export interface ServerView extends Server {
  runtime: Snapshot;
}
export interface CreateServer {
  name: string;
  minecraftVersion: string;
  serverType: ServerType;
  memoryMinMb: number;
  memoryMaxMb: number;
  port: number;
  maxPlayers: number;
  gameMode: string;
  difficulty: string;
  onlineMode: boolean;
  javaPath: string;
  eulaAccepted: boolean;
}
export interface JavaRuntime {
  path: string;
  major: number;
  version: string;
  managed?: boolean;
}
export interface ManagedJava {
  id: string;
  major: number;
  vendor: string;
  version: string;
  path: string;
  installedAt: string;
  sizeBytes: number;
}
export interface JavaProgress {
  id: string;
  stage: string;
  downloadedBytes: number;
  totalBytes: number | null;
  attempt: number;
}
export interface Version {
  id: string;
  type: string;
  url: string;
}
export interface ConsoleLine {
  id: string;
  sequence: number;
  timestamp: string;
  stream: string;
  text: string;
}
export interface ConsoleBatch {
  id: string;
  lines: ConsoleLine[];
}
export interface Progress {
  id: string;
  stage: string;
  downloadedBytes: number;
  totalBytes: number | null;
  attempt: number;
}
export type NoticeKind =
  | "serverStarted"
  | "serverCrashed"
  | "backupComplete"
  | "taskFailed";
export interface Notice {
  kind: NoticeKind;
  serverId: string;
  serverName: string;
  detail: string | null;
  at: string;
}
export type BackupReason = "manual" | "safety" | "scheduled";
export interface Backup {
  id: string;
  createdAt: string;
  sizeBytes: number;
  worlds: string[];
  safety: boolean;
  reason: BackupReason;
  label: string | null;
  minecraftVersion: string | null;
  serverType: ServerType | null;
}
export interface RestoreOutcome {
  backupId: string;
  safetyBackupId: string | null;
}
export interface Settings {
  appearance: "system" | "light" | "dark";
  language: "system" | "zh-CN" | "en";
  javaPath: string | null;
  downloadSource: "official" | "china";
  proxy: string | null;
  readOnly: boolean;
  assistantEndpoint: string | null;
  assistantModel: string | null;
}
export interface AuditEntry {
  at: string;
  action: string;
  serverId: string | null;
  serverName: string | null;
  detail: string;
}
export interface SecretStatus {
  available: boolean;
  names: string[];
}
export interface LaunchInfo {
  javaPath: string;
  workingDirectory: string;
  args: string[];
  command: string;
  jarPresent: boolean;
  installed: boolean;
}
export interface SystemInfo {
  totalMb: number;
  availableMb: number;
  recommendedMb: number;
  storagePath: string;
  downloadPath: string;
  logsPath: string;
  lanAddress: string | null;
}
export type HealthLevel = "ok" | "warning" | "critical" | "info";
export interface HealthCheck {
  id: string;
  level: HealthLevel;
  values: Record<string, string>;
}
export interface HealthReport {
  serverId: string;
  score: number;
  checks: HealthCheck[];
  generatedAt: string;
}
export type DiagnosisSeverity = "critical" | "warning";
export interface DiagnosisFinding {
  id: string;
  severity: DiagnosisSeverity;
  evidence: string[];
}
export interface DiagnosisReport {
  serverId: string;
  available: boolean;
  source: string | null;
  generatedAt: string;
  findings: DiagnosisFinding[];
}
export interface DiskCategory {
  id: string;
  bytes: number;
  files: number;
}
export interface DiskCleanup {
  id: string;
  bytes: number;
  files: number;
  sample: string[];
}
export interface DiskReport {
  serverId: string;
  totalBytes: number;
  freeBytes: number | null;
  locked: boolean;
  categories: DiskCategory[];
  cleanups: DiskCleanup[];
  generatedAt: string;
}
export interface DiskCleanupResult {
  freedBytes: number;
  removedFiles: number;
  failures: string[];
}
export interface ProjectHit {
  id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  iconUrl: string | null;
  downloads: number;
  follows: number;
  categories: string[];
  clientSide: string;
  serverSide: string;
  projectType: string;
}
export interface ProjectPage {
  hits: ProjectHit[];
  total: number;
  offset: number;
  limit: number;
}
export interface VersionFile {
  filename: string;
  url: string;
  size: number;
  sha1: string | null;
  sha512: string | null;
  primary: boolean;
}
export interface ProjectVersion {
  dependencies: {
    projectId: string | null;
    versionId: string | null;
    dependencyType: string;
  }[];
  id: string;
  projectId: string;
  name: string;
  versionNumber: string;
  channel: string;
  gameVersions: string[];
  loaders: string[];
  downloads: number;
  datePublished: string;
  files: VersionFile[];
}
export interface DependencyReport {
  modules: { id: string; version: string; file: string; kind: string }[];
  edges: {
    source: string;
    target: string;
    relation: string;
    requirement: string;
  }[];
  issues: { code: string; source: string; target: string; detail: string }[];
  remoteChecked: boolean;
}
export interface PlayerStats {
  trackingSince: string;
  players: Record<
    string,
    {
      name: string;
      visits: number;
      seconds: number;
      firstSeen: string;
      lastSeen: string;
    }
  >;
  sessions: {
    name: string;
    joinedAt: string;
    leftAt: string;
    seconds: number;
  }[];
  days: Record<string, { date: string; joins: number; seconds: number }>;
}
export interface BackupDiff {
  added: number;
  removed: number;
  modified: number;
  unchanged: number;
  beforeBytes: number;
  afterBytes: number;
  truncated: boolean;
  changes: {
    path: string;
    kind: string;
    beforeBytes: number;
    afterBytes: number;
  }[];
}
export interface ContentEntry {
  file: string;
  projectId: string;
  title: string;
  versionId: string;
  versionNumber: string;
  kind: string;
  installedAt: string;
}
export interface ContentRollback {
  projectId: string;
  versionNumber: string;
  updatedAt: string;
}
export interface ContentListing {
  entries: ContentEntry[];
  unmanaged: string[];
  pinned: string[];
  rollbacks: ContentRollback[];
}
export interface DependencyImpact {
  projectId: string;
  title: string | null;
  dependencyType: string;
  status: string;
  installed: boolean;
}
export interface ContentUpdateEntry {
  projectId: string;
  title: string;
  file: string;
  kind: string;
  currentVersionId: string;
  currentVersion: string;
  latestVersionId: string | null;
  latestVersion: string | null;
  status: string;
  channel: string | null;
  changelog: string | null;
  warnings: string[];
  dependencies: DependencyImpact[];
}
export interface ContentUpdateReport {
  entries: ContentUpdateEntry[];
  complete: boolean;
  errors: string[];
}
export interface ContentUpdateOutcome {
  projectId: string;
  title: string;
  fromVersion: string;
  toVersion: string;
  file: string;
}
export interface ContentRollbackOutcome {
  projectId: string;
  title: string;
  version: string;
}
export interface ContentProgress {
  id: string;
  stage: string;
  file: string;
  completed: number;
  total: number;
  downloadedBytes: number;
  totalBytes: number | null;
}
export interface ModpackReport {
  name: string;
  version: string;
  files: number;
  skipped: number;
  overrides: number;
  minecraft: string;
  loader: string;
}
export interface AppError {
  code?: string;
  message: string;
  details: string;
}
export type TaskAction =
  | "backup"
  | "restart"
  | "start"
  | "stop"
  | "command"
  | "cleanup"
  | "broadcast"
  | "maintenance";
export type TaskMode = "interval" | "daily" | "cron";
export type MissedRun = "skip" | "catchUp";
export interface ScheduledTask {
  id: string;
  action: TaskAction;
  mode: TaskMode;
  everyMinutes: number;
  atTime: string | null;
  cron: string | null;
  missedRun: MissedRun;
  dependsOn: string | null;
  command: string | null;
  message: string | null;
  keepDays: number;
  enabled: boolean;
  nextRunAt: string;
  lastRunAt: string | null;
  lastResult: string | null;
}
export interface TaskInput {
  action: TaskAction;
  mode: TaskMode;
  everyMinutes: number;
  atTime: string | null;
  cron: string | null;
  missedRun: MissedRun;
  dependsOn: string | null;
  command: string | null;
  message: string | null;
  keepDays: number;
  enabled: boolean;
}
export interface TaskCalendarEntry {
  taskId: string;
  action: TaskAction;
  at: string;
}
export interface TaskHistoryEntry {
  taskId: string;
  action: TaskAction;
  at: string;
  result: "ok" | "failed" | "missed" | "blocked";
  detail: string;
  trigger: "schedule" | "manual";
}
export type AlertKind = "cpu" | "memory" | "lag";
export interface ServerMetrics {
  id: string;
  sampledAt: string;
  status: Status;
  cpuPercent: number;
  memoryMb: number;
  memoryLimitMb: number;
  memoryPercent: number;
  players: number;
  maxPlayers: number;
  uptimeSecs: number;
  lagEvents: number;
}
export interface MetricsSample {
  at: string;
  cpuPercent: number;
  memoryPercent: number;
  players: number;
}
export interface ServerAlert {
  id: string;
  serverId: string;
  at: string;
  kind: AlertKind;
  value: number;
  threshold: number;
}
export interface AlertRules {
  enabled: boolean;
  cpuPercent: number;
  memoryPercent: number;
  lagEvents: number;
}
export interface WebConsoleInfo {
  running: boolean;
  url: string;
  token: string;
  port: number;
}
export interface PortMapping {
  enabled: boolean;
  externalPort: number;
  internalPort: number;
  internalIp: string;
  publicIp: string | null;
  router: string;
  mappedAt: string | null;
}
export interface NetworkCheck {
  port: number;
  lanIp: string | null;
  portFree: boolean;
  firewallRule: boolean;
  ruleName: string;
}
export interface TunnelConfig {
  executable: string;
  configPath: string;
  address: string;
}
export interface TunnelInfo {
  config: TunnelConfig;
  running: boolean;
  pid: number | null;
  exitCode: number | null;
}
export interface RconInfo {
  enabled: boolean;
  port: number;
  password: string | null;
  reachable: boolean;
}
export interface UpgradeRollbackPoint {
  version: string;
  artifact: string | null;
  upgradedAt: string;
  safetyBackupId: string | null;
}
export interface UpgradeInfo {
  currentVersion: string;
  installed: boolean;
  rollback: UpgradeRollbackPoint | null;
}
export interface UpgradePreview {
  currentVersion: string;
  targetVersion: string;
  requiredJava: number;
  javaAvailable: boolean;
  javaPath: string | null;
  downgrade: boolean;
  artifact: string;
  contentJars: number;
  worldExists: boolean;
}
export interface UpgradeOutcome {
  fromVersion: string;
  toVersion: string;
  safetyBackupId: string | null;
  artifact: string;
}
export interface UpgradeRollbackOutcome {
  version: string;
  artifact: string | null;
  safetyBackupId: string | null;
}
export interface NetworkReport {
  lanIp: string | null;
  publicIp: string | null;
  router: string | null;
  igdFound: boolean;
  mapping: PortMapping | null;
  address: string | null;
  port: number;
}
export interface ImportPreview {
  path: string;
  suggestedName: string;
  minecraftVersion: string;
  versionSource:
    | "versionHistory"
    | "latestLog"
    | "serverJar"
    | "libraries"
    | "versions"
    | "jarName"
    | "unknown";
  serverType: ServerType;
  port: number;
  jar: string;
  jarRenameNeeded: boolean;
  eulaAccepted: boolean;
  sizeBytes: number;
  fileCount: number;
  warnings: string[];
}
export interface ImportProgress {
  copiedBytes: number;
  totalBytes: number;
  copiedFiles: number;
  totalFiles: number;
}
export const active = (s: Status) =>
  ["starting", "running", "stopping", "restarting"].includes(s);
export const idle = (s: Status) => !active(s) && s !== "installing";
