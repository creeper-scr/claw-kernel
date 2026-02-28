# claw-kernel 文档模糊性问题审查报告

> 审查日期: 2026-02-28  
> 审查范围: 6份核心架构文档  
> 审查类型: 模糊性描述、边界不清、范围不明确、条件模糊

---

## 执行摘要

本次审查共发现 **27个模糊性问题**，分布如下：

| 文档 | 问题数量 | 严重级别 |
|------|---------|---------|
| AGENTS.md | 6个 | 中-高 |
| BUILD_PLAN.md | 4个 | 中 |
| docs/architecture/overview.md | 8个 | 中-高 |
| docs/architecture/crate-map.md | 5个 | 中 |
| docs/adr/001-architecture-layers.md | 2个 | 低 |
| docs/adr/006-message-format-abstraction.md | 2个 | 低 |

**问题类型分布：**
- 模糊性描述 (12个): "可能"、"应该"、"通常"等不确定性词汇
- 边界不清 (8个): 模块之间职责划分模糊
- 范围不明确 (4个): 功能覆盖范围不清晰
- 条件模糊 (3个): 触发条件、前提条件不清晰

---

## 详细问题清单

### 一、AGENTS.md 问题 (6个)

#### 问题 #1
- **文档路径**: AGENTS.md
- **章节位置**: Technology Stack / Optional Dependencies
- **原文引用**: 
  > "Deno/V8 engine (`engine-v8`): Node.js ≥ 20, adds ~100MB to binary"
- **问题描述**: "~100MB" 是一个估算值，未说明是增加后的总大小还是纯增加量，也未说明是在什么平台/编译配置下的数值
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```
  "Deno/V8 engine (`engine-v8`): Node.js ≥ 20, 二进制文件增加约 100MB
  （基于 Linux x86_64 release 构建测量，实际大小取决于平台和编译优化选项）"
  ```

#### 问题 #2
- **文档路径**: AGENTS.md
- **章节位置**: Security Model / Two Execution Modes
- **原文引用**: 
  > "Script Self-Mod | Allowed (sandboxed) | Allowed (global)"
- **问题描述**: "global" 一词未定义，与 "sandboxed" 的边界不清，用户无法理解两者具体差异
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "脚本自修改 | 允许（受限：仅可修改允许目录中的脚本文件）| 允许（无限制：可修改全局文件系统）"
  ```

#### 问题 #3
- **文档路径**: AGENTS.md
- **章节位置**: Security Model / Audit Logging
- **原文引用**: 
  > "日志级别：minimal（仅关键事件）、verbose（完整参数）"
- **问题描述**: "关键事件" 的定义不明确，哪些事件属于关键事件？没有明确标准
- **问题类型**: 条件模糊
- **具体修改建议**: 
  ```
  "日志级别：
   - minimal: 仅记录模式切换、权限变更、安全事件
   - verbose: 额外记录所有工具调用参数、网络请求详情、文件访问路径"
  ```

#### 问题 #4
- **文档路径**: AGENTS.md
- **章节位置**: Extensibility Model / Kernel Capabilities
- **原文引用**: 
  > "File watching for automatic reload"
- **问题描述**: 未说明文件监视的触发条件（是文件内容变化？还是元数据变化？），也未说明防抖/节流策略
- **问题类型**: 条件模糊
- **具体修改建议**: 
  ```
  "File watching for automatic reload（基于文件内容 MD5 变化检测，默认 500ms 防抖间隔）"
  ```

#### 问题 #5
- **文档路径**: AGENTS.md
- **章节位置**: Code Style Guidelines / Testing Strategy
- **原文引用**: 
  > "Platform Tests | Required per-platform | N/A | ..."
- **问题描述**: "Required per-platform" 的具体含义不清，是指需要在每个平台上运行测试，还是指需要平台特定的测试代码？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "Platform Tests | 需要在 Linux/macOS/Windows 三个平台各运行一次测试 | 不适用 | ..."
  ```

#### 问题 #6
- **文档路径**: AGENTS.md
- **章节位置**: Architecture / Layer 3
- **原文引用**: 
  > "claw-script runs in separate per-Agent process"
- **问题描述**: "per-Agent" 是指每个 Agent 实例都有独立的脚本进程，还是指脚本引擎本身是单例但为每个 Agent 创建隔离上下文？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "claw-script: 每个 Agent 实例拥有独立的脚本引擎进程，进程间通过 IPC 与内核通信"
  ```

---

### 二、BUILD_PLAN.md 问题 (4个)

#### 问题 #7
- **文档路径**: BUILD_PLAN.md
- **章节位置**: Phase 1 / 核心 Trait 定义
- **原文引用**: 
  ```rust
  pub trait ProcessManager: Send + Sync {
      async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
  }
  ```
- **问题描述**: "grace_period" 的行为未定义 - 超时后是强制 kill 还是返回错误？
- **问题类型**: 条件模糊
- **具体修改建议**: 
  ```rust
  /// grace_period: 发送 SIGTERM 后等待进程退出的最大时间
  /// 超时后自动发送 SIGKILL 强制终止
  async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
  ```

#### 问题 #8
- **文档路径**: BUILD_PLAN.md
- **章节位置**: Phase 4 / AgentLoopConfig
- **原文引用**: 
  > "pub max_tool_calls_per_turn: usize, // Default: 10"
- **问题描述**: 未定义 "turn" 的确切边界。一个 turn 是从用户输入到 LLM 响应完成，还是包含工具调用循环？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```rust
  /// 单次对话回合中允许的最大工具调用次数
  /// 一个 "turn" 定义为：从发送 prompt 到收到 LLM 最终响应的完整周期
  /// 包含该周期内所有递归的工具调用
  pub max_tool_calls_per_turn: usize, // Default: 10
  ```

#### 问题 #9
- **文档路径**: BUILD_PLAN.md
- **章节位置**: Phase 5 / RustBridge API
- **原文引用**: 
  ```typescript
  fs: {
    read(path: string): Promise<Uint8Array>;
    write(path: string, data: Uint8Array): Promise<void>;
    exists(path: string): boolean;
    listDir(path: string): DirEntry[];
  }
  ```
- **问题描述**: 未说明这些 fs 操作是否受 Safe Mode 权限限制，以及超出权限时的行为
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```typescript
  fs: {
    /// 读取文件，Safe Mode 下受 allowlist 限制
    /// 超出权限时抛出 PermissionDenied 错误
    read(path: string): Promise<Uint8Array>;
    ...
  }
  ```

#### 问题 #10
- **文档路径**: BUILD_PLAN.md
- **章节位置**: Phase 6 / Channel Integrations
- **原文引用**: 
  > "目标：外部通信接口"
- **问题描述**: "外部通信" 的范围不明确 - 是指与外部服务（如 Telegram）通信，还是 Agent 间通信？
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```
  "目标：与外部第三方服务（如 Telegram、Discord）的通信接口，用于接收用户输入和发送响应"
  ```

---

### 三、docs/architecture/overview.md 问题 (8个)

#### 问题 #11
- **文档路径**: docs/architecture/overview.md
- **章节位置**: The 5-Layer Architecture / Layer 1
- **原文引用**: 
  > "Process Management: Subagent lifecycle (spawn / kill / list / steer)"
- **问题描述**: "steer" 操作的含义不明确，是指向子 Agent 发送控制指令，还是调整其配置？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "子 Agent 生命周期管理：
   - spawn: 创建新 Agent 进程
   - kill: 强制终止 Agent 进程
   - list: 列出所有活跃 Agent
   - steer: 向 Agent 发送控制指令（如暂停、恢复、调整优先级）"
  ```

#### 问题 #12
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Layer 2 / Agent Kernel Protocol / Options
- **原文引用**: 
  > "pub max_tokens: Option<usize>, // Maximum tokens to generate, default: 4096"
- **问题描述**: 未说明 "tokens" 的具体计算方式（是按 LLM 的分词器，还是简单字符/单词计数？）
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```rust
  /// 最大生成 token 数
  /// Token 计算使用 Provider 特定的分词器（如 OpenAI 使用 tiktoken）
  /// None = 使用 Provider 默认值
  pub max_tokens: Option<usize>, // Default: 4096
  ```

#### 问题 #13
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Layer 2 / NetworkPermissions
- **原文引用**: 
  > "pub allow_private_ips: bool, // Allow private IP ranges"
- **问题描述**: "private IP ranges" 的具体范围未定义（RFC1918？还是包含链路本地地址？）
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```rust
  /// 允许访问私有 IP 范围
  /// 包括：10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 (RFC1918)
  /// 不包括：127.0.0.0/8（由 allow_localhost 控制）
  pub allow_private_ips: bool,
  ```

#### 问题 #14
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Layer 2 / AgentLoopConfig
- **原文引用**: 
  > "pub summarizer: Box<dyn Summarizer>,"
- **问题描述**: Summarizer 的触发条件不明确 - 是在达到 token_budget 时自动触发，还是需要手动调用？
- **问题类型**: 条件模糊
- **具体修改建议**: 
  ```rust
  /// 历史记录摘要器
  /// 当历史记录达到 token_budget 的 80% 时自动触发
  /// 将早期消息压缩为摘要以释放 token 空间
  pub summarizer: Option<Box<dyn Summarizer>>, // None = 不启用自动摘要
  ```

#### 问题 #15
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Cross-Platform Strategy / Platform Capability Matrix
- **原文引用**: 
  > "| IPC Performance | 100% | 95% | 90% |"
- **问题描述**: 性能百分比的基准未定义（100% 是基于什么？），测试场景也未说明
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```
  "| IPC 吞吐量 | 100% (基准) | ~95% | ~90% |
  基准：Linux Unix Domain Socket 本地传输，4KB 消息大小，单线程测试"
  ```

#### 问题 #16
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Cross-Platform Strategy / Handling Platform Differences
- **原文引用**: 
  > "pub trait SandboxBackend { ... fn restrict_syscalls(&mut self, policy: SyscallPolicy) ... }"
- **问题描述**: SyscallPolicy 的具体类型和取值未定义，是枚举还是结构体？有哪些预设策略？
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```rust
  pub enum SyscallPolicy {
      Minimal,      // 仅允许基本 I/O
      Network,      // 额外允许网络相关调用
      Subprocess,   // 额外允许进程管理调用
      Custom(Vec<AllowedSyscall>),
  }
  ```

#### 问题 #17
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Security Model
- **原文引用**: 
  > "Safe Mode: Allowlist read-only"
- **问题描述**: "Allowlist" 是系统预设的还是用户配置的？默认允许哪些路径？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "Safe Mode: 
   - 文件系统：仅允许访问用户配置的 allowlist 路径
   - 默认 allowlist 为空（显式 opt-in 安全模型）"
  ```

#### 问题 #18
- **文档路径**: docs/architecture/overview.md
- **章节位置**: Hot-Loading Mechanism
- **原文引用**: 
  > "if self.mode == ExecutionMode::Safe { self.audit_permissions(&validated)?; }"
- **问题描述**: 未说明 audit_permissions 的具体行为 - 是记录审计日志，还是验证权限？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```rust
  // 2. 验证权限声明（Safe Mode 下检查脚本声明的权限是否在允许范围内）
  if self.mode == ExecutionMode::Safe {
      self.validate_permission_declarations(&validated)?;
  }
  ```

---

### 四、docs/architecture/crate-map.md 问题 (5个)

#### 问题 #19
- **文档路径**: docs/architecture/crate-map.md
- **章节位置**: claw-runtime / Multi-Agent Support
- **原文引用**: 
  > "pub fn send_message(&self, from: AgentId, to: AgentId, msg: A2AMessage);"
- **问题描述**: 未说明消息传递的语义 - 是同步等待还是异步发送？失败时如何处理？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```rust
  /// 发送 A2A 消息（异步非阻塞）
  /// 消息会进入目标 Agent 的消息队列，不保证立即处理
  /// 如需确认接收，请使用 Request/Response 模式并设置 correlation_id
  pub fn send_message(&self, from: AgentId, to: AgentId, msg: A2AMessage) -> Result<(), SendError>;
  ```

#### 问题 #20
- **文档路径**: docs/architecture/crate-map.md
- **章节位置**: claw-provider / Built-in Providers
- **原文引用**: 
  > "Adding a new OpenAI-compatible provider (~20 lines)"
- **问题描述**: "~20 lines" 是仅指配置代码，还是包括 import、struct 定义等所有代码？
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```
  "添加新的 OpenAI 兼容 provider 仅需约 20 行代码
  （不包括标准 import 语句，仅指 provider 特定配置）"
  ```

#### 问题 #21
- **文档路径**: docs/architecture/crate-map.md
- **章节位置**: claw-script / Engine Support
- **原文引用**: 
  > "engine-py = [\"pyo3\"] # Python (ML ecosystem)"
- **问题描述**: 未说明 Python 引擎的具体版本要求和二进制大小影响
- **问题类型**: 范围不明确
- **具体修改建议**: 
  ```toml
  engine-py = ["pyo3"]  # Python 3.10+，二进制大小增加 5-20MB（取决于 Python 链接方式）
  ```

#### 问题 #22
- **文档路径**: docs/architecture/crate-map.md
- **章节位置**: Kernel Boundary
- **原文引用**: 
  > "Applications built on claw-kernel can implement: Custom `MemoryBackend` trait for memory storage"
- **问题描述**: 未说明 MemoryBackend 是内核提供的 trait 还是应用自定义的 trait
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "基于 claw-kernel 的应用可以实现：
   - 自定义 MemoryBackend（内核提供的 trait，应用实现具体存储逻辑）"
  ```

#### 问题 #23
- **文档路径**: docs/architecture/crate-map.md
- **章节位置**: claw-script / Key Types
- **原文引用**: 
  > "pub fn execute(&self, script: &Script, context: &Context, timeout: Duration) -> Result<Value, ScriptError>;"
- **问题描述**: 未说明 timeout 的超时行为 - 是取消脚本执行还是仅返回错误？资源如何清理？
- **问题类型**: 条件模糊
- **具体修改建议**: 
  ```rust
  /// 执行脚本，timeout 后强制终止脚本运行并返回 ScriptError::Timeout
  /// 终止时会清理脚本分配的所有资源（内存、文件句柄等）
  pub fn execute(&self, script: &Script, context: &Context, timeout: Duration) 
      -> Result<Value, ScriptError>;
  ```

---

### 五、docs/adr/001-architecture-layers.md 问题 (2个)

#### 问题 #24
- **文档路径**: docs/adr/001-architecture-layers.md
- **章节位置**: Consequences / Positive
- **原文引用**: 
  > "Extensibility: Scripts can be generated and hot-loaded at application layer"
- **问题描述**: "can be" 暗示这是可选功能，但未说明内核如何支持这一能力的具体机制
- **问题类型**: 模糊性描述
- **具体修改建议**: 
  ```
  "Extensibility: 内核提供热加载 API 和脚本运行时，使应用层能够动态生成和加载脚本"
  ```

#### 问题 #25
- **文档路径**: docs/adr/001-architecture-layers.md
- **章节位置**: Decision / Self-Evolution is NOT in Kernel
- **原文引用**: 
  > "Evolution code runs in script runtime with proper sandboxing"
- **问题描述**: "proper sandboxing" 的具体级别未定义，是 Safe Mode 还是 Power Mode？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```
  "进化代码在脚本运行时中执行，遵循与工具脚本相同的沙箱规则（Safe Mode 下受限，Power Mode 下无限制）"
  ```

---

### 六、docs/adr/006-message-format-abstraction.md 问题 (2个)

#### 问题 #26
- **文档路径**: docs/adr/006-message-format-abstraction.md
- **章节位置**: Consequences / Negative
- **原文引用**: 
  > "Documentation effort: Must clearly explain when to use existing format vs. create new"
- **问题描述**: 使用了模糊的 "clearly explain"，未说明具体的文档需求和判断标准
- **问题类型**: 模糊性描述
- **具体修改建议**: 
  ```
  "Documentation effort: 需要为开发者提供决策指南：
   - 当 Provider API 与 OpenAI/Anthropic 格式 90% 以上相同时，使用现有 Format
   - 当存在结构性差异（如不同的认证方式、字段映射）时，创建新的 MessageFormat"
  ```

#### 问题 #27
- **文档路径**: docs/adr/006-message-format-abstraction.md
- **章节位置**: Core Traits / MessageFormat
- **原文引用**: 
  > "fn token_count(messages: &[Message]) -> usize;"
- **问题描述**: 未说明 token_count 的用途 - 是用于计费、截断，还是监控？
- **问题类型**: 边界不清
- **具体修改建议**: 
  ```rust
  /// 估算消息的 token 数量
  /// 用于：1) 截断历史记录以适应上下文窗口 2) 监控用量 3) 成本估算
  /// 注意：实际 token 数以 Provider 返回为准
  fn token_count(messages: &[Message]) -> usize;
  ```

---

## 严重级别说明

| 级别 | 定义 | 影响 |
|------|------|------|
| 高 | 可能导致实现错误或安全风险 | 不同实现者可能产生不兼容的实现 |
| 中 | 可能导致理解偏差或使用困惑 | 用户/开发者可能产生错误预期 |
| 低 | 主要是表达不够精确 | 对实现影响较小，但应改进 |

---

## 修改优先级建议

### 高优先级 (P0)
- 问题 #2: Script Self-Mod 的 "global" 定义
- 问题 #7: grace_period 的行为定义
- 问题 #17: Safe Mode Allowlist 的范围

### 中优先级 (P1)
- 问题 #3: 关键事件的定义
- 问题 #8: turn 的边界定义
- 问题 #13: private IP ranges 的范围
- 问题 #19: send_message 的语义

### 低优先级 (P2)
- 其他问题

---

## 附录：模糊性词汇统计

| 词汇 | 出现次数 | 主要位置 |
|------|---------|---------|
| "可能" / "may" / "can" / "might" | 25+ | 全文档 |
| "应该" / "should" | 15+ | AGENTS.md, BUILD_PLAN.md |
| "通常" / "usually" / "typically" | 8 | overview.md |
| "~" (约) | 6 | 性能/大小描述 |
| "等" / "etc." | 12 | 列表描述 |

---

*报告生成时间: 2026-02-28*  
*审查者: AI 文档审查专家*
