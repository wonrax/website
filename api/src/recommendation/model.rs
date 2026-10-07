//! The taste model: a weighted logistic regression over sparse article
//! features, retrained from scratch whenever the owner's history or feedback
//! changes. Training data is small enough (hundreds to a few thousand
//! articles) that full-batch gradient descent finishes in well under a second
//! and keeps the result deterministic.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::SourceInfo;

const TRAINING_ITERATIONS: usize = 300;
const LEARNING_RATE: f64 = 1.0;
const L2_PENALTY: f64 = 1e-3;

pub type Features = Vec<(String, f64)>;

pub struct Example {
    pub features: Features,
    pub label: bool,
    pub weight: f64,
}

#[derive(Debug, Default)]
pub struct TasteModel {
    bias: f64,
    weights: HashMap<String, f64>,
    example_count: usize,
}

impl TasteModel {
    pub fn train(examples: &[Example]) -> Self {
        let total_weight = examples.iter().map(|example| example.weight).sum::<f64>();
        if examples.is_empty() || total_weight <= 0.0 {
            return Self::default();
        }

        let mut index: HashMap<&str, usize> = HashMap::new();
        let rows = examples
            .iter()
            .map(|example| {
                example
                    .features
                    .iter()
                    .map(|(name, value)| {
                        let next = index.len();
                        (*index.entry(name.as_str()).or_insert(next), *value)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let mut weights = vec![0.0; index.len()];
        let mut gradient = vec![0.0; index.len()];
        let mut bias = 0.0;

        for _ in 0..TRAINING_ITERATIONS {
            gradient.iter_mut().for_each(|value| *value = 0.0);
            let mut bias_gradient = 0.0;

            for (example, row) in examples.iter().zip(&rows) {
                let logit = bias
                    + row
                        .iter()
                        .map(|(feature, value)| {
                            weights.get(*feature).copied().unwrap_or(0.0) * value
                        })
                        .sum::<f64>();
                let target = if example.label { 1.0 } else { 0.0 };
                let error = (sigmoid(logit) - target) * example.weight / total_weight;

                bias_gradient += error;
                for (feature, value) in row {
                    if let Some(slot) = gradient.get_mut(*feature) {
                        *slot += error * value;
                    }
                }
            }

            bias -= LEARNING_RATE * bias_gradient;
            for (weight, gradient) in weights.iter_mut().zip(&gradient) {
                *weight -= LEARNING_RATE * (gradient + L2_PENALTY * *weight);
            }
        }

        let weights = index
            .into_iter()
            .filter_map(|(name, feature)| {
                let weight = weights.get(feature).copied()?;
                (weight != 0.0).then(|| (name.to_string(), weight))
            })
            .collect();

        Self {
            bias,
            weights,
            example_count: examples.len(),
        }
    }

    /// Probability that the owner wants to read an article with these features.
    /// `None` when the model has never seen a training example.
    pub fn predict(&self, features: &[(String, f64)]) -> Option<f64> {
        if self.example_count == 0 {
            return None;
        }

        let logit = self.bias
            + features
                .iter()
                .filter_map(|(name, value)| self.weights.get(name).map(|weight| weight * value))
                .sum::<f64>();

        Some(sigmoid(logit))
    }
}

fn sigmoid(logit: f64) -> f64 {
    1.0 / (1.0 + (-logit).exp())
}

/// Features every article has, wherever it came from: its domain and the terms
/// of its title and body. Raindrop history and the background sample only get
/// these, so the model can't learn that HN or Lobsters metadata means "not a
/// bookmark" just because bookmarks rarely carry any.
pub fn content_features(url: &str, terms: &HashSet<String>) -> Features {
    let mut features = Vec::new();

    if let Some(domain) = article_domain(url) {
        features.push((format!("domain:{domain}"), 1.0));
    }

    push_normalized(&mut features, "term", terms.iter().map(String::as_str));

    features
}

/// Content features plus what HN and Lobsters say about the submission.
pub fn feed_features(url: &str, terms: &HashSet<String>, sources: &[SourceInfo]) -> Features {
    let mut features = content_features(url, terms);

    let tags = sources
        .iter()
        .flat_map(|source| source.tags.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    push_normalized(&mut features, "tag", tags.into_iter());

    for source in sources {
        features.push((format!("source:{}", source.key), 1.0));
        if let Some(submitter) = &source.submitter {
            features.push((format!("submitter:{}:{submitter}", source.key), 1.0));
        }
    }

    features
}

/// Gives a group of features a combined L2 norm of 1 so an article with 48
/// terms doesn't outvote its domain and tags just by being long.
fn push_normalized<'a>(
    features: &mut Features,
    prefix: &str,
    values: impl ExactSizeIterator<Item = &'a str>,
) {
    if values.len() == 0 {
        return;
    }

    let value = 1.0 / (values.len() as f64).sqrt();
    features.extend(values.map(|name| (format!("{prefix}:{name}"), value)));
}

pub fn article_domain(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    Some(
        host.strip_prefix("www.")
            .map(str::to_string)
            .unwrap_or(host),
    )
}

pub fn article_terms(title: &str, stored_terms: Option<&serde_json::Value>) -> HashSet<String> {
    let stored_terms = match stored_terms {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|term| !term.is_empty() && !crate::utils::is_recommender_noise(term))
            .map(str::to_string)
            .collect::<HashSet<_>>(),
        _ => HashSet::new(),
    };

    if stored_terms.is_empty() {
        crate::utils::extract_recommender_terms(title, None)
            .into_iter()
            .collect()
    } else {
        stored_terms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(values: &[&str]) -> HashSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn example(url: &str, values: &[&str], label: bool) -> Example {
        Example {
            features: content_features(url, &terms(values)),
            label,
            weight: 1.0,
        }
    }

    #[test]
    fn untrained_model_has_no_opinion() {
        let model = TasteModel::train(&[]);
        assert_eq!(
            model.predict(&content_features("https://a.dev/x", &terms(&["rust"]))),
            None
        );
    }

    #[test]
    fn learns_preferred_terms_and_domains() {
        let mut examples = Vec::new();
        for _ in 0..20 {
            examples.push(example(
                "https://blog.rust-lang.org/a",
                &["rust", "compiler"],
                true,
            ));
            examples.push(example(
                "https://crypto.news/a",
                &["crypto", "token"],
                false,
            ));
            examples.push(example(
                "https://example.com/a",
                &["database", "index"],
                false,
            ));
        }
        examples.push(example("https://example.com/b", &["rust", "async"], true));

        let model = TasteModel::train(&examples);
        let liked = model
            .predict(&content_features(
                "https://example.com/c",
                &terms(&["rust", "borrow"]),
            ))
            .unwrap_or_default();
        let disliked = model
            .predict(&content_features(
                "https://crypto.news/b",
                &terms(&["token", "airdrop"]),
            ))
            .unwrap_or_default();

        assert!(liked > 0.5, "liked = {liked}");
        assert!(disliked < 0.5, "disliked = {disliked}");
    }

    #[test]
    fn heavier_examples_win_ties() {
        let mut light = example("https://a.dev/x", &["rust"], true);
        light.weight = 0.2;
        let heavy = example("https://a.dev/y", &["rust"], false);

        let model = TasteModel::train(&[light, heavy]);
        let prediction = model
            .predict(&content_features("https://a.dev/z", &terms(&["rust"])))
            .unwrap_or_default();

        assert!(prediction < 0.5, "prediction = {prediction}");
    }

    #[test]
    fn feed_features_include_source_metadata() {
        let sources = vec![
            SourceInfo {
                key: "lobsters".to_string(),
                tags: vec!["rust".to_string(), "plt".to_string()],
                submitter: Some("alice".to_string()),
                ..Default::default()
            },
            SourceInfo {
                key: "hacker-news".to_string(),
                tags: vec!["rust".to_string()],
                ..Default::default()
            },
        ];

        let features = feed_features("https://www.Example.com/post", &terms(&["async"]), &sources);
        let names = features
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<HashSet<_>>();

        assert!(names.contains("domain:example.com"));
        assert!(names.contains("term:async"));
        assert!(names.contains("tag:rust"));
        assert!(names.contains("tag:plt"));
        assert!(names.contains("source:lobsters"));
        assert!(names.contains("source:hacker-news"));
        assert!(names.contains("submitter:lobsters:alice"));
        assert_eq!(
            features
                .iter()
                .filter(|(name, _)| name.starts_with("tag:"))
                .count(),
            2
        );
    }

    #[test]
    fn drops_noise_from_stored_terms() {
        let terms = article_terms("Title", Some(&serde_json::json!(["thi", "http", "rust"])));
        assert_eq!(terms, HashSet::from(["rust".to_string()]));
    }

    #[test]
    fn falls_back_to_title_terms() {
        let terms = article_terms("Writing a Compiler in Rust", Some(&serde_json::json!([])));
        assert!(terms.contains("compiler"));
        assert!(terms.contains("rust"));
    }
}
