use axum::{
    async_trait,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
};

use crate::error::AppError;

// We define our own `Json` extractor that customizes the error from `axum::Json`
pub struct Json<T>(pub T);

#[async_trait]
impl<S, T> FromRequest<S> for Json<T>
where
    axum::Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();

        let req = Request::from_parts(parts, body);

        match axum::Json::<T>::from_request(req, state).await {
            Ok(value) => Ok(Self(value.0)),
            // convert the error from `axum::Json` into whatever we want
            Err(rejection) => Err((rejection.body_text(), StatusCode::UNPROCESSABLE_ENTITY).into()),
        }
    }
}
