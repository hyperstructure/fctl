//! The handler error type: an [`ApiError`] for the client, with internal
//! details logged but never sent.

use fctl_proto::{ApiError, ErrorCode};

#[derive(Debug)]
pub struct AppError(pub ApiError);

pub type AppResult<T> = Result<T, AppError>;

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl From<worker::Error> for AppError {
    fn from(e: worker::Error) -> Self {
        worker::console_error!("internal error: {e}");
        Self(ApiError::internal(
            "Something went wrong on our side. Try again.",
        ))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        worker::console_error!("json error: {e}");
        Self(ApiError::internal(
            "Something went wrong on our side. Try again.",
        ))
    }
}

impl From<wasm_bindgen::JsValue> for AppError {
    fn from(e: wasm_bindgen::JsValue) -> Self {
        worker::console_error!("js error: {e:?}");
        Self(ApiError::internal(
            "Something went wrong on our side. Try again.",
        ))
    }
}

/// Shorthand for an internal error with a logged cause.
pub fn internal(cause: impl std::fmt::Display) -> AppError {
    worker::console_error!("internal error: {cause}");
    AppError(ApiError::internal(
        "Something went wrong on our side. Try again.",
    ))
}

pub fn not_found(what: &str) -> AppError {
    AppError(ApiError::not_found(what))
}

pub fn forbidden(message: &str) -> AppError {
    AppError(ApiError::forbidden(message))
}

pub fn conflict(message: &str) -> AppError {
    AppError(ApiError::new(ErrorCode::Conflict, message))
}

pub fn unavailable(message: &str) -> AppError {
    AppError(ApiError::new(ErrorCode::Unavailable, message))
}

pub fn not_configured(message: &str) -> AppError {
    AppError(ApiError::new(ErrorCode::NotConfigured, message))
}

pub fn validation(field: &str, message: &str) -> AppError {
    AppError(ApiError::validation(field, message))
}
