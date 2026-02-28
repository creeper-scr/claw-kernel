---
title: claw-kernel Architecture Overview
description: Complete 5-layer architecture documentation for claw-kernel
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
language: bilingual
---

> **Project Status**: Design/Planning Phase — Architecture is documented but implementation has not started.

[English](#english) | [中文](#chinese)

<a name="english"></a>
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

| Component | Linux | macOS | Windows |
|-----------|-------|-------|---------|
| Sandbox | seccomp-bpf + Namespaces | sandbox(7) profile | AppContainer + Job Objects |
| IPC | Unix Domain Socket | Unix Domain Socket | Named Pipe |
| Process | fork()/exec() | fork()/exec() | CreateProcess() |
| Config | XDG dirs | ~/Library | %APPDATA% |

See [PAL Deep Dive](pal.md) for implementation details.

### Layer 1: System Runtime

The async foundation built on Tokio.

**Event Bus:**
```rust
// Central message bus for all components
pub enum Event {
    UserInput(Message),
    AgentOutput(Response),
    ToolCall(ToolInvocation),
    ToolResult(ToolOutput),
    AgentLifecycle(AgentState),
    Extension(ExtensionEvent),
    A2A(A2AMessage),
}

/// Agent-to-Agent message for inter-agent communication
pub struct A2AMessage {
    pub from: AgentId,
    pub to: Option<AgentId>,         // None = broadcast
    pub message_type: A2AMessageType,
    pub payload: Payload,            // Serialized message payload
    pub correlation_id: Option<Uuid>,// For request-response correlation
    pub timeout: Option<Duration>,   // Message timeout, default: 30s
    pub priority: MessagePriority,   // Message priority, default: Normal
    pub timestamp: SystemTime,       // Message creation time
}

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

pub type AgentId = String;

pub enum A2AMessageType {
    Request,    // Expects response
    Response,   // Response to request
    Event,      // Fire-and-forget
    Command,    // Directive (parent to child)
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

### Layer 2: Agent Kernel Protocol

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
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

// 嵌入接口（单独 trait，因为不是所有 provider 都支持）
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError>;
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
    pub model: Option<String>,              // Model identifier, None = use provider default
    pub temperature: Option<f32>,           // Range: 0.0-2.0, default: 1.0
    pub max_tokens: Option<usize>,          // Maximum tokens to generate, default: 4096
    pub stop_sequences: Vec<String>,        // Stop sequences, default: empty
    pub tools: Option<Vec<ToolDef>>,        // Available tools for function calling
    pub timeout: Duration,                  // Request timeout, default: 60s
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

| Provider | Format | Code Complexity |
|----------|--------|-----------------|
| `AnthropicProvider` | AnthropicFormat | ~20 lines (config) |
| `BedrockProvider` | AnthropicFormat + AWS auth | ~30 lines (config) |
| `OpenAIProvider` | OpenAIFormat | ~20 lines (config) |
| `DeepSeekProvider` | OpenAIFormat | ~20 lines (config) |
| `MoonshotProvider` | OpenAIFormat | ~20 lines (config) |
| `QwenProvider` | OpenAIFormat | ~20 lines (config) |
| `GrokProvider` | OpenAIFormat | ~20 lines (config) |
| `OllamaProvider` | OllamaFormat (OpenAI variant) | ~25 lines (config) |
| `ScriptableProvider` | Custom via script | Runtime defined |

> 90% code reduction: Adding a new OpenAI-compatible provider requires only configuration (base URL + auth), not HTTP implementation.

See [ADR-006](../adr/006-message-format-abstraction.md) for the full architectural decision.

#### Tool Registry
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

/// JSON Schema for tool parameters
pub type ToolSchema = serde_json::Value; // JSON Schema as JSON

/// Result of tool execution
pub struct ToolResult {
    pub output: Option<serde_json::Value>,  // Success output
    pub error: Option<ToolError>,           // Error information
    pub logs: Vec<LogEntry>,                // Execution logs with timestamps
    pub execution_time_ms: u64,             // Execution duration
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
    pub timestamp: SystemTime,
    pub level: LogLevel,
    pub message: String,
}

pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
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
    pub allowed_domains: Vec<String>,        // Supports wildcards like "*.example.com"
    pub allowed_ports: Vec<u16>,             // Allowed ports (applies to all domains)
    pub allow_localhost: bool,               // Allow localhost connections
    pub allow_private_ips: bool,             // Allow private IP ranges
}

impl Default for NetworkPermissions {
    fn default() -> Self {
        Self {
            allowed_domains: vec![],
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
    tools: HashMap<String, RegisteredTool>,
    hot_loading: Option<HotLoadingWatcher>,
}

pub struct RegisteredTool {
    pub tool: Box<dyn Tool>,
    pub source: ToolSource,
    pub loaded_at: SystemTime,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;
    pub fn unregister(&mut self, name: &str) -> Result<(), RegistryError>;
    pub fn get(&self, name: &str) -> Option<&dyn Tool>;
    pub fn list(&self) -> Vec<&ToolMeta>;
    pub async fn execute(&self, name: &str, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    
    // Hot-loading support
    pub async fn load_from_script(&mut self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub async fn load_from_directory(&mut self, path: &Path) -> Result<Vec<ToolMeta>, LoadError>;
    pub fn unload(&mut self, name: &str) -> Result<(), RegistryError>;
    pub async fn enable_hot_loading(&mut self, config: HotLoadingConfig) -> Result<(), WatchError>;
    pub fn disable_hot_loading(&mut self);
}

pub struct ToolMeta {
    pub name: String,
    pub description: String,
    pub version: String,
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
    /// Default: 50 — Based on Claude 3.5 Sonnet average interaction depth for complex tasks (P95)
    pub max_turns: Option<usize>,
    /// Default: 8000 — ~6K input + 2K output, suitable for Claude 3.5 Sonnet context window
    pub token_budget: Option<usize>,
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Default: true — Streaming improves perceived responsiveness
    pub enable_streaming: bool,
    /// Default: 30s — Most API calls complete within 10s, buffer for network variance
    pub tool_timeout: Duration,
    /// Default: 10 — Prevents runaway recursive calls, based on common workflow complexity
    pub max_tool_calls_per_turn: usize,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_turns: Some(50),
            token_budget: Some(8000),
            system_prompt: None,
            enable_streaming: true,
            tool_timeout: Duration::from_secs(30),
            max_tool_calls_per_turn: 10,
        }
    }
}

impl AgentLoop {
    pub fn builder() -> AgentLoopBuilder;
    pub async fn run(&mut self, context: impl Into<ConversationContext>) -> Result<AgentResult>;
    pub async fn stream_run(&mut self, context: impl Into<ConversationContext>) -> Result<BoxStream<'static, StreamChunk>>;
    pub fn history(&self) -> &dyn HistoryManager;
    pub fn clear_history(&mut self);
}

pub struct AgentResult {
    pub content: String,                    // Final response content
    pub tool_calls: Vec<ToolCall>,          // All tool call records
    pub turns: usize,                       // Actual conversation turns
    pub token_usage: TokenUsage,            // Token usage statistics
    pub finish_reason: FinishReason,        // Reason for completion
    pub execution_time: Duration,           // Total execution time
}

pub enum FinishReason {
    Completed,                               // Normal completion
    MaxTurnsReached,                         // Hit max turns limit
    TokenBudgetExceeded,                     // Token budget exceeded
    StopConditionMet(String),                // Custom stop condition triggered
    UserInterrupted,                         // Interrupted by user
    Error(AgentError),                       // Execution error
}

pub enum StreamChunk {
    Text { content: String, is_final: bool },
    ToolStart { id: String, name: String },
    ToolArguments { id: String, arguments: String },
    ToolComplete { id: String, result: ToolResult },
    ToolError { id: String, error: ToolError },
    UsageUpdate(TokenUsage),
    FinishReason(FinishReason),
    Error(AgentError),
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
    /// Compute embedding via claw-provider and persist to vector store
    async fn insert_semantic(
        &self,
        content: &str,
        metadata: serde_json::Value,
    ) -> Result<MemoryId, MemoryError>;

    /// Vector similarity search (top-k)
    async fn search_semantic(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError>;

    /// Append a timestamped episodic log entry
    async fn insert_episodic(
        &self,
        entry: EpisodicEntry,
    ) -> Result<EpisodeId, MemoryError>;

    /// Query episodic log by time range or tag filter
    async fn query_episodic(
        &self,
        filter: EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError>;
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

<a name="memory-system-architecture"></a>
## Memory System Architecture (The 3-Tier Model)

> **Mechanism vs. Policy**: Layer 2 (`claw-memory`) provides the storage *mechanism*. Layer 3 scripts define the *policy* — when to persist, how to summarize, which retrieval prompts to use. This separation enables Agent self-evolution without recompiling the kernel.

The memory system is organized into three tiers, each managed by a different component:

### Tier 1 — Working Memory (WM)

| Property | Value |
|----------|-------|
| **Location** | `claw-loop` — the `History` object |
| **Storage** | In-process heap (Vec\<Message\>) |
| **Capacity** | Token-budget bounded (FIFO eviction) |
| **Eviction** | Oldest messages dropped when token limit is exceeded |
| **Persistence** | None (volatile; survives only within the session) |

When Working Memory overflows, `claw-loop` emits a `MemoryPressure` event on the EventBus. A script listener (or the default kernel handler) can then call `claw.memory.logEpisode()` / `claw.memory.memorize()` to promote important context into the lower tiers.

```
Working Memory overflow  ──EventBus──►  claw-memory (EM / SM persistence)
         (claw-loop)                          (claw-memory)
```

### Tier 2 — Episodic Memory (EM)

| Property | Value |
|----------|-------|
| **Location** | `claw-memory` crate |
| **Storage** | SQLite (time-series log table) |
| **Scope** | Per-Agent, persists across sessions |
| **Purpose** | Records agent behavior trace: tool calls, LLM responses, errors |
| **Use Case** | Reflection, post-mortem analysis, self-evolution audit trail |

Episodic Memory is a **chronological journal** of what the Agent did. It answers: *"What did I try last time this happened?"* Scripts query it in `on_think` to avoid repeating past mistakes.

### Tier 3 — Semantic Memory (SM)

| Property | Value |
|----------|-------|
| **Location** | `claw-memory` crate |
| **Storage** | `sqlite-vec` (default) · `qdrant-client` (feature `qdrant`, ml-ready) |
| **Scope** | Configurable: per-Agent or shared knowledge space |
| **Purpose** | De-temporalized, factual knowledge for cross-session RAG |
| **Use Case** | Domain knowledge, learned procedures, distilled insights |

Semantic Memory is a **knowledge graph without timestamps**. It answers: *"What do I know about X?"* The Agent autonomously decides — via script policy — which observations are worth memorizing as reusable knowledge.

### 3-Tier Interaction Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                    Agent Execution Loop                       │
│                       (claw-loop)                            │
│                                                              │
│  on_think ──► search WM ──► search SM (claw.memory.search)   │
│                                   │                          │
│                            inject top-k results              │
│                            into LLM context                  │
│                                                              │
│  on_observe ──► evaluate value ──► if worthy:               │
│                                      claw.memory.memorize()  │
│                                      claw.memory.logEpisode()│
│                                                              │
│  WM overflow ──► EventBus ──► auto-persist to EM            │
└──────────────────────────────────────────────────────────────┘
         │                              │
         ▼                              ▼
  ┌─────────────┐               ┌─────────────────┐
  │  Episodic   │               │    Semantic      │
  │  Memory     │               │    Memory        │
  │  (SQLite)   │               │  (sqlite-vec /   │
  │  time-log   │               │   qdrant)        │
  └─────────────┘               └─────────────────┘
         both managed by claw-memory (Layer 2)
```

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

<a name="chinese"></a>
# claw-kernel 架构概述

> Claw 生态系统的基石 —— 一个用 Rust 构建的跨平台 Agent 内核

---

## 目录

- [设计哲学](#design-philosophy-cn)
- [五层架构](#the-5-layer-architecture-cn)
- [记忆系统架构（三级模型）](#memory-system-architecture-cn)
- [关键架构决策](#key-architectural-decisions-cn)
- [跨平台策略](#cross-platform-strategy-cn)
- [可扩展性架构](#extensibility-architecture-cn)
- [安全模型](#security-model-cn)

---

## 设计哲学

### 为什么选择 Rust 内核 + 脚本？

Claw 生态系统已有 8 个以上的独立实现，它们都在解决相同的问题：

| 项目 | 语言 | 代码行数 | 问题 |
|---------|----------|-------|---------|
| OpenClaw | TypeScript | 430K | 重复实现基础功能 |
| ZeroClaw | Rust | 150K | 缺少共享抽象 |
| PicoClaw | Go | 50K | 平台特定代码 |
| Nanobot | Python | 4K | 无法达到生产环境性能 |

**claw-kernel** 将这些通用原语提取到一个共享基础中：

```
Rust 内核 = Linux 内核（稳定核心、内存安全）
脚本层 = 用户空间程序（热插拔、可扩展）
```

### 核心原则

1. **不可变核心，可变脚本**
   - Rust 代码稳定、经过测试，从不热修补
   - 所有可扩展逻辑都存在于脚本中（Lua/Deno/Python）

2. **跨平台优先**
   - 核心代码中没有 Unix 特性，也没有 Windows 特性
   - 平台特定行为隔离在 `claw-pal` 中

3. **可扩展性设计**
   - 内核为运行时扩展提供基础能力
   - 无需重启即可热加载

4. **双模式安全**
   - 安全模式：默认沙箱化
   - 强力模式：完全访问，显式选择加入

---

## 五层架构

```
┌─────────────────────────────────────────────────────────┐
│                       内核层                             │
│  ┌─────────────────────────────────────────────────────┐│
│  │  第 3 层：扩展基础                                   ││
│  │  扩展基础 · 热加载 · 动态注册                         ││
│  │  Lua（默认）· Deno/V8（完整）· PyO3（ML）            ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  第 2 层：Agent 内核协议                             ││
│  │  提供者特性 · 工具注册表 · Agent 循环 · 历史记录      ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  第 1 层：系统运行时                                 ││
│  │  事件总线 · IPC 传输 · 进程守护进程 · Tokio          ││
│  └─────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────┐│
│  │  第 0.5 层：平台抽象层（PAL）                         ││
│  │  沙箱后端 · IPC 传输 · 配置目录                      ││
│  └─────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────┐│
│  │  第 0 层：Rust 硬核核心                              ││
│  │  内存安全 · 操作系统抽象 · 信任根                     ││
│  └─────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────┘
```

### 第 0 层：Rust 硬核核心

**信任根** —— 不可变、从不热修补、无平台假设。

**职责：**
- 进程生命周期管理
- 安全凭证存储（内存加密）
- 脚本引擎引导/运行时初始化
- 模式切换守卫（安全 ↔ 强力）

**关键约束：** 这一层不能被脚本修改。永远不能。

### 第 0.5 层：平台抽象层（PAL）

将平台特定代码与系统其余部分隔离。

| 组件 | Linux | macOS | Windows |
|-----------|-------|-------|---------|
| 沙箱 | seccomp-bpf + 命名空间 | sandbox(7) 配置文件 | AppContainer + 作业对象 |
| IPC | Unix 域套接字 | Unix 域套接字 | 命名管道 |
| 进程 | fork()/exec() | fork()/exec() | CreateProcess() |
| 配置 | XDG 目录 | ~/Library | %APPDATA% |

实现细节请参阅 [PAL 深度解析](pal.md)。

### 第 1 层：系统运行时

基于 Tokio 的异步基础。

**事件总线：**
```rust
// 所有组件的中央消息总线
pub enum Event {
    UserInput(Message),                    // 用户输入
    AgentOutput(Response),                 // Agent 输出
    ToolCall(ToolInvocation),              // 工具调用
    ToolResult(ToolOutput),                // 工具执行结果
    AgentLifecycle(AgentState),            // Agent 生命周期
    Extension(ExtensionEvent),             // 扩展事件
    A2A(A2AMessage),                       // Agent 间通信
}

/// Agent 间通信消息
pub struct A2AMessage {
    pub from: AgentId,                      // 发送者
    pub to: Option<AgentId>,                // 接收者，None = 广播
    pub message_type: A2AMessageType,       // 消息类型
    pub payload: Payload,                   // 序列化消息负载
    pub correlation_id: Option<Uuid>,       // 请求-响应关联ID
    pub timeout: Option<Duration>,          // 消息超时，默认30秒
    pub priority: MessagePriority,          // 消息优先级
    pub timestamp: SystemTime,              // 消息创建时间
}

pub enum Payload {
    Json(serde_json::Value),
    Cbor(Vec<u8>),
}

pub enum MessagePriority {
    Critical = 0,    // 关键
    High = 1,        // 高
    Normal = 2,      // 正常
    Low = 3,         // 低
    Background = 4,  // 后台
}

pub enum A2AMessageType {
    Request,     // 期望响应
    Response,    // 对请求的响应
    Event,       // 即发即弃
    Command,     // 指令（父到子）
}

/// 扩展相关事件
pub enum ExtensionEvent {
    ToolLoading { name: String },
    ToolLoaded { name: String, result: Result<(), LoadError> },
    ToolUnloaded { name: String },
    ScriptReloaded { path: PathBuf, result: Result<(), ReloadError> },
    ProviderRegistered { name: String },
}
```

**进程管理：**
- 子 Agent 生命周期（生成 / 终止 / 列表 / 控制）
- 健康检查和自动重启
- 资源配额（CPU/内存）

### 第 2 层：Agent 内核协议

系统的**核心** —— 所有 Claw 项目一直在重复造轮子的地方。

#### 提供者抽象

Provider 系统使用**三层架构**来在碎片化的 LLM API 生态系统中最大化代码复用：

```
第 3 层：LLMProvider trait      ← 面向用户的接口（complete、stream_complete）
第 2 层：HttpTransport trait    ← 可复用的 HTTP 逻辑（request、stream_request）
第 1 层：MessageFormat trait    ← 协议抽象（OpenAIFormat、AnthropicFormat）
```

**第 1 层：MessageFormat（协议抽象）**

市场已围绕两种主导格式整合：
- **OpenAI 格式** — OpenAI、DeepSeek、Moonshot、Qwen、Grok 及 50+ 服务商使用
- **Anthropic 格式** — Anthropic (Claude) 和 AWS Bedrock 使用

```rust
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

**第 2 层：HttpTransport（可复用逻辑）**

```rust
#[async_trait]
pub trait HttpTransport: Send + Sync {
    fn base_url(&self) -> &str;
    fn auth_headers(&self) -> HeaderMap;
    fn http_client(&self) -> &Client;
    
    async fn request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<CompletionResponse, ProviderError> {
        // 所有 providers 复用的通用 HTTP 逻辑
    }
    
    async fn stream_request<F: MessageFormat>(
        &self, messages: &[Message], opts: &Options
    ) -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
}
```

**第 3 层：LLMProvider（用户接口）**

```rust
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

// 嵌入接口（单独 trait，因为不是所有 provider 都支持）
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError>;
}
```

#### 基础类型定义

```rust
// 基础消息类型
pub struct Message {
    pub role: Role,  // system/user/assistant/tool
    pub content: String,
    pub name: Option<String>,        // 发送者标识（如函数名）
    pub metadata: Option<serde_json::Value>, // 扩展元数据
}

pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

// LLM 调用选项
pub struct Options {
    pub model: Option<String>,       // 模型标识，None = 使用 provider 默认值
    pub temperature: Option<f32>,    // 范围：0.0-2.0，默认：1.0
    pub max_tokens: Option<usize>,   // 最大生成 token 数，默认：4096
    pub stop_sequences: Vec<String>, // 停止序列，默认：空
    pub tools: Option<Vec<ToolDef>>, // 函数调用可用工具
    pub timeout: Duration,           // 请求超时，默认：60秒
    pub max_retries: u32,            // 最大重试次数，默认：3
}

// 流式响应增量
pub struct Delta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

// 工具调用
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}
```

**内置实现：**

| Provider | 使用格式 | 代码复杂度 |
|----------|----------|------------|
| `AnthropicProvider` | AnthropicFormat | ~20 行（仅配置） |
| `BedrockProvider` | AnthropicFormat + AWS 认证 | ~30 行（仅配置） |
| `OpenAIProvider` | OpenAIFormat | ~20 行（仅配置） |
| `DeepSeekProvider` | OpenAIFormat | ~20 行（仅配置） |
| `MoonshotProvider` | OpenAIFormat | ~20 行（仅配置） |
| `QwenProvider` | OpenAIFormat | ~20 行（仅配置） |
| `GrokProvider` | OpenAIFormat | ~20 行（仅配置） |
| `OllamaProvider` | OllamaFormat（OpenAI 变体） | ~25 行（仅配置） |
| `ScriptableProvider` | 脚本自定义格式 | 运行时定义 |

> 代码量减少 90%：添加新的 OpenAI 兼容 provider 只需要配置（base URL + 认证），无需 HTTP 实现。

完整架构决策请参阅 [ADR-006](../adr/006-message-format-abstraction.md)。

#### 工具注册表
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;           // 工具描述，供 LLM 使用
    fn version(&self) -> &str;               // 语义化版本，如 "1.0.0"
    fn schema(&self) -> ToolSchema;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    fn permissions(&self) -> PermissionSet;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }  // 默认超时
}

pub struct ToolRegistry {
    // 热加载是可扩展性的关键
    pub fn load_from_script(&mut self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub fn unload(&mut self, name: &str);
    pub fn list(&self) -> Vec<ToolMeta>;
}
```

#### Agent 循环
```rust
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,           // LLM 接口
    tools: Arc<ToolRegistry>,                 // 可用工具
    history: Box<dyn HistoryManager>,         // 可插拔组件
    stop_conditions: Vec<Box<dyn StopCondition>>,
    summarizer: Option<Box<dyn Summarizer>>,
    config: AgentLoopConfig,                  // 循环配置
}

pub struct AgentLoopConfig {
    pub max_turns: Option<usize>,             // 默认 50
    pub token_budget: Option<usize>,          // 默认 8000
    pub system_prompt: Option<String>,        // 系统提示词
    pub enable_streaming: bool,               // 默认 true
    pub tool_timeout: Duration,               // 默认 30s
    pub max_tool_calls_per_turn: usize,       // 默认 10
}
```

### 第 3 层：扩展基础

**扩展性边界说明：**
- **不可修改**：第 3 层的脚本引擎运行时本身不可热修改
- **可热加载**：运行在引擎之上的脚本/工具代码可以动态加载、更新和卸载
- 这种设计确保了内核稳定性，同时允许应用层通过脚本实现自进化

多引擎支持，统一接口。这一层提供**可扩展性的基础能力**——热加载、脚本执行、动态注册等能力，为上层进化提供支持。

| 引擎 | 二进制大小 | 优势 | 使用场景 |
|--------|-------------|----------|----------|
| **Lua (mlua)** | ~500KB | 零依赖，快速 | 默认，简单工具 |
| **Deno/V8** | ~100MB | 完整 TS/JS，强沙箱 | 复杂 Agent |
| **PyO3** | 可变 | ML 生态系统 | 数据/ML 工具 |

**核心能力：**
- **热加载**：无需重启即可加载/卸载脚本
- **权限桥接**：为脚本强制执行安全边界
- **类型安全层**：Rust 之上的 TypeScript 风格接口

**RustBridge：**
```typescript
// 暴露给脚本
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
  memory: { 
    get(key: string): Promise<any>;
    set(key: string, value: any): Promise<void>;
    search(query: string, topK: number): Promise<MemoryItem[]>;
  };
  events: { emit(event: string, data: any): void; on(event: string, handler: Function): void };
  fs: { read(path: string): Promise<Buffer>; write(path: string, data: Buffer): Promise<void> };
  agent: { spawn(config: AgentConfig): Promise<AgentHandle>; kill(handle: AgentHandle): Promise<void> };
}
```

## 关键架构决策

### 1. 为什么选择 Lua 作为默认引擎？

**决策：** Lua（通过 mlua）是默认脚本引擎。

**理由：**
- 零依赖（纯 Rust 绑定）
- 编译快速（<1 分钟）
- 运行时小巧（<500KB）
- 开箱即用的跨平台支持
- 足以应对大多数工具逻辑

**考虑的替代方案：**
- Deno/V8：太大，构建复杂
- Wasmer/WASM：沙箱好但工具不成熟

### 2. 为什么分离 PAL 层？

**决策：** 平台特定代码隔离在 `claw-pal` crate 中。

**理由：**
- 强制跨平台思维
- 使平台差异可见
- 支持平台特定优化而不泄漏

### 3. 为什么有两种安全模式？

**决策：** 显式安全/强力模式区分。

**理由：**
- 大多数 Agent 任务不需要完全系统访问
- 安全模式支持"信任但验证"部署
- 强力模式在需要时启用完全自动化
- 为用户提供清晰的心理模型

完整分析请参阅 [ADR-003：安全模型](../adr/003-security-model.md)。

---

## 跨平台策略

### 理念：零平台假设

第 0-3 层的所有代码都是**平台无关的**。平台特定性仅在以下位置：
- `claw-pal` crate
- PAL 特性实现
- 平台特定测试

### 平台能力矩阵

| 特性 | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| 安全模式 | Yes 强 (Strong) | Yes 中等 (Medium) | Yes 中等 (Medium) |
| 强力模式 | Yes 完全 | Yes 完全 | Yes 完全 |
| IPC 性能 | 100% | 95% | 90% |
| 进程隔离 | 最强 (Strongest) | 中等 (Medium) | 中等 (Medium) |
| 构建复杂度 | 低 | 低 | 中 |

### 处理平台差异

```rust
// claw-pal/src/sandbox/mod.rs
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub trait SandboxBackend {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, allowlist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}
```

操作系统特定细节请参阅 [平台指南](../platform/)。

---

## 可扩展性架构

### 内核职责

内核提供可扩展性的基础能力：

```
┌─────────────────────────────────────────────────────────┐
│  内核层（提供基础能力）                                   │
├─────────────────────────────────────────────────────────┤
│  扩展基础                   热加载 API                    │
│  （第 3 层）                 脚本运行时                   │
│                             权限桥接                      │
├─────────────────────────────────────────────────────────┤
│  Agent 内核协议             工具注册表                    │
│  （第 2 层）                 动态注册                     │
│                             LLM 提供者抽象                │
└─────────────────────────────────────────────────────────┘
```

### 运行时哪些可以扩展？

```
┌────────────────────────────────────────┐
│          运行时支持扩展                 │
├────────────────────────────────────────┤
│ 工具脚本（第 3 层脚本运行时）            │
│ 自定义 Provider                        │
│ 内存策略                               │
│ 停止条件                               │
│ 通道适配器                             │
└────────────────────────────────────────┘

┌────────────────────────────────────────┐
│       不能被修改                        │
├────────────────────────────────────────┤
│ Rust 内核代码                          │
│ 沙箱强制执行                           │
│ 模式切换守卫                           │
│ 凭证存储                               │
│ 脚本引擎运行时（第 3 层）[^1]            │
└────────────────────────────────────────┘

[^1]: 脚本引擎运行时不可修改，但脚本内容可热加载
```

### 热加载机制

内核提供热加载作为**基础能力**：

```rust
// 简化的热加载流程（内核第 3 层）
pub async fn hot_load_tool(&mut self, path: &Path) -> Result<()> {
    // 1. 读取并验证脚本
    let script = fs::read_to_string(path).await?;
    let validated = self.validate(&script)?;
    
    // 2. 检查权限（仅安全模式）
    if self.mode == ExecutionMode::Safe {
        self.audit_permissions(&validated)?;
    }
    
    // 3. 在隔离上下文中编译
    let compiled = self.script_engine.compile(&script)?;
    
    // 4. 注册到 ToolRegistry
    let tool = ScriptTool::new(compiled, self.bridge.clone());
    self.registry.register(tool)?;
    
    // 5. 发送事件
    self.events.emit(Event::ToolLoaded { name: tool.name() });
    
    Ok(())
}
```

基于内核的应用程序可以使用这些能力来实现自定义的可扩展性模式。

使用方法请参阅 [扩展能力指南](../guides/extension-capabilities.md)。

---

## 安全模型

### 双模式安全

| 维度 | 安全模式 | 强力模式 |
|-----------|-----------|------------|
| 文件系统 | 允许列表只读 | 完全访问 |
| 网络 | 域名/端口规则 | 无限制 |
| 子进程 | 阻止 | 允许 |
| 脚本扩展 | 允许（沙箱化） | 允许（全局） |
| 内核访问 | 阻止 | 阻止（硬约束） |

### 模式切换

```
┌─────────────┐      power-key + 显式标志       ┌─────────────┐
│  安全模式   │  ─────────────────────────────► │  强力模式   │
│  （默认）   │                                 │  （选择加入）│
└─────────────┘                                 └─────────────┘
       ▲                                                │
       │              重启或新进程                      │
       └────────────────────────────────────────────────┘
```

**重要：** 强力模式 → 安全模式需要重启。这是有意为之 —— 被攻破的强力模式 Agent 不能"降级"来隐藏证据。

### 审计追踪

所有扩展操作都会被记录：

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

## 下一步

- **对于用户：** [入门指南](../guides/getting-started.md)
- **对于贡献者：** [贡献指南](../../CONTRIBUTING.md)
- **对于架构细节：** [Crate 地图](crate-map.md), [ADR 索引](../adr/)
- **对于平台特定信息：** [Linux](../platform/linux.md), [macOS](../platform/macos.md), [Windows](../platform/windows.md)
