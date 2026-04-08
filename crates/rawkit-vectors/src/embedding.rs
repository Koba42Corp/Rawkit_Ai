use serde::{Deserialize, Serialize};
/// Trait for pluggable embedding providers.
///
/// Implementations can use local hash-based embeddings, remote APIs (OpenAI, etc.),
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
    #[error("HTTP error: {0}")]
    HttpError(String),
}

/// Configuration for which embedding provider to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmbeddingConfig {
    /// Use a remote OpenAI-compatible embedding API.
    #[serde(rename = "openai")]
    OpenAI {
        api_key: String,
        model: Option<String>,
        base_url: Option<String>,
    },
    /// Use the built-in local hash-based embeddings (no API key needed).
    /// Good enough for basic semantic similarity without external dependencies.
    #[serde(rename = "local")]
    Local { dimensions: Option<usize> },
    /// No embedding — vectors must be provided directly.
    #[serde(rename = "none")]
    None,
}

// ─── Local Hash Embedding ─────────────────────────────────────────────────────

/// A local embedding provider that generates deterministic embeddings from text
/// using character n-gram hashing. No external API or model required.
///
/// This is NOT a real language model — it doesn't understand semantics deeply.
/// But it DOES produce consistent embeddings where lexically similar texts get
/// similar vectors. Useful for:
/// - Development and testing without API keys
/// - Offline/air-gapped environments
/// - Basic deduplication and near-duplicate detection
///
/// For real semantic understanding, use the OpenAI provider.
pub struct LocalHashEmbedding {
    dims: usize,
}

impl LocalHashEmbedding {
    pub fn new(dims: usize) -> Self {
        LocalHashEmbedding { dims }
    }
}

impl EmbeddingProvider for LocalHashEmbedding {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vec = vec![0.0f32; self.dims];
        let lower = text.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();

        // Unigrams
        for (i, ch) in chars.iter().enumerate() {
            let hash = simple_hash(&[*ch as u32]) as usize;
            vec[hash % self.dims] += 1.0;
            // Positional encoding (attenuated)
            vec[(hash + i) % self.dims] += 0.3;
        }

        // Bigrams
        for window in chars.windows(2) {
            let hash = simple_hash(&[window[0] as u32, window[1] as u32]) as usize;
            vec[hash % self.dims] += 1.5;
        }

        // Trigrams
        for window in chars.windows(3) {
            let hash =
                simple_hash(&[window[0] as u32, window[1] as u32, window[2] as u32]) as usize;
            vec[hash % self.dims] += 2.0;
        }

        // Word-level features
        for word in lower.split_whitespace() {
            let hash = simple_hash_str(word) as usize;
            vec[hash % self.dims] += 3.0;
            // Word length feature
            vec[(hash + word.len()) % self.dims] += 0.5;
        }

        // L2 normalize
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
        "rawkit-local-hash-v1"
    }
}

fn simple_hash(values: &[u32]) -> u32 {
    let mut hash: u32 = 5381;
    for v in values {
        hash = hash.wrapping_mul(33).wrapping_add(*v);
    }
    hash
}

fn simple_hash_str(s: &str) -> u32 {
    let mut hash: u32 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }
    hash
}

// ─── OpenAI-Compatible Embedding Provider ─────────────────────────────────────

/// Embedding provider that calls OpenAI's embedding API (or any compatible endpoint).
///
/// Supports:
/// - OpenAI (text-embedding-3-small, text-embedding-3-large, text-embedding-ada-002)
/// - Azure OpenAI
/// - Any OpenAI-compatible API (Ollama, LiteLLM, vLLM, etc.)
pub struct OpenAIEmbedding {
    api_key: String,
    model: String,
    base_url: String,
    dims: usize,
}

impl OpenAIEmbedding {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<String>) -> Self {
        let model = model.unwrap_or_else(|| "text-embedding-3-small".to_string());
        let dims = match model.as_str() {
            "text-embedding-3-large" => 3072,
            "text-embedding-3-small" => 1536,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        };
        OpenAIEmbedding {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            dims,
        }
    }

    /// Make the HTTP request to the embedding API.
    /// This is a blocking call — use from sync context or wrap in spawn_blocking.
    fn call_api(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let url = format!("{}/embeddings", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        // Use ureq for sync HTTP (no tokio dependency needed in the vectors crate)
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| EmbeddingError::HttpError(e.to_string()))?;

        let resp_body: serde_json::Value = response
            .into_json()
            .map_err(|e| EmbeddingError::ApiError(e.to_string()))?;

        let data = resp_body["data"]
            .as_array()
            .ok_or_else(|| EmbeddingError::ApiError("missing data field".into()))?;

        let mut results = Vec::with_capacity(texts.len());
        for item in data {
            let embedding: Vec<f32> = item["embedding"]
                .as_array()
                .ok_or_else(|| EmbeddingError::ApiError("missing embedding".into()))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect();
            results.push(embedding);
        }

        Ok(results)
    }
}

impl EmbeddingProvider for OpenAIEmbedding {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let results = self.call_api(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::ApiError("empty response".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        // OpenAI supports up to 2048 inputs per request; chunk if needed
        let mut all_results = Vec::new();
        for chunk in texts.chunks(2048) {
            let mut results = self.call_api(chunk)?;
            all_results.append(&mut results);
        }
        Ok(all_results)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// ─── Factory ──────────────────────────────────────────────────────────────────

/// Create an embedding provider from config.
pub fn create_provider(config: &EmbeddingConfig) -> Option<Box<dyn EmbeddingProvider>> {
    match config {
        EmbeddingConfig::OpenAI {
            api_key,
            model,
            base_url,
        } => Some(Box::new(OpenAIEmbedding::new(
            api_key.clone(),
            model.clone(),
            base_url.clone(),
        ))),
        EmbeddingConfig::Local { dimensions } => {
            Some(Box::new(LocalHashEmbedding::new(dimensions.unwrap_or(384))))
        }
        EmbeddingConfig::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_hash_embedding() {
        let provider = LocalHashEmbedding::new(128);
        let emb = provider.embed("hello world").unwrap();
        assert_eq!(emb.len(), 128);

        // Normalized to unit length
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_similar_text_similar_embeddings() {
        let provider = LocalHashEmbedding::new(256);
        let e1 = provider.embed("the cat sat on the mat").unwrap();
        let e2 = provider.embed("the cat sat on a mat").unwrap();
        let e3 = provider.embed("quantum computing research papers").unwrap();

        let sim_12: f32 = e1.iter().zip(e2.iter()).map(|(a, b)| a * b).sum();
        let sim_13: f32 = e1.iter().zip(e3.iter()).map(|(a, b)| a * b).sum();

        assert!(
            sim_12 > sim_13,
            "similar texts should have higher cosine similarity: {sim_12} vs {sim_13}"
        );
    }

    #[test]
    fn test_deterministic() {
        let provider = LocalHashEmbedding::new(128);
        let e1 = provider.embed("test input").unwrap();
        let e2 = provider.embed("test input").unwrap();
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_create_provider_local() {
        let config = EmbeddingConfig::Local {
            dimensions: Some(256),
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.dimensions(), 256);
        assert_eq!(provider.model_name(), "rawkit-local-hash-v1");
    }

    #[test]
    fn test_create_provider_none() {
        let config = EmbeddingConfig::None;
        assert!(create_provider(&config).is_none());
    }
}
