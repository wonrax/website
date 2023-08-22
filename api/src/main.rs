//! Run with
//!
//! ```not_rust
//! cargo run -p example-hello-world
//! ```

use axum::{
    extract::ConnectInfo,
    http::{header, HeaderMap},
    response::Html,
    routing::get,
    Extension, Router,
};
use handlebars::Handlebars;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{net::SocketAddr, time::Duration};

#[derive(Clone)]
struct APIContext<'a> {
    pool: PgPool,
    handlebars: Handlebars<'a>,
    counters_ttl_cache: ttl_cache::TtlCache<String, bool>,
}

#[tokio::main]
async fn main() {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .idle_timeout(Duration::from_secs(120))
        .acquire_timeout(Duration::from_secs(10))
        .connect("postgres://postgres@localhost:5432/hhai-dev")
        .await
        .expect("couldn't connect to db");

    let github_views_html_template: &str = include_str!("github.html");

    let mut handlebars = Handlebars::new();
    handlebars
        .register_template_string("github-views", github_views_html_template)
        .unwrap();

    // build our application with a route
    let app = Router::new()
        .route("/github", get(handler))
        .route("/sleep", get(sleep_handler))
        .layer(Extension(APIContext {
            pool,
            handlebars,
            counters_ttl_cache: ttl_cache::TtlCache::new(10_000),
        }));

    // run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn handler(
    mut ctx: Extension<APIContext<'_>>,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Html<String> {
    let mut _ip: String = "".into();
    if headers.contains_key("x-forwarded-for") {
        _ip = headers["x-forwarded-for"]
            .to_str()
            .unwrap_or_default()
            .into();
    } else {
        _ip = addr.ip().to_string();
    }

    println!("ip: {}", _ip);

    let row: Result<(i64,), sqlx::Error>;

    // TODO: this cache is cloned for every request so technically it's not
    // working as expected.
    // Refer to this to fix it:
    // https://github.com/tokio-rs/axum/blob/main/examples/key-value-store/src/main.rs
    if ctx.counters_ttl_cache.contains_key(&_ip) {
        ctx.counters_ttl_cache.insert(_ip.clone(), true, Duration::from_secs(3600));
        row = sqlx::query_as(
            "
        INSERT INTO counters (key, name, count)
        VALUES ('github-profile-views', 'wonrax', 1)
        ON CONFLICT (key, name)
        SET count = count WHERE FALSE -- never executed, but still locks the row
        RETURNING count;
    ",
        )
        .fetch_one(&ctx.pool)
        .await;
    } else {
        row = sqlx::query_as(
            "
        INSERT INTO counters (key, name, count)
        VALUES ('github-profile-views', 'wonrax', 1)
        ON CONFLICT (key, name)
        DO UPDATE SET count = counters.count + 1
        RETURNING count;
    ",
        )
        .fetch_one(&ctx.pool)
        .await;
    }

    match row {
        Ok(row) => {
            let res = (&ctx.handlebars)
                .render("github-views", &json!({"views": row.0}))
                .unwrap();
            Html(res)
        }
        Err(e) => Html(e.to_string()),
    }
}

async fn sleep_handler() -> Html<String> {
    tokio::time::sleep(Duration::from_secs(5)).await;
    Html("ok".to_string())
}
