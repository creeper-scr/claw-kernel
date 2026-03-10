#!/usr/bin/env ts-node
/**
 * tool-registration.ts — 工具回调示例
 *
 * 展示如何将 TypeScript 函数暴露为 LLM 工具，并通过 AsyncGenerator 流式接收响应。
 *
 * 运行：
 *   cd sdk/typescript
 *   npm install
 *   npx ts-node examples/tool-registration.ts
 */

import { KernelClient, ExternalToolDef, ToolHandlerMap } from '../src';

// ─── 工具实现 ─────────────────────────────────────────────────────────────────

function getWeather(args: Record<string, unknown>): string {
  const city = String(args.city ?? '');
  const data: Record<string, string> = {
    Beijing: '晴天，18°C，微风',
    Shanghai: '多云，22°C，东南风',
    Shenzhen: '阴天，25°C，南风',
  };
  return data[city] ?? `${city} 天气数据暂不可用`;
}

function calculator(args: Record<string, unknown>): string {
  const expression = String(args.expression ?? '');
  // 仅允许数字和基本运算符
  if (!/^[\d+\-*/().\s]+$/.test(expression)) {
    return '错误：表达式包含非法字符';
  }
  try {
    // eslint-disable-next-line no-eval
    return String(eval(expression));
  } catch (err) {
    return `计算错误：${err}`;
  }
}

// ─── 工具 Schema（告知 LLM 如何调用）─────────────────────────────────────────

const TOOLS: ExternalToolDef[] = [
  {
    name: 'get_weather',
    description: '获取指定城市的当前天气信息',
    input_schema: {
      type: 'object',
      properties: {
        city: { type: 'string', description: '城市名称（中文或英文）' },
      },
      required: ['city'],
    },
  },
  {
    name: 'calculator',
    description: '计算数学表达式',
    input_schema: {
      type: 'object',
      properties: {
        expression: { type: 'string', description: "数学表达式，如 '2 + 3 * 4'" },
      },
      required: ['expression'],
    },
  },
];

// ─── 工具处理器映射 ────────────────────────────────────────────────────────────

const HANDLERS: ToolHandlerMap = {
  get_weather: getWeather,
  calculator,
};

// ─── 主程序 ────────────────────────────────────────────────────────────────────

async function main() {
  const client = await KernelClient.connect();
  console.log('Connected to claw-kernel.\n');

  const sessionId = await client.createSession(
    'You are a helpful assistant with access to weather and calculator tools.',
    { tools: TOOLS },
  );
  console.log(`Session: ${sessionId}\n`);

  const questions = [
    '北京今天天气怎么样？',
    '帮我算一下 (123 + 456) * 2 等于多少？',
  ];

  for (const question of questions) {
    console.log(`User: ${question}`);
    process.stdout.write('Assistant: ');

    for await (const token of client.sendMessage(sessionId, question, HANDLERS)) {
      process.stdout.write(token);
    }

    console.log('\n');
  }

  await client.destroySession(sessionId);
  client.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
