"""
Agent spawn and steer example.

Demonstrates spawning a persistent agent, announcing capabilities,
steering it with a message, and then terminating it.
"""

import time

from claw_kernel import AgentConfig, KernelClient


def main():
    with KernelClient() as client:
        # Spawn a persistent agent
        result = client.agent.spawn(
            AgentConfig(
                system_prompt="You are a data analysis agent. Summarize data concisely.",
                provider="anthropic",
            )
        )
        agent_id = result["agent_id"]
        print(f"Agent spawned: {agent_id}")

        # Announce capabilities for discovery
        client.agent.announce(agent_id, capabilities=["summarize", "analyze_data"])
        print("Capabilities announced.")

        # Discover available agents
        agents = client.agent.discover()
        print(f"\nDiscovered {len(agents)} agent(s):")
        for a in agents:
            print(f"  - {a.get('agent_id')}: {a.get('capabilities', [])}")

        # List all agents
        agent_list = client.agent.list()
        print(f"\nAll agents ({len(agent_list)}):")
        for a in agent_list:
            print(f"  - {a.agent_id} [{a.status}]")

        # Steer the agent (inject a message)
        print(f"\nSteering agent {agent_id}...")
        client.agent.steer(
            agent_id, "Summarize: Q1 revenue $1.2M (+15% YoY), expenses $800K."
        )
        time.sleep(2)  # Give it a moment to process

        # Terminate
        client.agent.kill(agent_id)
        print(f"Agent {agent_id} killed.")


if __name__ == "__main__":
    main()
