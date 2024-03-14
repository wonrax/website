use std::{collections::HashMap, fmt::Debug};

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

#[derive(Debug)]
pub enum ServerError {
    DatabaseError(sqlx::Error),
    HttpClientError(reqwest::Error),
    Unknown(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DatabaseError(e) => write!(f, "Database error: {}", e),
            Self::HttpClientError(e) => write!(f, "HTTP client error: {}", e),
            Self::Unknown(e) => write!(f, "Unknown error: {}", e),
        }
    }
}

impl Serialize for ServerError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}

pub trait ApiError: std::fmt::Display {
    fn status_code(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    fn error(&self) -> ErrorResponse {
        ErrorResponse::new(&self.to_string())
    }

    fn into_response(&self) -> axum::response::Response {
        let error = self.error();
        let status_code = self.status_code();
        (status_code, Json(error)).into_response()
    }
}

pub enum AppError {
    ServerError {
        error: ServerError,
        msg: Option<String>,
        backtrace: Backtrace,
    },
    ApiError(Box<dyn ApiError>),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::ServerError { error, .. } => write!(f, "Server error: {}", error),
            AppError::ApiError(e) => write!(f, "API error: {}", e),
        }
    }
}

#[derive(Serialize)]
pub struct ErrorResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,

    msg: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<HashMap<&'static str, Value>>,
}

impl ErrorResponse {
    pub fn new(msg: &str) -> Self {
        Self {
            code: None,
            msg: msg.into(),
            debug_info: None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::ApiError(e) => {
                debug!(api_error = %e, "api error");
                e.into_response()
            }
            AppError::ServerError {
                error,
                msg,
                backtrace,
            } => {
                tracing::error!(
                    error = %error,
                    backtrace = %backtrace,
                    "server error"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        #[cfg(debug_assertions)]
                        ErrorResponse {
                            code: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Internal server error".into(),
                            debug_info: Some(HashMap::from([
                                ("backtrace", serde_json::to_value(&backtrace).unwrap()),
                                ("error", serde_json::to_value(&error).unwrap()),
                            ])),
                        },
                        #[cfg(not(debug_assertions))]
                        ErrorResponse {
                            code: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: msg.unwrap_or_else(|| "Internal server error".into()),
                            debug_info: None,
                        },
                    ),
                )
                    .into_response()
            }
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::ServerError {
            error: ServerError::DatabaseError(e),
            msg: None,
            backtrace: create_backtrace(),
        }
    }
}

impl From<&'static str> for AppError {
    fn from(e: &'static str) -> Self {
        AppError::ServerError {
            error: ServerError::Unknown(e.into()),
            msg: None,
            backtrace: create_backtrace(),
        }
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError::ServerError {
            error: ServerError::Unknown(e),
            msg: None,
            backtrace: create_backtrace(),
        }
    }
}

pub struct ApiErrorImpl(pub &'static str);

impl std::fmt::Display for ApiErrorImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ApiErrorImpl> for AppError {
    fn from(e: ApiErrorImpl) -> Self {
        AppError::ApiError(Box::new(e))
    }
}

impl ApiError for ApiErrorImpl {}

/// An error that is used to represent an internal server error. Use this when
/// you want to include a custom context message into the error.
pub struct InternalServerError(pub &'static str, pub Option<Box<dyn std::error::Error>>);

impl std::fmt::Display for InternalServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<InternalServerError> for AppError {
    fn from(e: InternalServerError) -> Self {
        AppError::ServerError {
            error: ServerError::Unknown({
                let mut msg = e.0.to_string();
                if let Some(err) = e.1 {
                    msg.push_str(&format!(": {}", err));
                }
                msg
            }),
            msg: Some(e.0.to_string()),
            backtrace: create_backtrace(),
        }
    }
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        AppError::ServerError {
            error: ServerError::HttpClientError(value),
            msg: None,
            backtrace: create_backtrace(),
        }
    }
}

#[derive(Serialize, Debug)]
struct FrameInfo {
    name: String,
    loc: String,
}

impl std::fmt::Display for FrameInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\tat {}", self.name, self.loc)
    }
}

#[derive(Debug, Serialize)]
struct Backtrace(Vec<FrameInfo>);

impl std::fmt::Display for Backtrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for frame in self.0.iter() {
            write!(f, "{}\n", frame)?;
        }
        Ok(())
    }
}

fn create_backtrace() -> Backtrace {
    let backtrace = backtrace::Backtrace::new();
    Backtrace(filter_backtrace(Some(&backtrace)))
}

fn filter_backtrace(backtrace: Option<&backtrace::Backtrace>) -> Vec<FrameInfo> {
    match backtrace {
        Some(backtrace) => {
            const MODULE_PREFIX: &str = concat!(env!("CARGO_PKG_NAME"), "::");
            let mut frames_info: Vec<FrameInfo> = Vec::new();

            for frame in backtrace.frames() {
                for symbol in frame.symbols() {
                    if let (Some(name), Some(filename), Some(lineno)) = (
                        symbol.name().map(|n| n.to_string()),
                        symbol.filename().map(|f| f.to_owned()),
                        symbol.lineno(),
                    ) {
                        if name.contains(MODULE_PREFIX) {
                            frames_info.push(FrameInfo {
                                name,
                                loc: format!("{}:{}", filename.to_str().unwrap(), lineno),
                            });
                        }
                    }
                }
            }

            // Pop the two first frames, which are the `filter_backtrace` and
            // `create_backtrace` functions
            if frames_info.len() >= 2 {
                frames_info.drain(0..2);
            }

            frames_info
        }
        None => Vec::new(),
    }
}
