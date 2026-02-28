# claw-kernel 构建计划
# Build Plan

> 详细的分阶段构建路线图，指导 claw-kernel 的实现
> 版本：v1.0 | 日期：2026-02-28

---

## 概述 / Overview

本文档描述 claw-kernel 从设计到实现的详细构建计划。按照 7 个阶段逐步实现，确保架构的稳定性和可测试性。

---

## 构建阶段 / Build Phases

### Phase 1: Platform Abstraction Layer (claw-pal)

**目标**：实现跨平台沙箱、IPC 和进程管理

**核心 Trait 定义**：

```rust
// 沙箱后端
pub trait SandboxBackend: Send + Sync {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

// IPC 传输
pub trait IpcTransport: Send + Sync {
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}

// 进程管理
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError>;
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError>;
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError>;
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError>;
}
```

**平台实现**：

| 平台 | 沙箱技术 | IPC | 进程管理 |
|------|---------|-----|---------|
| Linux | seccomp-bpf + Namespaces | Unix Domain Socket | fork/exec |
| macOS | sandbox(7) profile | Unix Domain Socket | fork/exec |
| Windows | AppContainer + Job Objects | Named Pipe | CreateProcess |

**里程碑**：
- [ ] SandboxBackend trait 定义
- [ ] Linux 沙箱实现
- [ ] macOS 沙箱实现
- [ ] Windows 沙箱实现
- [ ] IPC 跨平台抽象
- [ ] 进程生命周期管理

---

### Phase 2: System Runtime (claw-runtime)

**目标**：事件总线、进程管理和多智能体编排

**核心组件**：

```rust
// 事件总线
pub struct EventBus {
    // 内部实现
}

impl EventBus {
    pub fn emit(&self, event: Event);
    pub fn subscribe(&self, filter: EventFilter) -> Receiver<Event>;
}

// 运行时
pub struct Runtime {
    pub event_bus: EventBus,
    pub process_manager: Arc<dyn ProcessManager>,
    pub ipc_router: IpcRouter,
}

// 多智能体编排
pub struct AgentOrchestrator {
    // 管理多个 Agent 实例
}

impl AgentOrchestrator {
    pub async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle, OrchestratorError>;
    pub async fn kill(&self, handle: AgentHandle) -> Result<(), OrchestratorError>;
    pub fn list(&self) -> Vec<AgentInfo>;
    pub async fn send_message(&self, from: AgentId, to: AgentId, msg: A2AMessage);
}
```

**事件类型**：

```rust
pub enum Event {
    UserInput(Message),
    AgentOutput(Response),
    ToolCall(ToolInvocation),
    ToolResult(ToolOutput),
    AgentLifecycle(AgentState),
    Extension(ExtensionEvent),
    A2A(A2AMessage),
}
```

**里程碑**：
- [ ] EventBus 实现
- [ ] Runtime 结构
- [ ] AgentOrchestrator
- [ ] A2A 消息协议

---

### Phase 3: Core Protocols - Part 1 (claw-provider + claw-tools)

#### 3a. claw-provider: LLM Provider 抽象

**三层架构**：

```rust
// Level 1: MessageFormat (协议抽象)
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

// Level 2: HttpTransport (可复用 HTTP 逻辑)
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

// Level 3: LLMProvider (用户接口)
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

**内置 Provider**：
- Anthropic (Claude)
- OpenAI
- DeepSeek
- Moonshot
- Qwen
- Grok
- Azure OpenAI
- AWS Bedrock
- Ollama (本地)

#### 3b. claw-tools: Tool Registry 和协议

**核心 Trait**：

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn version(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    fn permissions(&self) -> PermissionSet;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }
}

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

pub struct PermissionSet {
    pub filesystem: FsPermissions,
    pub network: NetworkPermissions,
    pub subprocess: SubprocessPolicy,
}

pub enum SubprocessPolicy {
    Allow { allowed_commands: Vec<String>, max_concurrent: usize },
    Deny,
}
```

**ToolRegistry**：

```rust
pub struct ToolRegistry {
    // 内部实现
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;
    pub fn unregister(&mut self, name: &str) -> Result<(), RegistryError>;
    pub fn get(&self, name: &str) -> Option<&dyn Tool>;
    pub fn list(&self) -> Vec<&ToolMeta>;
    pub async fn execute(&self, name: &str, params: serde_json::Value) -> Result<ToolResult, ToolError>;
    pub async fn load_from_script(&mut self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub async fn load_from_directory(&mut self, path: &Path) -> Result<Vec<ToolMeta>, LoadError>;
    pub async fn enable_hot_loading(&mut self, config: HotLoadingConfig) -> Result<(), WatchError>;
}
```

**里程碑**：
- [ ] MessageFormat trait
- [ ] HttpTransport trait
- [ ] LLMProvider trait
- [ ] EmbeddingProvider trait
- [ ] OpenAIFormat 实现
- [ ] AnthropicFormat 实现
- [ ] Tool trait
- [ ] ToolRegistry
- [ ] 权限系统

---

### Phase 4: Agent Loop & Memory (claw-loop)

**目标**：多轮对话管理、停止条件、历史管理

**核心类型**：

```rust
pub struct AgentLoop {
    provider: Arc<dyn LLMProvider>,
    tools: Arc<ToolRegistry>,
    history: Box<dyn HistoryManager>,
    stop_conditions: Vec<Box<dyn StopCondition>>,
    summarizer: Option<Box<dyn Summarizer>>,
    config: AgentLoopConfig,
}

pub struct AgentLoopConfig {
    pub max_turns: Option<usize>,      // Default: 50
    pub token_budget: Option<usize>,   // Default: 8000
    pub system_prompt: Option<String>,
    pub enable_streaming: bool,        // Default: true
    pub tool_timeout: Duration,        // Default: 30s
    pub max_tool_calls_per_turn: usize, // Default: 10
}

pub struct AgentResult {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub turns: usize,
    pub token_usage: TokenUsage,
    pub finish_reason: FinishReason,
    pub execution_time: Duration,
}

pub enum FinishReason {
    Completed,
    MaxTurnsReached,
    TokenBudgetExceeded,
    StopConditionMet(String),
    UserInterrupted,
    Error(AgentError),
}
```

**StopCondition**：

```rust
pub trait StopCondition: Send + Sync {
    fn should_stop(&self, state: &LoopState) -> bool;
}

pub struct LoopState {
    pub turn_count: usize,
    pub token_usage: TokenUsage,
    pub last_message: Option<Message>,
    pub tool_calls_made: usize,
}
```

**HistoryManager**：

```rust
pub trait HistoryManager: Send + Sync {
    fn append(&self, message: Message);
    fn get_context(&self, max_tokens: usize) -> Vec<Message>;
    fn truncate_to_fit(&self, max_tokens: usize);
    fn summarize(&self, strategy: &dyn Summarizer);
}
```

**里程碑**：
- [ ] AgentLoop 结构
- [ ] AgentLoopBuilder
- [ ] StopCondition trait
- [ ] HistoryManager trait
- [ ] 内置停止条件 (MaxTurns, TokenBudget, NoToolCall)
- [ ] 内存历史实现
- [ ] SQLite 历史实现 (可选)

---

### Phase 5: Script Runtime (claw-script)

**目标**：多引擎脚本执行和热加载基础

**核心 Trait**：

```rust
pub trait ScriptEngine: Send + Sync {
    fn compile(&self, source: &str, source_name: &str) -> Result<Script, CompileError>;
    fn execute(&self, script: &Script, context: &Context, timeout: Duration) -> Result<Value, ScriptError>;
    fn register_native(&self, name: &str, func: NativeFunction);
}

pub enum EngineType {
    Lua,
    #[cfg(feature = "engine-v8")]
    V8,
    #[cfg(feature = "engine-py")]
    Python,
}

pub struct Context {
    pub agent_id: AgentId,
    pub permissions: PermissionSet,
    pub runtime_data: HashMap<String, Value>,
}

pub type NativeFunction = Arc<dyn Fn(&Context, Vec<Value>) -> Result<Value, ScriptError> + Send + Sync>;
```

**RustBridge API**：

```typescript
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
  events: {
    emit(event: string, data: any): void;
    on(event: string, handler: (data: any) => void): void;
  };
  fs: {
    read(path: string): Promise<Uint8Array>;
    write(path: string, data: Uint8Array): Promise<void>;
    exists(path: string): boolean;
    listDir(path: string): DirEntry[];
  };
  net: {
    get(url: string, headers?: Headers): Promise<Response>;
    post(url: string, headers: Headers, body: string): Promise<Response>;
  };
}
```

**里程碑**：
- [ ] ScriptEngine trait
- [ ] Lua 引擎 (mlua)
- [ ] RustBridge API
- [ ] 热加载机制
- [ ] V8 引擎 (可选)
- [ ] Python 引擎 (可选)

---

### Phase 6: Channel Integrations (claw-channel)

**目标**：外部通信接口

**Channel Trait**：

```rust
#[async_trait]
pub trait Channel: Send + Sync {
    async fn start(&self) -> Result<(), ChannelError>;
    async fn stop(&self) -> Result<(), ChannelError>;
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError>;
    fn on_message(&self, handler: Box<dyn Fn(ChannelMessage) + Send + Sync>);
}
```

**实现**：
- [ ] Telegram 集成
- [ ] Discord 集成
- [ ] HTTP Webhook

---

### Phase 7: Meta Crate & Integration

**目标**：统一的 claw-kernel crate

```rust
// lib.rs
pub use claw_pal as pal;
pub use claw_provider as provider;
pub use claw_tools as tools;
pub use claw_loop as loop_;
pub use claw_runtime as runtime;
pub use claw_script as script;

// 重导出常用类型
pub use claw_provider::{LLMProvider, EmbeddingProvider, Message, Role};
pub use claw_tools::{Tool, ToolRegistry, ToolResult};
pub use claw_loop::{AgentLoop, AgentLoopConfig};
```

**里程碑**：
- [ ] claw-kernel meta-crate
- [ ] 集成测试
- [ ] 示例程序
- [ ] API 文档

---

## 依赖版本 / Dependency Versions

```toml
[dependencies]
# 核心运行时
tokio = { version = "1.35.0", features = ["rt-multi-thread", "macros", "sync", "time", "fs"] }
async-trait = "0.1.77"
reqwest = { version = "0.11.23", features = ["json", "stream"] }
serde = { version = "1.0.195", features = ["derive"] }
serde_json = "1.0.111"
thiserror = "1.0.56"
anyhow = "1.0.79"
tracing = "0.1.40"

# PAL 层
interprocess = { version = "1.2.1", features = ["tokio"] }
dirs = "5.0.1"

# Linux 沙箱
[target.'cfg(target_os = "linux")'.dependencies]
libseccomp = "0.3.0"
nix = { version = "0.27.1", features = ["process", "sched"] }

# 脚本引擎
mlua = { version = "0.9.4", features = ["lua54", "async", "send", "serde"], optional = true }
deno_core = { version = "0.245.0", optional = true }
pyo3 = { version = "0.28.0", features = ["auto-initialize"], optional = true }

# 工具与扩展
schemars = "0.8.16"
notify = { version = "6.1.1", features = ["tokio"] }
rusqlite = { version = "0.30.0", features = ["bundled", "chrono"], optional = true }
```

---

## 特性标志 / Feature Flags

```toml
[features]
default = ["engine-lua"]

# 脚本引擎
engine-lua = ["mlua"]
engine-v8 = ["deno_core"]
engine-py = ["pyo3"]

# 存储后端
sqlite = ["rusqlite"]

# Provider
openai = []
anthropic = []
ollama = []

# 测试
sandbox-tests = []
```

---

## 参考文档 / References

- [TECHNICAL_SPECIFICATION.md](./TECHNICAL_SPECIFICATION.md) - 技术规格
- [docs/architecture/overview.md](./docs/architecture/overview.md) - 架构概述
- [docs/architecture/crate-map.md](./docs/architecture/crate-map.md) - Crate 依赖图
- [AGENTS.md](./AGENTS.md) - AI 代理开发指南

---

*版本: v1.0*
*最后更新: 2026-02-28*
*维护者: claw-project team*
