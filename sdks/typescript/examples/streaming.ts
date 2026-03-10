import { KernelClient } from '../src';

async function main() {
  const client = await KernelClient.connect();
  const session = await client.createSession({
    systemPrompt: 'You are a storyteller. Keep stories short.',
  });

  process.stdout.write('Streaming response: ');

  for await (const chunk of session.stream('Tell me a one-sentence story about a robot.')) {
    if (chunk.type === 'delta' && chunk.delta) {
      process.stdout.write(chunk.delta);
    } else if (chunk.type === 'finish') {
      console.log(`\n[Finished: ${chunk.finishReason}]`);
    }
  }

  await session.close();
  client.close();
}

main().catch(console.error);
