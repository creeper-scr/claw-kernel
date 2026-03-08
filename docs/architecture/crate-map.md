---
title: claw-kernel Crate Map
description: Crate structure, dependencies, and relationships
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---


# Crate Map & Dependency Graph

This document describes the crate structure of claw-kernel, including dependencies and relationships.

---

## Architecture Overview

```
╔══════════════════════════════════════════════════════════════════════╗
║                     CORE KERNEL (Minimal & Stable)                    ║
╠══════════════════════════════════════════════════════════════════════╣
║  Layer 0         Layer 0.5      Layer 1        Layer 2        Layer 2.5    Layer 3    ║
║                                                                                       ║
║  ┌────┐  ┌─────────┐  ┌──────────┐   ┌───────────┐  ┌───────────┐  ┌───────────┐     ║
║  │Rust│  │claw-pal │  │claw-     │   │claw-loop  │  │claw-channel  │claw-script│     ║
║  │Hard│  │(Platform│  │runtime  │   │(Agent     │  │(Channel   │  │(Script    │     ║
║  │Core│  │Abstr.)  │  │(System  │   │ Loop)     │  │ Integr.)  │  │ Runtime)  │     ║
║  └──┬─┘  └────┬────┘  │ Runtime) │   └─────┬─────┘  └───────────┘  └───────────┘     ║
║       │       └────┬─────┘         │         │                                       ║
║       │            │          ┌────┴────┐    │                                       ║
║       │            │          │         │    │                                       ║
║       │            │       ┌──▼──┐  ┌───▼────┐│                                       ║
║       │            │       │claw-│  │claw-   ││                                       ║
║       │            │       │tools│  │provider││                                       ║
║       │            │       │(Tool│  │(LLM    ││                                       ║
║       │            │       │Proto│  │ Abstr.)││                                       ║
║       │            │       └─────┘  └────────┘│                                       ║
║       │            │                         │                                       ║
║       └────────────┴─────────────────────────┴──────────┐                            ║
║                                                         ▼                            ║
╠══════════════════════════════════════════════════════════════════════╣
║                         Meta-crate: claw-kernel                       ║
╚══════════════════════════════════════════════════════════════════════╝
```

---

## Core Kernel Crates (Layer 0.5 - 3)

> **Design Principle**: Minimal, stable, and essential for the kernel to function. These crates define the foundational abstractions and protocols.

### `claw-pal` — Platform Abstraction Layer

**Layer**: 0.5  
**Purpose**: Isolate all platform-specific code.

**Key Traits:**
- `SandboxBackend` — Platform sandbox implementations
- `IpcTransport` — Cross-platform IPC
- `ProcessManager` — Process lifecycle

**Platform Modules:**
```
claw-pal/src/
├── lib.rs
├── traits/
│   ├── sandbox.rs
│   ├── ipc.rs
│   └── process.rs
├── linux/
│   ├── sandbox.rs    # seccomp-bpf + namespaces
│   ├── ipc.rs        # Unix Domain Socket
│   └── process.rs    # fork/exec
├── macos/
│   ├── sandbox.rs    # sandbox(7) profile
│   ├── ipc.rs        # Unix Domain Socket
│   └── process.rs    # fork/exec
└── windows/
    ├── sandbox.rs    # AppContainer + Job Objects
    ├── ipc.rs        # Named Pipe
    └── process.rs    # CreateProcess
```

**Dependencies:** None (core only)

---

### `claw-runtime` — System Runtime

**Layer**: 1  
**Purpose**: Event bus, process management, multi-agent orchestration.

**Key Components:**
```rust
pub struct Runtime {
    pub event_bus: EventBus,
    pub process_manager: ProcessManager,
    pub ipc_router: IpcRouter,
}

pub struct EventBus {
    pub fn emit(&self, event: Event);
    pub fn subscribe(&self, filter: EventFilter) -> Receiver<Event>;
}
```

**Multi-Agent Support:**
```rust
pub struct AgentOrchestrator {
    pub fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
    pub fn kill(&self, handle: AgentHandle) -> Result<()>;
    pub fn list(&self) -> Vec<AgentInfo>;
    pub fn send_message(&self, from: AgentId, to: AgentId, msg: A2AMessage);
}

/// Handle to an agent process (wraps AgentId with process information)
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct AgentHandle {
    pub id: AgentId,
    pub pid: u32,
}

pub type AgentId = String;

pub struct AgentInfo {
    pub handle: AgentHandle,
    pub name: String,
    pub status: AgentStatus,
    pub capabilities: Vec<String>,
}

pub enum AgentStatus {
    Running,
    Stopped,
    Crashed,
}

pub struct A2AMessage {
    pub from: AgentId,
    pub to: AgentId,                 // Required: target agent ID
    pub correlation_id: String,      // For request-response correlation
    pub payload: serde_json::Value,  // Serialized message payload
}
// 注意: 当前代码缺少 message_type, timeout, priority, timestamp 字段

pub enum Payload {
    Json(serde_json::Value),
    Cbor(Vec<u8>),
}

pub enum MessagePriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Background = 4,
}

pub enum A2AMessageType {
    Request,      // Expects response
    Response,     // Response to request
    Event,        // Fire-and-forget
    Command,      // Directive (parent to child)
}

/// Event filter for subscription
pub struct EventFilter {
    pub event_types: Vec<EventType>,
    pub agent_id: Option<AgentId>,
}

pub enum EventType {
    UserInput,
    AgentOutput,
    ToolCall,
    ToolResult,
    AgentLifecycle,
    Extension,
    A2A,
}
```

**Dependencies:**
- `claw-pal`
- `tokio` — Async runtime

---

### `claw-provider` — LLM Provider Abstraction

**Layer**: 2 (System Architecture)  
**Purpose**: Unified interface for LLM APIs with three-layer architecture for maximum code reuse.

> **Note**: The following Layer 1/2/3 refers to internal architecture within claw-provider. claw-provider as a whole resides at Layer 2 of the system architecture.

**Architecture (Three-Layer Design):**

```
Level 3: LLMProvider trait - User-facing interface
    complete(), stream_complete()
    
Level 2: HttpTransport trait - Reusable HTTP logic
    request(), stream_request()
    
Level 1: MessageFormat trait - Protocol abstraction
    OpenAIFormat, AnthropicFormat, OllamaFormat
```

**Key Types:**
```rust
// Level 1: Protocol format abstraction
#[async_trait]
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    fn build_request(messages: &[Message], opts: &Options) -> Self::Request;
    fn parse_response(raw: Self::Response) -> Result<CompletionResponse, Self::Error>;
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
    fn token_count(messages: &[Message]) -> usize;
    fn endpoint() -> &'static str;
}

// Core types used across all levels
pub struct Message {
    pub role: Role,
    pub content: String,
    pub name: Option<String>,        // Identifier for the sender (e.g., function name)
    pub metadata: Option<serde_json::Value>, // Extended metadata
}

pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

pub struct Options {
    pub model: Option<String>,       // Model identifier, None = use provider default
    pub temperature: Option<f32>,    // Range: 0.0-2.0, default: 1.0
    pub max_tokens: Option<usize>,   // Maximum tokens to generate, default: 4096
    pub stop_sequences: Vec<String>, // Stop sequences, default: empty
    pub tools: Option<Vec<ToolDef>>, // Available tools for function calling
    pub timeout: Duration,           // Request timeout, default: 60s
    pub max_retries: u32,            // Max retry attempts, default: 3
}

// Level 2: Reusable HTTP transport
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    async fn request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<CompletionResponse, ProviderError>;
    
    async fn stream_request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}

pub struct CompletionResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    pub finish_reason: FinishReason,
}

pub struct Delta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

pub type Embedding = Vec<f32>;

// Level 3: User-facing provider trait
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

// Separate trait for embedding capability (not all providers support this)
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding, ProviderError>;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError>;
}
```

**Module Structure:**
```
claw-provider/src/
├── lib.rs              # Public exports
├── traits.rs           # LLMProvider, HttpTransport, MessageFormat traits
├── types.rs            # Message, Response, Options, etc.
├── format/             # Layer 1: Protocol implementations
│   ├── mod.rs
│   ├── openai.rs       # OpenAIFormat (shared by 50+ providers)
│   ├── anthropic.rs    # AnthropicFormat
│   └── ollama.rs       # OllamaFormat
├── transport.rs        # Layer 2: HttpTransport implementations
└── providers/          # Layer 3: Provider configurations
    ├── mod.rs
    ├── anthropic.rs    # AnthropicProvider (uses AnthropicFormat)
    ├── bedrock.rs      # BedrockProvider (uses AnthropicFormat + AWS auth)
    ├── openai.rs       # OpenAIProvider (uses OpenAIFormat)
    ├── deepseek.rs     # DeepSeekProvider (uses OpenAIFormat)
    ├── moonshot.rs     # MoonshotProvider (uses OpenAIFormat)
    ├── qwen.rs         # QwenProvider (uses OpenAIFormat)
    ├── grok.rs         # GrokProvider (uses OpenAIFormat)
    └── ollama.rs       # OllamaProvider (uses OllamaFormat)
```

**Built-in Providers (by format):**

| Format | Providers |
|--------|-----------|
| `AnthropicFormat` | `AnthropicProvider` (Claude), `BedrockProvider` (AWS) |
| `OpenAIFormat` | `OpenAIProvider`, `DeepSeekProvider`, `MoonshotProvider`, `QwenProvider`, `GrokProvider`, `AzureOpenAIProvider` |
| `OllamaFormat` | `OllamaProvider` (local models) |

**Adding a new OpenAI-compatible provider (~20 lines):**
```rust
pub struct NewProvider {
    api_key: String,
    model: String,
    client: Client,
}

impl HttpTransport for NewProvider {
    fn base_url(&self) -> &str { "https://api.newprovider.com/v1" }
    fn auth_headers(&self) -> HeaderMap { /* Bearer token */ }
    fn http_client(&self) -> &Client { &self.client }
}

#[async_trait]
impl LLMProvider for NewProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        self.request::<OpenAIFormat>(messages, opts).await
    }
    
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError> {
        self.stream_request::<OpenAIFormat>(messages, opts).await
    }
    
    // Note: This provider does not implement EmbeddingProvider
    // as Anthropic Claude does not have an embedding API
    
    fn token_count(&self, messages: &[Message]) -> usize {
        OpenAIFormat::token_count(messages)
    }
}
```

**Features:**
```toml
[features]
default = ["openai", "anthropic"]

# OpenAI-compatible providers (all reuse OpenAIFormat)
openai = []
deepseek = ["openai"]   # Reuses OpenAIFormat
moonshot = ["openai"]   # Reuses OpenAIFormat
qwen = ["openai"]       # Reuses OpenAIFormat
grok = ["openai"]       # Reuses OpenAIFormat
azure = ["openai"]      # Reuses OpenAIFormat with special auth

# Anthropic-compatible providers
anthropic = []
bedrock = ["anthropic"] # Reuses AnthropicFormat with AWS auth

# Local models
ollama = []

# All providers
full = ["openai", "deepseek", "moonshot", "qwen", "grok", "azure",
        "anthropic", "bedrock", "ollama"]
```

**Dependencies:**
- `reqwest` — HTTP client
- `serde` — Serialization
- `async-trait` — Async traits

**See also:**
- [ADR-006: Message Format Abstraction](../adr/006-message-format-abstraction.md)

---

### `claw-tools` — Tool Registry & Protocol

**Layer**: 2  
**Purpose**: Define tool-use protocol and hot-loading interface.  
**[Warning]  Note**: Only provides registration protocol and hot-loading mechanism. Specific tool generation logic is implemented at the application layer.

**Key Types:**
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;           // Tool description for LLM
    fn version(&self) -> &str;               // Semantic version, e.g., "1.0.0"
    fn schema(&self) -> ToolSchema;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    fn permissions(&self) -> PermissionSet;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }  // Default timeout
}

pub type ToolSchema = serde_json::Value; // JSON Schema as JSON

pub struct ToolResult {
    pub output: Option<serde_json::Value>,
    pub error: Option<ToolError>,
    pub logs: Vec<LogEntry>,
    pub execution_time_ms: u64,
}

pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

pub enum ToolErrorCode {
    InvalidParameter,
    ExecutionFailed,
    Timeout,
    PermissionDenied,
    ResourceNotFound,
    RateLimited,
    InternalError,
}

pub struct PermissionSet {
    pub filesystem: FsPermissions,
    pub network: NetworkPermissions,
    pub subprocess: SubprocessPolicy,
}

pub enum FsPermissions {
    ReadOnly(Vec<PathBuf>),
    ReadWrite(Vec<PathBuf>),
    None,
}

pub struct NetworkPermissions {
    pub allowed_domains: HashSet<String>,
    pub allowed_ports: Vec<u16>,
    pub allow_localhost: bool,
    pub allow_private_ips: bool,
}

pub enum SubprocessPolicy {
    Allow { 
        allowed_commands: Vec<String>,
        max_concurrent: usize,
    },
    Deny,
}

pub struct ToolRegistry {
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;
    pub fn unregister(&mut self, name: &str) -> Result<(), RegistryError>;
    pub fn get(&self, name: &str) -> Option<&dyn Tool>;
    pub fn list(&self) -> Vec<&ToolMeta>;
    pub async fn execute(&self, name: &str, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    pub async fn load_from_script(&mut self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub fn unload(&mut self, name: &str) -> Result<(), RegistryError>;
    pub async fn enable_hot_loading(&mut self, config: HotLoadingConfig) -> Result<(), WatchError>;
}
```

**Features:**
```toml
[features]
default = ["hot-loading"]
hot-loading = ["notify"]  # File system watching
schema-gen = ["schemars"]
```

**Dependencies:**
- `serde_json` — JSON handling
- `schemars` — Schema generation
- `notify` — File watching (optional)

---

### `claw-loop` — Agent Loop Engine

**Layer**: 2  
**Purpose**: Multi-turn conversation management and control flow.

**Key Types:**
```rust
pub struct AgentLoop {
    pub history: Box<dyn HistoryManager>,
    pub stop_conditions: Vec<Box<dyn StopCondition>>,
    pub max_turns: Option<usize>,
    pub token_budget: Option<usize>,
}

pub trait StopCondition: Send + Sync {
    fn should_stop(&self, state: &LoopState) -> bool;
}

pub trait HistoryManager: Send + Sync {
    fn append(&self, message: Message); // &self with internal mutability
    fn get_context(&self, max_tokens: usize) -> Vec<Message>;
    fn truncate_to_fit(&self, max_tokens: usize);
    fn summarize(&self, strategy: &dyn Summarizer);
}

pub trait Summarizer: Send + Sync {
    fn summarize(&self, messages: &[Message]) -> String;
}

pub struct LoopState {
    pub turn_count: usize,
    pub token_usage: TokenUsage,
    pub last_message: Option<Message>,
    pub tool_calls_made: usize,
}
```

**Built-in Stop Conditions:**
- `MaxTurnsCondition`
- `TokenBudgetCondition`
- `NoToolCallCondition`
- `UserInterruptCondition`

**Dependencies:**
- `claw-provider`
- `claw-tools`

---

### `claw-script` — Extension Foundation

**Layer**: 3  
**Purpose**: Script execution and hot-loading foundation.  
**[Warning]  Note**: This is an **extension base**, not a self-evolving system itself. It provides script execution, hot-loading, and runtime extension capabilities that applications can build upon.

**Engine Support:**
```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]        # Lua (default, zero deps)
engine-v8 = ["deno_core"]    # Deno/V8 (full TS/JS)
engine-py = ["pyo3"]         # Python (ML ecosystem)
```

**Key Types:**
```rust
pub trait ScriptEngine: Send + Sync {
    fn compile(&self, source: &str, source_name: &str) -> Result<Script>;
    fn execute(&self, script: &Script, context: &Context, timeout: Duration) -> Result<Value>;
    fn register_native(&self, name: &str, func: NativeFunction);
}

/// Compiled script (engine-specific representation)
pub enum Script {
    Lua(mlua::Function),
    #[cfg(feature = "engine-v8")]
    V8(deno_core::JsRuntime),
    #[cfg(feature = "engine-py")]
    Python(PythonScriptHandle),
}

/// Execution context passed to scripts
pub struct Context {
    pub agent_id: AgentId,
    pub permissions: PermissionSet,
    pub runtime_data: HashMap<String, Value>,
}

pub type Value = serde_json::Value;

/// Native function callable from scripts
pub type NativeFunction = Arc<dyn Fn(&Context, Vec<Value>) -> Result<Value, ScriptError> + Send + Sync>;

pub struct ScriptError {
    pub code: String,
    pub message: String,
    pub stack_trace: Option<String>,
}

pub struct RustBridge {
    // Exposed to scripts
    pub llm: LlmBridge,
    pub tools: ToolsBridge,
    pub memory: MemoryBridge,
    pub events: EventsBridge,
    pub fs: FsBridge,
    pub agent: AgentBridge,
}
```

**Dependencies:**
- `mlua` (optional) — Lua binding
- `deno_core` (optional) — V8 embedding
- `pyo3` (optional) — Python embedding

---

## Kernel Boundary

The Core Kernel consists of the following crates, all of which are stable, minimal, and essential:

| Crate | Layer | Purpose |
|-------|-------|---------|
| `claw-pal` | 0.5 | Platform Abstraction Layer - sandbox, IPC, process management (must support all platforms) |
| (Layer 0) | 0 | Rust Hard Core - memory safety, core types (platform-agnostic, no crate) |
| `claw-runtime` | 1 | System runtime (essential) |
| `claw-provider` | 2 | LLM abstraction (essential) |
| `claw-tools` | 2 | Tool protocol (essential) |
| `claw-loop` | 2 | Agent loop (essential) |
| `claw-script` | 3 | Script Runtime (optional but stable) |

Applications built on claw-kernel can implement:
- Custom `MemoryBackend` trait for memory storage
- Custom `Channel` trait for platform integrations
- Custom tool generation logic
- Self-evolving systems on top of claw-script

---

## Meta Crate

### `claw-kernel` — Re-export Crate

**Purpose**: Convenient single dependency for users wanting the full kernel.

**Structure:**
```rust
// lib.rs
pub use claw_pal as pal;
pub use claw_provider as provider;
pub use claw_tools as tools;
pub use claw_loop as loop_;
pub use claw_runtime as runtime;
pub use claw_script as script;

// Re-export common types
pub use claw_provider::LLMProvider;
pub use claw_tools::{Tool, ToolRegistry};
pub use claw_loop::AgentLoop;
```

**Features:**
```toml
[features]
default = ["engine-lua"]

# Re-export from claw-script
engine-lua = ["claw-script/engine-lua"]
engine-v8 = ["claw-script/engine-v8"]
engine-py = ["claw-script/engine-py"]
```

---

## Dependency Graph (Visual)

```
                        ┌─────────────┐
                        │claw-kernel  │
                        └──────┬──────┘
                               │
      ┌────────────────────────┼────────────────────────┐
      │            │           │           │            │
      ▼            ▼           ▼           ▼            ▼
┌─────────┐ ┌──────────┐ ┌─────────┐ ┌──────────┐ ┌──────────┐
│claw-pal │ │claw-tools│ │claw-loop│ │claw-script│
│(Core)   │ │(Core)    │ │(Core)   │ │(Core)    │
└────┬────┘ └────┬─────┘ └────┬────┘ └──────────┘
     │           │            │
     │      ┌────┴────┐       │
     │      │         │       │
     │      ▼         ▼       │
     │  ┌────────┐ ┌────────┐ │
     │  │claw-   │ │claw-   │ │
     │  │provider│ │runtime │ │
     │  │(Core)  │ │(Core)  │ │
     │  └────────┘ └────┬───┘ │
     │                  │     │
     └──────────────────┘     │
                              │
                              ▼
                        ┌──────────┐
                        │  tokio   │
                        └──────────┘
```

**Legend:**
- All crates shown are part of the Core Kernel (stable, minimal)

---

## Versioning Strategy

All crates in the workspace are versioned together ("unified versioning"):

- Current version: `0.1.0`
- All crates share the same version number
- Breaking changes bump all crates

This simplifies dependency management for users.

---

## Feature Flag Guidelines

### When to Use Features

1. **Platform-specific functionality** that pulls in heavy deps
   - Example: `engine-v8` (100MB+ binary impact)

2. **Optional script engines**
   - Example: `engine-py` (requires Python 3.10+)

### When NOT to Use Features

1. **Core functionality** — always include
2. **Small, pure-Rust dependencies** — just depend on them

---

## Testing Strategy by Crate

| Crate | Type | Unit Tests | Integration | Platform Tests |
|-------|------|:----------:|:-----------:|:--------------:|
| claw-pal | Core | Yes | Yes | Required per-platform |
| claw-provider | Core | Yes | Yes (mock HTTP) | N/A |
| claw-tools | Core | Yes | Yes | N/A |
| claw-loop | Core | Yes | Yes | N/A |
| claw-runtime | Core | Yes | Yes | Required |
| claw-script | Core | Yes | Yes | Required per-engine |
| claw-kernel | Meta | N/A | Yes | N/A |

---
