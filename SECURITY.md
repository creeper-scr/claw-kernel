[English](#english) | [中文](#chinese)

<a name="english"></a>

# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| latest stable | ✅ |
| previous minor | ✅ security fixes only |
| older | ❌ |

---

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Send a private report to: **security@claw-project.dev**

Include in your report:
- Affected crate(s) and version(s)
- Description of the vulnerability
- Steps to reproduce
- Potential impact assessment
- (Optional) Suggested fix or mitigation

You will receive an acknowledgment within **48 hours** and a detailed response within **7 days**.

---

## Security Model

claw-kernel has two distinct security postures. Understanding the model is critical before reporting.

### Safe Mode (default)

Safe Mode is the default execution environment. The following are **security guarantees** in Safe Mode — violations are valid vulnerabilities:

- Scripts cannot access files outside whitelisted directories
- Scripts cannot spawn arbitrary subprocesses
- Scripts cannot make network requests to non-allowlisted endpoints
- Scripts cannot escalate to Power Mode without the correct credential
- The Rust hard core (Layer 0) cannot be modified by scripts
- Kernel secret storage is inaccessible to scripts

### Power Mode (opt-in)

Power Mode is an **intentionally unrestricted** mode that grants the agent full system access. It requires explicit user activation (`--power-mode`).

**By design, Power Mode removes most restrictions.** The following are NOT vulnerabilities in Power Mode:
- Full filesystem access
- Arbitrary subprocess execution
- Unrestricted network access
- Script modification of tool definitions

Vulnerabilities in Power Mode that ARE valid:
- Unauthorized escalation from Safe Mode to Power Mode
- Credential/key exposure that enables undeclared Power Mode access
- Power Mode activation without user confirmation

### Out of scope

The following are **not in scope** for this security policy:
- Agents tricked into harmful actions via prompt injection (mitigate at the application layer)
- Denial-of-service via resource exhaustion in Power Mode
- Vulnerabilities in third-party LLM providers or their APIs

---

## Dependency Auditing

We run `cargo audit` on every CI build. You can run it locally:

```bash
cargo install cargo-audit
cargo audit
```

---

## Disclosure Policy

Once a vulnerability is confirmed and patched:
1. We will publish a security advisory on GitHub
2. A patched release will be published to crates.io
3. The reporter will be credited (unless they prefer anonymity)
4. CVE will be requested for high-severity issues

---

<a name="chinese"></a>

# 安全策略

## 支持的版本

| 版本 | 支持状态 |
|---------|-----------|
| 最新稳定版 | ✅ |
| 上一个次要版本 | ✅ 仅安全修复 |
| 更旧的版本 | ❌ |

---

## 报告漏洞

**请不要通过公开的 GitHub issues 报告安全漏洞。**

请发送私密报告至：**security@claw-project.dev**

报告内容请包含：
- 受影响的 crate(s) 和版本
- 漏洞描述
- 复现步骤
- 潜在影响评估
- （可选）建议的修复或缓解措施

您将在 **48 小时内**收到确认，在 **7 天内**收到详细回复。

---

## 安全模型

claw-kernel 有两种不同的安全姿态。在报告之前，理解此模型至关重要。

### 安全模式（默认）

安全模式是默认的执行环境。以下是安全模式中的**安全保证**——违反这些保证属于有效漏洞：

- 脚本无法访问允许列表目录之外的文件
- 脚本无法生成任意子进程
- 脚本无法向非允许列表端点发起网络请求
- 脚本无法在没有正确凭证的情况下升级到强力模式
- Rust 硬核（第 0 层）无法被脚本修改
- 内核密钥存储对脚本不可访问

### 强力模式（可选）

强力模式是一种**故意不受限制**的模式，授予代理完整的系统访问权限。它需要用户显式激活（`--power-mode`）。

**根据设计，强力模式会移除大部分限制。** 以下情况在强力模式中**不属于**漏洞：
- 完整的文件系统访问
- 任意子进程执行
- 无限制的网络访问
- 脚本修改工具定义

在强力模式中**属于**有效漏洞的情况：
- 从安全模式未经授权升级到强力模式
- 导致未声明的强力模式访问的凭证/密钥泄露
- 未经用户确认的强力模式激活

### 范围之外

以下内容**不在**本安全策略的范围内：
- 通过提示注入被诱骗执行有害行为的代理（在应用层缓解）
- 强力模式中的资源耗尽导致的服务拒绝
- 第三方 LLM 提供商或其 API 中的漏洞

---

## 依赖审计

我们在每次 CI 构建时运行 `cargo audit`。您可以在本地运行：

```bash
cargo install cargo-audit
cargo audit
```

---

## 披露策略

一旦漏洞被确认并修复：
1. 我们将在 GitHub 上发布安全公告
2. 修复版本将发布到 crates.io
3. 报告者将被致谢（除非他们希望匿名）
4. 对于高严重性问题，将申请 CVE
