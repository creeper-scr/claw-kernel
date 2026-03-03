//! Custom Tool Example
//!
//! This example demonstrates how to create and use custom Lua tools with claw-kernel.
//! It shows:
//!   - LuaEngine: Creating a Lua script engine
//!   - Lua tool scripts: Writing a calculator tool in Lua
//!   - ToolSchema: Defining input/output schema for tools
//!   - PermissionSet: Configuring Safe Mode permissions
//!   - ToolsBridge: Connecting Lua tools to ToolRegistry

use std::sync::Arc;
use std::time::Duration;

use claw_kernel::prelude::*;
use claw_script::LuaEngine;
use claw_script::types::{Script, ScriptContext};
use claw_tools::types::{PermissionSet, ToolSchema, ToolResult, ToolError, ToolContext};
use claw_tools::traits::Tool;
use claw_tools::registry::ToolRegistry;

/// A custom tool implementation that wraps a Lua script.
/// This demonstrates how to create a tool that executes Lua code.
pub struct LuaCalculatorTool {
    schema: ToolSchema,
    permissions: PermissionSet,
}

impl LuaCalculatorTool {
    pub fn new() -> Self {
        // Define the tool schema with input/output specifications
        let parameters = serde_json::json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Mathematical expression to evaluate (e.g., '2 + 3 * 4')"
                },
                "precision": {
                    "type": "integer",
                    "description": "Number of decimal places for the result",
                    "minimum": 0,
                    "maximum": 10,
                    "default": 2
                }
            },
            "required": ["expression"]
        });

        let schema = ToolSchema::new(
            "calculator",
            "A safe calculator that evaluates mathematical expressions",
            parameters,
        );

        // Configure permissions for Safe Mode:
        // - No filesystem access
        // - No network access  
        // - No subprocess spawning
        let permissions = PermissionSet::minimal();

        Self {
            schema,
            permissions,
        }
    }
}

#[async_trait::async_trait]
impl Tool for LuaCalculatorTool {
    fn name(&self) -> &str {
        &self.schema.name
    }

    fn description(&self) -> &str {
        &self.schema.description
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> ToolResult {
        // Create the Lua script that implements the calculator logic
        let lua_script = r#"
            -- Safe calculator implementation
            -- 'args' is injected as a global variable from ScriptContext
            local expression = args.expression
            local precision = args.precision or 2
            
            -- Basic validation
            if type(expression) ~= "string" or expression == "" then
                return {
                    success = false,
                    error = "Invalid expression: must be a non-empty string"
                }
            end
            
            -- Whitelist of allowed characters (safe math operations only)
            local allowed_pattern = "^[0-9%.%+%-%*/%%%(%)%s]+$"
            if not expression:match(allowed_pattern) then
                return {
                    success = false,
                    error = "Expression contains invalid characters. Only numbers and + - * / % ( ) are allowed."
                }
            end
            
            -- Load and execute the expression safely
            local func, err = load("return " .. expression, "=calculator", "t", {})
            if not func then
                return {
                    success = false,
                    error = "Failed to parse expression: " .. tostring(err)
                }
            end
            
            -- Execute in protected mode
            local ok, result = pcall(func)
            if not ok then
                return {
                    success = false,
                    error = "Calculation error: " .. tostring(result)
                }
            end
            
            -- Format result with specified precision
            local format_str = "%." .. precision .. "f"
            local formatted = string.format(format_str, result)
            
            -- Remove trailing zeros for cleaner output
            formatted = formatted:gsub("0+$", ""):gsub("%.$", "")
            
            return {
                success = true,
                result = tonumber(formatted),
                expression = expression
            }
        "#;

        // Create Lua engine and execute script
        let engine = LuaEngine::new();
        
        // Create script context with Safe Mode permissions
        let script = Script::lua("calculator", lua_script);
        let ctx = ScriptContext::new("custom-tool-agent")
            .with_global("args", args.clone())
            .with_timeout(Duration::from_secs(5));

        // Execute the Lua script
        match engine.execute(&script, &ctx).await {
            Ok(result) => {
                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                    ToolResult::ok(result, 0)
                } else {
                    let error_msg = result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    ToolResult::err(
                        ToolError::invalid_args(error_msg),
                        0,
                    )
                }
            }
            Err(e) => ToolResult::err(
                ToolError::internal(format!("Script execution failed: {}", e)),
                0,
            ),
        }
    }
}

/// Another example: A string manipulation tool in Lua
pub struct LuaStringTool {
    schema: ToolSchema,
    permissions: PermissionSet,
}

impl LuaStringTool {
    pub fn new() -> Self {
        let parameters = serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Input text to process"
                },
                "operation": {
                    "type": "string",
                    "enum": ["uppercase", "lowercase", "reverse", "word_count"],
                    "description": "Operation to perform on the text"
                }
            },
            "required": ["text", "operation"]
        });

        let schema = ToolSchema::new(
            "string_processor",
            "Process strings with various operations",
            parameters,
        );

        let permissions = PermissionSet::minimal();

        Self {
            schema,
            permissions,
        }
    }
}

#[async_trait::async_trait]
impl Tool for LuaStringTool {
    fn name(&self) -> &str {
        &self.schema.name
    }

    fn description(&self) -> &str {
        &self.schema.description
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> ToolResult {
        let lua_script = r#"
            -- 'args' is injected as a global variable from ScriptContext
            local text = args.text
            local operation = args.operation
            
            if type(text) ~= "string" then
                return { success = false, error = "text must be a string" }
            end
            
            local result
            if operation == "uppercase" then
                result = string.upper(text)
            elseif operation == "lowercase" then
                result = string.lower(text)
            elseif operation == "reverse" then
                result = string.reverse(text)
            elseif operation == "word_count" then
                local _, count = string.gsub(text, "%S+", "")
                result = count
            else
                return { success = false, error = "Unknown operation: " .. tostring(operation) }
            end
            
            return {
                success = true,
                result = result,
                operation = operation
            }
        "#;

        let engine = LuaEngine::new();
        let script = Script::lua("string_processor", lua_script);
        let ctx = ScriptContext::new("custom-tool-agent")
            .with_global("args", args)
            .with_timeout(Duration::from_secs(5));

        match engine.execute(&script, &ctx).await {
            Ok(result) => {
                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                    ToolResult::ok(result, 0)
                } else {
                    let error_msg = result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    ToolResult::err(
                        ToolError::invalid_args(error_msg),
                        0,
                    )
                }
            }
            Err(e) => ToolResult::err(
                ToolError::internal(format!("Script execution failed: {}", e)),
                0,
            ),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║           Custom Lua Tool Example - claw-kernel            ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // 1. Create LuaEngine
    println!("1. Creating LuaEngine...");
    let _lua_engine = LuaEngine::new();
    println!("   ✓ LuaEngine created");
    println!();

    // 2. Create ToolRegistry
    println!("2. Creating ToolRegistry...");
    let registry = Arc::new(ToolRegistry::new());
    println!("   ✓ ToolRegistry created");
    println!();

    // 3. Register Lua-based tools
    println!("3. Registering custom Lua tools...");
    registry.register(Box::new(LuaCalculatorTool::new()))?;
    registry.register(Box::new(LuaStringTool::new()))?;
    println!("   ✓ Registered 'calculator' tool (Lua-based)");
    println!("   ✓ Registered 'string_processor' tool (Lua-based)");
    println!();

    // 4. List registered tools
    println!("4. Registered tools:");
    for tool_name in registry.tool_names() {
        if let Some(meta) = registry.tool_meta(&tool_name) {
            println!("   • {} - {}", tool_name, meta.schema.description);
        }
    }
    println!();

    // 5. Test calculator tool
    println!("5. Testing calculator tool:");
    println!("   ─────────────────────────────────────");
    
    let test_expressions = vec![
        ("2 + 3 * 4", 2),
        ("(10 - 5) * 8", 2),
        ("100 / 3", 4),
        ("2 ^ 10", 0), // This should fail (exponentiation not allowed)
    ];

    for (expr, precision) in test_expressions {
        let args = serde_json::json!({
            "expression": expr,
            "precision": precision
        });

        let ctx = ToolContext::new("test-agent", PermissionSet::minimal());
        match registry.execute("calculator", args, ctx).await {
            Ok(result) => {
                if result.success {
                    let output = result.output.unwrap();
                    println!("   ✓ '{}' = {}", expr, output["result"]);
                } else {
                    let err = result.error.unwrap();
                    println!("   ✗ '{}' failed: {}", expr, err.message);
                }
            }
            Err(e) => println!("   ✗ '{}' error: {:?}", expr, e),
        }
    }
    println!();

    // 6. Test string processor tool
    println!("6. Testing string_processor tool:");
    println!("   ─────────────────────────────────────");

    let string_tests = vec![
        ("Hello, World!", "uppercase"),
        ("Hello, World!", "lowercase"),
        ("Hello, World!", "reverse"),
        ("The quick brown fox", "word_count"),
    ];

    for (text, operation) in string_tests {
        let args = serde_json::json!({
            "text": text,
            "operation": operation
        });

        let ctx = ToolContext::new("test-agent", PermissionSet::minimal());
        match registry.execute("string_processor", args, ctx).await {
            Ok(result) => {
                if result.success {
                    let output = result.output.unwrap();
                    println!("   ✓ '{}' [{}] = {}", 
                        text.chars().take(20).collect::<String>(),
                        operation,
                        output["result"]
                    );
                } else {
                    let err = result.error.unwrap();
                    println!("   ✗ '{}' failed: {}", text, err.message);
                }
            }
            Err(e) => println!("   ✗ '{}' error: {:?}", text, e),
        }
    }
    println!();

    // 7. Demonstrate PermissionSet configuration
    println!("7. PermissionSet configuration (Safe Mode):");
    let perms = PermissionSet::minimal();
    println!("   • Filesystem read paths: {:?}", perms.filesystem.read_paths);
    println!("   • Filesystem write paths: {:?}", perms.filesystem.write_paths);
    println!("   • Network domains: {:?}", perms.network.allowed_domains);
    println!("   • Subprocess policy: {:?}", perms.subprocess);
    println!();

    // 8. Show audit log
    println!("8. Audit log (recent calls):");
    let logs = registry.recent_log(10).await;
    for entry in logs {
        println!(
            "   • [{}] {}: {} ({} ms)",
            if entry.success { "✓" } else { "✗" },
            entry.tool_name,
            entry.agent_id,
            entry.duration_ms
        );
    }
    println!();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                   Example Complete!                        ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    Ok(())
}
