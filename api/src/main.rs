use axum::{
    Json, Router,
    extract::{MatchedPath, Query},
    http::{Method, Request, header::CONTENT_TYPE},
    response::Response,
    routing::get,
};
use config::ServerConfig;
use dotenv::dotenv;
use mimalloc::MiMalloc;
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, ops::Deref, sync::Arc, time::Duration};
use tower_http::{
    classify::ServerErrorsFailureClass,
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use tracing::{Span, debug, error, info, info_span};
use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::error::AppError;
#[cfg(debug_assertions)]
use crate::real_ip::ClientIp;

mod blog;
mod config;
mod crypto;
mod discord;
mod error;
mod github;
mod great_reads_feed;
mod identity;
mod json;
mod models;
mod real_ip;
mod schema;
mod utils;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Clone)]
pub struct App(Arc<Inner>);

impl Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Inner {
    counters_ttl_cache: retainer::Cache<String, bool>,
    great_reads_cache: retainer::Cache<String, Vec<u8>>,
    config: ServerConfig,
    diesel: diesel_async::pooled_connection::deadpool::Pool<diesel_async::AsyncPgConnection>,
    http: reqwest::Client,
}

#[tokio::main]
async fn main() {
    // temp subscriber for logging in the configuration loading phase
    let d = tracing_subscriber::FmtSubscriber::builder()
        .pretty()
        .compact()
        .finish();

    dotenv().ok();

    let config = tracing::subscriber::with_default(d, ServerConfig::new_from_env);

    const HTTP_REQUEST_SPAN: &str = "http_request";
    let (json, pretty, json_span, pretty_span) = match config.env {
        config::Env::Dev => (
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NONE),
            ),
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        // If it's a span, only enable it if it's NOT "http_request"
                        if metadata.is_span() {
                            metadata.name() != HTTP_REQUEST_SPAN
                        } else {
                            // If it's a log event (info!, etc.), ignore it
                            // (because log_layer already handles these)
                            false
                        }
                    })),
            ),
        ),
        _ => (
            Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_span_list(true)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NONE),
            ),
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_span_list(true)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        // If it's a span, only enable it if it's NOT "http_request"
                        if metadata.is_span() {
                            metadata.name() != HTTP_REQUEST_SPAN
                        } else {
                            // If it's a log event (info!, etc.), ignore it
                            // (because log_layer already handles these)
                            false
                        }
                    })),
            ),
            None,
        ),
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or("info".into()))
        .with(json)
        .with(pretty)
        .with(json_span)
        .with(pretty_span)
        .init();

    let postgres_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");

    let diesel_manager = diesel_async::pooled_connection::AsyncDieselConnectionManager::<
        diesel_async::AsyncPgConnection,
    >::new(postgres_url);
    // TODO consider using bb8 pool since it has more features (min_idle, max_lifetime etc.)
    let diesel_pool = diesel_async::pooled_connection::deadpool::Pool::builder(diesel_manager)
        .max_size(7)
        .build()
        .expect("could not build Diesel pool");

    let http_client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client should be correctly constructed");

    let shared_state = App(Arc::new(Inner {
        counters_ttl_cache: retainer::Cache::new(),
        great_reads_cache: retainer::Cache::new(),
        config: config.clone(),
        diesel: diesel_pool,
        http: http_client,
    }));

    let site_url = config.site_url.clone();
    let cors = CorsLayer::new()
        // allow `GET` and `POST` when accessing the resource
        .allow_methods(vec![
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([CONTENT_TYPE])
        .allow_credentials(true)
        .allow_origin(AllowOrigin::predicate(move |_, request| {
            request
                .headers
                .get("origin")
                .map(|origin| {
                    if let Ok(origin) = origin.to_str() {
                        origin.starts_with("http://localhost:") || origin.starts_with(&site_url)
                    } else {
                        false
                    }
                })
                .unwrap_or(false)
        }));

    // build our application with a route
    let app = Router::new()
        .route("/health", get(heath))
        .nest("/blog", blog::routes::route())
        .nest("/public", github::routes::route())
        .merge(identity::routes::route())
        .route("/great-reads-feed", get(great_reads_feed::proxy_rss))
        .route(
            "/great-reads-highlights",
            get(great_reads_feed::get_highlights),
        )
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
                        HTTP_REQUEST_SPAN,
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
                            "request failed",
                        );
                    },
                ),
        );

    tokio::spawn(async move {
        if let Err(e) = start_discord_service(config).await {
            error!("Error starting Discord service: {e:?}");
        }
    });

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("listening on http://0.0.0.0:3000");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn start_discord_service(config: ServerConfig) -> Result<(), eyre::Error> {
    use serenity::all::GatewayIntents;

    if let Some(discord_token) = config.discord_token.clone() {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_TYPING
            | GatewayIntents::DIRECT_MESSAGE_TYPING
            | GatewayIntents::GUILD_PRESENCES;

        // Create a new instance of the Client, logging in as a bot. This will automatically prepend
        // your bot token with "Bot ", which is a requirement by Discord for bot users.
        let mut discord_client = serenity::Client::builder(&discord_token, intents)
            .event_handler(discord::DiscordEventHandler::new(config.clone()).await)
            .await
            .map_err(|e| eyre::eyre!("Error creating Discord client: {e:?}"))?;

        discord_client
            .start()
            .await
            .map_err(|e| eyre::eyre!("Error starting Discord client: {e:?}"))?;

        Ok(())
    } else {
        eyre::bail!("Discord token or OpenAI API key not set in environment variables");
    }
}

async fn heath(
    #[cfg(debug_assertions)] ClientIp(ip): ClientIp,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, AppError> {
    #[cfg(debug_assertions)]
    tracing::debug!(ip = %ip, "Health check request received");

    if query.contains_key("fail") {
        return Err(AppError::from(eyre::eyre!(
            "Simulated failure for health check"
        )));
    }

    Ok(Json(json!({
        "status": 200,
        "msg": "OK",
        "detail": None::<String>,
    })))
}
