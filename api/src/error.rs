use std::collections::HashMap;

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

/// The main error type for the application. Every handler should return this
/// type as the error type.
pub struct Error {
    error: Inner,
    reason: Option<serde_json::Value>,
    backtrace: Option<Backtrace>,
    context: Option<HashMap<String, serde_json::Value>>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (reason: {}, context available: {})",
            self.error,
            self.reason.as_ref().unwrap_or(&Value::Null),
            self.context.is_some()
        )
    }
}

pub enum Inner {
    ServerError(Box<dyn std::error::Error>),
    ApiError(Box<dyn ApiRequestError>),
}

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Inner::ServerError(e) => write!(f, "Server error: {}", e),
            Inner::ApiError(e) => write!(f, "API error: {}", e),
        }
    }
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        Error {
            error: Inner::ServerError(Box::new(e)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<&'static str> for Error {
    fn from(e: &'static str) -> Self {
        #[derive(Debug)]
        struct Wrapper(&'static str);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl std::error::Error for Wrapper {}

        Error {
            error: Inner::ServerError(Box::new(Wrapper(e))),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<String> for Error {
    fn from(e: String) -> Self {
        #[derive(Debug)]
        struct Wrapper(String);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl std::error::Error for Wrapper {}

        Error {
            error: Inner::ServerError(Box::new(Wrapper(e))),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
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

impl From<(&'static str, StatusCode)> for Error {
    fn from((message, status_code): (&'static str, StatusCode)) -> Self {
        Error {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(message).with_status_code(status_code),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<(String, StatusCode)> for Error {
    fn from((message, status_code): (String, StatusCode)) -> Self {
        Error {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(message).with_status_code(status_code),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<(ErrorCode, &'static str, StatusCode)> for Error {
    fn from(value: (ErrorCode, &'static str, StatusCode)) -> Self {
        Error {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(value.1)
                    .with_code(value.0 .0)
                    .with_status_code(value.2),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

struct ErrorResponseBuilder {
    code: Option<String>,
    msg: String,
    reason: Option<serde_json::Value>,
    context: Option<HashMap<String, serde_json::Value>>,
    debug_info: Option<HashMap<&'static str, Value>>,
    status_code: Option<StatusCode>,
}

impl ErrorResponseBuilder {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            code: None,
            msg: msg.into(),
            reason: None,
            context: None,
            debug_info: None,
            status_code: None,
        }
    }

    fn with_code(mut self, code: &str) -> Self {
        self.code = Some(code.into());
        self
    }

    fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.into());
        self
    }

    fn with_debug_info(mut self, debug_info: HashMap<&'static str, Value>) -> Self {
        self.debug_info = Some(debug_info);
        self
    }

    fn with_status_code(mut self, status_code: StatusCode) -> Self {
        self.status_code = Some(status_code);
        self
    }

    fn with_context(mut self, context: HashMap<String, serde_json::Value>) -> Self {
        self.context = Some(context);
        self
    }

    fn build(&self) -> ErrorResponse {
        ErrorResponse {
            code: self.code.to_owned(),
            msg: self.msg.to_owned(),
            reason: self.reason.to_owned(),
            context: self.context.to_owned(),
            debug_info: self.debug_info.to_owned(),
        }
    }
}

impl std::fmt::Display for ErrorResponseBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.build())
    }
}

impl ApiRequestError for ErrorResponseBuilder {
    fn status_code(&self) -> StatusCode {
        self.status_code.unwrap_or(StatusCode::BAD_REQUEST)
    }

    fn error(&self) -> ErrorResponse {
        self.build()
    }
}

pub struct ErrorCode(&'static str);

impl<T> From<T> for Error
where
    T: ApiRequestError + 'static,
{
    fn from(value: T) -> Self {
        Error {
            error: Inner::ApiError(Box::new(value)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

#[derive(Serialize)]
pub struct ErrorResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,

    msg: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<HashMap<String, serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    debug_info: Option<HashMap<&'static str, Value>>,
}

impl ErrorResponse {
    pub fn new(msg: &str) -> Self {
        Self {
            code: None,
            msg: msg.into(),
            reason: None,
            context: None,
            debug_info: None,
        }
    }
}

impl std::fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Code {}: {} ({})",
            self.code.as_deref().unwrap_or("UNKNOWN"),
            self.msg,
            self.reason.as_ref().unwrap_or(&serde_json::Value::Null)
        )
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self.error {
            Inner::ApiError(e) => {
                debug!(
                    api_error = %e,
                    backtrace = %self.backtrace.clone().unwrap_or_default(),
                    context = ?self.context,
                    "api error",
                );
                e.into_response()
            }
            Inner::ServerError(ref e) => {
                tracing::error!(
                    error = %e,
                    backtrace = %self.backtrace.clone().unwrap_or_default(),
                    context = ?self.context,
                    "Internal server error"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        #[cfg(debug_assertions)]
                        ErrorResponse {
                            code: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Something has gone wrong from our side. We'll try to fix this as soon as possible. Please try again.".into(),
                            reason: self.reason,
                            context: self.context,
                            debug_info: Some(HashMap::from([
                                (
                                    "backtrace",
                                    self.backtrace.unwrap_or_default().to_string().into(),
                                ),
                                ("error", self.error.to_string().into()),
                            ])),
                        },
                        #[cfg(not(debug_assertions))]
                        ErrorResponse {
                            code: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Something has gone wrong from our side. We'll try to fix this as soon as possible. Please try again.".into(),
                            reason: self.reason,
                            context: self.context,
                            debug_info: None,
                        },
                    ),
                )
                    .into_response()
            }
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Error {
            error: Inner::ServerError(Box::new(value)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

#[derive(Debug, Clone)]
struct BacktraceFrame {
    name: String,
    loc: String,
}

impl std::fmt::Display for BacktraceFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n\tat {}", self.name, self.loc)
    }
}

#[derive(Debug, Default, Clone)]
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

            frames_info
        }
        None => Vec::new(),
    }
}
