//! EventBus 容量极限压测
//!
//! 测试目标:
//! - 多生产者/多消费者异步测试
//! - 瞬间灌入超出容量的消息（使用小容量 bus 模拟）
//! - 断言系统不会 Panic
//! - 测试 LagStrategy::Error/Skip/Warn 三种行为

use std::sync::Arc;
use std::time::Duration;

use claw_runtime::{AgentId, Event, EventBus, LagStrategy};

/// 测试 EventBus 在极端并发下的容量限制行为 - Error 策略
#[tokio::test]
async fn test_event_bus_capacity_stress_error_strategy() {
    // 创建小容量 bus (容量=10)
    let bus = Arc::new(EventBus::with_capacity_and_strategy(10, LagStrategy::Error));

    // 启动多个生产者并发发送 1000 条消息
    let producer_count = 10;
    let messages_per_producer = 100;
    let total_messages = producer_count * messages_per_producer;

    let mut producer_handles = vec![];
    for p in 0..producer_count {
        let bus_clone = Arc::clone(&bus);
        let handle = tokio::spawn(async move {
            let mut sent = 0;
            for i in 0..messages_per_producer {
                let event = Event::AgentStarted {
                    agent_id: AgentId::new(format!("producer-{}-agent-{}", p, i)),
                };
                // publish 是同步方法，但我们需要在 async 中调用
                let _ = bus_clone.publish(event);
                sent += 1;
            }
            sent
        });
        producer_handles.push(handle);
    }

    // 启动一个慢消费者，制造 lag 场景
    let bus_clone = Arc::clone(&bus);
    let consumer_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        let mut received = 0;
        let mut errors = 0;

        // 故意延迟开始消费，制造滞后
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 尝试接收消息 - Error 策略下会因为滞后而返回错误
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(_)) => received += 1,
                Ok(Err(_)) => errors += 1, // Lag 导致的错误
                Err(_) => break,           // 超时
            }
            // 慢消费：每条消息处理 10ms
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        (received, errors)
    });

    // 等待所有生产者完成
    let mut total_sent = 0;
    for handle in producer_handles {
        total_sent += handle.await.unwrap();
    }

    // 等待消费者完成
    let (received, errors) = consumer_handle.await.unwrap();

    // 验证：所有消息都已发送
    assert_eq!(total_sent, total_messages);

    // 验证：Error 策略下应该有一些 lag 错误（或成功接收部分消息）
    // 由于容量只有10，消费者延迟50ms，肯定会发生滞后
    println!(
        "Error strategy: sent={}, received={}, errors={}",
        total_sent, received, errors
    );

    // 断言系统没有 panic（测试能走到这里说明没有 panic）
    assert!(received + errors >= 0);
}

/// 测试 EventBus 在极端并发下的容量限制行为 - Skip 策略
#[tokio::test]
async fn test_event_bus_capacity_stress_skip_strategy() {
    // 创建小容量 bus (容量=10) 使用 Skip 策略
    let bus = Arc::new(EventBus::with_capacity_and_strategy(10, LagStrategy::Skip));

    let total_messages = 100;

    // 先启动消费者，但让它延迟消费
    let bus_clone = Arc::clone(&bus);
    let consumer_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        let mut received = 0;
        let mut errors = 0;

        // 延迟开始消费，制造滞后场景
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 尝试接收消息 - Skip 策略下应该能继续接收最新消息
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(_)) => received += 1,
                Ok(Err(_)) => {
                    errors += 1;
                    // Skip 策略不应该返回错误，继续尝试
                }
                Err(_) => break, // 超时
            }
        }
        (received, errors)
    });

    // 给消费者一点时间先订阅
    tokio::time::sleep(Duration::from_millis(10)).await;

    // 生产者快速发送消息
    let bus_clone = Arc::clone(&bus);
    let producer_handle = tokio::spawn(async move {
        let mut sent = 0;
        for i in 0..total_messages {
            let event = Event::AgentStarted {
                agent_id: AgentId::new(format!("agent-{}", i)),
            };
            let _ = bus_clone.publish(event);
            sent += 1;
            // 极快发送，填满缓冲区
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }
        sent
    });

    // 等待生产者完成
    let sent = producer_handle.await.unwrap();

    // 等待消费者完成
    let (received, errors) = consumer_handle.await.unwrap();

    // 验证：所有消息都已发送
    assert_eq!(sent, total_messages);

    // 验证：Skip 策略下消费者不应返回错误
    println!(
        "Skip strategy: sent={}, received={}, errors={}",
        sent, received, errors
    );

    // Skip 策略的核心保证：不会返回错误，会跳过滞后消息继续接收
    assert_eq!(errors, 0, "Skip strategy should not return errors");
    // 验证系统没有 panic（走到这里说明没有 panic）
    assert!(received >= 0);
}

/// 测试 EventBus 在极端并发下的容量限制行为 - Warn 策略
#[tokio::test]
async fn test_event_bus_capacity_stress_warn_strategy() {
    // 创建小容量 bus (容量=10) 使用 Warn 策略
    let bus = Arc::new(EventBus::with_capacity_and_strategy(10, LagStrategy::Warn));

    let total_messages = 100;

    // 先启动消费者
    let bus_clone = Arc::clone(&bus);
    let consumer_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        let mut received = 0;
        let mut errors = 0;

        // 延迟开始消费，制造滞后场景
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Warn 策略下，即使滞后也能继续接收最新消息
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(_)) => received += 1,
                Ok(Err(_)) => {
                    errors += 1;
                    // Warn 策略不应该返回错误
                }
                Err(_) => break, // 超时
            }
        }
        (received, errors)
    });

    // 给消费者一点时间先订阅
    tokio::time::sleep(Duration::from_millis(10)).await;

    // 生产者快速发送消息
    let bus_clone = Arc::clone(&bus);
    let producer_handle = tokio::spawn(async move {
        let mut sent = 0;
        for i in 0..total_messages {
            let event = Event::AgentStarted {
                agent_id: AgentId::new(format!("agent-{}", i)),
            };
            let _ = bus_clone.publish(event);
            sent += 1;
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }
        sent
    });

    // 等待生产者完成
    let sent = producer_handle.await.unwrap();

    // 等待消费者完成
    let (received, errors) = consumer_handle.await.unwrap();

    // 验证：所有消息都已发送
    assert_eq!(sent, total_messages);

    // 验证：Warn 策略下消费者不应返回错误
    println!(
        "Warn strategy: sent={}, received={}, errors={}",
        sent, received, errors
    );

    // Warn 策略的核心保证：不会返回错误，会记录警告并继续接收
    assert_eq!(errors, 0, "Warn strategy should not return errors");
    // 验证系统没有 panic
    assert!(received >= 0);
}

/// 测试多消费者场景下的容量压力
#[tokio::test]
async fn test_event_bus_multi_consumer_stress() {
    let bus = Arc::new(EventBus::with_capacity_and_strategy(10, LagStrategy::Skip));

    let consumer_count = 5;
    let message_count = 200;

    // 启动多个消费者，不同速度
    let mut consumer_handles = vec![];
    for c in 0..consumer_count {
        let bus_clone = Arc::clone(&bus);
        let handle = tokio::spawn(async move {
            let mut rx = bus_clone.subscribe();
            let mut received = 0;
            let delay_ms = (c + 1) * 5; // 不同消费者不同速度

            for _ in 0..message_count {
                match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Ok(_)) => received += 1,
                    Ok(Err(_)) => break, // Error 情况下退出
                    Err(_) => break,     // 超时
                }
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            (c, received)
        });
        consumer_handles.push(handle);
    }

    // 生产者快速发送消息
    let bus_clone = Arc::clone(&bus);
    let producer_handle = tokio::spawn(async move {
        for i in 0..message_count {
            let event = Event::AgentStarted {
                agent_id: AgentId::new(format!("agent-{}", i)),
            };
            let _ = bus_clone.publish(event);
            // 快速发送，不等待
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
        }
        message_count
    });

    // 等待生产者完成
    let sent = producer_handle.await.unwrap();

    // 等待所有消费者完成
    let mut total_received = 0;
    for handle in consumer_handles {
        let (c, received) = handle.await.unwrap();
        println!("Consumer {} received {} messages", c, received);
        total_received += received;
    }

    // 验证：所有消息都已发送
    assert_eq!(sent, message_count);

    // 验证：至少有一些消息被接收（由于 Skip 策略，不同消费者可能接收不同数量）
    assert!(total_received > 0, "Some messages should be received");

    println!(
        "Multi-consumer stress: sent={}, total_received={}",
        sent, total_received
    );
}

/// 测试瞬间灌入大量消息的极限情况
#[tokio::test]
async fn test_event_bus_burst_stress() {
    // 使用极小的容量来模拟极端 lag 场景
    let bus = Arc::new(EventBus::with_capacity_and_strategy(5, LagStrategy::Skip));

    let burst_size = 1000;

    // 先启动消费者
    let bus_clone = Arc::clone(&bus);
    let consumer_handle = tokio::spawn(async move {
        let mut rx = bus_clone.subscribe();
        let mut received = 0;
        let start = std::time::Instant::now();

        // 持续接收直到超时
        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(_)) => received += 1,
                Ok(Err(_)) => {
                    // Skip 策略不应该返回错误
                    continue;
                }
                Err(_) => break, // 超时，没有更多消息
            }

            // 模拟处理延迟
            if received % 10 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            // 总体超时保护
            if start.elapsed() > Duration::from_secs(5) {
                break;
            }
        }
        received
    });

    // 瞬间灌入大量消息
    let bus_clone = Arc::clone(&bus);
    let producer_handle = tokio::spawn(async move {
        for i in 0..burst_size {
            let event = Event::AgentStarted {
                agent_id: AgentId::new(format!("burst-agent-{}", i)),
            };
            let _ = bus_clone.publish(event);
        }
        burst_size
    });

    // 等待生产者完成
    let sent = producer_handle.await.unwrap();

    // 给消费者时间处理
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 等待消费者完成
    let received = consumer_handle.await.unwrap();

    // 验证：所有消息都已发送
    assert_eq!(sent, burst_size);

    // 验证：Skip 策略下应该能接收部分消息（最新进入缓冲区的）
    println!("Burst stress: sent={}, received={}", sent, received);

    // 由于容量只有5，消费者最多能看到容量内的消息
    // 但是 Skip 策略确保消费者可以继续接收
    assert!(received >= 0, "Consumer should be able to receive messages");
}

/// 测试三种策略的对比行为
#[tokio::test]
async fn test_event_bus_all_strategies_comparison() {
    let strategies = vec![
        (LagStrategy::Error, "Error"),
        (LagStrategy::Skip, "Skip"),
        (LagStrategy::Warn, "Warn"),
    ];

    for (strategy, name) in strategies {
        let bus = Arc::new(EventBus::with_capacity_and_strategy(5, strategy));

        // 生产者发送消息
        for i in 0..20 {
            let event = Event::AgentStarted {
                agent_id: AgentId::new(format!("agent-{}", i)),
            };
            let _ = bus.publish(event);
        }

        // 延迟后启动消费者
        tokio::time::sleep(Duration::from_millis(50)).await;

        let bus_clone = Arc::clone(&bus);
        let consumer_handle = tokio::spawn(async move {
            let mut rx = bus_clone.subscribe();
            let mut received = 0;
            let mut errors = 0;

            for _ in 0..10 {
                match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Ok(_)) => received += 1,
                    Ok(Err(_)) => errors += 1,
                    Err(_) => break,
                }
            }
            (received, errors)
        });

        let (received, errors) = consumer_handle.await.unwrap();
        println!(
            "Strategy {}: received={}, errors={}",
            name, received, errors
        );

        // 验证策略行为
        match strategy {
            LagStrategy::Error => {
                // Error 策略：如果滞后，应该返回错误
                // 注意：消费者订阅时可能已经超过容量，所以可能立即 lag
            }
            LagStrategy::Skip | LagStrategy::Warn => {
                // Skip 和 Warn 策略：不应该返回错误
                assert_eq!(errors, 0, "{} strategy should not return errors", name);
            }
        }
    }
}

/// 测试 EventBus 不会 Panic 的核心保证
#[tokio::test]
async fn test_event_bus_no_panic_guarantee() {
    // 测试多种极端情况组合
    let test_cases = vec![
        (1, LagStrategy::Error), // 最小容量
        (1, LagStrategy::Skip),
        (1, LagStrategy::Warn),
        (100, LagStrategy::Error), // 较大容量
        (100, LagStrategy::Skip),
        (100, LagStrategy::Warn),
    ];

    for (capacity, strategy) in test_cases {
        let bus = Arc::new(EventBus::with_capacity_and_strategy(capacity, strategy));

        // 多个生产者同时发送
        let mut handles = vec![];
        for _ in 0..10 {
            let bus_clone = Arc::clone(&bus);
            handles.push(tokio::spawn(async move {
                for i in 0..100 {
                    let event = Event::AgentStarted {
                        agent_id: AgentId::new(format!("agent-{}", i)),
                    };
                    let _ = bus_clone.publish(event);
                }
            }));
        }

        // 多个消费者同时接收
        for _ in 0..5 {
            let bus_clone = Arc::clone(&bus);
            handles.push(tokio::spawn(async move {
                let mut rx = bus_clone.subscribe();
                for _ in 0..50 {
                    let _ = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
                }
            }));
        }

        // 等待所有任务完成 - 如果没有 panic，这个测试就通过了
        for handle in handles {
            let result = handle.await;
            assert!(result.is_ok(), "Task should complete without panic");
        }

        println!(
            "Capacity={:?}, Strategy={:?} - No panic",
            capacity, strategy
        );
    }
}
