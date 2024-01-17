use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

pub enum AppError {
    DatabaseError(sqlx::Error),
    Unhandled(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    code: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    msg: Option<String>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status_code, error_response) = match self {
            // TODO log the error
            AppError::DatabaseError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                #[cfg(debug_assertions)]
                ErrorResponse {
                    code: "DB_ERR".into(),
                    msg: Some(format!("Database error: {}", e.to_string())),
                },
                #[cfg(not(debug_assertions))]
                ErrorResponse {
                    code: "SVR_ERR".into(),
                    msg: Some("Internal server error".into()),
                },
            ),
            AppError::Unhandled(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
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
        AppError::DatabaseError(e)
    }
}

impl From<&'static str> for AppError {
    fn from(e: &'static str) -> Self {
        AppError::Unhandled(e.into())
    }
}
