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
export interface Server {
  id: string;
  name: string;
  minecraftVersion: string;
  serverType: ServerType;
  memoryMinMb: number;
  memoryMaxMb: number;
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
export interface Progress {
  id: string;
  stage: string;
  downloadedBytes: number;
  totalBytes: number | null;
  attempt: number;
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
  id: string;
  name: string;
  versionNumber: string;
  channel: string;
  gameVersions: string[];
  loaders: string[];
  downloads: number;
  datePublished: string;
  files: VersionFile[];
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
export interface ContentListing {
  entries: ContentEntry[];
  unmanaged: string[];
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
  message: string;
  details: string;
}
export type TaskAction =
  | "backup"
  | "restart"
  | "start"
  | "stop"
  | "command"
  | "cleanup";
export type TaskMode = "interval" | "daily";
export interface ScheduledTask {
  id: string;
  action: TaskAction;
  mode: TaskMode;
  everyMinutes: number;
  atTime: string | null;
  command: string | null;
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
  command: string | null;
  keepDays: number;
  enabled: boolean;
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
export interface NetworkReport {
  lanIp: string | null;
  publicIp: string | null;
  router: string | null;
  igdFound: boolean;
  mapping: PortMapping | null;
  address: string | null;
  port: number;
}
export const active = (s: Status) =>
  ["starting", "running", "stopping", "restarting"].includes(s);
export const idle = (s: Status) => !active(s) && s !== "installing";
