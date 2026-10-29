//! Launch.log parser.

pub mod models;
pub mod parser;

pub use models::{LogGameInfo, LogInfo, LogSessionInfo};
pub use parser::{hosted_map, local_player_id, parse_launch_log};
