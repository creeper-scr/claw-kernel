//! Mistral AI provider via OpenAI-compatible API.
//!
//! Mistral's API is compatible with the OpenAI chat completions format,
//! so this module configures an [`OpenAIProvider`] with Mistral's endpoint
//! and a sensible default model.
//!
//! # Feature Flag
//!
//! Enable with the `mistral` Cargo feature:
//!
//! ```toml
//! [dependencies]
//! claw-provider = { version = "*", features = ["mistral"] }
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Required | Description |
//! |----------|----------|-------------|
//! | `MISTRAL_API_KEY` | Yes | Your Mistral AI API key |
//! | `MISTRAL_MODEL` | No | Model name (default: `mistral-large-latest`) |
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_provider::mistral::{mistral_provider, mistral_provider_from_env};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Explicit API key
//! let provider = mistral_provider("your-api-key", "mistral-large-latest");
//!
//! // From environment variables
//! let provider = mistral_provider_from_env()?;
//! # Ok(())
//! # }
//! ```

use crate::{error::ProviderError, openai::OpenAIProvider};

/// Mistral AI base URL.
const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";

/// Default Mistral model.
const DEFAULT_MISTRAL_MODEL: &str = "mistral-large-latest";

/// Create a Mistral AI provider using the OpenAI-compatible endpoint.
///
/// # Arguments
///
/// * `api_key` – Your Mistral AI API key.
/// * `model` – The Mistral model to use (e.g. `"mistral-large-latest"`,
///   `"mistral-small-latest"`, `"open-mixtral-8x7b"`).
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::mistral::mistral_provider;
/// use claw_provider::LLMProvider;
///
/// let provider = mistral_provider("YOUR_API_KEY", "mistral-large-latest");
/// assert_eq!(provider.model_id(), "mistral-large-latest");
/// ```
pub fn mistral_provider(
    api_key: impl Into<String>,
    model: impl Into<String>,
) -> OpenAIProvider {
    OpenAIProvider::new(api_key, model).with_base_url(MISTRAL_BASE_URL)
}

/// Create a Mistral AI provider from environment variables.
///
/// Reads `MISTRAL_API_KEY` (required) and `MISTRAL_MODEL` (optional,
/// defaults to `"mistral-large-latest"`).
///
/// # Errors
///
/// Returns [`ProviderError::Auth`] if `MISTRAL_API_KEY` is not set.
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::mistral::mistral_provider_from_env;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let provider = mistral_provider_from_env()?;
/// # Ok(())
/// # }
/// ```
pub fn mistral_provider_from_env() -> Result<OpenAIProvider, ProviderError> {
    let api_key = std::env::var("MISTRAL_API_KEY")
        .map_err(|_| ProviderError::Auth("MISTRAL_API_KEY not set".into()))?;
    let model = std::env::var("MISTRAL_MODEL")
        .unwrap_or_else(|_| DEFAULT_MISTRAL_MODEL.to_string());
    Ok(mistral_provider(api_key, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::LLMProvider;

    #[test]
    fn test_mistral_provider_model_id() {
        let p = mistral_provider("test-key", "open-mixtral-8x7b");
        assert_eq!(p.model_id(), "open-mixtral-8x7b");
    }

    #[test]
    fn test_mistral_provider_base_url() {
        let p = mistral_provider("test-key", "mistral-large-latest");
        assert_eq!(p.base_url, MISTRAL_BASE_URL);
    }

    #[test]
    fn test_mistral_provider_from_env_not_set() {
        std::env::remove_var("MISTRAL_API_KEY");
        let result = mistral_provider_from_env();
        assert!(result.is_err());
        if let Err(ProviderError::Auth(msg)) = result {
            assert!(msg.contains("MISTRAL_API_KEY"));
        } else {
            panic!("expected ProviderError::Auth");
        }
    }
}
