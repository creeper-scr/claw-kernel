use crate::{
    anthropic::AnthropicProvider,
    deepseek::DeepSeekProvider,
    error::ProviderError,
    moonshot::MoonshotProvider,
    ollama::OllamaProvider,
    openai::OpenAIProvider,
    traits::LLMProvider,
};

/// Create a provider from environment variables.
///
/// Detects the provider from the `CLAW_PROVIDER` env var (default: `"anthropic"`).
///
/// Supported values: `anthropic`, `openai`, `ollama`, `deepseek`, `moonshot`.
pub fn provider_from_env() -> Result<Box<dyn LLMProvider>, ProviderError> {
    let provider_name = std::env::var("CLAW_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());
    match provider_name.as_str() {
        "anthropic" => Ok(Box::new(AnthropicProvider::from_env()?)),
        "openai" => Ok(Box::new(OpenAIProvider::from_env()?)),
        "ollama" => Ok(Box::new(OllamaProvider::from_env()?)),
        "deepseek" => Ok(Box::new(DeepSeekProvider::from_env()?)),
        "moonshot" => Ok(Box::new(MoonshotProvider::from_env()?)),
        other => Err(ProviderError::Other(format!("unknown provider: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_from_env_unknown() {
        // Temporarily set to an unknown provider name
        std::env::set_var("CLAW_PROVIDER", "nonexistent_provider_xyz");
        let result = provider_from_env();
        // Clean up
        std::env::remove_var("CLAW_PROVIDER");
        assert!(result.is_err());
        if let Err(ProviderError::Other(msg)) = result {
            assert!(msg.contains("nonexistent_provider_xyz"));
        } else {
            panic!("expected ProviderError::Other");
        }
    }
}
