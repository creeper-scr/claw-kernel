//! Schedule — Time-triggered task scheduling for claw-runtime.
//!
//! Provides cron-like and interval-based scheduling capabilities.
//! This is a Layer 1 (System Runtime) primitive, on par with EventBus
//! and AgentOrchestrator.
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_runtime::schedule::{Scheduler, TaskConfig, TaskTrigger};
//! use std::time::Duration;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let scheduler = Scheduler::new();
//!
//! // Schedule an interval-based heartbeat
//! scheduler.schedule(TaskConfig::new(
//!     "heartbeat",
//!     TaskTrigger::Interval(Duration::from_secs(30)),
//!     || async {
//!         println!("Heartbeat!");
//!     }
//! )).await?;
//!
//! // Schedule a cron-like job
//! scheduler.schedule(TaskConfig::new(
//!     "daily-report",
//!     TaskTrigger::Cron("0 9 * * *".to_string()), // 9:00 AM daily
//!     || async {
//!         println!("Generating daily report...");
//!     }
//! )).await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod tokio_scheduler;

pub use error::ScheduleError;
pub use tokio_scheduler::TokioScheduler;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Unique identifier for a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TaskId(pub String);

impl TaskId {
    /// Create a new TaskId from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The trigger condition for a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TaskTrigger {
    /// Trigger at fixed time intervals.
    Interval(Duration),

    /// Trigger based on a cron expression.
    /// Format: "sec min hour day month dow" (optional seconds)
    /// Examples:
    /// - "0 * * * *" - At the start of every hour
    /// - "0 9 * * 1-5" - At 9:00 AM on weekdays
    /// - "*/30 * * * * *" - Every 30 seconds
    Cron(String),

    /// Trigger once at a specific timestamp (Unix seconds).
    Once(u64),

    /// Trigger immediately and then never again.
    Immediate,
}

impl TaskTrigger {
    /// Create an interval trigger.
    pub fn interval(duration: Duration) -> Self {
        Self::Interval(duration)
    }

    /// Create a cron trigger.
    pub fn cron(expr: impl Into<String>) -> Self {
        Self::Cron(expr.into())
    }

    /// Create a one-time trigger at a Unix timestamp.
    pub fn once(unix_secs: u64) -> Self {
        Self::Once(unix_secs)
    }

    /// Create an immediate trigger.
    pub fn immediate() -> Self {
        Self::Immediate
    }

    /// Check if this trigger should fire immediately upon registration.
    pub fn is_immediate(&self) -> bool {
        matches!(self, TaskTrigger::Immediate)
    }
}

/// Handler type for scheduled tasks.
pub type TaskHandler = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Configuration for a scheduled task.
#[derive(Clone)]
pub struct TaskConfig {
    /// Unique identifier for the task.
    pub id: TaskId,
    /// The trigger condition.
    pub trigger: TaskTrigger,
    /// The handler to execute.
    pub handler: TaskHandler,
    /// Maximum number of executions (None = unlimited).
    pub max_executions: Option<u64>,
    /// Whether to catch up missed executions on startup.
    pub catch_up: bool,
}

impl std::fmt::Debug for TaskConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskConfig")
            .field("id", &self.id)
            .field("trigger", &self.trigger)
            .field("max_executions", &self.max_executions)
            .field("catch_up", &self.catch_up)
            .finish_non_exhaustive()
    }
}

impl TaskConfig {
    /// Create a new task configuration.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique task identifier
    /// * `trigger` - The trigger condition
    /// * `handler` - Async function to execute
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claw_runtime::schedule::{TaskConfig, TaskTrigger};
    /// use std::time::Duration;
    ///
    /// let config = TaskConfig::new(
    ///     "heartbeat",
    ///     TaskTrigger::Interval(Duration::from_secs(30)),
    ///     || Box::pin(async {
    ///         println!("Heartbeat!");
    ///     })
    /// );
    /// ```
    pub fn new<F, Fut>(id: impl Into<String>, trigger: TaskTrigger, handler: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            id: TaskId::new(id),
            trigger,
            handler: Arc::new(move || Box::pin(handler())),
            max_executions: None,
            catch_up: false,
        }
    }

    /// Set the maximum number of executions.
    pub fn with_max_executions(mut self, count: u64) -> Self {
        self.max_executions = Some(count);
        self
    }

    /// Enable catch-up mode (execute missed tasks on startup).
    pub fn with_catch_up(mut self) -> Self {
        self.catch_up = true;
        self
    }
}

/// Core trait for task schedulers.
///
/// Implementations must be thread-safe and support concurrent scheduling.
#[async_trait::async_trait]
pub trait Scheduler: Send + Sync {
    /// Schedule a new task.
    ///
    /// Returns an error if a task with the same ID already exists.
    async fn schedule(&self, config: TaskConfig) -> Result<(), ScheduleError>;

    /// Cancel a scheduled task.
    ///
    /// The task will not execute again. Returns an error if the task doesn't exist.
    async fn cancel(&self, task_id: &TaskId) -> Result<(), ScheduleError>;

    /// Check if a task is scheduled.
    async fn is_scheduled(&self, task_id: &TaskId) -> bool;

    /// Get list of all scheduled task IDs.
    async fn list_tasks(&self) -> Vec<TaskId>;

    /// Pause a scheduled task (preserves state but stops execution).
    async fn pause(&self, task_id: &TaskId) -> Result<(), ScheduleError>;

    /// Resume a paused task.
    async fn resume(&self, task_id: &TaskId) -> Result<(), ScheduleError>;

    /// Get the next execution time for a task (Unix seconds).
    /// Returns None if the task doesn't exist or won't execute again.
    async fn next_execution(&self, task_id: &TaskId) -> Option<u64>;

    /// Shutdown the scheduler gracefully.
    ///
    /// Waits for currently executing tasks to complete, then stops all scheduling.
    async fn shutdown(&self) -> Result<(), ScheduleError>;
}

/// Statistics for a scheduled task.
#[derive(Debug, Clone, Default)]
pub struct TaskStats {
    /// Total number of executions.
    pub execution_count: u64,
    /// Number of successful executions.
    pub success_count: u64,
    /// Number of failed executions.
    pub failure_count: u64,
    /// Last execution time (Unix milliseconds).
    pub last_execution: Option<u64>,
    /// Next scheduled execution time (Unix milliseconds).
    pub next_execution: Option<u64>,
    /// Whether the task is currently paused.
    pub is_paused: bool,
}

/// Extension trait for scheduler management.
#[async_trait::async_trait]
pub trait SchedulerExt: Scheduler {
    /// Schedule a one-shot task that executes after a delay.
    async fn schedule_after<F, Fut>(
        &self,
        id: impl Into<String> + Send,
        delay: Duration,
        handler: F,
    ) -> Result<(), ScheduleError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let trigger = TaskTrigger::Interval(delay);
        let config = TaskConfig::new(id, trigger, handler).with_max_executions(1);
        self.schedule(config).await
    }

    /// Schedule a heartbeat task with a given interval.
    async fn schedule_heartbeat<F, Fut>(
        &self,
        id: impl Into<String> + Send,
        interval: Duration,
        handler: F,
    ) -> Result<(), ScheduleError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let trigger = TaskTrigger::Interval(interval);
        let config = TaskConfig::new(id, trigger, handler);
        self.schedule(config).await
    }
}

#[async_trait::async_trait]
impl<T: Scheduler> SchedulerExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_id() {
        let id = TaskId::new("test-task");
        assert_eq!(id.0, "test-task");
        assert_eq!(id.to_string(), "test-task");
    }

    #[test]
    fn test_task_trigger_interval() {
        let dur = Duration::from_secs(60);
        let trigger = TaskTrigger::interval(dur);
        assert_eq!(trigger, TaskTrigger::Interval(dur));
        assert!(!trigger.is_immediate());
    }

    #[test]
    fn test_task_trigger_cron() {
        let trigger = TaskTrigger::cron("0 * * * *");
        assert_eq!(trigger, TaskTrigger::Cron("0 * * * *".to_string()));
    }

    #[test]
    fn test_task_trigger_immediate() {
        let trigger = TaskTrigger::immediate();
        assert!(trigger.is_immediate());
    }

    #[test]
    fn test_task_config_builder() {
        let config = TaskConfig::new(
            "test",
            TaskTrigger::interval(Duration::from_secs(30)),
            || Box::pin(async {}),
        )
        .with_max_executions(10)
        .with_catch_up();

        assert_eq!(config.id.0, "test");
        assert_eq!(config.max_executions, Some(10));
        assert!(config.catch_up);
    }

    #[test]
    fn test_task_stats_default() {
        let stats = TaskStats::default();
        assert_eq!(stats.execution_count, 0);
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.failure_count, 0);
        assert!(stats.last_execution.is_none());
        assert!(stats.next_execution.is_none());
        assert!(!stats.is_paused);
    }
}
