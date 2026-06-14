use axum::{
    Json,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
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
    #[error("store error: {0}")]
    Store(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("payment required: {0}")]
    PaymentRequired(String),
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, ArturError>;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x402_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepts: Option<serde_json::Value>,
}

impl ArturError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Request(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Process(_) => StatusCode::BAD_GATEWAY,
            Self::Store(_) => StatusCode::BAD_GATEWAY,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PaymentRequired(_) => StatusCode::PAYMENT_REQUIRED,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Http(_) => StatusCode::BAD_GATEWAY,
            Self::Toml(_) => StatusCode::BAD_REQUEST,
            Self::Json(_) => StatusCode::BAD_REQUEST,
            Self::Sqlite(_) => StatusCode::BAD_GATEWAY,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "config_error",
            Self::Request(_) => "bad_request",
            Self::NotFound(_) => "not_found",
            Self::Process(_) => "process_error",
            Self::Store(_) => "store_error",
            Self::Forbidden(_) => "forbidden",
            Self::PaymentRequired(_) => "payment_required",
            Self::TooManyRequests(_) => "too_many_requests",
            Self::PayloadTooLarge(_) => "payload_too_large",
            Self::Io(_) => "io_error",
            Self::Http(_) => "http_error",
            Self::Toml(_) => "toml_error",
            Self::Json(_) => "json_error",
            Self::Sqlite(_) => "sqlite_error",
        }
    }
}

impl IntoResponse for ArturError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        let is_payment_required = matches!(self, Self::PaymentRequired(_));
        let accepts = if is_payment_required {
            Some(serde_json::json!([{
                "scheme": "x402-native",
                "description": "Submit a valid x-payment header for this request, or top up the referenced space balance and retry.",
                "headers": ["x-payment"]
            }]))
        } else {
            None
        };
        let body = ErrorBody {
            error: self.code().to_string(),
            message: self.to_string(),
            x402_version: is_payment_required.then_some(1),
            accepts: accepts.clone(),
        };
        if is_payment_required {
            let mut headers = HeaderMap::new();
            headers.insert("x402-version", HeaderValue::from_static("1"));
            if let Some(accepts) = accepts
                && let Ok(value) = HeaderValue::from_str(&accepts.to_string())
            {
                headers.insert("payment-required", value);
            }
            (status, headers, Json(body)).into_response()
        } else {
            (status, Json(body)).into_response()
        }
    }
}
