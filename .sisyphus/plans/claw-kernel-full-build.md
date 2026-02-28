# claw-kernel 全量构建计划

## TL;DR

> **快速摘要**: 从零实现 claw-kernel 全部 7 个 crate（Layer 0-3），遵循架构文档和 BUILD_PLAN.md 的设计，采用 TDD 方式逐层构建。
> 
> **交付物**:
> - claw-pal: 平台抽象层（沙箱、IPC、进程管理）— Linux/macOS 完整实现，Windows 骨架
> - claw-runtime: 事件总线、IPC 路由、多智能体编排
> - claw-provider: LLM Provider 三层架构 + 3 个内置 Provider（Anthropic、OpenAI、Ollama）
> - claw-tools: Tool 协议、ToolRegistry、热加载
> - claw-loop: Agent 循环引擎、历史管理、停止条件
> - claw-script: Lua 脚本引擎 + RustBridge API
> - claw-kernel: Meta-crate 重导出
> - 全部 crate 可 `cargo build` + `cargo test --workspace` 通过
> 
> **预估工作量**: XL（8 个构建阶段，40+ 任务）
> **并行执行**: YES — 6 个 Wave
> **关键路径**: Phase 0 脚手架 → claw-pal traits → claw-pal 平台实现 → claw-runtime → claw-provider + claw-tools → claw-loop → claw-script → meta-crate

---

## Context

### 原始需求
用户要求根据已有的设计文档（架构概述、ADR、BUILD_PLAN.md、TECHNICAL_SPECIFICATION.md 等），从零实现整个 claw-kernel 项目。初始请求仅包含 Layer 0/0.5/1，后扩展为全量构建（Layer 0-3）。

### 访谈摘要
**关键决策**:
- 平台范围: Linux、macOS、Windows 全部实现（Windows 沙箱可先用骨架）
- 沙箱深度: Linux 和 macOS 完整实现真实系统 API，Windows 先骨架
- 测试策略: TDD — 测试先行
- 构建顺序: 严格遵循 BUILD_PLAN.md 的 8 阶段顺序
- 接口优先: trait 定义先于实现

**研究发现**:
- 所有 ADR (001-008) 已接受且稳定
- 工作区 Cargo.toml 已完整配置，但 `crates/` 为空
- 工作区中包含 `claw-memory` 和 `claw-channel`，但超出核心 6 crate 范围

### Metis 审查
**识别的差距（已处理）**:
- `claw-memory` 和 `claw-channel` 在 workspace 中但未定义 — 需脚手架阶段处理
- meta-crate 路径 `claw-kernel/` 在 workspace 但不存在 — 需在 Phase 0 创建
- `panic = "abort"` 与 mlua 不兼容 — 强制使用 `panic = "unwind"`
- seccomp `SCMP_ACT_KILL` + `join()` 导致 panic — 使用 `SCMP_ACT_ERRNO(EPERM)`
- `interprocess` 并发 I/O 在 macOS 上 panic — 使用单读线程 + channel 派发模式
- macOS `sandbox_init()` 初始化窗口 — 在 main() 中优先应用沙箱
- Windows AppContainer + Named Pipe DACL 问题 — Windows 沙箱先用骨架
- `deno_core` 无稳定性保证 — 精确锁版本，仅 Linux CI 测试
- PyO3 GIL/Tokio 死锁 — 使用独立 Python 线程 + channel 桥接

---

## Work Objectives

### 核心目标
从设计文档出发，构建完整可编译、可测试的 claw-kernel 项目。所有 crate 的 trait 定义和核心实现就绪，`cargo test --workspace` 全绿。

### 具体交付物
- `crates/claw-pal/` — 完整的平台抽象层
- `crates/claw-runtime/` — 完整的系统运行时
- `crates/claw-provider/` — 3 个 Provider 实现
- `crates/claw-tools/` — Tool 注册表和热加载
- `crates/claw-loop/` — Agent 循环引擎
- `crates/claw-script/` — Lua 引擎 + RustBridge
- `claw-kernel/` — Meta-crate
- `crates/claw-memory/` — 最小占位（满足 workspace 编译）
- `crates/claw-channel/` — 最小占位（满足 workspace 编译）

### 完成标准
- [ ] `cargo build --workspace` 成功
- [ ] `cargo test --workspace` 全部通过
- [ ] `cargo clippy --workspace` 无警告
- [ ] `cargo fmt --all -- --check` 通过
- [ ] 每个 crate 都有 trait 定义 + 至少一个实现
- [ ] 每个 crate 都有单元测试

### Must Have
- 所有 trait 严格按照 BUILD_PLAN.md 定义
- Linux seccomp 沙箱完整实现（使用 `SCMP_ACT_ERRNO(EPERM)`，非 KILL）
- macOS sandbox(7) 完整实现（unsafe FFI `sandbox_init()`）
- EventBus 使用 `tokio::sync::broadcast`，容量 1024
- IPC 使用 `interprocess` crate，单读线程模式
- Power Key 最少 12 字符，Argon2 哈希存储
- Lua 引擎为默认脚本引擎
- 热加载使用 `notify` crate，50ms debounce
- 全部 workspace profile 使用 `panic = "unwind"`

### Must NOT Have（护栏）
- **不要** 在任何 profile 中设置 `panic = "abort"`
- **不要** 使用 `SCMP_ACT_KILL` 处理线程级沙箱违规
- **不要** 对 `interprocess` socket 进行并发双向 split I/O
- **不要** 在 Tokio worker 中直接持有 Python GIL
- **不要** 实现超过 3 个 LLM Provider（Anthropic + OpenAI + Ollama）
- **不要** 实现 Channel 功能（仅占位 crate）
- **不要** 实现 SQLite 历史后端（仅内存实现）
- **不要** 添加过度注释/JSDoc — 注释应简洁有效
- **不要** 创建通用名称（data/result/item/temp）的变量
- **不要** 使用 `as any` / `@ts-ignore`（这是 Rust 项目，但同理不要 unsafe 滥用）

---

## Verification Strategy

> **零人工干预** — 所有验证由 Agent 执行。无例外。

### 测试决策
- **基础设施存在**: NO — 从零搭建
- **自动测试**: TDD（测试先行）
- **框架**: Rust 内置 `#[cfg(test)]` + `cargo test`
- **如果 TDD**: 每个 task 遵循 RED（失败测试）→ GREEN（最小实现）→ REFACTOR

### QA 策略
每个 task 必须包含 Agent 执行的 QA 场景。
证据保存到 `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`。

- **库/模块**: 使用 Bash (`cargo test -p {crate}`) — 编译、运行测试、对比输出
- **CLI**: 使用 Bash (`cargo run --example`) — 运行示例，验证输出
- **构建**: 使用 Bash (`cargo build --workspace`, `cargo clippy`) — 验证全局编译

---

## Execution Strategy

### 并行执行 Wave

> 最大化吞吐量：独立任务分组并行执行。
> 每个 Wave 完成后才开始下一个。
> 目标: 5-8 tasks/wave。

```
Wave 0 (脚手架 — 全局基础，必须最先):
├── Task 1: 工作区脚手架 — 所有 8 个 crate 的 Cargo.toml + lib.rs [quick]
└── Task 2: 修复工作区配置 — panic profile、meta-crate 路径 [quick]

Wave 1 (claw-pal traits + types — 无外部依赖，MAX PARALLEL):
├── Task 3: claw-pal 错误类型和通用类型 [quick]
├── Task 4: SandboxBackend trait + 沙箱类型定义 [quick]
├── Task 5: IpcTransport trait + IPC 类型定义 [quick]
├── Task 6: ProcessManager trait + 进程类型定义 [quick]
├── Task 7: ExecutionMode + PowerKey (Layer 0 安全模型) [deep]
└── Task 8: dirs 模块（跨平台标准目录） [quick]

Wave 2 (claw-pal 平台实现 — 依赖 Wave 1 traits，MAX PARALLEL):
├── Task 9: Linux sandbox 实现 (seccomp-bpf + namespaces) [deep]
├── Task 10: macOS sandbox 实现 (sandbox(7) FFI) [deep]
├── Task 11: Windows sandbox 骨架 (AppContainer stub) [quick]
├── Task 12: IPC 传输实现 — 全平台 (interprocess) [deep]
├── Task 13: ProcessManager 实现 — 全平台 [deep]
└── Task 14: claw-pal lib.rs 整合 + 集成测试 [unspecified-high]

Wave 3 (claw-runtime + Layer 2 traits — 依赖 claw-pal，MAX PARALLEL):
├── Task 15: EventBus + FilteredReceiver + Event 枚举 [deep]
├── Task 16: IpcRouter 实现 [unspecified-high]
├── Task 17: Runtime 结构体 + AgentOrchestrator [deep]
├── Task 18: claw-provider traits (MessageFormat + HttpTransport + LLMProvider) [quick]
├── Task 19: claw-tools traits (Tool + ToolRegistry + PermissionSet) [quick]
└── Task 20: claw-loop traits (AgentLoop + StopCondition + HistoryManager) [quick]

Wave 4 (Layer 2 实现 — 依赖 Wave 3 traits，MAX PARALLEL):
├── Task 21: OpenAIFormat 实现 [deep]
├── Task 22: AnthropicFormat 实现 [deep]
├── Task 23: OllamaFormat 实现 [unspecified-high]
├── Task 24: HttpTransport 默认实现 + Provider 组装 [deep]
├── Task 25: ToolRegistry 核心实现 [deep]
├── Task 26: 热加载机制 (notify + 文件监视) [deep]
├── Task 27: AgentLoop + AgentLoopBuilder 实现 [deep]
├── Task 28: 内置 StopCondition + InMemoryHistory [unspecified-high]
└── Task 29: claw-runtime 集成测试 [unspecified-high]

Wave 5 (Layer 3 + Meta-crate — 依赖 Wave 4):
├── Task 30: ScriptEngine trait + Lua 引擎 (mlua) [deep]
├── Task 31: RustBridge API (脚本 ↔ Rust 桥接) [deep]
├── Task 32: 脚本热加载集成 [unspecified-high]
├── Task 33: claw-memory 最小占位 crate [quick]
├── Task 34: claw-channel 最小占位 crate [quick]
├── Task 35: claw-kernel meta-crate [quick]
└── Task 36: 全局编译验证 + clippy + fmt [unspecified-high]

Wave FINAL (验证 — 所有任务完成后，4 并行):
├── Task F1: 计划合规审计 (oracle)
├── Task F2: 代码质量审查 (unspecified-high)
├── Task F3: 实际 QA 验证 (unspecified-high)
└── Task F4: 范围保真检查 (deep)

关键路径: T1 → T3-8 → T9-14 → T15-17 → T21-28 → T30-32 → T36 → F1-F4
并行加速: ~65% faster than sequential
最大并发: 6 (Wave 1 & 2)
```

### 依赖矩阵

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 所有后续 | 0 |
| 2 | 1 | 所有后续 | 0 |
| 3 | 2 | 4-8, 9-14 | 1 |
| 4 | 3 | 9-11, 14 | 1 |
| 5 | 3 | 12, 14 | 1 |
| 6 | 3 | 13, 14 | 1 |
| 7 | 3 | 9-11, 14 | 1 |
| 8 | 3 | 14 | 1 |
| 9 | 4, 7 | 14 | 2 |
| 10 | 4, 7 | 14 | 2 |
| 11 | 4, 7 | 14 | 2 |
| 12 | 5 | 14, 16 | 2 |
| 13 | 6 | 14, 17 | 2 |
| 14 | 9-13 | 15-17 | 2 |
| 15 | 14 | 16, 17, 29 | 3 |
| 16 | 12, 15 | 17, 29 | 3 |
| 17 | 13, 15, 16 | 29 | 3 |
| 18 | 2 | 21-24 | 3 |
| 19 | 2 | 25-26 | 3 |
| 20 | 2 | 27-28 | 3 |
| 21 | 18 | 24 | 4 |
| 22 | 18 | 24 | 4 |
| 23 | 18 | 24 | 4 |
| 24 | 21-23 | 27 | 4 |
| 25 | 19 | 26, 27 | 4 |
| 26 | 25 | 32 | 4 |
| 27 | 20, 24, 25 | 30 | 4 |
| 28 | 20 | 27 | 4 |
| 29 | 15-17 | — | 4 |
| 30 | 27 | 31 | 5 |
| 31 | 30, 25 | 32 | 5 |
| 32 | 26, 31 | 36 | 5 |
| 33 | 2 | 35 | 5 |
| 34 | 2 | 35 | 5 |
| 35 | 33, 34 | 36 | 5 |
| 36 | All | F1-F4 | 5 |

### Agent 调度摘要

- **Wave 0**: **2** — T1 → `quick`, T2 → `quick`
- **Wave 1**: **6** — T3-T6 → `quick`, T7 → `deep`, T8 → `quick`
- **Wave 2**: **6** — T9-T10 → `deep`, T11 → `quick`, T12-T13 → `deep`, T14 → `unspecified-high`
- **Wave 3**: **6** — T15 → `deep`, T16 → `unspecified-high`, T17 → `deep`, T18-T20 → `quick`
- **Wave 4**: **9** — T21-T22 → `deep`, T23 → `unspecified-high`, T24-T27 → `deep`, T28-T29 → `unspecified-high`
- **Wave 5**: **7** — T30-T31 → `deep`, T32 → `unspecified-high`, T33-T35 → `quick`, T36 → `unspecified-high`
- **FINAL**: **4** — F1 → `oracle`, F2-F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

- [ ] 1. 工作区脚手架 — 所有 crate 的 Cargo.toml + lib.rs

  **What to do**:
  - 为所有 8 个 crate 创建目录: `crates/claw-pal/`, `crates/claw-runtime/`, `crates/claw-provider/`, `crates/claw-tools/`, `crates/claw-loop/`, `crates/claw-memory/`, `crates/claw-channel/`, `crates/claw-script/`
  - 创建 meta-crate 目录: `claw-kernel/` (在项目根目录下)
  - 每个 crate 创建 `Cargo.toml` 和 `src/lib.rs`
  - 各 crate Cargo.toml 使用 `workspace.package` 继承版本信息
  - 各 crate 按需声明对其他 workspace crate 的依赖（路径依赖）
  - `claw-memory` 和 `claw-channel` 为最小占位: 仅 `// placeholder crate`
  - `claw-kernel/src/lib.rs` 重导出其他 crate
  - 运行 `cargo check --workspace` 确保编译通过

  **Must NOT do**:
  - 不要添加任何实质性实现代码
  - 不要在占位 crate 中添加复杂依赖

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 纯文件创建，无复杂逻辑
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 0 (sequential with Task 2)
  - **Blocks**: Task 2, 及所有后续 tasks
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `Cargo.toml:4-15` — workspace members 列表，必须与创建的目录一一对应
  - `Cargo.toml:18-24` — workspace.package 配置，各 crate 应继承
  - `Cargo.toml:29-47` — workspace.dependencies，各 crate 按需引用

  **API/Type References**:
  - `BUILD_PLAN.md:477-500` — Phase 8 meta-crate 的 re-export 结构
  - `docs/architecture/crate-map.md` — crate 间依赖关系图

  **WHY Each Reference Matters**:
  - `Cargo.toml` members 列表定义了必须创建哪些目录
  - `crate-map.md` 决定了各 crate Cargo.toml 中的 path dependency 关系
  - meta-crate re-export 结构决定了 `claw-kernel/src/lib.rs` 的内容

  **Acceptance Criteria**:
  - [ ] 9 个 crate 目录全部存在且有 Cargo.toml + src/lib.rs
  - [ ] `cargo check --workspace` → 成功

  **QA Scenarios:**
  ```
  Scenario: workspace 全量编译检查
    Tool: Bash
    Preconditions: 所有 crate 目录和文件已创建
    Steps:
      1. 执行 `cargo check --workspace`
      2. 检查退出码为 0
      3. 执行 `ls crates/*/src/lib.rs claw-kernel/src/lib.rs` 确认文件存在
    Expected Result: cargo check 成功，9 个 lib.rs 全部存在
    Failure Indicators: 编译错误、缺少文件
    Evidence: .sisyphus/evidence/task-1-workspace-check.txt
  ```

  **Commit**: YES (Wave 0 commit)
  - Message: `chore: scaffold all workspace crates with Cargo.toml and lib.rs`
  - Files: `crates/*/Cargo.toml`, `crates/*/src/lib.rs`, `claw-kernel/Cargo.toml`, `claw-kernel/src/lib.rs`
  - Pre-commit: `cargo check --workspace`

---

- [ ] 2. 修复工作区配置 — panic profile、依赖清理

  **What to do**:
  - 在根 `Cargo.toml` 中添加 `[profile.release]` 和 `[profile.dev]`，确保 `panic = "unwind"` (显式声明)
  - 验证所有 crate 的 feature flags 正确配置
  - 确保 Linux-only 依赖 (`libseccomp`, `nix`) 仅在 `claw-pal` 中引用且有正确的 `cfg` 条件
  - 确保可选依赖 (`mlua`, `deno_core`, `pyo3`) 仅在 `claw-script` 中引用
  - 运行 `cargo check --workspace` 确认

  **Must NOT do**:
  - 不要设置 `panic = "abort"`
  - 不要更改已锁定的依赖版本

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 配置文件修改，逻辑简单
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (依赖 Task 1)
  - **Parallel Group**: Wave 0 (after Task 1)
  - **Blocks**: Wave 1 所有 tasks (3-8)
  - **Blocked By**: Task 1

  **References**:
  - `Cargo.toml:59-61` — Linux-only 依赖配置
  - `Cargo.toml:109-127` — 现有 feature flags 配置
  - Metis 发现: `panic = "abort"` 与 mlua 不兼容（SIGABRT 崩溃），必须显式设置 unwind

  **Acceptance Criteria**:
  - [ ] `Cargo.toml` 包含 `[profile.release] panic = "unwind"`
  - [ ] `cargo check --workspace` → 成功
  - [ ] `grep -r 'panic.*abort' . --include='*.toml'` → 无结果

  **QA Scenarios:**
  ```
  Scenario: panic profile 验证
    Tool: Bash
    Steps:
      1. 执行 `grep -r 'panic' Cargo.toml`
      2. 确认只有 `panic = "unwind"` 出现
      3. 执行 `cargo check --workspace`
    Expected Result: 无 panic="abort"，编译成功
    Evidence: .sisyphus/evidence/task-2-panic-profile.txt
  ```

  **Commit**: YES (与 Task 1 合并)
  - Message: `chore: scaffold all workspace crates with Cargo.toml and lib.rs`

---

- [ ] 3. claw-pal 错误类型和通用类型定义

  **What to do**:
  - TDD: 先写测试验证类型的序列化/反序列化和 Display 实现
  - 创建 `crates/claw-pal/src/error.rs` — 统一错误类型 `PalError`，使用 `thiserror`
  - 创建 `crates/claw-pal/src/types/mod.rs` — 模块入口
  - 定义通用配置类型: `PathRule`, `NetRule`, `ResourceLimits`
  - 所有类型实现 `Clone`, `Debug`，有意义的类型实现 `PartialEq`

  **Must NOT do**:
  - 不要添加平台特定代码

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 4-8)
  - **Blocks**: Tasks 4-8, 9-14
  - **Blocked By**: Task 2

  **References**:
  - `docs/crates/claw-pal.md` — claw-pal 的完整类型列表
  - `docs/architecture/pal.md` — PAL 层类型定义详情
  - `BUILD_PLAN.md:36-63` — trait 中引用的所有类型

  **Acceptance Criteria**:
  - [ ] `cargo test -p claw-pal` → PASS
  - [ ] 所有类型实现 `Clone + Debug`

  **QA Scenarios:**
  ```
  Scenario: 错误类型和通用类型验证
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- error`
      2. `cargo test -p claw-pal -- types`
    Expected Result: 所有测试通过
    Evidence: .sisyphus/evidence/task-3-types.txt
  ```

  **Commit**: NO (Wave 1 合并提交)

---

- [ ] 4. SandboxBackend trait + 沙箱类型定义

  **What to do**:
  - TDD: 先写测试验证 trait 可被 mock 实现
  - 创建 `crates/claw-pal/src/traits/mod.rs` 和 `traits/sandbox.rs`
  - 创建 `crates/claw-pal/src/types/sandbox.rs`
  - SandboxBackend trait 严格按照 BUILD_PLAN.md:37-44 定义
  - 定义: SandboxConfig, SandboxHandle, PlatformHandle, SyscallPolicy

  **Must NOT do**:
  - 不要添加任何平台实现
  - trait 方法保持同步（BUILD_PLAN.md 中都是同步方法）

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 3, 5-8)
  - **Blocks**: Tasks 9-11, 14
  - **Blocked By**: Task 3

  **References**:
  - `BUILD_PLAN.md:37-44` — SandboxBackend trait 完整签名 (必须精确复制)
  - `docs/architecture/pal.md` — 沙箱架构深度说明
  - `docs/adr/003-security-model.md` — 安全模型决策

  **Acceptance Criteria**:
  - [ ] `SandboxBackend` trait 与 BUILD_PLAN.md:37-44 完全一致
  - [ ] mock 测试可编译
  - [ ] `cargo test -p claw-pal` → PASS

  **QA Scenarios:**
  ```
  Scenario: SandboxBackend trait mock
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- sandbox`
    Expected Result: mock struct 实现 SandboxBackend，所有方法可调用
    Evidence: .sisyphus/evidence/task-4-sandbox-trait.txt
  ```

  **Commit**: NO (Wave 1 合并提交)

---

- [ ] 5. IpcTransport trait + IPC 类型定义

  **What to do**:
  - TDD: 先写测试验证 async trait mock
  - 创建 `crates/claw-pal/src/traits/ipc.rs`
  - 创建 `crates/claw-pal/src/types/ipc.rs` — IpcConnection, IpcListener, IpcMessage, IpcError
  - IpcTransport trait 严格按照 BUILD_PLAN.md:47-52 定义（async 方法，使用 async_trait）
  - IpcError 枚举: ConnectionRefused, Timeout, BrokenPipe, InvalidMessage, PermissionDenied

  **Must NOT do**:
  - 不要添加平台实现
  - 不要直接引用 interprocess crate

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 3-4, 6-8)
  - **Blocks**: Tasks 12, 14
  - **Blocked By**: Task 3

  **References**:
  - `BUILD_PLAN.md:47-52` — IpcTransport trait 完整签名
  - `docs/adr/005-ipc-multi-agent.md` — IPC 和 A2A 协议设计决策

  **Acceptance Criteria**:
  - [ ] `IpcTransport` trait 与 BUILD_PLAN.md:47-52 完全一致
  - [ ] async mock 测试可编译
  - [ ] `cargo test -p claw-pal` → PASS

  **QA Scenarios:**
  ```
  Scenario: IpcTransport async mock
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- ipc`
    Expected Result: 异步 mock 的 connect/listen/send/recv 可 .await
    Evidence: .sisyphus/evidence/task-5-ipc-trait.txt
  ```

  **Commit**: NO (Wave 1 合并提交)

---

- [ ] 6. ProcessManager trait + 进程类型定义

  **What to do**:
  - TDD: 先写测试验证 async trait mock
  - 创建 `crates/claw-pal/src/traits/process.rs`
  - 创建 `crates/claw-pal/src/types/process.rs`
  - ProcessManager trait 严格按照 BUILD_PLAN.md:55-62 定义
  - ProcessSignal 枚举: Term, Kill, Interrupt (跨平台抽象，不用 Unix 信号号)

  **Must NOT do**:
  - 不要添加平台实现

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 3-5, 7-8)
  - **Blocks**: Tasks 13, 14
  - **Blocked By**: Task 3

  **References**:
  - `BUILD_PLAN.md:55-62` — ProcessManager trait 完整签名
  - `BUILD_PLAN.md:57-58` — terminate 方法的 grace_period 参数

  **Acceptance Criteria**:
  - [ ] `ProcessManager` trait 与 BUILD_PLAN.md:55-62 完全一致
  - [ ] async mock 测试可编译
  - [ ] `cargo test -p claw-pal` → PASS

  **QA Scenarios:**
  ```
  Scenario: ProcessManager async mock
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- process`
    Expected Result: spawn/terminate/kill/wait/signal 可 .await
    Evidence: .sisyphus/evidence/task-6-process-trait.txt
  ```

  **Commit**: NO (Wave 1 合并提交)

---

- [ ] 7. ExecutionMode + PowerKey (Layer 0 安全模型)

  **What to do**:
  - TDD: 先写安全相关测试 — 弱密码拒绝、模式切换规则
  - 创建 `crates/claw-pal/src/security.rs`
  - 定义 `ExecutionMode` 枚举: Safe, Power
  - 实现 PowerKey 验证: 长度 ≥ 12、字符类型 ≥ 2（大写、小写、数字、特殊）
  - 实现 PowerKey 哈希: 使用 `argon2` crate（需添加到 claw-pal 依赖）
  - 定义 ModeTransition 规则: Safe→Power 需 key, Power→Safe 需重启
  - 定义 `SecurityError`: InvalidPowerKey, KeyTooShort, InsufficientComplexity, ModeTransitionDenied

  **Must NOT do**:
  - 不允许 Power→Safe 运行时切换
  - 不明文存储 Power Key
  - 不接受 < 12 字符的 key

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 安全关键代码，需仔细处理密码学
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 3-6, 8)
  - **Blocks**: Tasks 9-11, 14
  - **Blocked By**: Task 3

  **References**:
  - `docs/adr/003-security-model.md` — 安全模型完整设计 (MUST READ)
  - `docs/guides/power-mode.md` — Power Mode 激活方式
  - `TECHNICAL_SPECIFICATION.md` — Power Key 要求

  **Acceptance Criteria**:
  - [ ] `ExecutionMode::Safe` 为默认
  - [ ] Key < 12 字符 → `SecurityError::KeyTooShort`
  - [ ] Key 仅 1 种字符类型 → `SecurityError::InsufficientComplexity`
  - [ ] Power→Safe → `SecurityError::ModeTransitionDenied`
  - [ ] `cargo test -p claw-pal -- security` → PASS

  **QA Scenarios:**
  ```
  Scenario: Power Key 验证
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- security`
      2. 验证: "SecureKey123!" → OK, "Short1!" → KeyTooShort, "aaaaaaaaaaaa" → InsufficientComplexity
    Expected Result: 所有验证规则正确
    Evidence: .sisyphus/evidence/task-7-security.txt
  ```

  **Commit**: NO (Wave 1 合并提交)

---

- [ ] 8. dirs 模块（跨平台标准目录）

  **What to do**:
  - TDD: 先写测试验证各目录函数返回非空路径
  - 创建 `crates/claw-pal/src/dirs.rs`
  - 实现: `config_dir()`, `data_dir()`, `cache_dir()`, `tools_dir()`, `scripts_dir()`, `logs_dir()`, `agents_dir()`
  - 使用 `dirs` crate，在标准目录下创建 `claw-kernel/` 子目录
  - agents_dir: `~/.local/share/claw-kernel/agents/` (ADR-005)
  - logs_dir: `~/.local/share/claw-kernel/logs/` (审计日志)

  **Must NOT do**:
  - 不硬编码路径
  - 不自动创建目录（仅返回路径）

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 3-7)
  - **Blocks**: Task 14
  - **Blocked By**: Task 3

  **References**:
  - `docs/adr/005-ipc-multi-agent.md` — agents_dir 路径
  - `docs/architecture/pal.md` — dirs 模块 API

  **Acceptance Criteria**:
  - [ ] 所有函数返回 `PathBuf`，包含 `claw-kernel` 子目录
  - [ ] `cargo test -p claw-pal -- dirs` → PASS

  **QA Scenarios:**
  ```
  Scenario: 目录路径验证
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- dirs`
    Expected Result: 所有路径有效且包含 claw-kernel
    Evidence: .sisyphus/evidence/task-8-dirs.txt
  ```

  **Commit**: YES (Wave 1 commit)
  - Message: `feat(pal): define all trait interfaces and core types`
  - Pre-commit: `cargo test -p claw-pal`

---

- [ ] 9. Linux sandbox 实现 (seccomp-bpf + namespaces)

  **What to do**:
  - TDD: 先写测试（seccomp 规则应用、文件系统限制、网络限制）
  - 创建 `crates/claw-pal/src/linux/mod.rs` 和 `linux/sandbox.rs`
  - 实现 `LinuxSandbox: SandboxBackend`
  - 使用 `libseccomp` 0.3.0 创建 seccomp filter
  - **关键**: 使用 `SCMP_ACT_ERRNO(EPERM)` 而非 `SCMP_ACT_KILL`（Metis 发现: KILL + thread join = Rust panic）
  - 使用 `nix` crate 设置 namespace 和 chroot
  - 文件系统限制: bind mount allowlisted 路径
  - 网络限制: seccomp filter 阻止 socket 系统调用
  - 资源限制: setrlimit
  - 全部代码包在 `#[cfg(target_os = "linux")]` 中

  **Must NOT do**:
  - 不使用 `SCMP_ACT_KILL` 处理线程级违规
  - 不在 `claw-pal` 之外放置平台代码

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 低级系统 API，安全关键
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 10-13)
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 4, 7

  **References**:
  - `docs/architecture/pal.md` — Linux sandbox 完整架构
  - `docs/platform/linux.md` — Linux 平台特定说明
  - `docs/adr/003-security-model.md` — Safe/Power 模式下沙箱行为
  - Metis 发现: seccomp `SCMP_ACT_KILL` + `join()` = Rust panic (rust-lang/rust#112521)

  **Acceptance Criteria**:
  - [ ] `LinuxSandbox` 实现 `SandboxBackend` trait 所有方法
  - [ ] seccomp filter 使用 `SCMP_ACT_ERRNO(EPERM)`
  - [ ] `#[cfg(target_os = "linux")] cargo test -p claw-pal -- linux` → PASS

  **QA Scenarios:**
  ```
  Scenario: Linux seccomp sandbox 创建
    Tool: Bash
    Preconditions: Linux 环境（CI 或本地 Linux）
    Steps:
      1. `cargo test -p claw-pal -- linux::sandbox` (仅 Linux)
      2. 验证 seccomp filter 使用 ERRNO 而非 KILL
    Expected Result: sandbox 可创建和应用
    Evidence: .sisyphus/evidence/task-9-linux-sandbox.txt
  ```

  **Commit**: NO (Wave 2 合并提交)

---

- [ ] 10. macOS sandbox 实现 (sandbox(7) FFI)

  **What to do**:
  - TDD: 先写测试（sandbox profile 生成、应用）
  - 创建 `crates/claw-pal/src/macos/mod.rs` 和 `macos/sandbox.rs`
  - 实现 `MacOSSandbox: SandboxBackend`
  - 使用 unsafe FFI 调用 `sandbox_init()` C API
  - **关键**: sandbox 必须作为进程中第一个操作应用（Metis: 初始化窗口安全风险）
  - 生成 sandbox profile 字符串（基于 SandboxConfig）
  - profile 格式: Scheme-like S-expression (Apple sandbox profile language)
  - 全部代码包在 `#[cfg(target_os = "macos")]` 中

  **Must NOT do**:
  - 不使用已废弃的 `sandbox-exec` CLI
  - 不在 sandbox_init() 前执行任何用户代码

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: unsafe FFI，安全关键
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 9, 11-13)
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 4, 7

  **References**:
  - `docs/architecture/pal.md` — macOS sandbox 架构
  - `docs/platform/macos.md` — macOS 平台特定说明
  - Metis 确认: `sandbox_init()` C API 未废弃（Chrome/Firefox/Safari 都在用）

  **Acceptance Criteria**:
  - [ ] `MacOSSandbox` 实现 `SandboxBackend` trait
  - [ ] unsafe FFI 调用正确且有安全注释
  - [ ] `cargo test -p claw-pal -- macos::sandbox` → PASS (macOS)

  **QA Scenarios:**
  ```
  Scenario: macOS sandbox 创建
    Tool: Bash
    Preconditions: macOS 环境
    Steps:
      1. `cargo test -p claw-pal -- macos::sandbox`
    Expected Result: sandbox profile 可生成和应用
    Evidence: .sisyphus/evidence/task-10-macos-sandbox.txt
  ```

  **Commit**: NO (Wave 2 合并提交)

---

- [ ] 11. Windows sandbox 骨架 (AppContainer stub)

  **What to do**:
  - 创建 `crates/claw-pal/src/windows/mod.rs` 和 `windows/sandbox.rs`
  - 实现 `WindowsSandbox: SandboxBackend`（骨架实现）
  - 所有方法返回 `Ok(...)` 或 `Err(SandboxError::NotImplemented)`
  - 添加 TODO 注释标记需要 AppContainer + Job Objects 实现的位置
  - 全部代码包在 `#[cfg(target_os = "windows")]` 中

  **Must NOT do**:
  - 不需要完整实现 AppContainer（Metis: DACL 问题复杂，先骨架）

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 骨架/占位实现
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 9-10, 12-13)
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 4, 7

  **References**:
  - `docs/platform/windows.md` — Windows 平台说明
  - Metis 发现: AppContainer + Named Pipe DACL 问题需后续处理

  **Acceptance Criteria**:
  - [ ] `WindowsSandbox` 编译通过（在 Windows target 下）
  - [ ] 在非 Windows 平台 `cargo check --workspace` 不受影响

  **QA Scenarios:**
  ```
  Scenario: Windows sandbox 骨架编译
    Tool: Bash
    Steps:
      1. `cargo check --workspace` (当前平台)
      2. 验证 Windows 模块不影响其他平台编译
    Expected Result: 编译通过
    Evidence: .sisyphus/evidence/task-11-windows-stub.txt
  ```

  **Commit**: NO (Wave 2 合并提交)

---

- [ ] 12. IPC 传输实现 — 全平台 (interprocess)

  **What to do**:
  - TDD: 先写测试（连接/监听/发送/接收）
  - 创建平台特定 IPC 实现文件（linux/ipc.rs, macos/ipc.rs, windows/ipc.rs）或通用实现
  - 实现 `InterprocessTransport: IpcTransport`
  - 使用 `interprocess` crate v1.2.1 with tokio feature
  - **关键**: 使用单读线程 + channel 派发模式（Metis: interprocess 并发 I/O 在 macOS panic）
  - **关键**: 在 Tokio runtime shutdown 前显式 drop 所有 IPC handles
  - Linux/macOS: Unix Domain Socket
  - Windows: Named Pipe
  - 消息帧格式: length-prefixed (4 bytes big-endian length + payload)

  **Must NOT do**:
  - 不 split socket 做并发双向 I/O
  - 不在 Tokio shutdown 时泄露 IPC handles

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 异步 I/O + 跨平台 + 并发安全
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 9-11, 13)
  - **Blocks**: Tasks 14, 16
  - **Blocked By**: Task 5

  **References**:
  - `docs/adr/005-ipc-multi-agent.md` — IPC 设计决策
  - `docs/architecture/pal.md` — IPC 层架构
  - Metis 发现: interprocess 并发 I/O panic (kotauskas/interprocess#89)
  - Metis 发现: drop SendHalf during shutdown panic (interprocess#71)

  **Acceptance Criteria**:
  - [ ] 可在本地建立 IPC 连接并收发消息
  - [ ] 使用单读线程模式（非 split socket）
  - [ ] `cargo test -p claw-pal -- ipc` → PASS

  **QA Scenarios:**
  ```
  Scenario: IPC 本地通信
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- ipc::tests::roundtrip`
      2. 验证: listen → connect → send → recv 完整流程
    Expected Result: 消息正确传递
    Evidence: .sisyphus/evidence/task-12-ipc.txt
  ```

  **Commit**: NO (Wave 2 合并提交)

---

- [ ] 13. ProcessManager 实现 — 全平台

  **What to do**:
  - TDD: 先写测试（进程 spawn/wait/terminate）
  - 创建平台特定实现或通用 `tokio::process` 包装
  - 实现 `TokioProcessManager: ProcessManager`
  - 使用 `tokio::process::Command` 作为基础
  - terminate: 发送 SIGTERM → 等待 grace_period → 发送 SIGKILL
  - Windows: 使用 `TerminateProcess` API
  - 跟踪子进程列表: `DashMap<ProcessHandle, Child>`

  **Must NOT do**:
  - 不直接使用 std::process（使用 tokio::process）

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 异步进程管理 + 跨平台
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 9-12)
  - **Blocks**: Tasks 14, 17
  - **Blocked By**: Task 6

  **References**:
  - `BUILD_PLAN.md:55-62` — ProcessManager trait
  - `docs/architecture/pal.md` — 进程管理架构

  **Acceptance Criteria**:
  - [ ] 可 spawn 子进程并等待完成
  - [ ] terminate 实现 graceful shutdown (SIGTERM → wait → SIGKILL)
  - [ ] `cargo test -p claw-pal -- process` → PASS

  **QA Scenarios:**
  ```
  Scenario: 进程 spawn 和 wait
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal -- process::tests::spawn_and_wait`
      2. 验证: spawn echo → wait → ExitStatus success
    Expected Result: 进程正常启动和退出
    Evidence: .sisyphus/evidence/task-13-process.txt
  ```

  **Commit**: NO (Wave 2 合并提交)

---

- [ ] 14. claw-pal lib.rs 整合 + 集成测试

  **What to do**:
  - 整合所有模块到 `crates/claw-pal/src/lib.rs`
  - 导出所有公共类型和 trait
  - 使用条件编译导出平台特定实现
  - 创建 `crates/claw-pal/tests/` 目录，写集成测试
  - 集成测试: sandbox + process + IPC 联合场景
  - 确保 `cargo test -p claw-pal` 全部通过
  - 确保 `cargo doc -p claw-pal` 生成成功

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: 跨模块整合，需要全局视角
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (last task)
  - **Blocks**: Tasks 15-17
  - **Blocked By**: Tasks 9-13

  **References**:
  - `docs/crates/claw-pal.md` — 公开 API 列表
  - 所有 Wave 1-2 task 的输出

  **Acceptance Criteria**:
  - [ ] `cargo test -p claw-pal` → 全部 PASS
  - [ ] `cargo doc -p claw-pal --no-deps` → 成功
  - [ ] 所有公共 trait 和类型从 `claw_pal::` 可访问

  **QA Scenarios:**
  ```
  Scenario: claw-pal 完整性验证
    Tool: Bash
    Steps:
      1. `cargo test -p claw-pal`
      2. `cargo doc -p claw-pal --no-deps`
    Expected Result: 所有测试通过，文档生成成功
    Evidence: .sisyphus/evidence/task-14-pal-integration.txt
  ```

  **Commit**: YES (Wave 2 commit)
  - Message: `feat(pal): implement platform-specific sandbox, IPC, and process management`
  - Pre-commit: `cargo test -p claw-pal`

---

- [ ] 15. EventBus + FilteredReceiver + Event 枚举

  **What to do**:
  - TDD: 先写测试（emit/subscribe、过滤、多订阅者）
  - 创建 `crates/claw-runtime/src/event_bus.rs`
  - 实现 `EventBus`: `tokio::sync::broadcast` 容量 1024 (ADR-007)
  - 实现 `FilteredReceiver`: 包装 broadcast::Receiver + 过滤逻辑
  - 定义 `Event` 枚举: UserInput, AgentOutput, ToolCall, ToolResult, AgentLifecycle, Extension, A2A
  - 定义 `EventFilter` 和 `EventType`
  - Event 必须实现 `Clone`（broadcast 要求）

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 16-20)
  - **Blocks**: Tasks 16, 17, 29
  - **Blocked By**: Task 14

  **References**:
  - `BUILD_PLAN.md:89-132` — EventBus 和 Event 完整定义
  - `docs/adr/007-eventbus-implementation.md` — EventBus 实现决策
  - `docs/crates/claw-runtime.md` — claw-runtime 文档

  **Acceptance Criteria**:
  - [ ] EventBus 使用 broadcast 容量 1024
  - [ ] FilteredReceiver 可按事件类型过滤
  - [ ] `cargo test -p claw-runtime -- event_bus` → PASS

  **QA Scenarios:**
  ```
  Scenario: EventBus emit/subscribe
    Tool: Bash
    Steps:
      1. `cargo test -p claw-runtime -- event_bus`
    Expected Result: 发送事件后所有订阅者收到，过滤正确
    Evidence: .sisyphus/evidence/task-15-eventbus.txt
  ```

  **Commit**: NO (Wave 3 合并提交)

---

- [ ] 16. IpcRouter 实现

  **What to do**:
  - TDD: 先写测试（跨进程事件桥接）
  - 创建 `crates/claw-runtime/src/ipc_router.rs`
  - IpcRouter 持有 `Arc<EventBus>` + `Arc<dyn IpcTransport>` (ADR-007)
  - 实现 `on_incoming`: 将 IPC 消息转为 Event 并 emit
  - 实现 `run_outbound`: 将特定 Event 通过 IPC 发送给远程
  - EventBus 不了解 IPC，IpcRouter 是桥接层

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15, 17-20)
  - **Blocks**: Tasks 17, 29
  - **Blocked By**: Tasks 12, 15

  **References**:
  - `docs/adr/007-eventbus-implementation.md` — IpcRouter 与 EventBus 的关系
  - `docs/crates/claw-runtime.md` — IpcRouter 架构

  **Acceptance Criteria**:
  - [ ] IpcRouter 可将 IPC 消息桥接到 EventBus
  - [ ] `cargo test -p claw-runtime -- ipc_router` → PASS

  **QA Scenarios:**
  ```
  Scenario: IPC 事件桥接
    Tool: Bash
    Steps:
      1. `cargo test -p claw-runtime -- ipc_router`
    Expected Result: IPC 消息正确转为 Event
    Evidence: .sisyphus/evidence/task-16-ipc-router.txt
  ```

  **Commit**: NO (Wave 3 合并提交)

---

- [ ] 17. Runtime 结构体 + AgentOrchestrator

  **What to do**:
  - TDD: 先写测试（spawn/kill/list/send_message）
  - 创建 `crates/claw-runtime/src/runtime.rs` 和 `orchestrator.rs`
  - Runtime 包含: event_bus, process_manager, ipc_router
  - AgentOrchestrator: spawn, kill, list, send_message (BUILD_PLAN.md:108-117)
  - 定义 A2A 消息类型: A2AMessage, A2AMessageType, Payload, MessagePriority
  - 定义 Agent 类型: AgentId, AgentHandle, AgentInfo, AgentStatus, AgentConfig
  - Agent 管理: `DashMap<AgentId, AgentHandle>`

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15-16, 18-20)
  - **Blocks**: Task 29
  - **Blocked By**: Tasks 13, 15, 16

  **References**:
  - `BUILD_PLAN.md:91-132` — Runtime, AgentOrchestrator, Event 完整定义
  - `docs/adr/005-ipc-multi-agent.md` — A2A 协议设计
  - `docs/crates/claw-runtime.md` — Runtime crate 文档

  **Acceptance Criteria**:
  - [ ] Runtime 可正确初始化
  - [ ] AgentOrchestrator spawn/kill/list 工作正常
  - [ ] `cargo test -p claw-runtime` → PASS

  **QA Scenarios:**
  ```
  Scenario: AgentOrchestrator 基本操作
    Tool: Bash
    Steps:
      1. `cargo test -p claw-runtime -- orchestrator`
    Expected Result: spawn/kill/list/send_message 均正常
    Evidence: .sisyphus/evidence/task-17-orchestrator.txt
  ```

  **Commit**: YES (Wave 3 commit)
  - Message: `feat(runtime): implement EventBus, IpcRouter, and AgentOrchestrator`
  - Pre-commit: `cargo test -p claw-runtime`

---

- [ ] 18. claw-provider traits (MessageFormat + HttpTransport + LLMProvider)

  **What to do**:
  - 创建 claw-provider 三层 trait 架构
  - `MessageFormat` trait: build_request, parse_response, parse_stream_chunk, token_count, endpoint (BUILD_PLAN.md:149-162)
  - `HttpTransport` trait: base_url, auth_headers, http_client, request, stream_request (BUILD_PLAN.md:164-178)
  - `LLMProvider` trait: complete, stream_complete, token_count (BUILD_PLAN.md:180-187)
  - `EmbeddingProvider` trait: embed (BUILD_PLAN.md:190-193)
  - 定义核心类型: Message, Role, CompletionResponse, Delta, Options, ProviderError, TokenUsage
  - mock 测试验证 trait 可实现

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15-17, 19-20)
  - **Blocks**: Tasks 21-24
  - **Blocked By**: Task 2

  **References**:
  - `BUILD_PLAN.md:148-194` — 三层 provider trait 完整签名
  - `docs/crates/claw-provider.md` — claw-provider 文档
  - `docs/adr/006-provider-architecture.md` — Provider 架构决策（如存在）

  **Acceptance Criteria**:
  - [ ] 三层 trait 与 BUILD_PLAN.md 完全一致
  - [ ] mock 实现可编译
  - [ ] `cargo test -p claw-provider` → PASS

  **QA Scenarios:**
  ```
  Scenario: Provider trait mock
    Tool: Bash
    Steps:
      1. `cargo test -p claw-provider -- traits`
    Expected Result: 三层 trait mock 实现可编译和调用
    Evidence: .sisyphus/evidence/task-18-provider-traits.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 19. claw-tools traits (Tool + ToolRegistry + PermissionSet)

  **What to do**:
  - 创建 Tool trait: name, description, version, schema, execute, permissions, timeout (BUILD_PLAN.md:211-221)
  - 创建 ToolRegistry API: new, register, unregister, get, list, execute (BUILD_PLAN.md:255-265)
  - 定义: ToolResult, ToolError, ToolErrorCode, ToolSchema, ToolMeta
  - 定义: PermissionSet, FsPermissions, NetworkPermissions, SubprocessPolicy
  - 定义: HotLoadingConfig, LoadError, WatchError, RegistryError
  - mock 测试验证

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15-18, 20)
  - **Blocks**: Tasks 25-26
  - **Blocked By**: Task 2

  **References**:
  - `BUILD_PLAN.md:207-266` — Tool trait 和 ToolRegistry 完整定义
  - `docs/crates/claw-tools.md` — claw-tools 文档

  **Acceptance Criteria**:
  - [ ] Tool trait 与 BUILD_PLAN.md 完全一致
  - [ ] ToolRegistry API 完整
  - [ ] `cargo test -p claw-tools` → PASS

  **QA Scenarios:**
  ```
  Scenario: Tool trait 和 ToolRegistry mock
    Tool: Bash
    Steps:
      1. `cargo test -p claw-tools`
    Expected Result: mock tool 可注册、查找、执行
    Evidence: .sisyphus/evidence/task-19-tools-traits.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 20. claw-loop traits (AgentLoop + StopCondition + HistoryManager)

  **What to do**:
  - 定义 AgentLoop struct: provider, tools, history, stop_conditions, config (BUILD_PLAN.md:288-295)
  - 定义 AgentLoopConfig: max_turns, token_budget, system_prompt, enable_streaming, tool_timeout (BUILD_PLAN.md:297-304)
  - 定义 AgentResult: content, tool_calls, turns, token_usage, finish_reason, execution_time
  - 定义 FinishReason: Completed, MaxTurnsReached, TokenBudgetExceeded, StopConditionMet, UserInterrupted, Error
  - StopCondition trait: should_stop(state: &LoopState) (BUILD_PLAN.md:328-330)
  - HistoryManager trait: append, get_context, truncate_to_fit, summarize (BUILD_PLAN.md:342-348)
  - LoopState: turn_count, token_usage, last_message, tool_calls_made

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15-19)
  - **Blocks**: Tasks 27-28
  - **Blocked By**: Task 2

  **References**:
  - `BUILD_PLAN.md:281-358` — AgentLoop, StopCondition, HistoryManager 完整定义
  - `docs/crates/claw-loop.md` — claw-loop 文档
  - `docs/design/agent-loop-state-machine.md` — Agent 循环状态机

  **Acceptance Criteria**:
  - [ ] 所有类型和 trait 与 BUILD_PLAN.md 一致
  - [ ] `cargo test -p claw-loop` → PASS

  **QA Scenarios:**
  ```
  Scenario: AgentLoop types 和 traits
    Tool: Bash
    Steps:
      1. `cargo test -p claw-loop`
    Expected Result: 所有类型可构造，trait 可 mock
    Evidence: .sisyphus/evidence/task-20-loop-traits.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 21. OpenAIFormat 实现

  **What to do**:
  - TDD: 先写测试（请求构建、响应解析、流式 chunk 解析）
  - 实现 `OpenAIFormat: MessageFormat`
  - 构建 OpenAI Chat Completions API 请求格式
  - 解析响应和 SSE 流式 chunk
  - 处理 tool_calls 和 function_calling 格式
  - token_count: 估算方法（简化版，按字符数/4）
  - endpoint: `/v1/chat/completions`

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 22-28)
  - **Blocks**: Task 24
  - **Blocked By**: Task 18

  **References**:
  - `BUILD_PLAN.md:149-162` — MessageFormat trait 签名
  - `docs/crates/claw-provider.md` — OpenAI format 说明
  - OpenAI API docs: https://platform.openai.com/docs/api-reference/chat/create

  **Acceptance Criteria**:
  - [ ] 请求构建符合 OpenAI API 格式
  - [ ] 响应解析正确
  - [ ] `cargo test -p claw-provider -- openai` → PASS

  **QA Scenarios:**
  ```
  Scenario: OpenAI 请求/响应往返
    Tool: Bash
    Steps:
      1. `cargo test -p claw-provider -- openai`
    Expected Result: 请求结构正确，响应解析正确
    Evidence: .sisyphus/evidence/task-21-openai.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 22. AnthropicFormat 实现

  **What to do**:
  - TDD: 同 OpenAI 模式
  - 实现 `AnthropicFormat: MessageFormat`
  - Anthropic Messages API 格式（system 在 top-level，非 message 内）
  - 解析 Anthropic 特有的 content_block_delta 流式格式
  - tool_use / tool_result content block 处理
  - endpoint: `/v1/messages`

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21, 23-28)
  - **Blocks**: Task 24
  - **Blocked By**: Task 18

  **References**:
  - `BUILD_PLAN.md:149-162` — MessageFormat trait
  - Anthropic API docs: https://docs.anthropic.com/en/api/messages

  **Acceptance Criteria**:
  - [ ] Anthropic 请求格式正确（system 在 top-level）
  - [ ] 流式 content_block_delta 解析正确
  - [ ] `cargo test -p claw-provider -- anthropic` → PASS

  **QA Scenarios:**
  ```
  Scenario: Anthropic 请求/响应
    Tool: Bash
    Steps:
      1. `cargo test -p claw-provider -- anthropic`
    Expected Result: Anthropic 特有格式处理正确
    Evidence: .sisyphus/evidence/task-22-anthropic.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 23. OllamaFormat 实现

  **What to do**:
  - TDD: 同上
  - 实现 `OllamaFormat: MessageFormat`
  - Ollama 使用 OpenAI 兼容 API，但有差异
  - 默认 endpoint: `http://localhost:11434`
  - endpoint path: `/api/chat`
  - 不需要 auth headers

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-22, 24-28)
  - **Blocks**: Task 24
  - **Blocked By**: Task 18

  **References**:
  - Ollama API: https://github.com/ollama/ollama/blob/main/docs/api.md

  **Acceptance Criteria**:
  - [ ] Ollama 格式请求正确
  - [ ] `cargo test -p claw-provider -- ollama` → PASS

  **QA Scenarios:**
  ```
  Scenario: Ollama 格式
    Tool: Bash
    Steps:
      1. `cargo test -p claw-provider -- ollama`
    Expected Result: Ollama 请求/响应格式正确
    Evidence: .sisyphus/evidence/task-23-ollama.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 24. HttpTransport 默认实现 + Provider 组装

  **What to do**:
  - TDD: 使用 wiremock 模拟 HTTP 服务器
  - 实现 `DefaultHttpTransport: HttpTransport`（使用 reqwest）
  - 组装 3 个 Provider: `AnthropicProvider`, `OpenAIProvider`, `OllamaProvider`
  - 每个 Provider = HttpTransport + MessageFormat
  - 实现从环境变量初始化: `from_env()`
  - 流式请求: SSE parser for streaming responses

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-23, 25-28)
  - **Blocks**: Task 27
  - **Blocked By**: Tasks 21-23

  **References**:
  - `BUILD_PLAN.md:164-178` — HttpTransport trait
  - `docs/crates/claw-provider.md` — Provider 组装方式

  **Acceptance Criteria**:
  - [ ] 3 个 Provider 均可通过 `from_env()` 创建
  - [ ] wiremock 测试验证 HTTP 请求/响应
  - [ ] `cargo test -p claw-provider` → PASS

  **QA Scenarios:**
  ```
  Scenario: Provider HTTP 轮转测试
    Tool: Bash
    Steps:
      1. `cargo test -p claw-provider -- transport`
    Expected Result: wiremock 模拟服务器请求/响应正确
    Evidence: .sisyphus/evidence/task-24-transport.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 25. ToolRegistry 核心实现

  **What to do**:
  - TDD: 先写测试（注册、查找、执行、卸载）
  - 实现 ToolRegistry 所有方法
  - 内部使用 `HashMap<String, Box<dyn Tool>>` 存储
  - execute: 权限检查 → 超时控制 → 调用 tool.execute()
  - schema 生成: 使用 `schemars` 自动生成 JSON Schema
  - 支持多版本 tool（同名不同版本）

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-24, 26-28)
  - **Blocks**: Tasks 26, 27
  - **Blocked By**: Task 19

  **References**:
  - `BUILD_PLAN.md:250-265` — ToolRegistry API
  - `docs/crates/claw-tools.md` — ToolRegistry 架构
  - `docs/adr/008-hot-loading-mechanism.md` — 热加载机制

  **Acceptance Criteria**:
  - [ ] register/unregister/get/list/execute 均工作正常
  - [ ] 权限检查正确
  - [ ] `cargo test -p claw-tools -- registry` → PASS

  **QA Scenarios:**
  ```
  Scenario: ToolRegistry CRUD
    Tool: Bash
    Steps:
      1. `cargo test -p claw-tools -- registry`
    Expected Result: tool 注册/查找/执行/卸载全流程
    Evidence: .sisyphus/evidence/task-25-registry.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 26. 热加载机制 (notify + 文件监视)

  **What to do**:
  - TDD: 先写测试（文件变更触发重加载）
  - 实现 `HotLoader` 结构体
  - 使用 `notify` 6.1.1 with tokio feature 监视文件变更
  - debounce: 50ms（ADR-008）
  - 文件变更 → 重新加载 tool → 原子替换到 ToolRegistry
  - 支持监视目录和单文件
  - 错误处理: 加载失败不崩溃，保留旧版本

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-25, 27-28)
  - **Blocks**: Task 32
  - **Blocked By**: Task 25

  **References**:
  - `docs/adr/008-hot-loading-mechanism.md` — 热加载设计决策
  - `docs/crates/claw-tools.md` — HotLoadingConfig 定义

  **Acceptance Criteria**:
  - [ ] 文件变更后 50ms 内触发重加载
  - [ ] 加载失败保留旧版本
  - [ ] `cargo test -p claw-tools -- hot_load` → PASS

  **QA Scenarios:**
  ```
  Scenario: 热加载触发
    Tool: Bash
    Steps:
      1. `cargo test -p claw-tools -- hot_load`
    Expected Result: 文件变更触发重加载，debounce 50ms
    Evidence: .sisyphus/evidence/task-26-hotload.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 27. AgentLoop + AgentLoopBuilder 实现

  **What to do**:
  - TDD: 使用 mock provider 和 tools 测试循环逻辑
  - 实现 AgentLoopBuilder（builder 模式）
  - 实现 AgentLoop::run() 核心循环:
    1. 收集上下文 (history)
    2. 调用 LLM provider
    3. 检查是否有 tool_calls
    4. 执行 tools
    5. 检查 stop conditions
    6. 循环或返回
  - 跨 crate 依赖: claw-provider (LLMProvider), claw-tools (ToolRegistry)
  - 参考 agent-loop-state-machine.md 状态机设计

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-26, 28)
  - **Blocks**: Task 30
  - **Blocked By**: Tasks 20, 24, 25

  **References**:
  - `BUILD_PLAN.md:281-358` — AgentLoop 完整定义
  - `docs/design/agent-loop-state-machine.md` — 状态机设计
  - `docs/crates/claw-loop.md` — claw-loop 文档

  **Acceptance Criteria**:
  - [ ] AgentLoopBuilder 可构建 AgentLoop
  - [ ] run() 可与 mock provider/tools 完成多轮对话
  - [ ] stop conditions 正确触发
  - [ ] `cargo test -p claw-loop` → PASS

  **QA Scenarios:**
  ```
  Scenario: AgentLoop 多轮对话
    Tool: Bash
    Steps:
      1. `cargo test -p claw-loop -- agent_loop`
    Expected Result: mock provider 返回 tool_calls → 执行 → 继续 → 完成
    Evidence: .sisyphus/evidence/task-27-agent-loop.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 28. 内置 StopCondition + InMemoryHistory

  **What to do**:
  - 实现内置 stop conditions: MaxTurns, TokenBudget, NoToolCall
  - 实现 `InMemoryHistoryManager: HistoryManager`
  - InMemoryHistory: Vec<Message> 存储，get_context 按 token 限制截断
  - truncate_to_fit: 从开头删除旧消息
  - summarize: 委托给 Summarizer trait（可选）

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-27)
  - **Blocks**: Task 27
  - **Blocked By**: Task 20

  **References**:
  - `BUILD_PLAN.md:325-348` — StopCondition 和 HistoryManager
  - `docs/crates/claw-loop.md` — 内置实现说明

  **Acceptance Criteria**:
  - [ ] MaxTurns 在达到限制时返回 true
  - [ ] InMemoryHistory 正确截断
  - [ ] `cargo test -p claw-loop -- stop_condition` → PASS
  - [ ] `cargo test -p claw-loop -- history` → PASS

  **QA Scenarios:**
  ```
  Scenario: StopCondition 和 History
    Tool: Bash
    Steps:
      1. `cargo test -p claw-loop -- stop_condition`
      2. `cargo test -p claw-loop -- history`
    Expected Result: 停止条件和历史管理均正确
    Evidence: .sisyphus/evidence/task-28-stop-history.txt
  ```

  **Commit**: NO (Wave 4 合并提交)

---

- [ ] 29. claw-runtime 集成测试

  **What to do**:
  - 创建 `crates/claw-runtime/tests/` 目录
  - 写集成测试: EventBus + IpcRouter + AgentOrchestrator 联合
  - 测试场景: Agent spawn → A2A 消息 → EventBus 广播 → IpcRouter 桥接
  - 确保 `cargo test -p claw-runtime` 全部通过

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 21-28)
  - **Blocks**: None
  - **Blocked By**: Tasks 15-17

  **References**:
  - `docs/crates/claw-runtime.md` — 集成场景说明

  **Acceptance Criteria**:
  - [ ] 集成测试覆盖 EventBus + IpcRouter + Orchestrator
  - [ ] `cargo test -p claw-runtime` → 全部 PASS

  **QA Scenarios:**
  ```
  Scenario: claw-runtime 集成测试
    Tool: Bash
    Steps:
      1. `cargo test -p claw-runtime`
    Expected Result: 所有集成测试通过
    Evidence: .sisyphus/evidence/task-29-runtime-integration.txt
  ```

  **Commit**: YES (Wave 4 commit)
  - Message: `feat(provider,tools,loop): implement LLM providers, tool registry, and agent loop`
  - Pre-commit: `cargo test --workspace`

---

- [ ] 30. ScriptEngine trait + Lua 引擎 (mlua)

  **What to do**:
  - TDD: 先写测试（编译、执行、注册原生函数）
  - 创建 `crates/claw-script/src/` 结构
  - 定义 ScriptEngine trait: compile, execute, register_native (BUILD_PLAN.md:369-373)
  - 定义: EngineType, Context, Script, Value, CompileError, ScriptError, NativeFunction
  - 实现 `LuaEngine: ScriptEngine`（使用 mlua 0.9.4）
  - Lua 引擎支持: 异步执行 (mlua async feature)、serde 序列化
  - **关键**: 确保 workspace 不使用 `panic = "abort"` (Metis: mlua + abort = SIGABRT)
  - Feature gate: `#[cfg(feature = "engine-lua")]`

  **Must NOT do**:
  - 不在任何 profile 中设置 `panic = "abort"`

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 31-36)
  - **Blocks**: Task 31
  - **Blocked By**: Task 27

  **References**:
  - `BUILD_PLAN.md:362-390` — ScriptEngine trait 和 EngineType 完整定义
  - `docs/crates/claw-script.md` — claw-script 文档
  - Metis 发现: `panic = "abort"` + mlua = SIGABRT (mlua-rs/mlua#628)

  **Acceptance Criteria**:
  - [ ] ScriptEngine trait 与 BUILD_PLAN.md 一致
  - [ ] Lua 脚本可编译和执行
  - [ ] 原生函数可注册并从 Lua 调用
  - [ ] `cargo test -p claw-script` → PASS

  **QA Scenarios:**
  ```
  Scenario: Lua 脚本执行
    Tool: Bash
    Steps:
      1. `cargo test -p claw-script -- lua`
    Expected Result: Lua 脚本可编译、执行、调用原生函数
    Evidence: .sisyphus/evidence/task-30-lua-engine.txt
  ```

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 31. RustBridge API (脚本 ↔ Rust 桥接)

  **What to do**:
  - TDD: 先写测试（从 Lua 调用 Rust API）
  - 实现 RustBridge，将 Rust 功能暴露给脚本
  - bridge 模块: llm (complete/stream), tools (register/call/list), events (emit/on), fs (read/write/exists), net (get/post)
  - 每个 bridge 模块作为 Lua table 注册
  - fs/net 操作必须尊重 Safe Mode 白名单
  - 参考 BUILD_PLAN.md:393-425 RustBridge API 定义

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 30, 32-36)
  - **Blocks**: Task 32
  - **Blocked By**: Tasks 30, 25

  **References**:
  - `BUILD_PLAN.md:392-425` — RustBridge API 完整接口定义
  - `docs/crates/claw-script.md` — bridge 架构
  - `docs/guides/writing-tools.md` — 从脚本写工具的用户指南

  **Acceptance Criteria**:
  - [ ] Lua 可通过 bridge.tools.list() 获取工具列表
  - [ ] Lua 可通过 bridge.events.emit() 发送事件
  - [ ] fs/net 在 Safe Mode 下受限
  - [ ] `cargo test -p claw-script -- bridge` → PASS

  **QA Scenarios:**
  ```
  Scenario: RustBridge Lua 调用
    Tool: Bash
    Steps:
      1. `cargo test -p claw-script -- bridge`
    Expected Result: Lua 可调用 tools/events/fs bridge
    Evidence: .sisyphus/evidence/task-31-rust-bridge.txt
  ```

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 32. 脚本热加载集成

  **What to do**:
  - 集成 claw-tools 的 HotLoader 与 claw-script 的 ScriptEngine
  - 文件变更 → ScriptEngine 重新编译 → 更新 ToolRegistry
  - 测试: 修改 Lua 文件 → 50ms 后 tool 自动更新
  - 错误处理: 脚本编译失败保留旧版本

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 30-31, 33-36)
  - **Blocks**: Task 36
  - **Blocked By**: Tasks 26, 31

  **References**:
  - `docs/adr/008-hot-loading-mechanism.md` — 热加载设计
  - `docs/guides/extension-capabilities.md` — 扩展能力指南

  **Acceptance Criteria**:
  - [ ] Lua 文件修改后 tool 自动更新
  - [ ] 编译失败保留旧版本
  - [ ] `cargo test -p claw-script -- hot_reload` → PASS

  **QA Scenarios:**
  ```
  Scenario: 脚本热重载
    Tool: Bash
    Steps:
      1. `cargo test -p claw-script -- hot_reload`
    Expected Result: 文件变更触发重编译和更新
    Evidence: .sisyphus/evidence/task-32-script-hotreload.txt
  ```

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 33. claw-memory 最小占位 crate

  **What to do**:
  - 确保 `crates/claw-memory/` 已在 Task 1 中创建
  - 添加基本的 Memory trait 定义（占位）
  - 实现内存 (in-memory) 存储后端
  - 确保 `cargo check -p claw-memory` 通过

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 30-32, 34-36)
  - **Blocks**: Task 35
  - **Blocked By**: Task 2

  **Acceptance Criteria**:
  - [ ] `cargo check -p claw-memory` → 成功
  - [ ] `cargo test -p claw-memory` → PASS

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 34. claw-channel 最小占位 crate

  **What to do**:
  - 确保 `crates/claw-channel/` 已在 Task 1 中创建
  - 添加 Channel trait 定义（BUILD_PLAN.md:446-451）
  - 定义 ChannelMessage 和 ChannelError
  - 不需要实现具体 channel（仅 trait + 类型）
  - 确保 `cargo check -p claw-channel` 通过

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 30-33, 35-36)
  - **Blocks**: Task 35
  - **Blocked By**: Task 2

  **References**:
  - `BUILD_PLAN.md:438-458` — Channel trait 和实现列表
  - `docs/design/channel-message-protocol.md` — ChannelMessage 协议

  **Acceptance Criteria**:
  - [ ] Channel trait 与 BUILD_PLAN.md 一致
  - [ ] `cargo check -p claw-channel` → 成功

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 35. claw-kernel meta-crate

  **What to do**:
  - 更新 `claw-kernel/Cargo.toml` 添加所有 workspace crate 依赖
  - 更新 `claw-kernel/src/lib.rs` 重导出所有 crate (BUILD_PLAN.md:482-493)
  - re-export 常用类型: LLMProvider, Tool, ToolRegistry, AgentLoop, AgentLoopConfig
  - 确保 `cargo check -p claw-kernel` 通过
  - 确保 `cargo doc -p claw-kernel --no-deps` 生成成功

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with Tasks 30-34, 36)
  - **Blocks**: Task 36
  - **Blocked By**: Tasks 33, 34

  **References**:
  - `BUILD_PLAN.md:477-500` — meta-crate 结构

  **Acceptance Criteria**:
  - [ ] `claw_kernel::provider::LLMProvider` 可访问
  - [ ] `claw_kernel::tools::ToolRegistry` 可访问
  - [ ] `cargo doc -p claw-kernel --no-deps` → 成功

  **Commit**: NO (Wave 5 合并提交)

---

- [ ] 36. 全局编译验证 + clippy + fmt

  **What to do**:
  - 运行 `cargo build --workspace`
  - 运行 `cargo test --workspace`
  - 运行 `cargo clippy --workspace -- -D warnings`
  - 运行 `cargo fmt --all -- --check`
  - 运行 `cargo doc --workspace --no-deps`
  - 修复所有警告和格式问题
  - 确保零警告、零错误

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (Wave 5 最后)
  - **Parallel Group**: Wave 5 (last task)
  - **Blocks**: F1-F4
  - **Blocked By**: All previous tasks

  **Acceptance Criteria**:
  - [ ] `cargo build --workspace` → 成功
  - [ ] `cargo test --workspace` → 全部 PASS
  - [ ] `cargo clippy --workspace -- -D warnings` → 零警告
  - [ ] `cargo fmt --all -- --check` → 通过
  - [ ] `cargo doc --workspace --no-deps` → 成功

  **QA Scenarios:**
  ```
  Scenario: 全局质量检查
    Tool: Bash
    Steps:
      1. `cargo build --workspace`
      2. `cargo test --workspace`
      3. `cargo clippy --workspace -- -D warnings`
      4. `cargo fmt --all -- --check`
      5. `cargo doc --workspace --no-deps`
    Expected Result: 全部通过，零警告零错误
    Evidence: .sisyphus/evidence/task-36-global-check.txt
  ```

  **Commit**: YES (Wave 5 commit)
  - Message: `feat(script): implement Lua engine and RustBridge; add meta-crate`
  - Pre-commit: `cargo test --workspace && cargo clippy --workspace -- -D warnings`

---

## Final Verification Wave

> 4 个审查 Agent 并行运行。全部必须 APPROVE。拒绝 → 修复 → 重跑。

- [ ] F1. **计划合规审计** — `oracle`
  通读计划。对每个 "Must Have"：验证实现存在（读文件、运行命令）。对每个 "Must NOT Have"：搜索代码库中的禁止模式 — 如发现则以 file:line 拒绝。检查 evidence 文件是否存在于 `.sisyphus/evidence/`。对比交付物与计划。
  输出: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **代码质量审查** — `unspecified-high`
  运行 `cargo build --workspace` + `cargo clippy --workspace -- -D warnings` + `cargo test --workspace`。审查所有改动文件中的：unsafe 滥用、空 catch、todo!/unimplemented!（非占位）、注释掉的代码、未使用 import。检查 AI 陷阱：过度注释、过度抽象、通用变量名。
  输出: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **实际 QA 验证** — `unspecified-high`
  从干净状态开始。执行每个 task 的每个 QA 场景 — 按步骤操作，截取证据。测试跨 task 集成。测试边界情况：空状态、无效输入。保存到 `.sisyphus/evidence/final-qa/`。
  输出: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **范围保真检查** — `deep`
  对每个 task：读 "What to do"，读实际 diff。验证 1:1 — spec 中的都已构建（无遗漏），spec 外的没有构建（无蔓延）。检查 "Must NOT do" 合规。标记未计划的变更。
  输出: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- 每个 Wave 完成后提交一次
- Wave 0: `chore: scaffold all workspace crates with Cargo.toml and lib.rs`
- Wave 1: `feat(pal): define all trait interfaces and core types`
- Wave 2: `feat(pal): implement platform-specific sandbox, IPC, and process management`
- Wave 3: `feat(runtime): implement EventBus, IpcRouter, and AgentOrchestrator`
- Wave 4: `feat(provider,tools,loop): implement LLM providers, tool registry, and agent loop`
- Wave 5: `feat(script): implement Lua engine and RustBridge; add meta-crate`
- Final: `test: add final verification evidence`

---

## Success Criteria

### 验证命令
```bash
cargo build --workspace                           # Expected: 成功编译，无 error
cargo build --workspace --features engine-lua     # Expected: 成功编译
cargo test --workspace                            # Expected: 所有测试通过
cargo clippy --workspace -- -D warnings           # Expected: 零警告
cargo fmt --all -- --check                        # Expected: 格式正确
cargo doc --workspace --no-deps                   # Expected: 文档生成成功
```

### 最终检查清单
- [ ] 所有 "Must Have" 已实现
- [ ] 所有 "Must NOT Have" 未出现
- [ ] 所有测试通过
- [ ] 每个 crate 至少有 trait 定义 + 一个实现
- [ ] Linux seccomp 沙箱使用 ERRNO 而非 KILL
- [ ] 无 `panic = "abort"` profile
- [ ] IPC 使用单读线程模式
