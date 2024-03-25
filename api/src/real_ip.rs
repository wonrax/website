// Borrow a lot of code from crates.io
// https://github.com/rust-lang/crates.io/blob/986d296f910c2ed821be907b1e32a120c03338cb/src/real_ip.rs

use axum::{
    extract::ConnectInfo,
    http::{request::Parts, HeaderMap},
};
use ipnetwork::IpNetwork;
use std::{
    net::{IpAddr, SocketAddr},
    sync::OnceLock,
};

use crate::{error::Error, APIContext};

static CLOUDFRONT_PREFIXES: OnceLock<Vec<IpNetwork>> = OnceLock::new();

fn get_cloudfront_prefixes<'a>() -> &'a Vec<IpNetwork> {
    CLOUDFRONT_PREFIXES.get_or_init(|| {
        let ipv4_prefixes = aws_ip_ranges::IP_RANGES
            .prefixes
            .iter()
            .filter(|prefix| prefix.service == "CLOUDFRONT")
            .map(|prefix| prefix.ip_prefix);

        let ipv6_prefixes = aws_ip_ranges::IP_RANGES
            .ipv6_prefixes
            .iter()
            .filter(|prefix| prefix.service == "CLOUDFRONT")
            .map(|prefix| prefix.ipv6_prefix);

        ipv4_prefixes
            .chain(ipv6_prefixes)
            .filter_map(|prefix| match prefix.parse() {
                Ok(ip_network) => Some(ip_network),
                Err(error) => {
                    tracing::warn!(%error, "Failed to parse AWS CloudFront CIDR");
                    None
                }
            })
            .collect()
    })
}

pub fn is_cloudfront_ip(ip: &IpAddr) -> bool {
    CLOUDFRONT_PREFIXES
        .get()
        .unwrap_or_else(|| get_cloudfront_prefixes())
        .iter()
        .any(|trusted_proxy| trusted_proxy.contains(*ip))
}

/// Get the originating client IP address from the headers, which is the left-most
/// non-private IP address in the X-Forwarded-For header.
pub fn get_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(','))
        .filter_map(|ip| ip.trim().parse().ok())
        .next()
}

pub struct ClientIp(pub IpAddr);

#[axum::async_trait]
impl axum::extract::FromRequestParts<APIContext> for ClientIp {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &APIContext,
    ) -> Result<Self, Self::Rejection> {
        let socket_ip: IpAddr = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or("couldn't get connecting socket IP")?
            .0
            .ip();

        let client_ip = get_client_ip(&parts.headers);

        let client_ip = if is_cloudfront_ip(&socket_ip) {
            client_ip.unwrap_or_else(|| {
                tracing::warn!(
                    ?socket_ip,
                    "Request from CloudFront, but failed to get client IP from headers, using socket IP"
                );
                socket_ip
            })
        } else {
            let untrusted_client_ip = client_ip.unwrap_or(socket_ip);
            match socket_ip {
                IpAddr::V4(ip) => {
                    // Do not warn if the connecting socket is private IP (e.g. 127.0.0.1)
                    if !ip.is_private() && !ip.is_loopback() {
                        tracing::warn!(
                            ?socket_ip,
                            ?untrusted_client_ip,
                            "Request from non-CloudFront proxy"
                        );
                    }
                }
                IpAddr::V6(_) => {
                    // TODO wanted to check, but is_unique_local() is not stable
                    // https://doc.rust-lang.org/nightly/std/net/struct.Ipv6Addr.html#method.is_unique_local
                    tracing::warn!(
                        ?socket_ip,
                        ?untrusted_client_ip,
                        "Request from non-CloudFront proxy"
                    );
                }
            };

            untrusted_client_ip
        };

        Ok(ClientIp(client_ip))
    }
}
