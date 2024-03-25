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
        let mut x_forwarded_for_ips = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|header| header.to_str().ok())
            .flat_map(|header| header.split(','))
            .filter_map(|ip| ip.trim().parse::<IpAddr>().ok())
            .filter(|ip| match ip {
                IpAddr::V4(ip) => !ip.is_private() && !ip.is_loopback(),
                IpAddr::V6(_) => true,
            });

        // Get the originating client IP address from the headers, which is the
        // left-most non-private IP address in the X-Forwarded-For header.
        let client_ip = x_forwarded_for_ips.next();

        // Get the CloudFront IP address from the headers, which is the right-most
        // IP address that was appended by the Caddy reverse proxy
        let supposedly_cloudfront_ip = x_forwarded_for_ips.next_back();

        Ok(ClientIp(match (client_ip, supposedly_cloudfront_ip) {
            (Some(client_ip), Some(cf_ip)) if is_cloudfront_ip(&cf_ip) => client_ip,
            (Some(client_ip), cf_ip) => {
                tracing::warn!(
                    ?client_ip,
                    ?cf_ip,
                    "Request from non-CloudFront proxy, using the untrusted client IP"
                );
                client_ip
            }
            (None, _) => {
                let socket_ip: IpAddr = parts
                    .extensions
                    .get::<ConnectInfo<SocketAddr>>()
                    .ok_or("couldn't get connecting socket IP")?
                    .0
                    .ip();

                tracing::warn!(
                    ?socket_ip,
                    "No client IP found in X-Forwarded-For headers, using socket IP"
                );
                socket_ip
            }
        }))
    }
}
