use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::search::SearchResult;
use rawkit_core::Soul;

/// A point in the vector space, associated with a graph node soul.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorPoint {
    pub soul: Soul,
    pub embedding: Vec<f32>,
}

/// Vector index for semantic search over graph nodes.
///
/// v0.1: Brute-force exact search (correct, simple, fast to ship).
/// v0.2: HNSW approximate search for scale.
///
/// Each graph node can optionally have an associated embedding vector.
/// The index enables nearest neighbor search using cosine similarity.
pub struct VectorIndex {
    points: Arc<RwLock<HashMap<Soul, VectorPoint>>>,
    dimensions: usize,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self {
        VectorIndex {
            points: Arc::new(RwLock::new(HashMap::new())),
            dimensions,
        }
    }

    /// Add or update a vector for a graph node.
    pub fn upsert(&self, soul: &str, embedding: Vec<f32>) -> Result<(), VectorError> {
        if embedding.len() != self.dimensions {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimensions,
                got: embedding.len(),
            });
        }

        self.points.write().unwrap().insert(
            soul.to_string(),
            VectorPoint {
                soul: soul.to_string(),
                embedding,
            },
        );

        Ok(())
    }

    /// Remove a vector from the index.
    pub fn remove(&self, soul: &str) {
        self.points.write().unwrap().remove(soul);
    }

    /// Search for the nearest neighbors to a query vector using cosine similarity.
    pub fn search(&self, query: &[f32], top_k: usize) -> Result<Vec<SearchResult>, VectorError> {
        if query.len() != self.dimensions {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimensions,
                got: query.len(),
            });
        }

        let points = self.points.read().unwrap();

        let query_norm = vec_norm(query);
        if query_norm == 0.0 {
            return Ok(Vec::new());
        }

        let mut scored: Vec<SearchResult> = points
            .values()
            .map(|p| {
                let sim = cosine_similarity(query, &p.embedding, query_norm);
                SearchResult {
                    soul: p.soul.clone(),
                    score: sim,
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored)
    }

    /// Get the number of vectors in the index.
    pub fn len(&self) -> usize {
        self.points.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }
}

fn cosine_similarity(a: &[f32], b: &[f32], a_norm: f32) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let b_norm = vec_norm(b);
    if b_norm == 0.0 {
        return 0.0;
    }
    dot / (a_norm * b_norm)
}

fn vec_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },
    #[error("index error: {0}")]
    IndexError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_search() {
        let index = VectorIndex::new(3);

        index.upsert("doc/1", vec![1.0, 0.0, 0.0]).unwrap();
        index.upsert("doc/2", vec![0.0, 1.0, 0.0]).unwrap();
        index.upsert("doc/3", vec![0.9, 0.1, 0.0]).unwrap();

        let results = index.search(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].soul, "doc/1");
        assert!(results[0].score > 0.99);
    }

    #[test]
    fn test_dimension_mismatch() {
        let index = VectorIndex::new(3);
        let result = index.upsert("test", vec![1.0, 0.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove() {
        let index = VectorIndex::new(3);
        index.upsert("doc/1", vec![1.0, 0.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);
        index.remove("doc/1");
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_empty_search() {
        let index = VectorIndex::new(3);
        let results = index.search(&[1.0, 0.0, 0.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_similar_vectors_rank_higher() {
        let index = VectorIndex::new(3);
        index.upsert("close", vec![0.9, 0.1, 0.0]).unwrap();
        index.upsert("far", vec![0.0, 0.0, 1.0]).unwrap();
        index.upsert("medium", vec![0.5, 0.5, 0.0]).unwrap();

        let results = index.search(&[1.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(results[0].soul, "close");
        assert_eq!(results[2].soul, "far");
    }
}
