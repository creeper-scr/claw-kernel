---
title: claw-channel
description: "Channel abstraction layer: ChannelMessage protocol, platform adapters for Discord, HTTP Webhook, and Stdin"
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-channel.zh.md)

# claw-channel

Platform-agnostic channel abstraction for connecting agents to external messaging systems.

---

## Overview

`claw-channel` is the Layer 2.5 channel subsystem. It defines a unified `Channel` trait and `ChannelMessage` type that decouple agent logic from specific platform APIs.

## Components

- **`Channel` trait**: Async send/receive interface for all platforms
- **`ChannelMessage`**: Unified message envelope (text, attachments, metadata)
- **Platform adapters**:
  - `DiscordChannel`: Discord bot integration
  - `WebhookChannel`: HTTP webhook push/pull
  - `StdinChannel`: CLI / pipe-based interaction

## Usage

See [Extension Capabilities](../guides/extension-capabilities.md) for platform configuration details.
