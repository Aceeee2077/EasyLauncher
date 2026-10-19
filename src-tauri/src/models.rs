use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerType {
    Vanilla,
    Paper,
    Fabric,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Stopped,
    Installing,
    InstallationFailed,
    Starting,
    Running,
    Stopping,
    Crashed,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
    pub fn allows(self, to: Self) -> bool {
        use Status::*;
        matches!(
            (self, to),
            (Stopped | InstallationFailed, Installing)
                | (Installing, Stopped | InstallationFailed)
                | (Stopped | Crashed, Starting)
                | (Starting, Running | Stopping | Crashed)
                | (Running, Stopping | Crashed)
                | (Stopping, Stopped | Crashed)
        )
    }
}
fn schema() -> u32 {
    1
}
fn min_ram() -> u32 {
    1024
}
fn max_ram() -> u32 {
    2048
}
fn port() -> u16 {
    25565
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    #[serde(default = "schema")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub minecraft_version: String,
    pub server_type: ServerType,
    #[serde(default = "min_ram")]
    pub memory_min_mb: u32,
    #[serde(default = "max_ram")]
    pub memory_max_mb: u32,
    #[serde(default = "port")]
    pub port: u16,
    pub created_at: String,
    #[serde(default)]
    pub last_started_at: Option<String>,
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default)]
    pub eula_accepted_at: Option<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub required_java: Option<u32>,
    #[serde(default)]
    pub artifact: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServer {
    pub name: String,
    pub minecraft_version: String,
    pub server_type: ServerType,
    pub memory_min_mb: u32,
    pub memory_max_mb: u32,
    pub port: u16,
    pub max_players: u32,
    pub game_mode: String,
    pub difficulty: String,
    pub online_mode: bool,
    pub java_path: String,
    pub eula_accepted: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
    pub players: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub server: Server,
    pub runtime: Snapshot,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub id: String,
    pub sequence: u64,
    pub timestamp: String,
    pub stream: String,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default)]
    pub download_source: DownloadSource,
    #[serde(default)]
    pub proxy: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    #[default]
    Official,
    China,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchInfo {
    pub java_path: String,
    pub working_directory: String,
    pub args: Vec<String>,
    pub command: String,
    pub jar_present: bool,
    pub installed: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Language {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}
impl Default for Language {
    fn default() -> Self {
        Self::Chinese
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    System,
    #[default]
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum BackupReason {
    #[default]
    Manual,
    Safety,
    Scheduled,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub created_at: String,
    pub size_bytes: u64,
    pub worlds: Vec<String>,
    #[serde(default)]
    pub safety: bool,
    #[serde(default)]
    pub reason: BackupReason,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub minecraft_version: Option<String>,
    #[serde(default)]
    pub server_type: Option<ServerType>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    pub backup_id: String,
    pub safety_backup_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_machine() {
        assert!(Status::Stopped.allows(Status::Starting));
        assert!(Status::Starting.allows(Status::Running));
        assert!(!Status::Running.allows(Status::Starting));
        assert!(!Status::Stopped.allows(Status::Running));
        assert!(Status::Installing.allows(Status::InstallationFailed));
        assert!(Status::Running.allows(Status::Crashed));
    }
    #[test]
    fn metadata_defaults_roundtrip() {
        let old = r#"{"id":"test","name":"Old","minecraftVersion":"1.20.4","serverType":"vanilla","createdAt":"2026-01-01"}"#;
        let s: Server = serde_json::from_str(old).unwrap();
        assert_eq!(s.memory_max_mb, 2048);
        assert!(!s.installed);
        let round: Server = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(round.schema_version, 1);
        assert!(serde_json::from_str::<Server>("{}").is_err());
    }
    #[test]
    fn language_defaults_to_chinese() {
        let s: Settings = serde_json::from_str(r#"{"appearance":"dark"}"#).unwrap();
        assert_eq!(s.language, Language::Chinese);
        assert_eq!(serde_json::to_string(&Language::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Language::Chinese).unwrap(), "\"zh-CN\"");
        assert_eq!(serde_json::to_string(&Language::English).unwrap(), "\"en\"");
    }
}
