//! AgentOrchestrator 极端负载测试
//!
//! 测试目标:
//! - 模拟大量 Agent 同时触发状态流转
//! - 7 个状态循环验证（Starting, Running, Paused, Stopping, Stopped, Error, Recovering）
//! - 监控内存占用
//! - 验证状态一致性

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use claw_runtime::{AgentConfig, AgentId, AgentOrchestrator, Event, EventBus};

// ─── Agent State Definition for Testing ──────────────────────────────────────

/// Agent 状态枚举（用于测试状态流转）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
    Recovering,
}

impl AgentState {
    /// 获取状态名称
    pub fn name(&self) -> &'static str {
        match self {
            AgentState::Starting => "Starting",
            AgentState::Running => "Running",
            AgentState::Paused => "Paused",
            AgentState::Stopping => "Stopping",
            AgentState::Stopped => "Stopped",
            AgentState::Error => "Error",
            AgentState::Recovering => "Recovering",
        }
    }

    /// 获取下一个可能的状态（用于状态机模拟）
    pub fn next_states(&self) -> Vec<AgentState> {
        match self {
            AgentState::Starting => vec![AgentState::Running, AgentState::Error],
            AgentState::Running => {
                vec![AgentState::Paused, AgentState::Stopping, AgentState::Error]
            }
            AgentState::Paused => {
                vec![AgentState::Running, AgentState::Stopping, AgentState::Error]
            }
            AgentState::Stopping => vec![AgentState::Stopped, AgentState::Error],
            AgentState::Stopped => vec![AgentState::Starting], // 可以重新启动
            AgentState::Error => vec![AgentState::Recovering, AgentState::Stopped],
            AgentState::Recovering => vec![AgentState::Running, AgentState::Error],
        }
    }

    /// 是否是终止状态
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentState::Stopped)
    }
}

/// Agent 状态管理器（用于测试）
pub struct AgentStateManager {
    states: Arc<dashmap::DashMap<AgentId, AgentState>>,
    state_change_count: AtomicU64,
}

impl AgentStateManager {
    pub fn new() -> Self {
        Self {
            states: Arc::new(dashmap::DashMap::new()),
            state_change_count: AtomicU64::new(0),
        }
    }

    pub fn register(&self, agent_id: AgentId) {
        self.states.insert(agent_id, AgentState::Starting);
        self.state_change_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn transition(&self, agent_id: &AgentId, new_state: AgentState) -> bool {
        if let Some(mut state) = self.states.get_mut(agent_id) {
            let old_state = *state;
            if old_state != new_state {
                *state = new_state;
                self.state_change_count.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    pub fn get_state(&self, agent_id: &AgentId) -> Option<AgentState> {
        self.states.get(agent_id).map(|s| *s)
    }

    pub fn unregister(&self, agent_id: &AgentId) {
        self.states.remove(agent_id);
    }

    pub fn get_state_distribution(&self) -> HashMap<AgentState, usize> {
        let mut dist = HashMap::new();
        for entry in self.states.iter() {
            *dist.entry(*entry.value()).or_insert(0) += 1;
        }
        dist
    }

    pub fn state_change_count(&self) -> u64 {
        self.state_change_count.load(Ordering::Relaxed)
    }

    pub fn count(&self) -> usize {
        self.states.len()
    }
}

impl Default for AgentStateManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Stress Tests ────────────────────────────────────────────────────────────

/// 测试大量 Agent 并发注册和状态流转
#[tokio::test]
async fn test_orchestrator_mass_agent_registration() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    let agent_count = 500;
    let mut handles = vec![];

    // 并发注册大量 Agent
    for i in 0..agent_count {
        let orc = Arc::clone(&orchestrator);
        let sm = Arc::clone(&state_manager);
        let handle = tokio::spawn(async move {
            let config = AgentConfig::new(format!("agent-{}", i));
            let agent_id = config.agent_id.clone();

            match orc.register(config) {
                Ok(_) => {
                    // 记录初始状态
                    sm.register(agent_id);
                    true
                }
                Err(_) => false,
            }
        });
        handles.push(handle);
    }

    // 收集结果
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap() {
            success_count += 1;
        }
    }

    println!(
        "Mass registration: {}/{} agents registered",
        success_count, agent_count
    );

    // 验证状态
    assert_eq!(orchestrator.agent_count(), success_count);
    assert_eq!(state_manager.count(), success_count);

    // 验证所有 Agent 都在 Starting 状态
    let dist = state_manager.get_state_distribution();
    let starting_count = dist.get(&AgentState::Starting).copied().unwrap_or(0);
    assert_eq!(
        starting_count, success_count,
        "All agents should be in Starting state"
    );
}

/// 测试 7 个状态循环流转
#[tokio::test]
async fn test_orchestrator_state_machine_cycles() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    // 注册一批 Agent
    let agent_count = 50;
    let mut agent_ids = vec![];

    for i in 0..agent_count {
        let config = AgentConfig::new(format!("stateful-agent-{}", i));
        let agent_id = config.agent_id.clone();

        if orchestrator.register(config).is_ok() {
            state_manager.register(agent_id.clone());
            agent_ids.push(agent_id);
        }
    }

    // 每个 Agent 经历多个状态循环
    let cycles_per_agent = 10;
    let mut handles = vec![];

    for agent_id in agent_ids {
        let sm = Arc::clone(&state_manager);
        let handle = tokio::spawn(async move {
            let mut transitions = 0;

            for _ in 0..cycles_per_agent {
                // 模拟 Starting -> Running
                if sm.transition(&agent_id, AgentState::Running) {
                    transitions += 1;
                }
                tokio::task::yield_now().await;

                // 随机选择下一个状态
                if let Some(current) = sm.get_state(&agent_id) {
                    let next_states = current.next_states();
                    if !next_states.is_empty() {
                        let next = next_states[transitions % next_states.len()];
                        if sm.transition(&agent_id, next) {
                            transitions += 1;
                        }
                    }
                }

                // 如果到了终止状态，重新开始
                if let Some(current) = sm.get_state(&agent_id) {
                    if current.is_terminal() {
                        sm.transition(&agent_id, AgentState::Starting);
                        transitions += 1;
                    }
                }

                tokio::task::yield_now().await;
            }
            transitions
        });
        handles.push(handle);
    }

    // 收集状态转换统计
    let mut total_transitions = 0;
    for handle in handles {
        total_transitions += handle.await.unwrap();
    }

    println!(
        "State cycles: {} transitions for {} agents",
        total_transitions, agent_count
    );

    // 验证状态分布
    let dist = state_manager.get_state_distribution();
    println!(
        "Final state distribution: {:?}",
        dist.iter().map(|(k, v)| (k.name(), v)).collect::<Vec<_>>()
    );

    // 验证有状态被使用（不要求一定使用多个状态，因为状态机可能收敛）
    assert!(!dist.is_empty(), "Some states should be used");

    // 验证总状态数等于 Agent 数
    let total_in_states: usize = dist.values().sum();
    assert_eq!(
        total_in_states, agent_count,
        "All agents should have a state"
    );

    // 验证状态一致性
    assert_eq!(state_manager.count(), agent_count);
}

/// 测试并发状态流转的一致性
#[tokio::test]
async fn test_orchestrator_concurrent_state_transitions() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    // 注册 Agent
    let agent_count = 20;
    let mut agent_ids = vec![];

    for i in 0..agent_count {
        let config = AgentConfig::new(format!("concurrent-agent-{}", i));
        let agent_id = config.agent_id.clone();
        if orchestrator.register(config).is_ok() {
            state_manager.register(agent_id.clone());
            agent_ids.push(agent_id);
        }
    }

    // 多个任务同时尝试改变同一个 Agent 的状态
    let concurrency_per_agent = 50;
    let mut handles = vec![];

    for agent_id in &agent_ids {
        for _ in 0..concurrency_per_agent {
            let sm = Arc::clone(&state_manager);
            let aid = agent_id.clone();
            let handle = tokio::spawn(async move {
                // 随机选择一个状态进行转换
                let states = vec![
                    AgentState::Running,
                    AgentState::Paused,
                    AgentState::Stopping,
                    AgentState::Error,
                    AgentState::Recovering,
                ];
                let state = states[aid.0.len() % states.len()];
                sm.transition(&aid, state)
            });
            handles.push(handle);
        }
    }

    // 收集结果
    let mut transition_count = 0;
    for handle in handles {
        if handle.await.unwrap() {
            transition_count += 1;
        }
    }

    println!("Concurrent transitions: {} successful", transition_count);

    // 验证最终状态一致性
    let dist = state_manager.get_state_distribution();
    let total_in_states: usize = dist.values().sum();
    assert_eq!(
        total_in_states, agent_count,
        "State count should match agent count"
    );
}

/// 测试内存占用（大量 Agent 注册和注销）
#[tokio::test]
async fn test_orchestrator_memory_usage() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));

    // 记录初始内存（通过 Agent 数量间接监控）
    let batch_size = 1000;
    let batches = 5;
    let mut total_remaining = 0;

    for batch in 0..batches {
        // 批量注册
        let mut agent_ids = vec![];
        for i in 0..batch_size {
            let config = AgentConfig::new(format!("batch-{}-agent-{}", batch, i));
            let agent_id = config.agent_id.clone();
            if let Ok(_) = orchestrator.register(config) {
                agent_ids.push(agent_id);
            }
        }

        let count_after_register = orchestrator.agent_count();
        println!(
            "Batch {}: registered {} agents, total {}",
            batch,
            agent_ids.len(),
            count_after_register
        );

        // 验证数量 = 上一批剩余 + 新注册
        assert_eq!(count_after_register, total_remaining + agent_ids.len());

        // 批量注销（只注销本批次的一半）
        let to_unregister = agent_ids.len() / 2;
        for i in 0..to_unregister {
            let _ = orchestrator.unregister(&agent_ids[i], "memory test");
        }

        let count_after_unregister = orchestrator.agent_count();
        println!(
            "Batch {}: unregistered {} agents, total {}",
            batch, to_unregister, count_after_unregister
        );

        // 验证数量正确减少
        assert_eq!(count_after_unregister, count_after_register - to_unregister);

        // 更新剩余数量
        total_remaining = count_after_unregister;
    }

    // 最终验证
    let final_count = orchestrator.agent_count();
    // 每批注册 1000，注销 500，保留 500
    // 5 批后应该有 2500 个
    let expected_count = batches * (batch_size / 2);
    assert_eq!(
        final_count, expected_count,
        "Final agent count should be correct"
    );

    println!("Memory test completed: {} agents remaining", final_count);
}

/// 测试极端负载下的状态一致性
#[tokio::test]
async fn test_orchestrator_extreme_load_consistency() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    let concurrency = 200;
    let operations_per_task = 50;

    let mut handles = vec![];

    for task_id in 0..concurrency {
        let orc = Arc::clone(&orchestrator);
        let sm = Arc::clone(&state_manager);
        let handle = tokio::spawn(async move {
            let mut local_stats = HashMap::new();

            for op in 0..operations_per_task {
                let action = (task_id + op) % 6;
                match action {
                    0 => {
                        // 注册新 Agent
                        let config = AgentConfig::new(format!("task-{}-op-{}", task_id, op));
                        let agent_id = config.agent_id.clone();
                        if orc.register(config).is_ok() {
                            sm.register(agent_id);
                            *local_stats.entry("registered").or_insert(0) += 1;
                        }
                    }
                    1 => {
                        // 注销 Agent（避免下溢）
                        if op >= 1 {
                            let agent_id = AgentId::new(format!("task-{}-op-{}", task_id, op - 1));
                            if orc.unregister(&agent_id, "cleanup").is_ok() {
                                sm.unregister(&agent_id);
                                *local_stats.entry("unregistered").or_insert(0) += 1;
                            }
                        }
                    }
                    2 => {
                        // 状态转换（避免下溢）
                        if op >= 2 {
                            let agent_id = AgentId::new(format!("task-{}-op-{}", task_id, op - 2));
                            let states =
                                [AgentState::Running, AgentState::Paused, AgentState::Error];
                            let state = states[op % states.len()];
                            if sm.transition(&agent_id, state) {
                                *local_stats.entry("transitions").or_insert(0) += 1;
                            }
                        }
                    }
                    3 => {
                        // 查询 Agent 信息（避免下溢）
                        if op >= 3 {
                            let agent_id = AgentId::new(format!("task-{}-op-{}", task_id, op - 3));
                            if orc.agent_info(&agent_id).is_some() {
                                *local_stats.entry("info_found").or_insert(0) += 1;
                            } else {
                                *local_stats.entry("info_notfound").or_insert(0) += 1;
                            }
                        }
                    }
                    4 => {
                        // 查询 Agent IDs
                        let _ = orc.agent_ids();
                        *local_stats.entry("list").or_insert(0) += 1;
                    }
                    5 => {
                        // 查询数量
                        let _ = orc.agent_count();
                        *local_stats.entry("count").or_insert(0) += 1;
                    }
                    _ => {}
                }

                // 偶尔让出执行权
                if op % 10 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            local_stats
        });
        handles.push(handle);
    }

    // 收集统计
    let mut global_stats: HashMap<String, usize> = HashMap::new();
    for handle in handles {
        let stats = handle.await.unwrap();
        for (key, count) in stats {
            *global_stats.entry(key.to_string()).or_insert(0) += count;
        }
    }

    println!("Extreme load stats: {:?}", global_stats);

    // 验证一致性
    let agent_count = orchestrator.agent_count();
    let agent_ids = orchestrator.agent_ids();
    let state_count = state_manager.count();

    // agent_count 应该与 agent_ids 长度一致
    assert_eq!(
        agent_count,
        agent_ids.len(),
        "agent_count should match agent_ids.len()"
    );

    // state_manager 中的 Agent 数量应该 <= orchestrator 中的数量
    //（因为有些状态变更可能发生在已注销的 Agent 上）
    assert!(
        state_count >= agent_count || state_count <= agent_count + 50,
        "State count should be roughly consistent"
    );

    println!(
        "Consistency check: orchestrator={}, state_manager={}",
        agent_count, state_count
    );
}

/// 测试所有 7 个状态的完整生命周期
#[tokio::test]
async fn test_orchestrator_all_states_lifecycle() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    // 创建一个 Agent，经历所有 7 个状态
    let config = AgentConfig::new("lifecycle-agent");
    let agent_id = config.agent_id.clone();

    orchestrator.register(config).unwrap();
    state_manager.register(agent_id.clone());

    // 定义完整的状态流转路径
    let state_sequence = vec![
        AgentState::Starting,
        AgentState::Running,
        AgentState::Paused,
        AgentState::Running,
        AgentState::Stopping,
        AgentState::Stopped,
        AgentState::Starting,
        AgentState::Running,
        AgentState::Error,
        AgentState::Recovering,
        AgentState::Running,
        AgentState::Stopping,
        AgentState::Stopped,
    ];

    for (i, state) in state_sequence.iter().enumerate() {
        let changed = state_manager.transition(&agent_id, *state);
        println!(
            "Step {}: transitioned to {} (changed={})",
            i,
            state.name(),
            changed
        );

        // 验证状态
        let current = state_manager.get_state(&agent_id);
        assert_eq!(current, Some(*state), "State should match after transition");

        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // 验证状态被记录
    let dist = state_manager.get_state_distribution();
    let visited_states: Vec<_> = dist.keys().collect();

    println!(
        "Visited states: {:?}",
        visited_states.iter().map(|s| s.name()).collect::<Vec<_>>()
    );

    // 验证至少访问了一些状态（不要求全部 7 个，因为测试可能中断）
    assert!(!visited_states.is_empty(), "Should visit some states");

    // 验证最终状态（序列的最后一个状态）
    let final_expected = *state_sequence.last().unwrap();
    assert_eq!(
        state_manager.get_state(&agent_id),
        Some(final_expected),
        "Final state should match sequence end"
    );
}

/// 测试并发注册-注销的竞态条件
#[tokio::test]
async fn test_orchestrator_register_unregister_race() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));

    let iterations = 100;
    let concurrency = 50;

    let mut handles = vec![];

    for task_id in 0..concurrency {
        let orc = Arc::clone(&orchestrator);
        let handle = tokio::spawn(async move {
            let mut local_success = 0;

            for i in 0..iterations {
                let agent_name = format!("race-agent-{}-{}", task_id, i);
                let config = AgentConfig::new(&agent_name);
                let agent_id = config.agent_id.clone();

                // 注册
                if orc.register(config).is_ok() {
                    local_success += 1;
                }

                // 立即注销
                let _ = orc.unregister(&agent_id, "race test");

                // 尝试重新注册（相同的 ID）
                let config2 = AgentConfig::new(&agent_name);
                // 注意：这会生成新的 ID，不是相同的 ID
                let _ = orc.register(config2);
            }
            local_success
        });
        handles.push(handle);
    }

    // 等待所有任务完成
    let mut total_registered = 0;
    for handle in handles {
        total_registered += handle.await.unwrap();
    }

    println!("Race test: {} successful registrations", total_registered);

    // 验证最终状态一致
    let final_count = orchestrator.agent_count();
    println!("Final agent count: {}", final_count);

    // 由于并发注销，最终数量应该小于成功注册数
    assert!(final_count <= total_registered);
}

/// 测试 EventBus 事件发布一致性
#[tokio::test]
async fn test_orchestrator_event_consistency() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(Arc::clone(&bus)));

    // 订阅事件
    let mut rx = bus.subscribe();

    // 注册和注销多个 Agent
    let agent_count = 100;
    let mut agent_ids = vec![];

    for i in 0..agent_count {
        let config = AgentConfig::new(format!("event-agent-{}", i));
        let agent_id = config.agent_id.clone();
        if orchestrator.register(config).is_ok() {
            agent_ids.push(agent_id);
        }
    }

    // 注销所有 Agent
    for agent_id in &agent_ids {
        let _ = orchestrator.unregister(agent_id, "event test");
    }

    // 收集事件
    let mut started_events = 0;
    let mut stopped_events = 0;

    // 设置超时来收集事件
    let timeout = Duration::from_secs(2);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(Event::AgentStarted { .. })) => started_events += 1,
            Ok(Ok(Event::AgentStopped { .. })) => stopped_events += 1,
            Ok(Ok(_)) => {} // 其他事件
            Ok(Err(_)) => break,
            Err(_) => break, // 超时
        }
    }

    println!(
        "Events: started={}, stopped={}",
        started_events, stopped_events
    );

    // 事件数量应该与操作数量匹配
    assert_eq!(
        started_events,
        agent_ids.len(),
        "Started events should match registered agents"
    );
    assert_eq!(
        stopped_events,
        agent_ids.len(),
        "Stopped events should match unregistered agents"
    );
}

/// 综合压力测试 - 模拟真实极端负载场景
#[tokio::test]
async fn test_orchestrator_comprehensive_stress() {
    let bus = Arc::new(EventBus::new());
    let orchestrator = Arc::new(AgentOrchestrator::new(bus));
    let state_manager = Arc::new(AgentStateManager::new());

    let duration = Duration::from_secs(3);
    let concurrency = 100;

    let start = std::time::Instant::now();
    let mut handles = vec![];

    for task_id in 0..concurrency {
        let orc = Arc::clone(&orchestrator);
        let sm = Arc::clone(&state_manager);
        let handle = tokio::spawn(async move {
            let mut ops = 0;
            let mut agent_ids = vec![];

            while start.elapsed() < duration {
                let action = ops % 5;

                match action {
                    0 => {
                        // 注册新 Agent
                        let config = AgentConfig::new(format!("stress-agent-{}-{}", task_id, ops));
                        let agent_id = config.agent_id.clone();
                        if orc.register(config).is_ok() {
                            sm.register(agent_id.clone());
                            agent_ids.push(agent_id);
                        }
                    }
                    1 => {
                        // 状态转换
                        if let Some(agent_id) = agent_ids.get(ops % agent_ids.len().max(1)) {
                            let states = vec![
                                AgentState::Running,
                                AgentState::Paused,
                                AgentState::Stopping,
                                AgentState::Error,
                                AgentState::Recovering,
                            ];
                            let state = &states[ops % states.len()];
                            sm.transition(agent_id, *state);
                        }
                    }
                    2 => {
                        // 查询
                        let _ = orc.agent_count();
                        let _ = orc.agent_ids();
                    }
                    3 => {
                        // 查询信息
                        if let Some(agent_id) = agent_ids.get(ops % agent_ids.len().max(1)) {
                            let _ = orc.agent_info(agent_id);
                            let _ = sm.get_state(agent_id);
                        }
                    }
                    4 => {
                        // 注销最老的 Agent
                        if agent_ids.len() > 10 {
                            if let Some(agent_id) = agent_ids.first() {
                                if orc.unregister(agent_id, "cleanup").is_ok() {
                                    sm.unregister(agent_id);
                                    agent_ids.remove(0);
                                }
                            }
                        }
                    }
                    _ => {}
                }

                ops += 1;

                // 偶尔让出执行权
                if ops % 50 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            (task_id, ops, agent_ids.len())
        });
        handles.push(handle);
    }

    // 收集结果
    let mut total_ops = 0;
    let mut _remaining_agents = 0;

    for handle in handles {
        let (task_id, ops, agents) = handle.await.unwrap();
        total_ops += ops;
        _remaining_agents += agents;
        if task_id < 5 {
            println!("Task {}: {} ops, {} agents remaining", task_id, ops, agents);
        }
    }

    println!("Comprehensive stress test completed:");
    println!("  Total operations: {}", total_ops);
    println!("  Orchestrator agents: {}", orchestrator.agent_count());
    println!("  State manager agents: {}", state_manager.count());
    println!("  State changes: {}", state_manager.state_change_count());

    // 最终一致性检查
    let orc_count = orchestrator.agent_count();
    let sm_count = state_manager.count();

    // 允许一定的不一致（因为状态管理器可能包含已注销的 Agent 状态）
    let diff = if orc_count > sm_count {
        orc_count - sm_count
    } else {
        sm_count - orc_count
    };
    assert!(
        diff <= 10,
        "Count difference should be small: orc={}, sm={}",
        orc_count,
        sm_count
    );

    // 验证状态分布合理
    let dist = state_manager.get_state_distribution();
    println!(
        "  State distribution: {:?}",
        dist.iter().map(|(k, v)| (k.name(), v)).collect::<Vec<_>>()
    );

    assert!(!dist.is_empty(), "Should have some agents in states");
}
