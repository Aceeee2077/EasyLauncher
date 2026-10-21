use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerType {
    Vanilla,
    Paper,
    Purpur,
    Fabric,
}

impl ServerType {
    /// Paper and Purpur share the Bukkit plugin layout.
    pub fn plugins(self) -> bool {
        matches!(self, Self::Paper | Self::Purpur)
    }
    pub fn content_folder(self) -> Option<&'static str> {
        if self.plugins() {
            Some("plugins")
        } else if matches!(self, Self::Fabric) {
            Some("mods")
        } else {
            None
        }
    }
    pub fn content_loader(self) -> Option<&'static str> {
        if self.plugins() {
            Some("paper")
        } else if matches!(self, Self::Fabric) {
            Some("fabric")
        } else {
            None
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Paper => "paper",
            Self::Purpur => "purpur",
            Self::Fabric => "fabric",
        }
    }
    pub fn supports_snapshots(self) -> bool {
        matches!(self, Self::Vanilla | Self::Fabric)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum JvmPreset {
    #[default]
    Balanced,
    Aikar,
    Paper,
    Custom,
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
    Restarting,
    Crashed,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Stopping | Self::Restarting
        )
    }
    pub fn allows(self, to: Self) -> bool {
        use Status::*;
        matches!(
            (self, to),
            (Stopped | InstallationFailed, Installing)
                | (Installing, Stopped | InstallationFailed)
                | (Stopped | Crashed, Starting)
                | (Crashed, Restarting)
                | (Restarting, Starting | Stopped | Crashed)
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
    #[serde(default)]
    pub jvm_preset: JvmPreset,
    #[serde(default)]
    pub jvm_flags: Vec<String>,
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
    pub auto_restart: bool,
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
    pub exit_reason: Option<String>,
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleBatch {
    pub id: String,
    pub lines: Vec<ConsoleLine>,
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
    /// Blocks state-changing commands while enabled. Kept in Settings so the
    /// frontend and backend read one source of truth.
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub assistant_endpoint: Option<String>,
    #[serde(default)]
    pub assistant_model: Option<String>,
    /// Direct HTTPS URL of the signed update manifest, or none to disable.
    #[serde(default)]
    pub update_channel: Option<String>,
    /// Silent startup check; the user still downloads and opens installers.
    #[serde(default = "yes")]
    pub auto_check_updates: bool,
    /// How opaque the liquid-glass surfaces are, from 0.2 (very sheer) to 1.0.
    #[serde(default = "one")]
    pub glass_opacity: f32,
    /// Custom servers root. None keeps the default under the app data folder.
    #[serde(default)]
    pub server_storage: Option<String>,
    /// Re-apply the router port mapping and firewall rule whenever a server starts.
    #[serde(default)]
    pub auto_join: bool,
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum BackupFormat {
    /// A full copy of the world directories, used before chunked backups.
    #[default]
    Directory,
    /// Zstandard-compressed, content-addressed chunks shared by all backups.
    Chunked,
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
    #[serde(default)]
    pub format: BackupFormat,
    /// Sum of the compressed chunk sizes this backup references.
    #[serde(default)]
    pub stored_bytes: u64,
    #[serde(default)]
    pub chunk_count: u32,
    #[serde(default)]
    pub file_count: u32,
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
        assert!(Status::Crashed.allows(Status::Restarting));
        assert!(Status::Restarting.allows(Status::Starting));
        assert!(Status::Restarting.allows(Status::Stopped));
        assert!(Status::Restarting.active());
        assert!(!Status::Running.allows(Status::Restarting));
    }
    #[test]
    fn metadata_defaults_roundtrip() {
        let old = r#"{"id":"test","name":"Old","minecraftVersion":"1.20.4","serverType":"vanilla","createdAt":"2026-01-01"}"#;
        let s: Server = serde_json::from_str(old).unwrap();
        assert_eq!(s.memory_max_mb, 2048);
        assert!(!s.installed);
        assert_eq!(s.jvm_preset, JvmPreset::Balanced);
        assert!(s.jvm_flags.is_empty());
        let round: Server = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(round.schema_version, 1);
        assert!(serde_json::from_str::<Server>("{}").is_err());
    }
    #[test]
    fn language_defaults_to_chinese() {
        let s: Settings = serde_json::from_str(r#"{"appearance":"dark"}"#).unwrap();
        assert_eq!(s.language, Language::Chinese);
        assert_eq!(
            serde_json::to_string(&Language::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&Language::Chinese).unwrap(),
            "\"zh-CN\""
        );
        assert_eq!(serde_json::to_string(&Language::English).unwrap(), "\"en\"");
    }
}
