"""
Channel registration and webhook trigger example.

Shows how to register a channel adapter and add a webhook trigger
that fires an agent when an HTTP request arrives.
"""

from claw_kernel import AgentConfig, ChannelConfig, KernelClient


def main():
    with KernelClient() as client:
        # Spawn an agent to handle incoming messages
        result = client.agent.spawn(
            AgentConfig(system_prompt="You are a webhook handler agent.")
        )
        agent_id = result["agent_id"]
        print(f"Agent spawned: {agent_id}")

        # Register a webhook channel
        channel = ChannelConfig(
            channel_type="webhook",
            channel_id="my-webhook",
            config={"description": "Main webhook endpoint"},
        )
        channel_result = client.channel.register(channel)
        print(f"Channel registered: {channel_result}")

        # Add a routing rule: messages on this channel → our agent
        client.channel.route_add(
            agent_id=agent_id,
            rule_type="channel",
            channel_id="my-webhook",
        )
        print("Routing rule added.")

        # Add a webhook trigger
        trigger_result = client.trigger.add_webhook(
            trigger_id="webhook-trigger-1",
            target_agent=agent_id,
            hmac_secret="super-secret-key",
        )
        print(f"Webhook trigger added: {trigger_result}")

        # List triggers
        triggers = client.trigger.list()
        print(f"\nActive triggers ({len(triggers)}):")
        for t in triggers:
            print(f"  - {t}")

        # List channels
        channels = client.channel.list()
        print(f"\nRegistered channels ({len(channels)}):")
        for ch in channels:
            print(f"  - {ch}")

        # Simulate an inbound message
        print("\nSimulating inbound message...")
        response = client.channel.inbound(
            channel_id="my-webhook",
            sender_id="user-001",
            content="Hello from webhook!",
            thread_id="thread-001",
        )
        print(f"Inbound result: {response}")

        # Clean up
        client.trigger.remove("webhook-trigger-1")
        client.channel.unregister("my-webhook")
        client.agent.kill(agent_id)
        print("\nCleanup complete.")


if __name__ == "__main__":
    main()
