export type ServerType = "vanilla" | "paper" | "fabric";
export type Status =
  | "stopped"
  | "installing"
  | "installationFailed"
  | "starting"
  | "running"
  | "stopping"
  | "crashed";
export interface Snapshot {
  id: string;
  status: Status;
  pid: number | null;
  startedAt: string | null;
  exitCode: number | null;
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
export interface Backup {
  id: string;
  createdAt: string;
  sizeBytes: number;
  worlds: string[];
  safety: boolean;
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
export interface AppError {
  message: string;
  details: string;
}
export const active = (s: Status) =>
  ["starting", "running", "stopping"].includes(s);
export const idle = (s: Status) => !active(s) && s !== "installing";
