"""
Async streaming example using AsyncKernelClient.

Run with: python async_streaming.py
"""

import asyncio

from claw_kernel import AsyncKernelClient, SessionConfig


async def main():
    async with AsyncKernelClient() as client:
        info = await client.info()
        print(f"Connected to claw-kernel v{info.version}")

        session_id = await client.session.create(
            SessionConfig(system_prompt="You are a creative storyteller.")
        )

        prompt = "Write a short 3-sentence story about a curious robot."
        print(f"\nUser: {prompt}")
        print("Assistant: ", end="", flush=True)

        async for token in client.session.send(session_id, prompt):
            print(token, end="", flush=True)

        print("\n")
        await client.session.destroy(session_id)
        print("Done.")


if __name__ == "__main__":
    asyncio.run(main())
