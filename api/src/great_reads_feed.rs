use axum::{response::IntoResponse, http::StatusCode, extract::State};
use crate::App;

pub async fn proxy_rss(State(app): State<App>) -> impl IntoResponse {
    let url = "https://bg.raindrop.io/rss/public/55948413";
    match app.http.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = [(axum::http::header::CONTENT_TYPE, "application/xml")];
            let bytes = resp.bytes().await.unwrap_or_default();
            (status, headers, bytes).into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to fetch RSS feed").into_response(),
    }
}
