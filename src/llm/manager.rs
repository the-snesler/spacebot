//! LLM manager for provider routing and model resolution.

use crate::config::LlmConfig;
use crate::error::{LlmError, Result};
use anyhow::Context as _;
use std::sync::Arc;

/// Manages LLM provider clients and routes requests by model name.
pub struct LlmManager {
    config: LlmConfig,
    /// HTTP client for making requests.
    http_client: reqwest::Client,
}

impl LlmManager {
    /// Create a new LLM manager with the given configuration.
    pub async fn new(config: LlmConfig) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .with_context(|| "failed to build HTTP client")?;
        
        Ok(Self {
            config,
            http_client,
        })
    }
    
    /// Get the appropriate API key for a provider.
    pub fn get_api_key(&self, provider: &str) -> Result<String> {
        match provider {
            "anthropic" => self.config.anthropic_key.clone()
                .ok_or_else(|| LlmError::MissingProviderKey("anthropic".into()).into()),
            "openai" => self.config.openai_key.clone()
                .ok_or_else(|| LlmError::MissingProviderKey("openai".into()).into()),
            _ => Err(LlmError::UnknownProvider(provider.into()).into()),
        }
    }
    
    /// Get the HTTP client.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }
    
    /// Resolve a model name to provider and model components.
    pub fn resolve_model(&self, model_name: &str) -> Result<(String, String)> {
        // Format: "provider/model-name" or just "model-name"
        if let Some((provider, model)) = model_name.split_once('/') {
            Ok((provider.to_string(), model.to_string()))
        } else {
            // Default to anthropic if no provider specified
            Ok(("anthropic".into(), model_name.into()))
        }
    }
}
