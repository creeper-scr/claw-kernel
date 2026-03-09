---
title: claw-kernel Architecture Overview
description: Complete 5-layer architecture documentation for claw-kernel
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---


# claw-kernel Architecture Overview

> The Claw Ecosystem's Foundation — A Cross-Platform Agent Kernel Built in Rust

---

## Table of Contents

- [Design Philosophy](#design-philosophy)
- [The 5-Layer Architecture](#the-5-layer-architecture)
- [Memory System Architecture (3-Tier Model)](#memory-system-architecture)
- [Key Architectural Decisions](#key-architectural-decisions)
- [Cross-Platform Strategy](#cross-platform-strategy)
- [Extensibility Architecture](#extensibility-architecture)
- [Security Model](#security-model)

---

## Design Philosophy

### Why Rust Kernel + Scripting?

The Claw ecosystem has seen 8+ implementations independently solving the same problems:

| Project | Language | Lines | Problem |
|---------|----------|-------|---------|
| OpenClaw | TypeScript | 430K | Reimplements primitives repeatedly |
| ZeroClaw | Rust | 150K | Missing shared abstractions |
| PicoClaw | Go | 50K | Platform-specific code |
| Nanobot | Python | 4K | No path to production performance |

**claw-kernel** extracts these common primitives into a shared foundation:

```
Rust Kernel = Linux kernel (stable core, memory safety)
Script Layer = Userland programs (hot-swappable, extensible)
```

### Core Principles

1. **Immutable Core, Mutable Scripts**
   - Rust code is stable, tested, and never hot-patched
   - All extensible logic lives in scripts (Lua/Deno/Python)

2. **Cross-Platform First**
   - No Unix-isms, no Windows-isms in core code
   - Platform-specific behavior isolated in `claw-pal`

3. **Extensibility by Design**
   - Kernel provides foundation for runtime extension
   - Hot-loading without restart

4. **Dual Mode Security**
   - Safe Mode: Sandboxed by default
   - Power Mode: Full access, explicit opt-in

---

## The 5-Layer Architecture

> 中文：五层架构

```
┌─────────────────────────────────────────────────────────┐
│                    Kernel Layer                          │
│  ┌─────────────────────────────────────────────────────┐│
│  │  Layer 3: Extension Foundation                      ││
│  │  Extension Foundation · Hot-loading · Dynamic Reg.  ││
│  │  Lua (default) · Deno/V8 (full) · PyO3 (ML)         ││
│  │                                                     ││
│  │  Architecture: Independent + Bridge (via IPC)       ││
│  │  - claw-script runs in separate per-Agent process   ││
│  │  - Communicates with kernel via RustBridge (IPC)    ││
│  │  - Low coupling: kernel does not depend on scripts  ││
│  │  - Provides crash isolation (script crash ≠ agent)  ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  Layer 2: Agent Kernel Protocol                     ││
│  │  Provider Trait · ToolRegistry · AgentLoop · History││
│  │  claw-memory (LTM: Working / Episodic / Semantic)   ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  Layer 1: System Runtime                            ││
│  │  Event Bus · IPC Transport · Process Daemon · Tokio ││
│  └─────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────┐│
│  │  Layer 0.5: Platform Abstraction Layer (PAL)        ││
│  │  Sandbox Backend · IPC Transport · Config Dirs      ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  Layer 0: Rust Hard Core                            ││
│  │  Memory Safety · OS Abstraction · Trust Root        ││
│  └─────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────┘
```

### Layer 0: Rust Hard Core

The **trust root** — immutable, never hot-patched, no platform assumptions.

**Responsibilities:**
- Process lifecycle management
- Secure credential storage (in-memory encryption)
- Script engine bootstrap / Runtime initialization
- Mode switching guards (Safe ↔ Power)

**Key Constraint:** This layer cannot be modified by scripts. Ever.

### Layer 0.5: Platform Abstraction Layer (PAL)

Isolates platform-specific code from the rest of the system.

| Component | Linux | macOS | Windows (v0.1.0) |
|-----------|-------|-------|-------------------|
| Sandbox | seccomp-bpf + Namespaces | sandbox(7) profile | AppContainer (stub) |
| IPC | Unix Domain Socket | Unix Domain Socket | Named Pipe (v0.2.0) |
| Process | fork()/exec() | fork()/exec() | CreateProcess() |
| Config | XDG dirs | ~/Library | %APPDATA% |

See [PAL Deep Dive](pal.md) for implementation details.

### Layer 1: System Runtime

The async foundation built on Tokio.

**Event Bus:**
```rust
// Central message bus for all components
pub enum Event {
    // Agent lifecycle
    AgentStarted { agent_id: AgentId },
    AgentStopped { agent_id: AgentId, reason: String },

    // LLM interaction
    LlmRequestStarted { agent_id: AgentId, provider: String },
    LlmRequestCompleted { agent_id: AgentId, prompt_tokens: u64, completion_tokens: u64 },

    // Message handling
    MessageReceived { agent_id: AgentId, channel: String, message_type: String },

    // Tool usage
    ToolCalled { agent_id: AgentId, tool_name: String, call_id: String },
    ToolResult { agent_id: AgentId, tool_name: String, call_id: String, success: bool },

    // Memory system
    ContextWindowApproachingLimit { agent_id: AgentId, token_count: u64, token_limit: u64 },
    MemoryArchiveComplete { agent_id: AgentId, archived_count: usize },

    // Security
    ModeChanged { agent_id: AgentId, to_power_mode: bool },

    // Extension events
    Extension(ExtensionEvent),

    // ── Agent-to-Agent messaging ─────────────────────────────────────────────
    /// Emitted when an A2A message is received via IPC.
    A2A(A2AMessage),

    // System
    Shutdown,
}

/// Agent-to-Agent message for inter-agent communication
pub struct A2AMessage {
    pub id: String,                          // Unique message ID
    pub source: AgentId,                     // Source agent
    pub target: Option<AgentId>,             // Target agent (None for broadcast)
    pub message_type: A2AMessageType,        // Request/Response/Event/Discovery/etc
    pub payload: A2AMessagePayload,          // Message content
    pub priority: MessagePriority,           // Critical/High/Normal/Low/Background
    pub correlation_id: Option<String>,      // For request/response matching
    pub timestamp: u64,                      // Unix timestamp in milliseconds
    pub ttl_secs: Option<u32>,               // Time-to-live (None for no expiry)
}

pub enum MessagePriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Background = 4,
}

/// Newtype wrapper for agent identifiers. Provides type safety over raw strings.
/// The inner string is accessible via `.0`.
pub struct AgentId(pub String);

pub enum A2AMessageType {
    Request,          // Expects response
    Response,         // Response to request
    Event,            // Fire-and-forget
    DiscoveryRequest, // Query for available capabilities
    DiscoveryResponse,// Reply to discovery request
    Heartbeat,        // Keep-alive
    Error,            // Indicates a processing error
}

/// Extension-related events
pub enum ExtensionEvent {
    ToolLoading { name: String },
    ToolLoaded { name: String, result: Result<(), LoadError> },
    ToolUnloaded { name: String },
    ScriptReloaded { path: PathBuf, result: Result<(), ReloadError> },
    ProviderRegistered { name: String },
}
```

**Process Management:**
- Subagent lifecycle (spawn / kill / list / steer)
- Health checking and auto-restart
- Resource quotas (CPU/memory)

> **注意**: IPC 远程消息投递当前未实现，仅支持本地进程内通信。Windows IPC 计划在 v0.2.0 中实现。

### Layer 2: Agent Kernel Protocol

> **Kernel Positioning**: Layer 2 provides mechanisms, not policies.
> - It gives you `AgentLoop`, not an opinionated agent
> - It gives you `HistoryManager`, not a fixed memory strategy
> - It gives you `ToolRegistry`, not pre-selected tools
>
> You compose these primitives in your application layer to build your product.

The **heart** of the system — where all Claw projects had been reinventing wheels.

#### Provider Abstraction

The provider system uses a **three-layer architecture** to maximize code reuse across the fragmented LLM API landscape:

```
Level 3: LLMProvider trait      ← User-facing interface (complete, stream_complete)
Level 2: HttpTransport trait    ← Reusable HTTP logic (request, stream_request)
Level 1: MessageFormat trait    ← Protocol abstraction (OpenAIFormat, AnthropicFormat)
```

**Level 1: MessageFormat (Protocol Abstraction)**

The market has consolidated around two dominant formats:
- **OpenAI Format** — Used by OpenAI, DeepSeek, Moonshot, Qwen, Grok, and 50+ providers
- **Anthropic Format** — Used by Anthropic (Claude) and AWS Bedrock

```rust
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
```

**Level 2: HttpTransport (Reusable Logic)**

```rust
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    async fn request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<CompletionResponse, ProviderError> {
        // Generic HTTP logic reused by ALL providers
    }
    
    async fn stream_request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}
```

**Level 3: LLMProvider (User Interface)**

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    fn provider_id(&self) -> &str;           // Provider identifier (e.g., "anthropic")
    fn model_id(&self) -> &str;              // Default model ID
    async fn complete(&self, messages: Vec<Message>, options: Options) -> Result<CompletionResponse, ProviderError>;
    async fn complete_stream(&self, messages: Vec<Message>, options: Options) 
        -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError>;
    fn token_count(&self, text: &str) -> usize {  // Rough estimate: chars / 4
        text.len() / 4
    }
}

// 嵌入接口（单独 trait，因为不是所有 provider 都支持）
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding, ProviderError>;
    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError>;
}
```

#### Basic Type Definitions

```rust
// Basic message types
pub struct Message {
    pub role: Role,  // system/user/assistant/tool
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

// LLM call options
pub struct Options {
    pub model: String,                      // Model identifier (e.g., "claude-opus-4-6", "gpt-4o")
    pub max_tokens: u32,                    // Maximum tokens to generate, default: 4096
    pub temperature: f32,                   // Sampling temperature (0.0–2.0), default: 0.7
    pub stream: bool,                       // Enable streaming response, default: false
    pub system: Option<String>,             // System prompt (overrides system message in list)
    pub stop_sequences: Vec<String>,        // Stop sequences, default: empty
    pub tools: Option<Vec<ToolDef>>,        // Available tools for function calling
    pub timeout_seconds: u64,               // Request timeout in seconds, default: 60
    pub max_retries: u32,                   // Max retry attempts, default: 3
}

// Streaming response delta
pub struct Delta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

// Tool call
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}
```

**Built-in implementations:**

| Provider | Format | Code Complexity | Status |
|----------|--------|-----------------|--------|
| `AnthropicProvider` | AnthropicFormat | ~20 lines (config) | ✅ Implemented |
| `OpenAIProvider` | OpenAIFormat | ~20 lines (config) | ✅ Implemented |
| `DeepSeekProvider` | OpenAIFormat | ~20 lines (config) | ✅ Implemented |
| `MoonshotProvider` | OpenAIFormat | ~20 lines (config) | ✅ Implemented |
| `OllamaProvider` | OllamaFormat | ~25 lines (config) | ✅ Implemented |
| `QwenProvider` | OpenAIFormat | ~20 lines (config) | 🚧 Planned |
| `GrokProvider` | OpenAIFormat | ~20 lines (config) | 🚧 Planned |
| `ScriptableProvider` | Custom via script | Runtime defined | 🚧 Planned |

> **Note:** Providers marked with 🚧 are not yet implemented.
> The kernel's provider architecture supports them; they will be added in future releases.

> 90% code reduction: Adding a new OpenAI-compatible provider requires only configuration (base URL + auth), not HTTP implementation.

See [ADR-006](../adr/006-message-format-abstraction.md) for the full architectural decision.

#### Tool Registry
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;           // Tool description for LLM
    fn schema(&self) -> &ToolSchema;
    fn permissions(&self) -> &PermissionSet;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }  // Default timeout
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

/// JSON Schema for tool parameters
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Result of tool execution
pub struct ToolResult {
    pub success: bool,                      // Whether execution succeeded
    pub output: Option<serde_json::Value>,  // Success output
    pub error: Option<ToolError>,           // Error information
    pub duration_ms: u64,                   // Execution duration
}

pub struct ToolError {
    pub code: ToolErrorCode,                // Error code enum
    pub message: String,                    // Human-readable message
    pub details: Option<serde_json::Value>, // Additional error details
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

pub struct LogEntry {
    pub timestamp_ms: u64,      // Unix timestamp in milliseconds
    pub agent_id: String,       // ID of the calling agent
    pub tool_name: String,      // Name of the tool executed
    pub success: bool,          // Whether execution succeeded
    pub duration_ms: u64,       // Execution duration in milliseconds
}

/// Permission set for tool execution
pub struct PermissionSet {
    pub filesystem: FsPermissions,
    pub network: NetworkPermissions,
    pub subprocess: SubprocessPolicy,
}

pub enum FsPermissions {
    ReadOnly(Vec<PathBuf>),                  // Allowlisted read-only paths
    ReadWrite(Vec<PathBuf>),                 // Allowlisted read-write paths
    None,                                    // No filesystem access
}

pub struct NetworkPermissions {
    pub allowed_domains: HashSet<String>,    // Supports wildcards like "*.example.com"
    pub allowed_ports: Vec<u16>,             // Allowed ports (applies to all domains)
    pub allow_localhost: bool,               // Allow localhost connections
    pub allow_private_ips: bool,             // Allow private IP ranges
}

impl Default for NetworkPermissions {
    fn default() -> Self {
        Self {
            allowed_domains: HashSet::new(),
            allowed_ports: vec![443, 80],      // Default: HTTPS and HTTP
            allow_localhost: true,
            allow_private_ips: false,
        }
    }
}

pub enum SubprocessPolicy {
    Allow { 
        allowed_commands: Vec<String>,       // Allowlisted commands
        max_concurrent: usize,               // Maximum concurrent subprocesses
    },
    Deny,
}

pub struct ToolRegistry {
    // Internal implementation uses DashMap for thread-safe access
}

impl ToolRegistry {
    /// Register a native Rust tool.
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;

    /// Unregister a tool by name.
    pub fn unregister(&self, name: &str) -> Result<(), RegistryError>;

    /// Get a registered tool.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;

    /// List all registered tool metadata.
    pub fn list(&self) -> Vec<ToolMeta>;

    /// Execute a tool with given arguments and context.
    pub async fn execute(&self, name: &str, args: serde_json::Value, ctx: ToolContext)
        -> Result<ToolResult, RegistryError>;

    /// Get recent audit log entries.
    pub async fn recent_log(&self, n: usize) -> Vec<LogEntry>;

    // Hot-loading support
    pub async fn load_from_script(&self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub async fn load_from_directory(&self, path: &Path) -> Result<Vec<ToolMeta>, LoadError>;
    pub fn unload(&self, name: &str) -> Result<(), RegistryError>;
    pub async fn enable_hot_loading(&self, config: HotLoadingConfig) -> Result<(), WatchError>;
    pub async fn disable_hot_loading(&self);
}

pub struct ToolMeta {
    pub name: String,
    pub description: String,
    pub schema: ToolSchema,
    pub permissions: PermissionSet,
    pub source: ToolSource,
}

pub enum ToolSource {
    Native,
    Script { path: PathBuf, language: ScriptLanguage },
    Dynamic { id: Uuid },
}

pub enum ScriptLanguage {
    Lua,
    TypeScript,
    Python,
}

pub struct HotLoadingConfig {
    pub debounce_ms: u64,                   // Debounce interval, default: 500ms
    pub watch_paths: Vec<PathBuf>,          // Directories to watch
    pub exclude_patterns: Vec<String>,      // Exclude patterns, e.g., ["*.tmp"]
}

pub enum RegistryError {
    AlreadyExists(String),
    NotFound(String),
    ExecutionFailed(ToolError),
}

pub enum LoadError {
    IoError(std::io::Error),
    ParseError { path: PathBuf, message: String },
    InvalidSchema(String),
    PermissionValidationFailed(String),
}
```

#### Agent Loop
```rust
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    tools: Arc<ToolRegistry>,
    history: Box<dyn HistoryManager>,
    stop_conditions: Vec<Box<dyn StopCondition>>,
    summarizer: Box<dyn Summarizer>,
    config: AgentLoopConfig,
}

pub struct AgentLoopConfig {
    /// Default: 20 — Prevents runaway loops, adjustable for complex tasks
    pub max_turns: u32,
    /// Default: 0 (unlimited) — Token budget for the session
    pub token_budget: u64,
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Whether to enable tool use. Default: true
    pub tool_use_enabled: bool,
    /// Default: 30s — Most API calls complete within 10s, buffer for network variance
    pub tool_timeout_seconds: u64,
    /// Default: 10 — Prevents runaway recursive calls, based on common workflow complexity
    pub max_tool_calls_per_turn: usize,
    /// Enable streaming response. Default: false
    pub enable_streaming: bool,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_turns: 20,
            token_budget: 0,
            system_prompt: None,
            tool_use_enabled: true,
            tool_timeout_seconds: 30,
            max_tool_calls_per_turn: 10,
            enable_streaming: false,
        }
    }
}

impl AgentLoop {
    /// Create a new builder for configuring the agent loop.
    pub fn builder() -> AgentLoopBuilder {
        AgentLoopBuilder::new()
    }
    pub async fn run(&mut self, initial_message: impl Into<String>) -> Result<AgentResult>;
    pub fn history(&self) -> &dyn HistoryManager;
    pub fn clear_history(&mut self);
}

pub struct AgentResult {
    /// Why the loop finished.
    pub finish_reason: FinishReason,
    /// The last assistant message.
    pub last_message: Option<Message>,
    /// Total token usage.
    pub usage: TokenUsage,
    /// Total turns executed.
    pub turns: u32,
    /// Final response content (convenience accessor).
    pub content: String,
    /// All tool calls executed across all turns.
    pub tool_calls: Vec<ToolCall>,
    /// Wall-clock execution time in milliseconds.
    pub execution_time_ms: u64,
}

pub enum FinishReason {
    Stop,          // LLM returned stop with no tool calls
    MaxTurns,      // Reached max_turns limit
    TokenBudget,   // Exceeded token_budget
    NoToolCall,    // LLM made no tool calls (stop condition)
    StopCondition, // Custom stop condition triggered
    Error,         // Loop stopped due to error
}

pub enum StreamChunk {
    Text { content: String, is_final: bool },
    ToolStart { id: String, name: String },
    ToolArguments { id: String, arguments: String },
    ToolComplete { id: String, result: serde_json::Value },
    ToolError { id: String, error: String },
    UsageUpdate(TokenUsage),
    Finish(FinishReason),
    Error(String),
}

pub struct ConversationContext {
    pub system: Option<String>,
    pub history: Vec<Message>,
    pub initial_prompt: String,
}

impl From<&str> for ConversationContext {
    fn from(prompt: &str) -> Self {
        Self {
            system: None,
            history: vec![],
            initial_prompt: prompt.to_string(),
        }
    }
}
```

#### Long-Term Memory (`claw-memory`)

> **Design Principle — Mechanism vs. Policy Separation**
> The Rust kernel (Layer 2) is responsible only for *how* to store and retrieve memories safely and efficiently (Mechanism). All lifecycle rules, summarization algorithms, and retrieval prompts are defined in Layer 3 scripts (Policy), enabling Agent self-evolution without kernel changes.

`claw-memory` exposes a core trait `MemoryStore` with four capabilities:

```rust
/// Core trait for the long-term memory subsystem (Layer 2)
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a memory item. Returns the assigned ID.
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;

    /// Retrieve a specific item by ID.
    async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError>;

    /// Search episodic history with a filter.
    async fn search_episodic(
        &self,
        filter: &EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError>;

    /// Semantic search: find items whose embeddings are closest to the query vector.
    async fn semantic_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError>;

    /// Delete a memory item.
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError>;

    /// Clear all items in a namespace.
    async fn clear_namespace(&self, namespace: &str) -> Result<usize, MemoryError>;

    /// Total storage used by a namespace, in bytes (approximate).
    async fn namespace_usage(&self, namespace: &str) -> Result<u64, MemoryError>;
}

pub struct MemoryItem {
    pub id: MemoryId,
    pub content: String,
    pub score: f32,                         // Cosine similarity
    pub metadata: serde_json::Value,
    pub created_at: SystemTime,
}

pub struct EpisodicEntry {
    pub kind: EpisodeKind,                  // ToolCall | LLMResponse | Error | Custom
    pub content: serde_json::Value,
    pub tags: Vec<String>,
    pub timestamp: SystemTime,
}

pub struct EpisodicFilter {
    pub since: Option<SystemTime>,
    pub until: Option<SystemTime>,
    pub tags: Vec<String>,
    pub limit: Option<usize>,
}

pub enum EpisodeKind {
    ToolCall,
    LLMResponse,
    Error,
    Custom(String),
}
```

**Dependencies:**
- `claw-provider` — Embedding computation (`EmbeddingProvider::embed`)
- `claw-pal` — Sandboxed filesystem access (database path enforcement)
- Works closely with `claw-loop` — Working Memory overflow triggers persistence via EventBus

**Feature Flags:**
```toml
[features]
default = ["sqlite-vec"]          # Zero-dependency vector store
sqlite-vec = ["rusqlite"]         # Default: SQLite + sqlite-vec extension
qdrant = ["qdrant-client"]        # ml-ready: external Qdrant cluster
```

---

### Layer 3: Extension Foundation

Multi-engine support with unified interface. This layer provides the **foundation for extensibility** — hot-loading (热加载), script execution, and dynamic registration capabilities that enable upper layers to evolve.

| Engine | Binary Size | Strength | Use Case |
|--------|-------------|----------|----------|
| **Lua (mlua)** | ~500KB | Zero deps, fast | Default, simple tools |
| **Deno/V8** | ~100MB | Full TS/JS, strong sandbox | Complex agents |
| **PyO3** | Varies | ML ecosystem | Data/ML tools |

**Key Capabilities:**
- **Hot-loading**: Load/unload scripts without restart
- **Permission Bridge**: Enforce security boundaries for scripts
- **Type Safety Layer**: TypeScript-style interfaces over Rust

**RustBridge (Script Layer API — `claw.*` namespace):**
```typescript
// Exposed to scripts via claw.* namespace (Layer 3 → Layer 2 bridge)
interface RustBridge {
  llm: {
    complete(messages: Message[]): Promise<Response>;
    stream(messages: Message[]): AsyncIterable<Delta>;
  };
  tools: {
    register(def: ToolDef): void;
    call(name: string, params: any): Promise<any>;
    list(): ToolMeta[];
  };
  // claw.memory.* — Policy defined in scripts, Mechanism provided by claw-memory (Layer 2)
  // Agent decides WHAT to memorize and WHEN; the kernel handles HOW.
  //
  // Typical usage pattern:
  //   on_think:   relevant = await claw.memory.search(current_topic, 5)
  //   on_observe: if valuable, await claw.memory.memorize({ content, space: "knowledge" })
  memory: {
    // Semantic memory (RAG) — cross-session, de-temporalized knowledge
    search(query: string, topK: number): Promise<MemoryItem[]>;
    memorize(item: { content: string; metadata?: any; space?: string }): Promise<string>;
    // Episodic memory — time-series action log (tool calls, errors, reflections)
    logEpisode(entry: { kind: string; content: any; tags?: string[] }): Promise<string>;
    queryEpisodes(filter: { since?: Date; until?: Date; tags?: string[]; limit?: number }): Promise<EpisodicEntry[]>;
  };
  events: { emit(event: string, data: any): void; on(event: string, handler: Function): void };
  fs: { read(path: string): Promise<Buffer>; write(path: string, data: Buffer): Promise<void> };
  agent: { spawn(config: AgentConfig): Promise<AgentHandle>; kill(handle: AgentHandle): Promise<void> };
}
```

---

<a name="memory-in-kernel"></a>
### Memory in Kernel

The kernel provides only **short-term memory** via `HistoryManager`:

| Aspect | Kernel Provides | Application Provides |
|--------|----------------|---------------------|
| Short-term | `HistoryManager` trait, `InMemoryHistory`, `SqliteHistory` | - |
| Overflow handling | `set_overflow_callback()` hook | Archive policy (file/DB/API) |
| Long-term | ❌ Nothing | Application implements |

**Extension Example**:

```rust
// Application implements long-term memory
let mut history = InMemoryHistory::new(8192);
history.set_overflow_callback(Box::new(|current, limit| {
    // Your policy: archive to DB, write to file, or discard
    my_archive_system.save_overflow(current, limit);
}));
```

See [ADR-010](../adr/010-memory-system-boundary.md) for full rationale.

---

## Key Architectural Decisions

### 1. Why Lua as Default Engine?

**Decision:** Lua (via mlua) is the default script engine.

**Rationale:**
- Zero dependencies (pure Rust binding)
- Fast compilation (<1 min)
- Small runtime (<500KB)
- Cross-platform out of the box
- Sufficient for most tool logic

**Alternatives considered:**
- Deno/V8: Too large, complex build
- Wasmer/WASM: Good sandbox but tooling immature

### 2. Why Separate PAL Layer?

**Decision:** Platform-specific code isolated in `claw-pal` crate.

**Rationale:**
- Forces cross-platform thinking
- Makes platform gaps visible
- Enables platform-specific optimization without leaking

### 3. Why Two Security Modes?

**Decision:** Explicit Safe/Power mode distinction.

**Rationale:**
- Most agent tasks don't need full system access
- Safe Mode enables "trust but verify" deployment
- Power Mode enables full automation when needed
- Clear mental model for users

See [ADR-003: Security Model](../adr/003-security-model.md) for full analysis.

---

## Terminology Reference

For detailed terminology definitions, please refer to [AGENTS.md](../../AGENTS.md) and [docs/terminology.md](../terminology.md).

---

## Cross-Platform Strategy

### Philosophy: Zero Platform Assumptions

All code at Layer 0-3 is **platform-agnostic**. Platform specifics only in:
- `claw-pal` crate
- PAL trait implementations
- Platform-specific tests

### Platform Capability Matrix

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| Safe Mode | Yes Strong | Yes Medium | Yes Medium |
| Power Mode | Yes Full | Yes Full | Yes Full |
| IPC Performance (relative to Linux UDS) | 100% | 95% | 90% |
| Process Isolation | Strongest | Medium | Medium |
| Build Complexity | Low | Low | Medium |

### Handling Platform Differences

```rust
// claw-pal/src/sandbox/mod.rs
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

// Platform-specific sandbox implementations
#[cfg(target_os = "linux")]
pub type DefaultSandboxBackend = LinuxSandbox;
#[cfg(target_os = "macos")]
pub type DefaultSandboxBackend = MacSandbox;
#[cfg(target_os = "windows")]
pub type DefaultSandboxBackend = WindowsSandbox;

/// Factory trait for creating sandbox instances
pub trait SandboxFactory: Send + Sync {
    type Backend: SandboxBackend;
    type Error: std::error::Error;
    
    fn create(config: SandboxConfig) -> Result<Self::Backend, Self::Error>;
}

// See [PAL Architecture](pal.md) for the complete SandboxBackend trait definition.
// The trait is defined in pal.md to avoid duplication and ensure consistency.
```

See [Platform Guides](../platform/) for OS-specific details.

---

## Extensibility Architecture

### Kernel Responsibilities

The kernel provides the foundation for extensibility:

```
┌─────────────────────────────────────────────────────────┐
│  KERNEL LAYER (Provides Foundations)                    │
├─────────────────────────────────────────────────────────┤
│  Extension Foundation      Hot-loading API              │
│  (Layer 3)                 Script Runtime               │
│                            Permission Bridge            │
├─────────────────────────────────────────────────────────┤
│  Agent Kernel Protocol     Tool Registry                │
│  (Layer 2)                 Dynamic Registration         │
│                            LLM Provider Abstraction     │
└─────────────────────────────────────────────────────────┘
```

### What Can Be Extended at Runtime?

```
┌────────────────────────────────────────┐
│          EXTENSIBLE AT RUNTIME         │
├────────────────────────────────────────┤
│ Tool scripts                           │
│ Custom providers                       │
│ Memory strategies                      │
│ Stop conditions                        │
│ Channel adapters                       │
└────────────────────────────────────────┘

┌────────────────────────────────────────┐
│       CANNOT BE MODIFIED               │
├────────────────────────────────────────┤
│ Rust kernel code                       │
│ Sandbox enforcement                    │
│ Mode switching guards                  │
│ Credential storage                     │
│ Script Engine Runtime (Layer 3) [^1]   │
└────────────────────────────────────────┘

[^1]: The script engine runtime cannot be modified, but script content can be hot-loaded.
```

### Hot-Loading Mechanism

The kernel provides hot-loading as a **foundation capability**:

```rust
// Simplified hot-load flow (Kernel Layer 3)
pub async fn hot_load_tool(&mut self, path: &Path) -> Result<()> {
    // 1. Read and validate script
    let script = fs::read_to_string(path).await?;
    let validated = self.validate(&script)?;
    
    // 2. Check permissions (Safe Mode only)
    if self.mode == ExecutionMode::Safe {
        self.audit_permissions(&validated)?;
    }
    
    // 3. Compile in isolated context
    let compiled = self.script_engine.compile(&script)?;
    
    // 4. Register with ToolRegistry
    let tool = ScriptTool::new(compiled, self.bridge.clone());
    self.registry.register(tool)?;
    
    // 5. Emit event
    self.events.emit(Event::ToolLoaded { name: tool.name() });
    
    Ok(())
}
```

Applications built on the kernel can use these capabilities to implement custom extensibility patterns.

See [Extension Capabilities Guide](../guides/extension-capabilities.md) for usage.

---

## Security Model

### Two-Mode Security

| Dimension | Safe Mode | Power Mode |
|-----------|-----------|------------|
| File System | Allowlist read-only | Full access |
| Network | Domain/port rules | Unrestricted |
| Subprocess | Blocked | Allowed |
| Script Extension | Allowed (sandboxed) | Allowed (global) |
| Kernel Access | Blocked | Blocked (hard constraint) |
| Memory Isolation | DB locked to `/sandbox/agent_{id}/memory.db`; A2A memory access strictly forbidden; disk quota enforced | Cross-path reads allowed; shared memory spaces permitted; Shell-level access to external/enterprise database clusters |

### Mode Switching

```
┌─────────────┐      power-key + explicit flag      ┌─────────────┐
│  Safe Mode  │  ─────────────────────────────────► │  Power Mode │
│  (default)  │                                     │  (opt-in)   │
└─────────────┘                                     └─────────────┘
       ▲                                                    │
       │              restart or new process                │
       └────────────────────────────────────────────────────┘
```

**Important:** Power Mode → Safe Mode requires restart. This is intentional — a compromised Power Mode agent cannot "downgrade" to hide evidence.

### Audit Trail

All extension actions are logged:

```json
{
  "timestamp": "2024-01-15T10:30:00Z",
  "event": "tool_loaded",
  "tool_name": "file_search",
  "source": "self_generated",
  "permissions_requested": ["fs.read", "fs.write"],
  "execution_mode": "safe",
  "agent_id": "agent-123"
}
```

---

## Next Steps

- **For users:** [Getting Started Guide](../guides/getting-started.md)
- **For contributors:** [Contributing Guide](../../CONTRIBUTING.md)
- **For architecture details:** [Crate Map](crate-map.md), [ADR Index](../adr/)
- **For platform specifics:** [Linux](../platform/linux.md), [macOS](../platform/macos.md), [Windows](../platform/windows.md)

---
