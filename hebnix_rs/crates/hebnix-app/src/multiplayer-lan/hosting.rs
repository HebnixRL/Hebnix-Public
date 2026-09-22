use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::Ordering,
    mpsc::{self, Sender},
};
use std::thread::{self, JoinHandle};

use super::beacon::BeaconRelay;
use super::{
    CreateRoomRequest, PACKET_PUMP_INTERVAL, RoomClient, RoomCredentials, SESSION_HEARTBEAT_INTERVAL,
    TsnetSidecarHandle, TunnelStats,
};

pub struct HostSession {
    pub credentials: RoomCredentials,
    pub stats: Arc<TunnelStats>,
    client: RoomClient,
    stop_sender: Sender<()>,
    worker: Option<JoinHandle<()>>,
    // kept alive only so the tailnet connection stays up for as long as
    // hosting does; nothing here reads from it directly
    _sidecar: Arc<TsnetSidecarHandle>,
}

impl std::fmt::Debug for HostSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostSession")
            .finish_non_exhaustive()
    }
}

impl HostSession {
    /// `host_tailnet_ip` is this machine's own address on the tailnet
    /// (learned from the sidecar before this is called); `request.port` is
    /// only informational now (see models.rs::HostEndpoint) since guests no
    /// longer connect to a Hebnix-owned tunnel port at all -- Rocket League
    /// itself talks directly to peers once `-multihome` is set.
    pub fn start(
        client: RoomClient,
        request: CreateRoomRequest,
        sidecar: Arc<TsnetSidecarHandle>,
        host_tailnet_ip: String,
    ) -> Result<Self, String> {
        let host_octets = parse_ipv4(&host_tailnet_ip)?;
        let relay = BeaconRelay::bind()?;
        let (room, credentials) = client.create_room(&request)?;
        let stats = Arc::new(TunnelStats::default());
        let (stop_sender, stop_receiver) = mpsc::channel();
        let refresh_client = client.clone();
        let pin = credentials.pin.clone();
        let host_secret = credentials.host_secret.clone();
        let worker_stats = stats.clone();
        let worker = thread::spawn(move || {
            let mut next_heartbeat = std::time::Instant::now() + SESSION_HEARTBEAT_INTERVAL;
            // learned from the room's player list on each heartbeat; the
            // host has no other way to find out a guest's tailnet address
            let mut guest_addresses: Vec<SocketAddr> = Vec::new();
            loop {
                if stop_receiver.try_recv().is_ok() {
                    break;
                }
                if let Some((payload, _source)) = relay.try_receive() {
                    let rewritten = rewrite_lan_beacon_payload(payload, host_octets);
                    for &guest in &guest_addresses {
                        if relay.send_to(&rewritten, guest).is_ok() {
                            worker_stats.sent.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut value) = worker_stats.last_beacon_relayed.lock() {
                                *value = format!("beacon → {guest}");
                            }
                        }
                    }
                }
                if std::time::Instant::now() >= next_heartbeat {
                    if let Ok(room) = refresh_client.heartbeat(&pin, &host_secret) {
                        guest_addresses = room
                            .players
                            .iter()
                            .filter_map(|player| {
                                format!("{}:{}", player.tailnet_ip, super::RL_LAN_PORT)
                                    .parse()
                                    .ok()
                            })
                            .collect();
                        worker_stats
                            .connected
                            .store(!guest_addresses.is_empty(), Ordering::Relaxed);
                    }
                    next_heartbeat += SESSION_HEARTBEAT_INTERVAL;
                }
                thread::sleep(PACKET_PUMP_INTERVAL);
            }
        });
        let _ = room; // room details already folded into `credentials`/heartbeat above
        Ok(Self {
            credentials,
            stats,
            client,
            stop_sender,
            worker: Some(worker),
            _sidecar: sidecar,
        })
    }

    pub fn suspend(&mut self) {
        let _ = self.stop_sender.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.suspend();
        self.client
            .close_room(&self.credentials.pin, &self.credentials.host_secret)
    }
}

fn parse_ipv4(address: &str) -> Result<[u8; 4], String> {
    address
        .parse::<std::net::Ipv4Addr>()
        .map(|value| value.octets())
        .map_err(|_| format!("invalid tailnet address: {address}"))
}

/// Rewrites the host's real LAN ip:port embedded in Rocket League's LAN
/// discovery beacon to point at `address` (a tailnet IP) instead, so a
/// guest that can't reach the host's actual LAN address gets pointed at one
/// it can. Operates on the raw UDP payload only -- no more IP/UDP header or
/// checksum work needed, since this is now sent via an ordinary
/// `UdpSocket::send_to` rather than spliced into a captured Ethernet frame.
pub(crate) fn rewrite_lan_beacon_payload(mut payload: Vec<u8>, address: [u8; 4]) -> Vec<u8> {
    let address_string = format!(
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    );
    if let Some((offset, source_len, replacement)) =
        find_unreal_lan_endpoint(&payload, &address_string)
    {
        payload.splice(offset..offset + source_len, replacement);
    } else {
        let _ = replace_binary_lan_endpoint(&mut payload, address)
            || replace_equal_length_ascii_endpoint(&mut payload, &address_string);
    }
    payload
}

fn find_unreal_lan_endpoint(
    payload: &[u8],
    replacement_ip: &str,
) -> Option<(usize, usize, Vec<u8>)> {
    for offset in 0..payload.len().saturating_sub(4) {
        let length = i32::from_le_bytes(payload[offset..offset + 4].try_into().ok()?);
        if (2..=64).contains(&length) {
            let length = length as usize;
            let end = offset + 4 + length;
            if end <= payload.len() && payload[end - 1] == 0 {
                if let Ok(value) = std::str::from_utf8(&payload[offset + 4..end - 1]) {
                    if is_lan_game_endpoint(value) {
                        return Some((
                            offset,
                            4 + length,
                            unreal_ansi_string(&format!("{replacement_ip}:{}", super::RL_LAN_PORT)),
                        ));
                    }
                }
            }
        }
        if (-64..=-2).contains(&length) {
            let chars = (-length) as usize;
            let end = offset + 4 + chars * 2;
            if end <= payload.len() && payload[end - 2..end] == [0, 0] {
                let values = payload[offset + 4..end - 2]
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect::<Vec<_>>();
                if let Ok(value) = String::from_utf16(&values) {
                    if is_lan_game_endpoint(&value) {
                        return Some((
                            offset,
                            4 + chars * 2,
                            unreal_utf16_string(&format!("{replacement_ip}:{}", super::RL_LAN_PORT)),
                        ));
                    }
                }
            }
        }
    }
    None
}

fn replace_binary_lan_endpoint(payload: &mut [u8], replacement: [u8; 4]) -> bool {
    if payload.len() < 6 {
        return false;
    }
    let port = super::RL_LAN_PORT;
    for offset in 0..=payload.len() - 6 {
        let candidate = [
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ];
        if candidate == replacement || !std::net::Ipv4Addr::from(candidate).is_private() {
            continue;
        }
        let next = [payload[offset + 4], payload[offset + 5]];
        if u16::from_be_bytes(next) == port || u16::from_le_bytes(next) == port {
            payload[offset..offset + 4].copy_from_slice(&replacement);
            return true;
        }
    }
    false
}

fn replace_equal_length_ascii_endpoint(payload: &mut [u8], replacement: &str) -> bool {
    let port_suffix = format!(":{}", super::RL_LAN_PORT);
    let replacement = format!("{replacement}{port_suffix}");
    let suffix_bytes = port_suffix.as_bytes();
    let suffix_len = suffix_bytes.len();
    if payload.len() < suffix_len {
        return false;
    }
    for end in suffix_len..=payload.len() {
        if payload[end - suffix_len..end] != *suffix_bytes {
            continue;
        }
        let mut start = end - suffix_len;
        while start > 0 && (payload[start - 1].is_ascii_digit() || payload[start - 1] == b'.') {
            start -= 1;
        }
        let value = std::str::from_utf8(&payload[start..end]).ok();
        if value.is_some_and(is_lan_game_endpoint) && end - start == replacement.len() {
            payload[start..end].copy_from_slice(replacement.as_bytes());
            return true;
        }
    }
    false
}

fn is_lan_game_endpoint(value: &str) -> bool {
    let Some((address, port)) = value.rsplit_once(':') else {
        return false;
    };
    port.parse::<u16>().is_ok_and(|port| port == super::RL_LAN_PORT)
        && address.parse::<std::net::Ipv4Addr>().is_ok()
}

fn unreal_ansi_string(value: &str) -> Vec<u8> {
    let mut bytes = ((value.len() + 1) as i32).to_le_bytes().to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    bytes
}

fn unreal_utf16_string(value: &str) -> Vec<u8> {
    let chars = value.encode_utf16().count() + 1;
    let mut bytes = (-(chars as i32)).to_le_bytes().to_vec();
    for character in value.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&character.to_le_bytes());
    }
    bytes
}

impl Drop for HostSession {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::{find_unreal_lan_endpoint, replace_binary_lan_endpoint, replace_equal_length_ascii_endpoint, unreal_ansi_string};

    #[test]
    fn rewrites_the_physical_lan_endpoint_to_the_tailnet_host() {
        let payload = unreal_ansi_string("192.168.0.119:7777");
        let (_, _, replacement) = find_unreal_lan_endpoint(&payload, "100.64.0.1")
            .expect("the LAN endpoint should be found");
        assert_eq!(replacement, unreal_ansi_string("100.64.0.1:7777"));
    }

    #[test]
    fn rewrites_binary_and_equal_length_lan_endpoints() {
        let tailnet_octets = [100, 64, 0, 1];
        let mut binary = [172, 31, 64, 1, 0x1e, 0x61];
        assert!(replace_binary_lan_endpoint(&mut binary, tailnet_octets));
        assert_eq!(&binary[..4], &tailnet_octets);
        // same byte length as the source address -- this rewrite only fires
        // on an exact-length match, by design (see replace_equal_length_ascii_endpoint)
        let mut text = b"172.31.64.1:7777".to_vec();
        assert!(replace_equal_length_ascii_endpoint(&mut text, "100.64.77.1"));
        assert_eq!(text, b"100.64.77.1:7777");
    }
}
