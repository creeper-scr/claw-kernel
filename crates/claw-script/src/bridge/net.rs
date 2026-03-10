//! Lua-Rust network bridge with domain/port filtering.

use std::collections::HashSet;
use std::time::Duration;

use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};
use reqwest::Client;

/// Maximum response body size: 4 MiB.
const MAX_RESPONSE_BODY_SIZE: usize = 4 * 1024 * 1024;

/// HTTP response wrapper for Lua.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    /// Get the HTTP status code.
    fn status(&self) -> u16 {
        self.status
    }

    /// Get the response body as string.
    fn body(&self) -> &str {
        &self.body
    }

    /// Get a header value by name.
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

impl UserData for HttpResponse {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("status", |_, this, ()| Ok(this.status()));
        methods.add_method("body", |_, this, ()| Ok(this.body().to_string()));
        methods.add_method("header", |_, this, name: String| Ok(this.header(&name)));
    }
}

/// Network bridge exposing HTTP operations to Lua with domain/port filtering.
///
/// URLs are validated against `allowed_domains` and `allowed_ports`.
/// Loopback access can be explicitly enabled/disabled.
pub struct NetBridge {
    /// HTTP client for making requests.
    client: Client,
    /// Set of allowed domains (e.g., "api.example.com").
    allowed_domains: HashSet<String>,
    /// Set of allowed ports. Empty means only standard ports (80, 443).
    allowed_ports: HashSet<u16>,
    /// Whether loopback addresses are allowed.
    allow_loopback: bool,
    /// Request timeout.
    timeout: Duration,
}

impl NetBridge {
    /// Create a new NetBridge with default settings (no network access).
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            allowed_domains: HashSet::new(),
            allowed_ports: [80, 443].iter().cloned().collect(),
            allow_loopback: false,
            timeout: Duration::from_secs(30),
        }
    }

    /// Create a NetBridge with specific allowed domains.
    pub fn with_domains(domains: impl IntoIterator<Item = String>) -> Self {
        let mut bridge = Self::new();
        bridge.allowed_domains = domains.into_iter().collect();
        bridge
    }

    /// Set allowed ports.
    pub fn with_ports(mut self, ports: impl IntoIterator<Item = u16>) -> Self {
        self.allowed_ports = ports.into_iter().collect();
        self
    }

    /// Set whether loopback is allowed.
    pub fn with_loopback(mut self, allow: bool) -> Self {
        self.allow_loopback = allow;
        self
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("Failed to create HTTP client");
        self
    }

    /// Validate that a URL is allowed.
    ///
    /// Checks domain against allowlist and port restrictions.
    fn validate_url(&self, url: &str) -> Result<reqwest::Url, String> {
        let parsed = url
            .parse::<reqwest::Url>()
            .map_err(|e| format!("Invalid URL '{}': {}", url, e))?;

        // Check scheme
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(format!(
                "Unsupported URL scheme '{}': only http and https are allowed",
                scheme
            ));
        }

        // Get host
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("URL '{}' has no host", url))?;

        // Check for loopback
        if self.is_loopback(host) && !self.allow_loopback {
            return Err(format!(
                "Permission denied: loopback access is not allowed (host: '{}')",
                host
            ));
        }

        // Check domain allowlist (skip if loopback is allowed and this is loopback)
        if !self.allow_loopback || !self.is_loopback(host) {
            if self.allowed_domains.is_empty() {
                return Err(format!(
                    "Permission denied: no network access allowed (URL: '{}')",
                    url
                ));
            }

            let mut allowed = false;
            for allowed_domain in &self.allowed_domains {
                // Exact match or subdomain
                if host == allowed_domain || host.ends_with(&format!(".{}", allowed_domain)) {
                    allowed = true;
                    break;
                }
            }

            if !allowed {
                return Err(format!(
                    "Permission denied: domain '{}' is not in the allowlist",
                    host
                ));
            }
        }

        // Check port
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| format!("URL '{}' has no port", url))?;

        if !self.allowed_ports.contains(&port) {
            return Err(format!(
                "Permission denied: port {} is not allowed (allowed: {:?})",
                port, self.allowed_ports
            ));
        }

        Ok(parsed)
    }

    /// Check if a host is a loopback address.
    fn is_loopback(&self, host: &str) -> bool {
        // Check common loopback names
        if host == "localhost" || host == "127.0.0.1" || host == "::1" {
            return true;
        }

        // Try to parse as IP address
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return ip.is_loopback();
        }

        false
    }

    /// Parse headers from a Lua table.
    fn parse_headers(
        &self,
        _lua: &Lua,
        headers_table: Option<mlua::Table>,
    ) -> LuaResult<reqwest::header::HeaderMap> {
        let mut headers = reqwest::header::HeaderMap::new();

        if let Some(table) = headers_table {
            for pair in table.pairs::<String, String>() {
                let (key, value) = pair?;
                if let Ok(header_name) = key.parse::<reqwest::header::HeaderName>() {
                    if let Ok(header_value) = value.parse::<reqwest::header::HeaderValue>() {
                        headers.insert(header_name, header_value);
                    }
                }
            }
        }

        Ok(headers)
    }

    /// Perform a GET request.
    async fn get(
        &self,
        url: &str,
        headers: Option<reqwest::header::HeaderMap>,
    ) -> Result<HttpResponse, String> {
        let validated_url = self.validate_url(url)?;

        let mut request = self.client.get(validated_url);
        if let Some(h) = headers {
            request = request.headers(h);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        self.convert_response(response).await
    }

    /// Perform a POST request.
    async fn post(
        &self,
        url: &str,
        body: String,
        headers: Option<reqwest::header::HeaderMap>,
    ) -> Result<HttpResponse, String> {
        let validated_url = self.validate_url(url)?;

        let mut request = self.client.post(validated_url).body(body);
        if let Some(h) = headers {
            request = request.headers(h);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        self.convert_response(response).await
    }

    /// Convert a reqwest Response to our HttpResponse.
    async fn convert_response(&self, response: reqwest::Response) -> Result<HttpResponse, String> {
        let status = response.status().as_u16();

        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                let key = k.to_string();
                let value = v.to_str().ok()?.to_string();
                Some((key, value))
            })
            .collect();

        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        if body_bytes.len() > MAX_RESPONSE_BODY_SIZE {
            return Err(format!(
                "Response body too large: {} bytes (max {} bytes)",
                body_bytes.len(),
                MAX_RESPONSE_BODY_SIZE
            ));
        }

        let body = String::from_utf8_lossy(&body_bytes).to_string();

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

impl Default for NetBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl UserData for NetBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method(
            "get",
            |lua, this, (url, headers): (String, Option<mlua::Table>)| {
                // Note: Called from spawn_blocking context; use block_on for async operations.
                let header_map = this.parse_headers(lua, headers)?;
                tokio::runtime::Handle::current()
                    .block_on(this.get(&url, Some(header_map)))
                    .map_err(mlua::Error::runtime)
            },
        );

        methods.add_method(
            "post",
            |lua, this, (url, body, headers): (String, String, Option<mlua::Table>)| {
                // Note: Called from spawn_blocking context; use block_on for async operations.
                let header_map = this.parse_headers(lua, headers)?;
                tokio::runtime::Handle::current()
                    .block_on(this.post(&url, body, Some(header_map)))
                    .map_err(mlua::Error::runtime)
            },
        );
    }
}

/// Register the NetBridge as a global `net` table in the Lua instance.
///
/// # Example in Lua:
/// ```lua
/// -- GET request
/// local response = net:get("https://api.example.com/data")
/// print(response:status())
/// print(response:body())
///
/// -- GET with headers
/// local response = net:get("https://api.example.com/data", {Authorization = "Bearer token"})
///
/// -- POST request
/// local response = net:post("https://api.example.com/submit", "{\"key\":\"value\"}")
///
/// -- Access response headers
/// local content_type = response:header("Content-Type")
/// ```
pub fn register_net(lua: &Lua, bridge: NetBridge) -> LuaResult<()> {
    lua.globals().set("net", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_net_bridge_validate_url_allowed_domain() {
        let bridge = NetBridge::with_domains(vec!["api.example.com".to_string()]);

        let result = bridge.validate_url("https://api.example.com/path");
        assert!(result.is_ok());

        let result = bridge.validate_url("https://sub.api.example.com/path");
        assert!(result.is_ok());
    }

    #[test]
    fn test_net_bridge_validate_url_denied_domain() {
        let bridge = NetBridge::with_domains(vec!["api.example.com".to_string()]);

        let result = bridge.validate_url("https://evil.com/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in the allowlist"));
    }

    #[test]
    fn test_net_bridge_validate_url_no_domains() {
        let bridge = NetBridge::new();

        let result = bridge.validate_url("https://any.com/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no network access allowed"));
    }

    #[test]
    fn test_net_bridge_validate_url_invalid_url() {
        let bridge = NetBridge::with_domains(vec!["example.com".to_string()]);

        let result = bridge.validate_url("not a url");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid URL"));
    }

    #[test]
    fn test_net_bridge_validate_url_loopback_denied() {
        let bridge = NetBridge::with_domains(vec!["example.com".to_string()]);

        let result = bridge.validate_url("http://localhost:8080/path");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("loopback access is not allowed"));
    }

    #[test]
    fn test_net_bridge_validate_url_loopback_allowed() {
        let bridge = NetBridge::with_domains(vec!["example.com".to_string()])
            .with_loopback(true)
            .with_ports([80, 443, 8080]);

        let result = bridge.validate_url("http://localhost:8080/path");
        assert!(result.is_ok());
    }

    #[test]
    fn test_net_bridge_validate_url_port_restriction() {
        let bridge = NetBridge::with_domains(vec!["example.com".to_string()]);

        // Port 443 is allowed by default
        let result = bridge.validate_url("https://example.com/path");
        assert!(result.is_ok());

        // Port 8080 is not allowed by default
        let result = bridge.validate_url("http://example.com:8080/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("port 8080 is not allowed"));
    }

    #[test]
    fn test_net_bridge_is_loopback() {
        let bridge = NetBridge::new();

        assert!(bridge.is_loopback("localhost"));
        assert!(bridge.is_loopback("127.0.0.1"));
        assert!(bridge.is_loopback("::1"));
        assert!(!bridge.is_loopback("example.com"));
        assert!(!bridge.is_loopback("192.168.1.1"));
    }

    #[test]
    fn test_http_response() {
        let response = HttpResponse {
            status: 200,
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("X-Custom".to_string(), "value".to_string()),
            ],
            body: "{\"key\":\"value\"}".to_string(),
        };

        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), "{\"key\":\"value\"}");
        assert_eq!(
            response.header("Content-Type"),
            Some("application/json".to_string())
        );
        assert_eq!(
            response.header("content-type"),
            Some("application/json".to_string())
        );
        assert_eq!(response.header("X-Custom"), Some("value".to_string()));
        assert_eq!(response.header("Missing"), None);
    }
}
