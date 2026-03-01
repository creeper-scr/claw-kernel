---
title: claw-memory
description: "记忆层：Ngram 嵌入器、SQLite 向量存储、带配额限制的 SecureMemoryStore"
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](claw-memory.md)

# claw-memory

Agent 内核的长期记忆层 —— 语义检索、持久存储与配额管理。

---

## 概述

`claw-memory` 提供 claw-kernel 的第 2 层记忆子系统，无需外部嵌入服务即可实现轻量级语义检索。

## 组件

- **NgramEmbedder**：基于字符级 bigram + trigram 的 64 维嵌入器
- **SqliteMemoryStore**：基于 SQLite 持久化、内存余弦相似度检索
- **SecureMemoryStore**：在 `SqliteMemoryStore` 基础上施加每 Agent 50 MB 配额限制

## 架构

```
Agent
  └── SecureMemoryStore（50 MB 配额）
        └── SqliteMemoryStore（余弦相似度，内存索引）
              └── NgramEmbedder（64 维 bigram+trigram）
                    └── SQLite（rusqlite + sqlite-vec）
```

## 使用方式

参见 [编写工具指南](../guides/writing-tools.zh.md)。
