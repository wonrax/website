use axum::{
    http::{header::CONTENT_TYPE, Method},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use dotenv::dotenv;
use mimalloc::MiMalloc;
use serde::Serialize;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

mod blog;
mod error;
mod github;
mod utils;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Clone)]
pub struct APIContext {
    pool: Pool<Postgres>,
    counters_ttl_cache: Arc<retainer::Cache<String, bool>>,
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    let postgres_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .idle_timeout(Duration::from_secs(120))
        .acquire_timeout(Duration::from_secs(10))
        .connect(&postgres_url)
        .await
        .expect("couldn't connect to db");

    let shared_state = APIContext {
        pool,
        counters_ttl_cache: Arc::from(retainer::Cache::new()),
    };

    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods(vec![Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE])
        .allow_origin(AllowOrigin::predicate(|_, request| {
            request
                .headers
                .get("origin")
                .map(|origin| origin.to_str().unwrap().starts_with("http://localhost:"))
                .unwrap_or(false)
        }));

    // build our application with a route
    let app = Router::new()
        .route("/health", get(heath))
        .nest("/public/blog", blog::routes::route())
        .nest("/public", github::routes::route())
        .layer(cors)
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("listening on {}", "0.0.0.0:3000");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[derive(Serialize)]
struct Health {
    status: i32,
    msg: String,
    detail: Option<String>,
}

async fn heath() -> impl IntoResponse {
    Json(json!({
        "status": 200,
        "msg": "OK",
        "detail": None::<String>,
    }))
}
