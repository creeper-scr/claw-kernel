---
title: Agent Loop State Machine
description: FSM-driven execution model for AgentLoop
---

# Agent Loop State Machine

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

`AgentLoop` uses a finite state machine to manage execution lifecycle.

## States

```rust
pub enum AgentState {
    Idle,           // Initial state
    Running,        // Active execution
    AwaitingLLM,    // Waiting for LLM response
    ToolExecuting,  // Executing tool calls
    Completed,      // Finished successfully
    Error,          // Error occurred
}
```

## State Transitions

```
Idle ──Start──► Running ──LLMRequestSent──► AwaitingLLM
                                                │
                      ┌─────────────────────────┘
                      │ LLMResponseReceived (no tools)
                      ▼
                ToolExecuting ◄──ToolsRequired──┤
                      │                           │
                      └──ToolsCompleted───────────┤
                      ▼                           │
                Completed ◄──StopConditionMet─────┘
                      ▲
                      └──────Error────────────────┘
```

## Extension Point: State Subscription

Applications can observe state changes:

```rust
let mut agent = AgentLoop::builder()
    .provider(provider)
    .build()?;

let mut state_rx = agent.subscribe_state();

tokio::spawn(async move {
    while let Ok(state) = state_rx.recv().await {
        println!("Agent state: {:?}", state);
        // Application can react: update UI, log metrics, etc.
    }
});
```

> **Note**: State machine is a mechanism. What you do on state change is your policy.
