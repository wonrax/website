use pgvector::Vector;
use text_splitter::MarkdownSplitter;

use crate::utils::embed_texts;

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
        let embeddings = embed_texts(chunks).map_err(|err| eyre::eyre!(err))?;
        let vectors = embeddings.into_iter().map(Vector::from).collect();
        Ok::<_, eyre::Error>(vectors)
    })
    .await
    .map_err(|err| eyre::eyre!(err))?
}
