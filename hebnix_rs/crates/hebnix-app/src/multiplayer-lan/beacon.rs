// Captures and relays Rocket League's LAN discovery beacon now that there's
// no TAP adapter to read raw Ethernet frames off of. Actual game traffic no
// longer goes through Hebnix at all -- once RL is bound to a tailnet address
// via -multihome, its own UDP sockets talk directly to the peer over the
// WireGuard tunnel tsnet provides. The only thing Hebnix still needs to do
// is make sure a guest's RL process learns the host's *tailnet* address in
// the first place, since RL's own beacon broadcasts the host's real LAN
// ip:port, which guests can't reach.
//
// UNVERIFIED: this binds with SO_REUSEADDR so Hebnix can listen on RL's own
// LAN port (7777) without stealing RL's bind on the same port, the same way
// multicast listeners share a port. Windows *should* fan a broadcast/unicast
// datagram out to every SO_REUSEADDR-bound socket on the port, but this has
// not been tested against a real Rocket League LAN broadcast -- if RL's own
// socket doesn't also opt into address/port sharing, this may see nothing,
// or may fail to bind at all if RL already owns the port exclusively. This
// needs a real smoke test (even a single machine hosting a LAN match while
// Hebnix's relay is running) before relying on it further.

use std::net::SocketAddr;

use socket2::{Domain, Socket, Type};

pub struct BeaconRelay {
    socket: std::net::UdpSocket,
}

impl BeaconRelay {
    pub fn bind() -> Result<Self, String> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)
            .map_err(|error| format!("could not create the beacon relay socket: {error}"))?;
        socket
            .set_reuse_address(true)
            .map_err(|error| format!("could not set SO_REUSEADDR: {error}"))?;
        socket
            .set_broadcast(true)
            .map_err(|error| format!("could not enable broadcast: {error}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address: SocketAddr = ([0, 0, 0, 0], super::RL_LAN_PORT).into();
        socket
            .bind(&address.into())
            .map_err(|error| format!("could not bind the beacon relay to UDP {}: {error}", super::RL_LAN_PORT))?;
        Ok(Self {
            socket: socket.into(),
        })
    }

    pub fn try_receive(&self) -> Option<(Vec<u8>, SocketAddr)> {
        let mut buffer = [0u8; 2048];
        match self.socket.recv_from(&mut buffer) {
            Ok((length, peer)) => Some((buffer[..length].to_vec(), peer)),
            Err(_) => None,
        }
    }

    pub fn send_to(&self, payload: &[u8], destination: SocketAddr) -> Result<(), String> {
        self.socket
            .send_to(payload, destination)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
