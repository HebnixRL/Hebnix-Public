//! Rocket League profile service client.

pub mod cache;
pub mod client;
pub mod models;

pub use cache::TtlCache;
pub use client::{TrackerClient, player_profile_json};
pub use models::{LifetimeStats, PlayerStats, PlaylistAverage, PlaylistRank};
