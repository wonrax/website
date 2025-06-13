use std::{net::SocketAddr, time::Duration};

use axum::{
    extract::{ConnectInfo, State},
    http::{
        header::{self, USER_AGENT},
        HeaderMap,
    },
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::{
    utils::{readable_uint, render_template},
    App,
};

const GITHUB_VIEWS_HTML_TEMPLATE: &str = include_str!("github.html");

pub fn route() -> Router<App> {
    Router::<App>::new().route(
        "/github-profile-views",
        get(handle_fetch_git_hub_profile_views),
    )
}

async fn handle_fetch_git_hub_profile_views(
    State(ctx): State<App>,
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

    let cache = &ctx.counters_ttl_cache;

    let mut count_result: Result<i64, diesel::result::Error>;

    // Since we put the badge on GitHub readme, we only allow the badge to be fetched from GitHub
    // and not from direct API calls.
    let user_agent_from_github = headers
        .get(USER_AGENT)
        .map(|ua| ua.to_str().unwrap_or_default().contains("github-camo"))
        .unwrap_or(false);

    // NOTE: that currently the badge is behind GitHub's proxy since it's hosted on GitHub markdown
    // renderer, so the IP address will always be GitHub's IP address. It means we're assuming that
    // there should not be more than one person viewing the badge within a second.
    if cache.get(&_ip).await.is_none() && user_agent_from_github {
        cache
            .insert(_ip.clone(), true, Duration::from_secs(1))
            .await;

        use crate::schema::counters;
        let mut conn = ctx.diesel.get().await.unwrap();

        count_result = diesel::update(counters::table)
            .filter(counters::key.eq("github-profile-views"))
            .filter(counters::name.eq("wonrax"))
            .set(counters::count.eq(counters::count + 1))
            .returning(counters::count)
            .get_result(&mut conn)
            .await;
    } else {
        use crate::schema::counters;
        let mut conn = ctx.diesel.get().await.unwrap();

        count_result = counters::table
            .filter(counters::key.eq("github-profile-views"))
            .filter(counters::name.eq("wonrax"))
            .select(counters::count)
            .first(&mut conn)
            .await;
    }

    if count_result.is_err() {
        use crate::schema::counters;
        let mut conn = ctx.diesel.get().await.unwrap();

        let _ = diesel::insert_into(counters::table)
            .values((
                counters::key.eq("github-profile-views"),
                counters::name.eq("wonrax"),
                counters::count.eq(1i64),
            ))
            .execute(&mut conn)
            .await;

        count_result = Ok(1);
    }

    match count_result {
        Ok(count) => {
            let readable_views = readable_uint(count.to_string());
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
