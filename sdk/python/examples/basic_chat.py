"""
Basic chat example using the synchronous KernelClient.

Run with: python basic_chat.py
"""

from claw_kernel import KernelClient, SessionConfig


def main():
    with KernelClient() as client:
        # Show kernel info
        info = client.info()
        print(f"Connected to claw-kernel v{info.version}")
        print(f"Active provider: {info.active_provider} / {info.active_model}")
        print()

        # Create a session
        session_id = client.session.create(
            SessionConfig(system_prompt="You are a friendly assistant.")
        )
        print(f"Session created: {session_id}")

        # Send messages in a loop
        while True:
            user_input = input("\nYou: ").strip()
            if user_input.lower() in ("exit", "quit", "q"):
                break

            print("Assistant: ", end="", flush=True)
            for token in client.session.send(session_id, user_input):
                print(token, end="", flush=True)
            print()

        # Clean up
        client.session.destroy(session_id)
        print("\nSession destroyed. Goodbye!")


if __name__ == "__main__":
    main()
