use axum::{
    extract::MatchedPath,
    http::{header::CONTENT_TYPE, Method, Request},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use dotenv::dotenv;
use mimalloc::MiMalloc;
use serde::Serialize;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower_http::{
    classify::ServerErrorsFailureClass,
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use tracing::{debug, error, info, info_span, Span};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod blog;
mod crypto;
mod error;
mod github;
mod identity;
mod utils;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Clone)]
enum EnvironmentType {
    Dev,
    Staging,
    Production,
}

#[derive(Clone)]
struct ServerConfig {
    environment: EnvironmentType,
}

#[derive(Clone)]
pub struct APIContext {
    pool: Pool<Postgres>,
    counters_ttl_cache: Arc<retainer::Cache<String, bool>>,
    config: ServerConfig,
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

    let config = ServerConfig {
        environment: match std::env::var("ENVIRONMENT") {
            Ok(env) => match env.as_str() {
                "dev" => EnvironmentType::Dev,
                "staging" => EnvironmentType::Staging,
                "production" => EnvironmentType::Production,
                _ => EnvironmentType::Dev,
            },
            Err(_) => EnvironmentType::Dev,
        },
    };

    let (json, pretty) = match config.environment {
        EnvironmentType::Dev => (None, Some(tracing_subscriber::fmt::layer().pretty())),
        _ => (
            Some(tracing_subscriber::fmt::layer().json().flatten_event(true)),
            None,
        ),
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or("info".into()))
        .with(json)
        .with(pretty)
        .init();

    let shared_state = APIContext {
        pool,
        counters_ttl_cache: Arc::from(retainer::Cache::new()),
        config,
    };

    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods(vec![Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE])
        .allow_credentials(true)
        .allow_origin(AllowOrigin::predicate(|_, request| {
            request
                .headers
                .get("origin")
                .map(|origin| {
                    if let Ok(origin) = origin.to_str() {
                        origin.starts_with("http://localhost:")
                            || origin.starts_with("https://hhai.dev")
                    } else {
                        false
                    }
                })
                .unwrap_or(false)
        }));

    // build our application with a route
    let app = Router::new()
        .route("/health", get(heath))
        .nest("/blog", blog::routes::route(shared_state.clone()))
        .nest("/public", github::routes::route())
        .nest("/identity", identity::routes::route())
        .layer(cors)
        .with_state(shared_state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    // Log the matched route's path (with placeholders not filled in).
                    // Use request.uri() or OriginalUri if you want the real path.
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);

                    info_span!(
                        "http_request",
                        method = ?request.method(),
                        matched_path,
                    )
                })
                .on_response(|_response: &Response, _latency: Duration, _span: &Span| {
                    if !_response.status().is_server_error() {
                        debug!(
                            time = ?_latency,
                            status = ?_response.status(),
                            "response",
                        );
                    }
                })
                .on_failure(
                    |_error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                        // TODO when encouter an error, add it to the span so
                        // we can log it here
                        error!(
                            time = ?_latency,
                            error = ?_error,
                            "response_failure",
                        );
                    },
                ),
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("listening on 0.0.0.0:3000");
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
