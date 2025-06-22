use chrono;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use qdrant_client::{
    qdrant::{
        CreateCollectionBuilder, Distance, PointStruct, SearchPointsBuilder, UpsertPointsBuilder,
        Value as QdrantValue, VectorParams, VectorsConfig,
    },
    Qdrant,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Type alias for the shared Qdrant client wrapped in Arc for easy sharing across threads
pub type SharedQdrantClient = Arc<QdrantSharedClient>;

#[derive(Debug, Error)]
#[error("Qdrant client error: {0}")]
pub struct QdrantClientError(String);

/// Configuration for Qdrant client
#[derive(Clone)]
pub struct QdrantConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub default_collection: Option<String>,
    pub channel_id: Option<u64>, // Discord channel ID for collection naming
}

/// Shared Qdrant client with common functionality
pub struct QdrantSharedClient {
    client: Qdrant,
    embedding_model: TextEmbedding,
    pub config: QdrantConfig, // Make config public for access
}

impl QdrantSharedClient {
    /// Create a new Qdrant client with configuration
    pub async fn new(config: QdrantConfig) -> Result<Self, QdrantClientError> {
        let mut client_builder = Qdrant::from_url(&config.url);

        // Add API key if provided
        if let Some(api_key) = &config.api_key {
            client_builder = client_builder.api_key(api_key.clone());
        }

        let client = client_builder
            // if not the qdrant client will crash the server if compatibility check fails
            .skip_compatibility_check()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| QdrantClientError(format!("Failed to create Qdrant client: {}", e)))?;

        // Initialize embedding model with better error handling
        tracing::info!("Initializing FastEmbed model (this may download files on first run)...");
        let embedding_model = match tokio::task::spawn_blocking(|| {
            TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML12V2))
        })
        .await
        {
            Ok(Ok(model)) => model,
            Ok(Err(e)) => {
                return Err(QdrantClientError(format!(
                    "Failed to initialize embedding model: {}",
                    e
                )));
            }
            Err(e) => {
                return Err(QdrantClientError(format!(
                    "Embedding model initialization task failed: {}",
                    e
                )));
            }
        };

        // Test the connection and log any issues but don't fail
        match client.health_check().await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Qdrant health check failed at {} (this may be normal if server is starting or unavailable): {}",
                    config.url, e
                );
                tracing::info!(
                    "Bot will continue without memory features until Qdrant becomes available"
                );
            }
        }

        Ok(Self {
            client,
            embedding_model,
            config,
        })
    }

    /// Create a new shared Qdrant client wrapped in Arc for easy sharing across threads
    pub async fn new_shared(config: QdrantConfig) -> Result<SharedQdrantClient, QdrantClientError> {
        let client = Self::new(config).await?;
        Ok(Arc::new(client))
    }

    /// Get the collection name to use, incorporating channel ID if available
    pub fn get_collection_name(
        &self,
        collection_name: Option<&str>,
    ) -> Result<String, QdrantClientError> {
        let base_collection = collection_name
            .or(self.config.default_collection.as_deref())
            .ok_or_else(|| {
                QdrantClientError(
                    "No collection specified and no default collection set".to_string(),
                )
            })?;

        // If channel_id is provided, create a channel-specific collection
        if let Some(channel_id) = self.config.channel_id {
            Ok(format!("{}_channel_{}", base_collection, channel_id))
        } else {
            Ok(base_collection.to_string())
        }
    }

    /// Ensure collection exists, create if it doesn't
    pub async fn ensure_collection(&self, collection_name: &str) -> Result<(), QdrantClientError> {
        // Check if collection exists
        let collections = match self.client.list_collections().await {
            Ok(collections) => collections,
            Err(e) => {
                return Err(QdrantClientError(format!(
                    "Cannot access Qdrant server (server may be down or misconfigured): {}",
                    e
                )));
            }
        };

        let collection_exists = collections
            .collections
            .iter()
            .any(|collection| collection.name == collection_name);

        if !collection_exists {
            // Create collection with appropriate vector configuration
            let create_collection =
                CreateCollectionBuilder::new(collection_name).vectors_config(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::Params(
                        VectorParams {
                            size: 384, // all-MiniLM-L12-v2 produces 384-dimensional vectors
                            distance: Distance::Cosine.into(),
                            ..Default::default()
                        },
                    )),
                });

            self.client
                .create_collection(create_collection)
                .await
                .map_err(|e| {
                    QdrantClientError(format!(
                        "Failed to create collection {}: {}",
                        collection_name, e
                    ))
                })?;
        }

        Ok(())
    }

    /// Store information in Qdrant
    pub async fn store(
        &self,
        information: &str,
        collection_name: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<String, QdrantClientError> {
        let collection = self.get_collection_name(collection_name)?;

        // Ensure collection exists
        self.ensure_collection(&collection).await?;

        // Generate embeddings
        let embeddings = self
            .embedding_model
            .embed(vec![information], None)
            .map_err(|e| QdrantClientError(format!("Failed to generate embeddings: {}", e)))?;

        let embedding = embeddings
            .first()
            .ok_or_else(|| QdrantClientError("No embeddings generated".to_string()))?;

        // Create point
        let point_id = Uuid::new_v4().to_string();
        let mut payload = HashMap::new();

        // Add the original text
        payload.insert(
            "content".to_string(),
            QdrantValue::from(information.to_string()),
        );

        // Add metadata if provided
        if let Some(metadata) = metadata {
            payload.insert(
                "metadata".to_string(),
                convert_json_to_qdrant_value(metadata)?,
            );
        }

        // Add timestamp
        payload.insert(
            "timestamp".to_string(),
            QdrantValue::from(chrono::Utc::now().to_rfc3339()),
        );

        let point = PointStruct::new(point_id.clone(), embedding.clone(), payload);

        // Store the point
        let upsert_points = UpsertPointsBuilder::new(&collection, vec![point]);
        self.client
            .upsert_points(upsert_points)
            .await
            .map_err(|e| QdrantClientError(format!("Failed to store point: {}", e)))?;

        Ok(point_id)
    }

    /// Update existing information in Qdrant by point ID
    pub async fn update(
        &self,
        point_id: &str,
        information: &str,
        collection_name: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<(), QdrantClientError> {
        let collection = self.get_collection_name(collection_name)?;

        // Ensure collection exists
        self.ensure_collection(&collection).await?;

        // Generate embeddings for the new content
        let embeddings = self
            .embedding_model
            .embed(vec![information], None)
            .map_err(|e| QdrantClientError(format!("Failed to generate embeddings: {}", e)))?;

        let embedding = embeddings
            .first()
            .ok_or_else(|| QdrantClientError("No embeddings generated".to_string()))?;

        // Create updated payload
        let mut payload = HashMap::new();

        // Add the updated text
        payload.insert(
            "content".to_string(),
            QdrantValue::from(information.to_string()),
        );

        // Add metadata if provided
        if let Some(metadata) = metadata {
            payload.insert(
                "metadata".to_string(),
                convert_json_to_qdrant_value(metadata)?,
            );
        }

        // Update timestamp to reflect when it was last modified
        payload.insert(
            "timestamp".to_string(),
            QdrantValue::from(chrono::Utc::now().to_rfc3339()),
        );

        let point = PointStruct::new(point_id.to_string(), embedding.clone(), payload);

        // Update the point using upsert (this will replace the existing point)
        let upsert_points = UpsertPointsBuilder::new(&collection, vec![point]);
        self.client
            .upsert_points(upsert_points)
            .await
            .map_err(|e| QdrantClientError(format!("Failed to update point: {}", e)))?;

        Ok(())
    }

    /// Search for information in Qdrant
    pub async fn search(
        &self,
        query: &str,
        collection_name: Option<&str>,
        limit: u64,
    ) -> Result<Vec<SearchResult>, QdrantClientError> {
        let collection = self.get_collection_name(collection_name)?;

        // Check if collection exists before searching
        let collections = match self.client.list_collections().await {
            Ok(collections) => collections,
            Err(e) => {
                // If we can't list collections (server down, wrong URL, etc.), treat as no collections exist
                tracing::warn!(
                    "Cannot access Qdrant server (server may be down or misconfigured): {}. Treating as empty collection.",
                    e
                );
                tracing::info!(
                    "Collection '{}' cannot be verified (Qdrant unavailable), returning empty search results for query: '{}'",
                    collection,
                    query
                );
                return Ok(vec![]);
            }
        };

        let collection_exists = collections
            .collections
            .iter()
            .any(|coll| coll.name == collection);

        if !collection_exists {
            // Return empty results if collection doesn't exist yet
            tracing::info!(
                "Collection '{}' doesn't exist yet, returning empty search results for query: '{}'",
                collection,
                query
            );
            return Ok(vec![]);
        }

        tracing::debug!(
            "Searching collection '{}' for query: '{}'",
            collection,
            query
        );

        // Generate query embeddings with better error handling
        tracing::debug!("Generating embeddings for search query...");
        let embeddings = self.embedding_model.embed(vec![query], None).map_err(|e| {
            tracing::error!("Failed to generate query embeddings: {}", e);
            QdrantClientError(format!("Failed to generate query embeddings: {}", e))
        })?;

        let query_embedding = embeddings
            .first()
            .ok_or_else(|| QdrantClientError("No query embeddings generated".to_string()))?;

        // Search for similar points
        let search_points = SearchPointsBuilder::new(&collection, query_embedding.clone(), limit)
            .with_payload(true);

        let search_result = self
            .client
            .search_points(search_points)
            .await
            .map_err(|e| QdrantClientError(format!("Failed to search points: {}", e)))?;

        let mut results = Vec::new();
        for scored_point in search_result.result {
            let point_id = scored_point
                .id
                .and_then(|id| id.point_id_options)
                .map(|id_option| match id_option {
                    qdrant_client::qdrant::point_id::PointIdOptions::Num(n) => n.to_string(),
                    qdrant_client::qdrant::point_id::PointIdOptions::Uuid(u) => u,
                })
                .unwrap_or_else(|| "unknown".to_string());

            let content = scored_point
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Unknown content".to_string());

            let metadata = scored_point
                .payload
                .get("metadata")
                .map(|v| convert_qdrant_value_to_json(v.clone()));

            let timestamp = scored_point
                .payload
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            results.push(SearchResult {
                point_id,
                content,
                score: scored_point.score,
                metadata,
                timestamp,
            });
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

/// Convert serde_json::Value to qdrant::Value
fn convert_json_to_qdrant_value(json_value: Value) -> Result<QdrantValue, QdrantClientError> {
    match json_value {
        Value::Null => Ok(QdrantValue::from("")),
        Value::Bool(b) => Ok(QdrantValue::from(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(QdrantValue::from(i))
            } else if let Some(f) = n.as_f64() {
                Ok(QdrantValue::from(f))
            } else {
                Ok(QdrantValue::from(n.to_string()))
            }
        }
        Value::String(s) => Ok(QdrantValue::from(s)),
        Value::Array(arr) => {
            let mut qdrant_list = Vec::new();
            for item in arr {
                qdrant_list.push(convert_json_to_qdrant_value(item)?);
            }
            Ok(QdrantValue::from(qdrant_list))
        }
        Value::Object(obj) => {
            let qdrant_struct: Vec<(&str, QdrantValue)> = obj
                .iter()
                .map(
                    |(key, value)| -> Result<(&str, QdrantValue), QdrantClientError> {
                        Ok((key.as_str(), convert_json_to_qdrant_value(value.clone())?))
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            Ok(QdrantValue::from(qdrant_struct))
        }
    }
}

/// Convert qdrant::Value to serde_json::Value
fn convert_qdrant_value_to_json(qdrant_value: QdrantValue) -> Value {
    match qdrant_value.kind {
        Some(qdrant_client::qdrant::value::Kind::NullValue(_)) => Value::Null,
        Some(qdrant_client::qdrant::value::Kind::BoolValue(b)) => Value::Bool(b),
        Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) => Value::Number(i.into()),
        Some(qdrant_client::qdrant::value::Kind::DoubleValue(f)) => {
            Value::Number(serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()))
        }
        Some(qdrant_client::qdrant::value::Kind::StringValue(s)) => Value::String(s),
        Some(qdrant_client::qdrant::value::Kind::ListValue(list)) => {
            let mut json_list = Vec::new();
            for item in list.values {
                json_list.push(convert_qdrant_value_to_json(item));
            }
            Value::Array(json_list)
        }
        Some(qdrant_client::qdrant::value::Kind::StructValue(structure)) => {
            let mut json_obj = serde_json::Map::new();
            for (key, value) in structure.fields {
                json_obj.insert(key, convert_qdrant_value_to_json(value));
            }
            Value::Object(json_obj)
        }
        None => Value::Null,
    }
}
