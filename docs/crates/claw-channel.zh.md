---
title: claw-channel
description: "渠道抽象层：ChannelMessage 协议与 Discord、HTTP Webhook、Stdin 平台适配器"
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-channel.md)

# claw-channel

平台无关的渠道抽象层，用于将 Agent 连接至外部消息系统。

---

## 概述

`claw-channel` 是第 2.5 层渠道子系统。它定义了统一的 `Channel` trait 和 `ChannelMessage` 类型，将 Agent 逻辑与具体平台 API 解耦。

## 组件

- **`Channel` trait**：面向所有平台的异步收发接口
- **`ChannelMessage`**：统一消息信封（文本、附件、元数据）
- **平台适配器**：
  - `DiscordChannel`：Discord 机器人集成
  - `WebhookChannel`：HTTP Webhook 推送/拉取
  - `StdinChannel`：CLI / 管道交互

## 使用方式

参见 [扩展能力指南](../guides/extension-capabilities.zh.md)。
