//! Azure OpenAI provider.
//!
//! Azure OpenAI Service hosts OpenAI models in Azure infrastructure with
//! per-deployment endpoints. Each deployment has its own URL in the form:
//!
//! ```text
//! https://{resource}.openai.azure.com/openai/deployments/{deployment}
//! ```
//!
//! The Azure API is OpenAI-compatible but requires an `api-version` query
//! parameter on every request. This module appends that parameter to the
//! base URL so that the underlying [`OpenAIProvider`] transport sends it
//! automatically on every request.
//!
//! # Feature Flag
//!
//! Enable with the `azure-openai` Cargo feature:
//!
//! ```toml
//! [dependencies]
//! claw-provider = { version = "*", features = ["azure-openai"] }
//! ```
//!
//! # Environment Variables
//!
//! | Variable | Required | Description |
//! |----------|----------|-------------|
//! | `AZURE_OPENAI_API_KEY` | Yes | Azure OpenAI API key |
//! | `AZURE_OPENAI_RESOURCE` | Yes | Azure resource name |
//! | `AZURE_OPENAI_DEPLOYMENT` | Yes | Deployment / model name |
//! | `AZURE_OPENAI_API_VERSION` | No | API version (default: `2024-02-01`) |
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_provider::azure_openai::{azure_openai_provider, azure_openai_provider_from_env};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Explicit configuration
//! let provider = azure_openai_provider(
//!     "my-resource",
//!     "gpt-4o-deployment",
//!     "2024-02-01",
//!     "your-api-key",
//! );
//!
//! // From environment variables
//! let provider = azure_openai_provider_from_env()?;
//! # Ok(())
//! # }
//! ```

use crate::{error::ProviderError, openai::OpenAIProvider};

/// Default Azure OpenAI API version.
const DEFAULT_AZURE_API_VERSION: &str = "2024-02-01";

/// Create an Azure OpenAI provider.
///
/// Constructs the deployment-scoped base URL and appends the `api-version`
/// query parameter, then delegates all completion logic to [`OpenAIProvider`].
///
/// # Arguments
///
/// * `resource_name` – Your Azure OpenAI resource name (the subdomain part of
///   `{resource}.openai.azure.com`).
/// * `deployment_id` – The deployment name in Azure (maps to a specific model).
/// * `api_version` – The Azure OpenAI API version string (e.g. `"2024-02-01"`).
/// * `api_key` – Your Azure OpenAI API key.
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::azure_openai::azure_openai_provider;
/// use claw_provider::LLMProvider;
///
/// let provider = azure_openai_provider(
///     "my-resource",
///     "gpt-4o",
///     "2024-02-01",
///     "YOUR_AZURE_KEY",
/// );
/// assert_eq!(provider.model_id(), "gpt-4o");
/// ```
pub fn azure_openai_provider(
    resource_name: impl Into<String>,
    deployment_id: impl Into<String>,
    api_version: impl Into<String>,
    api_key: impl Into<String>,
) -> OpenAIProvider {
    let resource = resource_name.into();
    let deployment = deployment_id.into();
    let version = api_version.into();

    // Azure endpoint: base URL includes the deployment path; the chat
    // completions relative path `/chat/completions` is appended by OpenAIProvider.
    // The `api-version` query param is embedded in the base URL so it is
    // sent on every request without modifying the transport layer.
    let base_url = format!(
        "https://{resource}.openai.azure.com/openai/deployments/{deployment}?api-version={version}"
    );

    OpenAIProvider::new(api_key, deployment).with_base_url(base_url)
}

/// Create an Azure OpenAI provider from environment variables.
///
/// Reads the following variables:
///
/// - `AZURE_OPENAI_API_KEY` (required)
/// - `AZURE_OPENAI_RESOURCE` (required)
/// - `AZURE_OPENAI_DEPLOYMENT` (required)
/// - `AZURE_OPENAI_API_VERSION` (optional, defaults to `"2024-02-01"`)
///
/// # Errors
///
/// Returns [`ProviderError::Auth`] if any required variable is missing.
///
/// # Example
///
/// ```rust,no_run
/// use claw_provider::azure_openai::azure_openai_provider_from_env;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let provider = azure_openai_provider_from_env()?;
/// # Ok(())
/// # }
/// ```
pub fn azure_openai_provider_from_env() -> Result<OpenAIProvider, ProviderError> {
    let api_key = std::env::var("AZURE_OPENAI_API_KEY")
        .map_err(|_| ProviderError::Auth("AZURE_OPENAI_API_KEY not set".into()))?;
    let resource = std::env::var("AZURE_OPENAI_RESOURCE")
        .map_err(|_| ProviderError::Auth("AZURE_OPENAI_RESOURCE not set".into()))?;
    let deployment = std::env::var("AZURE_OPENAI_DEPLOYMENT")
        .map_err(|_| ProviderError::Auth("AZURE_OPENAI_DEPLOYMENT not set".into()))?;
    let api_version = std::env::var("AZURE_OPENAI_API_VERSION")
        .unwrap_or_else(|_| DEFAULT_AZURE_API_VERSION.to_string());

    Ok(azure_openai_provider(resource, deployment, api_version, api_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::LLMProvider;

    // Env-var mutation tests must run serially to avoid races.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_azure_provider_model_id() {
        let p = azure_openai_provider("my-resource", "gpt-4o-deploy", "2024-02-01", "key");
        assert_eq!(p.model_id(), "gpt-4o-deploy");
    }

    #[test]
    fn test_azure_provider_base_url_format() {
        let p = azure_openai_provider("my-resource", "gpt-4o-deploy", "2024-02-01", "key");
        assert!(p.base_url.contains("my-resource.openai.azure.com"));
        assert!(p.base_url.contains("gpt-4o-deploy"));
        assert!(p.base_url.contains("2024-02-01"));
    }

    #[test]
    fn test_azure_provider_from_env_missing_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("AZURE_OPENAI_API_KEY");
        let result = azure_openai_provider_from_env();
        assert!(result.is_err());
        if let Err(ProviderError::Auth(msg)) = result {
            assert!(msg.contains("AZURE_OPENAI_API_KEY"));
        } else {
            panic!("expected ProviderError::Auth");
        }
    }

    #[test]
    fn test_azure_provider_from_env_missing_resource() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("AZURE_OPENAI_API_KEY", "fake-key");
        std::env::remove_var("AZURE_OPENAI_RESOURCE");
        let result = azure_openai_provider_from_env();
        std::env::remove_var("AZURE_OPENAI_API_KEY");
        assert!(result.is_err());
        if let Err(ProviderError::Auth(msg)) = result {
            assert!(msg.contains("AZURE_OPENAI_RESOURCE"));
        } else {
            panic!("expected ProviderError::Auth");
        }
    }
}
