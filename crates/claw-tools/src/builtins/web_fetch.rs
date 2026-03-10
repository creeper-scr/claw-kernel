//! Built-in tool for fetching web content via HTTP GET or POST.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;

use crate::traits::Tool;
use crate::types::{
    FsPermissions, NetworkPermissions, PermissionSet, SubprocessPolicy, ToolContext, ToolError,
    ToolResult, ToolSchema,
};

/// Built-in tool for fetching web content via HTTP GET or POST.
///
/// Enforces a 30s timeout and 4 MiB response body limit.
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

static WEB_FETCH_SCHEMA: OnceLock<ToolSchema> = OnceLock::new();
static WEB_FETCH_PERMS: OnceLock<PermissionSet> = OnceLock::new();

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL via HTTP GET or POST. Max 4 MiB response, 30s timeout."
    }

    fn schema(&self) -> &ToolSchema {
        WEB_FETCH_SCHEMA.get_or_init(|| {
            ToolSchema::new(
                "web_fetch",
                "Fetch content from a URL via HTTP GET or POST. Max 4 MiB response, 30s timeout.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to fetch"
                        },
                        "method": {
                            "type": "string",
                            "enum": ["GET", "POST"],
                            "default": "GET",
                            "description": "HTTP method"
                        },
                        "body": {
                            "type": "string",
                            "description": "Request body for POST requests"
                        },
                        "headers": {
                            "type": "object",
                            "description": "Optional HTTP headers as key-value pairs"
                        }
                    },
                    "required": ["url"]
                }),
            )
        })
    }

    fn permissions(&self) -> &PermissionSet {
        WEB_FETCH_PERMS.get_or_init(|| PermissionSet {
            filesystem: FsPermissions::none(),
            network: NetworkPermissions::default(),
            subprocess: SubprocessPolicy::Denied,
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => {
                return ToolResult::err(ToolError::invalid_args("'url' parameter is required"), 0);
            }
        };
        let method = args["method"].as_str().unwrap_or("GET");

        const MAX_BODY: usize = 4 * 1024 * 1024; // 4 MiB

        let start = std::time::Instant::now();

        let result: Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> = async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?;

            let response = match method {
                "POST" => {
                    let body = args["body"].as_str().unwrap_or("").to_string();
                    client.post(&url).body(body).send().await?
                }
                _ => client.get(&url).send().await?,
            };

            let status = response.status().as_u16();
            let bytes = response.bytes().await?;

            if bytes.len() > MAX_BODY {
                return Ok(ToolResult::err(
                    ToolError::invalid_args(format!(
                        "Response body exceeds 4 MiB limit ({} bytes)",
                        bytes.len()
                    )),
                    start.elapsed().as_millis() as u64,
                ));
            }

            let text = String::from_utf8_lossy(&bytes).into_owned();
            Ok(ToolResult::ok(
                serde_json::json!({ "status": status, "body": text }),
                start.elapsed().as_millis() as u64,
            ))
        }
        .await;

        match result {
            Ok(r) => r,
            Err(e) => ToolResult::err(
                ToolError::internal(format!("HTTP request failed: {e}")),
                start.elapsed().as_millis() as u64,
            ),
        }
    }
}
