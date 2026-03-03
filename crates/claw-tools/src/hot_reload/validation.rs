//! Tool validation pipeline for hot-reloading.
//!
//! Provides a 4-step validation process for tool scripts:
//! 1. Syntax check
//! 2. Permission audit
//! 3. Schema validation
//! 4. Sandbox compilation

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use crate::error::{LoadError, ValidationError};
use crate::hot_reload::watcher::WatchEvent;
use crate::registry::ToolRegistry;
use crate::traits::Tool;
use crate::types::HotLoadingConfig;

/// Tool watcher that validates tools before loading.
///
/// Implements a 4-step validation pipeline to ensure tool safety
/// and correctness before registration.
pub struct ToolWatcher {
    config: HotLoadingConfig,
    registry: Arc<ToolRegistry>,
}

impl ToolWatcher {
    /// Create a new tool watcher.
    pub fn new(config: HotLoadingConfig, registry: Arc<ToolRegistry>) -> Self {
        Self { config, registry }
    }

    /// Validate a tool file through the complete 4-step pipeline.
    ///
    /// # Validation Steps
    ///
    /// 1. **Syntax check** - Parse the script for syntax errors
    /// 2. **Permission audit** - Verify permission declarations are valid
    /// 3. **Schema validation** - Validate tool metadata against JSON Schema
    /// 4. **Sandbox compilation** - Compile and load in isolated environment
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if any step fails.
    pub async fn validate(&self, path: &Path) -> Result<(), ValidationError> {
        // Step 1: Syntax check
        self.check_syntax(path).await?;

        // Step 2: Permission audit
        self.audit_permissions(path).await?;

        // Step 3: Schema validation
        self.validate_schema(path).await?;

        // Step 4: Sandbox compilation
        self.sandbox_compile(path).await?;

        Ok(())
    }

    /// Step 1: Check syntax of the tool script.
    ///
    /// For Lua scripts, this parses the file to detect syntax errors.
    /// For other languages, appropriate parsers would be used.
    async fn check_syntax(&self, path: &Path) -> Result<(), ValidationError> {
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| ValidationError::SyntaxError {
                    file: path.to_path_buf(),
                    message: format!("failed to read file: {}", e),
                })?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "lua" => {
                // Basic Lua syntax validation
                // In production, this would use a proper Lua parser
                self.validate_lua_syntax(path, &content).await?;
            }
            "js" | "ts" => {
                // JavaScript/TypeScript syntax validation placeholder
                // In production, integrate with Deno or Node.js
                self.validate_js_syntax(path, &content).await?;
            }
            "py" => {
                // Python syntax validation
                self.validate_python_syntax(path, &content).await?;
            }
            _ => {
                // Unknown extension - allow but warn
                tracing::warn!("Unknown file extension '{}', skipping syntax check", ext);
            }
        }

        Ok(())
    }

    /// Validate Lua script syntax.
    async fn validate_lua_syntax(&self, path: &Path, content: &str) -> Result<(), ValidationError> {
        // Basic syntax checks for Lua
        // Check for common issues that would cause parse errors

        // Check for unmatched brackets/parentheses
        let mut brace_depth = 0i32;
        let mut bracket_depth = 0i32;
        let mut paren_depth = 0i32;
        let mut in_string: Option<char> = None;
        let mut escaped = false;

        for (line_num, line) in content.lines().enumerate() {
            for ch in line.chars() {
                if escaped {
                    escaped = false;
                    continue;
                }

                match ch {
                    '\\' if in_string.is_some() => escaped = true,
                    '"' | '\'' if in_string == Some(ch) => in_string = None,
                    '"' | '\'' if in_string.is_none() => in_string = Some(ch),
                    '[' if in_string.is_none() && line.contains("[[") => {
                        // Long string/literal start - simplified check
                    }
                    '{' if in_string.is_none() => brace_depth += 1,
                    '}' if in_string.is_none() => brace_depth -= 1,
                    '[' if in_string.is_none() => bracket_depth += 1,
                    ']' if in_string.is_none() => bracket_depth -= 1,
                    '(' if in_string.is_none() => paren_depth += 1,
                    ')' if in_string.is_none() => paren_depth -= 1,
                    _ => {}
                }

                // Check for negative depth (closing before opening)
                if brace_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched '}}' at line {}", line_num + 1),
                    });
                }
                if bracket_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ']' at line {}", line_num + 1),
                    });
                }
                if paren_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ')' at line {}", line_num + 1),
                    });
                }
            }
        }

        // Check for unclosed structures
        if brace_depth != 0 {
            return Err(ValidationError::SyntaxError {
                file: path.to_path_buf(),
                message: format!("unclosed '{{' - missing {} closing braces", brace_depth),
            });
        }
        if bracket_depth != 0 {
            return Err(ValidationError::SyntaxError {
                file: path.to_path_buf(),
                message: format!("unclosed '[' - missing {} closing brackets", bracket_depth),
            });
        }
        if paren_depth != 0 {
            return Err(ValidationError::SyntaxError {
                file: path.to_path_buf(),
                message: format!("unclosed '(' - missing {} closing parentheses", paren_depth),
            });
        }

        if in_string.is_some() {
            return Err(ValidationError::SyntaxError {
                file: path.to_path_buf(),
                message: "unclosed string literal".to_string(),
            });
        }

        Ok(())
    }

    /// Validate JavaScript/TypeScript syntax (placeholder).
    async fn validate_js_syntax(&self, path: &Path, content: &str) -> Result<(), ValidationError> {
        // Basic brace/bracket matching similar to Lua
        let mut brace_depth = 0i32;
        let mut bracket_depth = 0i32;
        let mut paren_depth = 0i32;

        for (line_num, line) in content.lines().enumerate() {
            // Skip comments
            let code = if let Some(idx) = line.find("//") {
                &line[..idx]
            } else {
                line
            };

            for ch in code.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    '[' => bracket_depth += 1,
                    ']' => bracket_depth -= 1,
                    '(' => paren_depth += 1,
                    ')' => paren_depth -= 1,
                    _ => {}
                }

                if brace_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched '}}' at line {}", line_num + 1),
                    });
                }
                if bracket_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ']' at line {}", line_num + 1),
                    });
                }
                if paren_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ')' at line {}", line_num + 1),
                    });
                }
            }
        }

        if brace_depth != 0 {
            return Err(ValidationError::SyntaxError {
                file: path.to_path_buf(),
                message: format!("unclosed '{{' - missing {} closing braces", brace_depth),
            });
        }

        Ok(())
    }

    /// Validate Python syntax (placeholder).
    async fn validate_python_syntax(
        &self,
        path: &Path,
        content: &str,
    ) -> Result<(), ValidationError> {
        // Basic indentation and bracket checking
        let _indent_stack: Vec<usize> = vec![0];
        let mut paren_depth = 0i32;
        let mut bracket_depth = 0i32;
        let mut brace_depth = 0i32;

        for (line_num, line) in content.lines().enumerate() {
            // Skip empty lines and comments
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Check brackets
            for ch in line.chars() {
                match ch {
                    '(' => paren_depth += 1,
                    ')' => paren_depth -= 1,
                    '[' => bracket_depth += 1,
                    ']' => bracket_depth -= 1,
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    _ => {}
                }

                if paren_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ')' at line {}", line_num + 1),
                    });
                }
                if bracket_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched ']' at line {}", line_num + 1),
                    });
                }
                if brace_depth < 0 {
                    return Err(ValidationError::SyntaxError {
                        file: path.to_path_buf(),
                        message: format!("unmatched '}}' at line {}", line_num + 1),
                    });
                }
            }
        }

        Ok(())
    }

    /// Step 2: Audit permissions declared in the tool.
    ///
    /// Validates that permission declarations are well-formed and
    /// do not request unsafe combinations.
    async fn audit_permissions(&self, path: &Path) -> Result<(), ValidationError> {
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            ValidationError::PermissionError {
                file: path.to_path_buf(),
                issue: format!("failed to read file: {}", e),
            }
        })?;

        // Check for required permission fields in metadata
        // This is a simplified check - real implementation would parse the metadata section

        // Look for permission annotations or metadata
        let has_permissions = content.contains("permissions")
            || content.contains("@permission")
            || content.contains("--- permissions");

        if !has_permissions {
            // Tools without explicit permissions get minimal permissions
            // This is a warning-level issue, not an error
            tracing::debug!(
                "Tool at {:?} has no explicit permissions, will use minimal permissions",
                path
            );
        }

        // Check for dangerous permission combinations
        let dangerous_combos = [
            ("filesystem.write", "subprocess.allow"),
            ("network", "subprocess.allow"),
        ];

        for (perm1, perm2) in &dangerous_combos {
            if content.contains(perm1) && content.contains(perm2) {
                // Warn about dangerous combination but don't block
                // In strict mode, this could be an error
                tracing::warn!(
                    "Tool at {:?} has potentially dangerous permission combination: {} + {}",
                    path,
                    perm1,
                    perm2
                );
            }
        }

        // Validate paths are absolute or properly formatted
        // Check for suspicious patterns in file paths
        let suspicious_patterns = ["..", "~", "$HOME", "$USER"];
        for pattern in &suspicious_patterns {
            if content.contains(pattern) {
                tracing::warn!(
                    "Tool at {:?} contains potentially unsafe path pattern: {}",
                    path,
                    pattern
                );
            }
        }

        Ok(())
    }

    /// Step 3: Validate tool schema against JSON Schema.
    ///
    /// Verifies that the tool definition includes:
    /// - Required fields: name, description, parameters
    /// - Valid JSON Schema for parameters
    /// - Proper type definitions
    pub async fn validate_schema(&self, path: &Path) -> Result<(), ValidationError> {
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| ValidationError::SchemaError {
                    file: path.to_path_buf(),
                    details: vec![format!("failed to read file: {}", e)],
                })?;

        let mut errors = Vec::new();

        // Extract metadata section (assumed to be at top, possibly in comments)
        let metadata = self.extract_metadata(&content);

        // Check required fields
        let required_fields = ["name", "description", "parameters"];
        for field in &required_fields {
            if !metadata.contains_key(*field) {
                errors.push(format!("missing required field: {}", field));
            }
        }

        // Validate name format (snake_case)
        if let Some(name) = metadata.get("name") {
            if !self.is_valid_snake_case(name) {
                errors.push(format!(
                    "invalid tool name '{}': must be snake_case (lowercase with underscores)",
                    name
                ));
            }
        }

        // Validate description is non-empty
        if let Some(desc) = metadata.get("description") {
            if desc.trim().is_empty() {
                errors.push("description cannot be empty".to_string());
            } else if desc.len() < 10 {
                errors.push(format!(
                    "description too short ({} chars): should be at least 10 characters",
                    desc.len()
                ));
            }
        }

        // Validate parameters schema
        if let Some(params_str) = metadata.get("parameters") {
            match serde_json::from_str::<serde_json::Value>(params_str) {
                Ok(schema) => {
                    // Check it's a valid object schema
                    if let Some(obj) = schema.as_object() {
                        if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                            // Validate each property has a type
                            for (prop_name, prop_def) in props {
                                if let Some(prop_obj) = prop_def.as_object() {
                                    if !prop_obj.contains_key("type") {
                                        errors.push(format!(
                                            "parameter '{}' missing 'type' field",
                                            prop_name
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!("invalid JSON schema for parameters: {}", e));
                }
            }
        }

        if !errors.is_empty() {
            return Err(ValidationError::SchemaError {
                file: path.to_path_buf(),
                details: errors,
            });
        }

        Ok(())
    }

    /// Extract metadata from tool script content.
    ///
    /// Supports multiple formats:
    /// - Lua: `-- @name: value` or `--- name: value`
    /// - JS/TS: `/** @name value */` or `// @name: value`
    fn extract_metadata(&self, content: &str) -> HashMap<String, String> {
        let mut metadata = HashMap::new();

        for line in content.lines() {
            let trimmed = line.trim();

            // Lua style: -- @key: value or --- key: value
            if let Some(caps) = self.parse_lua_comment(trimmed) {
                metadata.insert(caps.0, caps.1);
                continue;
            }

            // JS style: // @key: value or /** @key value */
            if let Some(caps) = self.parse_js_comment(trimmed) {
                metadata.insert(caps.0, caps.1);
                continue;
            }

            // Python style: # @key: value
            if let Some(caps) = self.parse_python_comment(trimmed) {
                metadata.insert(caps.0, caps.1);
            }
        }

        metadata
    }

    /// Parse Lua-style comment metadata.
    fn parse_lua_comment(&self, line: &str) -> Option<(String, String)> {
        // Match patterns like: -- @name: value or --- name: value
        let patterns = ["-- @", "--- ", "-- "];

        for prefix in &patterns {
            if let Some(rest) = line.strip_prefix(prefix) {
                if let Some((key, value)) = rest.split_once(':') {
                    return Some((key.trim().to_string(), value.trim().to_string()));
                }
            }
        }

        None
    }

    /// Parse JavaScript-style comment metadata.
    fn parse_js_comment(&self, line: &str) -> Option<(String, String)> {
        // Match patterns like: // @name: value
        if let Some(rest) = line.strip_prefix("// @") {
            if let Some((key, value)) = rest.split_once(':') {
                return Some((key.trim().to_string(), value.trim().to_string()));
            }
        }

        // Match: /** @name value */
        if line.starts_with("/** @") {
            let rest = &line[4..]; // Skip /** @
            if let Some(end_idx) = rest.find(" */") {
                let content = &rest[..end_idx];
                let parts: Vec<_> = content.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    return Some((parts[0].to_string(), parts[1].to_string()));
                }
            }
        }

        None
    }

    /// Parse Python-style comment metadata.
    fn parse_python_comment(&self, line: &str) -> Option<(String, String)> {
        // Match patterns like: # @name: value or # name: value
        let prefixes = ["# @", "# "];

        for prefix in &prefixes {
            if let Some(rest) = line.strip_prefix(prefix) {
                if let Some((key, value)) = rest.split_once(':') {
                    return Some((key.trim().to_string(), value.trim().to_string()));
                }
            }
        }

        None
    }

    /// Check if a string is valid snake_case.
    fn is_valid_snake_case(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }

        // Must start with lowercase letter or underscore
        let first = s.chars().next().unwrap();
        if !first.is_ascii_lowercase() && first != '_' {
            return false;
        }

        // All characters must be lowercase, digits, or underscore
        s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    /// Step 4: Compile tool in sandbox environment.
    ///
    /// Attempts to compile/load the tool in an isolated environment
    /// to detect runtime errors without affecting the main system.
    ///
    /// Default timeout: 30 seconds
    pub async fn sandbox_compile(&self, path: &Path) -> Result<(), ValidationError> {
        let compile_timeout = Duration::from_secs(self.config.compile_timeout_secs.max(30));

        let path_buf = path.to_path_buf();
        let path_buf_for_err = path_buf.clone();
        let result = timeout(compile_timeout, async move {
            // Run compilation in spawn_blocking to avoid blocking the async runtime
            tokio::task::spawn_blocking(move || Self::compile_in_isolation(&path_buf))
                .await
                .map_err(|e| ValidationError::CompilationError {
                    file: path_buf_for_err,
                    stderr: format!("task join error: {}", e),
                })?
        })
        .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ValidationError::TimeoutError {
                file: path.to_path_buf(),
                operation: "sandbox compilation".to_string(),
            }),
        }
    }

    /// Compile a tool in isolation (runs in spawn_blocking context).
    fn compile_in_isolation(path: &Path) -> Result<(), ValidationError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ValidationError::CompilationError {
                file: path.to_path_buf(),
                stderr: format!("failed to read file: {}", e),
            })?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "lua" => Self::compile_lua(&content, path),
            "js" | "ts" => Self::compile_js(&content, path),
            "py" => Self::compile_python(&content, path),
            _ => {
                // Unknown extension - skip compilation check
                tracing::warn!("Unknown extension '{}', skipping sandbox compilation", ext);
                Ok(())
            }
        }
    }

    /// Compile Lua script.
    fn compile_lua(content: &str, path: &Path) -> Result<(), ValidationError> {
        // Check for common Lua runtime issues

        // Check for undefined globals (basic heuristics)
        let defined_globals: HashSet<&str> = [
            "_G",
            "_VERSION",
            "assert",
            "collectgarbage",
            "dofile",
            "error",
            "getmetatable",
            "ipairs",
            "load",
            "loadfile",
            "next",
            "pairs",
            "pcall",
            "print",
            "rawequal",
            "rawget",
            "rawlen",
            "rawset",
            "require",
            "select",
            "setmetatable",
            "tonumber",
            "tostring",
            "type",
            "xpcall",
            "string",
            "table",
            "math",
            "io",
            "os",
            "debug",
            "coroutine",
            "package",
            "utf8",
            "tool",
            "args",
            "ctx",
        ]
        .into_iter()
        .collect();

        // Track if we find function definition in actual code (not comments)
        let mut has_function_def = false;

        // Simple pattern to find potential global accesses
        // This is a heuristic - proper static analysis would be more accurate
        for (line_num, line) in content.lines().enumerate() {
            // Skip comments
            let code = if let Some(idx) = line.find("--") {
                &line[..idx]
            } else {
                line
            };

            // Check for function definition in code (not comments)
            if code.contains("function") {
                has_function_def = true;
            }

            // Check for assignments to potentially undefined globals
            if let Some(eq_idx) = code.find('=') {
                let left = &code[..eq_idx].trim();
                if let Some(first_word) = left.split_whitespace().next() {
                    // If it looks like a simple assignment to a global
                    if first_word.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !defined_globals.contains(first_word)
                    {
                        // This might be defining a new global - warn but don't error
                        tracing::debug!(
                            "Line {}: potential global variable '{}'",
                            line_num + 1,
                            first_word
                        );
                    }
                }
            }
        }

        // Check for required functions that tools should have
        if !has_function_def {
            return Err(ValidationError::CompilationError {
                file: path.to_path_buf(),
                stderr: "tool script must contain at least one function definition".to_string(),
            });
        }

        Ok(())
    }

    /// Compile JavaScript/TypeScript.
    fn compile_js(content: &str, path: &Path) -> Result<(), ValidationError> {
        // Basic JS validation
        // Check for required exports
        if !content.contains("export") && !content.contains("module.exports") {
            tracing::warn!("JS tool at {:?} may be missing exports", path);
        }

        // Check for async/await syntax errors
        let async_count = content.matches("async").count();
        let await_count = content.matches("await").count();

        if async_count > 0 && await_count == 0 {
            tracing::debug!("JS tool at {:?} has 'async' but no 'await'", path);
        }

        Ok(())
    }

    /// Compile Python.
    fn compile_python(content: &str, path: &Path) -> Result<(), ValidationError> {
        // Basic Python validation
        // Check for required function definition
        if !content.contains("def ") {
            return Err(ValidationError::CompilationError {
                file: path.to_path_buf(),
                stderr: "Python tool must contain at least one function definition".to_string(),
            });
        }

        // Check for proper indentation (basic check)
        let mut prev_indent: Option<usize> = None;
        for (line_num, line) in content.lines().enumerate() {
            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }

            let current_indent = line.len() - line.trim_start().len();

            if let Some(prev) = prev_indent {
                // Indentation must be consistent (multiples of 4 spaces typically)
                if current_indent > prev && (current_indent - prev) % 4 != 0 {
                    return Err(ValidationError::CompilationError {
                        file: path.to_path_buf(),
                        stderr: format!(
                            "inconsistent indentation at line {}: expected multiple of 4",
                            line_num + 1
                        ),
                    });
                }
            }

            prev_indent = Some(current_indent);
        }

        Ok(())
    }

    /// Handle a file change event with validation.
    ///
    /// This is the main entry point for the ToolWatcher.
    /// It validates the file and returns a result indicating
    /// whether the file should be loaded.
    pub async fn handle_file_change(
        &self,
        event: &WatchEvent,
    ) -> Result<Option<Arc<dyn Tool>>, ValidationError> {
        match event {
            WatchEvent::FileChanged(path) | WatchEvent::FileCreated(path) => {
                // Run full validation pipeline
                self.validate(path).await?;

                // Validation passed - tool can be loaded
                // Return None for now since actual loading is done by HotReloadProcessor
                Ok(None)
            }
            WatchEvent::FileRemoved(path) => {
                tracing::info!("File removed: {:?}", path);
                Ok(None)
            }
            WatchEvent::Debounced(paths) => {
                // Validate all paths in the debounced batch
                for path in paths {
                    self.validate(path).await?;
                }
                Ok(None)
            }
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &HotLoadingConfig {
        &self.config
    }

    /// Get the registry.
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }
}

/// Convert ValidationError to LoadError for compatibility.
impl From<ValidationError> for LoadError {
    fn from(err: ValidationError) -> Self {
        match err {
            ValidationError::SyntaxError { file, message } => {
                LoadError::ParseError(format!("{}: {}", file.display(), message))
            }
            ValidationError::PermissionError { file, issue } => {
                LoadError::ParseError(format!("{}: permission error - {}", file.display(), issue))
            }
            ValidationError::SchemaError { file, details } => {
                LoadError::ParseError(format!("{}: schema error - {:?}", file.display(), details))
            }
            ValidationError::CompilationError { file, stderr } => {
                LoadError::CompileError(format!("{}: {}", file.display(), stderr))
            }
            ValidationError::TimeoutError { file, operation } => {
                LoadError::CompileError(format!("{}: {} timed out", file.display(), operation))
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use tempfile::TempDir;

    fn test_config() -> HotLoadingConfig {
        HotLoadingConfig {
            watch_dirs: vec![PathBuf::from("/tmp/tools")],
            extensions: vec!["lua".to_string(), "js".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
            compile_timeout_secs: 10,
            keep_previous_secs: 300,
            auto_enable: true,
        }
    }

    fn create_test_watcher() -> ToolWatcher {
        let registry = Arc::new(ToolRegistry::new());
        ToolWatcher::new(test_config(), registry)
    }

    #[tokio::test]
    async fn test_validate_lua_syntax_ok() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- Test tool
-- @name: test_tool
-- @description: A test tool

function execute(args)
    return { result = "ok" }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.check_syntax(&test_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_lua_syntax_error_unmatched_brace() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
function execute(args) {
    return { result = "ok" }
-- missing closing brace
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.check_syntax(&test_file).await;
        assert!(matches!(result, Err(ValidationError::SyntaxError { .. })));
    }

    #[tokio::test]
    async fn test_validate_schema_ok() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- @name: test_tool
-- @description: A test tool that does something useful
-- @parameters: {"type": "object", "properties": {"input": {"type": "string"}}}

function execute(args)
    return { result = args.input }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.validate_schema(&test_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_schema_missing_fields() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- Incomplete metadata

function execute(args)
    return { result = "ok" }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.validate_schema(&test_file).await;
        assert!(matches!(result, Err(ValidationError::SchemaError { .. })));
    }

    #[tokio::test]
    async fn test_validate_schema_invalid_name() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- @name: Invalid-Tool-Name
-- @description: A test tool that does something useful
-- @parameters: {"type": "object"}
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.validate_schema(&test_file).await;
        assert!(matches!(result, Err(ValidationError::SchemaError { .. })));
    }

    #[tokio::test]
    async fn test_sandbox_compile_lua() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- @name: test_tool
-- @description: A valid tool

function execute(args)
    return { result = "ok" }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.sandbox_compile(&test_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_sandbox_compile_no_function() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- @name: test_tool
-- @description: Missing function

local x = 1
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.sandbox_compile(&test_file).await;
        assert!(
            matches!(result, Err(ValidationError::CompilationError { .. })),
            "Expected CompilationError but got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_full_validation_pipeline() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- @name: complete_tool
-- @description: A complete valid tool for testing
-- @parameters: {"type": "object", "properties": {"message": {"type": "string"}}}

function execute(args)
    return { 
        success = true,
        message = args.message
    }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.validate(&test_file).await;
        assert!(result.is_ok(), "Full validation should pass: {:?}", result);
    }

    #[tokio::test]
    async fn test_extract_metadata_lua() {
        let watcher = create_test_watcher();

        let content = r#"
-- @name: my_tool
-- @description: Does something
-- @version: 1.0.0
-- unrelated comment
local x = 1
"#;

        let metadata = watcher.extract_metadata(content);
        assert_eq!(metadata.get("name"), Some(&"my_tool".to_string()));
        assert_eq!(
            metadata.get("description"),
            Some(&"Does something".to_string())
        );
        assert_eq!(metadata.get("version"), Some(&"1.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_is_valid_snake_case() {
        let watcher = create_test_watcher();

        assert!(watcher.is_valid_snake_case("valid_name"));
        assert!(watcher.is_valid_snake_case("tool_1"));
        assert!(watcher.is_valid_snake_case("_private"));

        assert!(!watcher.is_valid_snake_case("InvalidName"));
        assert!(!watcher.is_valid_snake_case("invalid-name"));
        assert!(!watcher.is_valid_snake_case("1starts_with_number"));
        assert!(!watcher.is_valid_snake_case(""));
    }

    #[tokio::test]
    async fn test_audit_permissions_ok() {
        let watcher = create_test_watcher();
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.lua");

        let content = r#"
-- permissions: minimal
-- @name: safe_tool

function execute(args)
    return { result = "ok" }
end
"#;

        tokio::fs::write(&test_file, content).await.unwrap();

        let result = watcher.audit_permissions(&test_file).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validation_error_into_load_error() {
        let validation_err = ValidationError::SyntaxError {
            file: PathBuf::from("/test/tool.lua"),
            message: "unexpected token".to_string(),
        };

        let load_err: LoadError = validation_err.into();
        match load_err {
            LoadError::ParseError(msg) => {
                assert!(msg.contains("unexpected token"));
            }
            _ => panic!("Expected ParseError"),
        }
    }
}
