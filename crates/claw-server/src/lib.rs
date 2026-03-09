//! claw-server — KernelServer for remote agent control via JSON-RPC 2.0 over IPC.
//!
//! This crate exposes the claw-kernel agent loop functionality via a local IPC
//! interface using JSON-RPC 2.0 protocol. It allows external applications to:
//!
//! - Create and manage agent sessions
//! - Send messages to agents
//! - Receive streaming responses
//! - Execute tool calls
//! - Handle tool results
//!
//! ## Architecture
//!
//! The server consists of several components:
//!
//! - **`KernelServer`**: Main server that listens on a Unix socket
//! - **`SessionManager`**: Manages active agent sessions
//! - **`Session`**: Represents a single agent session with notification channels
//! - **`handler`**: Processes JSON-RPC requests and dispatches to appropriate handlers
//! - **`protocol`**: JSON-RPC 2.0 message types and error codes
//!
//! # Main Types
//!
//! - [`KernelServer`] - Main server that handles JSON-RPC requests
//! - [`ServerConfig`] - Configuration for the server
//! - [`ProviderConfig`] - LLM provider configuration
//! - [`SessionManager`] - Manages active sessions
//!
//! # Example
//!
//! ```no_run
//! use claw_server::{KernelServer, ServerConfig, ProviderConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ServerConfig {
//!         socket_path: "/tmp/claw-kernel.sock".to_string(),
//!         max_sessions: 100,
//!         provider_config: ProviderConfig::Anthropic {
//!             api_key: "your-api-key".to_string(),
//!             default_model: "claude-3-opus".to_string(),
//!         },
//!     };
//!
//!     let server = KernelServer::new(config);
//!     server.run().await?;
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]

pub mod error;
pub mod handler;
pub mod protocol;
pub mod server;
pub mod session;

pub use error::ServerError;
pub use server::{KernelServer, ProviderConfig, ServerConfig};
pub use session::SessionManager;

// Re-export handler notification functions for convenience
pub use handler::{notify_chunk, notify_finish, notify_tool_call};
