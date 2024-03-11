use std::collections::HashMap;

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug)]
pub enum ServerError {
    DatabaseError(sqlx::Error),
}

impl Serialize for ServerError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            ServerError::DatabaseError(e) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("message", &e.to_string())?;
                map.end()
            }
        }
    }
}

#[derive(Serialize)]
pub enum AppError {
    ServerError {
        error: ServerError,

        #[serde(skip_serializing)]
        #[cfg(debug_assertions)]
        backtrace: Option<backtrace::Backtrace>,
    },
    Unhandled(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    code: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    msg: Option<String>,

    #[cfg(debug_assertions)]
    debug_info: Option<HashMap<&'static str, Value>>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status_code, error_response) = match self {
            AppError::ServerError {
                error,
                #[cfg(debug_assertions)]
                backtrace,
            } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                #[cfg(debug_assertions)]
                {
                    let frames_info = filter_backtrace(backtrace.as_ref().unwrap());
                    ErrorResponse {
                        code: "DATABASE_ERR".into(),
                        msg: Some("Database error".into()),
                        debug_info: Some(HashMap::from([
                            ("backtrace", serde_json::to_value(&frames_info).unwrap()),
                            ("error", serde_json::to_value(&error).unwrap()),
                        ])),
                    }
                },
                #[cfg(not(debug_assertions))]
                ErrorResponse {
                    code: "SERVER_ERR".into(),
                    msg: Some("Internal server error".into()),
                },
            ),
            AppError::Unhandled(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                #[cfg(debug_assertions)]
                ErrorResponse {
                    code: "ERR".into(),
                    msg: Some(e),
                    debug_info: None,
                },
                #[cfg(not(debug_assertions))]
                ErrorResponse {
                    code: "ERR".into(),
                    msg: Some(e),
                },
            ),
        };

        (status_code, Json(error_response)).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::ServerError {
            error: ServerError::DatabaseError(e),

            #[cfg(debug_assertions)]
            backtrace: Some(backtrace::Backtrace::new()),
        }
    }
}

impl From<&'static str> for AppError {
    fn from(e: &'static str) -> Self {
        AppError::Unhandled(e.into())
    }
}

#[derive(Serialize, Debug)]
struct FrameInfo {
    name: String,
    loc: String,
}

fn filter_backtrace(backtrace: &backtrace::Backtrace) -> Vec<FrameInfo> {
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

    return frames_info;
}
