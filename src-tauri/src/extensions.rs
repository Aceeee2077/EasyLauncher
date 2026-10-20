//! Extension contracts. V0.1 intentionally has no marketplace; crash diagnosis
//! uses the deterministic signature engine in `server::crash`.
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
/// Deterministic crash analysis; `Output` carries the root cause and all
/// contributing signatures so callers can render localized reports.
pub trait CrashAnalyzer {
    type Output;
    fn analyze(&self, exit_code: Option<i32>, recent_lines: &[String]) -> Self::Output;
}
