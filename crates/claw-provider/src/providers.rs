use std::sync::Arc;

use crate::{error::ProviderError, traits::LLMProvider};

/// Create a provider from environment variables.
///
/// Detects the provider from the `CLAW_PROVIDER` env var (default: `"anthropic"`).
///
/// Supported values depend on which provider features are enabled.
/// By default all providers are included: `anthropic`, `openai`, `ollama`,
/// `deepseek`, `moonshot`.
pub fn provider_from_env() -> Result<Arc<dyn LLMProvider>, ProviderError> {
    let provider_name = std::env::var("CLAW_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());
    match provider_name.as_str() {
        #[cfg(feature = "anthropic")]
        "anthropic" => Ok(Arc::new(crate::anthropic::AnthropicProvider::from_env()?)),
        #[cfg(feature = "openai")]
        "openai" => Ok(Arc::new(crate::openai::OpenAIProvider::from_env()?)),
        #[cfg(feature = "ollama")]
        "ollama" => Ok(Arc::new(crate::ollama::OllamaProvider::from_env()?)),
        #[cfg(feature = "deepseek")]
        "deepseek" => Ok(Arc::new(crate::deepseek::DeepSeekProvider::from_env()?)),
        #[cfg(feature = "moonshot")]
        "moonshot" => Ok(Arc::new(crate::moonshot::MoonshotProvider::from_env()?)),
        #[cfg(feature = "gemini")]
        "gemini" => Ok(Arc::new(crate::gemini::gemini_provider_from_env()?)),
        #[cfg(feature = "mistral")]
        "mistral" => Ok(Arc::new(crate::mistral::mistral_provider_from_env()?)),
        #[cfg(feature = "azure-openai")]
        "azure-openai" => Ok(Arc::new(crate::azure_openai::azure_openai_provider_from_env()?)),
        other => Err(ProviderError::Other(format!("unknown provider: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var mutation tests must run serially to avoid races.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_provider_from_env_unknown() {
        let _guard = ENV_LOCK.lock().unwrap();
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

    #[test]
    fn test_provider_from_env_gemini_missing_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAW_PROVIDER", "gemini");
        std::env::remove_var("GEMINI_API_KEY");
        let result = provider_from_env();
        std::env::remove_var("CLAW_PROVIDER");
        // With gemini feature enabled this should fail with Auth error
        // Without feature enabled it fails with "unknown provider"
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_from_env_mistral_missing_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAW_PROVIDER", "mistral");
        std::env::remove_var("MISTRAL_API_KEY");
        let result = provider_from_env();
        std::env::remove_var("CLAW_PROVIDER");
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_from_env_azure_missing_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAW_PROVIDER", "azure-openai");
        std::env::remove_var("AZURE_OPENAI_API_KEY");
        let result = provider_from_env();
        std::env::remove_var("CLAW_PROVIDER");
        assert!(result.is_err());
    }
}
