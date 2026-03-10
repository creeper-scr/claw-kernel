# claw-kernel 内核功能规范

> 本文档从 OpenClaw 的功能全集中提取**属于内核职责的部分**，
> 定义每项能力的边界：内核提供什么、应用自己实现什么。
>
> 设计原则：**内核只提供机制，从不规定策略。**

---

## 目录

1. [内核定位](#内核定位)
2. [F1 · 消息渠道抽象](#f1--消息渠道抽象)
3. [F2 · 对话上下文管理](#f2--对话上下文管理)
4. [F3 · LLM 提供商抽象](#f3--llm-提供商抽象)
5. [F4 · 工具执行运行时](#f4--工具执行运行时)
6. [F5 · 技能按需加载引擎](#f5--技能按需加载引擎)
7. [F6 · 事件触发系统（Cron + Webhook）](#f6--事件触发系统)
8. [F7 · 多 Agent 编排](#f7--多-agent-编排)
9. [F8 · 安全与隔离模型](#f8--安全与隔离模型)
10. [F9 · 脚本扩展基础](#f9--脚本扩展基础)
11. [不属于内核的功能](#不属于内核的功能)
12. [功能边界速查表](#功能边界速查表)

---

## 内核定位

```
内核 = AI Agent 的操作系统
应用 = 跑在这个操作系统上的用户程序
```

三条硬约束：

| 约束 | 含义 |
|------|------|
| **面向所有类 OpenClaw 项目** | 不绑定任何一个上层产品的设计决策 |
| **语言无关** | Python / TypeScript / Go 等任何语言都能通过 IPC / SDK 调用内核能力，无需修改内核代码 |
| **无感体验** | 用户只写自己的应用代码；内核作为守护进程或库在背后运转，开发者感知不到它的存在 |

---

## F1 · 消息渠道抽象

### 来源

OpenClaw 支持 20+ 消息渠道，每个渠道独立实现。内核提取**渠道协议的公共部分**，让任意渠道都能接入同一套 Agent 运行时，而不是每次都重造路由层。

### 内核职责（机制）

```
Channel Trait
├── connect()          建立连接 / 监听入站消息
├── send(msg)          发送消息到渠道
├── receive() -> Stream<InboundMessage>   异步消息流
└── disconnect()

UnifiedMessage（归一化消息格式）
├── channel_id         来源渠道标识
├── sender_id          发送方在该渠道的 ID
├── content            消息正文（文本 / 多媒体 MIME 封装）
├── thread_id?         会话/线程追踪
└── metadata           渠道原始元数据（透传）

ChannelRouter
├── register(channel)       注册一个渠道实例
├── route(msg) -> AgentId   按策略决定投递给哪个 Agent
└── broadcast(agents, msg)  向多个 Agent 广播
```

**内核保证：**
- 入站消息经过归一化后以统一格式送入 EventBus
- 发送失败自动重试（指数退避，最多 3 次），超出后报错给应用层
- 每条消息携带唯一 `message_id`，保证幂等性（防重投）

### 应用职责（策略）

- 具体渠道实现（WhatsApp adapter / Discord adapter / stdin / webhook...）
- 路由规则（按用户 ID 路由、按渠道名路由、多 Agent 分流）
- 消息过滤与预处理（关键词过滤、@提及检测）
- 渠道特有能力（Discord 富文本、Telegram 按钮）

### 扩展点

```
任何语言通过以下方式接入：
  - 实现 Channel Protocol（IPC 协议文档见 docs/ipc-protocol.md）
  - 注册到 ChannelRouter（本地 IPC socket 调用）
  - 无需修改内核代码
```

---

## F2 · 对话上下文管理

### 来源

OpenClaw 的 Agent 需要在对话轮次之间维持上下文连贯性，同时当上下文窗口接近 token 上限时能优雅处理。内核提取**对话历史管理**这套机制，不涉及任何中长期记忆存储。

> **架构边界（v1.3.0 决策）：**
> 内核只管理 LLM 的**短期上下文窗口**（HistoryManager）。
> 中长期记忆、向量搜索、混合检索等能力属于**应用层策略**，
> 由应用自行选择 claw-memory crate 或第三方方案（Qdrant、Pinecone 等）实现。
> 此边界使内核无外部数据库依赖，适合嵌入任意环境。

### 内核职责（机制）

```
HistoryManager Trait（短期工作记忆 / 上下文窗口）
├── append(msg)                   追加一条消息到历史
├── messages() -> &[Message]      返回当前历史切片（送入 LLM）
├── token_count() -> usize        估算当前历史占用的 token 数
├── set_overflow_callback(fn)     上下文满时触发 —— 应用决定如何归档/截断
└── clear()                       清空历史（新对话开始）
```

**内核保证：**
- 默认实现：`InMemoryHistory`（无 I/O 依赖，零配置可用）
- 可选持久化：`SqliteHistory`（单文件 SQLite，开箱即用）
- overflow callback 机制让应用完全控制"记忆满了怎么办"——可以截断、可以摘要、可以写入外部存储
- HistoryManager 是 Trait，应用可完全替换实现

### 应用职责（策略）

- **中长期记忆存储**：选择 claw-memory、Qdrant、Pinecone 等，完全由应用决定
- **记忆检索注入**：从外部存储检索相关记忆后，以 system prompt 或 user message 形式注入 HistoryManager
- **上下文溢出处理**：实现 overflow callback（摘要压缩 / 截断 / 归档）
- **记忆生命周期**：何时写、写什么、何时忘——内核不参与
- **Embedding / 向量搜索**：应用层选型，内核不依赖任何 embedding 服务

### 扩展点

```
应用可替换：
  - HistoryManager 实现（自定义上下文压缩算法、分布式历史等）
内核仅依赖 Trait，不捆绑任何持久化后端。

中长期记忆推荐使用 claw-memory crate（可选应用层依赖）：
  - SqliteMemoryStore：本地 SQLite + FTS5
  - NgramEmbedder：64 维 bigram+trigram 向量
  - hybrid_search：BM25 + 语义混合检索（α 权重可调）
  - SecureMemoryStore：50MB 命名空间配额隔离
```

---

## F3 · LLM 提供商抽象

### 来源

OpenClaw 的"模型无关性"（BYOK + 多 LLM 支持）是其核心价值之一。内核提取**协议适配层**，让上层代码完全不关心底层是哪家模型。

### 内核职责（机制）

```
LLMProvider Trait
├── complete(messages, opts) -> CompletionResponse
├── complete_stream(messages, opts) -> Stream<Delta>
├── token_count(text) -> usize
├── provider_id() -> &str
└── model_id() -> &str

MessageFormat Trait（两大主流协议适配）
├── OpenAIFormat     覆盖 OpenAI / DeepSeek / Moonshot / Qwen / Grok 等 50+ 提供商
└── AnthropicFormat  覆盖 Anthropic Claude / AWS Bedrock

RetryPolicy（内置指数退避）
├── max_retries      默认 3 次
├── base_delay_ms    默认 1000ms
└── 可覆盖为自定义策略
```

**内核内置实现（开箱即用）：**

| 提供商 | 协议格式 | 功能 |
|--------|----------|------|
| Anthropic | AnthropicFormat | 完整 + 流式 + 工具调用 |
| OpenAI | OpenAIFormat | 完整 + 流式 + 工具调用 |
| DeepSeek | OpenAIFormat | 完整 + 流式 |
| Moonshot | OpenAIFormat | 完整 + 流式 |
| Ollama | OllamaFormat | 本地推理，完整 + 流式 |

**内核保证：**
- 任何新的 OpenAI 兼容提供商：只需提供 base_url + auth，0 行 HTTP 代码
- 流式响应统一为 `Stream<Delta>`，屏蔽各提供商 SSE 格式差异
- 工具调用（Function Calling）统一编解码，应用层收到统一格式

### 应用职责（策略）

- 选择使用哪个提供商 / 哪个模型
- API Key 管理与轮换
- 成本控制与限流策略
- 模型回退策略（主模型不可用时降级到备用）
- 自定义提供商接入（实现 `LLMProvider` trait）
- **Embedding / 向量搜索**：应用层选型（可用 NgramEmbedder 来自 claw-memory crate），内核不捆绑任何 embedding 服务

---

## F4 · 工具执行运行时

### 来源

OpenClaw 内置 25 种核心工具，并支持沙盒隔离执行。内核提取**工具注册、调度、权限执行、审计**这套基础设施。

### 内核职责（机制）

```
ToolRegistry
├── register(tool)              注册工具（Native Rust / Script）
├── unregister(name)
├── get(name) -> Tool
├── list() -> Vec<ToolMeta>
├── execute(name, args, ctx)    带超时 + 权限检查执行
└── recent_log(n) -> Vec<LogEntry>   审计日志

Tool Trait（应用实现具体工具）
├── name() -> &str
├── description() -> &str       LLM 可读的工具描述
├── schema() -> ToolSchema      JSON Schema 参数定义
├── permissions() -> PermissionSet
├── timeout() -> Duration       默认 30s
└── execute(args, ctx) -> ToolResult

PermissionSet（沙盒策略对象）
├── FsPermissions    允许读/写的路径（glob）
├── NetworkPermissions  允许的域名 + 端口
└── SubprocessPolicy    允许 / 禁止派生子进程

HotLoader
├── watch_directory(path)        监听目录变化（50ms 防抖）
├── load_script(path)            动态加载脚本工具
└── unload(name)                 热卸载
```

**内核保证：**
- `execute()` 超时强制终止（不允许工具无限运行）
- Safe 模式下 PermissionSet 强制检查，违规立即拒绝
- 每次调用追加 LogEntry（时间戳、Agent ID、成功/失败、耗时）
- 并发安全：DashMap 存储，允许多 Agent 同时调用不同工具

### 应用职责（策略）

- 具体工具实现（文件读写、Shell 执行、浏览器控制、API 调用...）
- 决定哪些工具开放给哪个 Agent
- 审计日志的持久化与告警策略
- 工具失败的业务级重试逻辑

---

## F5 · 技能按需加载引擎

### 来源

OpenClaw 的 AgentSkills / ClawHub 技能系统核心创新在于"按需加载"——LLM 不会在系统提示词里收到所有技能全文，只收到一份紧凑索引，按需读取。内核提取这套**加载机制**。

### 内核职责（机制）

```
SkillManifest（技能描述规范）
├── name: String
├── description: String      一行摘要（用于构建索引）
├── path: PathBuf            完整技能内容所在路径
├── version: SemVer
└── tags: Vec<String>

SkillIndex（注入系统提示词的紧凑索引）
  = Vec<{ name, description, path }>   仅包含三元组，不含全文

SkillLoader
├── scan_directory(path) -> Vec<SkillManifest>
├── build_index(manifests) -> SkillIndex
├── load_full(name) -> SkillContent      按需读取完整内容
└── resolve_priority(dirs) -> Vec<SkillManifest>  多目录优先级合并

优先级顺序（高→低）：
  <workspace>/skills → ~/.claw/skills → 内置技能
```

**内核保证：**
- 系统提示词注入的是**索引**，不是全文（无论安装多少技能，提示词体积恒定）
- LLM 调用 `load_full(name)` 时才读取完整技能内容（一次工具调用）
- 同名技能按优先级覆盖（workspace > user global > builtin）

### 应用职责（策略）

- 具体技能内容编写（SKILL.md / 任意格式）
- 技能版本管理与分发（如接入 ClawHub）
- 技能安全审计（内核不做信任判断）
- 决定哪些技能目录对哪个 Agent 可见

---

## F6 · 事件触发系统

### 来源

OpenClaw 的 Cron 定时任务 + Webhook 触发器让 Agent 脱离"等待输入"模式，变成能主动行动的自动化 Agent。内核提取**触发-事件-路由**这条管道。

### 内核职责（机制）

```
Trigger（触发源抽象）
├── CronTrigger       接收 cron 表达式，按时发出触发事件
├── WebhookTrigger    监听 HTTP 端点，收到 POST 后发出触发事件
└── EventTrigger      订阅 EventBus 中的任意事件，条件满足时转发

TriggerEvent（归一化触发事件）
├── trigger_id
├── trigger_type: Cron | Webhook | Event
├── fired_at: Timestamp
├── payload: serde_json::Value   Cron 为空；Webhook 为请求体
└── target_agent: AgentId?       None 表示广播

WebhookServer（内核内置）
├── POST /hooks/{trigger_id}     接收外部回调
├── HMAC-SHA256 签名验证（可选，强烈建议配置）
└── 限流：默认 100 req/min/trigger

CronScheduler（内核内置）
├── add(expr, trigger_id)        注册定时任务
├── remove(trigger_id)
└── list() -> Vec<CronJob>

所有触发事件统一进入 EventBus，由 AgentOrchestrator 分发。
```

**内核保证：**
- Cron 精度：秒级（支持 6 字段表达式）
- Webhook 去重：相同 `X-Request-Id` 的请求在 60s 内只处理一次
- 触发器持久化：重启后自动恢复（存储在内核管理的 SQLite 中）

### 应用职责（策略）

- 决定触发后执行什么动作（注入什么 message 给 Agent）
- Webhook HMAC secret 配置
- 触发频率超出内核限流时的业务处理
- 触发结果的通知与监控

---

## F7 · 多 Agent 编排

### 来源

OpenClaw 支持将不同渠道/账号路由到相互隔离的 Agent 实例。内核提取 **Agent 生命周期管理 + 进程间通信**这套基础设施。

### 内核职责（机制）

```
AgentOrchestrator
├── spawn(config) -> AgentHandle     启动新 Agent 进程/任务
├── kill(handle)                     终止 Agent
├── list() -> Vec<AgentStatus>       查询所有 Agent 状态
├── steer(handle, msg)               向运行中的 Agent 注入消息
├── health_check()                   定期探活，自动重启崩溃的 Agent
└── apply_restart_policy(handle, policy)

AgentHandle（Agent 的引用凭证）
├── agent_id: AgentId
├── status: Running | Paused | Stopped | Crashed
└── resource_usage: CpuMs + MemoryBytes

A2AMessage（Agent 间通信格式）
├── source: AgentId
├── target: AgentId?             None = 广播
├── message_type: Request | Response | Event | Discovery | Heartbeat
├── payload: serde_json::Value
├── priority: Critical → Background（5 级）
├── correlation_id?              请求/响应配对
└── ttl_secs?                   消息过期时间

IpcRouter（跨进程消息路由）
├── send(msg)               发送到目标 Agent（跨进程 IPC）
├── subscribe(agent_id)     订阅某 Agent 的事件
└── broadcast(msg)          广播给所有在线 Agent

RestartPolicy
├── Always                  崩溃后立即重启
├── OnFailure(max_retries)  失败时重启，超过次数后放弃
└── Never
```

**内核保证：**
- 每个 Agent 在独立进程（或隔离任务）中运行，崩溃不传播
- A2A 消息通过 IPC 传输（Unix Domain Socket / Named Pipe），不经过外部网络
- AgentId 全局唯一（AtomicU64 计数器，防并发冲突）
- Discovery 机制：Agent 可查询其他 Agent 暴露的能力列表

### 应用职责（策略）

- Agent 的业务角色划分（Research Agent / Writing Agent / ...）
- 共享 Memory 的读写策略（内核只提供隔离，不提供共享）
- 跨 Agent 任务编排逻辑（Orchestrator 模式 or 去中心化协作）
- Agent 间通信的业务协议设计

---

## F8 · 安全与隔离模型

### 来源

OpenClaw 的"本地优先 + 沙盒隔离"设计。内核将其固化为**不可绕过的硬约束**，同时通过 Safe/Power 双模式给应用层灵活性。

### 内核职责（机制）

```
ExecutionMode
├── Safe    默认模式：沙盒强制、网络限制、不允许子进程
└── Power   显式启用：全访问，需要 power_key + 明确 flag

SandboxBackend Trait（PAL 层实现平台差异）
├── Linux   seccomp-bpf + Namespaces
├── macOS   sandbox(7) profile
└── Windows AppContainer

ModeTransitionGuard
├── enter_power_mode(power_key) -> Result<Guard>
└── Guard 析构时记录审计日志（无法静默关闭）

CredentialStore（内存加密存储）
├── set(key, value)      AES-256-GCM 加密存储
├── get(key) -> Secret
└── 进程退出时自动清零（zeroize）

AuditLog（不可篡改日志流）
  每次工具执行 / 模式切换 / Agent 启动 均写入审计条目
  格式：{ timestamp, event_type, agent_id, details, mode }
```

**内核硬约束（任何代码不能绕过）：**
- Safe 模式下，文件系统访问只能在 allowlist 路径内
- Safe 模式下，Agent 不能直接访问其他 Agent 的 Memory 命名空间
- Power 模式 → Safe 模式**必须重启**（防止已受损 Agent 降级隐藏痕迹）
- Kernel 代码本身对脚本层不可见（脚本无法 `require` 内核内部模块）

### 应用职责（策略）

- 何时启用 Power 模式（由用户或运维显式决定）
- 具体的 allowlist 路径配置
- 审计日志的持久化、告警与合规上报
- HMAC secret / API Key 的来源管理

---

## F9 · 脚本扩展基础

### 来源

内核需要成为"语言无关"的基础。脚本层（Lua / V8/Deno）是让各种语言开发者都能扩展内核能力的接入层。

### 内核职责（机制）

```
ScriptEngine Trait
├── execute(script, ctx) -> ScriptResult
├── call(fn_name, args) -> Value
└── load_module(path)

RustBridge（内核暴露给脚本层的 API，claw.* 命名空间）
├── claw.llm.complete(messages, opts)
├── claw.llm.stream(messages, opts) -> AsyncIterable
├── claw.tools.register(def)
├── claw.tools.call(name, params)
├── claw.tools.list()
├── claw.memory.search(query, k)
├── claw.memory.memorize({ content, space })
├── claw.memory.logEpisode({ kind, content, tags })
├── claw.memory.queryEpisodes(filter)
├── claw.events.emit(event, data)
├── claw.events.on(event, handler)
├── claw.fs.read(path)
├── claw.fs.write(path, data)
├── claw.fs.glob(pattern)
├── claw.agent.spawn(config)
└── claw.agent.kill(handle)

HotLoader（运行时加载脚本工具）
├── watch_directory(path)       监听脚本目录变化
├── load_script(path)           加载脚本定义的工具
└── 50ms 防抖 + 自动重注册
```

**内置引擎：**

| 引擎 | 体积 | 适用场景 |
|------|------|----------|
| Lua (mlua) | ~500KB | 默认，零依赖，轻量工具 |
| V8 (Deno) | ~100MB | TypeScript / 复杂 Agent 逻辑 |

**内核保证：**
- 脚本崩溃不导致内核崩溃（进程隔离）
- Safe 模式下，`claw.fs` 和网络操作受 PermissionSet 约束
- 任何语言只要能做 IPC + JSON，就能调用 RustBridge（不局限于 Lua/V8）

### 应用职责（策略）

- 编写具体脚本逻辑（工具实现、Agent 行为）
- 选择使用哪个脚本引擎
- 热加载策略（哪些目录监听、哪些脚本自动重载）

---

## 不属于内核的功能

以下功能是**应用层能力**，内核不实现，也不应该耦合：

| OpenClaw 功能 | 为什么不属于内核 |
|---------------|------------------|
| 浏览器控制（CDP） | 特定工具，应用按需集成 Chromium |
| 语音交互（Wake Word / TTS） | 特定渠道 + 硬件依赖，平台高度耦合 |
| 实时画布（Live Canvas / A2UI） | 特定 UI 协议，非通用基础设施 |
| 具体渠道实现（WhatsApp / Discord 等） | 渠道 adapter，实现 Channel Trait 即可 |
| ClawHub 注册表 | 社区分发层，非运行时职责 |
| 12 层记忆架构 | 应用层策略，在 MemoryStore 上构建 |
| 知识图谱 / 领域 RAG | 应用层策略 |
| 内容工厂 / 游戏开发流水线等用例 | 上层应用，不是内核能力 |
| 模型选择策略 / 成本控制 | 应用决策，内核只提供切换机制 |
| 中长期记忆存储与检索（MemoryStore / hybrid_search） | v1.3.0 决策：应用层选型，内核只管上下文窗口 |

---

## 功能边界速查表

| 功能模块 | 内核提供（机制） | 应用实现（策略） |
|----------|-----------------|-----------------|
| **消息渠道** | Channel Trait、UnifiedMessage、ChannelRouter、重试、幂等 | 具体渠道 adapter、路由规则、消息预处理 |
| **上下文管理** | HistoryManager Trait、InMemoryHistory、SqliteHistory（可选）、overflow callback | 中长期记忆存储（推荐 claw-memory crate）、记忆检索注入、Embedding 选型 |
| **LLM 提供商** | LLMProvider Trait、OpenAI/Anthropic 格式适配、流式统一、内置 5 个提供商 | 提供商选择、BYOK 管理、回退策略 |
| **工具执行** | ToolRegistry、PermissionSet 强制、超时、审计日志、HotLoader | 具体工具实现、审计持久化 |
| **技能加载** | SkillManifest、SkillIndex、按需 load_full、优先级合并 | 技能内容、版本分发、安全审计 |
| **调度触发** | CronScheduler、WebhookServer、TriggerEvent 归一化、去重、持久化 | 触发后的动作注入、HMAC 配置 |
| **多 Agent** | AgentOrchestrator、A2AMessage、IpcRouter、重启策略、Discovery | Agent 角色划分、跨 Agent 协调逻辑 |
| **安全** | Safe/Power 双模式、SandboxBackend、CredentialStore、不可篡改 AuditLog | Allowlist 配置、Power 模式授权策略 |
| **脚本扩展** | ScriptEngine Trait、RustBridge (claw.*)、Lua + V8 内置引擎、HotLoader | 脚本逻辑、引擎选择、热加载规则 |

---

*本文档是内核功能的**规范文档**，不描述具体实现。实现细节见 [architecture/overview.md](architecture/overview.md) 和各 crate 的文档。*
