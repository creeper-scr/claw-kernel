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
//! [`ChannelMessage`] carries a top-level `sender_id: Option<String>` field.
//! [`RoutingRule::BySenderId`] compares that field directly — no JSON look-up
//! required.
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};
use dashmap::DashMap;

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
    /// Messages where `sender_id` equals a specific string → target agent.
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
            } => match msg.sender_id.as_deref() {
                Some(s) if s == sender_id.as_str() => Some(agent_id),
                _ => None,
            },

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

    /// Return all agent IDs that match `msg` under "multi-match" semantics.
    ///
    /// Unlike [`route`][Self::route] which stops at the first matching rule
    /// (first-match-wins), this method collects **all** matching agents so that
    /// the same message can be fan-out delivered to multiple agents — e.g. a
    /// primary handler *and* a monitoring/audit agent.
    ///
    /// # Default rule handling
    ///
    /// A [`RoutingRule::Default`] rule is appended only when **no** explicit
    /// rule matched, preventing the default agent from receiving duplicate
    /// copies in cases where explicit rules already covered the message.
    ///
    /// # Duplicate suppression
    ///
    /// If multiple rules target the same `agent_id`, it appears only once in
    /// the returned `Vec`.
    pub fn broadcast_route(&self, msg: &ChannelMessage) -> Vec<String> {
        let rules = match self.rules.read() {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        let mut agents: Vec<String> = vec![];
        let mut has_explicit_match = false;
        let mut default_agent: Option<String> = None;

        for rule in rules.iter() {
            match rule {
                RoutingRule::Default { agent_id } => {
                    // Remember the default; it is only used when no explicit rule fires.
                    default_agent = Some(agent_id.clone());
                }
                other => {
                    if let Some(agent_id) = other.matches(msg) {
                        if !agents.contains(agent_id) {
                            agents.push(agent_id.clone());
                        }
                        has_explicit_match = true;
                    }
                }
            }
        }

        // Default only fires when nothing else matched.
        if !has_explicit_match {
            if let Some(agent_id) = default_agent {
                agents.push(agent_id);
            }
        }

        if !agents.is_empty() {
            debug!(
                channel = %msg.channel_id,
                agents  = ?agents,
                "Message broadcast-routed"
            );
        } else {
            warn!(
                channel = %msg.channel_id,
                "broadcast_route: no rules matched — message will be dropped"
            );
        }

        agents
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

    /// Route messages where `sender_id` equals `sender_id` to `agent_id`.
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

// ─── DeduplicatingRouter ──────────────────────────────────────────────────

/// Default TTL for seen message IDs.
const DEFAULT_DEDUP_TTL: Duration = Duration::from_secs(60);

/// How many `route` / `broadcast_route` calls between lazy GC sweeps.
const DEFAULT_GC_INTERVAL: usize = 64;

/// A wrapper around [`ChannelRouter`] that silently drops duplicate messages.
///
/// Deduplication is keyed on [`ChannelMessage::id`].  If the same `id` arrives
/// within [`ttl`][DeduplicatingRouter::ttl] (default 60 s) of its first
/// occurrence, the duplicate is discarded:
///
/// - [`route`][DeduplicatingRouter::route] returns `None`
/// - [`broadcast_route`][DeduplicatingRouter::broadcast_route] returns an empty
///   `Vec`
///
/// Expired entries are removed lazily every `gc_interval` calls (default 64)
/// to avoid accumulating unbounded state under low-traffic conditions.
///
/// # Example
///
/// ```rust
/// use claw_channel::router::{ChannelRouterBuilder, DeduplicatingRouter};
///
/// let inner = ChannelRouterBuilder::new()
///     .default_agent("agent-a")
///     .build();
/// let router = DeduplicatingRouter::new(inner);
/// ```
pub struct DeduplicatingRouter {
    inner: ChannelRouter,
    seen: DashMap<String, Instant>,
    ttl: Duration,
    call_count: AtomicUsize,
    gc_interval: usize,
}

impl DeduplicatingRouter {
    /// Wrap `inner` with the default 60 s TTL and GC interval of 64 calls.
    pub fn new(inner: ChannelRouter) -> Self {
        Self {
            inner,
            seen: DashMap::new(),
            ttl: DEFAULT_DEDUP_TTL,
            call_count: AtomicUsize::new(0),
            gc_interval: DEFAULT_GC_INTERVAL,
        }
    }

    /// Override the deduplication TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Override the lazy-GC interval (number of calls between sweeps).
    pub fn with_gc_interval(mut self, n: usize) -> Self {
        self.gc_interval = n;
        self
    }

    /// Check whether `id` is a duplicate and update the seen map.
    ///
    /// Returns `true` when the message is a **duplicate** (should be dropped).
    fn is_duplicate(&self, id: &str) -> bool {
        let now = Instant::now();

        // Trigger lazy GC every `gc_interval` calls.
        let prev = self.call_count.fetch_add(1, Ordering::Relaxed);
        if prev % self.gc_interval == 0 {
            let ttl = self.ttl;
            self.seen.retain(|_, t| now.duration_since(*t) < ttl);
        }

        if let Some(t) = self.seen.get(id) {
            if now.duration_since(*t) < self.ttl {
                return true; // duplicate within TTL window
            }
        }

        // First occurrence (or TTL expired) — record now and allow through.
        self.seen.insert(id.to_string(), now);
        false
    }

    /// Route `msg` to the appropriate agent, discarding duplicates.
    ///
    /// Returns `None` when the message is a duplicate **or** no rule matched.
    pub fn route(&self, msg: &ChannelMessage) -> Option<String> {
        if self.is_duplicate(&msg.id) {
            debug!(msg_id = %msg.id, "DeduplicatingRouter: dropping duplicate message");
            return None;
        }
        self.inner.route(msg)
    }

    /// Fan-out route `msg` to all matching agents, discarding duplicates.
    ///
    /// Returns an empty `Vec` when the message is a duplicate.
    pub fn broadcast_route(&self, msg: &ChannelMessage) -> Vec<String> {
        if self.is_duplicate(&msg.id) {
            debug!(msg_id = %msg.id, "DeduplicatingRouter: dropping duplicate (broadcast)");
            return vec![];
        }
        self.inner.broadcast_route(msg)
    }

    /// Number of unique message IDs currently tracked (including expired ones
    /// that have not yet been swept by the lazy GC).
    pub fn seen_count(&self) -> usize {
        self.seen.len()
    }

    /// Forward to the inner router for rule management.
    pub fn inner(&self) -> &ChannelRouter {
        &self.inner
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod dedup_tests {
    use super::*;
    use crate::types::{ChannelId, ChannelMessage, Platform};

    fn make_msg(id: &str) -> ChannelMessage {
        let mut msg =
            ChannelMessage::inbound(ChannelId::new("ch-1"), Platform::Stdin, "hello");
        msg.id = id.to_string();
        msg
    }

    fn router_with_default() -> DeduplicatingRouter {
        let inner = ChannelRouterBuilder::new()
            .default_agent("agent-a")
            .build();
        DeduplicatingRouter::new(inner)
    }

    #[test]
    fn test_first_message_routed() {
        let router = router_with_default();
        let msg = make_msg("msg-1");
        assert_eq!(router.route(&msg).as_deref(), Some("agent-a"));
    }

    #[test]
    fn test_duplicate_within_ttl_dropped() {
        let router = router_with_default();
        let msg = make_msg("msg-dup");
        // First delivery — should be routed.
        assert_eq!(router.route(&msg).as_deref(), Some("agent-a"));
        // Second delivery of the same ID within TTL — should be dropped.
        assert!(router.route(&msg).is_none());
    }

    #[test]
    fn test_different_ids_both_routed() {
        let router = router_with_default();
        assert_eq!(router.route(&make_msg("a")).as_deref(), Some("agent-a"));
        assert_eq!(router.route(&make_msg("b")).as_deref(), Some("agent-a"));
    }

    #[test]
    fn test_expired_message_re_routed() {
        let inner = ChannelRouterBuilder::new()
            .default_agent("agent-a")
            .build();
        // Very short TTL so we can expire it immediately.
        let router = DeduplicatingRouter::new(inner).with_ttl(Duration::from_nanos(1));

        let msg = make_msg("msg-old");
        router.route(&msg); // first pass — enters seen
        // Sleep just enough for TTL to expire (1 ns is already past at the next call).
        std::thread::sleep(Duration::from_millis(1));
        // Second pass — TTL has expired; should be re-routed.
        assert_eq!(router.route(&msg).as_deref(), Some("agent-a"));
    }

    #[test]
    fn test_broadcast_duplicate_dropped() {
        let router = router_with_default();
        let msg = make_msg("bc-dup");
        assert_eq!(router.broadcast_route(&msg), vec!["agent-a"]);
        assert!(router.broadcast_route(&msg).is_empty());
    }

    #[test]
    fn test_seen_count_increments() {
        let router = router_with_default();
        router.route(&make_msg("x1"));
        router.route(&make_msg("x2"));
        // Each unique ID adds one entry.
        assert_eq!(router.seen_count(), 2);
    }

    #[test]
    fn test_gc_removes_expired_entries() {
        let inner = ChannelRouterBuilder::new()
            .default_agent("agent-a")
            .build();
        // Short TTL + GC fires every 1 call so we can observe the cleanup.
        let router = DeduplicatingRouter::new(inner)
            .with_ttl(Duration::from_nanos(1))
            .with_gc_interval(1);

        router.route(&make_msg("gc-1"));
        std::thread::sleep(Duration::from_millis(1));
        // This call triggers GC which should evict the expired "gc-1" entry,
        // then re-insert it as fresh.
        router.route(&make_msg("gc-1"));
        // After re-insertion, seen_count == 1 (not growing unbounded).
        assert_eq!(router.seen_count(), 1);
    }

    #[test]
    fn test_with_ttl_builder() {
        let inner = ChannelRouterBuilder::new().build();
        let router = DeduplicatingRouter::new(inner).with_ttl(Duration::from_secs(30));
        assert_eq!(router.ttl, Duration::from_secs(30));
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelId, ChannelMessage, Platform};

    fn make_msg(channel: &str, content: &str) -> ChannelMessage {
        ChannelMessage::inbound(ChannelId::new(channel), Platform::Stdin, content)
    }

    fn make_msg_with_sender(channel: &str, content: &str, sender: &str) -> ChannelMessage {
        let mut msg = make_msg(channel, content);
        msg.sender_id = Some(sender.to_string());
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

        // No sender_id set at all → should not match
        let msg = make_msg("ch-1", "ping");
        assert!(router.route(&msg).is_none());
    }

    #[test]
    fn test_route_by_sender_id_uses_top_level_field() {
        // Ensure that metadata["sender_id"] alone does NOT satisfy BySenderId;
        // only the top-level field matters.
        let router = ChannelRouterBuilder::new()
            .route_sender("user-42", "agent-vip")
            .build();

        let mut msg = make_msg("ch-1", "ping");
        // Deliberately put it only in metadata (old behaviour) — must not match.
        msg.metadata = serde_json::json!({ "sender_id": "user-42" });
        assert!(router.route(&msg).is_none());

        // Now set the top-level field — must match.
        msg.sender_id = Some("user-42".to_string());
        assert_eq!(router.route(&msg).as_deref(), Some("agent-vip"));
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

    // ── broadcast_route ──────────────────────────────────────────────────────

    #[test]
    fn test_broadcast_route_collects_all_matches() {
        // Two rules that both match the same message → both agents returned.
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-events", "agent-primary")
            .route_channel("ch-events", "agent-monitor")
            .build();

        let msg = make_msg("ch-events", "hello");
        let mut agents = router.broadcast_route(&msg);
        agents.sort();
        assert_eq!(agents, vec!["agent-monitor", "agent-primary"]);
    }

    #[test]
    fn test_broadcast_route_no_duplicate_agents() {
        // A channel rule and a pattern rule both point to the same agent.
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-1", "agent-a")
            .route_pattern("hello", "agent-a")
            .unwrap()
            .build();

        let msg = make_msg("ch-1", "hello");
        let agents = router.broadcast_route(&msg);
        // agent-a should appear only once even though two rules matched.
        assert_eq!(agents, vec!["agent-a"]);
    }

    #[test]
    fn test_broadcast_route_default_fires_when_no_explicit_match() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .default_agent("agent-fallback")
            .build();

        let msg = make_msg("ch-other", "hello");
        let agents = router.broadcast_route(&msg);
        assert_eq!(agents, vec!["agent-fallback"]);
    }

    #[test]
    fn test_broadcast_route_default_suppressed_when_explicit_match() {
        // Explicit rule fires → default should NOT be appended.
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .default_agent("agent-fallback")
            .build();

        let msg = make_msg("ch-discord", "hello");
        let agents = router.broadcast_route(&msg);
        assert_eq!(agents, vec!["agent-discord"]);
    }

    #[test]
    fn test_broadcast_route_no_match_no_default_returns_empty() {
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-discord", "agent-discord")
            .build();

        let msg = make_msg("ch-other", "hello");
        let agents = router.broadcast_route(&msg);
        assert!(agents.is_empty());
    }

    #[test]
    fn test_broadcast_route_fan_out_mixed_rules() {
        // channel rule + pattern rule + sender rule — all fire for the same msg.
        let router = ChannelRouterBuilder::new()
            .route_channel("ch-alerts", "agent-channel")
            .route_pattern("^ALERT", "agent-pattern")
            .unwrap()
            .route_sender("bot-1", "agent-sender")
            .default_agent("agent-default")
            .build();

        let msg = make_msg_with_sender("ch-alerts", "ALERT: disk full", "bot-1");
        let mut agents = router.broadcast_route(&msg);
        agents.sort();
        // All three explicit rules match → default suppressed.
        assert_eq!(
            agents,
            vec!["agent-channel", "agent-pattern", "agent-sender"]
        );
    }
}
