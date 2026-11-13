//! Workshop LAN multiplayer over tailcat.
//!
//! Every player's `hebnix-tc` sidecar (tailcat_sidecar.rs) announces itself
//! to the Hebnix directory and connects to every other player there, peer
//! to peer or through relay servers, so everyone sees everyone's LAN
//! matches. The sidecars proxy Rocket League's LAN discovery and game
//! traffic over loopback, so RL runs without `-multihome` (online play
//! keeps working) and nothing needs root, a TUN device or firewall rules.
//! Map sync (map_sync.rs) runs over the same connections.

mod caps;
mod firewall;
mod hosting;
mod map_sync;
mod models;
mod room_api;
mod tailcat_sidecar;

use std::time::Duration;

// one session type for hosting and joining, see hosting.rs
pub use hosting::HostSession;
pub use map_sync::{
    LocalInfo, MAP_SYNC_PORT, MAX_MAP_BYTES, MapFileProvider, MapProvider, PeerOffer, SlotMap,
    TransferProgress, fetch_map_file, hash_file, is_local_map_id, local_map_id, valid_map_id, valid_package_stem,
};
pub use models::{
    CreateRoomRequest, JoinRoomRequest, JoinedRoom, LeaveRoomRequest, Room,
    RoomCredentials, UpdatePlayerRequest,
};
pub use tailcat_sidecar::{PeerLink, TailcatSidecarHandle, redact};
pub use caps::raise_net_admin_ambient;
pub use firewall::ensure_sidecar_rule;

mod directory_auth {
    include!(concat!(env!("OUT_DIR"), "/directory_key.rs"));
}

/// where every player's sidecar announces itself and finds the others
/// (sidecar/directory)
pub const DIRECTORY_URL: &str = "https://mp.hebnix.com/directory";

/// DIRECTORY_URL, unless `HEBNIX_DIRECTORY_URL` points somewhere else (a
/// test or self-hosted directory)
pub fn directory_url() -> String {
    std::env::var("HEBNIX_DIRECTORY_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| DIRECTORY_URL.to_string())
}

/// the directory's shared key, baked in at build time
/// (`directory_key.txt` at the repository root, or the `HEBNIX_DIRECTORY_KEY`
/// env var in CI)
pub fn directory_key() -> Result<&'static str, String> {
    let key = directory_auth::KEY.trim();
    if key.is_empty() {
        Err("The multiplayer directory key is missing; add directory_key.txt at the repository root and rebuild Hebnix.".into())
    } else {
        Ok(key)
    }
}

/// how long a session stays alive after Rocket League exits before Hebnix
/// tears the room/tailnet down for real -- covers an ordinary crash/restart
/// without kicking everyone out of the room.
pub const CRASH_GRACE_WINDOW: Duration = Duration::from_secs(90);

pub fn cleanup_system_state() -> Result<(), String> {
    // only older versions' nftables rules are left to clean up; the sidecar
    // itself stops when its handle drops
    firewall::remove_rules()
}

// Windows leftovers (NRPT rule, WinDivert driver, old service) that Linux never had
pub fn clean_stale_nrpt_rule(_log: impl Fn(&str)) {}

pub fn remove_legacy_service(_base_dir: &std::path::Path, _log: impl Fn(&str)) {}

pub fn remove_windivert_files(_base_dir: &std::path::Path) {}
