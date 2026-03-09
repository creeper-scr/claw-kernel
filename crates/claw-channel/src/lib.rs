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
//! use claw_channel::{Channel, StdinChannel};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a stdin channel
//! let mut channel = StdinChannel::new();
//!
//! // In a real application, you would run the channel
//! // channel.run().await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod stdin;
pub mod traits;
pub mod types;

#[cfg(feature = "webhook")]
pub mod webhook;

#[cfg(feature = "discord")]
pub mod discord;

pub use error::ChannelError;
pub use stdin::StdinChannel;
pub use traits::Channel;
pub use types::{ChannelId, ChannelMessage, MessageDirection, Platform};

#[cfg(feature = "webhook")]
pub use webhook::WebhookChannel;

#[cfg(feature = "discord")]
pub use discord::DiscordChannel;
