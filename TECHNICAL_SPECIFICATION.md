---
title: claw-kernel Technical Specification
description: Locked technical choices, version requirements, and compatibility constraints
status: design-phase
version: "1.0"
last_updated: "2026-02-28"
---

> **Project Status**: Design/Planning Phase — Specifications are defined but implementation has not started.

# claw-kernel 技术规格说明书
# Technical Specification

**目标**：锁定所有技术选型、版本要求和兼容性约束  
**版本**：v1.0 | **日期**：2026-02-28

---

## 1. 环境要求 / Environment Requirements

### 1.1 最低系统要求

| 组件 | 最低版本 | 推荐版本 | 说明 |
|------|----------|----------|------|
| **Rust** | **1.83+** | 1.83+ | MSRV，由 PyO3 0.28+ 决定 |
| Cargo | 随 Rust | 随 Rust | 构建工具 |
| Node.js | 20 | 20 LTS | 仅 `engine-v8` 特性 |
| Python | 3.10 | 3.12 | 仅 `engine-py` 特性 |

### 1.2 平台特定要求

#### Linux
```bash
# Ubuntu/Debian
sudo apt-get install libseccomp-dev pkg-config

# Fedora/RHEL
sudo dnf install libseccomp-devel

# Arch
sudo pacman -S libseccomp
```
- 内核版本：4.15+ (推荐 5.0+)
- 需要 `CONFIG_SECCOMP=y`

#### macOS
- macOS 10.15+ (推荐 11.0+)
- Xcode Command Line Tools
- 完整沙箱测试需要代码签名

#### Windows
- Windows 10/11 64-bit
- Visual Studio 2019+ 或 Build Tools
- **必须使用 MSVC 工具链**：`rustup set default-host x86_64-pc-windows-msvc`
- 沙箱测试需要管理员权限

---

## 2. 依赖版本锁定 / Dependency Pinning

### 2.1 核心运行时 (Core Runtime)

```toml
# Cargo.toml
[dependencies]
# 异步运行时 - 经过 Tokio 1.35+ 验证
tokio = { version = "1.35.0", features = ["rt-multi-thread", "macros", "sync", "time", "fs"] }
async-trait = "0.1.77"

# HTTP 客户端 - 与 Tokio 完全兼容
reqwest = { version = "0.11.23", features = ["json", "stream"] }

# 序列化
serde = { version = "1.0.195", features = ["derive"] }
serde_json = "1.0.111"

# 错误处理
thiserror = "1.0.56"
anyhow = "1.0.79"

# 日志与追踪
tracing = "0.1.40"
tracing-subscriber = { version = "0.3.18", features = ["env-filter"] }
```

### 2.2 平台抽象层 (PAL)

```toml
# 跨平台 IPC
interprocess = { version = "1.2.1", features = ["tokio"] }

# 配置目录
dirs = "5.0.1"

# Linux 沙箱 (条件编译)
[target.'cfg(target_os = "linux")'.dependencies]
libseccomp = "0.3.0"
nix = { version = "0.27.1", features = ["process", "sched"] }
```

### 2.3 脚本引擎 (Script Engines)

```toml
# Lua 引擎 - 默认，零依赖
mlua = { version = "0.9.4", features = ["lua54", "async", "send", "serde"], optional = true }

# Deno/V8 引擎 - 可选，构建慢
# Node.js ≥ 20 需要
deno_core = { version = "0.245.0", optional = true }

# Python 引擎 - 可选，有限制
# Rust 1.83+, Python ≥ 3.10 需要
# 注意：GIL 与 async 不完全兼容
pyo3 = { version = "0.28.0", features = ["auto-initialize"], optional = true }
```

### 2.4 工具与扩展

```toml
# JSON Schema
schemars = "0.8.16"

# 文件监视 (热加载)
notify = { version = "6.1.1", features = ["tokio"] }

# SQLite 后端
rusqlite = { version = "0.30.0", features = ["bundled", "chrono"], optional = true }
```

### 2.5 Channel 集成

```toml
# Discord
twilight-gateway = { version = "0.15.4", optional = true }
twilight-model = { version = "0.15.4", optional = true }

# HTTP Webhook
axum = { version = "0.7.4", optional = true }
tower = { version = "0.4.13", optional = true }
```

---

## 3. 特性配置矩阵 / Feature Matrix

### 3.1 预定义配置

| 配置 | 特性 | 适用场景 | 构建时间 | 二进制大小 |
|------|------|----------|----------|------------|
| **minimal** | `engine-lua` | 原型开发、简单工具 | < 2 min* | ~5 MB |
| **default** | `engine-lua`, `sqlite` | 生产环境、持久化 | < 3 min* | ~8 MB |
| **full-js** | `engine-lua`, `engine-v8`, `sqlite` | 需要 TS/JS 生态 | ~30 min* | ~100-110 MB** |
| **ml-ready** | `engine-lua`, `engine-py`, `sqlite` | ML 集成 | ~10 min* | ~50 MB |
| **complete** | 全部 | 完整功能 | ~35 min* | ~120 MB |

*构建时间基准：AMD Ryzen 5 5600X, 32GB RAM, SSD, 首次构建（无 sccache）  
**二进制大小（release, stripped）：Linux ~100 MB, macOS ~105 MB, Windows ~110 MB

### 3.2 特性依赖关系

```
engine-lua
  └── mlua (lua54, async, send, serde)

engine-v8
  └── deno_core
      └── Node.js ≥ 20 (构建时)

engine-py
  └── pyo3
      └── Python ≥ 3.10 (运行时)
          └── Rust 1.83+ (构建时强制要求)

sqlite
  └── rusqlite (bundled)
      └── 自动编译 SQLite，无需系统依赖

sandbox-tests
  └── 启用平台特定的沙箱隔离测试
      └── 需要管理员/root权限运行

discord
  └── twilight-gateway
      └── 基于 Tokio，完全兼容

http
  └── axum + tower
      └── 基于 Tokio，完全兼容

full
  └── 启用所有可选特性
      ├── engine-lua
      ├── engine-v8
      ├── engine-py
      ├── sqlite
      └── sandbox-tests
```

---

## 4. 兼容性约束 / Compatibility Constraints

### 4.1 Rust 版本约束

| 特性组合 | 最低 Rust | 原因 |
|----------|-----------|------|
| + engine-v8 | 1.83+ | deno_core 要求 |
| **+ engine-py** | **1.83+** | **PyO3 0.28+ 强制要求** |
| 全部特性 | **1.83+** | 统一要求 |

**决策**: 统一要求 **Rust 1.83+** 以简化维护，因为 engine-py 特性需要 PyO3 0.28+

### 4.2 外部系统依赖

| 特性 | 外部依赖 | 安装方式 |
|------|----------|----------|
| engine-v8 | Node.js ≥ 20 | 系统包管理器或官网 |
| engine-py | Python ≥ 3.10 + dev headers | 系统包管理器 |
| sqlite (Linux) | 无 | rusqlite bundled |
| sandbox-tests | 无 | 运行时权限要求 |
| PAL (Linux) | libseccomp | 系统包管理器 |
| PAL (macOS) | 无 | 系统自带 |
| PAL (Windows) | MSVC | Visual Studio Installer |

### 4.3 已知限制

#### PyO3 (engine-py)
```yaml
限制: Python GIL 与 Rust async 冲突
影响: 在 async 函数中调用 Python 代码可能阻塞
解决方案:
  1. 使用 pyo3-async-runtimes 桥接
  2. 在独立线程运行 Python
  3. 限制使用场景 (仅 ML 计算)
```

#### deno_core (engine-v8)
```yaml
限制: 构建时间长
影响: 首次构建 ~30 分钟，CI 缓存压力大
解决方案:
  1. 使用 sccache 缓存编译产物
  2. CI 中预编译基础镜像
  3. 默认使用 Lua 引擎
```

#### Windows 沙箱
```yaml
限制: AppContainer 配置复杂，需要管理员权限测试
影响: 开发体验较差，CI 配置复杂
解决方案:
  1. 使用 WSL2 进行大部分开发
  2. 仅在发布前在原生 Windows 测试
  3. 考虑降级为较弱隔离 (仅 Job Objects)
```

---

## 5. 安全模型规格 / Security Model Specification

### 5.1 Safe Mode (默认)

| 资源 | 默认策略 | 可配置 |
|------|----------|--------|
| 文件系统 | 允许列表只读 | Yes 可添加读写目录 |
| 网络 | 域名/端口允许列表 | Yes 可添加端点 |
| 子进程 | 完全禁止 | No 不可配置 |
| 系统调用 | 过滤危险调用 | Yes 可自定义策略 |

### 5.2 Power Mode

| 资源 | 策略 | 备注 |
|------|------|------|
| 文件系统 | 完全访问 | 受 OS 权限约束 |
| 网络 | 无限制 | 受防火墙约束 |
| 子进程 | 允许 | 可执行任意命令 |
| 内核代码 | 不可修改 | 硬限制 |

### 5.3 模式切换

```
Safe Mode ──► Power Mode
  (默认)      (需 --power-mode + --power-key)
     ▲              │
     │              │
     └──────────────┘ (需要重启进程)
```

**Power Key 机制：**
- 最小长度：12 位字符（2026年安全标准）
- 复杂度要求：至少包含大写字母、小写字母、数字中的两种
- 激活方式：
  1. 命令行参数：`--power-mode --power-key <key>`
  2. 环境变量：`CLAW_KERNEL_POWER_KEY=<key>`
  3. 配置文件：`~/.config/claw-kernel/power.key`
- 设置命令：`claw-kernel --set-power-key`
- 重要约束：Power Mode → Safe Mode 切换**必须重启进程**（防止恶意降级）

详细说明请参考 [AGENTS.md](AGENTS.md) 的 "Power Mode Activation" 章节。

### 5.4 审计日志

审计日志详细规格请参考 [AGENTS.md](AGENTS.md) 的 "Audit Logging" 章节。

简要说明：
- 日志位置：`~/.local/share/claw-kernel/logs/audit.log`
- 保留策略：默认 30 天，可配置
- 日志级别：minimal（仅关键事件）、verbose（完整参数）
- 记录事件：工具调用、文件访问、模式切换、网络请求

---

## 6. 构建配置示例 / Build Configuration Examples

### 6.1 最小配置

```toml
# Cargo.toml
[dependencies]
claw-kernel = { version = "0.1", default-features = false, features = ["engine-lua"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
anyhow = "1"
```

### 6.2 推荐配置

```toml
# Cargo.toml
[dependencies]
claw-kernel = { version = "0.1", features = ["engine-lua", "sqlite"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
tracing = "0.1"
```

### 6.3 完整配置

```toml
# Cargo.toml
[dependencies]
claw-kernel = { version = "0.1", features = ["full"] }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

---

## 7. 验证检查清单 / Verification Checklist

### 7.1 开发环境验证

```bash
# 1. Rust 版本
rustc --version  # 应 >= 1.83.0
cargo --version

# 2. 克隆并构建
git clone https://github.com/claw-project/claw-kernel
cd claw-kernel
cargo build --workspace

# 3. 运行测试
cargo test --workspace

# 4. 验证特性 (可选)
cargo build --features engine-v8    # 需要 Node.js ≥ 20
cargo build --features engine-py    # 需要 Python ≥ 3.10
```

### 7.2 平台特定验证

#### Linux
```bash
# 验证 seccomp 支持
cargo test --workspace --features sandbox-tests

# 检查内核配置
zcat /proc/config.gz | grep CONFIG_SECCOMP
```

#### macOS
```bash
# 验证沙箱配置文件编译
cargo build --features sandbox-tests

# 代码签名 (测试用)
codesign -s - target/debug/claw-kernel
```

#### Windows
```bash
# 以管理员身份运行 PowerShell
cargo test --workspace

# 验证工具链
rustup show
# 应显示: x86_64-pc-windows-msvc
```

---

## 8. 文档索引 / Documentation Index

| 文档 | 内容 |
|------|------|
| [TECHNICAL_SPECIFICATION.md](TECHNICAL_SPECIFICATION.md) | 本文件：技术规格和版本锁定 |
| [docs/technical-feasibility-analysis.md](docs/technical-feasibility-analysis.md) | 详细可行性分析 |
| [docs/terminology.md](docs/terminology.md) | 术语对照表 |
| [BUILD_PLAN.md](BUILD_PLAN.md) | 构建路线图 |
| [AGENTS.md](AGENTS.md) | AI 代理开发指南 |

---

*规格版本: v1.0*
*最后更新: 2026-02-28*
*维护者: claw-project team*
