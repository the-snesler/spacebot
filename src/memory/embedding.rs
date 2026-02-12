//! Embedding generation via fastembed.

use crate::error::{LlmError, Result};

/// Embedding model wrapper.
pub struct EmbeddingModel {
    model: fastembed::TextEmbedding,
}

impl EmbeddingModel {
    /// Create a new embedding model with the default all-MiniLM-L6-v2.
    pub fn new() -> Result<Self> {
        let model = fastembed::TextEmbedding::try_new(Default::default())
            .map_err(|e| LlmError::EmbeddingFailed(e.to_string()))?;
        
        Ok(Self { model })
    }
    
    /// Generate embeddings for multiple texts.
    pub fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.model.embed(texts, None)
            .map_err(|e| LlmError::EmbeddingFailed(e.to_string()).into())
    }
    
    /// Generate embedding for a single text.
    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed(vec![text.to_string()])?;
        Ok(embeddings.into_iter().next().unwrap_or_default())
    }
}

impl Default for EmbeddingModel {
    fn default() -> Self {
        // Note: In production, this should handle the error properly
        // For now, we panic on initialization failure
        Self::new().expect("Failed to initialize embedding model")
    }
}

/// Convenience function to embed a single text.
pub async fn embed_text(text: &str) -> Result<Vec<f32>> {
    // Since fastembed is synchronous, we run it in a blocking task
    let text = text.to_string();
    let result = tokio::task::spawn_blocking(move || {
        let model = EmbeddingModel::new()?;
        model.embed_one(&text)
    })
    .await
    .map_err(|e| crate::Error::Other(anyhow::anyhow!("embedding task failed: {}", e)))?;
    
    result
}
