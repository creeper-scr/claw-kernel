//! Official channel implementations for claw-kernel.
//!
//! This crate provides ready-to-use channel adapters that implement the
//! [`claw_channel::Channel`] trait.
//!
//! # Features
//!
//! - `discord` — Discord bot channel (migrated from `claw-channel`, uses twilight)
//! - `websocket` — Bidirectional WebSocket channel (multi-client fan-out)
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_channels::WebSocketChannel;
//! use claw_channel::Channel;
//!
//! let ws = WebSocketChannel::new(claw_channel::ChannelId::new("ws-main"));
//! ws.connect().await?;
//! ```

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "websocket")]
pub mod websocket;

#[cfg(feature = "discord")]
pub use discord::DiscordChannel;

#[cfg(feature = "websocket")]
pub use websocket::WebSocketChannel;
