import { KernelClient } from '../src';

// Simulated in-process database
const db: Record<string, string> = {
  alice: 'Alice is a software engineer who specializes in distributed systems.',
  bob: 'Bob is a data scientist focused on machine learning research.',
};

async function main() {
  const client = await KernelClient.connect();

  const session = await client.createSession({
    systemPrompt: 'You have access to a people database. Use it to answer questions.',
    tools: [
      {
        name: 'lookup_person',
        description: 'Look up information about a person by name.',
        inputSchema: {
          type: 'object',
          properties: {
            name: {
              type: 'string',
              description: 'The person name to look up (lowercase)',
            },
          },
          required: ['name'],
        },
        execute: async (args) => {
          const name = (args['name'] as string).toLowerCase();
          return db[name] ?? `Person '${name}' not found.`;
        },
      },
    ],
  });

  console.log('Asking about Alice and Bob...\n');
  const response = await session.send('Tell me about Alice and Bob.');
  console.log('Response:', response);

  await session.close();
  client.close();
}

main().catch(console.error);
