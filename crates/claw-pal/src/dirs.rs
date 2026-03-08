//! Directory management for claw-kernel.

use std::path::PathBuf;

/// Get the configuration directory.
///
/// Returns the platform-specific config directory with "claw-kernel" appended.
///
/// # Platform Differences
///
/// - **Linux**: `~/.config/claw-kernel`
/// - **macOS**: `~/Library/Application Support/claw-kernel`
/// - **Windows**: `%APPDATA%/claw-kernel` (e.g., `C:\Users\<user>\AppData\Roaming\claw-kernel`)
///
/// # Example
///
/// ```
/// use claw_pal::dirs::config_dir;
///
/// if let Some(path) = config_dir() {
///     println!("Config directory: {}", path.display());
/// }
/// ```
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("claw-kernel"))
}

/// Get the data directory.
///
/// Returns the platform-specific data directory with "claw-kernel" appended.
/// This is where user data, tools, logs, and other persistent files are stored.
///
/// # Platform Differences
///
/// - **Linux**: `~/.local/share/claw-kernel`
/// - **macOS**: `~/Library/Application Support/claw-kernel`
/// - **Windows**: `%APPDATA%/claw-kernel`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::data_dir;
///
/// if let Some(path) = data_dir() {
///     println!("Data directory: {}", path.display());
/// }
/// ```
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claw-kernel"))
}

/// Get the cache directory.
///
/// Returns the platform-specific cache directory with "claw-kernel" appended.
/// Cache files can be deleted at any time and will be recreated as needed.
///
/// # Platform Differences
///
/// - **Linux**: `~/.cache/claw-kernel`
/// - **macOS**: `~/Library/Caches/claw-kernel`
/// - **Windows**: `%LOCALAPPDATA%/claw-kernel/Cache`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::cache_dir;
///
/// if let Some(path) = cache_dir() {
///     println!("Cache directory: {}", path.display());
/// }
/// ```
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("claw-kernel"))
}

/// Get the tools directory.
///
/// Returns the directory where tool scripts and binaries are stored.
/// This is a subdirectory of the data directory.
///
/// # Platform Differences
///
/// - **Linux**: `~/.local/share/claw-kernel/tools`
/// - **macOS**: `~/Library/Application Support/claw-kernel/tools`
/// - **Windows**: `%APPDATA%/claw-kernel/tools`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::tools_dir;
///
/// if let Some(path) = tools_dir() {
///     println!("Tools directory: {}", path.display());
/// }
/// ```
pub fn tools_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tools"))
}

/// Get the scripts directory.
///
/// Returns the directory where user scripts are stored.
/// This is a subdirectory of the data directory.
///
/// # Platform Differences
///
/// - **Linux**: `~/.local/share/claw-kernel/scripts`
/// - **macOS**: `~/Library/Application Support/claw-kernel/scripts`
/// - **Windows**: `%APPDATA%/claw-kernel/scripts`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::scripts_dir;
///
/// if let Some(path) = scripts_dir() {
///     println!("Scripts directory: {}", path.display());
/// }
/// ```
pub fn scripts_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("scripts"))
}

/// Get the logs directory.
///
/// Returns the directory where log files are stored.
/// This is a subdirectory of the data directory.
///
/// # Platform Differences
///
/// - **Linux**: `~/.local/share/claw-kernel/logs`
/// - **macOS**: `~/Library/Application Support/claw-kernel/logs`
/// - **Windows**: `%APPDATA%/claw-kernel/logs`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::logs_dir;
///
/// if let Some(path) = logs_dir() {
///     println!("Logs directory: {}", path.display());
/// }
/// ```
pub fn logs_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("logs"))
}

/// Get the agents directory.
///
/// Returns the directory where agent-specific data and state is stored.
/// This is a subdirectory of the data directory.
///
/// # Platform Differences
///
/// - **Linux**: `~/.local/share/claw-kernel/agents`
/// - **macOS**: `~/Library/Application Support/claw-kernel/agents`
/// - **Windows**: `%APPDATA%/claw-kernel/agents`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::agents_dir;
///
/// if let Some(path) = agents_dir() {
///     println!("Agents directory: {}", path.display());
/// }
/// ```
pub fn agents_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("agents"))
}

/// Get the power key file path.
///
/// Returns the path to the power key file used for Power Mode authentication.
/// The power key grants elevated privileges in Power Mode.
///
/// # Platform Differences
///
/// - **Linux**: `~/.config/claw-kernel/power.key`
/// - **macOS**: `~/Library/Application Support/claw-kernel/power.key`
/// - **Windows**: `%APPDATA%/claw-kernel/power.key`
///
/// # Example
///
/// ```
/// use claw_pal::dirs::power_key_path;
///
/// if let Some(path) = power_key_path() {
///     println!("Power key file: {}", path.display());
/// }
/// ```
pub fn power_key_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("power.key"))
}

/// Kernel directory paths.
#[derive(Debug, Clone)]
pub struct KernelDirs {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub tools_dir: PathBuf,
}

impl KernelDirs {
    /// Create a new KernelDirs instance.
    pub fn new() -> Result<Self, std::io::Error> {
        let config_dir = Self::config_dir()?;
        let data_dir = Self::data_dir()?;

        Ok(Self {
            log_dir: data_dir.join("logs"),
            agents_dir: data_dir.join("agents"),
            tools_dir: data_dir.join("tools"),
            config_dir,
            data_dir,
        })
    }

    /// Get the configuration directory.
    pub fn config_dir() -> Result<PathBuf, std::io::Error> {
        dirs::config_dir()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find config directory",
                )
            })
            .map(|d| d.join("claw-kernel"))
    }

    /// Get the data directory.
    pub fn data_dir() -> Result<PathBuf, std::io::Error> {
        dirs::data_dir()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find data directory",
                )
            })
            .map(|d| d.join("claw-kernel"))
    }

    /// Ensure all directories exist, creating them if necessary.
    pub async fn ensure_all(&self) -> Result<(), std::io::Error> {
        tokio::fs::create_dir_all(&self.config_dir).await?;
        tokio::fs::create_dir_all(&self.data_dir).await?;
        tokio::fs::create_dir_all(&self.log_dir).await?;
        tokio::fs::create_dir_all(&self.agents_dir).await?;
        tokio::fs::create_dir_all(&self.tools_dir).await?;
        Ok(())
    }

    /// Create directories synchronously.
    pub fn ensure_all_sync(&self) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        std::fs::create_dir_all(&self.agents_dir)?;
        std::fs::create_dir_all(&self.tools_dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_kernel_dirs_structure() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dirs = KernelDirs {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("data/logs"),
            agents_dir: root.join("data/agents"),
            tools_dir: root.join("data/tools"),
        };

        assert_eq!(dirs.log_dir, root.join("data/logs"));
        assert_eq!(dirs.agents_dir, root.join("data/agents"));
        assert_eq!(dirs.tools_dir, root.join("data/tools"));
    }

    #[tokio::test]
    async fn test_ensure_all() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dirs = KernelDirs {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("data/logs"),
            agents_dir: root.join("data/agents"),
            tools_dir: root.join("data/tools"),
        };

        dirs.ensure_all().await.unwrap();

        assert!(dirs.config_dir.exists());
        assert!(dirs.log_dir.exists());
        assert!(dirs.agents_dir.exists());
        assert!(dirs.tools_dir.exists());
    }
}
