use chromadb::client::ChromaClientOptions;
use chromadb::collection::{CollectionEntries, QueryOptions};
use chromadb::{ChromaClient, ChromaCollection};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

use crate::config::VectorDbConfig;

/// Type alias for the shared vector client wrapped in Arc for easy sharing across threads
#[derive(Clone)]
pub struct SharedVectorClient(Arc<VectorClient>);

impl std::ops::Deref for SharedVectorClient {
    type Target = VectorClient;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SharedVectorClient {
    /// Create a new shared vector client wrapped in Arc for easy sharing across threads
    pub async fn new(config: VectorDbConfig) -> Result<SharedVectorClient, VectorClientError> {
        let client = VectorClient::new(config).await?;
        Ok(SharedVectorClient(Arc::new(client)))
    }
}

#[derive(Debug, Error)]
#[error("Vector client error: {0}")]
pub struct VectorClientError(String);

/// Shared vector database client with common functionality
pub struct VectorClient {
    client: ChromaClient,
    embedding_model: TextEmbedding,
    config: VectorDbConfig,
}

impl VectorClient {
    /// Create a new vector client with configuration
    pub async fn new(config: VectorDbConfig) -> Result<Self, VectorClientError> {
        let client_options = ChromaClientOptions {
            url: Some(config.url.clone()),
            database: config.database.clone(),
            auth: chromadb::client::ChromaAuthMethod::TokenAuth {
                token: config.token.clone(),
                header: chromadb::client::ChromaTokenHeader::XChromaToken,
            },
        };

        let client = ChromaClient::new(client_options)
            .await
            .map_err(|e| VectorClientError(format!("Failed to create ChromaDB client: {}", e)))?;

        // Initialize embedding model with better error handling
        tracing::info!("Initializing FastEmbed model (this may download files on first run)...");
        let embedding_model = TextEmbedding::try_new(InitOptions::new(
            EmbeddingModel::AllMiniLML12V2,
        ))
        .map_err(|e| VectorClientError(format!("Failed to initialize embedding model: {}", e)))?;

        Ok(Self {
            client,
            embedding_model,
            config,
        })
    }

    /// Get the collection name to use, incorporating channel ID if available
    pub fn get_collection_name(&self, channel_id: u64) -> String {
        match &self.config.default_collection {
            Some(col) => format!("{}_channel_{}", col, channel_id),
            None => format!("discord_memory_channel_{}", channel_id),
        }
    }

    /// Get or create a collection
    async fn get_or_create_collection(
        &self,
        collection_name: &str,
    ) -> Result<ChromaCollection, VectorClientError> {
        self.client
            .get_or_create_collection(collection_name, None)
            .await
            .map_err(|e| {
                VectorClientError(format!(
                    "Failed to get or create collection {}: {}",
                    collection_name, e
                ))
            })
    }

    /// Store information in the vector database
    pub async fn store(
        &self,
        information: &str,
        channel_id: u64,
        metadata: Option<Value>,
    ) -> Result<String, VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);
        let collection = self.get_or_create_collection(&collection_name).await?;

        // Generate embeddings
        let embeddings = self
            .embedding_model
            .embed(vec![information], None)
            .map_err(|e| VectorClientError(format!("Failed to generate embeddings: {}", e)))?;

        let embedding = embeddings
            .first()
            .ok_or_else(|| VectorClientError("No embeddings generated".to_string()))?;

        // Create point
        let point_id = Uuid::new_v4().to_string();

        // Prepare metadata
        let final_metadata = match metadata {
            Some(Value::Object(obj)) => obj,
            Some(_) => serde_json::Map::new(),
            None => serde_json::Map::new(),
        };

        let collection_entries = CollectionEntries {
            ids: vec![&point_id],
            embeddings: Some(vec![embedding.clone()]),
            metadatas: Some(vec![final_metadata]),
            documents: Some(vec![information]),
        };

        // Store the point
        collection
            .upsert(collection_entries, None)
            .await
            .map_err(|e| VectorClientError(format!("Failed to store point: {}", e)))?;

        Ok(point_id)
    }

    /// Update existing information in the vector database by point ID
    pub async fn update(
        &self,
        point_id: &str,
        information: &str,
        channel_id: u64,
        metadata: Option<Value>,
    ) -> Result<(), VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);
        let collection = self.get_or_create_collection(&collection_name).await?;

        // Generate embeddings for the new content
        let embeddings = self
            .embedding_model
            .embed(vec![information], None)
            .map_err(|e| VectorClientError(format!("Failed to generate embeddings: {}", e)))?;

        let embedding = embeddings
            .first()
            .ok_or_else(|| VectorClientError("No embeddings generated".to_string()))?;

        // Prepare metadata
        let final_metadata = match metadata {
            Some(Value::Object(obj)) => obj,
            Some(_) => serde_json::Map::new(),
            None => serde_json::Map::new(),
        };

        let collection_entries = CollectionEntries {
            ids: vec![point_id],
            embeddings: Some(vec![embedding.clone()]),
            metadatas: Some(vec![final_metadata]),
            documents: Some(vec![information]),
        };

        // Update the point using upsert (this will replace the existing point)
        collection
            .upsert(collection_entries, None)
            .await
            .map_err(|e| VectorClientError(format!("Failed to update point: {}", e)))?;

        Ok(())
    }

    /// Search for information in the vector database
    pub async fn search(
        &self,
        query: &str,
        channel_id: u64,
        limit: u64,
    ) -> Result<Vec<SearchResult>, VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);

        // Try to get the collection, return empty results if it doesn't exist
        let collection = match self.client.get_collection(&collection_name).await {
            Ok(collection) => collection,
            Err(_) => {
                return Ok(vec![]);
            }
        };

        let embeddings = self.embedding_model.embed(vec![query], None).map_err(|e| {
            VectorClientError(format!("Failed to generate query embeddings: {}", e))
        })?;

        let query_embedding = embeddings
            .first()
            .ok_or_else(|| VectorClientError("No query embeddings generated".to_string()))?;

        // Search for similar points using ChromaDB query
        let query_options = QueryOptions {
            query_texts: None,
            query_embeddings: Some(vec![query_embedding.clone()]),
            where_metadata: None,
            where_document: None,
            n_results: Some(limit as usize),
            include: Some(vec!["documents", "metadatas", "distances"]),
        };

        let mut query_result = collection
            .query(query_options, None)
            .await
            .map_err(|e| VectorClientError(format!("Failed to search points: {}", e)))?;

        let mut results = Vec::new();

        // Process query results
        if let (Some(ids), Some(documents), Some(distances), metadatas) = (
            query_result.ids.into_iter().next(),
            query_result
                .documents
                .take()
                .and_then(|docs| docs.into_iter().next()),
            query_result
                .distances
                .take()
                .and_then(|dists| dists.into_iter().next()),
            query_result
                .metadatas
                .take()
                .and_then(|metas| metas.into_iter().next()),
        ) {
            for (id, (doc, (distance, meta))) in ids.into_iter().zip(
                documents.into_iter().zip(
                    distances.into_iter().zip(
                        metadatas
                            .map(|m| {
                                Box::new(m.into_iter())
                                    as Box<dyn Iterator<Item = Option<serde_json::Map<_, _>>>>
                            })
                            .unwrap_or_else(|| Box::new(std::iter::repeat(None))),
                    ),
                ),
            ) {
                // Convert distance to similarity score (ChromaDB returns distances, we want similarity)
                let score = 1.0 - distance.clamp(0.0, 1.0); // Convert distance to similarity score

                let timestamp = meta
                    .as_ref()
                    .and_then(|m| m.get("timestamp"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());

                results.push(SearchResult {
                    point_id: id.clone(),
                    content: doc,
                    score,
                    metadata: meta.map(serde_json::Value::Object),
                    timestamp,
                });
            }
        }

        Ok(results)
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub point_id: String,
    pub content: String,
    pub score: f32,
    pub metadata: Option<Value>,
    pub timestamp: Option<String>,
}
