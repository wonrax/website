use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use pgvector::Vector;
use std::sync::{Arc, LazyLock, Mutex};
use text_splitter::MarkdownSplitter;

use crate::config::FASTEMBED_CACHE_DIR;

static EMBEDDING_MODEL: LazyLock<Arc<Mutex<TextEmbedding>>> = LazyLock::new(|| {
    embedding_model().expect("failed to initialize recommendation embedding model")
});

pub async fn generate_embeddings(title: &str, markdown: &str) -> Result<Vec<Vector>, eyre::Error> {
    // AllMiniLML12V2 truncates input text longer than 256 tokens
    let splitter = MarkdownSplitter::new(512..768);
    let chunks: Vec<String> = if markdown.trim().is_empty() {
        vec![format!("Title: {title}")]
    } else {
        splitter
            .chunks(markdown)
            .map(|chunk| format!("Title: {title}\n{chunk}"))
            .take(64) // Limit to 64 chunks
            .collect()
    };

    if chunks.is_empty() {
        return Ok(Vec::new());
    }

    tokio::task::spawn_blocking(move || {
        let mut model = EMBEDDING_MODEL
            .lock()
            .map_err(|_| eyre::eyre!("embedding model lock poisoned"))?;
        let embeddings = model.embed(chunks, None).map_err(|err| eyre::eyre!(err))?;
        let vectors = embeddings.into_iter().map(Vector::from).collect();
        Ok::<_, eyre::Error>(vectors)
    })
    .await
    .map_err(|err| eyre::eyre!(err))?
}

fn embedding_model() -> Result<Arc<Mutex<TextEmbedding>>, eyre::Error> {
    tracing::info!("Initializing recommendation FastEmbed model");
    let model = TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::AllMiniLML12V2).with_cache_dir(
            FASTEMBED_CACHE_DIR
                .parse()
                .map_err(|err| eyre::eyre!("invalid fastembed cache dir: {err}"))?,
        ),
    )
    .map_err(|err| eyre::eyre!(err))?;
    let model = Arc::new(Mutex::new(model));
    Ok(model)
}
