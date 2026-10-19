//! Extension contracts. V0.1 intentionally has no marketplace or automated diagnosis.
use crate::models::ServerType;
pub trait ContentProvider {
    fn supports(&self, software: ServerType) -> bool;
}
pub trait ModManager {
    fn directory_name(&self) -> &str {
        "mods"
    }
}
pub trait PluginManager {
    fn directory_name(&self) -> &str {
        "plugins"
    }
}
pub trait DependencyResolver {
    fn provider_name(&self) -> &str;
}
pub trait CrashAnalyzer {
    fn analyze(&self, exit_code: Option<i32>, recent_lines: &[String]) -> Option<String>;
}
