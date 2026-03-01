---
title: claw-kernel 架构概述
description: claw-kernel 完整五层架构文档
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](overview.md)

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

实现细节请参阅 [PAL 深度解析](pal.zh.md)。

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

完整架构决策请参阅 [ADR-006](../adr/006-message-format-abstraction.zh.md)。

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

完整分析请参阅 [ADR-003：安全模型](../adr/003-security-model.zh.md)。

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

使用方法请参阅 [扩展能力指南](../guides/extension-capabilities.zh.md)。

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

- **对于用户：** [入门指南](../guides/getting-started.zh.md)
- **对于贡献者：** [贡献指南](../../CONTRIBUTING.md)
- **对于架构细节：** [Crate 地图](crate-map.zh.md), [ADR 索引](../adr/README.zh.md)
- **对于平台特定信息：** [Linux](../platform/linux.md), [macOS](../platform/macos.md), [Windows](../platform/windows.md)
