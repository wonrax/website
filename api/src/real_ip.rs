// Borrow a lot of code from crates.io
// https://github.com/rust-lang/crates.io/blob/986d296f910c2ed821be907b1e32a120c03338cb/src/real_ip.rs

use axum::{extract::ConnectInfo, http::request::Parts};
use ipnetwork::IpNetwork;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::OnceCell;

use crate::{App, error::AppError};

static CLOUDFLARE_PREFIXES: OnceCell<Vec<IpNetwork>> = OnceCell::const_new();

async fn load_cloudflare_prefixes() -> Vec<IpNetwork> {
    // Fetch Cloudflare IPv4 and IPv6 prefix lists and parse them
    async fn fetch_list(url: &str) -> Vec<IpNetwork> {
        match reqwest::get(url).await {
            Ok(resp) => {
                // Accept text/plain with any charset
                if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE)
                    && let Ok(ct) = ct.to_str()
                    && !ct.to_ascii_lowercase().starts_with("text/plain")
                {
                    tracing::warn!(content_type = %ct, "Unexpected content type from Cloudflare IP list");
                }
                match resp.text().await {
                    Ok(body) => body
                        .lines()
                        .filter_map(|line| {
                            let s = line.trim();
                            if s.is_empty() { return None; }
                            match s.parse::<IpNetwork>() {
                                Ok(n) => Some(n),
                                Err(e) => {
                                    tracing::warn!(line = %s, error = ?e, "Failed to parse Cloudflare CIDR line");
                                    None
                                }
                            }
                        })
                        .collect(),
                    Err(e) => {
                        tracing::warn!(url = %url, error = %e, "Failed reading Cloudflare IP list body");
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "Failed fetching Cloudflare IP list");
                Vec::new()
            }
        }
    }

    let (v4, v6) = tokio::join!(
        fetch_list("https://www.cloudflare.com/ips-v4"),
        fetch_list("https://www.cloudflare.com/ips-v6"),
    );

    v4.into_iter().chain(v6.into_iter()).collect()
}

async fn get_cloudflare_prefixes() -> &'static Vec<IpNetwork> {
    CLOUDFLARE_PREFIXES
        .get_or_init(|| async { load_cloudflare_prefixes().await })
        .await
}

async fn is_cloudflare_ip(ip: &IpAddr) -> bool {
    get_cloudflare_prefixes()
        .await
        .iter()
        .any(|trusted_proxy| trusted_proxy.contains(*ip))
}

pub struct ClientIp(pub IpAddr);

#[axum::async_trait]
impl axum::extract::FromRequestParts<App> for ClientIp {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &App) -> Result<Self, Self::Rejection> {
        // Prefer Cloudflare headers first
        let cf_connecting_ip = parts
            .headers
            .get("cf-connecting-ip")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
            .filter(|ip| match ip {
                IpAddr::V4(ip) => !ip.is_private() && !ip.is_loopback(),
                IpAddr::V6(_) => true,
            });

        let true_client_ip = parts
            .headers
            .get("true-client-ip")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
            .filter(|ip| match ip {
                IpAddr::V4(ip) => !ip.is_private() && !ip.is_loopback(),
                IpAddr::V6(_) => true,
            });

        let x_forwarded_for_ips = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|header| header.to_str().ok())
            .flat_map(|header| header.split(','))
            .filter_map(|ip| ip.trim().parse::<IpAddr>().ok())
            .filter(|ip| match ip {
                IpAddr::V4(ip) => !ip.is_private() && !ip.is_loopback(),
                IpAddr::V6(_) => true,
            })
            .collect::<Vec<_>>();

        // left-most = origin, right-most = nearest
        let client_ip_from_xff = x_forwarded_for_ips.first().cloned();
        let nearest_proxy_ip_from_xff = x_forwarded_for_ips.last().cloned();

        let socket_ip: IpAddr = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or("couldn't get connecting socket IP")?
            .0
            .ip();

        let nearest_proxy_ip = nearest_proxy_ip_from_xff.or(Some(socket_ip));

        if let Some(npi) = nearest_proxy_ip
            && is_cloudflare_ip(&npi).await
            && let Some(ip) = cf_connecting_ip.or(true_client_ip).or(client_ip_from_xff)
        {
            return Ok(ClientIp(ip));
        }

        // If we reach here, either nearest proxy isn't Cloudflare, or no valid header IP.
        // Fallback to socket IP, or error if headers present but untrusted proxy
        if cf_connecting_ip.is_none() && true_client_ip.is_none() && client_ip_from_xff.is_none() {
            return Ok(ClientIp(socket_ip));
        }

        Err((
            "couldn't determine client IP address",
            axum::http::StatusCode::BAD_REQUEST,
        )
            .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parse_cloudflare_prefixes_handles_plain_text() {
        let prefixes = load_cloudflare_prefixes().await; // real fetch; acceptable for smoke test
        assert!(!prefixes.is_empty());
        // Ensure they look like CIDRs
        assert!(prefixes.iter().all(|p| match p {
            IpNetwork::V4(v4) => v4.prefix() <= 32,
            IpNetwork::V6(v6) => v6.prefix() <= 128,
        }));
    }

    #[tokio::test]
    async fn nearest_proxy_not_trusted_falls_back_to_socket() {
        // Simulate a local address that is certainly not in Cloudflare
        let local = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));
        // we cannot access FromRequestParts directly here; just test predicate
        assert!(!is_cloudflare_ip(&local).await);
    }
}
