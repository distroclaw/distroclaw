//! Errors returned by the service layer. Tauri commands turn them into `{code, message}` for
//! the UI; the LAN server maps them to HTTP status codes.

use crate::license::Feature;
use serde::Serialize;
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum AppError {
    /// No valid session.
    Unauthorized,
    /// Signed in, but not allowed to do this.
    Forbidden,
    /// The licence's edition does not include this feature.
    Upgrade(Feature),
    /// Missing, or owned by someone else (never revealed as such).
    NotFound(&'static str),
    /// Bad input; the message is shown to the user.
    Invalid(String),
    /// A reply is already being written in this conversation.
    Busy,
    /// A path outside the allowed folders, or the operating system refused access.
    AccessDenied(String),
    /// Unexpected failure; the message is shown to the user and logged.
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    /// Stable machine-readable code the UI switches on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::Upgrade(_) => "upgrade",
            Self::NotFound(_) => "notFound",
            Self::Invalid(_) => "invalid",
            Self::Busy => "busy",
            Self::AccessDenied(_) => "accessDenied",
            Self::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Please sign in again."),
            Self::Forbidden => f.write_str("You don't have permission to do that."),
            Self::Upgrade(feature) => match feature.edition() {
                crate::license::Edition::Business => f.write_str("This is part of DistroClaw Business."),
                _ => f.write_str("This is part of DistroClaw Pro."),
            },
            Self::NotFound(what) => write!(f, "{what} not found."),
            Self::Busy => f.write_str("A reply is already being written in this chat."),
            Self::Invalid(m) | Self::AccessDenied(m) | Self::Internal(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        log::error!("database error: {e}");
        Self::Internal(format!("Database error: {e}"))
    }
}

/// What a failed command sends to the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppErrorDto {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature: Option<Feature>,
}

impl From<AppError> for AppErrorDto {
    fn from(e: AppError) -> Self {
        let feature = match e {
            AppError::Upgrade(f) => Some(f),
            _ => None,
        };
        Self { code: e.code(), message: e.to_string(), feature }
    }
}

impl From<rusqlite::Error> for AppErrorDto {
    fn from(e: rusqlite::Error) -> Self {
        AppError::from(e).into()
    }
}

/// Shorthand for `Err(AppError::Invalid(..))`.
pub fn invalid<T>(message: impl Into<String>) -> AppResult<T> {
    Err(AppError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_serialize_with_a_code_and_the_feature_that_needs_an_upgrade() {
        let dto: AppErrorDto = AppError::Upgrade(Feature::FileOps).into();
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["code"], "upgrade");
        assert_eq!(json["feature"], "fileOps");
        assert!(json["message"].as_str().unwrap().contains("Pro"));
        let dto: AppErrorDto = AppError::Busy.into();
        assert_eq!((dto.code, dto.feature), ("busy", None));
        assert!(serde_json::to_value(&dto).unwrap().get("feature").is_none());
    }
}
