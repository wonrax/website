use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::config::FASTEMBED_CACHE_DIR;

pub const RECOMMENDER_EMBEDDING_BITS: usize = 384;
pub const MAX_RECOMMENDER_TERMS: usize = 48;
pub const MAX_RECOMMENDER_CONTENT_CHARS: usize = 24_000;

static RECOMMENDER_STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "about", "after", "again", "against", "all", "also", "and", "any", "are", "around",
        "because", "been", "before", "being", "between", "both", "but", "can", "could", "does",
        "doing", "done", "down", "during", "each", "even", "every", "for", "from", "had", "has",
        "have", "having", "here", "how", "into", "its", "just", "like", "many", "more", "most",
        "much", "not", "now", "off", "onto", "other", "our", "out", "over", "really", "should",
        "since", "some", "such", "than", "that", "the", "their", "them", "then", "there", "these",
        "they", "this", "those", "through", "under", "until", "very", "was", "were", "what",
        "when", "where", "which", "while", "who", "will", "with", "would", "your",
    ])
});

/// Replace placeholder in template with data.
pub fn render_template(template: &str, data: &[(&str, &str)]) -> String {
    let mut result = String::from(template);

    for (placeholder, value) in data {
        result = result.replace(placeholder, value);
    }

    result
}

/// Convert uint to readable format. Example: `12345 -> 12,345`.
pub fn readable_uint(int_str: String) -> String {
    let mut s = String::new();
    for (i, char) in int_str.chars().rev().enumerate() {
        if i % 3 == 0 && i != 0 {
            s.insert(0, ',');
        }
        s.insert(0, char);
    }
    s
}

fn normalize_recommender_term(token: &str) -> Option<String> {
    if token.len() < 3 || token.chars().all(|char| char.is_ascii_digit()) {
        return None;
    }

    let mut normalized = token.to_ascii_lowercase();

    if normalized.ends_with("ies") && normalized.len() > 4 {
        normalized.truncate(normalized.len() - 3);
        normalized.push('y');
    } else if normalized.ends_with("ing") && normalized.len() > 5 {
        normalized.truncate(normalized.len() - 3);
    } else if normalized.ends_with("ed") && normalized.len() > 4 {
        normalized.truncate(normalized.len() - 2);
    } else if normalized.ends_with('s') && normalized.len() > 3 && !normalized.ends_with("ss") {
        normalized.truncate(normalized.len() - 1);
    }

    (!RECOMMENDER_STOPWORDS.contains(normalized.as_str()) && normalized.len() >= 3)
        .then_some(normalized)
}

fn score_recommender_terms(text: &str, weight: f64, scores: &mut HashMap<String, f64>) {
    for token in text.split(|char: char| !char.is_ascii_alphanumeric()) {
        let Some(term) = normalize_recommender_term(token) else {
            continue;
        };

        let length_bonus = ((term.len().saturating_sub(4)) as f64).min(6.0) * 0.08;
        *scores.entry(term).or_insert(0.0) += weight + length_bonus;
    }
}

pub fn extract_recommender_terms(title: &str, content: Option<&str>) -> Vec<String> {
    let mut scores = HashMap::new();
    score_recommender_terms(title, 4.0, &mut scores);

    if let Some(content) = content {
        score_recommender_terms(content, 1.0, &mut scores);
    }

    let mut ranked_terms = scores.into_iter().collect::<Vec<_>>();
    ranked_terms.sort_by(|(left_term, left_score), (right_term, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| right_term.len().cmp(&left_term.len()))
            .then_with(|| left_term.cmp(right_term))
    });

    ranked_terms
        .into_iter()
        .take(MAX_RECOMMENDER_TERMS)
        .map(|(term, _)| term)
        .collect()
}

pub fn truncate_recommender_content(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(
        trimmed
            .chars()
            .take(MAX_RECOMMENDER_CONTENT_CHARS)
            .collect(),
    )
}

/// Shared FastEmbed embedding model instance.
/// This is lazily initialized on first use and shared across the application
/// to reduce memory usage by avoiding multiple model instances.
static SHARED_EMBEDDING_MODEL: LazyLock<Arc<Mutex<TextEmbedding>>> = LazyLock::new(|| {
    tracing::info!("Initializing shared FastEmbed embedding model");
    let model = TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::AllMiniLML12V2).with_cache_dir(
            FASTEMBED_CACHE_DIR
                .parse()
                .expect("invalid fastembed cache dir"),
        ),
    )
    .expect("failed to initialize embedding model");
    Arc::new(Mutex::new(model))
});

/// Error type for embedding operations
#[derive(Debug, thiserror::Error)]
#[error("Embedding error: {0}")]
pub struct EmbeddingError(String);

/// Generate embeddings for a list of texts using the shared model.
pub fn embed_texts(texts: Vec<String>) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    let mut model = SHARED_EMBEDDING_MODEL
        .lock()
        .map_err(|_| EmbeddingError("embedding model lock poisoned".to_string()))?;
    model
        .embed(texts, None)
        .map_err(|e| EmbeddingError(format!("failed to generate embeddings: {e}")))
}
