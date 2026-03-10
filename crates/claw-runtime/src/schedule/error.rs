//! Error types for the schedule module.

use thiserror::Error;

/// Errors that can occur in the scheduler.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// Task ID already exists.
    #[error("task already exists: {0}")]
    TaskAlreadyExists(String),

    /// Task not found.
    #[error("task not found: {0}")]
    TaskNotFound(String),

    /// Invalid cron expression.
    #[error("invalid cron expression '{expr}': {reason}")]
    InvalidCronExpression {
        /// The offending expression.
        expr: String,
        /// Human-readable reason from the parser.
        reason: String,
    },

    /// Cron expression is valid but will never fire (e.g. Feb 30).
    #[error("cron expression will never fire: {0}")]
    NeverFires(String),

    /// Invalid interval.
    #[error("invalid interval: {0}")]
    InvalidInterval(String),

    /// Scheduler is shutting down.
    #[error("scheduler is shutting down")]
    ShuttingDown,

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_error_display() {
        let err = ScheduleError::TaskAlreadyExists("task-1".to_string());
        assert_eq!(err.to_string(), "task already exists: task-1");

        let err = ScheduleError::TaskNotFound("task-2".to_string());
        assert_eq!(err.to_string(), "task not found: task-2");

        let err = ScheduleError::InvalidCronExpression {
            expr: "* * * *".to_string(),
            reason: "too few fields".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "invalid cron expression '* * * *': too few fields"
        );

        let err = ScheduleError::InvalidInterval("zero".to_string());
        assert_eq!(err.to_string(), "invalid interval: zero");

        let err = ScheduleError::ShuttingDown;
        assert_eq!(err.to_string(), "scheduler is shutting down");

        let err = ScheduleError::Internal("db error".to_string());
        assert_eq!(err.to_string(), "internal error: db error");
    }

    #[test]
    fn test_schedule_error_clone() {
        let err = ScheduleError::TaskNotFound("test".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
