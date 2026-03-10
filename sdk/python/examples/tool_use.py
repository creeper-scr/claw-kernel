"""
Tool use example — register external tools and handle callbacks.

The assistant can call Python functions on the client side.
"""

import json
import math

from claw_kernel import KernelClient, SessionConfig, ToolDef


def calculate(expression: str) -> str:
    """Safely evaluate a simple math expression."""
    try:
        # Restrict to safe builtins.
        result = eval(expression, {"__builtins__": {}}, {"math": math})  # noqa: S307
        return str(result)
    except Exception as exc:
        return f"Error: {exc}"


def get_current_time() -> str:
    """Return the current UTC time."""
    from datetime import datetime, timezone

    return datetime.now(tz=timezone.utc).isoformat()


def main():
    tools_config = [
        ToolDef(
            name="calculate",
            description="Evaluate a mathematical expression",
            input_schema={
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "Math expression to evaluate (e.g. '2 + 2')",
                    }
                },
                "required": ["expression"],
            },
        ),
        ToolDef(
            name="get_current_time",
            description="Return the current UTC time in ISO 8601 format",
            input_schema={
                "type": "object",
                "properties": {},
            },
        ),
    ]

    tool_handlers = {
        "calculate": calculate,
        "get_current_time": get_current_time,
    }

    with KernelClient() as client:
        session_id = client.session.create(
            SessionConfig(
                system_prompt=(
                    "You are a helpful assistant with access to a calculator "
                    "and a clock. Use them when needed."
                ),
                tools=tools_config,
            )
        )

        prompt = "What is 123 * 456, and what time is it right now?"
        print(f"User: {prompt}")
        print("Assistant: ", end="", flush=True)

        response = client.session.send_collect(session_id, prompt, tools=tool_handlers)
        print(response)

        client.session.destroy(session_id)


if __name__ == "__main__":
    main()
