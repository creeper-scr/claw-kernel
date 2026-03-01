[English](#english) | [中文](#chinese)

<a name="english"></a>

# Contributing to claw-kernel

claw-kernel is the shared foundation of the Claw ecosystem. We hold contributions to a high standard, but the bar for getting started is intentionally low. Every bug report, doc fix, and test improvement matters.

> **Project Status**: v0.1.0 released. 9 crates implemented, 389 tests passing. Great time to contribute new providers, sandbox hardening, or test coverage.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By participating, you agree to uphold a welcoming, harassment-free environment.

## Ways to Contribute

- **Bug reports** — Open an issue with reproduction steps, expected vs. actual behavior, and your platform/Rust version.
- **Feature requests** — Open a discussion first. Features touching `claw-pal` or cross-platform behavior need prior agreement.
- **Documentation** — Fix typos, improve clarity, add examples, translate. No PR is too small.
- **Code** — See [High-Priority Areas](#high-priority-areas) below.
- **Architecture review** — Read the ADRs in `docs/adr/` and comment on open discussions.

## Your First Contribution

Look for issues labeled `good first issue` or `help wanted`. Documentation and test additions are great entry points. Comment on an issue before starting so others know you're working on it. Open a draft PR early — feedback before you finish is better than after.

## Development Setup

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.83+ | Via [rustup](https://rustup.rs) |
| cargo | bundled | — |
| Node.js | >= 20 (optional) | Only for `engine-v8` feature |
| Python | >= 3.10 (optional) | Only for `engine-py` feature |

**Linux (Ubuntu/Debian):** `sudo apt-get install libseccomp-dev pkg-config`
**Linux (Fedora/RHEL):** `sudo dnf install libseccomp-devel`
**Windows:** `rustup set default-host x86_64-pc-windows-msvc`

### Build and Test

```bash
cargo build                                        # default (Lua only)
cargo build --features engine-v8                   # with Deno/V8 (5-15 min)
cargo build --features engine-py                   # with Python
cargo test --workspace                             # all tests
cargo test --workspace --features integration-tests
cargo test --workspace --features sandbox-tests    # Linux only
cargo fmt --all                                    # format
cargo clippy --workspace -- -D warnings            # lint (same as CI)
cargo audit                                        # security audit
```

## Fork and Clone Workflow

```bash
# 1. Fork on GitHub, then:
git clone https://github.com/<your-username>/claw-kernel.git
cd claw-kernel
git remote add upstream https://github.com/claw-project/claw-kernel.git

# 2. Create a branch
git checkout -b feat/add-gemini-provider

# 3. Push and open a PR
git push origin feat/add-gemini-provider
```

To keep your fork up to date: `git fetch upstream && git rebase upstream/main`

## Branch Naming

| Prefix | Use for |
|--------|---------|
| `feat/` | New features |
| `fix/` | Bug fixes |
| `docs/` | Documentation only |
| `chore/` | Maintenance, CI, dependency updates |
| `refactor/` | Code restructuring without behavior change |
| `test/` | Adding or fixing tests |

Max 30 characters, lowercase with hyphens. Example: `feat/add-gemini-provider`

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/).

```
<type>(<scope>): <short summary>   # 72 chars max, imperative mood

[optional body: explain why, not what]

[optional footer: Fixes #123]
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `perf`
Scope (optional): crate or area, e.g. `claw-pal`, `sandbox`, `ci`

Breaking changes: add `BREAKING CHANGE:` in the footer.

## Pull Request Process

**Before opening a PR:** for bug fixes, open an issue first. For new features, open a discussion. For significant architectural changes, follow the [ADR process](#architecture-decision-records).

**PR checklist:**
- [ ] Tests pass locally (`cargo test --workspace`)
- [ ] No new clippy warnings (`cargo clippy --workspace -- -D warnings`)
- [ ] Code formatted (`cargo fmt --all`)
- [ ] Documentation updated if behavior changed
- [ ] PR description explains *why*, not just *what*
- [ ] Platform impact noted (Linux only? All platforms?)
- [ ] CHANGELOG.md updated for user-facing changes
- [ ] ADR added for significant architectural changes (if applicable)

Keep PRs small and focused. One logical change per PR. One maintainer approval required to merge. CI must pass.

## Code Quality Standards

**Cross-platform first.** Platform-specific code belongs only in `claw-pal`, isolated with `#[cfg(target_os = "...")]`. No `fork()`, no POSIX signals, no hardcoded `/` paths in shared code.

**Documentation.** All public APIs need doc comments (`///`). Include at least one example for non-trivial functions.

**Error handling.** Use `thiserror` for library errors. `anyhow` is for application/example code only.

**Feature flags.** Use for optional heavy dependencies (`engine-v8`, `engine-py`). Default build must have minimal dependencies.

## Testing Requirements

| Crate | Unit | Integration | Platform |
|-------|:----:|:-----------:|:--------:|
| claw-pal | Yes | Yes | Required per-platform |
| claw-provider | Yes | Yes (mock HTTP) | N/A |
| claw-tools | Yes | Yes | N/A |
| claw-loop | Yes | Yes | N/A |
| claw-runtime | Yes | Yes | Required |
| claw-script | Yes | Yes | Required per-engine |

For `claw-pal` platform code: write a failing test first, satisfy the shared trait, include an integration test, document in `docs/platform/`.

## Architecture Decision Records

Major decisions are recorded as ADRs in [`docs/adr/`](docs/adr/). A change is "significant" if it adds/modifies public APIs, changes cross-platform behavior, introduces new dependencies, or affects the security model.

**Process:** open a GitHub Discussion with the `adr` label -> reach consensus -> open a PR with the new ADR.

ADR format: **Context -> Decision -> Consequences**.

## High-Priority Areas

- **New LLM providers** — Gemini, Mistral, local GGUF models (llama.cpp-compatible)
- **Windows sandbox hardening** — AppContainer/Job Object implementation is not yet complete
- **Deno/V8 engine bridge** — `engine-v8` feature using `deno_core`
- **Python engine bridge** — `engine-py` feature using PyO3
- **Integration test expansion** — cross-platform CI coverage, sandbox-tests
- **Documentation** — guide corrections, rustdoc examples, translation

## Development FAQ

**Q: Where do I start?**
Run `cargo test --workspace` to get your bearings. Then read `docs/architecture/overview.md` to understand the layer structure. Pick a High-Priority Area above.

**Q: Do I need to test on all three platforms?**
Test on your local platform. CI covers the others. Note which platform you tested in the PR.

**Q: My clippy check fails with a warning I disagree with.**
Open a discussion. We can add targeted `#[allow(...)]` with a comment, but we don't disable lints globally.

**Q: Does my change need an ADR?**
If it changes a public interface, adds a dependency, or affects the security model, probably yes. When in doubt, ask in a GitHub Discussion.

## Dependency Versions

> Actual pinned versions are in `Cargo.toml`. Below is the authoritative reference.

### Core Runtime

```toml
tokio = { version = "1.35.0", features = ["rt-multi-thread", "macros", "sync", "time", "fs"] }
async-trait = "0.1.77"
reqwest = { version = "0.11.23", features = ["json", "stream"] }
serde = { version = "1.0.195", features = ["derive"] }
serde_json = "1.0.111"
thiserror = "1.0.56"
anyhow = "1.0.79"
tracing = "0.1.40"
```

### PAL Layer

```toml
interprocess = { version = "1.2.1", features = ["tokio_support"] }
dirs = "5.0.1"
# Linux only:
libseccomp = "0.3.0"
nix = { version = "0.27.1", features = ["process", "sched"] }
```

### Script Engines & Optional

```toml
mlua = { version = "0.9.4", features = ["lua54", "async", "send", "serde"], optional = true }
deno_core = { version = "0.245.0", optional = true }       # engine-v8; needs Node.js >= 20
pyo3 = { version = "0.28.0", features = ["auto-initialize"], optional = true }  # engine-py; needs Python >= 3.10
schemars = "0.8.16"
notify = { version = "6.1.1", features = ["tokio"] }
rusqlite = { version = "0.30.0", features = ["bundled", "chrono"], optional = true }
```

## Feature Matrix

| Config | Features | Use Case | Approx Build Time | Binary Size |
|--------|----------|----------|-------------------|-------------|
| minimal | `engine-lua` | Prototyping | < 2 min | ~5 MB |
| **default** | `engine-lua`, `sqlite` | Production | < 3 min | ~8 MB |
| full-js | + `engine-v8` | JS/TS ecosystem | ~30 min | ~100 MB |
| ml-ready | + `engine-py` | ML integration | ~10 min | ~50 MB |
| complete | all | Everything | ~35 min | ~120 MB |

*Build times: AMD Ryzen 5 5600X, 32 GB RAM, SSD, first build, no sccache. Binary sizes: release+stripped, Linux.*

**Known constraints:**
- `engine-py` requires Rust **1.83+** (PyO3 0.28+ hard requirement) and Python ≥ 3.10; GIL conflicts with async — run Python code in a separate thread or use `pyo3-async-runtimes`
- `engine-v8` first build ~30 min; mitigate with `sccache` or CI cache
- Windows sandbox (AppContainer) requires admin for testing; WSL2 recommended for development

## Adding claw-kernel as a Dependency

```toml
# Minimal
claw-kernel = { version = "0.1", default-features = false, features = ["engine-lua"] }

# Recommended (with persistence)
claw-kernel = { version = "0.1", features = ["engine-lua", "sqlite"] }

# Full
claw-kernel = { version = "0.1", features = ["full"] }
```

## Recognition

All contributors are credited in release notes and the GitHub contributors graph. By submitting a PR, you agree your contribution is licensed under MIT OR Apache-2.0.

Questions? [GitHub Discussions](https://github.com/claw-project/claw-kernel/discussions) for design questions, [Issues](https://github.com/claw-project/claw-kernel/issues) for bugs, tag `@claw-project/maintainers` in your PR for faster review.

---

<a name="chinese"></a>

# 参与贡献 claw-kernel

claw-kernel 是 Claw 生态系统的共享基础。我们对贡献有较高的标准，但开始参与的门槛被有意设置得很低。

> **项目状态**：v0.1.0 已发布。9 个 crate 已实现，389 个测试通过。现在是参与新 Provider、沙箱加固或测试覆盖的好时机。

## 行为准则

本项目遵循[贡献者公约](CODE_OF_CONDUCT.md)。通过参与，您同意为所有人维护友好、无骚扰的环境。

## 贡献方式

- **Bug 报告** — 提交包含复现步骤、预期与实际行为、平台和 Rust 版本的 Issue
- **功能请求** — 先开启讨论，涉及 `claw-pal` 或跨平台行为的功能需要事先达成一致
- **文档** — 修正错别字、改善清晰度、添加示例、翻译，没有太小的 PR
- **代码** — 参见下方[高优先级领域](#chinese-high-priority)
- **架构审查** — 阅读 `docs/adr/` 中的 ADR 并在开放讨论中发表评论

## 第一次贡献

查找标有 `good first issue` 或 `help wanted` 的 Issue。在开始前在 Issue 上评论，让其他人知道您正在处理。尽早开启草稿 PR，完成前获得反馈比完成后更好。

## 开发环境设置

### 前置条件

| 工具 | 版本 | 说明 |
|------|------|------|
| Rust | 1.83+ | 通过 [rustup](https://rustup.rs) 安装 |
| cargo | 随附 | — |
| Node.js | >= 20（可选） | 仅 `engine-v8` feature 需要 |
| Python | >= 3.10（可选） | 仅 `engine-py` feature 需要 |

**平台特定依赖：**
- Linux (Ubuntu/Debian)：`sudo apt-get install libseccomp-dev pkg-config`
- Linux (Fedora/RHEL)：`sudo dnf install libseccomp-devel`
- Windows：`rustup set default-host x86_64-pc-windows-msvc`

### 构建与测试

```bash
cargo build                                        # 默认构建（仅 Lua）
cargo build --features engine-v8                   # 含 Deno/V8（5-15 分钟）
cargo build --features engine-py                   # 含 Python
cargo test --workspace                             # 全部测试
cargo test --workspace --features integration-tests
cargo test --workspace --features sandbox-tests    # 仅 Linux
cargo fmt --all                                    # 格式化
cargo clippy --workspace -- -D warnings            # Lint（与 CI 一致）
cargo audit                                        # 安全审计
```

## Fork 和克隆工作流

```bash
# 1. 在 GitHub 上 Fork，然后：
git clone https://github.com/<您的用户名>/claw-kernel.git
cd claw-kernel
git remote add upstream https://github.com/claw-project/claw-kernel.git

# 2. 创建分支
git checkout -b feat/add-gemini-provider

# 3. 推送并开启 PR
git push origin feat/add-gemini-provider
```

保持 fork 最新：`git fetch upstream && git rebase upstream/main`

## 分支命名

使用小写字母和连字符，最多 30 个字符。前缀：`feat/`（新功能）、`fix/`（Bug 修复）、`docs/`（文档）、`chore/`（维护）、`refactor/`（重构）、`test/`（测试）。

## 提交信息规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

```
<类型>(<范围>): <简短摘要>   # 最多 72 字符，祈使语气

[可选正文：解释为什么，而不是做了什么]

[可选页脚：Fixes #123]
```

类型：`feat`、`fix`、`docs`、`chore`、`refactor`、`test`、`perf`。破坏性变更在页脚添加 `BREAKING CHANGE:`。

## Pull Request 流程

**开启 PR 前：** Bug 修复先开 Issue，新功能先开讨论，重大架构变更遵循 ADR 流程。

**PR 检查清单：**
- [ ] 测试在本地平台通过（`cargo test --workspace`）
- [ ] 没有新的 clippy 警告（`cargo clippy --workspace -- -D warnings`）
- [ ] 代码已格式化（`cargo fmt --all`）
- [ ] 行为变化时已更新文档
- [ ] PR 描述解释了*为什么*，而不仅仅是*做了什么*
- [ ] 注明了平台影响（仅 Linux？所有平台？）
- [ ] 面向用户的变更已更新 CHANGELOG.md
- [ ] 重大架构变更已添加 ADR（如适用）

保持 PR 小而专注，每个 PR 一个逻辑变更。合并需要一位维护者批准，CI 必须通过。

## 代码质量标准

**跨平台优先。** 平台特定代码只能放在 `claw-pal` 中，使用 `#[cfg(target_os = "...")]` 隔离。共享代码中不使用 `fork()`、POSIX 信号、硬编码的 `/` 路径。

**文档。** 所有公共 API 需要文档注释（`///`），非平凡函数至少包含一个示例。

**错误处理。** 库错误使用 `thiserror`，`anyhow` 只用于应用/示例代码。

**功能标志。** 对重量级可选依赖使用功能标志（`engine-v8`、`engine-py`），默认构建必须具有最少依赖。

## 测试要求

| Crate | 单元测试 | 集成测试 | 平台测试 |
|-------|:--------:|:--------:|:--------:|
| claw-pal | 是 | 是 | 每平台必须 |
| claw-provider | 是 | 是（mock HTTP） | N/A |
| claw-tools | 是 | 是 | N/A |
| claw-loop | 是 | 是 | N/A |
| claw-runtime | 是 | 是 | 必须 |
| claw-script | 是 | 是 | 每引擎必须 |

对于 `claw-pal` 平台特定代码：先写失败测试，满足共享 trait，包含集成测试，在 `docs/platform/` 中记录行为。

## 架构决策记录

重大决策记录为 ADR，存放在 [`docs/adr/`](docs/adr/)。重大变更包括：新增/修改公共 API、跨平台行为变更、新依赖、安全模型变更。

**流程：** 开启带 `adr` 标签的 GitHub Discussion -> 达成共识 -> 提交添加新 ADR 的 PR。

ADR 格式：**背景 -> 决策 -> 后果**。

<a name="chinese-high-priority"></a>

## 高优先级领域

- **新的 LLM Provider 实现** — Gemini、Mistral、本地 GGUF 模型（llama.cpp 兼容）
- **Windows 沙箱加固** — AppContainer/Job Object 实现尚不完整
- **Deno/V8 引擎桥接** — 使用 `deno_core` 实现 `engine-v8` feature
- **Python 引擎桥接** — 使用 PyO3 实现 `engine-py` feature
- **集成测试扩展** — 跨平台 CI 覆盖、sandbox-tests
- **文档** — 指南修正、rustdoc 示例、翻译改进

## 开发常见问题

**Q：从哪里开始？** 运行 `cargo test --workspace` 熟悉代码库，然后阅读 `docs/architecture/overview.md` 了解层次架构，再从上方高优先级领域中选择一个切入点。

**Q：需要在所有三个平台上测试吗？** 在本地平台测试即可，CI 覆盖其他平台，在 PR 中注明测试平台。

**Q：clippy 警告我不同意怎么办？** 开启讨论，可以添加带注释的 `#[allow(...)]`，但不全局禁用 lint。

**Q：我的变更需要 ADR 吗？** 如果改变了公共接口、添加了依赖或影响了安全模型，可能需要。不确定时在 GitHub Discussion 中询问。

## 致谢

所有贡献者都会在发布说明和 GitHub 贡献者图表中获得致谢。提交 PR 即表示您同意贡献在 MIT OR Apache-2.0 双重许可证下授权。

有问题？[GitHub Discussions](https://github.com/claw-project/claw-kernel/discussions) 用于设计问题，[Issues](https://github.com/claw-project/claw-kernel/issues) 用于 Bug，在 PR 中标记 `@claw-project/maintainers` 获得更快审查。

## 依赖版本

> 实际锁定版本以 `Cargo.toml` 为准，下表为权威参考。

### 核心运行时

```toml
tokio = { version = "1.35.0", features = ["rt-multi-thread", "macros", "sync", "time", "fs"] }
async-trait = "0.1.77"
reqwest = { version = "0.11.23", features = ["json", "stream"] }
serde = { version = "1.0.195", features = ["derive"] }
serde_json = "1.0.111"
thiserror = "1.0.56"
anyhow = "1.0.79"
tracing = "0.1.40"
```

### PAL 层

```toml
interprocess = { version = "1.2.1", features = ["tokio_support"] }
dirs = "5.0.1"
# 仅 Linux：
libseccomp = "0.3.0"
nix = { version = "0.27.1", features = ["process", "sched"] }
```

### 脚本引擎与可选依赖

```toml
mlua = { version = "0.9.4", features = ["lua54", "async", "send", "serde"], optional = true }
deno_core = { version = "0.245.0", optional = true }       # engine-v8；需要 Node.js >= 20
pyo3 = { version = "0.28.0", features = ["auto-initialize"], optional = true }  # engine-py；需要 Python >= 3.10
schemars = "0.8.16"
notify = { version = "6.1.1", features = ["tokio"] }
rusqlite = { version = "0.30.0", features = ["bundled", "chrono"], optional = true }
```

## 特性矩阵

| 配置 | Features | 适用场景 | 大约构建时间 | 二进制大小 |
|------|----------|----------|-------------|-----------|
| minimal | `engine-lua` | 原型开发 | < 2 分钟 | ~5 MB |
| **default** | `engine-lua`, `sqlite` | 生产环境 | < 3 分钟 | ~8 MB |
| full-js | + `engine-v8` | JS/TS 生态 | ~30 分钟 | ~100 MB |
| ml-ready | + `engine-py` | ML 集成 | ~10 分钟 | ~50 MB |
| complete | 全部 | 全功能 | ~35 分钟 | ~120 MB |

*构建时间：AMD Ryzen 5 5600X，32 GB RAM，SSD，首次构建，无 sccache。二进制大小：release+stripped，Linux。*

**已知限制：**
- `engine-py` 需要 Rust **1.83+**（PyO3 0.28+ 硬性要求）和 Python ≥ 3.10；GIL 与 async 有冲突——在独立线程中运行 Python 代码，或使用 `pyo3-async-runtimes`
- `engine-v8` 首次构建约 30 分钟；可用 `sccache` 或 CI 缓存缓解
- Windows 沙箱（AppContainer）测试需要管理员权限；推荐使用 WSL2 进行开发
