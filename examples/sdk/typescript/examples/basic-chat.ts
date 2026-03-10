#!/usr/bin/env ts-node
/**
 * basic-chat.ts — 最简对话示例（10 行核心代码接入 claw-kernel）
 *
 * 运行：
 *   cd examples/sdk/typescript
 *   npm install
 *   npx ts-node examples/basic-chat.ts
 */

import { KernelClient } from '../src';

async function main() {
  // 1. 连接内核（自动鉴权，如未启动则自动拉起 daemon）
  const client = await KernelClient.connect();
  console.log('Connected to claw-kernel.');

  // 2. 打印内核信息
  const info = await client.info();
  console.log(`Kernel v${info.version}  provider=${info.active_provider}  model=${info.active_model}\n`);

  // 3. 创建会话
  const sessionId = await client.createSession('You are a helpful assistant.');
  console.log(`Session: ${sessionId}\n`);

  // 4. 流式发送消息
  const question = 'Hello! Introduce yourself briefly.';
  console.log(`User: ${question}`);
  process.stdout.write('Assistant: ');

  for await (const token of client.sendMessage(sessionId, question)) {
    process.stdout.write(token);
  }
  console.log('\n');

  // 5. 销毁会话 & 关闭连接
  await client.destroySession(sessionId);
  client.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
