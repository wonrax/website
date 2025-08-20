pub mod routes;

use ipnetwork::IpNetwork;
use tokio::sync::OnceCell;

static GITHUB_PREFIXES: OnceCell<Vec<IpNetwork>> = OnceCell::const_new();

async fn fetch_github_meta() -> Vec<IpNetwork> {
    let url = "https://api.github.com/meta";
    let client = reqwest::Client::new();
    let resp = match client
        .get(url)
        .header(reqwest::header::USER_AGENT, "wrx.sh-api/1.0")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to fetch GitHub meta");
            return Vec::new();
        }
    };

    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ct.starts_with("application/json") {
        tracing::warn!(content_type = %ct, "Unexpected content type from GitHub meta");
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to parse GitHub meta JSON");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for key in ["web", "api", "hooks", "actions"].iter() {
        if let Some(arr) = json.get(key).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if let Ok(n) = s.parse::<IpNetwork>() {
                        out.push(n);
                    } else {
                        tracing::warn!(entry = %s, key = %key, "Bad CIDR from GitHub meta");
                    }
                }
            }
        }
    }
    out
}

pub async fn github_prefixes() -> &'static Vec<IpNetwork> {
    GITHUB_PREFIXES
        .get_or_init(|| async { fetch_github_meta().await })
        .await
}

pub async fn is_github_ip(ip: &std::net::IpAddr) -> bool {
    github_prefixes().await.iter().any(|n| n.contains(*ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn github_meta_fetches_and_parses() {
        let prefixes = github_prefixes().await;
        assert!(!prefixes.is_empty());
        // Sanity: each prefix has a sensible mask length
        assert!(prefixes.iter().all(|p| match p {
            IpNetwork::V4(v4) => v4.prefix() <= 32,
            IpNetwork::V6(v6) => v6.prefix() <= 128,
        }));
    }

    #[tokio::test]
    async fn is_github_ip_reports_true_for_prefix_base() {
        let prefixes = github_prefixes().await;
        if let Some(p) = prefixes.first() {
            assert!(is_github_ip(&p.ip()).await);
        }
    }
}
