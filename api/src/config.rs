#[derive(Clone)]
pub enum Env {
    Dev,
    Staging,
    Production,
}

pub struct ServerConfig {
    pub env: Env,

    /// Website URL (i.e. frontend) in full form without trailing slash
    /// e.g. https://hhai.dev
    pub site_url: String,

    pub github_oauth: Option<GitHubOauth>,
    pub spotify_oauth: Option<SpotifyOauth>,

    // My ID in the identities table
    pub owner_identity_id: i32,
}

pub struct GitHubOauth {
    pub client_id: String,
    pub client_secret: String,
}

pub struct SpotifyOauth {
    pub client_id: String,
    pub client_secret: String,
}

fn var(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(env) => Ok(Some(env)),
        Err(e) => {
            tracing::warn!("Mising environment variable `{key}`");
            match e {
                std::env::VarError::NotPresent => Ok(None),
                std::env::VarError::NotUnicode(_) => Err(format!(
                    "Could not get the environment variable `{key}` due to unicode error"
                )),
            }
        }
    }
}

fn required_var(key: &str) -> String {
    let val = var(key);
    match val {
        Ok(val) => match val {
            Some(val) => val,
            None => {
                tracing::error!("Environment variable `{key}` is required");
                std::process::exit(1)
            }
        },
        Err(e) => {
            tracing::error!(
                "Environment variable `{key}` is required, but could not retrieve: {e}"
            );
            std::process::exit(1)
        }
    }
}

/// Either all or none variables are set, otherwise panics
fn all_or_none_vars(keys: Vec<&str>) -> Option<Vec<String>> {
    keys.iter().fold(None, |accum, k| match var(k) {
        Ok(Some(val)) => match accum {
            Some(mut l) => {
                l.push(val);
                Some(l)
            }
            None => Some(vec![val]),
        },
        _ => match accum {
            Some(_) => {
                let mut rest = keys.clone();
                rest.retain(|_k| _k != k);
                tracing::error!(
                    "Environment variable `{k}` is required if variables {rest:?} are present"
                );
                std::process::exit(1);
            }
            None => None,
        },
    })
}

impl ServerConfig {
    pub fn new_from_env() -> Self {
        let github_oauth =
            all_or_none_vars(vec!["GITHUB_OAUTH_CLIENT_ID", "GITHUB_OAUTH_CLIENT_SECRET"]).map(
                |mut vars| GitHubOauth {
                    client_id: vars.remove(0),
                    client_secret: vars.remove(0),
                },
            );

        let spotify_oauth = all_or_none_vars(vec![
            "SPOTIFY_OAUTH_CLIENT_ID",
            "SPOTIFY_OAUTH_CLIENT_SECRET",
        ])
        .map(|mut vars| SpotifyOauth {
            client_id: vars.remove(0),
            client_secret: vars.remove(0),
        });

        let env = match var("ENVIRONMENT") {
            Ok(Some(env)) => match env.as_str() {
                "dev" => Env::Dev,
                "staging" => Env::Staging,
                "production" => Env::Production,
                _ => Env::Dev,
            },
            _ => Env::Dev,
        };

        let site_url = var("SITE_URL")
            .unwrap_or(Some("http://localhost:4321".to_string()))
            .unwrap_or("http://localhost:4321".to_string());

        ServerConfig {
            env,
            site_url,
            github_oauth,
            spotify_oauth,
            owner_identity_id: 1,
        }
    }
}
