//! Run with
//!
//! ```not_rust
//! cargo run -p example-hello-world
//! ```

use axum::{response::Html, routing::get, Extension, Router};
use handlebars::Handlebars;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{net::SocketAddr, time::Duration};

#[derive(Clone)]
struct APIContext<'a> {
    pool: PgPool,
    handlebars: Handlebars<'a>,
}

mod db_migrations {
    use refinery::embed_migrations;
    embed_migrations!("./migrations");
}

#[tokio::main]
async fn main() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgres://postgres@localhost:5432/hhai-dev")
        .await
        .expect("couldn't connect to db");

    // migration only connection
    let (mut client, conn) = tokio_postgres::connect(
        "postgres://postgres@localhost:5432/hhai-dev",
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("connection error: {}", e);
        }
    });

    print!("is closed {:?}", client.is_closed());

    let res = client.query_one("SELECT 12", &[]).await.unwrap();
    print!("{:?}", res);

    db_migrations::migrations::runner()
        .set_migration_table_name("migrations")
        .run_async(&mut client)
        .await
        .unwrap();

    let github_views_html_template: &str = include_str!("github.html");

    let mut handlebars = Handlebars::new();
    handlebars
        .register_template_string("github-views", github_views_html_template)
        .unwrap();

    // build our application with a route
    let app = Router::new()
        .route("/github", get(handler))
        .route("/sleep", get(sleep_handler))
        .layer(Extension(APIContext { pool, handlebars }));

    // run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handler(ctx: Extension<APIContext<'_>>) -> Html<String> {
    let row: (i32,) = sqlx::query_as("SELECT 123;")
        .bind(150_i32)
        .fetch_one(&ctx.pool)
        .await
        .expect("bruh");
    let res = (&ctx.handlebars)
        .render("github-views", &json!({"views": row.0}))
        .unwrap();
    Html(res)
}

async fn sleep_handler() -> Html<String> {
    tokio::time::sleep(Duration::from_secs(5)).await;
    Html("ok".to_string())
}
