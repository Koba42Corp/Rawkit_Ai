use serde::{Deserialize, Serialize};

/// Trait for pluggable embedding providers.
///
/// Implementations can use local ONNX models, remote APIs (OpenAI, Anthropic, Cohere),
/// or any other embedding source.
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError>;
    fn dimensions(&self) -> usize;
    fn model_name(&self) -> &str;
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("API error: {0}")]
    ApiError(String),
}

/// Configuration for which embedding provider to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmbeddingConfig {
    /// Use a local ONNX model (server-side only).
    #[serde(rename = "local")]
    Local {
        model_path: String,
        model_name: String,
    },
    /// Use a remote embedding API.
    #[serde(rename = "remote")]
    Remote {
        api_url: String,
        api_key: Option<String>,
        model_name: String,
    },
    /// No embedding — vectors must be provided directly.
    #[serde(rename = "none")]
    None,
}

/// A mock embedding provider for testing.
/// Generates deterministic fake embeddings based on text content.
#[cfg(test)]
pub struct MockEmbedding {
    dims: usize,
}

#[cfg(test)]
impl MockEmbedding {
    pub fn new(dims: usize) -> Self {
        MockEmbedding { dims }
    }
}

#[cfg(test)]
impl EmbeddingProvider for MockEmbedding {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        // Generate a simple deterministic embedding from text
        let mut vec = vec![0.0f32; self.dims];
        for (i, byte) in text.bytes().enumerate() {
            vec[i % self.dims] += byte as f32 / 255.0;
        }
        // Normalize
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        "mock-embedding"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_embedding() {
        let provider = MockEmbedding::new(128);
        let emb = provider.embed("hello world").unwrap();
        assert_eq!(emb.len(), 128);

        // Normalized to unit length
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_similar_text_similar_embeddings() {
        let provider = MockEmbedding::new(128);
        let e1 = provider.embed("hello world").unwrap();
        let e2 = provider.embed("hello world!").unwrap();
        let e3 = provider.embed("completely different text").unwrap();

        // Cosine similarity between similar texts should be higher
        let sim_12: f32 = e1.iter().zip(e2.iter()).map(|(a, b)| a * b).sum();
        let sim_13: f32 = e1.iter().zip(e3.iter()).map(|(a, b)| a * b).sum();

        assert!(sim_12 > sim_13);
    }
}
