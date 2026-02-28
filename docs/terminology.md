# claw-kernel 术语对照表 / Terminology Reference

> 本文档提供 claw-kernel 项目中英文术语对照，确保文档一致性。

---

## 核心架构术语 / Core Architecture Terms

| 英文 (English) | 中文 (Chinese) | 备注 |
|----------------|----------------|------|
| Architecture Layer | 架构层 | 统一使用 5 层架构 |
| Layer 0: Rust Hard Core | 第 0 层：Rust 硬核核心 | Trust root，不可修改 |
| Layer 0.5: PAL | 第 0.5 层：平台抽象层 | Platform Abstraction Layer |
| Layer 1: System Runtime | 第 1 层：系统运行时 | Tokio 异步运行时 |
| Layer 2: Agent Kernel Protocol | 第 2 层：Agent 内核协议 | 核心协议层 |
| Layer 3: Extension Foundation | 第 3 层：扩展基础 | 脚本运行时，内核边界 |

---

## 安全模型术语 / Security Model Terms

| 英文 (English) | 中文 (Chinese) | 备注 |
|----------------|----------------|------|
| Safe Mode | 安全模式 (Safe Mode) | 默认沙箱模式 |
| Power Mode | 强力模式 (Power Mode) | 完全访问，需显式授权 |
| Sandbox | 沙箱 (Sandbox) | 执行环境隔离 |
| Power Key | 强力密钥 | 激活 Power Mode 的凭证 |
| Execution Mode | 执行模式 | Safe 或 Power |
| Sandbox Backend | 沙箱后端 | 平台特定实现 |

---

## 扩展性术语 / Extensibility Terms

| 英文 (English) | 中文 (Chinese) | 备注 |
|----------------|----------------|------|
| Hot-loading | 热加载 (Hot-loading) | 运行时加载/卸载 |
| Hot-reload | 热重载 | 避免使用，统一用 Hot-loading |
| Tool Registry | 工具注册表 | 工具管理 |
| Script Engine | 脚本引擎 | Lua/V8/Python |
| Extension Foundation | 扩展基础 | Layer 3 定位 |
| Self-Evolution | 自进化 | 应用层能力 |
| Dynamic Registration | 动态注册 | 运行时注册工具 |

---

## 技术术语 / Technical Terms

| 英文 (English) | 中文 (Chinese) | 备注 |
|----------------|----------------|------|
| Feature Flag | 特性标志 (Feature Flag) | Cargo 特性 |
| engine-lua | engine-lua | 连字符格式 |
| engine-v8 | engine-v8 | 连字符格式 |
| engine-py | engine-py | 连字符格式 |
| Crate | Crate | Rust 包单位 |
| Trait | Trait | Rust 接口 |
| IPC | IPC | 进程间通信 |
| Provider | 提供商/提供者 | LLM 提供商 |

---

## 平台术语 / Platform Terms

| 英文 (English) | 中文 (Chinese) | 备注 |
|----------------|----------------|------|
| Linux | Linux | 最强隔离 (Strongest) |
| macOS | macOS | 中等隔离 (Medium) |
| Windows | Windows | 中等隔离 (Medium) |
| seccomp-bpf | seccomp-bpf | Linux 沙箱技术 |
| Namespaces | 命名空间 | Linux 隔离技术 |
| Sandbox Profile | 沙箱配置文件 | macOS Seatbelt |
| AppContainer | AppContainer | Windows 沙箱 |

---

## 文档规范 / Documentation Guidelines

1. **中英双语文档**：主要文档需包含中英文对照
2. **技术术语**：首次出现时用 `(English)` 标注，如 `热加载 (Hot-loading)`
3. **Feature Flag**：统一使用连字符格式 `engine-lua`，不用下划线
4. **架构层数**：统一使用 **五层架构 (Five-Layer Architecture)**，Layer 3 统一使用 **Extension Foundation / 扩展基础**
5. **版本号**：Rust **1.83+**, Node.js ≥ 20, Python ≥ 3.10

---

## 禁止的用法 / Avoid These

| ❌ 避免 | ✅ 使用 |
|---------|---------|
| Six-Layer Architecture | Five-Layer Architecture |
| 六层架构 | 五层架构 |
| engine_lua | engine-lua |
| hot reload | hot-loading |
| 沙盒 | 沙箱 (Sandbox) |
| 中 (隔离级别) | 中等 (Medium) |
