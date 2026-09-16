use chromadb::client::ChromaClientOptions;
use chromadb::collection::{CollectionEntries, GetOptions, QueryOptions};
use chromadb::{ChromaClient, ChromaCollection};
use serde_json::{Map, Value};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

use crate::config::VectorDbConfig;
use crate::utils::embed_texts;

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
pub enum VectorClientError {
    #[error("no memory with id {0}")]
    NotFound(String),
    #[error("Vector client error: {0}")]
    Other(String),
}

/// Metadata key of the storage time. It keeps the name it had before source messages existed
/// so older memories keep their time.
const STORED_AT_KEY: &str = "timestamp";
const SOURCE_MESSAGE_IDS_KEY: &str = "source_message_ids";

/// What a memory carries besides its text. Chroma takes only scalar metadata values and rejects
/// an empty metadata map, so the message IDs travel as one comma-separated string and the
/// storage time is always written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryMetadata {
    /// RFC 3339, set on store and refreshed on every update
    pub stored_at: Option<String>,
    /// The Discord messages the memory was derived from, oldest first
    pub source_message_ids: Vec<u64>,
}

impl MemoryMetadata {
    fn from_map(map: &Map<String, Value>) -> Self {
        let stored_at = map
            .get(STORED_AT_KEY)
            .and_then(Value::as_str)
            .map(str::to_string);
        let source_message_ids = map
            .get(SOURCE_MESSAGE_IDS_KEY)
            .and_then(Value::as_str)
            .map(|ids| {
                ids.split(',')
                    .filter_map(|id| id.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            stored_at,
            source_message_ids,
        }
    }

    fn into_map(self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert(
            STORED_AT_KEY.to_string(),
            Value::String(self.stored_at.unwrap_or_else(now)),
        );
        if !self.source_message_ids.is_empty() {
            let ids: Vec<String> = self.source_message_ids.iter().map(u64::to_string).collect();
            map.insert(
                SOURCE_MESSAGE_IDS_KEY.to_string(),
                Value::String(ids.join(",")),
            );
        }
        map
    }

    /// Adds the sources the memory doesn't have yet. Snowflakes grow with time, so sorting
    /// keeps the list oldest first.
    fn add_sources(&mut self, ids: &[u64]) {
        self.source_message_ids.extend_from_slice(ids);
        self.source_message_ids.sort_unstable();
        self.source_message_ids.dedup();
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Shared vector database client with common functionality
pub struct VectorClient {
    client: ChromaClient,
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

        let client = ChromaClient::new(client_options).await.map_err(|e| {
            VectorClientError::Other(format!("Failed to create ChromaDB client: {}", e))
        })?;

        Ok(Self { client, config })
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
                VectorClientError::Other(format!(
                    "Failed to get or create collection {}: {}",
                    collection_name, e
                ))
            })
    }

    fn embed(text: &str) -> Result<Vec<f32>, VectorClientError> {
        embed_texts(vec![text.to_string()])
            .map_err(|e| VectorClientError::Other(format!("Failed to generate embeddings: {}", e)))?
            .pop()
            .ok_or_else(|| VectorClientError::Other("No embeddings generated".to_string()))
    }

    /// The metadata of one memory, or None when the collection has no memory with that id
    async fn get_metadata(
        collection: &ChromaCollection,
        point_id: &str,
    ) -> Result<Option<MemoryMetadata>, VectorClientError> {
        let result = collection
            .get(GetOptions {
                ids: vec![point_id.to_string()],
                include: Some(vec!["metadatas".to_string()]),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                VectorClientError::Other(format!("Failed to get point {}: {}", point_id, e))
            })?;

        if result.ids.is_empty() {
            return Ok(None);
        }
        let metadata = result
            .metadatas
            .and_then(|mut metadatas| metadatas.pop())
            .flatten()
            .map(|map| MemoryMetadata::from_map(&map))
            .unwrap_or_default();
        Ok(Some(metadata))
    }

    /// Store a new memory and return its point id
    pub async fn store(
        &self,
        information: &str,
        channel_id: u64,
        source_message_ids: &[u64],
    ) -> Result<String, VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);
        let collection = self.get_or_create_collection(&collection_name).await?;
        let embedding = Self::embed(information)?;
        let point_id = Uuid::new_v4().to_string();

        let mut metadata = MemoryMetadata {
            stored_at: Some(now()),
            source_message_ids: vec![],
        };
        metadata.add_sources(source_message_ids);

        let collection_entries = CollectionEntries {
            ids: vec![&point_id],
            embeddings: Some(vec![embedding]),
            metadatas: Some(vec![metadata.into_map()]),
            documents: Some(vec![information]),
        };

        collection
            .upsert(collection_entries, None)
            .await
            .map_err(|e| VectorClientError::Other(format!("Failed to store point: {}", e)))?;

        Ok(point_id)
    }

    /// Replace a memory's text and add `source_message_ids` to the ones it already cites.
    /// Returns the metadata as stored. Fails with `NotFound` when no memory has the id rather
    /// than creating one under an id the caller made up.
    pub async fn update(
        &self,
        point_id: &str,
        information: &str,
        channel_id: u64,
        source_message_ids: &[u64],
    ) -> Result<MemoryMetadata, VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);
        let collection = self.get_or_create_collection(&collection_name).await?;

        let Some(mut metadata) = Self::get_metadata(&collection, point_id).await? else {
            return Err(VectorClientError::NotFound(point_id.to_string()));
        };
        metadata.add_sources(source_message_ids);
        metadata.stored_at = Some(now());

        let embedding = Self::embed(information)?;
        let collection_entries = CollectionEntries {
            ids: vec![point_id],
            embeddings: Some(vec![embedding]),
            metadatas: Some(vec![metadata.clone().into_map()]),
            documents: Some(vec![information]),
        };

        collection
            .upsert(collection_entries, None)
            .await
            .map_err(|e| VectorClientError::Other(format!("Failed to update point: {}", e)))?;

        Ok(metadata)
    }

    /// Delete information from the vector database
    pub async fn delete(
        &self,
        channel_id: u64,
        ids: Option<Vec<&str>>,
        where_metadata: Option<Value>,
        where_document: Option<Value>,
    ) -> Result<(), VectorClientError> {
        let collection_name = self.get_collection_name(channel_id);

        // Try to get the collection, return error if it doesn't exist
        let collection = match self.client.get_collection(&collection_name).await {
            Ok(collection) => collection,
            Err(e) => {
                return Err(VectorClientError::Other(format!(
                    "Failed to get collection {}: {}",
                    collection_name, e
                )));
            }
        };

        // Call ChromaDB delete method
        collection
            .delete(ids, where_metadata, where_document)
            .await
            .map_err(|e| VectorClientError::Other(format!("Failed to delete entries: {}", e)))?;

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

        // A channel without memories has no collection yet
        let Ok(collection) = self.client.get_collection(&collection_name).await else {
            return Ok(vec![]);
        };

        let query_embedding = Self::embed(query)?;

        let query_options = QueryOptions {
            query_texts: None,
            query_embeddings: Some(vec![query_embedding]),
            where_metadata: None,
            where_document: None,
            n_results: Some(limit as usize),
            include: Some(vec!["documents", "metadatas", "distances"]),
        };

        let mut query_result = collection
            .query(query_options, None)
            .await
            .map_err(|e| VectorClientError::Other(format!("Failed to search points: {}", e)))?;

        // One query embedding in, so one row of results out
        let ids = query_result.ids.pop().unwrap_or_default();
        let documents = query_result
            .documents
            .take()
            .and_then(|mut rows| rows.pop())
            .unwrap_or_default();
        let distances = query_result
            .distances
            .take()
            .and_then(|mut rows| rows.pop())
            .unwrap_or_default();
        let metadatas = query_result
            .metadatas
            .take()
            .and_then(|mut rows| rows.pop())
            .unwrap_or_default();

        let results = ids
            .into_iter()
            .enumerate()
            .map(|(i, point_id)| {
                // Chroma returns distances; the agent reads similarities
                let score = 1.0 - distances.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0);
                SearchResult {
                    point_id,
                    content: documents.get(i).cloned().unwrap_or_default(),
                    score,
                    metadata: metadatas
                        .get(i)
                        .and_then(Option::as_ref)
                        .map(MemoryMetadata::from_map)
                        .unwrap_or_default(),
                }
            })
            .collect();

        Ok(results)
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub point_id: String,
    pub content: String,
    pub score: f32,
    pub metadata: MemoryMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_round_trips_with_sources_as_one_sorted_string() {
        let mut metadata = MemoryMetadata {
            stored_at: Some("2026-09-14T12:03:30+00:00".to_string()),
            source_message_ids: vec![],
        };
        metadata.add_sources(&[30, 10, 20, 10]);
        assert_eq!(metadata.source_message_ids, vec![10, 20, 30]);

        let map = metadata.clone().into_map();
        assert_eq!(map.get("source_message_ids"), Some(&json!("10,20,30")));
        assert_eq!(MemoryMetadata::from_map(&map), metadata);
    }

    #[test]
    fn legacy_metadata_has_a_time_and_no_sources() {
        let map = json!({ "timestamp": "2026-07-09T00:00:00+00:00" })
            .as_object()
            .cloned()
            .expect("object");
        let metadata = MemoryMetadata::from_map(&map);
        assert_eq!(
            metadata.stored_at.as_deref(),
            Some("2026-07-09T00:00:00+00:00")
        );
        assert!(metadata.source_message_ids.is_empty());

        // Chroma rejects empty metadata, so the time is always written
        assert!(
            MemoryMetadata::default()
                .into_map()
                .contains_key("timestamp")
        );
    }
}
