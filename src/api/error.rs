//! A single error type route handlers can return instead of hand-rolling
//! their own `(StatusCode, Json<serde_json::Value>)` error tuple at every
//! call site (see the triage review that flagged this -- api/routes/*.rs had
//! three different ad-hoc shapes for "something went wrong": a `"success":
//! false, "error": ...` JSON body, a `"message"` field instead of `"error"`
//! on a handful of routes, and a few raw `(StatusCode, String)` plain-text
//! bodies with no JSON at all).
//!
//! `AppError`'s `IntoResponse` impl always produces the first (and by far
//! most common) shape: `{"success": false, "error": "<message>"}` at the
//! matching status code -- the de facto wire contract `web/src/core/
//! api_client.ts` and every other route file already assume. New or
//! rewritten handlers should return `Result<T, AppError>` and use `?` with
//! `.map_err(AppError::Internal)` (or construct a variant directly) instead
//! of inlining another one-off JSON error body.
//!
//! Existing handlers are migrated incrementally, not all at once -- see
//! api/routes/keyring.rs for the first fully-migrated file. The other route
//! files' existing `(StatusCode, Json<...>)` error tuples already match this
//! same JSON shape in the vast majority of cases, so converting them is
//! low-risk, but doing all ~150 call sites across every route file in one
//! pass without verifying each endpoint's actual frontend caller would be a
//! much larger, riskier change than this pass covers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "success": false, "error": message }))).into_response()
    }
}
