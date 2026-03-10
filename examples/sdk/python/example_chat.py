#!/usr/bin/env python3
"""最简对话示例 — 10 行代码接入 claw-kernel。"""
from kernel_client import KernelClient

with KernelClient() as kernel:
    session = kernel.create_session("You are a helpful assistant.")
    print(f"Session: {session}")

    for token in kernel.send_message(session, "Hello! What can you do?"):
        print(token, end="", flush=True)
    print()

    kernel.destroy_session(session)
