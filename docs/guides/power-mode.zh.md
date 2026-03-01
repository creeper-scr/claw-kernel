---
title: 强力模式指南
description: Power mode activation and security guide
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](power-mode.md)


# 强力模式指南

强力模式授予智能体完全系统访问权限。当你需要智能体执行无限制的自动化任务时使用它。

---

## 什么是强力模式？

强力模式移除大多数限制：

| 能力 | 安全模式 | 强力模式 |
|------|----------|----------|
| **文件系统** | 仅允许列表目录 | 完全访问 |
| **网络** | 仅允许域名 | 无限制 |
| **子进程** | 被阻止 | 允许 |
| **系统调用** | 被过滤 | 无限制 |
| **可扩展性** | 沙箱 (Sandbox)限制 | 完整能力 |

**使用场景：**
- 系统管理自动化
- 软件安装和配置
- 完整系统备份
- 开发环境设置

---

## 启用强力模式

### 要求

强力模式需要同时满足：
1. `--power-mode` 标志（显式意图）
2. `--power-key` 或配置的密钥（认证）

### 命令行

```bash
# 交互式（提示输入密钥）
my-agent --power-mode

# 使用密钥文件
echo "my-secret-key" > ~/.config/claw-kernel/power.key
chmod 600 ~/.config/claw-kernel/power.key
my-agent --power-mode

# 使用环境变量（不建议用于生产）
CLAW_KERNEL_POWER_KEY=my-secret-key my-agent --power-mode

# 内联（最不安全，仅用于测试）
my-agent --power-mode --power-key my-secret-key
```

### 编程方式

```rust
use claw_kernel::pal::{SandboxConfig, PowerKey};

// 从安全存储加载密钥
let key = PowerKey::from_file("~/.config/claw-kernel/power.key")?;

let config = SandboxConfig::power_mode()
    .with_key(key)
    .build();

let runtime = Runtime::with_sandbox(config)?;
```

---

## 安全模型

### 强力模式允许什么

设计上，强力模式允许：
- 读取/写入用户可访问的任何文件
- 向任何端点发起网络请求
- 生成子进程和 shell 命令
- 加载动态库

### 强力模式仍然保护什么

即使在强力模式下，以下内容仍受保护：
- **内核代码** — 无法修改 claw-kernel 本身
- **凭证存储** — 无法访问内核的安全存储
- **其他用户的数据** — 仍受操作系统权限约束

### 密钥保护

强力密钥仅用于认证，不用于加密：

```rust
// 密钥使用 Argon2 哈希
let verification_hash = argon2::hash_raw(key, SALT, PARAMS)?;

// 常量时间比较防止时序攻击
constant_time_eq(&provided_hash, &stored_hash)
```

---

## 启动时模式选择

**重要：** 执行模式在进程启动时确定，重启才能更改。

```
┌─────────────┐      --power-mode + 密钥      ┌─────────────┐
│  安全模式   │  ─────────────────────────►  │  强力模式   │
│  （默认）   │                              │  （显式启用）│
└─────────────┘                              └─────────────┘
       ▲                                            │
       │              以新配置重启                  │
       └────────────────────────────────────────────┘
```

与动态模式切换不同，这种设计：
- 防止被入侵的强力模式智能体通过"降级"隐藏
- 消除模式变更期间的竞态条件
- 防止混淆副手攻击

---

## 最佳实践

### 1. 默认使用安全模式

仅对特定任务启用强力模式：

```rust
// 默认：安全模式
let config = SandboxConfig::safe_mode().build();

// 仅在需要时：强力模式
if user_explicitly_requested_power() {
    let key = prompt_for_power_key()?;
    config = SandboxConfig::power_mode()
        .with_key(key)
        .build();
}
```

### 2. 审计强力模式使用

记录所有强力模式激活：

```rust
if config.mode == ExecutionMode::Power {
    audit_log.record(AuditEvent::PowerModeActivated {
        timestamp: Utc::now(),
        user: current_user(),
        reason: user_provided_reason(),
    });
}
```

### 3. 短寿命强力会话

最小化在强力模式下的时间：

```bash
# 良好：完成强力工作，然后退出
claw-agent --power-mode --task "install-dependencies"
# 自动退出

# 风险：长时间运行的强力模式智能体
claw-agent --power-mode --interactive  # 保持强力模式
```

### 4. 分离强力模式智能体

考虑为强力任务使用专用智能体：

```
agent-safe/          # 常规智能体，安全模式
├── 读取文件
└── 回答问题

agent-power/         # 管理智能体，强力模式
├── 安装软件
└── 修改系统
```

---

## 风险与缓解

| 风险 | 缓解措施 |
|------|----------|
| **提示注入** | 在强力操作前审查工具输出 |
| **意外删除** | 尽可能使用 `--dry-run` 标志 |
| **网络外泄** | 防火墙规则、网络监控 |
| **加密货币挖矿** | 资源限制、CPU 监控 |
| **持久化** | 定期审计 cron 任务、启动项 |

---

## 强力模式配置

### 最小示例

```rust
use claw_kernel::{
    provider::AnthropicProvider,
    loop_::AgentLoop,
    pal::{SandboxConfig, PowerKey},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = AnthropicProvider::from_env()?;
    
    // 强力模式
    let key = PowerKey::from_env()?
        .ok_or("需要 CLAW_KERNEL_POWER_KEY")?;
    
    let config = SandboxConfig::power_mode()
        .with_key(key)
        .build();
    
    let runtime = Runtime::with_sandbox(config)?;
    
    // 智能体现在可以做任何事
    let mut agent = AgentLoop::builder()
        .provider(provider)
        .runtime(runtime)
        .build();
    
    agent.run("安装并配置 nginx").await?;
    
    Ok(())
}
```

### 资源限制

即使在强力模式下，也应用资源约束：

```rust
let config = SandboxConfig::power_mode()
    .with_key(key)
    // 限制子进程
    .max_subprocesses(10)
    // 限制内存
    .max_memory_mb(2048)
    // 限制执行时间
    .max_execution_time(Duration::from_hours(1))
    .build();
```

---

## 故障排除

### "需要强力密钥"

你忘记了密钥：
```bash
# 错误
my-agent --power-mode

# 正确
my-agent --power-mode --power-key $(cat ~/.config/claw-kernel/power.key)
```

### "无法降级到安全模式"

这是设计如此。重启进程：
```bash
# 终止强力模式智能体
pkill my-agent

# 以安全模式重启
my-agent
```

### 强力模式下工具仍受限制

检查工具是否声明了权限：
```lua
-- 即使在强力模式下，此工具仍会检查权限
-- @permissions fs.read  <-- 必需！
```

强力模式绕过操作系统级限制，但除非另行配置，工具级权限检查可能仍然适用。

---

## 与容器对比

| 方案 | 隔离性 | 便利性 | 使用场景 |
|------|--------|--------|----------|
| **安全模式** | 强 | 简单 | 默认、不受信任的代码 |
| **强力模式** | 无 | 简单 | 受信任的自动化 |
| **Docker** | 强 | 中等 | 部署隔离 |
| **虚拟机** | 很强 | 困难 | 最大隔离 |

强力模式 + Docker：
```bash
# 在容器内以强力模式运行智能体
docker run --cap-add=ALL claw-agent --power-mode
# 容器内强力，容器与主机隔离
```

---

## 另请参阅

- [安全模式指南](safe-mode.md) — 受限执行
- [安全策略](../../SECURITY.md) — 完整安全模型
