//! Google Gemini provider via OpenAI-compatible API.
//!
//! Gemini's OpenAI-compatible endpoint accepts the same request/response
//! format as OpenAI, so this module simply configures an [`OpenAIProvider`]
//! with the correct base URL and a sensible default model.
//!
//! # Feature Flag
//!
//! Enable with the `gemini` Cargo feature:
//!
//! ```toml
//! [dependencies]
//! claw-provider = { version = "*", features = ["gemini"] }
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Required | Description |
//! |----------|----------|-------------|
//! | `GEMINI_API_KEY` | Yes | Your Google AI Studio API key |
//! | `GEMINI_MODEL` | No | Model name (default: `gemini-2.0-flash`) |
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_provider::gemini::{gemini_provider, gemini_provider_from_env};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Explicit API key
//! let provider = gemini_provider("your-api-key", "gemini-2.0-flash");
//!
//! // From environment variables
//! let provider = gemini_provider_from_env()?;
//! # Ok(())
//! # }
//! ```

use crate::{error::ProviderError, openai::OpenAIProvider};

/// Gemini OpenAI-compatible base URL.
const GEMINI_BASE_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai";

/// Default Gemini model.
const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";

/// Create a Gemini provider using the OpenAI-compatible endpoint.
///
/// Internally creates an [`OpenAIProvider`] configured for Gemini's API.
///
/// # Arguments
///
/// * `api_key` – Your Google AI Studio API key.
/// * `model` – The Gemini model to use (e.g. `"gemini-2.0-flash"`,
///   `"gemini-1.5-pro"`).
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::gemini::gemini_provider;
/// use claw_provider::LLMProvider;
///
/// let provider = gemini_provider("YOUR_API_KEY", "gemini-2.0-flash");
/// assert_eq!(provider.provider_id(), "openai");
/// assert_eq!(provider.model_id(), "gemini-2.0-flash");
/// ```
pub fn gemini_provider(
    api_key: impl Into<String>,
    model: impl Into<String>,
) -> OpenAIProvider {
    OpenAIProvider::new(api_key, model).with_base_url(GEMINI_BASE_URL)
}

/// Create a Gemini provider from environment variables.
///
/// Reads `GEMINI_API_KEY` (required) and `GEMINI_MODEL` (optional,
/// defaults to `"gemini-2.0-flash"`).
///
/// # Errors
///
/// Returns [`ProviderError::Auth`] if `GEMINI_API_KEY` is not set.
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::gemini::gemini_provider_from_env;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let provider = gemini_provider_from_env()?;
/// # Ok(())
/// # }
/// ```
pub fn gemini_provider_from_env() -> Result<OpenAIProvider, ProviderError> {
    let api_key = std::env::var("GEMINI_API_KEY")
        .map_err(|_| ProviderError::Auth("GEMINI_API_KEY not set".into()))?;
    let model = std::env::var("GEMINI_MODEL")
        .unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string());
    Ok(gemini_provider(api_key, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::LLMProvider;

    #[test]
    fn test_gemini_provider_model_id() {
        let p = gemini_provider("test-key", "gemini-1.5-pro");
        assert_eq!(p.model_id(), "gemini-1.5-pro");
    }

    #[test]
    fn test_gemini_provider_base_url() {
        let p = gemini_provider("test-key", "gemini-2.0-flash");
        assert_eq!(p.base_url, GEMINI_BASE_URL);
    }

    #[test]
    fn test_gemini_provider_default_model_from_env_not_set() {
        std::env::remove_var("GEMINI_API_KEY");
        let result = gemini_provider_from_env();
        assert!(result.is_err());
        if let Err(ProviderError::Auth(msg)) = result {
            assert!(msg.contains("GEMINI_API_KEY"));
        } else {
            panic!("expected ProviderError::Auth");
        }
    }
}
