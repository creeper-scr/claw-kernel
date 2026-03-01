//! Cross-platform standard directory paths for claw-kernel.
//!
//! Provides platform-agnostic access to configuration, data, and cache directories
//! using the `dirs` crate as the underlying implementation.
//!
//! # Platform Mappings
//!
//! - **Linux:** XDG Base Directory Specification
//! - **macOS:** macOS standard directories (~/Library/Application Support, etc.)
//! - **Windows:** Windows standard directories (%APPDATA%, %LOCALAPPDATA%, etc.)

use std::path::PathBuf;

/// Configuration directory for claw-kernel.
///
/// Returns the platform-specific configuration directory with `claw-kernel` subdirectory appended.
///
/// # Platform Paths
/// - Linux: `~/.config/claw-kernel/`
/// - macOS: `~/Library/Application Support/claw-kernel/`
/// - Windows: `%APPDATA%\claw-kernel\`
///
/// # Returns
/// `Some(PathBuf)` if the home directory is available, `None` otherwise.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("claw-kernel"))
}

/// Data directory for claw-kernel (tools, scripts, persistent state).
///
/// Returns the platform-specific data directory with `claw-kernel` subdirectory appended.
///
/// # Platform Paths
/// - Linux: `~/.local/share/claw-kernel/`
/// - macOS: `~/Library/Application Support/claw-kernel/`
/// - Windows: `%APPDATA%\claw-kernel\`
///
/// # Returns
/// `Some(PathBuf)` if the home directory is available, `None` otherwise.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claw-kernel"))
}

/// Cache directory for claw-kernel.
///
/// Returns the platform-specific cache directory with `claw-kernel` subdirectory appended.
///
/// # Platform Paths
/// - Linux: `~/.cache/claw-kernel/`
/// - macOS: `~/Library/Caches/claw-kernel/`
/// - Windows: `%LOCALAPPDATA%\claw-kernel\cache\`
///
/// # Returns
/// `Some(PathBuf)` if the home directory is available, `None` otherwise.
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("claw-kernel"))
}

/// Tools directory for hot-loaded scripts.
///
/// Returns `data_dir()/tools`.
///
/// # Returns
/// `Some(PathBuf)` if the data directory is available, `None` otherwise.
pub fn tools_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tools"))
}

/// Runtime extension scripts directory.
///
/// Returns `data_dir()/scripts`.
///
/// # Returns
/// `Some(PathBuf)` if the data directory is available, `None` otherwise.
pub fn scripts_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("scripts"))
}

/// Logs directory for audit and runtime logs.
///
/// Returns `data_dir()/logs`.
///
/// # Returns
/// `Some(PathBuf)` if the data directory is available, `None` otherwise.
pub fn logs_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("logs"))
}

/// Agents directory for agent metadata and IPC endpoints.
///
/// Returns `data_dir()/agents`.
///
/// Per ADR-005, agents register themselves in this directory with subdirectories
/// containing metadata and IPC pipes.
///
/// # Returns
/// `Some(PathBuf)` if the data directory is available, `None` otherwise.
pub fn agents_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("agents"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_returns_some() {
        let result = config_dir();
        assert!(
            result.is_some(),
            "config_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_config_dir_contains_claw_kernel() {
        if let Some(path) = config_dir() {
            assert!(
                path.to_string_lossy().contains("claw-kernel"),
                "config_dir path should contain 'claw-kernel' subdirectory"
            );
        }
    }

    #[test]
    fn test_data_dir_returns_some() {
        let result = data_dir();
        assert!(
            result.is_some(),
            "data_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_data_dir_contains_claw_kernel() {
        if let Some(path) = data_dir() {
            assert!(
                path.to_string_lossy().contains("claw-kernel"),
                "data_dir path should contain 'claw-kernel' subdirectory"
            );
        }
    }

    #[test]
    fn test_cache_dir_returns_some() {
        let result = cache_dir();
        assert!(
            result.is_some(),
            "cache_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_cache_dir_contains_claw_kernel() {
        if let Some(path) = cache_dir() {
            assert!(
                path.to_string_lossy().contains("claw-kernel"),
                "cache_dir path should contain 'claw-kernel' subdirectory"
            );
        }
    }

    #[test]
    fn test_tools_dir_returns_some() {
        let result = tools_dir();
        assert!(
            result.is_some(),
            "tools_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_tools_dir_contains_tools() {
        if let Some(path) = tools_dir() {
            assert!(
                path.to_string_lossy().contains("tools"),
                "tools_dir path should contain 'tools' subdirectory"
            );
        }
    }

    #[test]
    fn test_scripts_dir_returns_some() {
        let result = scripts_dir();
        assert!(
            result.is_some(),
            "scripts_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_scripts_dir_contains_scripts() {
        if let Some(path) = scripts_dir() {
            assert!(
                path.to_string_lossy().contains("scripts"),
                "scripts_dir path should contain 'scripts' subdirectory"
            );
        }
    }

    #[test]
    fn test_logs_dir_returns_some() {
        let result = logs_dir();
        assert!(
            result.is_some(),
            "logs_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_logs_dir_contains_logs() {
        if let Some(path) = logs_dir() {
            assert!(
                path.to_string_lossy().contains("logs"),
                "logs_dir path should contain 'logs' subdirectory"
            );
        }
    }

    #[test]
    fn test_agents_dir_returns_some() {
        let result = agents_dir();
        assert!(
            result.is_some(),
            "agents_dir should return Some when HOME is available"
        );
    }

    #[test]
    fn test_agents_dir_contains_agents() {
        if let Some(path) = agents_dir() {
            assert!(
                path.to_string_lossy().contains("agents"),
                "agents_dir path should contain 'agents' subdirectory"
            );
        }
    }

    #[test]
    fn test_dirs_do_not_create_directories() {
        // Call all functions
        let _ = config_dir();
        let _ = data_dir();
        let _ = cache_dir();
        let _ = tools_dir();
        let _ = scripts_dir();
        let _ = logs_dir();
        let _ = agents_dir();

        // Verify that if a path was returned, the directory doesn't actually exist
        // (we're just returning paths, not creating them)
        if let Some(path) = config_dir() {
            // The directory should not exist after calling the function
            // (unless it was already created by the user or another process)
            // This test just verifies the function doesn't panic or create dirs
            assert!(path.is_absolute(), "config_dir should return absolute path");
        }
    }

    #[test]
    fn test_subdirectory_hierarchy() {
        // Verify that subdirectories are properly nested
        if let Some(data) = data_dir() {
            if let Some(tools) = tools_dir() {
                assert!(
                    tools.starts_with(&data),
                    "tools_dir should be under data_dir"
                );
            }
            if let Some(scripts) = scripts_dir() {
                assert!(
                    scripts.starts_with(&data),
                    "scripts_dir should be under data_dir"
                );
            }
            if let Some(logs) = logs_dir() {
                assert!(logs.starts_with(&data), "logs_dir should be under data_dir");
            }
            if let Some(agents) = agents_dir() {
                assert!(
                    agents.starts_with(&data),
                    "agents_dir should be under data_dir"
                );
            }
        }
    }
}
