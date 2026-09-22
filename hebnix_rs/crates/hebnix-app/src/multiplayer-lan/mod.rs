mod beacon;
mod direct_udp;
mod firewall;
mod guest;
mod hosting;
mod models;
mod room_api;
mod tsnet_sidecar;

use std::time::Duration;

pub use direct_udp::TunnelStats;
pub use firewall::{ensure_beacon_relay_rule, ensure_rocket_league_lan_rule, ensure_sidecar_rule};
pub use guest::GuestSession;
pub use hosting::HostSession;
pub use models::{
    CreateRoomRequest, JoinRoomRequest, JoinedRoom, LeaveRoomRequest, MapDescriptor, Room,
    RoomCredentials, TsnetAuthKey, UpdatePlayerRequest,
};
pub use room_api::RoomClient;
pub use tsnet_sidecar::{PeerInfo, TsState, TsnetSidecarHandle};

/// Where the headscale coordination server for Workshop LAN lives. Harry
/// said he'll likely just run headscale on the existing api.hebnix.com box
/// rather than standing up a separate subdomain, so this points there by
/// default -- update this one constant if he ends up hosting it elsewhere.
pub const TSNET_CONTROL_URL: &str = "https://api.hebnix.com";
pub const ROOM_API_BASE_URL: &str = "https://api.hebnix.com";

/// Rocket League's own LAN discovery/game port. The beacon relay listens
/// here and guests' `-multihome` sockets receive on it too.
pub const RL_LAN_PORT: u16 = 7777;

pub const PACKET_PUMP_INTERVAL: Duration = Duration::from_millis(50);
pub const SESSION_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(300);
/// how long a session stays alive after Rocket League exits before Hebnix
/// tears the room/tailnet down for real -- covers an ordinary crash/restart
/// without kicking everyone out of the room.
pub const CRASH_GRACE_WINDOW: Duration = Duration::from_secs(90);

pub fn cleanup_system_state() -> Result<(), String> {
    firewall::remove_rules()
}
