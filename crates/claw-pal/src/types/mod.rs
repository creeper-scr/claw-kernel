//! Common types for claw-pal.
//!
//! Provides configuration and policy types used across sandbox, IPC, and process management.

pub mod ipc;
pub mod process;
pub mod sandbox;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Filesystem access rule.
///
/// Defines read, write, and execute permissions for a specific path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathRule {
    /// Path to allow/restrict.
    pub path: PathBuf,
    /// Allow read access.
    pub read: bool,
    /// Allow write access.
    pub write: bool,
    /// Allow execute access.
    pub execute: bool,
}

impl PathRule {
    /// Create a new path rule with all permissions disabled.
    ///
    /// Use the builder methods (`with_read`, `with_write`, `with_execute`)
    /// to enable specific permissions.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::PathRule;
    /// use std::path::PathBuf;
    ///
    /// let rule = PathRule::new(PathBuf::from("/data/logs"))
    ///     .with_read()
    ///     .with_write();
    ///
    /// assert_eq!(rule.path, PathBuf::from("/data/logs"));
    /// assert!(rule.read);
    /// assert!(rule.write);
    /// assert!(!rule.execute);
    /// ```
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            read: false,
            write: false,
            execute: false,
        }
    }

    /// Enable read access for this path.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::PathRule;
    /// use std::path::PathBuf;
    ///
    /// let rule = PathRule::new(PathBuf::from("/etc/config")).with_read();
    /// assert!(rule.read);
    /// ```
    pub fn with_read(mut self) -> Self {
        self.read = true;
        self
    }

    /// Enable write access for this path.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::PathRule;
    /// use std::path::PathBuf;
    ///
    /// let rule = PathRule::new(PathBuf::from("/tmp/output")).with_write();
    /// assert!(rule.write);
    /// ```
    pub fn with_write(mut self) -> Self {
        self.write = true;
        self
    }

    /// Enable execute access for this path.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::PathRule;
    /// use std::path::PathBuf;
    ///
    /// let rule = PathRule::new(PathBuf::from("/bin/script.sh")).with_execute();
    /// assert!(rule.execute);
    /// ```
    pub fn with_execute(mut self) -> Self {
        self.execute = true;
        self
    }
}

/// Network access rule.
///
/// Defines allowed or denied network endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetRule {
    /// Hostname or IP address.
    pub host: String,
    /// Port number (None = all ports).
    pub port: Option<u16>,
    /// Allow (true) or deny (false).
    pub allow: bool,
}

impl NetRule {
    /// Create a new network rule with explicit parameters.
    ///
    /// For common cases, use the convenience constructors: [`NetRule::allow`],
    /// [`NetRule::allow_port`], or [`NetRule::deny`].
    ///
    /// # Arguments
    ///
    /// * `host` - Hostname or IP address
    /// * `port` - Port number (None = all ports)
    /// * `allow` - Whether to allow (true) or deny (false) connections
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::NetRule;
    ///
    /// let rule = NetRule::new("api.example.com".to_string(), Some(443), true);
    /// assert_eq!(rule.host, "api.example.com");
    /// assert_eq!(rule.port, Some(443));
    /// assert!(rule.allow);
    /// ```
    pub fn new(host: String, port: Option<u16>, allow: bool) -> Self {
        Self { host, port, allow }
    }

    /// Create an allow rule for a host (all ports).
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::NetRule;
    ///
    /// let rule = NetRule::allow("api.openai.com".to_string());
    /// assert_eq!(rule.host, "api.openai.com");
    /// assert!(rule.allow);
    /// assert_eq!(rule.port, None);
    /// ```
    pub fn allow(host: String) -> Self {
        Self {
            host,
            port: None,
            allow: true,
        }
    }

    /// Create an allow rule for a specific host:port combination.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::NetRule;
    ///
    /// let rule = NetRule::allow_port("localhost".to_string(), 8080);
    /// assert_eq!(rule.host, "localhost");
    /// assert_eq!(rule.port, Some(8080));
    /// assert!(rule.allow);
    /// ```
    pub fn allow_port(host: String, port: u16) -> Self {
        Self {
            host,
            port: Some(port),
            allow: true,
        }
    }

    /// Create a deny rule for a host (all ports).
    ///
    /// # Example
    ///
    /// ```
    /// use claw_pal::NetRule;
    ///
    /// let rule = NetRule::deny("malicious.com".to_string());
    /// assert_eq!(rule.host, "malicious.com");
    /// assert!(!rule.allow);
    /// assert_eq!(rule.port, None);
    /// ```
    pub fn deny(host: String) -> Self {
        Self {
            host,
            port: None,
            allow: false,
        }
    }
}

/// Resource limits for sandboxed processes.
///
/// Defines memory, CPU, file descriptor, and process count limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes (None = unlimited).
    pub max_memory_bytes: Option<u64>,
    /// Maximum CPU usage percentage (0-100, None = unlimited).
    pub max_cpu_percent: Option<u8>,
    /// Maximum number of file descriptors (None = unlimited).
    pub max_file_descriptors: Option<u32>,
    /// Maximum number of processes (None = unlimited).
    pub max_processes: Option<u32>,
}

impl ResourceLimits {
    /// Create unlimited resource limits.
    pub fn unlimited() -> Self {
        Self {
            max_memory_bytes: None,
            max_cpu_percent: None,
            max_file_descriptors: None,
            max_processes: None,
        }
    }

    /// Create restrictive resource limits suitable for untrusted code.
    pub fn restrictive() -> Self {
        Self {
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_cpu_percent: Some(50),
            max_file_descriptors: Some(256),
            max_processes: Some(10),
        }
    }

    /// Set maximum memory.
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.max_memory_bytes = Some(bytes);
        self
    }

    /// Set maximum CPU percentage.
    pub fn with_cpu(mut self, percent: u8) -> Self {
        self.max_cpu_percent = Some(percent.min(100));
        self
    }

    /// Set maximum file descriptors.
    pub fn with_fds(mut self, count: u32) -> Self {
        self.max_file_descriptors = Some(count);
        self
    }

    /// Set maximum processes.
    pub fn with_processes(mut self, count: u32) -> Self {
        self.max_processes = Some(count);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_rule_new() {
        let rule = PathRule::new(PathBuf::from("/data"));
        assert_eq!(rule.path, PathBuf::from("/data"));
        assert!(!rule.read);
        assert!(!rule.write);
        assert!(!rule.execute);
    }

    #[test]
    fn test_path_rule_builder() {
        let rule = PathRule::new(PathBuf::from("/data"))
            .with_read()
            .with_write();
        assert!(rule.read);
        assert!(rule.write);
        assert!(!rule.execute);
    }

    #[test]
    fn test_path_rule_clone() {
        let rule = PathRule::new(PathBuf::from("/data")).with_read();
        let cloned = rule.clone();
        assert_eq!(rule, cloned);
    }

    #[test]
    fn test_path_rule_serialize() {
        let rule = PathRule::new(PathBuf::from("/data")).with_read();
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: PathRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_net_rule_new() {
        let rule = NetRule::new("example.com".to_string(), Some(443), true);
        assert_eq!(rule.host, "example.com");
        assert_eq!(rule.port, Some(443));
        assert!(rule.allow);
    }

    #[test]
    fn test_net_rule_allow() {
        let rule = NetRule::allow("example.com".to_string());
        assert_eq!(rule.host, "example.com");
        assert_eq!(rule.port, None);
        assert!(rule.allow);
    }

    #[test]
    fn test_net_rule_allow_port() {
        let rule = NetRule::allow_port("example.com".to_string(), 443);
        assert_eq!(rule.host, "example.com");
        assert_eq!(rule.port, Some(443));
        assert!(rule.allow);
    }

    #[test]
    fn test_net_rule_deny() {
        let rule = NetRule::deny("malicious.com".to_string());
        assert_eq!(rule.host, "malicious.com");
        assert_eq!(rule.port, None);
        assert!(!rule.allow);
    }

    #[test]
    fn test_net_rule_clone() {
        let rule = NetRule::allow("example.com".to_string());
        let cloned = rule.clone();
        assert_eq!(rule, cloned);
    }

    #[test]
    fn test_net_rule_serialize() {
        let rule = NetRule::allow_port("example.com".to_string(), 443);
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: NetRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_resource_limits_unlimited() {
        let limits = ResourceLimits::unlimited();
        assert_eq!(limits.max_memory_bytes, None);
        assert_eq!(limits.max_cpu_percent, None);
        assert_eq!(limits.max_file_descriptors, None);
        assert_eq!(limits.max_processes, None);
    }

    #[test]
    fn test_resource_limits_restrictive() {
        let limits = ResourceLimits::restrictive();
        assert_eq!(limits.max_memory_bytes, Some(256 * 1024 * 1024));
        assert_eq!(limits.max_cpu_percent, Some(50));
        assert_eq!(limits.max_file_descriptors, Some(256));
        assert_eq!(limits.max_processes, Some(10));
    }

    #[test]
    fn test_resource_limits_builder() {
        let limits = ResourceLimits::unlimited()
            .with_memory(512 * 1024 * 1024)
            .with_cpu(75)
            .with_fds(512)
            .with_processes(20);
        assert_eq!(limits.max_memory_bytes, Some(512 * 1024 * 1024));
        assert_eq!(limits.max_cpu_percent, Some(75));
        assert_eq!(limits.max_file_descriptors, Some(512));
        assert_eq!(limits.max_processes, Some(20));
    }

    #[test]
    fn test_resource_limits_cpu_clamped() {
        let limits = ResourceLimits::unlimited().with_cpu(150);
        assert_eq!(limits.max_cpu_percent, Some(100));
    }

    #[test]
    fn test_resource_limits_clone() {
        let limits = ResourceLimits::restrictive();
        let cloned = limits.clone();
        assert_eq!(limits, cloned);
    }

    #[test]
    fn test_resource_limits_serialize() {
        let limits = ResourceLimits::restrictive();
        let json = serde_json::to_string(&limits).unwrap();
        let deserialized: ResourceLimits = serde_json::from_str(&json).unwrap();
        assert_eq!(limits, deserialized);
    }
}
