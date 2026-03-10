//! LLM bridge — exposes LLMProvider to Lua scripts.
//!
//! Registered as the global `llm` table.
//!
//! # Example in Lua:
//! ```lua
//! -- Non-streaming completion
//! local reply = llm:complete(
//!     {{ role = "user", content = "What is Rust?" }},
//!     { model = "claude-opus-4-6", max_tokens = 1024 }
//! )
//! print(reply)
//!
//! -- Streaming: returns array of text chunks
//! local chunks = llm:stream(
//!     {{ role = "user", content = "Tell me a joke" }},
//!     { model = "claude-opus-4-6" }
//! )
//! for _, chunk in ipairs(chunks) do
//!     io.write(chunk)
//! end
//! ```

use std::sync::Arc;

use claw_provider::{
    traits::LLMProvider,
    types::{Message, Options, Role},
};
use mlua::{Lua, Result as LuaResult, Table, UserData, UserDataMethods, Value as LuaValue};
use tokio::runtime::Handle;

/// LLM bridge exposing LLMProvider to Lua scripts.
///
/// Allows scripts to call LLM completions and streaming directly.
/// Registered as the global `llm` table.
pub struct LlmBridge {
    pub provider: Arc<dyn LLMProvider>,
}

impl LlmBridge {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self { provider }
    }
}

/// Parse a Lua table of messages into Vec<Message>.
///
/// Each entry should be a table with `role` (string) and `content` (string) fields.
fn parse_messages(msgs: Table) -> LuaResult<Vec<Message>> {
    let mut messages = Vec::new();
    let len = msgs.raw_len();

    for i in 1..=(len as i64) {
        let entry: Table = msgs
            .raw_get(i)
            .map_err(|e| mlua::Error::RuntimeError(format!("messages[{}] is not a table: {}", i, e)))?;

        let role_str: String = entry
            .get("role")
            .map_err(|_| mlua::Error::RuntimeError(format!("messages[{}].role is missing", i)))?;
        let content: String = entry
            .get("content")
            .map_err(|_| mlua::Error::RuntimeError(format!("messages[{}].content is missing", i)))?;

        let role = match role_str.to_lowercase().as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            "tool" => Role::Tool,
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "messages[{}]: unknown role '{}'",
                    i, other
                )))
            }
        };

        messages.push(Message {
            role,
            content,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    Ok(messages)
}

/// Parse an optional Lua opts table into an Options struct.
///
/// Recognises: `model` (string), `max_tokens` (integer), `temperature` (number).
/// All fields are optional; defaults come from `Options::new(model)`.
fn parse_opts(opts_val: LuaValue, default_model: &str) -> LuaResult<Options> {
    let model = match &opts_val {
        LuaValue::Table(t) => t
            .get::<_, Option<String>>("model")
            .unwrap_or(None)
            .unwrap_or_else(|| default_model.to_string()),
        _ => default_model.to_string(),
    };

    let mut options = Options::new(model);

    if let LuaValue::Table(t) = opts_val {
        if let Ok(Some(max_tokens)) = t.get::<_, Option<u32>>("max_tokens") {
            options = options.with_max_tokens(max_tokens);
        }
        if let Ok(Some(temp)) = t.get::<_, Option<f32>>("temperature") {
            options = options.with_temperature(temp).map_err(|e| {
                mlua::Error::RuntimeError(format!("opts.temperature: {}", e))
            })?;
        }
    }

    Ok(options)
}

impl UserData for LlmBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        // llm:complete(messages_table, opts_table?) -> string
        //
        // Blocks until the LLM returns a full response, then returns the
        // assistant's content string.  Uses block_on inside spawn_blocking
        // (consistent with NetBridge Fix-F pattern).
        methods.add_method("complete", |_lua, this, (msgs, opts_val): (Table, LuaValue)| {
            let messages = parse_messages(msgs)?;
            let options = parse_opts(opts_val, this.provider.model_id())?;
            let provider = Arc::clone(&this.provider);

            // We're already inside spawn_blocking (Lua executes in spawn_blocking).
            // Use Handle::current() + block_on to drive the async future.
            let handle = Handle::current();
            let result = handle.block_on(async move { provider.complete(messages, options).await });

            match result {
                Ok(resp) => Ok(resp.message.content),
                Err(e) => Err(mlua::Error::RuntimeError(format!("llm.complete error: {}", e))),
            }
        });

        // llm:stream(messages_table, opts_table?) -> table of strings (chunks)
        //
        // Collects all streaming deltas and returns them as a Lua array of strings.
        // This avoids the complexity of crossing the mlua async boundary.
        methods.add_method("stream", |lua, this, (msgs, opts_val): (Table, LuaValue)| {
            let messages = parse_messages(msgs)?;
            let options = parse_opts(opts_val, this.provider.model_id())?;
            let provider = Arc::clone(&this.provider);

            let handle = Handle::current();
            let result = handle.block_on(async move {
                use futures::StreamExt;

                let mut stream = provider.complete_stream(messages, options).await?;
                let mut chunks: Vec<String> = Vec::new();
                while let Some(delta) = stream.next().await {
                    match delta {
                        Ok(d) => {
                            if let Some(content) = d.content {
                                if !content.is_empty() {
                                    chunks.push(content);
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(chunks)
            });

            match result {
                Ok(chunks) => {
                    let table = lua.create_table()?;
                    for (i, chunk) in chunks.into_iter().enumerate() {
                        table.raw_set(i + 1, chunk)?;
                    }
                    Ok(table)
                }
                Err(e) => Err(mlua::Error::RuntimeError(format!("llm.stream error: {}", e))),
            }
        });
    }
}

/// Register the LlmBridge as a global `llm` table in the Lua instance.
pub fn register_llm(lua: &Lua, bridge: LlmBridge) -> LuaResult<()> {
    lua.globals().set("llm", bridge)
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
    use mlua::Lua;
    use std::pin::Pin;
    use futures::Stream;

    /// Mock provider that returns a fixed response.
    struct MockProvider {
        response: String,
    }

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn provider_id(&self) -> &str { "mock" }
        fn model_id(&self) -> &str { "mock-v1" }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                id: "mock-id".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant(self.response.clone()),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage::default(),
            })
        }

        async fn complete_stream(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError> {
            let chunks = vec![
                Ok(Delta { content: Some("Hello".to_string()), tool_call: None, finish_reason: None, usage: None }),
                Ok(Delta { content: Some(" world".to_string()), tool_call: None, finish_reason: None, usage: None }),
            ];
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    fn make_bridge() -> LlmBridge {
        LlmBridge::new(Arc::new(MockProvider {
            response: "Rust is a systems programming language.".to_string(),
        }))
    }

    #[tokio::test]
    async fn test_llm_bridge_complete() {
        let bridge = make_bridge();
        // Run in spawn_blocking to match the production Lua execution environment.
        // (block_on requires being outside an async task, which spawn_blocking provides)
        let result = tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            register_llm(&lua, bridge).unwrap();
            lua.load(r#"
                return llm:complete(
                    {{ role = "user", content = "What is Rust?" }},
                    { model = "mock-v1" }
                )
            "#)
            .eval::<String>()
            .unwrap()
        })
        .await
        .unwrap();

        assert_eq!(result, "Rust is a systems programming language.");
    }

    #[tokio::test]
    async fn test_llm_bridge_complete_no_opts() {
        let bridge = make_bridge();
        let result = tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            register_llm(&lua, bridge).unwrap();
            lua.load(r#"
                return llm:complete(
                    {{ role = "user", content = "Hello" }}
                )
            "#)
            .eval::<String>()
            .unwrap()
        })
        .await
        .unwrap();

        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn test_llm_bridge_stream() {
        let bridge = make_bridge();
        let count = tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            register_llm(&lua, bridge).unwrap();
            lua.load(r#"
                local chunks = llm:stream(
                    {{ role = "user", content = "Hi" }},
                    { model = "mock-v1" }
                )
                return #chunks
            "#)
            .eval::<i64>()
            .unwrap()
        })
        .await
        .unwrap();

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_llm_bridge_stream_concatenate() {
        let bridge = make_bridge();
        let combined = tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            register_llm(&lua, bridge).unwrap();
            lua.load(r#"
                local chunks = llm:stream(
                    {{ role = "user", content = "Hi" }},
                    {}
                )
                local result = ""
                for _, chunk in ipairs(chunks) do
                    result = result .. chunk
                end
                return result
            "#)
            .eval::<String>()
            .unwrap()
        })
        .await
        .unwrap();

        assert_eq!(combined, "Hello world");
    }

    #[tokio::test]
    async fn test_llm_bridge_invalid_role() {
        let bridge = make_bridge();
        let result = tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            register_llm(&lua, bridge).unwrap();
            lua.load(r#"
                return llm:complete(
                    {{ role = "unknown_role", content = "test" }}
                )
            "#)
            .eval::<String>()
        })
        .await
        .unwrap();

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown role"), "expected 'unknown role' in error, got: {}", err);
    }
}
