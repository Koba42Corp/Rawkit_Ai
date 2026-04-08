use rawkit_core::Soul;
use serde::{Deserialize, Serialize};

/// A search result from the vector index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The soul of the matching graph node.
    pub soul: Soul,
    /// Cosine similarity score (0.0 to 1.0, higher = more similar).
    pub score: f32,
}

/// A search query combining vector similarity with optional graph filters.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The query vector (embedding of the search text).
    pub vector: Vec<f32>,
    /// Maximum number of results to return.
    pub top_k: usize,
    /// Minimum similarity score threshold.
    pub min_score: Option<f32>,
    /// Only search within nodes matching this path prefix.
    pub path_filter: Option<String>,
}

impl SearchQuery {
    pub fn new(vector: Vec<f32>, top_k: usize) -> Self {
        SearchQuery {
            vector,
            top_k,
            min_score: None,
            path_filter: None,
        }
    }

    pub fn with_min_score(mut self, min_score: f32) -> Self {
        self.min_score = Some(min_score);
        self
    }

    pub fn with_path_filter(mut self, prefix: impl Into<String>) -> Self {
        self.path_filter = Some(prefix.into());
        self
    }
}
