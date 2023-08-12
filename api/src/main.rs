//! Run with
//!
//! ```not_rust
//! cargo run -p example-hello-world
//! ```

use axum::{response::Html, routing::get, Extension, Router};
use handlebars::Handlebars;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect("postgres://postgres:SECURE_PASSWORD_HERE@localhost:5432/hhai-dev")
        .await
        .expect("couldn't connect to db");

    // build our application with a route
    let app = Router::new()
        .route("/github", get(handler))
        .layer(Extension(pool));

    // run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

const TEMPLATE_HTML: &'static str = include_str!("github.html");

async fn handler(ctx: Extension<PgPool>) -> Html<String> {
    let row: (i32,) = sqlx::query_as("SELECT 123;")
        .bind(150_i32)
        .fetch_one(&ctx.0)
        .await
        .expect("bruh");
    let reg = Handlebars::new();
    let res = reg
        .render_template(TEMPLATE_HTML, &json!({"views": row.0}))
        .unwrap();
    Html(res)
}
