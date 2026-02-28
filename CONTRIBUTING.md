[English](#english) | [中文](#chinese)

<a name="english"></a>

# Contributing to claw-kernel

Thank you for your interest in contributing! claw-kernel is the shared foundation of the Claw ecosystem, so we hold contributions to a high standard — but the bar for getting started is intentionally low.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Ways to Contribute](#ways-to-contribute)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Submitting Changes](#submitting-changes)
- [Platform-Specific Guidelines](#platform-specific-guidelines)
- [Architecture Decisions](#architecture-decisions)

---

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By participating, you agree to uphold a welcoming, harassment-free environment.

---

## Ways to Contribute

### High-priority areas

- **Windows sandbox hardening** — AppContainer/Job Object coverage is the weakest link
- **New LLM provider implementations** — Gemini, Mistral, local GGUF models
- **Script bridge improvements** — Lua ↔ Rust FFI performance, Deno/V8 embedding stability
- **Platform-specific test coverage** — especially Windows CI edge cases
- **Documentation** — architecture explanations, guide corrections, translation

### Other contributions welcome

- Bug reports with reproduction steps
- Performance benchmarks and regressions
- New channel integrations (Matrix, XMPP, Nostr)
- Example agents demonstrating extensibility patterns

---

## Development Setup

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.83+ (see `rust-toolchain.toml`) | Via [rustup](https://rustup.rs) |
| cargo | bundled with Rust | — |
| Node.js | ≥ 20 (optional) | Only if working on `engine-v8` feature |
| Python | ≥ 3.10 (optional) | Only if working on `engine-py` feature |

#### Platform-specific dependencies

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install libseccomp-dev pkg-config
```

**Linux (Fedora/RHEL):**
```bash
sudo dnf install libseccomp-devel
```

**macOS:**
```bash
# Typically no additional dependencies required
```

**Windows:**
```bash
# Use MSVC toolchain, not GNU
rustup set default-host x86_64-pc-windows-msvc
```

### Clone and build

```bash
git clone https://github.com/claw-project/claw-kernel.git
cd claw-kernel

# Default build (Lua engine only, zero extra dependencies)
cargo build

# With Deno/V8 engine (downloads precompiled V8, may take a few minutes)
cargo build --features engine-v8

# With Python engine
cargo build --features engine-py

# Run all tests across all crates
cargo test --workspace
```

### Running the cross-platform test suite

```bash
# Unit tests only
cargo test --workspace

# Include integration tests
cargo test --workspace --features integration-tests

# Platform-specific sandbox tests (Linux only)
cargo test --workspace --features sandbox-tests
```

---

## Project Structure

> **Note**: The project is currently in the design/planning stage. The `crates/` directory is empty — implementation follows the architecture in `docs/architecture/`.

```
claw-kernel/
├── crates/               # Workspace crates (one per library) - CURRENTLY EMPTY
│   ├── claw-pal/         # Platform Abstraction Layer
│   ├── claw-provider/    # LLM provider trait + implementations
│   ├── claw-tools/       # Tool registry and hot-loading
│   ├── claw-loop/        # Agent loop engine
│   ├── claw-runtime/     # Event bus and process management
│   ├── claw-script/      # Embedded script engines
│   └── claw-kernel/      # Meta-crate (re-exports all above)
├── docs/
│   ├── architecture/     # Layer-by-layer architecture docs
│   ├── adr/              # Architecture Decision Records
│   ├── guides/           # User-facing how-to guides
│   ├── crates/           # Per-crate documentation
│   └── platform/         # Platform-specific notes
├── examples/             # Runnable example agents
└── .github/              # CI workflows and issue templates
```

See [docs/architecture/crate-map.md](docs/architecture/crate-map.md) for the full crate dependency graph.

---

## Submitting Changes

### Key Terms

- **Significant changes**: Changes that add/modify public APIs or traits, modify cross-platform behavior, introduce new dependencies, change the security model, or impact existing architecture decisions. These require an ADR.
- **User-facing change**: Changes that affect the public API, add new features, modify behavior, or change configuration options. Internal refactoring and test updates are excluded. These require CHANGELOG updates.
- **Short description**: Branch names should use lowercase letters and hyphens, maximum 30 characters. Examples: `fix/memory-leak-in-sandbox`, `feat/add-gemini-provider`.
- **Consensus**: Typically indicated by a maintainer's approval comment on the discussion issue.

### For bug fixes

1. Open an issue describing the bug (or comment on an existing one)
2. Fork the repository and create a branch: `fix/<short-description>`
3. Write a failing test that reproduces the bug
4. Fix the bug
5. Confirm tests pass on your local platform
6. Submit a pull request referencing the issue

### For new features

1. **Open an issue or discussion first** — features that touch `claw-pal` or cross-platform behavior need prior agreement
2. For significant changes, write an [ADR](docs/adr/) describing the decision
3. Fork and create a branch: `feat/<short-description>`
4. Implement with tests
5. Update relevant documentation in `docs/`
6. Submit a pull request

### Pull request checklist

- [ ] Tests pass on your local platform (`cargo test --workspace`)
- [ ] No new `clippy` warnings (`cargo clippy --workspace`)
- [ ] Code formatted (`cargo fmt --all`)
- [ ] Documentation updated if behavior changed
- [ ] PR description explains *why*, not just *what*
- [ ] Platform impact noted (Linux only? All platforms?)
- [ ] CHANGELOG.md updated (if user-facing change)
- [ ] ADR added for significant architectural changes (if applicable)

---

## Platform-Specific Guidelines

### Adding a platform-specific implementation

All platform-specific code lives in `claw-pal`. Use Rust's conditional compilation:

```rust
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;
```

Every platform-specific implementation must:
1. Satisfy the shared `Trait` defined in `claw-pal/src/traits.rs`
2. Include at least one integration test in `tests/`
3. Be documented in the corresponding `docs/platform/` file

### Windows contributors

- Use MSVC toolchain, not GNU (`rustup set default-host x86_64-pc-windows-msvc`)
- Avoid Unix-isms: no `fork()`, no POSIX signals, no hardcoded `/` paths
- Test Named Pipe behavior explicitly — it differs subtly from Unix Domain Sockets

---

## Architecture Decisions

Major decisions are recorded as ADRs (Architecture Decision Records) in [`docs/adr/`](docs/adr/). Before proposing a significant architectural change:

1. Read the existing ADRs to understand past decisions
2. Open a discussion issue with the `adr` label
3. If consensus is reached, open a PR adding a new ADR

ADRs use a simple format: **Context → Decision → Consequences**.

---

## Questions?

- Open a [GitHub Discussion](https://github.com/claw-project/claw-kernel/discussions) for design questions
- Open an [Issue](https://github.com/claw-project/claw-kernel/issues) for bugs
- Tag `@claw-project/maintainers` in your PR for faster review

---

<a name="chinese"></a>

# 参与贡献 claw-kernel

感谢您有兴趣参与贡献！claw-kernel 是 Claw 生态系统的共享基础，因此我们对贡献有较高的标准 —— 但开始参与的门槛被有意设置得很低。

---

## 目录

- [行为准则](#chinese-code-of-conduct)
- [贡献方式](#chinese-ways-to-contribute)
- [开发环境设置](#chinese-development-setup)
- [项目结构](#chinese-project-structure)
- [提交更改](#chinese-submitting-changes)
- [平台特定指南](#chinese-platform-specific-guidelines)
- [架构决策](#chinese-architecture-decisions)

---

<a name="chinese-code-of-conduct"></a>

## 行为准则

本项目遵循[贡献者公约](CODE_OF_CONDUCT.md)。通过参与，您同意维护一个友好、无骚扰的环境。

---

<a name="chinese-ways-to-contribute"></a>

## 贡献方式

### 高优先级领域

- **Windows 沙箱加固** — AppContainer/Job Object 覆盖是最薄弱的环节
- **新的 LLM 提供商实现** — Gemini、Mistral、本地 GGUF 模型
- **脚本桥接改进** — Lua ↔ Rust FFI 性能、Deno/V8 嵌入稳定性
- **平台特定的测试覆盖** — 特别是 Windows CI 边缘情况
- **文档** — 架构说明、指南修正、翻译

### 其他受欢迎的贡献

- 带有复现步骤的 Bug 报告
- 性能基准测试和回归测试
- 新的通道集成（Matrix、XMPP、Nostr）
- 展示可扩展性模式的示例代理

---

<a name="chinese-development-setup"></a>

## 开发环境设置

### 前置条件

| 工具 | 版本 | 说明 |
|------|---------|-------|
| Rust | 1.83+ (参见 `rust-toolchain.toml`) | 通过 [rustup](https://rustup.rs) 安装 |
| cargo | 与 Rust 捆绑 | — |
| Node.js | ≥ 20 (可选) | 仅在开发 `engine-v8` 功能时需要 |
| Python | ≥ 3.10 (可选) | 仅在开发 `engine-py` 功能时需要 |

#### 平台特定依赖

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install libseccomp-dev pkg-config
```

**Linux (Fedora/RHEL):**
```bash
sudo dnf install libseccomp-devel
```

**macOS:**
```bash
# 通常无需额外依赖
```

**Windows:**
```bash
# 使用 MSVC 工具链，而非 GNU
rustup set default-host x86_64-pc-windows-msvc
```

### 克隆并构建

```bash
git clone https://github.com/claw-project/claw-kernel.git
cd claw-kernel

# 默认构建（仅 Lua 引擎，无额外依赖）
cargo build

# 使用 Deno/V8 引擎（下载预编译的 V8，可能需要几分钟）
cargo build --features engine-v8

# 使用 Python 引擎
cargo build --features engine-py

# 运行所有 crate 的测试
cargo test --workspace
```

### 运行跨平台测试套件

```bash
# 仅单元测试
cargo test --workspace

# 包含集成测试
cargo test --workspace --features integration-tests

# 平台特定的沙箱测试（仅限 Linux）
cargo test --workspace --features sandbox-tests
```

---

<a name="chinese-project-structure"></a>

## 项目结构

> **注意**：项目当前处于设计/规划阶段。`crates/` 目录为空 — 实现遵循 `docs/architecture/` 中的架构设计。

```
claw-kernel/
├── crates/               # 工作空间 crate（每个库一个）- 当前为空
│   ├── claw-pal/         # 平台抽象层
│   ├── claw-provider/    # LLM 提供商 trait + 实现
│   ├── claw-tools/       # 工具注册表和热加载
│   ├── claw-loop/        # 代理循环引擎
│   ├── claw-runtime/     # 事件总线和进程管理
│   ├── claw-script/      # 嵌入式脚本引擎
│   └── claw-kernel/      # 元 crate（重新导出以上所有）
├── docs/
│   ├── architecture/     # 逐层架构文档
│   ├── adr/              # 架构决策记录
│   ├── guides/           # 面向用户的操作指南
│   ├── crates/           # 每个 crate 的文档
│   └── platform/         # 平台特定说明
├── examples/             # 可运行的示例代理
└── .github/              # CI 工作流和 Issue 模板
```

完整的 crate 依赖图请参见 [docs/architecture/crate-map.md](docs/architecture/crate-map.md)。

---

<a name="chinese-submitting-changes"></a>

## 提交更改

### 关键术语

- **重大变更 (Significant changes)**：新增或修改公共 API 或 trait、修改跨平台行为、引入新依赖、改变安全模型、或影响现有架构决策的变更。这些需要编写 ADR。
- **面向用户的变更 (User-facing change)**：影响公共 API、新增功能、修改行为或更改配置选项的变更。内部重构和测试更新除外。这些需要更新 CHANGELOG。
- **简短描述 (Short description)**：分支名应使用小写字母和连字符，最多 30 个字符。示例：`fix/memory-leak-in-sandbox`、`feat/add-gemini-provider`。
- **达成共识 (Consensus)**：通常以维护者在讨论 Issue 上的批准评论为标志。

### 修复 Bug

1. 打开一个 Issue 描述 Bug（或在现有 Issue 上评论）
2. Fork 仓库并创建分支：`fix/<简短描述>`
3. 编写一个能复现 Bug 的失败测试
4. 修复 Bug
5. 确认测试在您的本地平台上通过
6. 提交引用该 Issue 的 Pull Request

### 新功能

1. **首先打开 Issue 或 Discussion** — 涉及 `claw-pal` 或跨平台行为的功能需要事先达成一致
2. 对于重大更改，编写一个 [ADR](docs/adr/) 描述该决策
3. Fork 并创建分支：`feat/<简短描述>`
4. 带着测试实现
5. 更新 `docs/` 中的相关文档
6. 提交 Pull Request

### Pull Request 检查清单

- [ ] 测试在您的本地平台上通过 (`cargo test --workspace`)
- [ ] 没有新的 `clippy` 警告 (`cargo clippy --workspace`)
- [ ] 代码已格式化 (`cargo fmt --all`)
- [ ] 如果行为发生变化，已更新文档
- [ ] PR 描述解释了*为什么*，而不仅仅是*做了什么*
- [ ] 注明了平台影响（仅 Linux？所有平台？）
- [ ] CHANGELOG.md 已更新（如果是面向用户的变更）
- [ ] 已添加 ADR（如果是重大架构变更）

---

<a name="chinese-platform-specific-guidelines"></a>

## 平台特定指南

### 添加平台特定的实现

所有平台特定的代码都位于 `claw-pal` 中。使用 Rust 的条件编译：

```rust
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;
```

每个平台特定的实现必须：
1. 满足 `claw-pal/src/traits.rs` 中定义的共享 `Trait`
2. 在 `tests/` 中包含至少一个集成测试
3. 在相应的 `docs/platform/` 文件中有文档说明

### Windows 贡献者

- 使用 MSVC 工具链，而非 GNU (`rustup set default-host x86_64-pc-windows-msvc`)
- 避免 Unix 风格：不要使用 `fork()`、POSIX 信号、硬编码的 `/` 路径
- 显式测试命名管道行为 — 它与 Unix 域套接字有微妙的差异

---

<a name="chinese-architecture-decisions"></a>

## 架构决策

重大决策以 ADR（架构决策记录）的形式记录在 [`docs/adr/`](docs/adr/) 中。在提议重大架构更改之前：

1. 阅读现有的 ADR 以了解过去的决策
2. 打开一个带有 `adr` 标签的讨论 Issue
3. 如果达成共识，提交一个添加新 ADR 的 PR

ADR 使用简单的格式：**背景 → 决策 → 后果**。

---

## 有问题？

- 打开 [GitHub Discussion](https://github.com/claw-project/claw-kernel/discussions) 讨论设计问题
- 打开 [Issue](https://github.com/claw-project/claw-kernel/issues) 报告 Bug
- 在您的 PR 中标记 `@claw-project/maintainers` 以获得更快的审查
