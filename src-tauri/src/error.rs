use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// Stable identifier the frontend can translate; empty for uncoded errors.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub code: String,
    pub message: String,
    pub details: String,
}
pub type Result<T> = std::result::Result<T, AppError>;
impl AppError {
    pub fn new(message: impl Into<String>, details: impl ToString) -> Self {
        Self {
            code: String::new(),
            message: message.into(),
            details: details.to_string(),
        }
    }
    pub fn user(message: impl Into<String>) -> Self {
        Self::new(message, "")
    }
    pub fn with_code(mut self, code: &str) -> Self {
        self.code = code.to_string();
        self
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_errors_serialize_code_and_plain_errors_omit_it() {
        let plain = serde_json::to_value(AppError::user("plain")).unwrap();
        assert!(plain.get("code").is_none());
        assert_eq!(plain["details"], "");

        let coded =
            serde_json::to_value(AppError::user("coded").with_code("server.notFound")).unwrap();
        assert_eq!(coded["code"], "server.notFound");
        assert_eq!(coded["message"], "coded");
    }
}
