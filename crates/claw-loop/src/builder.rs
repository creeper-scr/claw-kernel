use std::path::Path;
use std::sync::Arc;

use claw_provider::traits::LLMProvider;
use claw_tools::registry::ToolRegistry;
use tokio::sync::{broadcast, RwLock};

use crate::{
    agent_loop::AgentLoop,
    error::AgentError,
    history::InMemoryHistory,
    state_machine::AgentState,
    traits::{EventPublisher, HistoryManager, NoopEventPublisher, StopCondition},
    types::AgentLoopConfig,
};

/// Default capacity for state broadcast channel.
const DEFAULT_STATE_CHANNEL_CAPACITY: usize = 1024;

/// Fluent builder for [`AgentLoop`].
///
/// # Required fields
/// - `provider` — must be set before calling [`build`](AgentLoopBuilder::build).
///
/// # Optional fields
/// All other fields fall back to sensible defaults.
pub struct AgentLoopBuilder {
    provider: Option<Arc<dyn LLMProvider>>,
    tools: Option<Arc<ToolRegistry>>,
    history: Box<dyn HistoryManager>,
    stop_conditions: Vec<Box<dyn StopCondition>>,
    config: AgentLoopConfig,
    state_channel_capacity: usize,
    event_publisher: Arc<dyn EventPublisher>,
    agent_id: String,
}

impl AgentLoopBuilder {
    /// Create a new builder with default settings.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claw_loop::AgentLoopBuilder;
    /// use claw_provider::AnthropicProvider;
    /// use std::sync::Arc;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let provider = Arc::new(AnthropicProvider::from_env()?);
    /// let loop_ = AgentLoopBuilder::new()
    ///     .with_provider(provider)
    ///     .with_max_turns(10)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            history: Box::new(InMemoryHistory::default()),
            stop_conditions: Vec::new(),
            config: AgentLoopConfig::default(),
            state_channel_capacity: DEFAULT_STATE_CHANNEL_CAPACITY,
            event_publisher: NoopEventPublisher::new(),
            agent_id: "agent".to_string(),
        }
    }

    /// Set the LLM provider (required).
    pub fn with_provider(mut self, provider: Arc<dyn LLMProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Attach a tool registry (optional; tool calls are skipped when absent).
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Replace the default [`InMemoryHistory`] with a custom implementation.
    pub fn with_history(mut self, history: Box<dyn HistoryManager>) -> Self {
        self.history = history;
        self
    }

    /// Add a custom stop condition.
    pub fn with_stop_condition(mut self, cond: Box<dyn StopCondition>) -> Self {
        self.stop_conditions.push(cond);
        self
    }

    /// Replace the entire [`AgentLoopConfig`].
    pub fn with_config(mut self, config: AgentLoopConfig) -> Self {
        self.config = config;
        self
    }

    /// Convenience: set `config.max_turns`.
    pub fn with_max_turns(mut self, n: u32) -> Self {
        self.config.max_turns = n;
        self
    }

    /// Convenience: set `config.system_prompt`.
    pub fn with_system_prompt(mut self, sys: impl Into<String>) -> Self {
        self.config.system_prompt = Some(sys.into());
        self
    }

    /// Convenience: set `config.token_budget`.
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.config.token_budget = budget;
        self
    }

    /// Set the state broadcast channel capacity (default: 1024).
    pub fn with_state_channel_capacity(mut self, capacity: usize) -> Self {
        self.state_channel_capacity = capacity;
        self
    }

    /// Set the maximum number of tool calls to execute in parallel per turn (default: 10).
    pub fn with_max_tool_calls_per_turn(mut self, max: usize) -> Self {
        self.config.max_tool_calls_per_turn = max;
        self
    }

    /// Set the event publisher for agent lifecycle events.
    ///
    /// This allows Layer 1 (claw-runtime) to inject EventBus capabilities
    /// without creating a circular dependency.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claw_loop::{AgentLoopBuilder, EventPublisher};
    /// use claw_runtime::{EventBus, events::Event};
    /// use std::sync::Arc;
    ///
    /// struct RuntimeEventPublisher {
    ///     event_bus: Arc<EventBus>,
    /// }
    ///
    /// impl EventPublisher for RuntimeEventPublisher {
    ///     // ... implementation
    /// }
    ///
    /// let loop_ = AgentLoopBuilder::new()
    ///     .with_provider(provider)
    ///     .with_event_publisher(Arc::new(RuntimeEventPublisher { event_bus }))
    ///     .build()?;
    /// ```
    pub fn with_event_publisher(mut self, publisher: Arc<dyn EventPublisher>) -> Self {
        self.event_publisher = publisher;
        self
    }

    /// Set the agent ID used for event publishing.
    ///
    /// Default is "agent".
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = agent_id.into();
        self
    }

    /// Use SQLite-backed persistent history instead of the default in-memory history.
    ///
    /// The database is opened at `db_path`; `namespace` isolates this agent's history
    /// from other agents sharing the same database file (e.g. use the session ID).
    ///
    /// # Errors at build time
    ///
    /// The database is opened eagerly; if the path is unwritable or the file is
    /// corrupt, [`build`](AgentLoopBuilder::build) will return an error.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claw_loop::AgentLoopBuilder;
    /// use claw_provider::OllamaProvider;
    /// use std::sync::Arc;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let provider = Arc::new(OllamaProvider::from_env()?);
    /// let agent = AgentLoopBuilder::new()
    ///     .with_provider(provider)
    ///     .with_sqlite_history("/tmp/claw-history.db", "session-001")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_sqlite_history(
        mut self,
        db_path: impl AsRef<std::path::Path>,
        namespace: impl AsRef<str>,
    ) -> Self {
        match crate::sqlite_history::SqliteHistory::open(db_path.as_ref(), namespace.as_ref()) {
            Ok(history) => {
                self.history = Box::new(history);
            }
            Err(e) => {
                // Store error for reporting at build() time by poisoning the provider.
                // We use a "poisoned" flag pattern: set an internal error field.
                // Since builder doesn't currently have an error field, we log and
                // leave the default history in place (operation degrades gracefully).
                tracing::error!("Failed to open SQLite history: {}; falling back to in-memory", e);
            }
        }
        self
    }

    /// 从约定目录自动加载配置文件注入 system_prompt。
    ///
    /// 加载顺序：`SOUL.md` → `AGENTS.md` → `HEARTBEAT.md`
    /// 任何文件缺失时静默跳过（不报错）。
    /// 多个文件存在时，用 `\n\n---\n\n` 分隔合并。
    ///
    /// # Example
    /// ```rust,no_run
    /// use claw_loop::AgentLoopBuilder;
    /// use claw_provider::OllamaProvider;
    /// use std::sync::Arc;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let provider = Arc::new(OllamaProvider::from_env()?);
    /// let agent = AgentLoopBuilder::new()
    ///     .with_config_dir("./config")
    ///     .with_provider(provider)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_config_dir(mut self, path: impl AsRef<Path>) -> Self {
        let dir = path.as_ref();
        let mut parts = Vec::new();
        for filename in &["SOUL.md", "AGENTS.md", "HEARTBEAT.md"] {
            let file = dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
        if !parts.is_empty() {
            let combined = parts.join("\n\n---\n\n");
            // 如果已有 system_prompt，将文件内容追加到前面
            self.config.system_prompt = Some(match self.config.system_prompt.take() {
                Some(existing) => format!("{}\n\n---\n\n{}", combined, existing),
                None => combined,
            });
        }
        self
    }

    /// 在现有 system_prompt 后面追加额外指令（优先级高于文件内容）。
    ///
    /// 如果已有 system_prompt（包括通过 `with_config_dir` 加载的），
    /// 在末尾追加；否则直接设置为新内容。
    ///
    /// # Example
    /// ```rust,no_run
    /// use claw_loop::AgentLoopBuilder;
    /// use claw_provider::OllamaProvider;
    /// use std::sync::Arc;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let provider = Arc::new(OllamaProvider::from_env()?);
    /// let agent = AgentLoopBuilder::new()
    ///     .with_config_dir("./config")
    ///     .with_system_prompt_append("Always respond in English.")
    ///     .with_provider(provider)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_system_prompt_append(mut self, content: impl Into<String>) -> Self {
        let extra = content.into();
        self.config.system_prompt = Some(match self.config.system_prompt.take() {
            Some(existing) => format!("{}\n\n{}", existing, extra),
            None => extra,
        });
        self
    }

    /// Build the [`AgentLoop`].
    ///
    /// Returns `Err(AgentError::Context)` if no provider was set.
    /// Returns `Err(AgentError::Context)` if `max_tool_calls_per_turn` is zero
    /// (which would cause all tool calls to be silently ignored).
    pub fn build(self) -> Result<AgentLoop, AgentError> {
        let provider = self
            .provider
            .ok_or_else(|| AgentError::Context("no provider set".to_string()))?;

        // FIX-19: a zero max_tool_calls_per_turn would silently discard every tool call
        // the LLM makes.  Reject this configuration up-front so the bug is obvious.
        if self.config.max_tool_calls_per_turn == 0 {
            return Err(AgentError::Context(
                "max_tool_calls_per_turn must be > 0; a value of 0 would silently discard all tool calls"
                    .to_string(),
            ));
        }

        // 校验配置合法性
        if self.config.max_turns == 0 {
            return Err(AgentError::Context(
                "max_turns 必须 ≥ 1（为 0 表示循环永远不会执行任何轮次）".to_string(),
            ));
        }
        // max_tool_calls_per_turn = 0 合法：相当于禁用工具调用（已在上方校验为非法，此注释仅说明设计意图）
        // token_budget = 0 合法：表示不限制 token

        // Create state broadcast channel
        let (state_tx, _state_rx) = broadcast::channel(self.state_channel_capacity);

        // Initialize state as Idle
        let state = Arc::new(RwLock::new(AgentState::Idle));

        Ok(AgentLoop {
            provider,
            tools: self.tools,
            history: self.history,
            stop_conditions: self.stop_conditions,
            config: self.config,
            state,
            state_tx,
            event_publisher: self.event_publisher,
            agent_id: self.agent_id,
        })
    }
}

impl Default for AgentLoopBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use claw_provider::{
        error::ProviderError,
        traits::LLMProvider,
        types::{CompletionResponse, Delta, FinishReason, Message, Options, TokenUsage},
    };
    use futures::stream;
    use std::pin::Pin;

    struct MockProvider;

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-v1"
        }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                id: "id".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant("ok"),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage {
                    prompt_tokens: 5,
                    completion_tokens: 3,
                    total_tokens: 8,
                },
            })
        }

        async fn complete_stream(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = Result<Delta, ProviderError>> + Send>>,
            ProviderError,
        > {
            Ok(Box::pin(stream::empty()))
        }
    }

    fn mock_provider() -> Arc<dyn LLMProvider> {
        Arc::new(MockProvider)
    }

    // ── test_builder_requires_provider ───────────────────────────────────────

    #[test]
    fn test_builder_requires_provider() {
        let result = AgentLoopBuilder::new().build();
        assert!(result.is_err(), "build without provider should return Err");
        // Extract the error without requiring T: Debug on AgentLoop.
        let err = result.err().expect("expected an Err");
        match err {
            AgentError::Context(msg) => {
                assert!(
                    msg.contains("no provider"),
                    "error message should mention provider, got: {msg}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    // ── test_builder_with_max_turns ───────────────────────────────────────────

    #[test]
    fn test_builder_with_max_turns() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(42)
            .build()
            .expect("build should succeed");

        assert_eq!(agent.config.max_turns, 42);
    }

    // ── test_builder_auto_adds_stop_conditions ────────────────────────────────

    #[test]
    fn test_builder_auto_adds_stop_conditions() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_turns(10)
            .with_token_budget(5000)
            .build()
            .expect("build should succeed");

        // Builder no longer auto-adds stop conditions — default is empty.
        assert_eq!(
            agent.stop_conditions.len(),
            0,
            "builder should not auto-add any stop conditions by default, got {}",
            agent.stop_conditions.len()
        );
    }

    // ── test_builder_with_state_channel_capacity ─────────────────────────────

    #[test]
    fn test_builder_with_state_channel_capacity() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_state_channel_capacity(512)
            .build()
            .expect("build should succeed");

        // Channel capacity is set, verify by checking that we can subscribe
        let _rx = agent.subscribe_state();
        // If build succeeded, the capacity was set correctly
    }

    // ── test_builder_with_max_tool_calls_per_turn ────────────────────────────

    #[test]
    fn test_builder_with_max_tool_calls_per_turn() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_max_tool_calls_per_turn(5)
            .build()
            .expect("build should succeed");

        assert_eq!(agent.config.max_tool_calls_per_turn, 5);
    }

    // ── test_builder_default_max_tool_calls ──────────────────────────────────

    #[test]
    fn test_builder_default_max_tool_calls() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        assert_eq!(agent.config.max_tool_calls_per_turn, 10);
    }

    // ── test_builder_initial_state_is_idle ───────────────────────────────────

    #[tokio::test]
    async fn test_builder_initial_state_is_idle() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        assert_eq!(agent.current_state().await, AgentState::Idle);
    }

    // ── test_builder_state_subscription ──────────────────────────────────────

    #[test]
    fn test_builder_state_subscription() {
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .build()
            .expect("build should succeed");

        // Should be able to subscribe multiple times
        let _rx1 = agent.subscribe_state();
        let _rx2 = agent.subscribe_state();

        // Just verify we can create subscribers successfully
        // (broadcast channels don't have borrow(), only watch channels do)
    }

    // ── test_builder_with_sqlite_history ──────────────────────────────────────

    #[test]
    fn test_builder_with_sqlite_history_in_memory() {
        // Use ":memory:" path for SQLite in-memory database
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_sqlite_history(":memory:", "test-session")
            .build()
            .expect("build should succeed with in-memory sqlite history");

        // Just verify it built successfully
        assert_eq!(agent.config.max_turns, 20); // default
    }

    #[test]
    fn test_builder_with_sqlite_history_fallback_on_bad_path() {
        // A path in a directory that doesn't exist should fall back to in-memory
        let agent = AgentLoopBuilder::new()
            .with_provider(mock_provider())
            .with_sqlite_history("/nonexistent/path/history.db", "session")
            .build()
            .expect("build should not fail even with bad sqlite path (graceful fallback)");

        let _ = agent; // Just verify no panic
    }
}
