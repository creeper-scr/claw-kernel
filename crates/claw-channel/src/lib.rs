//! Channel integrations — inbound/outbound message adapters.
//!
//! Channels connect the agent system to external messaging platforms
//! (Discord, webhooks, etc.).  Each channel adapter implements the
//! `Channel` trait.

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
