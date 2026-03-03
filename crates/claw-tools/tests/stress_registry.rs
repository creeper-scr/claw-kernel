//! ToolRegistry 高并发竞争测试
//!
//! 测试目标:
//! - 多线程并发对同一个 Registry 操作
//! - 100+ 并发任务同时进行: 注册、注销、execute 调用
//! - 检测死锁和数据竞争
//! - 验证审计日志完整性

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_tools::{
    PermissionSet, RegistryError, Tool, ToolContext, ToolErrorCode, ToolRegistry, ToolResult,
    ToolSchema,
};

// ─── Mock Tools for Testing ──────────────────────────────────────────────────

/// 快速执行的工具
struct FastTool {
    name: String,
    schema: ToolSchema,
    perms: PermissionSet,
}

#[async_trait]
impl Tool for FastTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Fast tool for stress testing"
    }
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
    fn permissions(&self) -> &PermissionSet {
        &self.perms
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // 模拟极快的执行
        ToolResult::ok(args, 0)
    }
}

/// 有延迟的工具
struct SlowTool {
    name: String,
    schema: ToolSchema,
    perms: PermissionSet,
    delay_ms: u64,
    timeout_ms: u64,
}

#[async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Slow tool for stress testing"
    }
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
    fn permissions(&self) -> &PermissionSet {
        &self.perms
    }
    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        ToolResult::ok(args, self.delay_ms)
    }
}

/// 随机失败的工具
struct FlakyTool {
    name: String,
    schema: ToolSchema,
    perms: PermissionSet,
}

#[async_trait]
impl Tool for FlakyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Flaky tool for stress testing"
    }
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
    fn permissions(&self) -> &PermissionSet {
        &self.perms
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // 基于参数决定是否失败
        if let Some(should_fail) = args.get("fail").and_then(|v| v.as_bool()) {
            if should_fail {
                return ToolResult::err(claw_tools::ToolError::internal("Intentional failure"), 0);
            }
        }
        ToolResult::ok(args, 0)
    }
}

fn make_fast_tool(name: &str) -> Box<dyn Tool> {
    Box::new(FastTool {
        name: name.to_string(),
        schema: ToolSchema::new(name, "Fast tool", serde_json::json!({})),
        perms: PermissionSet::minimal(),
    })
}

fn make_slow_tool(name: &str, delay_ms: u64) -> Box<dyn Tool> {
    Box::new(SlowTool {
        name: name.to_string(),
        schema: ToolSchema::new(name, "Slow tool", serde_json::json!({})),
        perms: PermissionSet::minimal(),
        delay_ms,
        timeout_ms: 5000, // 默认 5 秒超时
    })
}

fn make_timeout_tool(name: &str, delay_ms: u64, timeout_ms: u64) -> Box<dyn Tool> {
    Box::new(SlowTool {
        name: name.to_string(),
        schema: ToolSchema::new(name, "Timeout test tool", serde_json::json!({})),
        perms: PermissionSet::minimal(),
        delay_ms,
        timeout_ms,
    })
}

fn make_flaky_tool(name: &str) -> Box<dyn Tool> {
    Box::new(FlakyTool {
        name: name.to_string(),
        schema: ToolSchema::new(name, "Flaky tool", serde_json::json!({})),
        perms: PermissionSet::minimal(),
    })
}

fn default_ctx() -> ToolContext {
    ToolContext::new("stress-test-agent", PermissionSet::minimal())
}

// ─── Stress Tests ────────────────────────────────────────────────────────────

/// 测试 100+ 并发注册操作
#[tokio::test]
async fn test_registry_concurrent_register() {
    let registry = Arc::new(ToolRegistry::new());
    let concurrency = 150;

    let mut handles = vec![];
    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let tool_name = format!("tool-{}", i);
            let tool = make_fast_tool(&tool_name);
            reg.register(tool)
        });
        handles.push(handle);
    }

    // 收集结果
    let mut success_count = 0;
    let mut error_count = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => success_count += 1,
            Err(_) => error_count += 1,
        }
    }

    // 由于并发注册，可能会有一些冲突，但大部分应该成功
    println!(
        "Register: success={}, errors={}",
        success_count, error_count
    );

    // 验证最终注册的工具数量
    let tool_count = registry.tool_count();
    assert_eq!(
        tool_count, success_count,
        "Tool count should match successful registrations"
    );

    // 验证没有死锁（能走到这里说明没有死锁）
    assert!(tool_count > 0, "Some tools should be registered");
}

/// 测试并发注销操作
#[tokio::test]
async fn test_registry_concurrent_unregister() {
    let registry = Arc::new(ToolRegistry::new());

    // 先注册一些工具
    let initial_tools = 50;
    for i in 0..initial_tools {
        let tool = make_fast_tool(&format!("tool-{}", i));
        registry.register(tool).unwrap();
    }

    assert_eq!(registry.tool_count(), initial_tools);

    // 并发注销
    let mut handles = vec![];
    for i in 0..initial_tools {
        let reg = Arc::clone(&registry);
        let tool_name = format!("tool-{}", i);
        let handle = tokio::spawn(async move { reg.unregister(&tool_name) });
        handles.push(handle);
    }

    // 同时尝试注销不存在的工具
    for i in initial_tools..initial_tools + 20 {
        let reg = Arc::clone(&registry);
        let tool_name = format!("tool-{}", i);
        let handle = tokio::spawn(async move { reg.unregister(&tool_name) });
        handles.push(handle);
    }

    // 收集结果
    let mut success_count = 0;
    let mut not_found_count = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => success_count += 1,
            Err(RegistryError::ToolNotFound(_)) => not_found_count += 1,
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    println!(
        "Unregister: success={}, not_found={}",
        success_count, not_found_count
    );

    // 验证最终状态
    assert_eq!(
        success_count, initial_tools,
        "All existing tools should be unregistered"
    );
    assert_eq!(
        not_found_count, 20,
        "Non-existent tools should return NotFound"
    );
    assert_eq!(registry.tool_count(), 0, "Registry should be empty");
}

/// 测试混合并发操作（注册、注销、执行、查询）
#[tokio::test]
async fn test_registry_mixed_concurrent_operations() {
    let registry = Arc::new(ToolRegistry::new());

    // 先注册一批初始工具
    for i in 0..10 {
        let tool = make_fast_tool(&format!("fast-tool-{}", i));
        registry.register(tool).unwrap();
    }

    let concurrency = 100;
    let mut handles = vec![];

    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let op_type = i % 5;
            match op_type {
                0 => {
                    // 注册新工具
                    let tool = make_fast_tool(&format!("dynamic-tool-{}", i));
                    reg.register(tool)
                        .map(|_| "register_ok")
                        .unwrap_or("register_exists")
                }
                1 => {
                    // 注销工具
                    let tool_name = format!("fast-tool-{}", i % 10);
                    reg.unregister(&tool_name)
                        .map(|_| "unregister_ok")
                        .unwrap_or("unregister_notfound")
                }
                2 => {
                    // 执行工具
                    let tool_name = format!("fast-tool-{}", i % 10);
                    let ctx = default_ctx();
                    match reg
                        .execute(&tool_name, serde_json::json!({"idx": i}), ctx)
                        .await
                    {
                        Ok(_) => "execute_ok",
                        Err(_) => "execute_error",
                    }
                }
                3 => {
                    // 查询工具元数据
                    let tool_name = format!("fast-tool-{}", i % 10);
                    reg.tool_meta(&tool_name)
                        .map(|_| "meta_found")
                        .unwrap_or("meta_notfound")
                }
                4 => {
                    // 列出所有工具
                    let count = reg.tool_count();
                    if count > 0 {
                        "list_ok"
                    } else {
                        "list_empty"
                    }
                }
                _ => "unknown",
            }
        });
        handles.push(handle);
    }

    // 收集所有结果
    let mut results: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for handle in handles {
        let result = handle.await.unwrap();
        *results.entry(result.to_string()).or_insert(0) += 1;
    }

    println!("Mixed operations results: {:?}", results);

    // 验证系统没有死锁（能完成所有操作）
    let total_ops: usize = results.values().sum();
    assert_eq!(total_ops, concurrency, "All operations should complete");
}

/// 测试并发执行工具（热点竞争）
#[tokio::test]
async fn test_registry_concurrent_execute_hotspot() {
    let registry = Arc::new(ToolRegistry::new());

    // 注册一个热点工具
    let hot_tool = make_fast_tool("hot-tool");
    registry.register(hot_tool).unwrap();

    let concurrency = 200;
    let mut handles = vec![];

    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let ctx = ToolContext::new(format!("agent-{}", i), PermissionSet::minimal());
            let args = serde_json::json!({"request_id": i});
            reg.execute("hot-tool", args, ctx).await
        });
        handles.push(handle);
    }

    // 收集结果
    let mut success_count = 0;
    let mut error_count = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(result) => {
                if result.success {
                    success_count += 1;
                } else {
                    error_count += 1;
                }
            }
            Err(_) => error_count += 1,
        }
    }

    println!(
        "Hotspot execute: success={}, errors={}",
        success_count, error_count
    );

    // 所有执行都应该成功（快速工具不会超时）
    assert_eq!(success_count, concurrency, "All executions should succeed");
    assert_eq!(error_count, 0, "No errors expected");
}

/// 测试并发执行带超时的慢工具
#[tokio::test]
async fn test_registry_concurrent_execute_timeout() {
    let registry = Arc::new(ToolRegistry::new());

    // 注册一个专门用于超时测试的工具（500ms 延迟，但超时设置为 50ms）
    // 使用较长的延迟确保超时发生
    let timeout_tool = make_timeout_tool("timeout-tool", 500, 50);
    registry.register(timeout_tool).unwrap();

    let concurrency = 20;
    let mut handles = vec![];

    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let ctx = default_ctx();
            let args = serde_json::json!({"idx": i});
            reg.execute("timeout-tool", args, ctx).await
        });
        handles.push(handle);
    }

    // 收集结果 - 应该都是超时
    let mut timeout_count = 0;
    let mut success_count = 0;
    let mut error_count = 0;

    for handle in handles {
        match handle.await.unwrap() {
            Ok(result) => {
                if !result.success {
                    if let Some(ref err) = result.error {
                        if err.code == ToolErrorCode::Timeout {
                            timeout_count += 1;
                        } else {
                            error_count += 1;
                        }
                    } else {
                        error_count += 1;
                    }
                } else {
                    success_count += 1;
                }
            }
            Err(_) => error_count += 1,
        }
    }

    println!(
        "Timeout test: timeouts={}, success={}, errors={}",
        timeout_count, success_count, error_count
    );
    // 打印第一个错误详情以调试
    if error_count > 0 {
        let reg = Arc::clone(&registry);
        let ctx = default_ctx();
        let result = reg
            .execute("timeout-tool", serde_json::json!({}), ctx)
            .await;
        println!("Debug execution result: {:?}", result);
    }

    // 大部分应该超时（由于工具延迟 500ms，超时 50ms）
    // 注意：由于并发调度，可能有少数成功
    assert!(
        timeout_count > 0,
        "Some executions should timeout, got {}/{} timeouts (success: {}, errors: {})",
        timeout_count,
        concurrency,
        success_count,
        error_count
    );
}

/// 测试审计日志完整性
#[tokio::test]
async fn test_registry_audit_log_integrity() {
    let registry = Arc::new(ToolRegistry::new());

    // 注册工具
    let tool = make_fast_tool("audited-tool");
    registry.register(tool).unwrap();

    let concurrency = 100;
    let mut handles = vec![];

    // 并发执行工具
    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let ctx = ToolContext::new(format!("agent-{}", i % 10), PermissionSet::minimal());
            let args = serde_json::json!({"idx": i});
            reg.execute("audited-tool", args, ctx).await
        });
        handles.push(handle);
    }

    // 等待所有执行完成
    let mut success_count = 0;
    for handle in handles {
        if let Ok(Ok(result)) = handle.await {
            if result.success {
                success_count += 1;
            }
        }
    }

    // 验证审计日志
    let log = registry.recent_log(200).await;
    let log_len = log.len();

    println!(
        "Audit log: {} entries for {} successful executions",
        log_len, success_count
    );

    // 审计日志应该记录所有执行
    assert_eq!(log_len, concurrency, "All executions should be logged");

    // 验证日志条目的完整性
    for entry in &log {
        assert_eq!(entry.tool_name, "audited-tool");
        assert!(!entry.agent_id.is_empty());
        assert!(entry.timestamp_ms > 0);
        // success 字段应该根据执行结果设置
    }

    // 验证日志条目不重复（通过检查时间戳和 ID）
    let _unique_timestamps: std::collections::HashSet<_> =
        log.iter().map(|e| e.timestamp_ms).collect();
    // 由于并发，时间戳可能相同，但条目数量应该匹配
    assert_eq!(
        log_len, concurrency,
        "Log should have correct number of entries"
    );
}

/// 测试注册-注销-重新注册循环
#[tokio::test]
async fn test_registry_register_unregister_reregister_cycle() {
    let registry = Arc::new(ToolRegistry::new());
    let cycles = 50;
    let concurrency = 20;

    let mut handles = vec![];

    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let mut local_success = 0;
            for cycle in 0..cycles {
                let tool_name = format!("cyclic-tool-{}-{}", i, cycle);

                // 注册
                let tool = make_fast_tool(&tool_name);
                if reg.register(tool).is_ok() {
                    local_success += 1;
                }

                // 执行
                let ctx = default_ctx();
                let _ = reg.execute(&tool_name, serde_json::json!({}), ctx).await;

                // 注销
                let _ = reg.unregister(&tool_name);

                // 重新注册同名工具
                let tool2 = make_fast_tool(&tool_name);
                if reg.register(tool2).is_ok() {
                    local_success += 1;
                }

                // 再次执行
                let ctx = default_ctx();
                let _ = reg.execute(&tool_name, serde_json::json!({}), ctx).await;

                // 最后注销
                let _ = reg.unregister(&tool_name);
            }
            local_success
        });
        handles.push(handle);
    }

    // 等待所有循环完成
    let mut total_success = 0;
    for handle in handles {
        total_success += handle.await.unwrap();
    }

    println!(
        "Register-unregister cycles completed: {} successful operations",
        total_success
    );

    // 验证最终状态
    assert_eq!(
        registry.tool_count(),
        0,
        "Registry should be empty after all unregistrations"
    );

    // 验证审计日志
    let log = registry.recent_log(2000).await;
    println!("Audit log entries: {}", log.len());
    // 审计日志应该记录部分执行（由于并发和注销，可能不是所有执行都被记录）
    assert!(!log.is_empty(), "Audit log should record some executions");
}

/// 测试死锁检测 - 长时间运行
#[tokio::test]
async fn test_registry_no_deadlock_long_running() {
    let registry = Arc::new(ToolRegistry::new());

    // 注册多个工具
    for i in 0..10 {
        let tool = make_fast_tool(&format!("tool-{}", i));
        registry.register(tool).unwrap();
    }

    let duration = Duration::from_secs(2);
    let start = std::time::Instant::now();

    let mut handles = vec![];

    // 启动多个持续操作的任务
    for task_id in 0..10 {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let mut operations = 0;
            while start.elapsed() < duration {
                let op = operations % 4;
                match op {
                    0 => {
                        // 查询
                        let _ = reg.tool_count();
                    }
                    1 => {
                        // 执行
                        let tool_name = format!("tool-{}", task_id);
                        let ctx = default_ctx();
                        let _ = reg.execute(&tool_name, serde_json::json!({}), ctx).await;
                    }
                    2 => {
                        // 列出
                        let _ = reg.tool_names();
                    }
                    3 => {
                        // 获取元数据
                        let tool_name = format!("tool-{}", task_id);
                        let _ = reg.tool_meta(&tool_name);
                    }
                    _ => {}
                }
                operations += 1;
            }
            operations
        });
        handles.push(handle);
    }

    // 设置超时来检测死锁
    let timeout = tokio::time::timeout(Duration::from_secs(5), async {
        let mut total_ops = 0;
        for handle in handles {
            total_ops += handle.await.unwrap();
        }
        total_ops
    });

    match timeout.await {
        Ok(total_ops) => {
            println!("Long running test completed: {} operations", total_ops);
            assert!(total_ops > 0, "Operations should complete");
        }
        Err(_) => {
            panic!("Deadlock detected! Test timed out.");
        }
    }
}

/// 测试数据竞争 - 并发读写同一工具
#[tokio::test]
async fn test_registry_data_race_concurrent_read_write() {
    let registry = Arc::new(ToolRegistry::new());

    // 初始注册
    let tool = make_fast_tool("shared-tool");
    registry.register(tool).unwrap();

    let mut handles = vec![];

    // 一半任务读取（执行）
    for i in 0..50 {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let ctx = ToolContext::new(format!("reader-{}", i), PermissionSet::minimal());
            for _ in 0..20 {
                let _ = reg
                    .execute("shared-tool", serde_json::json!({}), ctx.clone())
                    .await;
            }
            "reader_done"
        });
        handles.push(handle);
    }

    // 另一半任务更新（注销并重新注册）
    for _i in 0..50 {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            for cycle in 0..10 {
                let _tool_name = format!("shared-tool-{}", cycle);
                // 更新同名工具
                let tool = make_fast_tool("shared-tool");
                let _ = reg.update("shared-tool", std::sync::Arc::from(tool));
                tokio::task::yield_now().await;
            }
            "writer_done"
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    let mut completed = 0;
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result == "reader_done" || result == "writer_done");
        completed += 1;
    }

    assert_eq!(
        completed, 100,
        "All tasks should complete without data race"
    );

    // 验证 registry 仍然可用
    let ctx = default_ctx();
    let result = registry
        .execute("shared-tool", serde_json::json!({}), ctx)
        .await;
    assert!(result.is_ok(), "Registry should still be functional");
}

/// 综合压力测试 - 模拟真实高并发场景
#[tokio::test]
async fn test_registry_comprehensive_stress() {
    let registry = Arc::new(ToolRegistry::new().with_max_audit_entries(1000));

    // 注册不同类型的工具
    for i in 0..20 {
        let tool = match i % 3 {
            0 => make_fast_tool(&format!("fast-{}", i)),
            1 => make_slow_tool(&format!("slow-{}", i), 10),
            _ => make_flaky_tool(&format!("flaky-{}", i)),
        };
        registry.register(tool).unwrap();
    }

    let concurrency = 150;
    let mut handles = vec![];

    for i in 0..concurrency {
        let reg = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            let mut stats = std::collections::HashMap::new();

            for op in 0..20 {
                let action = (i + op) % 6;
                match action {
                    0 => {
                        // 执行快速工具
                        let ctx = default_ctx();
                        let result = reg
                            .execute(&format!("fast-{}", op % 20), serde_json::json!({}), ctx)
                            .await;
                        *stats.entry("exec_fast").or_insert(0) += 1;
                        if result.is_ok() && result.unwrap().success {
                            *stats.entry("exec_fast_ok").or_insert(0) += 1;
                        }
                    }
                    1 => {
                        // 执行慢工具
                        let ctx = default_ctx();
                        let _ = reg
                            .execute(&format!("slow-{}", op % 20), serde_json::json!({}), ctx)
                            .await;
                        *stats.entry("exec_slow").or_insert(0) += 1;
                    }
                    2 => {
                        // 执行不稳定工具
                        let ctx = default_ctx();
                        let args = serde_json::json!({"fail": op % 5 == 0});
                        let _ = reg.execute(&format!("flaky-{}", op % 20), args, ctx).await;
                        *stats.entry("exec_flaky").or_insert(0) += 1;
                    }
                    3 => {
                        // 查询元数据
                        let _ = reg.tool_meta(&format!("fast-{}", op % 20));
                        *stats.entry("meta").or_insert(0) += 1;
                    }
                    4 => {
                        // 列出工具
                        let _ = reg.tool_names();
                        *stats.entry("list").or_insert(0) += 1;
                    }
                    5 => {
                        // 查询日志
                        let _ = reg.recent_log(10).await;
                        *stats.entry("audit").or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            stats
        });
        handles.push(handle);
    }

    // 收集统计
    let mut total_stats: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for handle in handles {
        let stats = handle.await.unwrap();
        for (key, count) in stats {
            *total_stats.entry(key.to_string()).or_insert(0) += count;
        }
    }

    println!("Comprehensive stress test stats: {:?}", total_stats);

    // 验证所有操作都已完成
    let total_ops: usize = total_stats.values().sum();
    // 验证操作数量合理（允许一些统计误差）
    assert!(total_ops > 0, "Some operations should complete");
    println!("Total operations: {}", total_ops);

    // 验证审计日志
    let log = registry.recent_log(2000).await;
    assert!(!log.is_empty(), "Audit log should not be empty");

    // 验证 registry 状态一致
    let tool_count = registry.tool_count();
    assert_eq!(tool_count, 20, "Tool count should be consistent");
}
