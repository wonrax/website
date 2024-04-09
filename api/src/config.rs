use std::collections::VecDeque;

#[derive(Clone)]
pub enum Env {
    Dev,
    Staging,
    Production,
}

pub struct ServerConfig {
    pub env: Env,
    pub github_oauth: Option<GitHubOauth>,
    pub spotify_oauth: Option<SpotifyOauth>,
}

struct GitHubOauth {
    client_id: String,
    client_secret: String,
}

struct SpotifyOauth {
    client_id: String,
    client_secret: String,
}

fn var(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(env) => Ok(Some(env)),
        Err(e) => {
            tracing::warn!("Mising environment variable `key`");
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

/// Either all or none variables are set
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
                tracing::error!(
                    "Environment variable `{k}` is required if variables {keys:?} are present"
                );
                None
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

        ServerConfig {
            env: match var("ENVIRONMENT") {
                Ok(Some(env)) => match env.as_str() {
                    "dev" => Env::Dev,
                    "staging" => Env::Staging,
                    "production" => Env::Production,
                    _ => Env::Dev,
                },
                _ => Env::Dev,
            },
            github_oauth,
            spotify_oauth,
        }
    }
}
