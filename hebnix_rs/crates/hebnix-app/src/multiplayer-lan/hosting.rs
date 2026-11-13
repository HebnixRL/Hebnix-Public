use std::sync::Arc;

use crossbeam_channel::Sender;

use super::map_sync::{MapFileProvider, MapProvider, MapSync, PeerOffer};
use super::TailcatSidecarHandle;
use crate::messages::AppMsg;

/// One Workshop multiplayer session. All of Rocket
/// League's LAN traffic (discovery and game) goes through the sidecar's
/// loopback proxies, so what's left here is map sync.
pub struct HostSession {
    // kept alive so the tunnel stays up for as long as the session does;
    // map sync also polls it for the live peer list
    _sidecar: Arc<TailcatSidecarHandle>,
    // None if its port couldn't be bound; LAN play works without it
    map_sync: Option<MapSync>,
}

impl std::fmt::Debug for HostSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostSession")
            .finish_non_exhaustive()
    }
}

impl HostSession {
    pub fn start(
        sidecar: Arc<TailcatSidecarHandle>,
        map_provider: MapProvider,
        map_files: MapFileProvider,
        tx: Sender<AppMsg>,
    ) -> Result<Self, String> {
        // every player serves their maps and polls everyone else's; the
        // sidecar carries both directions over loopback
        let map_sync = match MapSync::start(sidecar.clone(), map_provider, map_files) {
            Ok(sync) => Some(sync),
            Err(error) => {
                let _ = tx.send(AppMsg::Log(format!(
                    "[Core] Map sync unavailable: {}",
                    super::redact(&error)
                )));
                None
            }
        };
        Ok(Self {
            _sidecar: sidecar,
            map_sync,
        })
    }

    /// workshop maps online peers say they have installed
    pub fn peer_offers(&self) -> Vec<PeerOffer> {
        self.map_sync
            .as_ref()
            .map(MapSync::offers)
            .unwrap_or_default()
    }

    /// poll every peer for their maps right away
    pub fn poll_now(&self) {
        if let Some(map_sync) = &self.map_sync {
            map_sync.poll_now();
        }
    }

    /// hides a player's maps and refuses their map sync connections for the
    /// rest of this session
    pub fn block(&self, ip: std::net::IpAddr, label: String) {
        if let Some(map_sync) = &self.map_sync {
            map_sync.block(ip, label);
        }
    }

    pub fn unblock(&self, ip: std::net::IpAddr) {
        if let Some(map_sync) = &self.map_sync {
            map_sync.unblock(ip);
        }
    }

    /// players blocked this session, by name
    pub fn blocked(&self) -> Vec<(std::net::IpAddr, String)> {
        self.map_sync
            .as_ref()
            .map(MapSync::blocked)
            .unwrap_or_default()
    }

    pub fn suspend(&mut self) {
        self.map_sync = None;
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.suspend();
        Ok(())
    }
}
