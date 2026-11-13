// Runs `hebnix-tc` (sidecar/hebnix-tc, Go, built on tailcat) for Workshop
// multiplayer. It needs no TUN device and no special permissions. Every
// player's sidecar announces itself to the Hebnix directory (DIRECTORY_URL,
// sidecar/directory) and connects to everyone else announced there, so every
// player sees every other player's LAN matches with no codes to share; the
// in-game LAN match password keeps a match private. Each other player gets a
// loopback alias here (127.10.0.n): Rocket League joins their match at
// alias:7777 and map sync reaches them at alias:14790. Rocket League runs
// without `-multihome`, so its online play keeps working.
//
// The sidecar reports JSON events, one per line, on stdout (see its
// main.go); they arrive here as `AppMsg::Tailcat*`. It exits on its own
// when its stdin closes, so it can't outlive Hebnix, and stopping it drops
// every connection at once.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Sender;
use serde::Deserialize;

use crate::messages::AppMsg;

const BINARY: &str = if cfg!(windows) {
    "hebnix-tc.exe"
} else {
    "hebnix-tc"
};
/// how long to wait for the sidecar's first announce to the directory
const READY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// the directory's node ID for this player
    pub id: String,
    /// this player's loopback alias, where their map sync is reached
    pub tailnet_ip: String,
    pub hostname: String,
    pub online: bool,
}

/// another player's connection, for the UI
#[derive(Debug, Clone)]
pub struct PeerLink {
    pub name: String,
    pub alias: Option<std::net::IpAddr>,
    /// "direct", "relay <region>" or "unreachable"
    pub path: String,
    pub latency_ms: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Event {
    event: String,
    id: String,
    name: String,
    alias: String,
    online: Option<bool>,
    peers: Vec<EventPeer>,
    message: String,
    fatal: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EventPeer {
    id: String,
    name: String,
    path: String,
    latency_ms: Option<f64>,
}

/// A running `hebnix-tc`. Dropping it stops the sidecar.
pub struct TailcatSidecarHandle {
    child: Mutex<Option<Child>>,
    // held open: the sidecar exits when this closes
    stdin: Mutex<Option<ChildStdin>>,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
}

impl std::fmt::Debug for TailcatSidecarHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TailcatSidecarHandle")
            .finish_non_exhaustive()
    }
}

impl TailcatSidecarHandle {
    /// Starts the sidecar and waits until it has announced itself to the
    /// directory. `relay_only` sends everything through relay servers (the
    /// "Hide my IP" setting).
    pub fn spawn(
        player_name: &str,
        node_id: &str,
        relay_only: bool,
        tx: Sender<AppMsg>,
    ) -> Result<Self, String> {
        let key = super::directory_key()?;
        let binary = find_binary()?;
        let mut command = Command::new(&binary);
        if relay_only {
            command.arg("-relay-only");
        }
        command
            .arg(format!("-directory={}", super::directory_url()))
            .arg(format!("-name={player_name}"))
            // not a flag, so it doesn't show up in process listings
            .env("HEBNIX_DIRECTORY_KEY", key)
            .env("HEBNIX_WORKSHOP_NODE_ID", node_id);
        hide_console_window(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("sidecar has no stdout")?;
        let stderr = child.stderr.take().ok_or("sidecar has no stderr")?;
        spawn_log_reader(stderr);

        let peers = Arc::new(Mutex::new(HashMap::new()));
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        spawn_event_reader(stdout, peers.clone(), tx, ready_tx);

        let handle = Self {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            peers,
        };
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("the multiplayer helper did not start in time".to_string()),
        }
    }

    /// The current peer list, for a background worker (map sync) that
    /// polls on its own cadence.
    pub fn peers_now(&self) -> Result<Vec<PeerInfo>, String> {
        self.peers
            .lock()
            .map(|peers| peers.values().cloned().collect())
            .map_err(|_| "peer list unavailable".to_string())
    }

    fn stop(&self) {
        // closing stdin is the sidecar's signal to exit
        if let Ok(mut stdin) = self.stdin.lock() {
            stdin.take();
        }
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        let Some(mut process) = child.take() else {
            return;
        };
        for _ in 0..20 {
            if process.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = process.kill();
        let _ = process.wait();
    }
}

impl Drop for TailcatSidecarHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_event_reader(
    stdout: impl Read + Send + 'static,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    tx: Sender<AppMsg>,
    ready: crossbeam_channel::Sender<Result<(), String>>,
) {
    std::thread::Builder::new()
        .name("tailcat-events".into())
        .spawn(move || {
            let mut reported_ready = false;
            let mut initial_roster = true;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                let Ok(event) = serde_json::from_str::<Event>(&line) else {
                    continue;
                };
                match event.event.as_str() {
                    "ready" => {
                        reported_ready = true;
                        let _ = ready.try_send(Ok(()));
                    }
                    "peer" => {
                        let online = event.online.unwrap_or(false);
                        let name = peer_label(&event.name, &event.id);
                        if let Ok(mut peers) = peers.lock() {
                            if online {
                                peers.insert(
                                    event.id.clone(),
                                    PeerInfo {
                                        id: event.id.clone(),
                                        tailnet_ip: event.alias.clone(),
                                        hostname: name.clone(),
                                        online: true,
                                    },
                                );
                            } else {
                                peers.remove(&event.id);
                            }
                        }
                        // The first announce emits the existing peers too. Its
                        // events precede the first periodic status snapshot.
                        if !initial_roster {
                            let _ = tx.send(AppMsg::TailcatPeer {
                                id: event.id,
                                name,
                                online,
                            });
                        }
                    }
                    "status" => {
                        initial_roster = false;
                        let mut links = Vec::new();
                        if let Ok(mut peers) = peers.lock() {
                            for peer in &event.peers {
                                let alias = peers.get_mut(&peer.id).and_then(|known| {
                                    known.online = peer.path != "unreachable";
                                    known.tailnet_ip.parse().ok()
                                });
                                links.push(PeerLink {
                                    name: peer_label(&peer.name, &peer.id),
                                    alias,
                                    path: peer.path.clone(),
                                    latency_ms: peer.latency_ms,
                                });
                            }
                        }
                        let _ = tx.send(AppMsg::TailcatStatus { peers: links });
                    }
                    "error" => {
                        if !reported_ready {
                            // before the first announce, any error means
                            // this connection attempt failed
                            let _ = ready.try_send(Err(event.message.clone()));
                        }
                        let _ = tx.send(AppMsg::Log(format!(
                            "[Multiplayer] {}",
                            super::redact(&event.message)
                        )));
                    }
                    _ => {}
                }
            }
            if !reported_ready {
                let _ = ready.try_send(Err("the multiplayer helper exited".to_string()));
            }
            let _ = tx.send(AppMsg::TailcatExited);
        })
        .ok();
}

fn peer_label(name: &str, id: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        // a short tag is enough to tell two nameless players apart
        format!("Player {}", id.chars().take(4).collect::<String>())
    } else {
        name.chars().take(64).collect()
    }
}

/// the sidecar's own log goes to Hebnix's log file, not the console
fn spawn_log_reader(stderr: impl Read + Send + 'static) {
    std::thread::Builder::new()
        .name("tailcat-log".into())
        .spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::info!("hebnix-tc: {}", super::redact(&line));
            }
        })
        .ok();
}

#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

/// `%AppData%\Hebnix\multiplayer-lan`, where Hebnix extracts its embedded
/// copy on start (see multiplayer_assets.rs), else a `multiplayer-lan`
/// folder next to Hebnix or Hebnix's own folder
fn find_binary() -> Result<PathBuf, String> {
    let mut dirs = vec![crate::config::base_dir().join("multiplayer-lan")];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir.join("multiplayer-lan"));
        dirs.push(exe_dir);
    }
    dirs.iter()
        .map(|dir| dir.join(BINARY))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            let places: Vec<String> = dirs.iter().map(|dir| dir.display().to_string()).collect();
            format!(
                "the multiplayer helper ({BINARY}) is missing, looked in: {}",
                places.join(", ")
            )
        })
}

/// hides tailcat tokens and ip addresses in text that ends up on screen
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            out.push_str(&redact_word(&word));
            word.clear();
            out.push(c);
        } else {
            word.push(c);
        }
    }
    out.push_str(&redact_word(&word));
    out
}

fn redact_word(word: &str) -> String {
    // tailcat tokens are long base64ish strings starting with "tc"
    if let Some(at) = word.find("tc") {
        let rest = &word[at..];
        if rest.len() > 40 && rest.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c)) {
            return format!("{}<hidden>", &word[..at]);
        }
    }
    mask_ipv4(word)
}

/// swaps anything shaped like an ipv4 address for x.x.x.x
fn mask_ipv4(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.trim_matches('.').parse::<std::net::Ipv4Addr>().is_ok() {
            let lead = run.len() - run.trim_start_matches('.').len();
            let tail = run.len() - run.trim_end_matches('.').len();
            out.push_str(&".".repeat(lead));
            out.push_str("x.x.x.x");
            out.push_str(&".".repeat(tail));
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in word.chars() {
        if c.is_ascii_digit() || c == '.' {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}
