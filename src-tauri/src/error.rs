use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub message: String,
    pub details: String,
}
pub type Result<T> = std::result::Result<T, AppError>;
impl AppError {
    pub fn new(message: impl Into<String>, details: impl ToString) -> Self {
        Self {
            message: message.into(),
            details: details.to_string(),
        }
    }
    pub fn user(message: impl Into<String>) -> Self {
        Self::new(message, "")
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.message, self.details)
    }
}
impl std::error::Error for AppError {}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new(
            "Could not access the managed files. Check permissions and available disk space.",
            e,
        )
    }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(
            "A configuration file could not be read. See details for the affected data.",
            e,
        )
    }
}
impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        Self::new(
            "The download service could not be reached. Check your connection and try again.",
            e,
        )
    }
}
