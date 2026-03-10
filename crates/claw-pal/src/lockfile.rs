//! Daemon lock file mechanism for preventing duplicate daemon instances.
//!
//! Uses advisory file locking to ensure only one daemon instance runs at a time.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Error type for lock file operations.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another instance is already running.
    #[error("daemon already running (PID {pid})")]
    AlreadyRunning { pid: u32 },
    /// I/O error accessing the lock file.
    #[error("lock file I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// An advisory exclusive lock on a PID file.
///
/// The lock is released and the file is deleted when this guard is dropped.
pub struct DaemonLock {
    /// The locked file (kept open to maintain the flock).
    _file: File,
    /// Path to the lock file (for cleanup on drop).
    path: std::path::PathBuf,
}

impl DaemonLock {
    /// Try to acquire an exclusive advisory lock on `lock_path`.
    ///
    /// On success, writes `pid` to the file and returns the lock guard.
    /// On failure (lock held by another process), returns `Err(LockError::AlreadyRunning)`.
    pub fn acquire(lock_path: &Path, pid: u32) -> Result<Self, LockError> {
        // Ensure parent directory exists
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(lock_path)?;

        #[cfg(unix)]
        {
            let ret = libc_flock(file.as_raw_fd(), LOCK_EX | LOCK_NB);
            if ret != 0 {
                // Lock is held — try to read PID from the file
                let existing_pid = Self::read_pid(lock_path).unwrap_or(0);
                return Err(LockError::AlreadyRunning { pid: existing_pid });
            }
        }

        #[cfg(windows)]
        {
            // Windows: no-op advisory lock (daemon deduplication via PID check)
            // Just write PID and proceed
        }

        // Write PID to file
        let mut file = file;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        write!(file, "{}", pid)?;
        file.flush()?;

        Ok(Self {
            _file: file,
            path: lock_path.to_path_buf(),
        })
    }

    /// Read the PID stored in an existing lock file, without acquiring the lock.
    ///
    /// Returns `None` if the file doesn't exist or doesn't contain a valid PID.
    pub fn read_pid(lock_path: &Path) -> Option<u32> {
        let mut file = File::open(lock_path).ok()?;
        let mut contents = String::new();
        file.read_to_string(&mut contents).ok()?;
        contents.trim().parse::<u32>().ok()
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        // Remove the lock file on drop (flock is automatically released when file is closed)
        let _ = std::fs::remove_file(&self.path);
    }
}

// ── Unix flock syscall wrapper ────────────────────────────────────────────────

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(unix)]
const LOCK_EX: i32 = 2;

#[cfg(unix)]
const LOCK_NB: i32 = 4;

#[cfg(unix)]
extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

#[cfg(unix)]
fn libc_flock(fd: i32, operation: i32) -> i32 {
    unsafe { flock(fd, operation) }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_release() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.pid");

        {
            let lock = DaemonLock::acquire(&lock_path, 12345).expect("should acquire lock");
            // PID file should exist and contain our PID
            let pid = DaemonLock::read_pid(&lock_path).expect("should read pid");
            assert_eq!(pid, 12345);
            // lock is dropped here
            drop(lock);
        }

        // After drop, file should be removed
        assert!(!lock_path.exists(), "lock file should be deleted on drop");
    }

    #[test]
    fn test_double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("double.pid");

        let _lock1 = DaemonLock::acquire(&lock_path, 111).expect("first acquire should succeed");

        let result = DaemonLock::acquire(&lock_path, 222);
        assert!(
            matches!(result, Err(LockError::AlreadyRunning { .. })),
            "second acquire should fail with AlreadyRunning"
        );
    }

    #[test]
    fn test_read_pid_nonexistent() {
        let pid = DaemonLock::read_pid(Path::new("/tmp/nonexistent_claw_test_pid_xyz.pid"));
        assert!(pid.is_none());
    }
}
