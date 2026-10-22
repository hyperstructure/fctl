//! The error body every API endpoint returns on failure.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Machine-readable error codes. Clients branch on these, never on messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request is malformed.
    BadRequest,
    /// A field failed validation; see `field`.
    Validation,
    /// No valid credentials were sent.
    Unauthorized,
    /// Signed in, but not allowed to do this.
    Forbidden,
    /// The email is not on the instance's allowlist (or the user is disabled).
    AccessDenied,
    /// Wrong email or password.
    InvalidCredentials,
    NotFound,
    Conflict,
    RateLimited,
    /// The instance has no owner yet; finish setup first.
    SetupRequired,
    /// A dependency (GitHub, a runner, email) is unavailable.
    Unavailable,
    /// The feature is not configured on this instance.
    NotConfigured,
    /// The instance requires two-factor sign-in, and this account hasn't
    /// set it up: nothing else works until it has.
    TwoFactorSetupRequired,
    /// A sensitive action, which the instance asks people to confirm it's
    /// them for, with a passkey or a code from their authenticator app
    /// (`POST /api/v1/auth/verify`), and the browser, CLI or token that
    /// asks hasn't within the last 15 minutes.
    VerificationRequired,
    Internal,
}

impl ErrorCode {
    pub fn http_status(self) -> u16 {
        match self {
            Self::BadRequest | Self::Validation => 400,
            Self::Unauthorized | Self::InvalidCredentials => 401,
            Self::Forbidden
            | Self::AccessDenied
            | Self::TwoFactorSetupRequired
            | Self::VerificationRequired => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::SetupRequired => 409,
            Self::RateLimited => 429,
            Self::NotConfigured => 501,
            Self::Unavailable => 503,
            Self::Internal => 500,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ApiError {
    pub code: ErrorCode,
    /// A sentence for people. Never contains secrets.
    pub message: String,
    /// The request field this error is about, for form validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub field: Option<String>,
    /// Seconds to wait before retrying (rate limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub retry_after: Option<u32>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
            retry_after: None,
        }
    }

    pub fn validation(field: &str, message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Validation,
            message: message.into(),
            field: Some(field.to_owned()),
            retry_after: None,
        }
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} not found"))
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Forbidden, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(ErrorCode::Unauthorized, "Sign in to continue")
    }

    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after = Some(seconds);
        self
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({:?})", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}
