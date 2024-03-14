use std::{collections::HashMap, fmt::Debug};

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

/// The main error type for the application. Every handler should return this
/// type as the error type.
pub enum AppError {
    ServerError {
        error: ServerError,
        reason: Option<String>,
        backtrace: Backtrace,
    },
    ApiError(Box<dyn ApiRequestError>),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::ServerError { error, .. } => write!(f, "Server error: {}", error),
            AppError::ApiError(e) => write!(f, "API error: {}", e),
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::ServerError {
            error: ServerError::DatabaseError(e),
            reason: None,
            backtrace: create_backtrace(),
        }
    }
}

impl From<&'static str> for AppError {
    fn from(e: &'static str) -> Self {
        AppError::ServerError {
            error: ServerError::Unknown(e.into()),
            reason: None,
            backtrace: create_backtrace(),
        }
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError::ServerError {
            error: ServerError::Unknown(e),
            reason: None,
            backtrace: create_backtrace(),
        }
    }
}

#[derive(Debug)]
pub enum ServerError {
    DatabaseError(sqlx::Error),
    HttpClientError(reqwest::Error),
    Any(Box<dyn std::error::Error>),
    Unknown(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DatabaseError(e) => write!(f, "Database error: {}", e),
            Self::HttpClientError(e) => write!(f, "HTTP client error: {}", e),
            Self::Any(e) => write!(f, "Error: {}", e),
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

pub trait ApiRequestError: std::fmt::Display {
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

impl From<(&'static str, StatusCode)> for AppError {
    fn from((message, status_code): (&'static str, StatusCode)) -> Self {
        ApiStringError::new(message)
            .with_status_code(status_code)
            .into()
    }
}

impl<T> From<T> for AppError
where
    T: ApiRequestError + 'static,
{
    fn from(value: T) -> Self {
        AppError::ApiError(Box::new(value))
    }
}

/// An API error that has a static string message as the underlying error. This
/// is useful when you want to return a quick and simple error message.
pub struct ApiStringError(&'static str, Option<StatusCode>);

impl ApiStringError {
    pub fn new(msg: &'static str) -> Self {
        Self(msg, None)
    }

    pub fn with_status_code(mut self, status_code: StatusCode) -> Self {
        self.1 = Some(status_code);
        self
    }
}

impl std::fmt::Display for ApiStringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ApiRequestError for ApiStringError {
    fn status_code(&self) -> StatusCode {
        self.1.unwrap_or(StatusCode::BAD_REQUEST)
    }
}

#[derive(Serialize)]
pub struct ErrorResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,

    msg: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<HashMap<&'static str, Value>>,
}

impl ErrorResponse {
    pub fn new(msg: &str) -> Self {
        Self {
            code: None,
            msg: msg.into(),
            reason: None,
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
                reason,
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
                            reason,
                            debug_info: Some(HashMap::from([
                                ("backtrace", serde_json::to_value(&backtrace).unwrap()),
                                ("error", serde_json::to_value(&error).unwrap()),
                            ])),
                        },
                        #[cfg(not(debug_assertions))]
                        ErrorResponse {
                            code: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Internal server error".into(),
                            reason,
                            debug_info: None,
                        },
                    ),
                )
                    .into_response()
            }
        }
    }
}

/// An error that is used to represent an internal server error. Use this when
/// you want to include a custom context message into the error.
pub struct InternalServerError(&'static str, Option<Box<dyn std::error::Error>>);

impl InternalServerError {
    pub fn new(msg: &'static str) -> Self {
        Self(msg, None)
    }

    pub fn with_error<E: std::error::Error + 'static>(msg: &'static str, err: E) -> Self {
        Self(msg, Some(Box::new(err)))
    }
}

impl std::fmt::Display for InternalServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(err) = &self.1 {
            write!(f, "{}: {}", self.0, err)
        } else {
            write!(f, "{}", self.0)
        }
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
            reason: Some(e.0.to_string()),
            backtrace: create_backtrace(),
        }
    }
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        AppError::ServerError {
            error: ServerError::HttpClientError(value),
            reason: None,
            backtrace: create_backtrace(),
        }
    }
}

#[derive(Serialize, Debug)]
struct BacktraceFrame {
    name: String,
    loc: String,
}

impl std::fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\tat {}", self.name, self.loc)
    }
}

#[derive(Debug, Serialize)]
pub struct Backtrace(Vec<BacktraceFrame>);

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

fn filter_backtrace(backtrace: Option<&backtrace::Backtrace>) -> Vec<BacktraceFrame> {
    match backtrace {
        Some(backtrace) => {
            const MODULE_PREFIX: &str = concat!(env!("CARGO_PKG_NAME"), "::");
            let mut frames_info: Vec<BacktraceFrame> = Vec::new();

            for frame in backtrace.frames() {
                for symbol in frame.symbols() {
                    if let (Some(name), Some(filename), Some(lineno)) = (
                        symbol.name().map(|n| n.to_string()),
                        symbol.filename().map(|f| f.to_owned()),
                        symbol.lineno(),
                    ) {
                        if name.contains(MODULE_PREFIX) {
                            frames_info.push(BacktraceFrame {
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
