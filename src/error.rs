use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ArturError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ArturError>;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ArturError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Request(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Process(_) => StatusCode::BAD_GATEWAY,
            Self::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Http(_) => StatusCode::BAD_GATEWAY,
            Self::Toml(_) => StatusCode::BAD_REQUEST,
            Self::Json(_) => StatusCode::BAD_REQUEST,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "config_error",
            Self::Request(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::Process(_) => "process_error",
            Self::Io(_) => "io_error",
            Self::Http(_) => "http_error",
            Self::Toml(_) => "toml_error",
            Self::Json(_) => "json_error",
        }
    }
}

impl IntoResponse for ArturError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        let body = ErrorBody {
            error: self.code().to_string(),
            message: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
