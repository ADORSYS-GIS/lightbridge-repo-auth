use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid webhook signature")]
    BadSignature,

    #[error("unauthorized")]
    Unauthorized,

    #[error("not found")]
    NotFound,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("github api error: {0}")]
    Github(String),

    #[error(transparent)]
    Db(#[from] sqlx::Error),

    #[error(transparent)]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        // Webhook signature / auth failures are logged at debug only — they are
        // expected probing noise on a public webhook endpoint. Everything else
        // (DB, GitHub) is a real fault and fails closed (5xx).
        let (status, msg) = match &self {
            Error::BadSignature | Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Error::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Error::BadRequest(m) => {
                tracing::debug!(error = %m, "bad request");
                (StatusCode::BAD_REQUEST, "bad request")
            }
            Error::Db(e) => {
                tracing::error!(error = %e, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
            Error::Github(e) => {
                tracing::error!(error = %e, "github api error");
                (StatusCode::BAD_GATEWAY, "upstream error")
            }
            other => {
                tracing::error!(error = %other, "unhandled error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}
