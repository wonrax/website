use axum::{
    extract::{ConnectInfo, Path, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use dotenv::dotenv;
use serde::Serialize;
use sqlx::{
    postgres::PgPoolOptions,
    FromRow, Pool, Postgres,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};

mod utils;
use utils::{readable_uint, render_template};

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
        .route(
            "/public/github-profile-views",
            get(handle_fetch_git_hub_profile_views),
        )
        .route("/public/blog/:slug/comments", get(get_blog_post_comments))
        .with_state(shared_state);

    // run it
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn handle_fetch_git_hub_profile_views<'a>(
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

#[derive(FromRow, Debug, Serialize)]
struct Comment {
    id: i32,
    author_ip: String,
    author_name: String,
    author_email: String,
    content: String,
    post_id: i32,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
}

async fn get_blog_post_comments(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
) -> Json<Vec<Comment>> {
    let rows = sqlx::query_as::<_, Comment>(
        "
        SELECT * FROM comments
        JOIN posts ON posts.id = comments.post_id
        WHERE posts.slug = $1;
        ",
    )
    .bind(slug)
    .fetch_all(&ctx.pool)
    .await;

    match rows {
        Ok(rows) => Json(rows),
        Err(_) => Json(vec![]),
    }
}
