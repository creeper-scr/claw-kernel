---
title: claw-kernel Crate Map
description: Crate structure, dependencies, and relationships
status: implemented
version: "1.0.0"
last_updated: "2026-03-01"
language: en
---


# Crate Map & Dependency Graph

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

This document describes the crate structure of claw-kernel, including dependencies and relationships.

---

## Architecture Overview

```
╔══════════════════════════════════════════════════════════════════════╗
║                     CORE KERNEL (Minimal & Stable)                    ║
╠══════════════════════════════════════════════════════════════════════╣
║  Layer 0         Layer 0.5      Layer 1        Layer 2                Layer 3        ║
║                                                                                       ║
║  ┌────┐  ┌─────────┐  ┌──────────┐   ┌──────────────────────────┐  ┌───────────┐     ║
║  │Rust│  │claw-pal │  │claw-     │   │claw-loop  │  │claw-channel│  │claw-script│     ║
║  │Hard│  │(Platform│  │runtime  │   │(Agent     │  │(Channel    │  │(Script    │     ║
║  │Core│  │Abstr.)  │  │(System  │   │ Loop)     │  │  Adapter)  │  │ Runtime)  │     ║
║  └──┬─┘  └────┬────┘  │ Runtime) │   └─────┬────┘  └───────────┘  └───────────┘     ║
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

/// Newtype wrapper for agent identifiers. Provides type safety over raw strings.
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self;
    pub fn generate() -> Self;
    pub fn as_str(&self) -> &str;
}

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
    pub id: String,                  // Unique message ID
    pub source: AgentId,             // Source agent ID
    pub target: Option<AgentId>,     // Target agent ID (None for broadcast)
    pub message_type: A2AMessageType, // Request, Response, Event, Command
    pub payload: A2AMessagePayload,  // Typed payload (see below)
    pub priority: MessagePriority,   // Critical, High, Normal, Low, Background
    pub correlation_id: Option<String>, // For request-response matching
    pub timestamp: u64,              // Unix milliseconds
    pub ttl_secs: Option<u32>,       // Time-to-live (None for no expiry)
}

pub enum A2AMessageType {
    Request,           // Expects response
    Response,          // Response to request
    Event,             // Fire-and-forget
    DiscoveryRequest,  // Query for available capabilities
    DiscoveryResponse, // Reply to discovery request
    Heartbeat,         // Keep-alive message
    Error,             // Indicates a processing error
}

pub enum A2AMessagePayload {
    Request {
        action: String,
        extra: HashMap<String, serde_json::Value>,
    },
    Response {
        status: ResponseStatus,
        result: serde_json::Value,
    },
    Event {
        event_type: String,
        data: serde_json::Value,
    },
    DiscoveryRequest {
        query: Option<String>,
    },
    DiscoveryResponse {
        capabilities: Vec<AgentCapability>,
        metadata: Option<HashMap<String, String>>,
    },
    Heartbeat {
        status: AgentStatus,
    },
    Error {
        code: String,
        message: String,
    },
}

pub enum MessagePriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Background = 4,
}

pub enum ResponseStatus {
    Success,
    Partial,
    Failure,
}

pub enum AgentStatus {
    Active,
    Idle,
    Busy,
    ShuttingDown,
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
    pub model: String,               // Model identifier (required)
    pub temperature: f32,            // Range: 0.0-2.0, default: 0.7
    pub max_tokens: u32,             // Maximum tokens to generate, default: 4096
    pub stop_sequences: Vec<String>, // Stop sequences, default: empty
    pub tools: Option<Vec<ToolDef>>, // Available tools for function calling
    pub timeout_seconds: u64,        // Request timeout, default: 60s
    pub max_retries: u32,            // Max retry attempts, default: 3
    pub stream: bool,                // Enable streaming, default: false
    pub system: Option<String>,      // System prompt override
}

// Level 2: Reusable HTTP transport (uses RPITIT - Rust 2021+)
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    fn request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options,
    ) -> impl Future<Output = Result<CompletionResponse, ProviderError>> + Send;
    
    fn stream_request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options,
    ) -> impl Future<Output = Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>> + Send;
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
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
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

// Level 3: User-facing provider trait (uses RPITIT - Rust 2021+)
pub trait LLMProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<CompletionResponse, ProviderError>;
    
    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Options,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>;
    
    fn token_count(&self, text: &str) -> usize {
        text.len() / 4
    }
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

pub struct FsPermissions {
    pub read_paths: HashSet<String>,   // Allowlisted read paths
    pub write_paths: HashSet<String>,  // Allowlisted write paths
}

pub struct NetworkPermissions {
    pub allowed_domains: HashSet<String>,
    pub allowed_ports: Vec<u16>,
    pub allow_localhost: bool,
    pub allow_private_ips: bool,
}

pub enum SubprocessPolicy {
    Denied,
    Allowed,
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
    pub max_turns: u32,
    pub token_budget: u64,
}

pub trait StopCondition: Send + Sync {
    /// Return true if the loop should stop given the current state.
    fn should_stop(&self, state: &LoopState) -> bool;
    /// Human-readable name for this condition (used in logs).
    fn name(&self) -> &str;
}

pub trait HistoryManager: Send + Sync {
    /// Append a message to history.
    fn append(&mut self, message: Message);
    /// Get all current messages.
    fn messages(&self) -> &[Message];
    /// Number of messages in history.
    fn len(&self) -> usize;
    /// Whether history is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Rough token estimate for the whole history.
    fn token_estimate(&self) -> usize;
    /// Clear all history.
    fn clear(&mut self);
    /// Set a callback invoked when history approaches the context limit.
    fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>);
}

pub trait Summarizer: Send + Sync {
    /// Summarize the given messages. Returns a concise summary string.
    async fn summarize(&self, messages: &[Message]) -> Result<String, AgentError>;
}

pub struct LoopState {
    pub turn: u32,
    pub usage: TokenUsage,
    pub history_len: usize,
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

### `claw-memory` — Memory System

**Layer**: 2  
**Purpose**: Long-term memory layer with semantic search, persistent storage, and quota management.  
**[Info] Note**: This is an **optional reference implementation** per [ADR-010](../adr/010-memory-system-boundary.md). The kernel only requires `HistoryManager` for short-term context window; mid/long-term memory is application responsibility.

**Architecture:**
```
Agent -> SecureMemoryStore (50MB quota)
      -> SqliteMemoryStore (cosine similarity search)
      -> NgramEmbedder (64-dim bigram+trigram)
      -> SQLite (rusqlite + sqlite-vec)
```

**Key Types:**
```rust
/// 64-dimensional character n-gram embedder (no external API needed)
pub struct NgramEmbedder {
    dim: usize,  // default: 64
}

impl Embedder for NgramEmbedder {
    fn embed(&self, text: &str) -> Vec<f32>;
    fn embed_batch(&self, texts: &[String]) -> Vec<Vec<f32>>;
}

/// SQLite-backed memory store with in-process cosine similarity
pub struct SqliteMemoryStore {
    conn: Connection,
    embedder: Arc<dyn Embedder>,
}

impl MemoryStore for SqliteMemoryStore {
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;
    async fn store_batch(&self, items: Vec<MemoryItem>) -> Result<Vec<MemoryId>, MemoryError>;
    async fn retrieve(&self, id: &MemoryId) -> Option<MemoryItem>;
    async fn semantic_search(
        &self,
        query: &str,
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>, MemoryError>;
}

/// Quota-enforcing wrapper (50MB default per agent)
pub struct SecureMemoryStore {
    inner: SqliteMemoryStore,
    quota_bytes: usize,  // default: 50MB
    current_usage: AtomicUsize,
}

/// Memory item with metadata
pub struct MemoryItem {
    pub id: MemoryId,              // Unique identifier (auto-generated)
    pub namespace: String,          // Agent-specific isolation
    pub content: String,
    pub embedding: Option<Vec<f32>>, // Auto-computed if None
    pub tags: Vec<String>,
    pub created_at_ms: u64,
    pub accessed_at_ms: u64,
    pub importance: f32,            // 0.0-1.0, affects retention
}

/// Episodic memory entry (conversation context linking)
pub struct EpisodicEntry {
    pub episode_id: String,         // Conversation/session ID
    pub role: String,               // "user", "assistant", "system"
    pub turn_index: usize,          // Position in conversation
    pub content: String,
    pub timestamp_ms: u64,
}
```

**MemoryStore Trait API:**
```rust
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;
    async fn store_batch(&self, items: Vec<MemoryItem>) -> Result<Vec<MemoryId>, MemoryError>;
    
    /// Store with quota check - fails if would exceed quota
    async fn store_with_quota_check(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;
    
    async fn retrieve(&self, id: &MemoryId) -> Option<MemoryItem>;
    
    /// Semantic search by embedding similarity
    async fn semantic_search(
        &self,
        query_embedding: &[f32],
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<ScoredMemory>, MemoryError>;
    
    /// Search episodic memory by episode_id and filters
    async fn search_episodic(
        &self,
        filter: EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError>;
    
    async fn clear_namespace(&self, namespace: &str) -> Result<(), MemoryError>;
}
```

**Features:**
```toml
[features]
default = ["sqlite", "ngram-embedder"]
sqlite = ["rusqlite", "sqlite-vec"]
ngram-embedder = []  # Pure Rust, no external deps
```

**Dependencies:**
- `rusqlite` — SQLite binding
- `sqlite-vec` — Vector similarity search extension

**See also:**
- [ADR-010: Memory System Boundary](../adr/010-memory-system-boundary.md)
- [Writing Tools Guide](../guides/writing-tools.md)

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
```

**Key Types:**
```rust
pub trait ScriptEngine: Send + Sync {
    /// Name of this engine (e.g., "lua").
    fn engine_type(&self) -> &str;
    /// Execute a script and return the last expression value.
    async fn execute(&self, script: &Script, ctx: &ScriptContext) -> Result<ScriptValue, ScriptError>;
    /// Check if a script compiles (no execution).
    fn validate(&self, script: &Script) -> Result<(), ScriptError>;
}

/// Compiled/loaded script.
pub struct Script {
    pub name: String,
    pub source: String,
    pub engine: EngineType,
}

/// Supported scripting engines.
pub enum EngineType {
    Lua,
    #[cfg(feature = "engine-v8")]
    JavaScript,
}

/// Execution context passed to scripts.
pub struct ScriptContext {
    pub agent_id: String,
    pub globals: HashMap<String, ScriptValue>,
    pub timeout: Duration,
    pub fs_config: FsBridgeConfig,
    pub net_config: NetBridgeConfig,
    pub permissions: PermissionSet,
}

pub type ScriptValue = serde_json::Value;

pub struct ScriptError {
    pub code: String,
    pub message: String,
    pub stack_trace: Option<String>,
}

/// Bridge exposing kernel capabilities to scripts.
pub struct RustBridge {
    pub tools: ToolsBridge,
    pub memory: MemoryBridge,
    pub events: EventsBridge,
    pub fs: FsBridge,
    pub agent: AgentBridge,
    pub dirs: DirsBridge,
}
```

**Dependencies:**
- `mlua` (optional) — Lua binding
- `deno_core` (optional) — V8 embedding

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
| `claw-memory` | 2 | Memory system - NgramEmbedder, SqliteMemoryStore, SecureMemoryStore (optional reference implementation) |
| `claw-channel` | 2 | Channel integrations - Discord, Webhook, Stdin (optional) |

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

- Current version: `1.0.0`
- All crates share the same version number
- Breaking changes bump all crates

This simplifies dependency management for users.

---

## Feature Flag Guidelines

### When to Use Features

1. **Platform-specific functionality** that pulls in heavy deps
   - Example: `engine-v8` (100MB+ binary impact)

2. **Optional script engines**

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
