//! Channel integrations — inbound/outbound message adapters.
//!
//! Channels connect the agent system to external messaging platforms
//! (Discord, webhooks, etc.). Each channel adapter implements the
//! `Channel` trait.
//!
//! # Main Types
//!
//! - [`Channel`] - Trait for channel implementations
//! - [`ChannelMessage`] - Message type for channel communication
//! - [`StdinChannel`] - Read from stdin
//! - [`ChannelId`] - Channel identifier
//! - [`Platform`] - Enum of supported platforms
//!
//! # Optional Features
//!
//! - `webhook` - HTTP webhook channel support
//! - `discord` - Discord bot integration
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_channel::{Channel, ChannelId, StdinChannel};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a stdin channel — requires a ChannelId.
//! let channel = StdinChannel::new(ChannelId::new("cli"));
//!
//! // In a real application, connect first then receive messages:
//! // channel.connect().await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod retry;
pub mod router;
pub mod stdin;
pub mod traits;
pub mod types;

#[cfg(feature = "webhook")]
pub mod webhook;

#[cfg(feature = "discord")]
pub mod discord;

pub use error::ChannelError;
pub use retry::RetryableChannel;
pub use router::{AgentId as RouterAgentId, ChannelRouter, ChannelRouterBuilder, DeduplicatingRouter, RouterError, RoutingRule};
pub use stdin::StdinChannel;
pub use traits::{Channel, ChannelEvent, ChannelEventPublisher, NoopChannelEventPublisher};
pub use types::{ChannelId, ChannelMessage, MessageDirection, Platform};

#[cfg(feature = "webhook")]
pub use webhook::WebhookChannel;

#[cfg(feature = "discord")]
pub use discord::DiscordChannel;
