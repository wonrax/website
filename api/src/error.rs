use std::collections::HashMap;

use axum::{Json, http::StatusCode, response::IntoResponse};
use eyre::eyre;
use serde::Serialize;
use serde_json::Value;
use tracing::debug;

/// The main error type for the application. Every handler should return this
/// type as the error type. This error type when created will be attached
/// debugging info and get logged automatically. If you intent to not use this
/// type, you have to handle logging and debugging by yourself.
pub struct AppError {
    error: Inner,
    reason: Option<serde_json::Value>,
    backtrace: Option<Backtrace>,
    context: Option<HashMap<String, serde_json::Value>>,
}

/// The error type for internal errors, should be used for reporting and
/// debugging purpose only and not be exposed to the client. When this error is
/// turned into a [AppError] (e.g. by [Into] or [From]), it defaults to a server
/// error and leaks no debugging context to the client.
pub type Error = eyre::Error;

pub enum Inner {
    /// When the wrapper [AppError] is turned into a [axum::response::Response],
    /// the message displayed to the client will always be a internal server
    /// error with no underlying error expose. Serverside, this will get logged
    /// for debugging purpose.
    ServerError(Error),

    /// Expected and properly handled error that will be displayed on the
    /// client.
    ApiError(Box<dyn ApiRequestError>),
}

impl From<Error> for AppError {
    fn from(value: Error) -> Self {
        AppError {
            error: Inner::ServerError(value),
            backtrace: Some(create_backtrace()),
            context: None,
            reason: None,
        }
    }
}

impl std::fmt::Display for AppError {
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

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Inner::ServerError(e) => write!(f, "Server error: {}", e),
            Inner::ApiError(e) => write!(f, "API error: {}", e),
        }
    }
}

impl From<diesel::result::Error> for AppError {
    fn from(e: diesel::result::Error) -> Self {
        AppError {
            error: Inner::ServerError(eyre!(e)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<&'static str> for AppError {
    fn from(e: &'static str) -> Self {
        AppError {
            error: Inner::ServerError(eyre!(e)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError {
            error: Inner::ServerError(eyre!(e)),
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

    #[allow(clippy::wrong_self_convention)]
    fn into_response(&self) -> axum::response::Response {
        let error = self.error();
        let status_code = self.status_code();
        (status_code, Json(error)).into_response()
    }
}

impl From<(&'static str, StatusCode)> for AppError {
    fn from((message, status_code): (&'static str, StatusCode)) -> Self {
        AppError {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(message).with_status_code(status_code),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<(String, StatusCode)> for AppError {
    fn from((message, status_code): (String, StatusCode)) -> Self {
        AppError {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(message).with_status_code(status_code),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<(ErrorCode, &'static str, StatusCode)> for AppError {
    fn from(value: (ErrorCode, &'static str, StatusCode)) -> Self {
        AppError {
            error: Inner::ApiError(Box::new(
                ErrorResponseBuilder::new(value.1)
                    .with_code(value.0.0)
                    .with_status_code(value.2),
            )),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

#[derive(Debug)]
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

    #[allow(dead_code)]
    fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[allow(dead_code)]
    fn with_debug_info(mut self, debug_info: HashMap<&'static str, Value>) -> Self {
        self.debug_info = Some(debug_info);
        self
    }

    fn with_status_code(mut self, status_code: StatusCode) -> Self {
        self.status_code = Some(status_code);
        self
    }

    #[allow(dead_code)]
    fn with_context(mut self, context: HashMap<String, serde_json::Value>) -> Self {
        self.context = Some(context);
        self
    }

    fn build(&self) -> ErrorResponse {
        ErrorResponse {
            error: self.code.to_owned(),
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

impl<T> From<T> for AppError
where
    T: ApiRequestError + 'static,
{
    fn from(value: T) -> Self {
        AppError {
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
    error: Option<String>,

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
            error: None,
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
            self.error.as_deref().unwrap_or("UNKNOWN"),
            self.msg,
            self.reason.as_ref().unwrap_or(&serde_json::Value::Null)
        )
    }
}

impl IntoResponse for AppError {
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
                    error = ?e,
                    backtrace = %self.backtrace.clone().unwrap_or_default(),
                    context = ?self.context, // TODO turn this into tracing::Value to prettify the logs
                    "Internal server error"
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        #[cfg(debug_assertions)]
                        ErrorResponse {
                            error: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Something has gone wrong from our side. \
                                  We'll try to fix this as soon as possible. \
                                  Please try again later."
                                .into(),
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
                            error: Some("INTERNAL_SERVER_ERROR".into()),
                            msg: "Something has gone wrong from our side. \
                                  We'll try to fix this as soon as possible. \
                                  Please try again later."
                                .into(),
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

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        AppError {
            error: Inner::ServerError(eyre!(value)),
            reason: None,
            backtrace: Some(create_backtrace()),
            context: None,
        }
    }
}

impl From<diesel_async::pooled_connection::deadpool::PoolError> for AppError {
    fn from(e: diesel_async::pooled_connection::deadpool::PoolError) -> Self {
        AppError {
            error: Inner::ServerError(eyre!(e)),
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
            writeln!(f, "{}", frame)?;
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
