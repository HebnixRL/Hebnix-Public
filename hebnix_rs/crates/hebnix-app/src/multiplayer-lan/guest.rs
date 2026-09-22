use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use super::{
    JoinRoomRequest, JoinedRoom, RoomClient, SESSION_HEARTBEAT_INTERVAL, TsnetSidecarHandle,
    TunnelStats,
};

pub struct GuestSession {
    pub joined: JoinedRoom,
    pub stats: Arc<TunnelStats>,
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
    // kept alive only so the tailnet connection stays up while joined
    _sidecar: Arc<TsnetSidecarHandle>,
}

impl std::fmt::Debug for GuestSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestSession")
            .finish_non_exhaustive()
    }
}

impl GuestSession {
    /// There's nothing left for Hebnix to relay on the guest side: once
    /// Rocket League is launched with `-multihome=<our tailnet ip>`, its own
    /// UDP sockets receive the host's beacon (which the host unicasts
    /// directly to that address, see hosting.rs) and talk to the host
    /// exactly as they would on a real LAN. All that's left here is the
    /// periodic re-join used as a heartbeat, matching the host's heartbeat
    /// cadence.
    pub fn start(
        joined: JoinedRoom,
        identity: JoinRoomRequest,
        sidecar: Arc<TsnetSidecarHandle>,
    ) -> Result<Self, String> {
        let stats = Arc::new(TunnelStats::default());
        stats
            .connected
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (stop, rx) = mpsc::channel();
        let heartbeat_pin = joined.room.pin.clone();
        let worker = thread::spawn(move || {
            let client = RoomClient::new(super::ROOM_API_BASE_URL);
            let mut next_heartbeat = std::time::Instant::now() + SESSION_HEARTBEAT_INTERVAL;
            loop {
                if rx.try_recv().is_ok() {
                    break;
                }
                if std::time::Instant::now() >= next_heartbeat {
                    let _ = client.join_room(&heartbeat_pin, &identity);
                    next_heartbeat += SESSION_HEARTBEAT_INTERVAL;
                }
                thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        Ok(Self {
            joined,
            stats,
            stop,
            worker: Some(worker),
            _sidecar: sidecar,
        })
    }

    pub fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn leave(&mut self) -> Result<(), String> {
        self.stop();
        RoomClient::new(super::ROOM_API_BASE_URL)
            .leave_room(&self.joined.room.pin, &self.joined.leave_token)
    }
}

impl Drop for GuestSession {
    fn drop(&mut self) {
        self.stop();
    }
}
