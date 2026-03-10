---
title: Security Policy
description: Vulnerability reporting and security model for claw-kernel
status: active
version: "1.0.0"
last_updated: "2026-02-28"
language: bilingual
---

> **⚠️ Security Warning / 安全警告**
> 
> The current sandbox and security implementations are **incomplete and not fully audited**. 
> 当前的沙箱和安全实现**尚不完善，未经充分审计**。
>
> - Windows sandbox is partially implemented (Job Objects only, AppContainer pending v1.5.0)
> - macOS sandbox has limited syscall filtering capabilities
> - Some security boundaries may not be fully enforced
>
> **We welcome security contributions!** If you have expertise in sandboxing, secure coding, or 
> platform security, please consider contributing. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.
> **欢迎安全方面的贡献！**如果您在沙箱技术、安全编码或平台安全方面有专业知识，请考虑贡献。

> **Project Status**: Active. v1.0.0 released. Security model is implemented with known limitations (see KNOWN-ISSUES.md).

[English](#english) | [中文](#chinese)

<a name="english"></a>

# Security Policy

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.
## Supported Versions

| Version | Supported | Notes |
|---------|-----------|-------|
| latest stable (x.y.z) | Yes | Full support: features and security fixes |
| previous minor (x.{y-1}.*) | Yes | Security fixes only, for 6 months after new release |
| older (< x.{y-1}) | No | Please upgrade |

---

## Reporting a Vulnerability

**Do NOT use public GitHub Issues to report security vulnerabilities.** Public disclosure before a fix is available puts all users at risk.

Send a private report to: **security@claw-project.dev**

Include in your report:
- Affected crate(s) and version(s)
- Description of the vulnerability
- Steps to reproduce
- Potential impact assessment
- (Optional) Suggested fix or mitigation

**Response timeline:**
- Acknowledgment within **48 hours** (UTC)
- Detailed response within **7 days**
- Fix target within **90 days** for confirmed vulnerabilities
- We will request a CVE for confirmed vulnerabilities with CVSS v3.1 score >= 7.0, or any sandbox escape from Safe Mode

If you don't hear back within 48 hours, follow up by opening a GitHub Issue with the title "Security contact needed" (no vulnerability details) to prompt a response.

---

## Security Model

claw-kernel has two distinct security postures. Understanding the model is critical before reporting.

### Safe Mode (default)

Safe Mode is the default execution environment. These are **security guarantees** in Safe Mode. Violations are valid vulnerabilities:

- Scripts cannot access files outside allowlisted directories
- Scripts cannot spawn arbitrary subprocesses
- Scripts cannot make network requests to non-allowlisted endpoints
- Scripts cannot escalate to Power Mode without the correct credential
- The Rust hard core (Layer 0) cannot be modified by scripts
- Kernel secret storage is inaccessible to scripts

### Power Mode (opt-in)

Power Mode grants the agent full system access. It requires explicit user activation (`--power-mode --power-key <key>`).

**By design, Power Mode removes most restrictions.** The following are NOT vulnerabilities in Power Mode:
- Full filesystem access
- Arbitrary subprocess execution
- Unrestricted network access
- Script modification of tool definitions

Vulnerabilities in Power Mode that ARE valid:
- Unauthorized escalation from Safe Mode to Power Mode
- Credential/key exposure enabling undeclared Power Mode access
- Power Mode activation without user confirmation

### Out of Scope

- Agents tricked into harmful actions via prompt injection (mitigate at the application layer)
- Denial-of-service via resource exhaustion in Power Mode
- Vulnerabilities in third-party LLM providers or their APIs

---

## IPC Trust Model

claw-kernel's inter-process communication relies on **OS-level socket file permissions** for access control.

### Current Model
- The IPC Unix domain socket file is created with `0700` permissions (owner read/write/execute only)
- Only processes running under the **same Unix user ID** can connect to the socket
- All processes that successfully connect are implicitly trusted — there is no per-message authentication or signing

### Known Limitation
Messages routed through the IPC layer carry no cryptographic signature. A malicious process running as the same OS user could:
- Connect to the socket and impersonate any `AgentId`
- Inject arbitrary `A2AMessage` payloads

This is an accepted risk for the v1.0 single-user deployment model. Multi-tenant environments or deployments where processes from different security contexts share the same Unix user should not use claw-kernel without additional network-level isolation.

### Roadmap
Per-agent HMAC token authentication is planned for a future release. See [ROADMAP.md](ROADMAP.md) for details.

---

## Disclosure Policy

Once a vulnerability is confirmed and patched:
1. We publish a security advisory on GitHub
2. A patched release is published to crates.io
3. The reporter is credited in the advisory and the Hall of Fame below (unless they prefer anonymity)
4. We request a CVE for high-severity issues (CVSS v3.1 >= 7.0, or any Safe Mode sandbox escape)

---

## Dependency Auditing

We run `cargo audit` on every CI build. Run it locally:

```bash
cargo install cargo-audit
cargo audit
```

---

## Hall of Fame

We thank the following researchers for responsible disclosure:

*No entries yet. Be the first.*

---

<a name="chinese"></a>

# 安全策略

## 支持的版本

| 版本 | 支持状态 | 说明 |
|---------|-----------|-------|
| 最新稳定版 (x.y.z) | 是 | 完全支持：功能更新和安全修复 |
| 上一个小版本 (x.{y-1}.*) | 是 | 仅安全修复，新版本发布后 6 个月内 |
| 更早版本 (< x.{y-1}) | 否 | 请升级 |

---

## 报告漏洞

**请不要通过公开的 GitHub Issues 报告安全漏洞。** 在修复发布前公开漏洞会让所有用户面临风险。

请发送私密报告至：**security@claw-project.dev**

报告内容请包含：
- 受影响的 crate(s) 和版本
- 漏洞描述
- 复现步骤
- 潜在影响评估
- （可选）建议的修复或缓解措施

**响应时间表：**
- **48 小时内**确认收到报告（UTC）
- **7 天内**提供详细回复
- 确认漏洞的修复目标为 **90 天内**
- 对于 CVSS v3.1 评分 >= 7.0 或任何安全模式沙箱逃逸的确认漏洞，我们将申请 CVE

如果 48 小时内没有收到回复，请开启一个标题为“Security contact needed”的 GitHub Issue（不要包含漏洞详情）以提醒我们回复。

---

## 安全模型

claw-kernel 有两种不同的安全姿态。在报告之前理解此模型至关重要。

### 安全模式（默认）

安全模式是默认执行环境。以下是安全模式中的**安全保证**，违反这些保证属于有效漏洞：

- 脚本无法访问允许列表目录之外的文件
- 脚本无法生成任意子进程
- 脚本无法向非允许列表端点发起网络请求
- 脚本无法在没有正确凭证的情况下升级到强力模式
- Rust 硬核（第 0 层）无法被脚本修改
- 内核密钥存储对脚本不可访问

### 强力模式（可选）

强力模式授予代理完整的系统访问权限，需要用户显式激活（`--power-mode --power-key <key>`）。

**根据设计，强力模式会移除大部分限制。** 以下情况在强力模式中不属于漏洞：
- 完整的文件系统访问
- 任意子进程执行
- 无限制的网络访问
- 脚本修改工具定义

强力模式中属于有效漏洞的情况：
- 从安全模式未经授权升级到强力模式
- 导致未声明强力模式访问的凭证/密钥泄露
- 未经用户确认的强力模式激活

### 范围之外

- 通过提示注入误导代理执行有害行为（在应用层缓解）
- 强力模式中的资源耗尽导致的拒绝服务
- 第三方 LLM 提供商或其 API 中的漏洞

---

## IPC 信任模型

claw-kernel 的进程间通信依赖**操作系统级别的套接字文件权限**进行访问控制。

### 当前模型
- IPC Unix 域套接字文件以 `0700` 权限创建（仅所有者可读/写/执行）
- 只有以**相同 Unix 用户 ID** 运行的进程才能连接到套接字
- 所有成功连接的进程都被隐式信任——不存在逐消息的身份验证或签名

### 已知限制
通过 IPC 层路由的消息不携带任何加密签名。以相同 OS 用户身份运行的恶意进程可能：
- 连接到套接字并冒充任何 `AgentId`
- 注入任意 `A2AMessage` 载荷

对于 v1.0 单用户部署模型，这是一个已接受的风险。多租户环境，或不同安全上下文的进程共享同一 Unix 用户的部署，在没有额外网络层隔离的情况下不应使用 claw-kernel。

### 路线图
每个 Agent 的 HMAC 令牌认证计划在未来版本中实现。详情参见 [ROADMAP.md](ROADMAP.md)。

---

## 披露策略

一旦漏洞被确认并修复：
1. 我们在 GitHub 上发布安全公告
2. 修复版本发布到 crates.io
3. 报告者将在公告和下方荣誉榜中获得致谢（除非希望匿名）
4. 对于高严重性问题（CVSS v3.1 >= 7.0 或任何安全模式沙箱逃逸），将申请 CVE

---

## 依赖审计

我们在每次 CI 构建时运行 `cargo audit`。本地运行：

```bash
cargo install cargo-audit
cargo audit
```

---

## 荣誉榜

感谢以下研究人员负责任地披露漏洞：

*暂无记录。期待您成为第一个。*
