//! Channel integrations — inbound/outbound message adapters.
//!
//! Channels connect the agent system to external messaging platforms
//! (Discord, webhooks, etc.).  Each channel adapter implements the
//! `Channel` trait.

pub mod error;
pub mod types;
pub mod traits;

pub use error::ChannelError;
pub use types::{ChannelId, ChannelMessage, MessageDirection, Platform};
pub use traits::Channel;
