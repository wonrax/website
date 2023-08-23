use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use dotenv::dotenv;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::{net::SocketAddr, sync::Arc, time::Duration};

#[derive(Clone)]
struct APIContext {
    pool: Pool<Postgres>,
    counters_ttl_cache: Arc<retainer::Cache<String, bool>>,
}

const GITHUB_VIEWS_HTML_TEMPLATE: &str = include_str!("github.html");

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

async fn handler<'a>(
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

    match row {
        Ok(row) => {
            let readable_views = readable_uint(row.0.to_string());
            let int_length = readable_views.len();
            let width = 16 + (int_length * 6);
            let total_width = width + 81;
            let x_offset = 81 + (width / 2);

            let res = render_template(
                GITHUB_VIEWS_HTML_TEMPLATE,
                &[
                    ("{{views}}", &readable_views),
                    ("{{views-width}}", &width.to_string()),
                    ("{{total-width}}", &total_width.to_string()),
                    ("{{views-offset-x}}", &x_offset.to_string()),
                ],
            );

            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "image/svg+xml".parse().unwrap());
            headers.insert(
                header::CACHE_CONTROL,
                "max-age=0, no-cache, no-store, must-revalidate"
                    .parse()
                    .unwrap(),
            );
            (headers, res).into_response()
        }
        Err(e) => e.to_string().into_response(),
    }
}

fn render_template(template: &str, data: &[(&str, &str)]) -> String {
    let mut result = String::from(template);

    for (placeholder, value) in data {
        result = result.replace(placeholder, value);
    }

    result
}

fn readable_uint(int_str: String) -> String {
    let mut s = String::new();
    for (i ,char) in int_str.chars().rev().enumerate() {
        if i % 3 == 0 && i != 0 {
            s.insert(0, ',');
        }
        s.insert(0, char);
    }
    return s
}
