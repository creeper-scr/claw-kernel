//! Simple Agent Example
//!
//! This example demonstrates the basic usage of claw-kernel components:
//! - OllamaProvider: Connect to local Ollama instance (no API key needed)
//! - ToolRegistry: Register and execute custom tools
//! - AgentLoopBuilder: Configure and build the agent loop
//! - InMemoryHistory: Store conversation history in memory
//! - MaxTurns: Limit the number of conversation turns

use std::sync::Arc;

use claw_kernel::prelude::*;
use claw_kernel::provider::OllamaProvider;
use claw_kernel::tools::{Tool, ToolContext, ToolResult, ToolSchema, PermissionSet};
use claw_kernel::agent_loop::MaxTurns;
use async_trait::async_trait;

/// A simple calculator tool that adds two numbers.
struct CalculatorTool {
    schema: ToolSchema,
    perms: PermissionSet,
}

impl CalculatorTool {
    fn new() -> Self {
        let schema = ToolSchema::new(
            "calculator",
            "Add two numbers together",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "a": {
                        "type": "number",
                        "description": "First number"
                    },
                    "b": {
                        "type": "number",
                        "description": "Second number"
                    }
                },
                "required": ["a", "b"]
            }),
        );
        
        Self {
            schema,
            perms: PermissionSet::minimal(),
        }
    }
}

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }

    fn description(&self) -> &str {
        "Add two numbers together"
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.perms
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Parse the arguments
        let a = args.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = args.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        
        let result = a + b;
        
        ToolResult::ok(
            serde_json::json!({
                "result": result,
                "operation": format!("{} + {} = {}", a, b, result)
            }),
            0,
        )
    }
}

/// A simple echo tool that echoes back the input message.
struct EchoTool {
    schema: ToolSchema,
    perms: PermissionSet,
}

impl EchoTool {
    fn new() -> Self {
        let schema = ToolSchema::new(
            "echo",
            "Echo back the input message",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to echo"
                    }
                },
                "required": ["message"]
            }),
        );
        
        Self {
            schema,
            perms: PermissionSet::minimal(),
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo back the input message"
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.perms
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let message = args.get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("No message provided");
        
        ToolResult::ok(
            serde_json::json!({
                "echo": message,
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            }),
            0,
        )
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║           Simple Agent Example - claw-kernel               ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    // ── 1. Create OllamaProvider ─────────────────────────────────────────────
    // Uses local Ollama instance (default: http://localhost:11434)
    // No API key required! Make sure Ollama is running locally.
    println!("1. Initializing OllamaProvider (local model, no API key needed)...");
    let provider = Arc::new(OllamaProvider::new("llama3.2")) as Arc<dyn LLMProvider>;
    println!("   ✓ Provider: {} / model: {}\n", provider.provider_id(), provider.model_id());

    // ── 2. Create ToolRegistry and register tools ────────────────────────────
    println!("2. Creating ToolRegistry and registering tools...");
    let tools = Arc::new(ToolRegistry::new());
    
    // Register the calculator tool
    tools.register(Box::new(CalculatorTool::new()))?;
    println!("   ✓ Registered tool: calculator");
    
    // Register the echo tool
    tools.register(Box::new(EchoTool::new()))?;
    println!("   ✓ Registered tool: echo");
    println!("   ✓ Total tools registered: {}\n", tools.tool_count());

    // ── 3. Build AgentLoop using AgentLoopBuilder ────────────────────────────
    println!("3. Building AgentLoop with configuration...");
    let mut agent = AgentLoopBuilder::new()
        .with_provider(provider)
        .with_tools(tools)
        .with_system_prompt(
            "You are a helpful assistant with access to calculator and echo tools. \
             When the user asks for math, use the calculator tool. \
             Be concise in your responses."
        )
        .with_max_turns(5)  // Limit to 5 turns max
        .with_stop_condition(Box::new(MaxTurns(3)))  // Additional safety: stop after 3 turns
        .build()?;
    println!("   ✓ AgentLoop built successfully");
    println!("   ✓ Max turns: 5");
    println!("   ✓ Stop condition: MaxTurns(3)\n");

    // ── 4. Run the agent with a user query ──────────────────────────────────
    println!("4. Running agent with user query...\n");
    
    let user_input = "Please calculate 123 + 456 for me.";
    println!("   User: {}", user_input);
    println!("   ─────────────────────────────────────────\n");

    match agent.run(user_input).await {
        Ok(result) => {
            println!("\n   ─────────────────────────────────────────");
            println!("\n5. Agent execution completed!");
            println!("   ✓ Finish reason: {:?}", result.finish_reason);
            println!("   ✓ Total turns: {}", result.turns);
            println!("   ✓ Token usage: {} prompt, {} completion, {} total",
                result.usage.prompt_tokens,
                result.usage.completion_tokens,
                result.usage.total_tokens
            );
            
            if let Some(last_msg) = result.last_message {
                println!("\n   Assistant's final response:");
                println!("   \"{}\"", last_msg.content);
            }
            
            // Show conversation history
            println!("\n6. Conversation history ({} messages):", agent.history().len());
            for (i, msg) in agent.history().iter().enumerate() {
                let role = format!("{:?}", msg.role);
                let preview = if msg.content.len() > 60 {
                    format!("{}...", &msg.content[..60])
                } else {
                    msg.content.clone()
                };
                println!("   [{}] {}: {}", i, role, preview);
            }
        }
        Err(e) => {
            eprintln!("\n   ✗ Agent execution failed: {}", e);
            return Err(e.into());
        }
    }

    println!("\n✓ Example completed successfully!");
    Ok(())
}
