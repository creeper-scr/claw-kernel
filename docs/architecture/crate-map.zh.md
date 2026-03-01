---
title: Crate 地图与依赖图
description: Crate structure, dependencies, and relationships
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](crate-map.md)

# Crate 地图与依赖图

本文档描述 claw-kernel 的 crate 结构，包括依赖关系和关联。

---

## 架构概览

```
╔══════════════════════════════════════════════════════════════════════╗
║                     内核核心（最小化且稳定）                            ║
╠══════════════════════════════════════════════════════════════════════╣
║  第 0 层        第 0.5 层       第 1 层         第 2 层            第 3 层           ║
║                                                                      ║
║  ┌────┐  ┌─────────┐  ┌──────────┐   ┌───────────┐  ┌───────────┐  ║
║  │Rust│  │claw-pal │  │claw-     │   │claw-loop  │  │claw-script│  ║
║  │硬核│  │(平台    │  │runtime  │   │(Agent     │  │(脚本      │  ║
║  │核心│  │ 抽象层) │  │ 运行时)  │   │ 循环)     │  │ 运行时)   │  ║
║  └──┬─┘  └────┬────┘  │         │   └─────┬─────┘  └───────────┘  ║
║       │       └────┬─────┘         │                                ║
║       │            │          ┌────┴────┐                           ║
║       │            │          │         │                           ║
║       │            │       ┌──▼──┐  ┌───▼────┐                      ║
║       │            │       │claw-│  │claw-   │                      ║
║       │            │       │tools│  │provider│                      ║
║       │            │       │(工具│  │(LLM    │                      ║
║       │            │       │协议)│  │ 抽象层)│                      ║
║       │            │       └─────┘  └────────┘                      ║
║       │            │                                                ║
║       └────────────┴───────────────────────────────────┐            ║
║                                                        ▼            ║
╠══════════════════════════════════════════════════════════════════════╣
║                    元 crate: claw-kernel                              ║
╚══════════════════════════════════════════════════════════════════════╝
```

---

## 内核核心 Crate（第 0.5 - 3 层）

> **设计原则**: 最小化、稳定、对内核功能至关重要。这些 crate 定义了基础抽象和协议。

### `claw-pal` —— 平台抽象层

**层级**: 0.5  
**用途**: 隔离所有平台特定代码。

**关键特性：**
- `SandboxBackend` —— 平台沙箱实现
- `IpcTransport` —— 跨平台 IPC
- `ProcessManager` —— 进程生命周期

**平台模块：**
```
claw-pal/src/
├── lib.rs
├── traits/
│   ├── sandbox.rs
│   ├── ipc.rs
│   └── process.rs
├── linux/
│   ├── sandbox.rs    # seccomp-bpf + 命名空间
│   ├── ipc.rs        # Unix 域套接字
│   └── process.rs    # fork/exec
├── macos/
│   ├── sandbox.rs    # sandbox(7) 配置文件
│   ├── ipc.rs        # Unix 域套接字
│   └── process.rs    # fork/exec
└── windows/
    ├── sandbox.rs    # AppContainer + 作业对象
    ├── ipc.rs        # 命名管道
    └── process.rs    # CreateProcess
```

**依赖：** 无（仅核心）

---

### `claw-runtime` —— 系统运行时

**层级**: 1  
**用途**: 事件总线、进程管理、多 Agent 编排。

**关键组件：**
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

**多 Agent 支持：**
```rust
pub struct AgentOrchestrator {
    pub fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
    pub fn kill(&self, handle: AgentHandle) -> Result<()>;
    pub fn list(&self) -> Vec<AgentInfo>;
    pub fn send_message(&self, from: AgentId, to: AgentId, msg: A2AMessage);
}

pub struct A2AMessage {
    pub from: AgentId,
    pub to: Option<AgentId>,         // None = 广播
    pub message_type: A2AMessageType,
    pub payload: Payload,            // 序列化消息负载
    pub correlation_id: Option<Uuid>,// 请求-响应关联
    pub timeout: Option<Duration>,   // 消息超时
    pub priority: MessagePriority,   // 消息优先级
    pub timestamp: SystemTime,       // 消息创建时间
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

pub enum A2AMessageType {
    Request,      // 期望响应
    Response,     // 对请求的响应
    Event,        // 即发即弃
    Command,      // 指令（父到子）
}

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

**依赖：**
- `claw-pal`
- `tokio` —— 异步运行时

---

### `claw-provider` —— LLM 提供者抽象

**层级**: 2  
**用途**: LLM API 的统一接口，采用三层架构最大化代码复用。

**架构（三层设计）：**

```
第 3 层：Provider 配置（面向用户）
    LLMProvider trait - complete(), stream_complete()
    
第 2 层：HttpTransport（可复用）
    通用请求/流式逻辑
    限流、重试、连接池
    
第 1 层：MessageFormat（协议抽象）
    OpenAIFormat - 50+ 服务商使用
    AnthropicFormat - Claude 和 Bedrock 使用
    OllamaFormat - 本地模型变体
```

**关键类型：**
```rust
// 第 1 层：协议格式抽象
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

// 第 2 层：可复用 HTTP 传输
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

// 第 3 层：面向用户的 provider trait
#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError>;
    async fn stream_complete(&self, messages: &[Message], opts: &Options) 
        -> Result<BoxStream<'static, Result<Delta, ProviderError>>, ProviderError>;
    fn token_count(&self, messages: &[Message]) -> usize;
}

// 单独的嵌入接口（不是所有 provider 都支持）
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, ProviderError>;
}
```

**模块结构：**
```
claw-provider/src/
├── lib.rs              # 公开导出
├── traits.rs           # LLMProvider、HttpTransport、MessageFormat trait
├── types.rs            # Message、Response、Options 等
├── format/             # 第 1 层：协议实现
│   ├── mod.rs
│   ├── openai.rs       # OpenAIFormat（50+ 服务商共享）
│   ├── anthropic.rs    # AnthropicFormat
│   └── ollama.rs       # OllamaFormat
├── transport.rs        # 第 2 层：HttpTransport 实现
└── providers/          # 第 3 层：Provider 配置
    ├── mod.rs
    ├── anthropic.rs    # AnthropicProvider（使用 AnthropicFormat）
    ├── bedrock.rs      # BedrockProvider（使用 AnthropicFormat + AWS 认证）
    ├── openai.rs       # OpenAIProvider（使用 OpenAIFormat）
    ├── deepseek.rs     # DeepSeekProvider（使用 OpenAIFormat）
    ├── moonshot.rs     # MoonshotProvider（使用 OpenAIFormat）
    ├── qwen.rs         # QwenProvider（使用 OpenAIFormat）
    ├── grok.rs         # GrokProvider（使用 OpenAIFormat）
    └── ollama.rs       # OllamaProvider（使用 OllamaFormat）
```

**内置 Providers（按格式分类）：**

| 格式 | Providers |
|------|-----------|
| `AnthropicFormat` | `AnthropicProvider` (Claude), `BedrockProvider` (AWS) |
| `OpenAIFormat` | `OpenAIProvider`, `DeepSeekProvider`, `MoonshotProvider`, `QwenProvider`, `GrokProvider`, `AzureOpenAIProvider` |
| `OllamaFormat` | `OllamaProvider` (本地模型) |

**添加新的 OpenAI 兼容 provider（约 20 行）：**
```rust
pub struct NewProvider {
    api_key: String,
    model: String,
    client: Client,
}

impl HttpTransport for NewProvider {
    fn base_url(&self) -> &str { "https://api.newprovider.com/v1" }
    fn auth_headers(&self) -> HeaderMap { /* Bearer token */ }
}

#[async_trait]
impl LLMProvider for NewProvider {
    async fn complete(&self, messages: &[Message], opts: &Options) -> Result<CompletionResponse, ProviderError> {
        self.request::<OpenAIFormat>(messages, opts).await
    }
    // stream_complete、token_count 同样委托
}
```

**特性开关：**
```toml
[features]
default = ["openai", "anthropic"]

# OpenAI 兼容 providers（都复用 OpenAIFormat）
openai = []
deepseek = ["openai"]   # 复用 OpenAIFormat
moonshot = ["openai"]   # 复用 OpenAIFormat
qwen = ["openai"]       # 复用 OpenAIFormat
grok = ["openai"]       # 复用 OpenAIFormat
azure = ["openai"]      # 复用 OpenAIFormat，特殊认证

# Anthropic 兼容 providers
anthropic = []
bedrock = ["anthropic"] # 复用 AnthropicFormat，AWS 认证

# 本地模型
ollama = []

# 所有 providers
full = ["openai", "deepseek", "moonshot", "qwen", "grok", "azure",
        "anthropic", "bedrock", "ollama"]
```

**依赖：**
- `reqwest` —— HTTP 客户端
- `serde` —— 序列化
- `async-trait` —— 异步 trait

**另请参阅：**
- [ADR-006: 消息格式抽象](../adr/006-message-format-abstraction.md)

---

### `claw-tools` —— 工具注册表与协议

**层级**: 2  
**用途**: 定义工具使用协议和热加载接口。  
**[Warning]  注意**: 仅提供注册协议和热加载机制。具体的工具生成逻辑在应用层实现。

**关键类型：**
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;           // 工具描述，供 LLM 使用
    fn version(&self) -> &str;               // 语义化版本，如 "1.0.0"
    fn schema(&self) -> ToolSchema;
    async fn execute(&self, params: Value) -> Result<ToolResult, ToolError>;
    fn permissions(&self) -> PermissionSet;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }  // 默认超时
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

**特性：**
```toml
[features]
default = ["hot-loading"]
hot-loading = ["notify"]  # 文件系统监控
schema-gen = ["schemars"]
```

**依赖：**
- `serde_json` —— JSON 处理
- `schemars` —— 模式生成
- `notify` —— 文件监控（可选）

---

### `claw-loop` —— Agent 循环引擎

**层级**: 2  
**用途**: 多轮对话管理和控制流。

**关键类型：**
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
    fn append(&self, message: Message);  // Uses interior mutability
    fn get_context(&self, max_tokens: usize) -> Vec<Message>;
    fn truncate_to_fit(&self, max_tokens: usize);
    fn summarize(&self, strategy: &dyn Summarizer);
}
```

**内置停止条件：**
- `MaxTurnsCondition`
- `TokenBudgetCondition`
- `NoToolCallCondition`
- `UserInterruptCondition`

**依赖：**
- `claw-provider`
- `claw-tools`

---

### `claw-script` —— 脚本运行时

**层级**: 3  
**用途**: 脚本执行和热加载基础。  
**[Warning]  注意**: 这是**脚本运行时（第 3 层）**，不是自进化系统本身。它提供脚本执行、热加载和运行时扩展能力，应用可以在此基础上构建。

**引擎支持：**
```toml
[features]
default = ["engine-lua"]
engine-lua = ["mlua"]        # Lua（默认，零依赖）
engine-v8 = ["deno_core"]    # Deno/V8（完整 TS/JS）
engine-py = ["pyo3"]         # Python（ML 生态系统）
```

**关键类型：**
```rust
pub trait ScriptEngine: Send + Sync {
    fn compile(&self, source: &str, source_name: &str) -> Result<Script>;
    fn execute(&self, script: &Script, context: &Context, timeout: Duration) -> Result<Value>;
    fn register_native(&self, name: &str, func: NativeFunction);
}

pub struct RustBridge {
    // 暴露给脚本
    pub llm: LlmBridge,
    pub tools: ToolsBridge,
    pub memory: MemoryBridge,
    pub events: EventsBridge,
    pub fs: FsBridge,
    pub agent: AgentBridge,
}
```

**依赖：**
- `mlua`（可选）—— Lua 绑定
- `deno_core`（可选）—— V8 嵌入
- `pyo3`（可选）—— Python 嵌入

---

## 内核边界

内核核心包含以下 crate，全部稳定、最小化且必不可少：

| Crate | 层级 | 用途 |
|-------|------|------|
| `claw-pal` | 0 / 0.5 | Rust 硬核核心 + 平台抽象层（必须支持所有平台） |
| `claw-runtime` | 1 | 系统运行时（核心） |
| `claw-provider` | 2 | LLM 抽象层（核心） |
| `claw-tools` | 2 | 工具协议（核心） |
| `claw-loop` | 2 | Agent 循环（核心） |
| `claw-script` | 3 | 脚本运行时（可选但稳定） |

基于 claw-kernel 的应用可以实现：
- 自定义 `MemoryBackend` trait 用于内存存储
- 自定义 `Channel` trait 用于平台集成
- 自定义工具生成逻辑
- 基于 claw-script 的自进化系统

---

## 元 Crate

### `claw-kernel` —— 重新导出 Crate

**用途**: 为需要完整内核的用户提供便捷的单依赖。

**结构：**
```rust
// lib.rs
pub use claw_pal as pal;
pub use claw_provider as provider;
pub use claw_tools as tools;
pub use claw_loop as loop_;
pub use claw_runtime as runtime;
pub use claw_script as script;

// 重新导出通用类型
pub use claw_provider::LLMProvider;
pub use claw_tools::{Tool, ToolRegistry};
pub use claw_loop::AgentLoop;
```

**特性：**
```toml
[features]
default = ["engine-lua"]

# 从 claw-script 重新导出
engine-lua = ["claw-script/engine-lua"]
engine-v8 = ["claw-script/engine-v8"]
engine-py = ["claw-script/engine-py"]
```

---

## 依赖图（可视化）

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
│(核心)   │ │(核心)    │ │(核心)   │ │(核心)    │
└────┬────┘ └────┬─────┘ └────┬────┘ └──────────┘
     │           │            │
     │      ┌────┴────┐       │
     │      │         │       │
     │      ▼         ▼       │
     │  ┌────────┐ ┌────────┐ │
     │  │claw-   │ │claw-   │ │
     │  │provider│ │runtime │ │
     │  │(核心)  │ │(核心)  │ │
     │  └────────┘ └────┬───┘ │
     │                  │     │
     └──────────────────┘     │
                              │
                              ▼
                        ┌──────────┐
                        │  tokio   │
                        └──────────┘
```

**图例：**
- 所有显示的 crate 都是内核核心的一部分（稳定、最小化）

---

## 版本策略

工作空间中的所有 crate 一起版本化（"统一版本"）：

- 当前版本：`0.1.0`
- 所有 crate 共享相同的版本号
- 破坏性更改会提升所有 crate 版本

这简化了用户的依赖管理。

---

## 特性标志指南

### 何时使用特性

1. **引入重型依赖的平台特定功能**
   - 示例：`engine-v8`（100MB+ 二进制影响）

2. **可选脚本引擎**
   - 示例：`engine-py`（需要 Python 3.10+）

### 何时不使用特性

1. **核心功能** —— 始终包含
2. **小型纯 Rust 依赖** —— 直接依赖

---

## 各 Crate 测试策略

| Crate | 类型 | 单元测试 | 集成测试 | 平台测试 |
|-------|------|:----------:|:-----------:|:--------------:|
| claw-pal | 核心 | Yes | Yes | 每平台必需 |
| claw-provider | 核心 | Yes | Yes（模拟 HTTP）| N/A |
| claw-tools | 核心 | Yes | Yes | N/A |
| claw-loop | 核心 | Yes | Yes | N/A |
| claw-runtime | 核心 | Yes | Yes | 必需 |
| claw-script | 核心 | Yes | Yes | 每引擎必需 |
| claw-kernel | 元 | N/A | Yes | N/A |
