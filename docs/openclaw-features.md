# OpenClaw 功能概览文档

> OpenClaw 是一款开源、自托管的 AI Agent 网关，前身为 Clawdbot/MoltBot。
> 它将 AI 模型连接到你已有的消息应用，让 Agent 真正地"做事"而非仅仅"对话"。

---

## 目录

1. [核心理念](#核心理念)
2. [消息渠道（Channels）](#消息渠道)
3. [记忆系统（Memory）](#记忆系统)
4. [技能系统（AgentSkills / ClawHub）](#技能系统)
5. [工具集（Tools）](#工具集)
6. [调度与自动化（Cron / Webhook）](#调度与自动化)
7. [浏览器控制（Browser）](#浏览器控制)
8. [语音交互（Voice）](#语音交互)
9. [实时画布（Live Canvas）](#实时画布)
10. [多 Agent 路由（Multi-Agent）](#多-agent-路由)
11. [模型无关性（LLM Agnostic）](#模型无关性)
12. [安全与隐私](#安全与隐私)
13. [架构概述](#架构概述)
14. [快速安装](#快速安装)
15. [真实用例（Community Use Cases）](#真实用例)
16. [参考资料](#参考资料)

---

## 核心理念

OpenClaw 将 AI 助手视为**基础设施问题**而非提示词工程问题。它为 LLM 提供了一套完整的操作系统：

- **结构化执行环境**：会话管理、沙盒化工具执行、消息路由
- **本地优先（Local-first）**：所有数据存储在本地磁盘，绝不离开你的服务器
- **文件即真相**：记忆、配置、技能全部以普通文本文件存储，可直接编辑

> "LLM 提供智能；OpenClaw 提供操作系统。"

---

## 消息渠道

OpenClaw 支持 **20+ 消息渠道**，构成统一的多渠道收件箱：

| 类别 | 支持渠道 |
|------|---------|
| 即时通讯 | WhatsApp、Telegram、Signal、Discord、Slack |
| 企业协作 | Microsoft Teams、Google Chat、Feishu（飞书）、Mattermost |
| 去中心化 | Matrix、Nostr、IRC |
| 苹果生态 | BlueBubbles (iMessage)、iMessage (legacy)、macOS 原生 |
| 亚洲平台 | LINE、Zalo、Zalo Personal、Synology Chat |
| 其他 | Nextcloud Talk、Tlon、Twitch、WebChat、iOS/Android |

所有渠道统一路由到同一个 Gateway 控制平面，支持跨渠道一致的 Agent 行为。

---

## 记忆系统

OpenClaw 采用**两文件记忆模型**，以纯 Markdown 文件作为唯一真相来源：

### 文件结构
```
memory/
├── MEMORY.md              # 长期事实与用户偏好
└── YYYY-MM-DD.md          # 每日日志（近期上下文）
```

### 限制参数
| 参数 | 默认值 |
|------|--------|
| 单文件最大字符 | 20,000 字符 |
| 全部 bootstrap 文件上限 | 150,000 字符（约 50K tokens） |

### 混合搜索引擎（Vector + BM25）
- **向量搜索**权重：70%（语义相关性）
- **BM25 关键词搜索**权重：30%（精确匹配）
- Embedding 提供商自动优先级：`本地模型 → OpenAI → Gemini → BM25 降级`
- 本地 Embedding 使用 `node-llama-cpp`，自动从 HuggingFace 下载 GGUF 模型
- OpenAI Batch API 集成，批量索引成本降低 50%

### 上下文安全
- 内置压缩保护（compaction）：上下文过大前自动保存到磁盘
- 开发者可直接编辑 Markdown 文件，无黑盒

### 社区扩展：12 层记忆架构
社区版本提供更高级的记忆系统：
- 知识图谱（3000+ 事实节点）
- 多语言语义搜索（GPU 7ms 响应）
- 激活/衰减机制
- 领域 RAG
- Agent 每次启动时从文件重建自身状态

---

## 技能系统

### AgentSkills 规范
每个技能是一个包含 `SKILL.md` 的目录（YAML frontmatter + 自然语言指令）：

```
skills/
└── my-skill/
    └── SKILL.md    # 技能元数据 + 使用说明
```

### 按需加载架构（On-Demand Loading）
OpenClaw **不会**将所有技能的全文注入系统提示词。它只注入一份紧凑的技能索引（名称、描述、路径），模型在需要时自主决定加载哪个技能的完整内容。

这保持了基础提示词的精简，无论安装了多少技能。

### 优先级顺序（高→低）
```
<workspace>/skills  →  ~/.openclaw/skills  →  内置技能
```

可通过 `skills.load.extraDirs` 配置额外技能目录。

### ClawHub 公共注册表
- 语义版本控制 + 变更日志 + 标签
- 向量索引：用语义搜索发现相关技能
- 预置 **100+ AgentSkills**，涵盖 Shell 命令、文件系统管理、Web 自动化等

> ⚠️ **安全警告**：技能可执行任意代码。Cisco 审计显示 31,000 个公开技能中 26% 存在漏洞。请像对待 npm 包一样审慎安装技能。

---

## 工具集

OpenClaw 内置 **25 种核心工具**（据社区教程统计），包括：

| 类别 | 工具 |
|------|------|
| 文件系统 | 读取、写入、搜索、编辑文件 |
| Shell | 执行 Shell 命令（沙盒隔离） |
| 浏览器 | CDP 控制 Chrome/Chromium |
| 会话管理 | 创建/切换/终止 Agent 会话 |
| 画布 | 渲染 Live Canvas（A2UI） |
| Cron | 创建/管理定时任务 |
| Discord/Slack | 平台原生操作 |
| Nodes | 伴侣 App 节点管理 |

---

## 调度与自动化

### Cron 定时任务
```json
{
  "cron": "0 9 * * *",
  "action": "send_message",
  "target": "main-session",
  "message": "早上好，今天有什么安排？"
}
```

### Webhook 触发器
- Gateway 对外暴露 HTTP 端点
- 外部系统（如 Gmail、GitHub、Zapier）可 POST 触发 Agent 动作
- 三种扩展类型：
  - **Skills**：自然语言驱动的 API 集成（SKILL.md）
  - **Plugins**：TypeScript/JavaScript 深度 Gateway 扩展
  - **Webhooks**：HTTP 端点，供外部系统回调

---

## 浏览器控制

OpenClaw 通过 CDP（Chrome DevTools Protocol）控制专用浏览器实例，**与个人浏览器完全隔离**：

### 三种配置模式
| 模式 | 说明 |
|------|------|
| `openclaw-managed` | 专属 Chromium 实例，独立用户数据目录 + CDP 端口 |
| `remote` | 指定远程 CDP URL |
| `extension-relay` | 通过 Chrome 扩展控制已有标签页 |

### 支持的操作
- **标签控制**：列出 / 打开 / 聚焦 / 关闭标签
- **Agent 动作**：点击、输入、拖拽、选择
- **内容提取**：快照、截图、PDF 导出
- **多 Profile**：同时管理多个独立浏览器配置

---

## 语音交互

| 平台 | 能力 |
|------|------|
| macOS / iOS | 唤醒词（Wake Word）触发 |
| Android | 持续语音监听（Continuous Voice） |
| TTS 引擎 | ElevenLabs（优先）→ 系统 TTS（降级） |

---

## 实时画布

**Live Canvas** 是 Agent 驱动的可视化工作区，使用 A2UI（Agent-to-UI）协议：

- Agent 可主动渲染 UI 组件到画布
- 用户可在画布上与 Agent 进行可视化交互
- 适用于复杂工作流的可视化编排

---

## 多 Agent 路由

Gateway 支持将不同的入站渠道/账户/对端路由到**相互隔离的 Agent 实例**（独立工作区 + 独立会话），实现：

- 工作与个人环境分离
- 多账号并行运行
- 按渠道定制 Agent 行为

---

## 模型无关性

OpenClaw 支持**自带 API Key**，兼容所有主流 LLM：

| 类型 | 支持模型/平台 |
|------|--------------|
| 云端 | Claude（Anthropic）、GPT（OpenAI）、DeepSeek |
| 自托管 | Ollama、local GGUF（llama.cpp） |

也支持完全离线运行，数据不出本地。

---

## 安全与隐私

### 设计原则
- **本地优先**：无需云端账户，所有数据存储在本地磁盘
- **沙盒隔离**：工具执行在受控环境中运行
- **Tailscale 集成**：通过 Serve/Funnel 安全暴露 Gateway 仪表板和 WebSocket

### 已知风险
- **权限过宽**：Agent 可访问邮件、日历、消息平台等敏感服务，配置不当有泄露风险
- **提示注入**：第三方技能存在提示注入和数据窃取风险（Cisco 已验证）
- **ClawHub 供应链**：2026 年 2 月首周即有 230+ 恶意技能上传

---

## 架构概述

```
┌─────────────────────────────────────────────────────┐
│                    Gateway（控制平面）                │
│                                                     │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │ Sessions │  │  Config  │  │  WebSocket + UI   │  │
│  └──────────┘  └──────────┘  └───────────────────┘  │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────┐  │
│  │  Crons   │  │ Webhooks │  │  Canvas Host      │  │
│  └──────────┘  └──────────┘  └───────────────────┘  │
└─────────────────────────────────────────────────────┘
         │               │               │
   ┌─────┴──┐     ┌──────┴───┐    ┌──────┴──────┐
   │Channels│     │  Memory  │    │   Skills    │
   │20+ 渠道│     │Markdown  │    │ ClawHub     │
   └────────┘     │+ Hybrid  │    │ AgentSkills │
                  │ Search   │    └─────────────┘
                  └──────────┘
```

**Hub-and-Spoke 架构**：单一 Gateway 作为控制枢纽，连接所有输入渠道（WhatsApp、Slack、CLI、Web UI）与 AI Agent。

---

## 快速安装

```bash
# 1. 全局安装
npm install -g openclaw@latest

# 2. 初始化守护进程
openclaw onboard --install-daemon

# 3. 登录渠道并启动 Gateway
openclaw channels login
openclaw gateway --port 18789
```

---

## 真实用例

> 来源：[awesome-openclaw-usecases](https://github.com/hesamsheikh/awesome-openclaw-usecases) — 社区真实场景合集，只收录经过验证的用法。

---

### 个人生产力

#### 第二大脑 / 记忆捕获
将 OpenClaw 接入 WhatsApp 或 Telegram，随手发送的想法、笔记、链接会被 Agent 分类写入本地 Markdown 文件，并建立向量索引，随时可通过自然语言检索。

**涉及功能**：Memory 系统、Webhook、消息渠道

---

#### 家庭中枢 / 晨间简报
每天早上自动汇总家庭日历、待办事项、天气、消息提醒，推送到 WhatsApp/Telegram。同时监控家庭群聊中的预约信息，自动更新日历。

**涉及功能**：Cron 定时任务、多渠道、日历集成

---

#### 手机语音/短信接入
通过语音电话或 SMS 访问 OpenClaw，无需打开 App：实时查询日历、Jira 工单、搜索网页，完全解放双手。

**涉及功能**：Voice（语音交互）、消息渠道（SMS）、浏览器控制

---

### 内容与媒体

#### 内容工厂（Content Factory）
全自动内容生产流水线，多个子 Agent 协作：
- **Research Agent**（`#research` 频道）：每天早上爬取热门话题、竞品内容、社交媒体趋势，推送 5 条内容机会
- **Writing Agent**：根据研究结果自动撰写文章
- **Thumbnail Agent**：生成配套封面图

**涉及功能**：多 Agent 路由、Cron、浏览器控制、Discord/Slack 渠道

---

#### 每日 Reddit 摘要
根据你关注的 subreddit 和偏好，每日自动抓取并总结精华内容，推送到你的消息 App。

**涉及功能**：Cron、浏览器控制、消息渠道

---

#### 每日 YouTube 摘要
订阅你关注的 YouTube 频道，Agent 每天自动获取新视频并生成摘要，再也不会错过重要内容。

**涉及功能**：Cron、浏览器控制、消息渠道

---

### 开发与工程

#### 夜间迷你 App 构建器（Overnight Mini-App Builder）
你只需用对话描述目标，Agent 在夜间自主完成：
1. 分解目标为可执行任务
2. 安排和调度任务
3. 自主编写代码，生成"惊喜" mini-app
4. 第二天早上推送成果

**涉及功能**：Cron、Shell 执行、文件系统、多 Agent

---

#### 自主游戏开发流水线（Autonomous Game Dev Pipeline）
定义"游戏开发 Agent"，自主管理游戏从创建到维护的完整生命周期：
- 强制执行"Bug 优先"策略：先修复已报告的 Bug，再实现新功能
- 自动管理代码库、测试、版本发布

**涉及功能**：Shell、文件系统、GitHub 集成、多 Agent、Memory

---

#### 会议记录 → 任务管理器
将会议录音或文字记录转化为结构化摘要，并自动在 Jira、Linear 或 Todoist 中创建任务，分配给对应负责人。

**涉及功能**：Webhook、第三方集成（Jira/Linear/Todoist）、Memory

---

### 基础设施与 DevOps

#### 自愈家庭服务器（Self-Healing Home Server）
OpenClaw 通过 SSH 持续监控你的服务器，在你察觉之前自动检测、诊断并修复问题：
- Cron 定期健康检查
- 异常自动触发修复脚本
- 修复结果推送通知

**涉及功能**：Cron、Shell（SSH）、Webhook、消息渠道

---

### 数据与分析

#### 动态仪表盘（Dynamic Dashboard）
用对话描述你想监控的数据（GitHub Stars、Twitter 提及、Polymarket 交易量、系统健康）：
- Agent 自动拆分并并行抓取各数据源
- 汇总结果，格式化后推送到 Discord 或生成 HTML 文件
- 定时自动刷新

**涉及功能**：多 Agent（并行子 Agent）、Cron、浏览器控制、Canvas、Discord

---

#### 预测市场模拟交易（Prediction Market Paper Trading）
自动化预测市场模拟交易系统：
- 策略回测与分析
- 每日绩效报告
- 自动执行模拟交易

**涉及功能**：Cron、浏览器控制、Memory、消息渠道

---

#### 记忆文件语义搜索
为 OpenClaw 的 Markdown 记忆文件增加向量语义搜索：
- 混合检索（语义 + 关键词）
- 自动同步索引
- 多语言支持

**涉及功能**：Memory 系统（混合搜索）、AgentSkills

---

### 团队协作

#### 多 Agent 团队（Multi-Agent Team）
在 Telegram 上建立多个专属领域的 Agent 协作团队：
- 每个 Agent 专注特定领域（研究、写作、代码、运营）
- 通过共享 Memory 文件传递上下文和状态
- 在 Telegram 群组中统一协调

**涉及功能**：多 Agent 路由、共享 Memory、Telegram 渠道

---

#### 去中心化自主项目管理（Autonomous Project Management）
子 Agent 通过**共享状态文件**而非中央调度器协调工作：
- 每个子 Agent 独立领取和完成任务
- 无需中心化 Orchestrator，降低单点故障风险
- 进度、状态写入共享 Markdown 文件，对所有 Agent 可见

**涉及功能**：多 Agent、文件系统（共享状态）、Memory

---

### 用例功能矩阵

| 用例 | Cron | 多Agent | 浏览器 | Memory | Shell | Voice | Webhook |
|------|:----:|:-------:|:------:|:------:|:-----:|:-----:|:-------:|
| 第二大脑 | | | | ✓ | | | ✓ |
| 晨间简报 | ✓ | | | ✓ | | | |
| 语音接入 | | | ✓ | | | ✓ | |
| 内容工厂 | ✓ | ✓ | ✓ | ✓ | | | |
| Reddit/YouTube 摘要 | ✓ | | ✓ | | | | |
| 夜间 App 构建器 | ✓ | ✓ | | ✓ | ✓ | | |
| 游戏开发流水线 | | ✓ | | ✓ | ✓ | | |
| 会议→任务管理 | | | | ✓ | | | ✓ |
| 自愈服务器 | ✓ | | | | ✓ | | ✓ |
| 动态仪表盘 | ✓ | ✓ | ✓ | | | | |
| 模拟交易 | ✓ | | ✓ | ✓ | | | |
| 多 Agent 团队 | | ✓ | | ✓ | | | |
| 项目管理 | | ✓ | | ✓ | | | |

---

## 参考资料

- [官方网站](https://openclaw.ai/)
- [官方文档](https://docs.openclaw.ai)
- [GitHub 仓库](https://github.com/openclaw/openclaw)
- [npm 包](https://www.npmjs.com/package/openclaw)
- [DigitalOcean 文档](https://docs.digitalocean.com/products/marketplace/catalog/openclaw/)
- [Skills 文档](https://docs.openclaw.ai/tools/skills)
- [Browser 控制文档](https://docs.openclaw.ai/tools/browser)
- [Integrations 列表](https://openclaw.ai/integrations)
- [内存架构深度解析 - Milvus Blog](https://milvus.io/blog/openclaw-formerly-clawdbot-moltbot-explained-a-complete-guide-to-the-autonomous-ai-agent.md)
- [记忆系统详解 - VelvetShark](https://velvetshark.com/openclaw-memory-masterclass)
- [架构解析 - ppaolo.substack.com](https://ppaolo.substack.com/p/openclaw-system-architecture-overview)
- [安全加固指南 - Nebius](https://nebius.com/blog/posts/openclaw-security)
- [MindStudio 介绍文章](https://www.mindstudio.ai/blog/what-is-openclaw-ai-agent/)
- [DigitalOcean 概述](https://www.digitalocean.com/resources/articles/what-is-openclaw)
