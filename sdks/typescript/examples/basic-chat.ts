import { KernelClient } from '../src';

async function main() {
  console.log('Connecting to claw-kernel...');
  const client = await KernelClient.connect();

  const info = await client.info();
  console.log(`Connected! Kernel version: ${info.version}, protocol: v${info.protocolVersion}`);

  const session = await client.createSession({
    systemPrompt: 'You are a helpful assistant. Be concise.',
    maxTurns: 10,
  });

  console.log(`Session created: ${session.id}`);
  console.log('Sending message...\n');

  const response = await session.send('What is 2 + 2?');
  console.log('Response:', response);

  await session.close();
  client.close();
  console.log('\nDone!');
}

main().catch(console.error);
