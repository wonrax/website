//! Run with
//!
//! ```not_rust
//! cargo run -p example-hello-world
//! ```

use axum::{response::Html, routing::get, Router};
use handlebars::Handlebars;
use std::net::SocketAddr;
use serde_json::json;

#[tokio::main]
async fn main() {
    // build our application with a route
    let app = Router::new().route("/github", get(handler));

    // run it
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

const TEMPLATE_HTML: &'static str = include_str!("github.html");

async fn handler() -> Html<String> {
    let reg = Handlebars::new();
    let res = reg.render_template(TEMPLATE_HTML, &json!({"views": 190})).unwrap();
    Html(res)
}
