//! **claw-kernel** — Claw Agent runtime kernel.
//!
//! This meta-crate re-exports the most commonly used types from every
//! sub-crate so that consumers only need to add one dependency:
//!
//! ```toml
//! [dependencies]
//! claw-kernel = { path = "…", features = ["engine-lua"] }
//! ```
//!
//! Then import via:
//!
//! ```rust,ignore
//! use claw_kernel::prelude::*;
//! ```

// ── Layer 0.5: Platform Abstraction ─────────────────────────────────────────
pub use claw_pal as pal;

// ── Layer 1: Runtime ────────────────────────────────────────────────────────
pub use claw_runtime as runtime;

// ── Layer 2: Core subsystems ─────────────────────────────────────────────────
pub use claw_loop as agent_loop;
pub use claw_memory as memory;
pub use claw_provider as provider;
pub use claw_tools as tools;

// ── Layer 2: Channels (part of Agent Kernel Protocol) ───────────────────────
pub use claw_channel as channel;

// ── Layer 3: Script engines ──────────────────────────────────────────────────
pub use claw_script as script;

// ── Layer 3: Skill engine ────────────────────────────────────────────────────
pub use claw_skills as skills;
pub use claw_skills::{SkillIndex, SkillLoader, SkillManifest};

// ── Prelude ──────────────────────────────────────────────────────────────────

/// Convenient glob import of the most commonly used types.
///
/// ```rust,ignore
/// use claw_kernel::prelude::*;
/// ```
pub mod prelude {
    // ── Runtime ──────────────────────────────────────────────────────────────
    pub use claw_runtime::{
        agent_types::{AgentConfig, AgentHandle, AgentId},
        event_bus::EventBus,
        events::Event,
        orchestrator::AgentOrchestrator,
        runtime::Runtime,
    };
    // ExecutionMode is re-exported from claw_pal
    pub use claw_pal::ExecutionMode;

    // ── Provider ─────────────────────────────────────────────────────────────
    pub use claw_provider::{
        error::ProviderError,
        traits::{EmbeddingProvider, LLMProvider},
        types::{CompletionResponse, Message, Options, Role},
    };

    // ── Tools ─────────────────────────────────────────────────────────────────
    pub use claw_tools::{
        error::RegistryError,
        registry::ToolRegistry,
        traits::Tool,
        types::{PermissionSet, ToolContext, ToolResult, ToolSchema},
    };

    // ── Agent loop ───────────────────────────────────────────────────────────
    // Note: claw_loop::types::FinishReason is exported with an alias to avoid
    // collision with claw_provider::types::FinishReason (also in this prelude).
    pub use claw_loop::{
        agent_loop::AgentLoop,
        builder::AgentLoopBuilder,
        error::AgentError,
        summarizer::SimpleSummarizer,
        traits::{HistoryManager, StopCondition, Summarizer},
        types::FinishReason as LoopFinishReason,
        types::{AgentLoopConfig, AgentResult, LoopState},
    };

    // ── Memory ───────────────────────────────────────────────────────────────
    pub use claw_memory::{
        error::MemoryError,
        traits::MemoryStore,
        types::{MemoryId, MemoryItem},
    };
    // Concrete implementations re-exported for convenience
    pub use claw_memory::sqlite::SqliteMemoryStore;
    // NgramEmbedder lives in claw_provider::embedding (moved from claw_memory in v0.2.0)
    pub use claw_provider::embedding::NgramEmbedder;

    // ── Channel ──────────────────────────────────────────────────────────────
    pub use claw_channel::{
        error::ChannelError,
        traits::Channel,
        types::{ChannelId, ChannelMessage, Platform},
    };

    // ── Script ───────────────────────────────────────────────────────────────
    pub use claw_script::{
        error::ScriptError,
        traits::ScriptEngine,
        types::{EngineType, Script, ScriptContext},
    };

    #[cfg(feature = "engine-lua")]
    pub use claw_script::LuaEngine;

    #[cfg(feature = "engine-v8")]
    pub use claw_script::{V8Engine, V8EngineOptions};

    // ── PAL ──────────────────────────────────────────────────────────────────
    pub use claw_pal::{
        error::{PalError, SandboxError},
        traits::{IpcTransport, ProcessManager, SandboxBackend},
    };
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn test_pal_reexport_accessible() {
        // Can reference types through meta-crate namespace
        let _ = std::any::type_name::<crate::pal::error::PalError>();
    }

    #[test]
    fn test_runtime_reexport_accessible() {
        let _ = std::any::type_name::<crate::runtime::events::Event>();
    }

    #[test]
    fn test_provider_reexport_accessible() {
        let _ = std::any::type_name::<crate::provider::error::ProviderError>();
    }

    #[test]
    fn test_tools_reexport_accessible() {
        let _ = std::any::type_name::<crate::tools::registry::ToolRegistry>();
    }

    #[test]
    fn test_memory_reexport_accessible() {
        let _ = std::any::type_name::<dyn crate::memory::traits::MemoryStore>();
    }

    #[test]
    fn test_prelude_agent_id() {
        use crate::prelude::*;
        let id = AgentId::new("test");
        assert_eq!(id.as_str(), "test");
    }

    #[test]
    fn test_prelude_message() {
        use crate::prelude::*;
        let msg = Message::user("hello");
        assert_eq!(msg.content, "hello");
    }

    #[test]
    fn test_prelude_memory_id() {
        use crate::prelude::*;
        let id = MemoryId::new("mem-1");
        assert_eq!(id.0, "mem-1");
    }
}
