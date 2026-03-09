//! Tokio-based implementation of the Scheduler trait.

use super::{ScheduleError, Scheduler, TaskConfig, TaskId, TaskStats, TaskTrigger};
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant};

/// Internal task state.
struct TaskState {
    config: TaskConfig,
    stats: RwLock<TaskStats>,
    handle: Mutex<Option<JoinHandle<()>>>,
    is_paused: AtomicU64,
}

impl TaskState {
    fn new(config: TaskConfig) -> Self {
        Self {
            config,
            stats: RwLock::new(TaskStats::default()),
            handle: Mutex::new(None),
            is_paused: AtomicU64::new(0),
        }
    }

    fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::Relaxed) != 0
    }

    fn set_paused(&self, paused: bool) {
        self.is_paused.store(if paused { 1 } else { 0 }, Ordering::Relaxed);
    }
}

/// Tokio-based scheduler implementation.
///
/// Uses tokio::time for interval-based tasks and a background task
/// for cron-like scheduling.
pub struct TokioScheduler {
    tasks: Arc<DashMap<TaskId, Arc<TaskState>>>,
    shutdown: Mutex<bool>,
}

impl TokioScheduler {
    /// Create a new TokioScheduler.
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(DashMap::new()),
            shutdown: Mutex::new(false),
        }
    }

    /// Run a single task execution.
    async fn execute_task(state: Arc<TaskState>) {
        // Check if paused
        if state.is_paused() {
            return;
        }

        // Check max executions
        if let Some(max) = state.config.max_executions {
            let current = state.stats.read().await.execution_count;
            if current >= max {
                return;
            }
        }

        // Update stats
        {
            let mut stats = state.stats.write().await;
            stats.execution_count += 1;
            stats.last_execution = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
        }

        // Execute handler (with basic error handling)
        let handler = state.config.handler.clone();
        let result: Result<(), ()> = {
            let fut = handler();
            match fut.await {
                _ => Ok(()), // Task completed (we don't have result type, so assume success)
            }
        };

        // Update stats based on result
        {
            let mut stats = state.stats.write().await;
            match result {
                Ok(_) => stats.success_count += 1,
                Err(_) => stats.failure_count += 1,
            }
        }
    }

    /// Spawn an interval-based task.
    fn spawn_interval_task(
        &self,
        state: Arc<TaskState>,
        duration: Duration,
    ) -> JoinHandle<()> {
        let tasks = Arc::clone(&self.tasks);
        let task_id = state.config.id.clone();

        tokio::spawn(async move {
            // Handle immediate execution
            if state.config.trigger.is_immediate() {
                Self::execute_task(Arc::clone(&state)).await;
            }

            let start = Instant::now() + duration;
            let mut ticker = interval_at(start, duration);

            loop {
                ticker.tick().await;

                // Check if task still exists
                if !tasks.contains_key(&task_id) {
                    break;
                }

                // Check max executions
                if let Some(max) = state.config.max_executions {
                    let current = state.stats.read().await.execution_count;
                    if current >= max {
                        tasks.remove(&task_id);
                        break;
                    }
                }

                Self::execute_task(Arc::clone(&state)).await;
            }
        })
    }

    /// Spawn a one-time task.
    fn spawn_once_task(&self, state: Arc<TaskState>, unix_secs: u64) -> JoinHandle<()> {
        let tasks = Arc::clone(&self.tasks);
        let task_id = state.config.id.clone();

        tokio::spawn(async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if unix_secs > now {
                let delay = Duration::from_secs(unix_secs - now);
                tokio::time::sleep(delay).await;
            }

            // Check if task still exists
            if tasks.contains_key(&task_id) {
                Self::execute_task(Arc::clone(&state)).await;
                tasks.remove(&task_id);
            }
        })
    }

    /// Spawn an immediate task.
    fn spawn_immediate_task(&self, state: Arc<TaskState>) -> JoinHandle<()> {
        let tasks = Arc::clone(&self.tasks);
        let task_id = state.config.id.clone();

        tokio::spawn(async move {
            Self::execute_task(Arc::clone(&state)).await;
            tasks.remove(&task_id);
        })
    }

    /// Spawn a cron-based task (simplified implementation).
    /// For production use, consider using the `cron` crate.
    fn spawn_cron_task(&self, state: Arc<TaskState>, expr: String) -> JoinHandle<()> {
        // This is a simplified implementation
        // In production, parse the cron expression and calculate next execution time
        let tasks = Arc::clone(&self.tasks);
        let task_id = state.config.id.clone();

        tokio::spawn(async move {
            // Parse simple patterns
            // "*/n * * * *" = every n minutes
            // "0 * * * *" = every hour
            let interval = Self::parse_simple_cron(&expr).unwrap_or(Duration::from_secs(3600));

            let mut ticker = tokio::time::interval(interval);

            loop {
                ticker.tick().await;

                if !tasks.contains_key(&task_id) {
                    break;
                }

                Self::execute_task(Arc::clone(&state)).await;
            }
        })
    }

    /// Parse simple cron patterns into duration.
    fn parse_simple_cron(expr: &str) -> Option<Duration> {
        let parts: Vec<&str> = expr.split_whitespace().collect();

        // Simple patterns: "*/n" for minutes
        if parts.len() >= 2 && parts[1].starts_with("*/") {
            let mins: u64 = parts[1][2..].parse().ok()?;
            return Some(Duration::from_secs(mins * 60));
        }

        // "0 * * * *" = hourly
        if parts.len() >= 2 && parts[0] == "0" && parts[1] == "*" {
            return Some(Duration::from_secs(3600));
        }

        // Default: hourly
        Some(Duration::from_secs(3600))
    }

    /// Get task statistics.
    pub async fn stats(&self, task_id: &TaskId) -> Option<TaskStats> {
        self.tasks.get(task_id).map(|t| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async { t.stats.read().await.clone() })
        })
    }
}

impl Default for TokioScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Scheduler for TokioScheduler {
    async fn schedule(&self, config: TaskConfig) -> Result<(), ScheduleError> {
        // Check shutdown state
        if *self.shutdown.lock().await {
            return Err(ScheduleError::ShuttingDown);
        }

        let task_id = config.id.clone();

        // Check for duplicate
        if self.tasks.contains_key(&task_id) {
            return Err(ScheduleError::TaskAlreadyExists(task_id.0));
        }

        let state = Arc::new(TaskState::new(config));

        // Get trigger type before spawning to avoid borrow issues
        let trigger = state.config.trigger.clone();

        // Spawn the appropriate task type
        let handle = {
            let state_clone = Arc::clone(&state);
            match trigger {
                TaskTrigger::Interval(duration) => {
                    self.spawn_interval_task(state_clone, duration)
                }
                TaskTrigger::Once(unix_secs) => self.spawn_once_task(state_clone, unix_secs),
                TaskTrigger::Immediate => self.spawn_immediate_task(state_clone),
                TaskTrigger::Cron(expr) => self.spawn_cron_task(state_clone, expr),
            }
        };

        // Store handle
        *state.handle.lock().await = Some(handle);

        // Store task
        self.tasks.insert(task_id, state);

        Ok(())
    }

    async fn cancel(&self, task_id: &TaskId) -> Result<(), ScheduleError> {
        let state = self
            .tasks
            .remove(task_id)
            .ok_or_else(|| ScheduleError::TaskNotFound(task_id.0.clone()))?
            .1;

        // Abort the task handle
        if let Some(handle) = state.handle.lock().await.take() {
            handle.abort();
        }

        Ok(())
    }

    async fn is_scheduled(&self, task_id: &TaskId) -> bool {
        self.tasks.contains_key(task_id)
    }

    async fn list_tasks(&self) -> Vec<TaskId> {
        self.tasks.iter().map(|t| t.key().clone()).collect()
    }

    async fn pause(&self, task_id: &TaskId) -> Result<(), ScheduleError> {
        let state = self
            .tasks
            .get(task_id)
            .ok_or_else(|| ScheduleError::TaskNotFound(task_id.0.clone()))?;

        state.set_paused(true);

        // Update stats
        let mut stats = state.stats.write().await;
        stats.is_paused = true;

        Ok(())
    }

    async fn resume(&self, task_id: &TaskId) -> Result<(), ScheduleError> {
        let state = self
            .tasks
            .get(task_id)
            .ok_or_else(|| ScheduleError::TaskNotFound(task_id.0.clone()))?;

        state.set_paused(false);

        // Update stats
        let mut stats = state.stats.write().await;
        stats.is_paused = false;

        Ok(())
    }

    async fn next_execution(&self, task_id: &TaskId) -> Option<u64> {
        let state = self.tasks.get(task_id)?;
        let stats = state.stats.read().await;
        stats.next_execution
    }

    async fn shutdown(&self) -> Result<(), ScheduleError> {
        *self.shutdown.lock().await = true;

        // Cancel all tasks
        for entry in self.tasks.iter() {
            let state = entry.value();
            if let Some(handle) = state.handle.lock().await.take() {
                handle.abort();
            }
        }

        self.tasks.clear();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_tokio_scheduler_new() {
        let scheduler = TokioScheduler::new();
        assert!(scheduler.list_tasks().await.is_empty());
    }

    #[tokio::test]
    async fn test_schedule_interval_task() {
        let scheduler = TokioScheduler::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        scheduler
            .schedule(TaskConfig::new(
                "test-interval",
                TaskTrigger::interval(Duration::from_millis(50)),
                move || {
                    let c = Arc::clone(&counter_clone);
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::Relaxed);
                    })
                },
            ))
            .await
            .unwrap();

        // Wait for a few executions
        tokio::time::sleep(Duration::from_millis(200)).await;

        let count = counter.load(Ordering::Relaxed);
        assert!(count >= 2, "expected at least 2 executions, got {}", count);

        // Cleanup
        scheduler.cancel(&TaskId::new("test-interval")).await.unwrap();
    }

    #[tokio::test]
    async fn test_schedule_duplicate_fails() {
        let scheduler = TokioScheduler::new();

        scheduler
            .schedule(TaskConfig::new(
                "dup-test",
                TaskTrigger::interval(Duration::from_secs(60)),
                || Box::pin(async {}),
            ))
            .await
            .unwrap();

        let result = scheduler
            .schedule(TaskConfig::new(
                "dup-test",
                TaskTrigger::interval(Duration::from_secs(60)),
                || Box::pin(async {}),
            ))
            .await;

        assert!(matches!(result, Err(ScheduleError::TaskAlreadyExists(_))));

        scheduler.cancel(&TaskId::new("dup-test")).await.unwrap();
    }

    #[tokio::test]
    async fn test_cancel_task() {
        let scheduler = TokioScheduler::new();

        scheduler
            .schedule(TaskConfig::new(
                "cancel-test",
                TaskTrigger::interval(Duration::from_secs(60)),
                || Box::pin(async {}),
            ))
            .await
            .unwrap();

        assert!(scheduler.is_scheduled(&TaskId::new("cancel-test")).await);

        scheduler.cancel(&TaskId::new("cancel-test")).await.unwrap();

        assert!(!scheduler.is_scheduled(&TaskId::new("cancel-test")).await);
    }

    #[tokio::test]
    async fn test_pause_resume() {
        let scheduler = TokioScheduler::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        scheduler
            .schedule(TaskConfig::new(
                "pause-test",
                TaskTrigger::interval(Duration::from_millis(50)),
                move || {
                    let c = Arc::clone(&counter_clone);
                    Box::pin(async move {
                        c.fetch_add(1, Ordering::Relaxed);
                    })
                },
            ))
            .await
            .unwrap();

        // Wait for initial execution
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_before = counter.load(Ordering::Relaxed);

        // Pause
        scheduler.pause(&TaskId::new("pause-test")).await.unwrap();

        // Wait while paused
        tokio::time::sleep(Duration::from_millis(200)).await;
        let count_during = counter.load(Ordering::Relaxed);

        // Should not have increased much
        assert!(
            count_during <= count_before + 1,
            "task should be paused, before={}, during={}",
            count_before,
            count_during
        );

        // Resume
        scheduler.resume(&TaskId::new("pause-test")).await.unwrap();

        // Wait for more executions
        tokio::time::sleep(Duration::from_millis(150)).await;
        let count_after = counter.load(Ordering::Relaxed);

        assert!(
            count_after > count_during,
            "task should resume, during={}, after={}",
            count_during,
            count_after
        );

        scheduler.cancel(&TaskId::new("pause-test")).await.unwrap();
    }

    #[tokio::test]
    async fn test_shutdown() {
        let scheduler = TokioScheduler::new();

        for i in 0..3 {
            scheduler
                .schedule(TaskConfig::new(
                    format!("task-{}", i),
                    TaskTrigger::interval(Duration::from_secs(60)),
                    || Box::pin(async {}),
                ))
                .await
                .unwrap();
        }

        assert_eq!(scheduler.list_tasks().await.len(), 3);

        scheduler.shutdown().await.unwrap();

        assert!(scheduler.list_tasks().await.is_empty());
    }

    #[tokio::test]
    async fn test_max_executions() {
        let scheduler = TokioScheduler::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        scheduler
            .schedule(
                TaskConfig::new(
                    "max-exec-test",
                    TaskTrigger::interval(Duration::from_millis(50)),
                    move || {
                        let c = Arc::clone(&counter_clone);
                        Box::pin(async move {
                            c.fetch_add(1, Ordering::Relaxed);
                        })
                    },
                )
                .with_max_executions(3),
            )
            .await
            .unwrap();

        // Wait for executions
        tokio::time::sleep(Duration::from_millis(500)).await;

        let count = counter.load(Ordering::Relaxed);
        assert_eq!(count, 3, "should execute exactly 3 times");

        // Task should be auto-removed
        assert!(!scheduler.is_scheduled(&TaskId::new("max-exec-test")).await);
    }

    #[tokio::test]
    async fn test_schedule_after() {
        use super::super::SchedulerExt;

        let scheduler = TokioScheduler::new();
        let executed = Arc::new(AtomicUsize::new(0));
        let executed_clone = Arc::clone(&executed);

        scheduler
            .schedule_after(
                "after-test",
                Duration::from_millis(100),
                move || {
                    let e = Arc::clone(&executed_clone);
                    Box::pin(async move {
                        e.fetch_add(1, Ordering::Relaxed);
                    })
                },
            )
            .await
            .unwrap();

        assert_eq!(executed.load(Ordering::Relaxed), 0);

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(executed.load(Ordering::Relaxed), 1);
    }
}
