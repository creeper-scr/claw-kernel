use std::sync::Arc;

use claw_tools::{
    registry::ToolRegistry,
    types::{PermissionSet, ToolContext},
};

/// Host-side tools bridge — exposes ToolRegistry calls over JSON.
pub struct ToolsBridge {
    registry: Arc<ToolRegistry>,
}

impl ToolsBridge {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    /// Call a tool by name with JSON args. Returns ToolResult as JSON.
    pub async fn call_tool(
        &self,
        agent_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        let ctx = ToolContext::new(agent_id, PermissionSet::minimal());
        match self.registry.execute(tool_name, args, ctx).await {
            Ok(result) => serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
            Err(e) => serde_json::json!({"success": false, "error": e.to_string()}),
        }
    }

    /// List available tools.
    pub fn list_tools(&self) -> Vec<String> {
        self.registry.tool_names()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_bridge() -> ToolsBridge {
        ToolsBridge::new(Arc::new(ToolRegistry::new()))
    }

    #[test]
    fn test_tools_bridge_list_empty() {
        let bridge = empty_bridge();
        let tools = bridge.list_tools();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_tools_bridge_call_nonexistent_tool() {
        let bridge = empty_bridge();
        let result = bridge
            .call_tool("agent-1", "does_not_exist", serde_json::json!({}))
            .await;
        // Should return an error JSON (not panic)
        assert!(result.is_object());
        let obj = result.as_object().unwrap();
        assert_eq!(obj.get("success").and_then(|v| v.as_bool()), Some(false));
        assert!(obj.contains_key("error"));
    }
}
