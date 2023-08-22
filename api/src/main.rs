use axum::{
    debug_handler,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
    routing::get,
    Router, response::{Response, IntoResponse},
};
use dotenv::dotenv;
use handlebars::Handlebars;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::{net::SocketAddr, sync::Arc, time::Duration};

#[derive(Clone)]
struct APIContext {
    pool: Pool<Postgres>,
    handlebars: Handlebars<'static>,
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

    let github_views_html_template: &str = include_str!("github.html");

    let mut handlebars = Handlebars::new();
    handlebars
        .register_template_string("github-views", github_views_html_template)
        .unwrap();

    let shared_state = APIContext {
        pool,
        handlebars,
        counters_ttl_cache: Arc::from(retainer::Cache::new()),
    };

    // build our application with a route
    let app = Router::new()
        .route("/public/github-profile-views", get(handler))
        .with_state(shared_state);

    // run it
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

#[debug_handler]
async fn handler(
    State(ctx): State<APIContext>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Response {
    let mut _ip: String = "".into();
    // Trusted proxy from cloudflare so we can use x-forwarded-for
    if headers.contains_key("x-forwarded-for") {
        _ip = headers["x-forwarded-for"]
            .to_str()
            .unwrap_or_default()
            .into();
    } else {
        _ip = addr.ip().to_string();
    }

    let cache = ctx.counters_ttl_cache;

    let mut row: Result<(i64,), sqlx::Error>;

    if cache.get(&_ip).await.is_none() {
        cache
            .insert(_ip.clone(), true, Duration::from_secs(1))
            .await;
        row = sqlx::query_as(
            "
            UPDATE counters SET count = count + 1
            WHERE key = 'github-profile-views' AND name = 'wonrax'
            RETURNING count;
        ",
        )
        .fetch_one(&ctx.pool)
        .await;
    } else {
        row = sqlx::query_as(
            "
            SELECT count FROM counters
            WHERE key = 'github-profile-views' AND name = 'wonrax';
        ",
        )
        .fetch_one(&ctx.pool)
        .await;
    }

    if let Err(_) = row {
        let _ = sqlx::query(
            "
            INSERT INTO counters (key, name, count)
            VALUES ('github-profile-views', 'wonrax', 1);
        ",
        )
        .execute(&ctx.pool)
        .await;

        row = Ok((1,))
    }

    let handlebars = &ctx.handlebars;

    match row {
        Ok(row) => {
            let res = (&handlebars)
                .render("github-views", &json!({"views": row.0}))
                .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "image/svg+xml".parse().unwrap());
            headers.insert(header::CACHE_CONTROL, "max-age=0, no-cache, no-store, must-revalidate".parse().unwrap());
            (
                headers,
                res
            ).into_response()
        }
        Err(e) => e.to_string().into_response(),
    }
}
