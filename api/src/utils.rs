use std::sync::{Arc, LazyLock, Mutex};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::config::FASTEMBED_CACHE_DIR;

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
