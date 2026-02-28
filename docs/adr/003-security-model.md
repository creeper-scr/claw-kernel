[English](#english) | [中文](#chinese)

<a name="english"></a>
# ADR 003: Dual-Mode Security (Safe/Power)

**Status:** Accepted  
**Date:** 2024-01-25  
**Deciders:** claw-kernel core team, security review

---

## Context

Agents have conflicting security requirements:

1. **Default use case:** Execute LLM-generated code safely
   - Should not delete random files
   - Should not exfiltrate data
   - Should be deployable to shared environments

2. **Power use case:** Full system automation
   - Install software
   - Manage system services
   - Modify system configuration

We need a clear security model that addresses both.

---

## Decision

Implement **two explicit execution modes**:

| Aspect | Safe Mode (Default) | Power Mode (Opt-in) |
|--------|---------------------|---------------------|
| **Filesystem** | Allowlist read-only | Full access |
| **Network** | Domain/port rules | Unrestricted |
| **Subprocess** | Blocked | Allowed |
| **Self-modification** | Allowed (sandboxed) | Allowed (global) |
| **Activation** | Default | `--power-mode --power-key <key>` |

### Key Design Principles

**1. Explicit Opt-in Required**

Power Mode requires BOTH:
- `--power-mode` flag (explicit intent)
- `--power-key <key>` (authentication)

**2. No Downgrade Without Restart**

Power Mode → Safe Mode requires process restart. This prevents:
- Compromised Power Mode agent hiding evidence
- Race conditions in mode switching

**3. Kernel Immutable in Both Modes**

Rust Hard Core (Layer 0) is untouchable regardless of mode:
- No script can modify kernel code
- No script can access kernel credential storage
- No script can bypass sandbox enforcement

### Mode Switching Flow

```
┌─────────────┐      --power-mode + --power-key      ┌─────────────┐
│  Safe Mode  │  ─────────────────────────────────►  │  Power Mode │
│  (default)  │                                     │  (opt-in)   │
└─────────────┘                                     └─────────────┘
       ▲                                                    │
       │              restart or new process                 │
       └─────────────────────────────────────────────────────┘
```

---

## Consequences

### Positive

- **Clear mental model:** Users understand the trade-off
- **Safe by default:** No accidental full system access
- **Audit trail:** Mode switches are logged
- **Deployable:** Safe Mode suitable for shared/cloud environments

### Negative

- **UX friction:** Power Mode requires key management
- **Implementation complexity:** Two sandbox code paths

### Security Boundaries

**Safe Mode Guarantees (violations are bugs):**
- Scripts cannot access files outside allowlist
- Scripts cannot spawn subprocesses
- Scripts cannot make network calls outside rules
- Scripts cannot escalate to Power Mode without key
- Kernel secrets remain inaccessible

**Power Mode Guarantees:**
- Full system access BY DESIGN
- Only protection: unauthorized activation is blocked

---

## Alternatives Considered

### Alternative 1: Single Mode with Permission Prompts

**Rejected:** UX nightmare, prompts become muscle memory

### Alternative 2: Capability System (like Android)

**Rejected:** Too complex for CLI tools, overkill for our use case

### Alternative 3: Container/Docker Isolation

**Considered:** Excellent isolation, but:
- Requires Docker (not always available)
- Startup latency
- Complex volume mounting for file access

**Decision:** Use as implementation detail for sandboxing, not primary interface

---

## Implementation

### Power Key Management

**Design Decision**: Power Key is user-defined (not system-generated)

```rust
pub struct PowerKey {
    // Derived from user-provided key via Argon2
    verification_hash: [u8; 32],
}

impl PowerKey {
    pub fn verify(provided: &str) -> bool {
        let hash = argon2::hash_raw(provided.as_bytes(), SALT, PARAMS)?;
        constant_time_eq(&hash, &self.verification_hash)
    }
}
```

Key setup (user-defined):
```bash
# User sets their own power key
claw-kernel --set-power-key
Enter new power key: ********
Confirm power key: ********
Power key set successfully.
```

Key storage:
- Interactive: Prompt for key on `--power-mode`
- Config file: `~/.config/claw-kernel/power.key` (600 permissions, stores hash only)
- Environment: `CLAW_KERNEL_POWER_KEY` (not recommended for regular use)

**Security Note**: If power key is forgotten, user must reset via `--reset-power-key` (requires manual confirmation).

### Sandbox Configuration

```rust
pub struct SandboxConfig {
    pub mode: ExecutionMode,
    pub filesystem_allowlist: Vec<PathBuf>,
    pub network_rules: Vec<NetRule>,
    pub allow_subprocess: bool,
}

impl SandboxConfig {
    pub fn safe_default() -> Self {
        Self {
            mode: ExecutionMode::Safe,
            filesystem_allowlist: vec![
                dirs::data_dir().unwrap(),
                dirs::cache_dir().unwrap(),
            ],
            network_rules: vec![NetRule::Allow { 
                domains: vec!["api.openai.com", "api.anthropic.com"],
                ports: vec![443],
            }],
            allow_subprocess: false,
        }
    }
    
    pub fn power_mode() -> Self {
        Self {
            mode: ExecutionMode::Power,
            filesystem_allowlist: vec![],  // No restriction
            network_rules: vec![NetRule::AllowAll],
            allow_subprocess: true,
        }
    }
}
```

---

## Security Audit Checklist

Before release:

- [ ] Safe Mode sandbox escape attempts
- [ ] Power Mode key brute force resistance
- [ ] Credential storage encryption
- [ ] Mode transition race conditions
- [ ] Audit log completeness

---

## References

- [Security Policy](../../SECURITY.md)
- [Safe Mode Guide](../guides/safe-mode.md)
- [Power Mode Guide](../guides/power-mode.md)
- [Platform Abstraction Layer](../architecture/pal.md) (sandbox implementations)

---

<a name="chinese"></a>
# ADR 003: 双模式安全（安全/强力）

**状态：** 已接受  
**日期：** 2024-01-25  
**决策者：** claw-kernel 核心团队，安全审查

---

## 背景

智能体有相互冲突的安全要求：

1. **默认用例：** 安全执行 LLM 生成的代码
   - 不应删除随机文件
   - 不应外泄数据
   - 应可部署到共享环境

2. **强力用例：** 完整系统自动化
   - 安装软件
   - 管理系统服务
   - 修改系统配置

我们需要一个能解决两者的清晰安全模型。

---

## 决策

实现**两种明确的执行模式**：

| 方面 | 安全模式（默认） | 强力模式（可选） |
|------|------------------|------------------|
| **文件系统** | 允许列表只读 | 完全访问 |
| **网络** | 域名/端口规则 | 无限制 |
| **子进程** | 阻止 | 允许 |
| **自我修改** | 允许（沙箱化） | 允许（全局） |
| **激活方式** | 默认 | `--power-mode --power-key <key>` |

### 关键设计原则

**1. 需要明确选择加入**

强力模式需要同时满足：
- `--power-mode` 标志（明确意图）
- `--power-key <key>`（身份验证）

**2. 无重启无法降级**

强力模式 → 安全模式需要进程重启。这可以防止：
- 受损的强力模式智能体隐藏证据
- 模式切换的竞争条件

**3. 两种模式下内核都不可变**

无论哪种模式，Rust 硬核核心（第 0 层）都不可触碰：
- 没有脚本可以修改内核代码
- 没有脚本可以访问内核凭证存储
- 没有脚本可以绕过沙箱执行

### 模式切换流程

```
┌─────────────┐      --power-mode + --power-key      ┌─────────────┐
│   安全模式   │  ─────────────────────────────────►  │   强力模式   │
│   （默认）   │                                      │   （可选）   │
└─────────────┘                                      └─────────────┘
       ▲                                                     │
       │               重启或新进程                            │
       └──────────────────────────────────────────────────────┘
```

---

## 后果

### 积极方面

- **清晰的心理模型：** 用户理解权衡
- **默认安全：** 不会意外获得完整系统访问权限
- **审计跟踪：** 模式切换被记录
- **可部署：** 安全模式适合共享/云环境

### 消极方面

- **用户体验摩擦：** 强力模式需要密钥管理
- **实现复杂性：** 两种沙箱代码路径

### 安全边界

**安全模式保证（违反是 bug）：**
- 脚本无法访问允许列表外的文件
- 脚本无法生成子进程
- 脚本无法在规则外进行网络调用
- 脚本无法在没有密钥的情况下升级到强力模式
- 内核密钥保持不可访问

**强力模式保证：**
- 按设计获得完整系统访问权限
- 唯一保护：阻止未授权激活

---

## 考虑的替代方案

### 替代方案 1：带权限提示的单一模式

**已拒绝：** 用户体验噩梦，提示变成肌肉记忆

### 替代方案 2：能力系统（如 Android）

**已拒绝：** 对 CLI 工具太复杂，对我们的用例过度设计

### 替代方案 3：容器/Docker 隔离

**已考虑：** 优秀的隔离，但是：
- 需要 Docker（并非总是可用）
- 启动延迟
- 文件访问的卷挂载复杂

**决策：** 作为沙箱化的实现细节使用，不作为主要接口

---

## 实现

### 强力密钥管理

**设计决策**：Power Key 由用户自定义（非系统生成）

```rust
pub struct PowerKey {
    // 通过 Argon2 从用户提供的密钥派生
    verification_hash: [u8; 32],
}

impl PowerKey {
    pub fn verify(provided: &str) -> bool {
        let hash = argon2::hash_raw(provided.as_bytes(), SALT, PARAMS)?;
        constant_time_eq(&hash, &self.verification_hash)
    }
}
```

密钥设置（用户自定义）：
```bash
# 用户设置自己的 power key
claw-kernel --set-power-key
Enter new power key（最少 8 位字符）: ********
Confirm power key: ********
Power key set successfully.
```

**要求**：
- 最小长度：**8 位字符**
- 用户自定义（非系统生成）
- 以 Argon2 哈希存储（非明文）

密钥存储：
- 交互式：在 `--power-mode` 时提示输入密钥
- 配置文件：`~/.config/claw-kernel/power.key`（600 权限，仅存储哈希值）
- 环境变量：`CLAW_KERNEL_POWER_KEY`（不推荐常规使用）

**安全提示**：如果遗忘 power key，用户必须通过 `--reset-power-key` 重置（需要手动确认）。

### 沙箱配置

```rust
pub struct SandboxConfig {
    pub mode: ExecutionMode,
    pub filesystem_allowlist: Vec<PathBuf>,
    pub network_rules: Vec<NetRule>,
    pub allow_subprocess: bool,
}

impl SandboxConfig {
    pub fn safe_default() -> Self {
        Self {
            mode: ExecutionMode::Safe,
            filesystem_allowlist: vec![
                dirs::data_dir().unwrap(),
                dirs::cache_dir().unwrap(),
            ],
            network_rules: vec![NetRule::Allow { 
                domains: vec!["api.openai.com", "api.anthropic.com"],
                ports: vec![443],
            }],
            allow_subprocess: false,
        }
    }
    
    pub fn power_mode() -> Self {
        Self {
            mode: ExecutionMode::Power,
            filesystem_allowlist: vec![],  // 无限制
            network_rules: vec![NetRule::AllowAll],
            allow_subprocess: true,
        }
    }
}
```

---

## 安全审计清单

发布前：

- [ ] 安全模式沙箱逃逸尝试
- [ ] 强力模式密钥暴力破解抵抗
- [ ] 凭证存储加密
- [ ] 模式切换竞争条件
- [ ] 审计日志完整性

---

## 参考

- [安全政策](../../SECURITY.md)
- [安全模式指南](../guides/safe-mode.md)
- [强力模式指南](../guides/power-mode.md)
- [平台抽象层](../architecture/pal.md)（沙箱实现）
