//! Message routing — route incoming messages to specific Agents based on rules.
//!
//! [`ChannelRouter`] evaluates a prioritised list of [`RoutingRule`]s and
//! returns the `AgentId` (represented as a `String` inside claw-channel) that
//! should handle the message.
//!
//! # Rule evaluation order
//!
//! Rules are tested in the order they were added.  The first non-Default rule
//! that matches wins.  A [`RoutingRule::Default`] rule is used only when every
//! other rule fails to match — regardless of where in the list it was placed.
//!
//! # Sender-ID matching
//!
//! [`ChannelMessage`] does not carry an explicit `sender_id` field; instead the
//! sender is stored in `metadata["sender_id"]` as a JSON string.
//! [`RoutingRule::BySenderId`] reads that key at match time.
//!
//! # Example
//!
//! ```rust
//! use claw_channel::router::ChannelRouterBuilder;
//!
//! let router = ChannelRouterBuilder::new()
//!     .route_channel("ch-discord", "agent-discord")
//!     .route_pattern("^!admin", "agent-admin").unwrap()
//!     .default_agent("agent-fallback")
//!     .build();
//! ```

use std::path::Path;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use crate::types::ChannelMessage;

// ─── AgentId type alias ────────────────────────────────────────────────────

/// String alias for an agent identifier inside claw-channel.
///
/// When integrating with `claw-runtime`, the upper layer converts this `String`
/// into the runtime's strongly-typed `AgentId`.
pub type AgentId = String;

// ─── Errors ───────────────────────────────────────────────────────────────

/// Errors that can occur while building or using a [`ChannelRouter`].
#[derive(Debug, Error)]
pub enum RouterError {
    /// A regex pattern string was syntactically invalid.
    #[error("Regex parse error: {0}")]
    RegexError(#[from] regex::Error),

    /// An I/O error occurred reading a TOML config file.
    #[error("IO error reading config '{path}': {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The TOML config file could not be parsed.
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// No routing rule matched and no default agent was configured.
    #[error("No rules matched and no default agent configured")]
    NoMatch,
}

// ─── RoutingRule ──────────────────────────────────────────────────────────

/// A single routing rule that maps a message characteristic to an `AgentId`.
pub enum RoutingRule {
    /// All messages arriving on a specific channel → target agent.
    ByChannelId {
        channel_id: String,
        agent_id: AgentId,
    },
    /// Messages where `metadata["sender_id"]` equals a specific string → target agent.
    BySenderId {
        sender_id: String,
        agent_id: AgentId,
    },
    /// Messages whose `content` matches a regular expression → target agent.
    ByPattern {
        pattern: Regex,
        agent_id: AgentId,
    },
    /// Catch-all: used when no other rule matches.
    Default { agent_id: AgentId },
}

impl RoutingRule {
    /// Test the rule against `msg`.
    ///
    /// Returns `Some(&agent_id)` if this rule matches, `None` otherwise.
    /// Note: [`RoutingRule::Default`] always returns `Some`; callers should
    /// handle it separately (see [`ChannelRouter::route`]).
    pub fn matches(&self, msg: &ChannelMessage) -> Option<&AgentId> {
        match self {
            RoutingRule::ByChannelId {
                channel_id,
                agent_id,
            } => {
                if msg.channel_id.as_str() == channel_id.as_str() {
                    Some(agent_id)
                } else {
                    None
                }
            }

            RoutingRule::BySenderId {
                sender_id,
                agent_id,
            } => {
                // sender_id is stored in metadata["sender_id"] as a JSON string.
                let meta_sender = msg
                    .metadata
                    .get("sender_id")
                    .and_then(|v| v.as_str());
                match meta_sender {
                    Some(s) if s == sender_id.as_str() => Some(agent_id),
                    _ => None,
                }
            }

            RoutingRule::ByPattern { pattern, agent_id } => {
                if pattern.is_match(&msg.content) {
                    Some(agent_id)
                } else {
                    None
                }
            }

            RoutingRule::Default { agent_id } => Some(agent_id),
        }
    }
}

// ─── TOML config structures ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TomlConfig {
    #[serde(default)]
    rules: Vec<TomlRule>,
    default: Option<TomlDefault>,
}

#[derive(Debug, Deserialize)]
struct TomlRule {
    #[serde(rename = "type")]
    rule_type: String,
    channel_id: Option<String>,
    sender_id: Option<String>,
    pattern: Option<String>,
    agent_id: String,
}

#[derive(Debug, Deserialize)]
struct TomlDefault {
    agent_id: String,
}

// ─── ChannelRouter ────────────────────────────────────────────────────────

/// Serializable representation of a routing rule (for IPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRuleSpec {
    /// Rule type: "channel" | "sender" | "pattern" | "default"
    pub rule_type: String,
    /// Channel ID (for "channel" rules).
    pub channel_id: Option<String>,
    /// Sender ID (for "sender" rules).
    pub sender_id: Option<String>,
    /// Regex pattern string (for "pattern" rules).
    pub pattern: Option<String>,
    /// Target agent ID.
    pub agent_id: String,
}

/// Routes [`ChannelMessage`]s to the appropriate agent.
///
/// Constructed via [`ChannelRouterBuilder`].
pub struct ChannelRouter {
    rules: Arc<std::sync::RwLock<Vec<RoutingRule>>>,
}

impl ChannelRouter {
    /// Create a new [`ChannelRouterBuilder`].
    pub fn builder() -> ChannelRouterBuilder {
        ChannelRouterBuilder::new()
    }

    /// Determine which agent should handle `msg`.
    ///
    /// Rules are tested in insertion order.  The first non-Default rule that
    /// matches is returned.  If no non-Default rule matches, the Default rule
    /// (if any) is returned.  Returns `None` if nothing matched.
    ///
    /// Returns `Option<String>` (owned) to avoid lifetime issues with the
    /// internal `RwLock`.
    pub fn route(&self, msg: &ChannelMessage) -> Option<String> {
        let rules = match self.rules.read() {
            Ok(r) => r,
            Err(_) => return None,
        };
        let mut default_agent: Option<String> = None;

        for rule in rules.iter() {
            match rule {
                RoutingRule::Default { agent_id } => {
                    // Remember the default but keep evaluating other rules.
                    default_agent = Some(agent_id.clone());
                }
                other => {
                    if let Some(agent_id) = other.matches(msg) {
                        debug!(
                            channel = %msg.channel_id,
                            agent  = %agent_id,
                            "Message routed by explicit rule"
                        );
                        return Some(agent_id.clone());
                    }
                }
            }
        }

        if let Some(agent_id) = default_agent {
            debug!(
                channel = %msg.channel_id,
                agent  = %agent_id,
                "Message routed to default agent"
            );
            return Some(agent_id);
        }

        warn!(
            channel = %msg.channel_id,
            "No routing rule matched — message will be dropped"
        );
        None
    }

    /// Return the number of routing rules (including any Default rule).
    pub fn rule_count(&self) -> usize {
        self.rules.read().map(|r| r.len()).unwrap_or(0)
    }

    /// Add a routing rule dynamically at runtime.
    pub fn add_rule(&self, rule: RoutingRule) {
        if let Ok(mut rules) = self.rules.write() {
            rules.push(rule);
        }
    }

    /// Remove all routing rules targeting a specific agent.
    ///
    /// Returns the number of rules removed.
    pub fn remove_rules_for_agent(&self, agent_id: &str) -> usize {
        if let Ok(mut rules) = self.rules.write() {
            let before = rules.len();
            rules.retain(|r| match r {
                RoutingRule::ByChannelId { agent_id: a, .. } => a != agent_id,
                RoutingRule::BySenderId { agent_id: a, .. } => a != agent_id,
                RoutingRule::ByPattern { agent_id: a, .. } => a != agent_id,
                RoutingRule::Default { agent_id: a } => a != agent_id,
            });
            before - rules.len()
        } else {
            0
        }
    }

    /// Return a snapshot of current rules as serializable specs.
    pub fn list_rules(&self) -> Vec<RoutingRuleSpec> {
        let rules = match self.rules.read() {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        rules.iter().map(|r| match r {
            RoutingRule::ByChannelId { channel_id, agent_id } => RoutingRuleSpec {
                rule_type: "channel".to_string(),
                channel_id: Some(channel_id.clone()),
                sender_id: None,
                pattern: None,
                agent_id: agent_id.clone(),
            },
            RoutingRule::BySenderId { sender_id, agent_id } => RoutingRuleSpec {
                rule_type: "sender".to_string(),
                channel_id: None,
                sender_id: Some(sender_id.clone()),
                pattern: None,
                agent_id: agent_id.clone(),
            },
            RoutingRule::ByPattern { pattern, agent_id } => RoutingRuleSpec {
                rule_type: "pattern".to_string(),
                channel_id: None,
                sender_id: None,
                pattern: Some(pattern.as_str().to_string()),
                agent_id: agent_id.clone(),
            },
            RoutingRule::Default { agent_id } => RoutingRuleSpec {
                rule_type: "default".to_string(),
                channel_id: None,
                sender_id: None,
                pattern: None,
                agent_id: agent_id.clone(),
            },
        }).collect()
    }
}

// ─── ChannelRouterBuilder ─────────────────────────────────────────────────

/// Builder for [`ChannelRouter`].
///
/// Rules are evaluated in the order they are added.
pub struct ChannelRouterBuilder {
    rules: Vec<RoutingRule>,
}

impl ChannelRouterBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Route all messages from `channel_id` to `agent_id`.
    pub fn route_channel(
        mut self,
        channel_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        self.rules.push(RoutingRule::ByChannelId {
            channel_id: channel_id.into(),
            agent_id: agent_id.into(),
        });
        self
    }

    /// Route messages where `metadata["sender_id"]` equals `sender_id` to `agent_id`.
    pub fn route_sender(
        mut self,
        sender_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        self.rules.push(RoutingRule::BySenderId {
            sender_id: sender_id.into(),
            agent_id: agent_id.into(),
        });
        self
    }

    /// Route messages whose content matches the regular expression `pattern` to `agent_id`.
    ///
    /// Returns [`RouterError::RegexError`] if the pattern is syntactically invalid.
    pub fn route_pattern(
        mut self,
        pattern: &str,
        agent_id: impl Into<String>,
    ) -> Result<Self, RouterError> {
        let regex = Regex::new(pattern)?;
        self.rules.push(RoutingRule::ByPattern {
            pattern: regex,
            agent_id: agent_id.into(),
        });
        Ok(self)
    }

    /// Set the catch-all default agent used when no explicit rule matches.
    pub fn default_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.rules.push(RoutingRule::Default {
            agent_id: agent_id.into(),
        });
        self
    }

    /// Load routing rules from a TOML file.
    ///
    /// See [`from_toml_str`][Self::from_toml_str] for the expected format.
    pub fn from_toml_file(path: &Path) -> Result<Self, RouterError> {
        let content = std::fs::read_to_string(path).map_err(|e| RouterError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_toml_str(&content)
    }

    /// Load routing rules from a TOML string.
    ///
    /// # Expected TOML format
    ///
    /// ```toml
    /// [[rules]]
    /// type     = "channel"
    /// channel_id = "ch-discord"
    /// agent_id = "agent-discord"
    ///
    /// [[rules]]
    /// type    = "sender"
    /// sender_id = "user-42"
    /// agent_id  = "agent-vip"
    ///
    /// [[rules]]
    /// type     = "pattern"
    /// pattern  = "^!admin"
    /// agent_id = "agent-admin"
    ///
    /// [default]
    /// agent_id = "agent-fallback"
    /// ```
    pub fn from_toml_str(s: &str) -> Result<Self, RouterError> {
        let config: TomlConfig = toml::from_str(s)?;
        let mut builder = Self::new();

        for rule in config.rules {
            builder = match rule.rule_type.as_str() {
                "channel" => {
                    let channel_id = rule.channel_id.unwrap_or_default();
                    builder.route_channel(channel_id, rule.agent_id)
                }
                "sender" => {
                    let sender_id = rule.sender_id.unwrap_or_default();
                    builder.route_sender(sender_id, rule.agent_id)
                }
                "pattern" => {
                    let pattern = rule.pattern.unwrap_or_default();
                    builder.route_pattern(&pattern, rule.agent_id)?
                }
                unknown => {
                    warn!(rule_type = %unknown, "Unknown routing rule type, skipping");
                    builder
                }
            };
        }

        if let Some(default) = config.default {
            builder = builder.default_agent(default.agent_id);
        }

        Ok(builder)
    }

    /// Consume the builder and produce a [`ChannelRouter`].
    pub fn build(self) -> ChannelRouter {
        ChannelRouter { rules: Arc::new(std::sync::RwLock::new(self.rules)) }
    }
}

impl Default for ChannelRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelId, ChannelMessage, Platform};

    fn make_msg(channel: &str, content: &str) -> ChannelMessage {
        ChannelMessage::inbound(ChannelId::new(channel), Platform::Stdin, content)
    }

    fn make_msg_with_sender(channel: &str, content: &str, sender: &str) -> ChannelMessage {
        let mut msg = make_msg(channel, content);
        msg.metadata = serde_json::json!({ "sender_id": sender });
        msg
    }

    // ── route_channel ────────────────────────────────────────────────────

    #[test]
    fn test_route_by_channel_id() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .build();

        let msg = make_msg("ch-discord", "hello");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-discord"));
    }

    #[test]
    fn test_route_channel_no_match() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .build();

        let msg = make_msg("ch-other", "hello");
        assert!(router.route(&msg).is_none());
    }

    // ── route_sender ─────────────────────────────────────────────────────

    #[test]
    fn test_route_by_sender_id() {
        let router = ChannelRouterBuilder::new()
            .route_sender("user-42", "agent-vip")
            .build();

        let msg = make_msg_with_sender("ch-1", "ping", "user-42");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-vip"));
    }

    #[test]
    fn test_route_sender_missing_metadata() {
        let router = ChannelRouterBuilder::new()
            .route_sender("user-42", "agent-vip")
            .build();

        // No metadata at all → should not match
        let msg = make_msg("ch-1", "ping");
        assert!(router.route(&msg).is_none());
    }

    // ── route_pattern ────────────────────────────────────────────────────

    #[test]
    fn test_route_by_pattern() {
        let router = ChannelRouterBuilder::new()
            .route_pattern("^!admin", "agent-admin")
            .unwrap()
            .build();

        let msg = make_msg("ch-1", "!admin kick user");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-admin"));
    }

    #[test]
    fn test_route_pattern_no_match() {
        let router = ChannelRouterBuilder::new()
            .route_pattern("^!admin", "agent-admin")
            .unwrap()
            .build();

        let msg = make_msg("ch-1", "hello world");
        assert!(router.route(&msg).is_none());
    }

    #[test]
    fn test_route_pattern_invalid_regex() {
        let result = ChannelRouterBuilder::new().route_pattern("[invalid", "agent");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(matches!(err, RouterError::RegexError(_)));
    }

    // ── default_agent ────────────────────────────────────────────────────

    #[test]
    fn test_default_agent_fallback() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .default_agent("agent-fallback")
            .build();

        // No explicit rule matches "ch-other"
        let msg = make_msg("ch-other", "hello");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-fallback"));
    }

    #[test]
    fn test_explicit_rule_beats_default() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .default_agent("agent-fallback")
            .build();

        let msg = make_msg("ch-discord", "hello");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-discord"));
    }

    #[test]
    fn test_no_match_no_default() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .build();

        let msg = make_msg("ch-other", "hello");
        assert!(router.route(&msg).is_none());
    }

    // ── rule_count ───────────────────────────────────────────────────────

    #[test]
    fn test_rule_count() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-1", "a1")
            .route_sender("u1", "a2")
            .route_pattern(".*", "a3")
            .unwrap()
            .default_agent("a4")
            .build();
        assert_eq!(router.rule_count(), 4);
    }

    // ── first-match-wins ordering ─────────────────────────────────────────

    #[test]
    fn test_first_match_wins() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-1", "agent-first")
            .route_channel("ch-1", "agent-second")
            .build();

        let msg = make_msg("ch-1", "hello");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-first"));
    }

    // ── TOML loading ──────────────────────────────────────────────────────

    #[test]
    fn test_from_toml_str_channel_rule() {
        let toml = r#"
[[rules]]
type       = "channel"
channel_id = "ch-webhook"
agent_id   = "agent-wh"

[default]
agent_id = "agent-catch"
"#;
        let router = ChannelRouterBuilder::from_toml_str(toml).unwrap().build();
        assert_eq!(router.rule_count(), 2); // 1 channel rule + 1 default

        let msg = make_msg("ch-webhook", "event");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-wh"));

        let other = make_msg("ch-other", "event");
        assert_eq!(router.route(&other).as_deref(), Some("agent-catch"));
    }

    #[test]
    fn test_from_toml_str_sender_rule() {
        let toml = r#"
[[rules]]
type      = "sender"
sender_id = "bot-123"
agent_id  = "agent-bot"
"#;
        let router = ChannelRouterBuilder::from_toml_str(toml).unwrap().build();

        let msg = make_msg_with_sender("ch-1", "hi", "bot-123");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-bot"));
    }

    #[test]
    fn test_from_toml_str_pattern_rule() {
        let toml = r#"
[[rules]]
type     = "pattern"
pattern  = "^ALERT"
agent_id = "agent-alert"
"#;
        let router = ChannelRouterBuilder::from_toml_str(toml).unwrap().build();

        let msg = make_msg("ch-1", "ALERT: disk full");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-alert"));
    }

    #[test]
    fn test_from_toml_str_unknown_rule_skipped() {
        let toml = r#"
[[rules]]
type     = "unknown_type"
agent_id = "agent-x"

[default]
agent_id = "agent-default"
"#;
        // Should parse without error, unknown rule is skipped
        let router = ChannelRouterBuilder::from_toml_str(toml).unwrap().build();
        let msg = make_msg("ch-1", "anything");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-default"));
    }

    #[test]
    fn test_from_toml_str_invalid_toml() {
        let toml = "this is not valid toml @@##";
        let result = ChannelRouterBuilder::from_toml_str(toml);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(matches!(err, RouterError::TomlParse(_)));
    }

    #[test]
    fn test_default_builder_impl() {
        let builder = ChannelRouterBuilder::default();
        let router = builder.build();
        assert_eq!(router.rule_count(), 0);
    }
}
