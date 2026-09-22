// Drives the bundled `hebnix-tsnet-sidecar.exe` (a Go process embedding
// Tailscale's tsnet) over a loopback TCP + newline-delimited-JSON control
// connection. The sidecar owns the actual tsnet/WireGuard node; this module
// just spawns it, reads its one-line stdout handshake to learn which port it
// bound, and exchanges commands/events with it afterwards.
//
// Kept isolated in its own process (rather than linked into hebnix-app via
// cgo) so a tsnet/wireguard-go panic or crash can't take Hebnix down, and so
// it can be restarted independently of the rest of the app.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};

use crate::messages::AppMsg;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const SUPPORTED_PROTOCOL_VERSION: u32 = 1;

const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsState {
    Stopped,
    Starting,
    Connecting,
    Connected,
    Backoff,
}

impl TsState {
    fn parse(value: &str) -> Self {
        match value {
            "starting" => TsState::Starting,
            "connecting" => TsState::Connecting,
            "connected" => TsState::Connected,
            "backoff" => TsState::Backoff,
            _ => TsState::Stopped,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub tailnet_ip: String,
    pub hostname: String,
    pub online: bool,
}

#[derive(Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum SidecarCommand {
    Up {
        auth_key: String,
        hostname: String,
        control_url: String,
    },
    Status,
    Down,
    Shutdown,
}

#[derive(Deserialize)]
struct ReadyHandshake {
    #[serde(rename = "type")]
    kind: String,
    protocol_version: u32,
    port: u16,
}

#[derive(Deserialize)]
struct RawPeerInfo {
    tailnet_ip: String,
    hostname: String,
    online: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SidecarMessage {
    UpResult {
        ok: bool,
        error: Option<String>,
        tailnet_ip: Option<String>,
    },
    StatusResult {
        state: String,
        tailnet_ip: Option<String>,
        #[serde(default)]
        peers: Vec<RawPeerInfo>,
    },
    Event {
        kind: String,
        tailnet_ip: Option<String>,
    },
    DownResult {
        ok: bool,
    },
}

/// A handle to a running (or starting) sidecar process. Commands are
/// fire-and-forget over the control socket; results/events arrive
/// asynchronously as `AppMsg::Tsnet*` variants on the app's message channel,
/// following the same background-thread -> mpsc -> egui-main-thread pattern
/// used elsewhere in hebnix-app (see `app.rs`'s stats/game-event handling).
pub struct TsnetSidecarHandle {
    child: Child,
    control: Arc<Mutex<TcpStream>>,
}

impl TsnetSidecarHandle {
    /// Spawns the sidecar binary, waits for its ready handshake, connects the
    /// control socket, and starts background threads forwarding its stdout
    /// log lines and control-connection messages into `tx`.
    pub fn spawn(exe_path: &std::path::Path, state_dir: &std::path::Path, tx: Sender<AppMsg>) -> Result<Self, String> {
        let mut command = Command::new(exe_path);
        command
            .arg("--state-dir")
            .arg(state_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to launch the multiplayer helper process: {error}"))?;

        let mut stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| "sidecar process had no stdout".to_string())?,
        );
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "sidecar process had no stderr".to_string())?;

        let ready = read_ready_handshake(&mut stdout)?;
        if ready.kind != "ready" {
            return Err(format!("unexpected sidecar handshake: {}", ready.kind));
        }
        if ready.protocol_version != SUPPORTED_PROTOCOL_VERSION {
            return Err(format!(
                "the multiplayer helper is a different version than Hebnix expects (helper={}, hebnix={}); reinstall Hebnix",
                ready.protocol_version, SUPPORTED_PROTOCOL_VERSION
            ));
        }

        let stream = connect_with_timeout(ready.port, CONTROL_CONNECT_TIMEOUT)?;
        let control = Arc::new(Mutex::new(stream));

        spawn_log_forwarder(stdout, tx.clone(), "tsnet");
        spawn_log_forwarder(BufReader::new(stderr), tx.clone(), "tsnet");
        spawn_control_reader(control.clone(), tx);

        Ok(Self { child, control })
    }

    pub fn request_up(&self, auth_key: String, hostname: String, control_url: String) -> Result<(), String> {
        self.send(&SidecarCommand::Up {
            auth_key,
            hostname,
            control_url,
        })
    }

    pub fn request_status(&self) -> Result<(), String> {
        self.send(&SidecarCommand::Status)
    }

    pub fn request_down(&self) -> Result<(), String> {
        self.send(&SidecarCommand::Down)
    }

    /// Tells the sidecar to tear down and exit; does not wait for the
    /// process to actually terminate, callers that need that should follow
    /// up with `wait_for_exit`.
    pub fn request_shutdown(&self) -> Result<(), String> {
        self.send(&SidecarCommand::Shutdown)
    }

    pub fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(Some(_)) = self.child.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn send(&self, command: &SidecarCommand) -> Result<(), String> {
        let mut line = serde_json::to_string(command).map_err(|error| error.to_string())?;
        line.push('\n');
        let mut stream = self
            .control
            .lock()
            .map_err(|_| "multiplayer helper connection is unavailable".to_string())?;
        stream
            .write_all(line.as_bytes())
            .map_err(|error| format!("lost connection to the multiplayer helper: {error}"))
    }
}

impl Drop for TsnetSidecarHandle {
    fn drop(&mut self) {
        let _ = self.request_shutdown();
        if !self.wait_for_exit(Duration::from_secs(3)) {
            self.kill();
        }
    }
}

fn read_ready_handshake<R: BufRead>(reader: &mut R) -> Result<ReadyHandshake, String> {
    // No explicit timeout here: if the sidecar hangs without printing
    // anything, the blocking pipe read just blocks. A crashed or missing
    // sidecar is caught instead because the OS closes the pipe (Ok(0)) or
    // the spawn itself already failed above. Callers that need a hard
    // deadline should run this behind their own watchdog thread.
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err("multiplayer helper exited before it was ready".to_string()),
        Ok(_) => serde_json::from_str(line.trim())
            .map_err(|error| format!("could not understand the multiplayer helper's startup message: {error}")),
        Err(error) => Err(format!("failed to read the multiplayer helper's startup message: {error}")),
    }
}

fn connect_with_timeout(port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| format!("failed to connect to the multiplayer helper on port {port}: {error}"))
}

fn spawn_log_forwarder<R: std::io::Read + Send + 'static>(reader: BufReader<R>, tx: Sender<AppMsg>, label: &'static str) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        let _ = tx.send(AppMsg::Log(format!("[{label}] {trimmed}")));
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_control_reader(control: Arc<Mutex<TcpStream>>, tx: Sender<AppMsg>) {
    std::thread::spawn(move || {
        let stream = match control.lock() {
            Ok(guard) => match guard.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            },
            Err(_) => return,
        };
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = tx.send(AppMsg::TsnetSidecarDisconnected);
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<SidecarMessage>(trimmed) {
                        Ok(message) => forward_message(message, &tx),
                        Err(error) => {
                            let _ = tx.send(AppMsg::Log(format!(
                                "[tsnet] could not parse helper message: {error}"
                            )));
                        }
                    }
                }
                Err(_) => {
                    let _ = tx.send(AppMsg::TsnetSidecarDisconnected);
                    break;
                }
            }
        }
    });
}

fn forward_message(message: SidecarMessage, tx: &Sender<AppMsg>) {
    let app_message = match message {
        SidecarMessage::UpResult { ok, error, tailnet_ip } => AppMsg::TsnetUpResult {
            result: if ok {
                Ok(tailnet_ip.unwrap_or_default())
            } else {
                Err(error.unwrap_or_else(|| "the multiplayer helper could not start".to_string()))
            },
        },
        SidecarMessage::StatusResult { state, tailnet_ip, peers } => AppMsg::TsnetStatus {
            state: TsState::parse(&state),
            tailnet_ip,
            peers: peers
                .into_iter()
                .map(|peer| PeerInfo {
                    tailnet_ip: peer.tailnet_ip,
                    hostname: peer.hostname,
                    online: peer.online,
                })
                .collect(),
        },
        SidecarMessage::Event { kind, tailnet_ip } => AppMsg::TsnetPeerEvent {
            online: kind == "peer_online",
            tailnet_ip: tailnet_ip.unwrap_or_default(),
        },
        SidecarMessage::DownResult { ok } => AppMsg::TsnetDownResult { ok },
    };
    let _ = tx.send(app_message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Drives the real built sidecar binary through spawn -> ready handshake
    // -> status round-trip -> shutdown. Ignored by default because it needs
    // the Go sidecar built separately (see sidecar/README or the tsnet
    // rework plan) and its path passed via HEBNIX_TSNET_SIDECAR_EXE; this
    // isn't wired into the normal cargo build yet.
    //
    // Run with:
    //   HEBNIX_TSNET_SIDECAR_EXE=D:/hebnix/hebnix_rs/sidecar/hebnix-tsnet-sidecar.exe \
    //     cargo test -p hebnix-app --release tsnet_sidecar::tests -- --ignored --nocapture
    #[test]
    #[ignore = "requires the separately-built Go sidecar binary"]
    fn spawn_status_and_shutdown_round_trip() {
        let exe_path = std::env::var("HEBNIX_TSNET_SIDECAR_EXE")
            .expect("set HEBNIX_TSNET_SIDECAR_EXE to the built hebnix-tsnet-sidecar.exe path");
        let state_dir = std::env::temp_dir().join("hebnix-tsnet-sidecar-test-state");
        std::fs::create_dir_all(&state_dir).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut handle = TsnetSidecarHandle::spawn(std::path::Path::new(&exe_path), &state_dir, tx)
            .expect("sidecar failed to spawn / complete ready handshake");

        handle.request_status().expect("failed to send status command");

        let message = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no response from sidecar within 5s");
        match message {
            AppMsg::TsnetStatus { state, .. } => {
                assert_eq!(state, TsState::Stopped, "fresh sidecar should report stopped, not connected");
            }
            other => panic!("expected TsnetStatus, got {other:?}"),
        }

        handle.request_shutdown().expect("failed to send shutdown command");
        assert!(
            handle.wait_for_exit(Duration::from_secs(5)),
            "sidecar did not exit after shutdown command"
        );
    }
}
