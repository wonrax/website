use axum::{routing::get, Json, Router, response::IntoResponse};
use dotenv::dotenv;
use serde::Serialize;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::{net::SocketAddr, sync::Arc, time::Duration};

mod utils;

mod blog;
mod github;

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

    // build our application with a route
    let app = Router::new()
        .route("/health", get(heath))
        .nest("/public/blog", blog::routes::route())
        .nest("/public", github::routes::route())
        .with_state(shared_state);

    // run it
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
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
