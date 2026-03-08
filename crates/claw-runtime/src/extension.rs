//! Extension-related events for tool hot-loading, script reloading, and
//! provider registration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

// ─── Error Types ──────────────────────────────────────────────────────────────

/// Error that occurred while loading a tool.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum LoadError {
    #[error("IO error loading tool '{name}': {message}")]
    Io { name: String, message: String },

    #[error("Invalid schema for tool '{name}': {message}")]
    InvalidSchema { name: String, message: String },

    #[error("Permission validation failed for tool '{name}': {message}")]
    PermissionDenied { name: String, message: String },

    #[error("Tool '{name}' already registered")]
    AlreadyExists { name: String },
}

/// Error that occurred while reloading a script.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ReloadError {
    #[error("IO error reloading script at '{path}': {message}")]
    Io {
        path: PathBuf,
        message: String,
    },

    #[error("Compilation error in script at '{path}': {message}")]
    Compile {
        path: PathBuf,
        message: String,
    },

    #[error("Runtime error reloading script at '{path}': {message}")]
    Runtime {
        path: PathBuf,
        message: String,
    },
}

// ─── ExtensionEvent ───────────────────────────────────────────────────────────

/// Events emitted by the extension subsystem (hot-loading, script reloads,
/// provider registration).
///
/// Published on the `EventBus` as `Event::Extension(ExtensionEvent)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExtensionEvent {
    /// A tool is being loaded (before completion).
    ToolLoading { name: String },

    /// A tool has finished loading (success or failure).
    ToolLoaded {
        name: String,
        result: Result<(), LoadError>,
    },

    /// A tool has been unloaded from the registry.
    ToolUnloaded { name: String },

    /// A script file has been reloaded (e.g., after a file-system change).
    ScriptReloaded {
        path: PathBuf,
        result: Result<(), ReloadError>,
    },

    /// A new LLM provider has been registered at runtime.
    ProviderRegistered { name: String },
}
