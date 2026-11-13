//! RL window focus + geometry, via Hyprland's IPC socket or (best-effort,
//! untested live -- see module note below) KWin/Plasma's `kdotool` +
//! `kscreen-doctor`.
//!
//! Hyprland exposes a plain-text-request/JSON-response unix socket at
//! `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`.
//! Sending `j/activewindow` or `j/clients` returns JSON describing windows,
//! keyed by process pid (`pid` field) -- so like the windows version we
//! identify RL by pid, not by title.
//!
//! KDE Plasma/KWin has no equivalent single IPC socket. The closest
//! analogue is `kdotool` (https://github.com/jinliu/kdotool, packaged as
//! `kdotool-bin` on the AUR) -- an xdotool-alike that, on each invocation,
//! generates a small KWin script, loads+runs+unloads it over KWin's own
//! scripting D-Bus interface, and prints results to stdout. Monitor
//! geometry goes through `kscreen-doctor -j` (ships with Plasma) instead,
//! since kdotool has no screen-geometry command.
//!
//! IMPORTANT: the KWin path below was written from kdotool's and
//! kscreen-doctor's own source (to get exact output formats right) but has
//! never been run against a real Plasma session -- there is no KDE install
//! to test against here. It should be treated as a first draft: expect to
//! need a few rounds of "here's what broke" from someone actually running
//! Plasma. Two things it deliberately does NOT attempt, for lack of a
//! verifiable mechanism via kdotool's builtin actions: exempting our own
//! window from blur/shadow effects, and forcing it to start floating (both
//! Hyprland-only niceties -- on KWin those functions are no-ops, so at
//! worst the window looks like an ordinary KWin window rather than
//! matching the Windows app's chromeless-floating look).
//!
//! niri has its own JSON IPC socket at `$NIRI_SOCKET`: one JSON request
//! per line (`"Windows"`, `"FocusedWindow"`, `{"Action":{...}}`), answered
//! with one `{"Ok":{...}}` / `{"Err":"..."}` line. Written from niri's IPC
//! schema (niri-ipc), not yet tested against a live niri session.
//!
//! Sway speaks i3's IPC protocol over `$SWAYSOCK` (`i3ipc.rs`): the window
//! tree gives pid, focus, geometry and workspace, and commands float and
//! raise our own window. i3 and every other X11 window manager that speaks
//! EWMH (Linux Mint's Cinnamon, XFCE, MATE, GNOME/KDE on X11) are queried
//! through x11rb (`x11.rs`); i3 additionally gets floating/workspace
//! commands over `$I3SOCK`. Written from the i3 IPC and EWMH specs and
//! unit-tested against canned trees, not yet run against a live Sway/i3.
//!
//! On any other compositor we fall back to "always focused", and treat RL
//! as covering the whole monitor (it's almost always fullscreen) so drawn
//! overlays still show -- a warning is logged once.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use super::{i3ipc, x11};
use serde::Deserialize;
use serde_json::Value;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compositor {
    Hyprland,
    Kwin,
    Niri,
    /// Sway: i3's IPC protocol over `$SWAYSOCK`
    Sway,
    /// i3: window queries over EWMH, commands over i3's IPC
    I3,
    /// any other X11 window manager that speaks EWMH (Cinnamon, XFCE, MATE,
    /// GNOME/KDE on X11, ...)
    X11,
    Other,
}

fn compositor() -> Compositor {
    static DETECTED: std::sync::OnceLock<Compositor> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| {
        detect(
            |name| std::env::var_os(name).is_some_and(|value| !value.is_empty()),
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        )
    })
}

/// `is_set(name)` says whether an environment variable is set and non-empty
fn detect(is_set: impl Fn(&str) -> bool, desktop: &str) -> Compositor {
    let wayland = is_set("WAYLAND_DISPLAY");
    let kde = desktop.to_uppercase().contains("KDE") || is_set("KDE_FULL_SESSION");
    if is_set("HYPRLAND_INSTANCE_SIGNATURE") {
        Compositor::Hyprland
    } else if is_set("NIRI_SOCKET") {
        Compositor::Niri
    } else if is_set("SWAYSOCK") {
        Compositor::Sway
    } else if kde && wayland {
        Compositor::Kwin
    } else if !wayland && is_set("DISPLAY") {
        if is_set("I3SOCK") || desktop.to_lowercase().split(':').any(|part| part == "i3") {
            Compositor::I3
        } else {
            Compositor::X11
        }
    } else if kde {
        // KDE with no usable display variables: keep the old behaviour
        Compositor::Kwin
    } else {
        Compositor::Other
    }
}

fn warn_unsupported_compositor_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "process::window: no Hyprland, KDE, niri, Sway or X11 session detected, window focus/geometry \
             tracking is unavailable -- assuming RL is always focused and fills the monitor"
        );
    });
}

fn warn_kdotool_missing_once() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "process::window: KDE detected but `kdotool` isn't on PATH -- window focus/geometry \
             tracking needs it (AUR: kdotool-bin). Falling back to \"always focused\"."
        );
    });
}

// ===================== Hyprland backend =====================

fn hyprland_socket_path() -> Option<std::path::PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime_dir =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string());
    Some(std::path::PathBuf::from(runtime_dir).join("hypr").join(sig).join(".socket.sock"))
}

/// send a plain-text hyprctl command over the IPC socket, returning the raw
/// response bytes. `j/<cmd>` gets JSON back.
fn hyprctl_raw(cmd: &str) -> Option<Vec<u8>> {
    let path = hyprland_socket_path()?;
    let mut stream = UnixStream::connect(&path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok();
    stream.write_all(cmd.as_bytes()).ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn hyprctl_json(cmd: &str) -> Option<Value> {
    let raw = hyprctl_raw(&format!("j/{cmd}"))?;
    serde_json::from_slice(&raw).ok()
}

/// is the given layer-shell surface (matched by its `namespace`, e.g.
/// "waybar") currently visible (alpha > 0)? `None` if it isn't found at
/// all (compositor other than Hyprland, or the surface isn't mapped).
///
/// Exists because toggling something via a stateless signal (like waybar's
/// SIGUSR1 hide toggle) and tracking "hidden or not" with our own boolean
/// desyncs the moment *anything else* also toggles it -- the user's own
/// Hyprland keybind for the same signal, in this case. Querying the real
/// state each time and only sending the toggle when it actually needs to
/// change is self-correcting regardless of what else touches it.
pub fn hyprland_layer_visible(namespace: &str) -> Option<bool> {
    let json = hyprctl_json("layers")?;
    let monitors = json.as_object()?;
    for mon in monitors.values() {
        let levels = mon.get("levels")?.as_object()?;
        for layer_arr in levels.values() {
            for layer in layer_arr.as_array()? {
                if layer.get("namespace").and_then(Value::as_str) == Some(namespace) {
                    return Some(layer.get("alpha").and_then(Value::as_f64).unwrap_or(0.0) > 0.0);
                }
            }
        }
    }
    None
}

#[derive(Debug, Deserialize, Default)]
struct HyprClient {
    #[serde(default)]
    pid: i64,
    #[serde(default)]
    at: Option<[i32; 2]>,
    #[serde(default)]
    size: Option<[i32; 2]>,
    #[serde(default)]
    title: String,
}

/// RL's pid via Hyprland's own window list, matched by title rather than
/// process name. Under Proton, `sysinfo`'s process scan is unreliable here
/// -- it occasionally returns a short-lived pid (looks like a worker
/// thread's tid surfacing as a "process") that also happens to share
/// RocketLeague.exe's comm name, causing the real window to intermittently
/// read as unfocused. Hyprland already knows which pid owns which window,
/// so trust that instead of re-deriving it from a process name scan.
fn hyprland_rl_pid() -> Option<u32> {
    let json = hyprctl_json("clients")?;
    let arr = json.as_array()?;
    arr.iter()
        .find(|c| {
            c.get("title")
                .and_then(Value::as_str)
                .map(|t| t.to_lowercase().contains("rocket league"))
                .unwrap_or(false)
        })
        .and_then(|c| c.get("pid"))
        .and_then(Value::as_i64)
        .map(|p| p as u32)
}

/// which workspace RL's own window currently lives on (Hyprland-only).
fn hyprland_rl_workspace(pid: u32) -> Option<i64> {
    let json = hyprctl_json("clients")?;
    let arr = json.as_array()?;
    arr.iter().find(|c| c.get("pid").and_then(Value::as_i64) == Some(pid as i64))
        .and_then(|c| c.get("workspace"))
        .and_then(|w| w.get("id"))
        .and_then(Value::as_i64)
}

// ===================== KWin backend (kdotool + kscreen-doctor) =====================

/// run kdotool, returning stdout lines (trimmed, empty lines dropped) on a
/// zero exit status. `None` (rather than panicking or spamming) if
/// kdotool isn't installed or the call otherwise fails -- callers treat
/// that the same as "no window found".
fn kdotool_lines(args: &[&str]) -> Option<Vec<String>> {
    let output = match std::process::Command::new("kdotool").args(args).output() {
        Ok(o) => o,
        Err(_) => {
            warn_kdotool_missing_once();
            return None;
        }
    };
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(
        stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn kdotool_line(args: &[&str]) -> Option<String> {
    kdotool_lines(args)?.into_iter().next()
}

/// RL's pid via kdotool, matched by window caption the same way the
/// Hyprland path matches by title. `search --name` takes a regex against
/// the window caption; "Rocket League" has no special-regex characters so
/// no escaping is needed. `--limit 1` keeps this to the first match;
/// chaining `getwindowpid` onto the same invocation runs it against
/// whatever `search` put on the window stack, xdotool-style.
fn kwin_rl_pid() -> Option<u32> {
    kdotool_line(&["search", "--name", "Rocket League", "--limit", "1", "getwindowpid"])?
        .parse()
        .ok()
}

/// (left, top, right, bottom) via `search --name <title> --limit 1
/// getwindowgeometry`, which prints exactly three lines:
///   Window {id}
///     Position: x,y
///     Geometry: WxH
///
/// Takes the window by title, not pid: `search --pid <pid>` was confirmed
/// live to ignore the pid filter entirely on this kdotool build (returns
/// *every* window in the session regardless of which pid is given), so
/// this used to silently grab whatever window happened to come first in
/// kdotool's own listing order - anything from Steam to a file manager,
/// not necessarily Rocket League at all.
fn kwin_rl_window_rect() -> Option<(i32, i32, i32, i32)> {
    let lines =
        kdotool_lines(&["search", "--name", "Rocket League", "--limit", "1", "getwindowgeometry"])?;
    let pos_line = lines.iter().find(|l| l.contains("Position:"))?;
    let geo_line = lines.iter().find(|l| l.contains("Geometry:"))?;
    let (x, y) = pos_line.split("Position:").nth(1)?.trim().split_once(',')?;
    let (w, h) = geo_line.split("Geometry:").nth(1)?.trim().split_once('x')?;
    let (left, top) = (x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?);
    let (width, height) = (w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?);
    let (right, bottom) = (left + width, top + height);
    if right > left && bottom > top {
        Some((left, top, right, bottom))
    } else {
        None
    }
}

fn kscreen_doctor_json() -> Option<Value> {
    let output = std::process::Command::new("kscreen-doctor").arg("-j").output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

// ===================== niri backend =====================

/// one request over niri's IPC socket. `request` is the JSON request value
/// (`"Windows"`, or `{"Action":{..}}`); returns the payload inside the
/// `{"Ok": ...}` reply, e.g. `{"Windows":[..]}`.
fn niri_request(request: &Value) -> Option<Value> {
    use std::io::BufRead;
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();
    stream.set_write_timeout(Some(Duration::from_millis(500))).ok();
    let mut line = serde_json::to_vec(request).ok()?;
    line.push(b'\n');
    stream.write_all(&line).ok()?;
    let mut reply = String::new();
    std::io::BufReader::new(stream).read_line(&mut reply).ok()?;
    let mut value: Value = serde_json::from_str(&reply).ok()?;
    value.get_mut("Ok").map(Value::take)
}

fn niri_query(name: &str) -> Option<Value> {
    niri_request(&Value::String(name.to_string()))?.get_mut(name).map(Value::take)
}

fn niri_action(action: Value) -> bool {
    niri_request(&serde_json::json!({ "Action": action })).is_some()
}

#[derive(Debug, Deserialize, Default)]
struct NiriWindow {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    workspace_id: Option<u64>,
    /// only on newer niri (25.08+)
    #[serde(default)]
    layout: Option<NiriWindowLayout>,
}

#[derive(Debug, Deserialize, Default)]
struct NiriWindowLayout {
    #[serde(default)]
    window_size: Option<[i32; 2]>,
    /// tile position within the workspace view, i.e. relative to its
    /// output; None while the window is off screen
    #[serde(default)]
    tile_pos_in_workspace_view: Option<[f64; 2]>,
    #[serde(default)]
    window_offset_in_tile: Option<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct NiriWorkspace {
    id: u64,
    #[serde(default)]
    output: Option<String>,
}

/// (x, y, width, height) of an output: logical position, physical size
/// (the physical size is what the monitor-size helpers report elsewhere)
fn niri_output_rects() -> Vec<(String, (i32, i32, i32, i32))> {
    let Some(outputs) = niri_query("Outputs") else {
        return Vec::new();
    };
    let Some(outputs) = outputs.as_object() else {
        return Vec::new();
    };
    outputs
        .iter()
        .filter_map(|(name, output)| {
            let logical = output.get("logical")?;
            let x = logical.get("x")?.as_i64()? as i32;
            let y = logical.get("y")?.as_i64()? as i32;
            let mode = output
                .get("current_mode")
                .and_then(Value::as_u64)
                .and_then(|index| output.get("modes")?.get(index as usize));
            let (w, h) = match mode {
                Some(mode) => (
                    mode.get("width")?.as_i64()? as i32,
                    mode.get("height")?.as_i64()? as i32,
                ),
                None => (
                    logical.get("width")?.as_i64()? as i32,
                    logical.get("height")?.as_i64()? as i32,
                ),
            };
            Some((name.clone(), (x, y, w, h)))
        })
        .collect()
}

fn niri_windows() -> Vec<NiriWindow> {
    niri_query("Windows")
        .and_then(|windows| serde_json::from_value(windows).ok())
        .unwrap_or_default()
}

fn niri_rl_window() -> Option<NiriWindow> {
    niri_windows().into_iter().find(|window| {
        window
            .title
            .as_deref()
            .is_some_and(|title| title.to_lowercase().contains("rocket league"))
    })
}

fn niri_rl_pid() -> Option<u32> {
    niri_rl_window()?.pid.map(|pid| pid as u32)
}

fn niri_focused_pid() -> Option<i64> {
    let window: NiriWindow = serde_json::from_value(niri_query("FocusedWindow")?).ok()?;
    window.pid
}

/// the output rect RL's workspace is shown on
fn niri_rl_output_rect(rl: &NiriWindow) -> Option<(i32, i32, i32, i32)> {
    let outputs = niri_output_rects();
    let workspaces: Vec<NiriWorkspace> =
        serde_json::from_value(niri_query("Workspaces")?).ok()?;
    let output = rl
        .workspace_id
        .and_then(|id| workspaces.into_iter().find(|ws| ws.id == id))
        .and_then(|ws| ws.output);
    output
        .and_then(|name| outputs.iter().find(|(n, _)| *n == name).map(|(_, rect)| *rect))
        .or_else(|| outputs.first().map(|(_, rect)| *rect))
}

/// RL's window rect from niri's layout info when it has it (newer niri),
/// otherwise the whole output RL is on - RL is nearly always fullscreen
fn niri_rl_window_rect() -> Option<(i32, i32, i32, i32)> {
    let rl = niri_rl_window()?;
    let output = niri_rl_output_rect(&rl);
    let from_layout = rl.layout.as_ref().and_then(|layout| {
        let [w, h] = layout.window_size?;
        let [tx, ty] = layout.tile_pos_in_workspace_view?;
        let [ox, oy] = layout.window_offset_in_tile.unwrap_or([0.0, 0.0]);
        let (base_x, base_y) = output.map_or((0, 0), |(x, y, _, _)| (x, y));
        let left = base_x + (tx + ox).round() as i32;
        let top = base_y + (ty + oy).round() as i32;
        (w > 0 && h > 0).then_some((left, top, left + w, top + h))
    });
    from_layout.or_else(|| output.map(|(x, y, w, h)| (x, y, x + w, y + h)))
}

// ===================== Sway / i3 / X11 backends =====================

/// a window as seen by the Sway, i3 or X11 backends
#[derive(Clone, Debug)]
struct GenWin {
    pid: Option<u32>,
    class: String,
    title: String,
    rect: (i32, i32, i32, i32),
    focused: bool,
    /// Sway/i3 workspace name
    workspace: Option<String>,
    /// X11 virtual desktop
    desktop: Option<u32>,
}

/// our own window's fixed title (see main.rs's with_title)
const OWN_TITLE: &str = "Hebnix";

/// every window the backend can see. None when the window manager can't be
/// asked at all (no IPC socket, no EWMH), which callers treat as "no window
/// tracking" rather than "no windows".
fn generic_windows(compositor: Compositor) -> Option<Vec<GenWin>> {
    match compositor {
        Compositor::Sway => {
            let tree = i3ipc::tree()?;
            Some(
                i3ipc::windows_from_tree(&tree)
                    .into_iter()
                    .map(|w| GenWin {
                        pid: w.pid,
                        class: w.class,
                        title: w.title,
                        rect: w.rect,
                        focused: w.focused,
                        workspace: w.workspace,
                        desktop: None,
                    })
                    .collect(),
            )
        }
        Compositor::I3 | Compositor::X11 => Some(
            x11::windows()?
                .into_iter()
                .map(|w| GenWin {
                    pid: w.pid,
                    class: w.class,
                    title: w.title,
                    rect: w.rect,
                    focused: w.focused,
                    workspace: None,
                    desktop: w.desktop,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn generic_outputs(compositor: Compositor) -> Vec<(i32, i32, i32, i32)> {
    match compositor {
        Compositor::Sway => i3ipc::tree()
            .map(|tree| i3ipc::outputs_from_tree(&tree))
            .unwrap_or_default(),
        Compositor::I3 | Compositor::X11 => x11::monitors(),
        _ => Vec::new(),
    }
}

fn is_rl_window(window: &GenWin) -> bool {
    let class = window.class.to_lowercase();
    let title = window.title.to_lowercase();
    class.contains("rocketleague")
        || class.contains("steam_app_252950")
        || title.starts_with("rocket league (")
}

/// the RL window: by pid when known, else by class/title
fn generic_rl_window(windows: &[GenWin], pid: Option<u32>) -> Option<&GenWin> {
    windows
        .iter()
        .find(|w| pid.is_some() && w.pid == pid)
        .or_else(|| windows.iter().find(|w| is_rl_window(w)))
}

fn generic_rl_pid(compositor: Compositor) -> Option<u32> {
    generic_windows(compositor)?
        .iter()
        .find(|w| is_rl_window(w))
        .and_then(|w| w.pid)
}

/// the output rect containing the middle of `rect`, else the first output
fn output_for(outputs: &[(i32, i32, i32, i32)], rect: Option<(i32, i32, i32, i32)>) -> Option<(i32, i32, i32, i32)> {
    if let Some((l, t, r, b)) = rect {
        let (cx, cy) = ((l + r) / 2, (t + b) / 2);
        if let Some(found) = outputs
            .iter()
            .find(|(ol, ot, or, ob)| cx >= *ol && cx < *or && cy >= *ot && cy < *ob)
        {
            return Some(*found);
        }
    }
    outputs.first().copied()
}

/// sway/i3 criteria selecting our own window
fn own_criteria(compositor: Compositor, own_pid: u32) -> String {
    if compositor == Compositor::Sway {
        format!("[pid={own_pid}]")
    } else {
        format!("[title=\"^{OWN_TITLE}$\"]")
    }
}

fn quote_workspace(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

/// RL's workspace name, from the i3/Sway tree
fn i3_rl_workspace() -> Option<String> {
    let tree = i3ipc::tree()?;
    i3ipc::windows_from_tree(&tree)
        .into_iter()
        .find(|w| {
            let class = w.class.to_lowercase();
            class.contains("rocketleague")
                || class.contains("steam_app_252950")
                || w.title.to_lowercase().starts_with("rocket league (")
        })
        .and_then(|w| w.workspace)
}

/// float our window (so it isn't tiled next to the game), drop its border and
/// optionally move it to RL's workspace and focus it
fn i3_float_own(compositor: Compositor, own_pid: u32, pop_over: bool) -> bool {
    let sel = own_criteria(compositor, own_pid);
    let mut ok = i3ipc::run_command(&format!("{sel} floating enable, border none"));
    if pop_over {
        if let Some(workspace) = i3_rl_workspace() {
            i3ipc::run_command(&format!("{sel} move to workspace {}", quote_workspace(&workspace)));
        }
        ok &= i3ipc::run_command(&format!("{sel} focus"));
    }
    ok
}

// ===================== shared cache + dispatch =====================

/// cached RL pid, the process scan / kdotool round-trip costs real time and
/// the focus helpers run several times a second.
fn cached_rl_pid() -> Option<u32> {
    static CACHE: Mutex<Option<(Instant, Option<u32>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((ts, pid)) = *cache {
        if ts.elapsed() < Duration::from_secs(3) {
            return pid;
        }
    }
    let pid = match compositor() {
        Compositor::Hyprland => hyprland_rl_pid(),
        Compositor::Kwin => kwin_rl_pid(),
        Compositor::Niri => niri_rl_pid(),
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => generic_rl_pid(c),
        Compositor::Other => None,
    }
    .or_else(|| {
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        sys.processes()
            .iter()
            .find(|(_, p)| {
                p.name()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains("rocketleague")
            })
            .map(|(pid, _)| pid.as_u32())
    });
    *cache = Some((Instant::now(), pid));
    pid
}

fn hyprland_active() -> bool {
    compositor() == Compositor::Hyprland
}

/// there's no real HWND concept on wayland; we use RL's pid as a stand-in
/// "handle" so callers that just need an opaque identity still work.
pub fn rocket_league_hwnd() -> Option<u32> {
    cached_rl_pid()
}

/// does the RL window have focus.
pub fn is_rocket_league_focused() -> bool {
    let Some(pid) = cached_rl_pid() else {
        return false;
    };
    match compositor() {
        Compositor::Hyprland => {
            let Some(json) = hyprctl_json("activewindow") else {
                return false;
            };
            json.get("pid").and_then(Value::as_i64) == Some(pid as i64)
        }
        Compositor::Kwin => {
            let Some(active_pid) = kdotool_line(&["getactivewindow", "getwindowpid"]) else {
                return false;
            };
            active_pid.parse::<u32>().map(|p| p == pid).unwrap_or(false)
        }
        Compositor::Niri => niri_focused_pid() == Some(pid as i64),
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => {
            match generic_windows(c) {
                Some(windows) if !windows.is_empty() => {
                    generic_rl_window(&windows, Some(pid)).is_some_and(|w| w.focused)
                }
                // the window manager can't be asked (no IPC, no EWMH): same
                // as Other, assume focused so overlays/binds don't go dead
                _ => {
                    warn_unsupported_compositor_once();
                    true
                }
            }
        }
        Compositor::Other => {
            warn_unsupported_compositor_once();
            // no window tracking available: assume focused so overlays/binds
            // still work rather than silently going dead.
            true
        }
    }
}

/// does the window owned by `pid` have focus right now. False when the
/// compositor can't say (unlike `is_rocket_league_focused`, which assumes
/// focus there), so callers gating global key reads on this stay inert
/// instead of listening to every keystroke on the desktop.
pub fn is_pid_focused(pid: u32) -> bool {
    match compositor() {
        Compositor::Hyprland => hyprctl_json("activewindow")
            .and_then(|json| json.get("pid").and_then(Value::as_i64))
            == Some(pid as i64),
        Compositor::Kwin => kdotool_line(&["getactivewindow", "getwindowpid"])
            .and_then(|active| active.parse::<u32>().ok())
            == Some(pid),
        Compositor::Niri => niri_focused_pid() == Some(pid as i64),
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => generic_windows(c)
            .is_some_and(|windows| windows.iter().any(|w| w.focused && w.pid == Some(pid))),
        Compositor::Other => false,
    }
}

/// (left, top, right, bottom) pixel rect of the RL window
pub fn get_rocket_league_window_rect() -> Option<(i32, i32, i32, i32)> {
    let pid = cached_rl_pid()?;
    match compositor() {
        Compositor::Hyprland => {
            let json = hyprctl_json("clients")?;
            let clients: Vec<HyprClient> = serde_json::from_value(json).ok()?;
            let client = clients.into_iter().find(|c| c.pid == pid as i64)?;
            let at = client.at?;
            let size = client.size?;
            let (left, top) = (at[0], at[1]);
            let (right, bottom) = (left + size[0], top + size[1]);
            if right > left && bottom > top {
                Some((left, top, right, bottom))
            } else {
                None
            }
        }
        Compositor::Kwin => kwin_rl_window_rect(),
        Compositor::Niri => niri_rl_window_rect(),
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => {
            let windows = generic_windows(c)?;
            let rect = generic_rl_window(&windows, Some(pid))?.rect;
            (rect.2 > rect.0 && rect.3 > rect.1).then_some(rect)
        }
        Compositor::Other => {
            // no geometry available: assume RL fills the monitor (it's
            // almost always fullscreen) rather than returning None, which
            // hides every drawn overlay
            warn_unsupported_compositor_once();
            let (w, h) = rocket_league_monitor_size();
            Some((0, 0, w, h))
        }
    }
}

/// Hyprland-only: exempt our own window from decoration effects some rice
/// setups apply broadly to floating/scratchpad-style windows (blur,
/// dim-around) -- these can render as a huge blurred/dimmed halo behind a
/// transparent egui surface, since the surface has no opaque content to
/// bound the effect against. Also makes the window start floating rather
/// than tiled, matching the reference Windows app's always-floating
/// behavior. Injected live over the IPC socket via `keyword`, the same
/// mechanism `hyprctl keyword ...` uses -- this is runtime compositor
/// state, NOT a write to hyprland.conf, and only matches our own window
/// class, so it can't touch any other app's tiling/scratchpad/dim
/// behavior. Lost on Hyprland reload/restart, which is fine since we
/// re-apply it every time we start.
///
/// No KWin equivalent: kdotool's builtin window-state toggles don't cover
/// blur/shadow/compositing-exemption, and Plasma's window rules for that
/// live in user config rather than anything scriptable live over D-Bus in
/// a way that's been verified here. No-op on KWin.
pub fn exempt_own_window_decorations() {
    if !hyprland_active() {
        return;
    }
    // Hyprland >= 0.53 renamed the old `windowrulev2`/`noblur,class:...`
    // syntax to `windowrule` + `match:class ...`, and boolean decoration
    // toggles now take an explicit value with an underscore
    // (`no_blur 1`, not `noblur`). Also: this must run before our window
    // maps -- these are static rules Hyprland only evaluates at map time,
    // not retroactively against an already-mapped window.
    for rule in [
        "no_blur 1",
        "no_shadow 1",
        "no_dim 1",
        "rounding 0",
        "float 1",
        // the active-window border isn't covered by any of the above --
        // it's a separate decoration with its own field. Left at its
        // default, hiding the window (ViewportCommand::Visible(false))
        // can leave a stray outline of it on screen until something else
        // forces a full repaint (confirmed live: focusing RL doesn't
        // clear it, but toggling RL's own fullscreen does).
        "border_size 0",
    ] {
        hyprctl_raw(&format!("keyword windowrule {rule}, match:class ^(Hebnix)$"));
    }
}

/// belt-and-suspenders alongside `exempt_own_window_decorations`: that one
/// is a static windowrule (matched only at window map time, before we can
/// be sure it actually took). This looks up our own window's live address
/// after it's mapped and re-applies the same overrides directly via
/// `dispatch setprop`, which takes effect immediately regardless of
/// map-time rule evaluation. Call once, shortly after the window is first
/// shown. Hyprland-only, see `exempt_own_window_decorations`.
pub fn reassert_own_window_decorations(own_pid: u32) {
    if let c @ (Compositor::Sway | Compositor::I3) = compositor() {
        // our window is only just mapping: keep trying until the window
        // manager knows it, then float it so it isn't tiled next to the game
        std::thread::spawn(move || {
            for _ in 0..40 {
                if i3_float_own(c, own_pid, false) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        return;
    }
    if !hyprland_active() {
        return;
    }
    let Some(json) = hyprctl_json("clients") else {
        return;
    };
    let Some(arr) = json.as_array() else { return };
    let Some(addr) = arr
        .iter()
        .find(|c| c.get("pid").and_then(Value::as_i64) == Some(own_pid as i64))
        .and_then(|c| c.get("address"))
        .and_then(Value::as_str)
    else {
        return;
    };
    // `setprop`'s property names are a different namespace from
    // `windowrule`'s (confirmed against hyprctl's own usage doc) -- this
    // was previously using the windowrule names (no_blur, no_shadow,
    // no_dim), which aren't valid setprop properties at all, so every call
    // here silently no-op'd. The real names are forcenoblur/forcenoshadow/
    // forcenodim/forcenoborder (booleans) and rounding/bordersize (ints).
    for (prop, val) in [
        ("forcenoblur", 1),
        ("forcenoshadow", 1),
        ("forcenodim", 1),
        ("forcenoborder", 1),
        ("rounding", 0),
    ] {
        hyprctl_raw(&format!("dispatch setprop address:{addr} {prop} {val}"));
    }
}

/// Move and resize our own window in the compositor's coordinates (the same ones
/// `get_rocket_league_window_rect` reports). Wayland clients cannot position
/// themselves, so the side panel needs the compositor to do it. `false` when this
/// compositor has no implementation, so the caller can fall back to asking the toolkit.
pub fn place_own_window(own_pid: u32, x: i32, y: i32, w: i32, h: i32) -> bool {
    match compositor() {
        Compositor::Hyprland => {
            let selector = format!("pid:{own_pid}");
            hyprctl_raw(&format!("dispatch setfloating {selector}"));
            hyprctl_raw(&format!("dispatch resizewindowpixel exact {w} {h},{selector}"));
            hyprctl_raw(&format!("dispatch movewindowpixel exact {x} {y},{selector}"));
            true
        }
        Compositor::Kwin => {
            let size = format!("{w}");
            let height = format!("{h}");
            let (px, py) = (format!("{x}"), format!("{y}"));
            kdotool_lines(&["search", "--name", "^Hebnix$", "windowsize", &size, &height, "windowmove", &px, &py])
                .is_some()
        }
        _ => false,
    }
}

/// bring our own window to the front, on top of a fullscreened RL, the way
/// BakkesMod's overlay pops up on Windows. No-op (returns false) on any
/// compositor we don't have a real implementation for.
pub fn focus_own_window_over_game(own_pid: u32) -> bool {
    match compositor() {
        Compositor::Hyprland => {
            let selector = format!("pid:{own_pid}");
            if let Some(ws) = cached_rl_pid().and_then(hyprland_rl_workspace) {
                hyprctl_raw(&format!("dispatch movetoworkspacesilent {ws},{selector}"));
            }
            hyprctl_raw(&format!("dispatch setfloating {selector}"));
            hyprctl_raw(&format!("dispatch focuswindow {selector}"));
            hyprctl_raw("dispatch bringactivetotop");
            true
        }
        Compositor::Kwin => {
            // `workspace.activeWindow = window` (what windowactivate does)
            // is documented to switch to whatever virtual desktop the
            // window lives on, same as real xdotool on X11 -- unverified
            // here, but no separate desktop-switch step should be needed.
            //
            // By title, not pid: confirmed live against a real KWin/kdotool
            // session that `search --pid <pid>` ignores the pid filter
            // entirely and returns *every* window in the session (Steam,
            // a file manager, Rocket League, whatever else happened to be
            // open) - chaining windowactivate/windowraise onto that raised
            // and activated all of them in sequence, not specifically our
            // own window, which is exactly the reported symptom (F2 doing
            // nothing useful while focused on the game). "Hebnix" is a
            // fixed, unique window title (see main.rs's with_title), so
            // matching by name sidesteps the broken filter entirely -
            // verified live to return exactly one match.
            kdotool_lines(&["search", "--name", "^Hebnix$", "windowactivate", "windowraise"])
                .is_some()
        }
        Compositor::Niri => {
            let windows = niri_windows();
            let Some(own) = windows.iter().find(|w| w.pid == Some(own_pid as i64)) else {
                return false;
            };
            let rl_workspace = cached_rl_pid().and_then(|pid| {
                windows
                    .iter()
                    .find(|w| w.pid == Some(pid as i64))
                    .and_then(|w| w.workspace_id)
            });
            if let Some(workspace) = rl_workspace.filter(|&ws| Some(ws) != own.workspace_id) {
                niri_action(serde_json::json!({ "MoveWindowToWorkspace": {
                    "window_id": own.id,
                    "reference": { "Id": workspace },
                    "focus": false,
                }}));
            }
            niri_action(serde_json::json!({ "MoveWindowToFloating": { "id": own.id } }));
            niri_action(serde_json::json!({ "FocusWindow": { "id": own.id } }))
        }
        // Sway/i3: float our window (it is tiled by default), move it to
        // RL's workspace and focus it. Sway shows a fullscreen window alone
        // on its workspace, so run RL borderless windowed rather than
        // fullscreen if the pop-over doesn't appear over it.
        c @ (Compositor::Sway | Compositor::I3) => i3_float_own(c, own_pid, true),
        Compositor::X11 => {
            let Some(own) = x11::own_window(own_pid, OWN_TITLE) else {
                return false;
            };
            let rl_desktop = generic_windows(Compositor::X11).and_then(|windows| {
                generic_rl_window(&windows, cached_rl_pid()).and_then(|w| w.desktop)
            });
            if let Some(desktop) = rl_desktop {
                x11::move_to_desktop(own, desktop);
            }
            x11::set_above(own, true);
            x11::activate(own)
        }
        Compositor::Other => {
            warn_unsupported_compositor_once();
            false
        }
    }
}

/// un-minimize and activate our own window in place, without the
/// workspace move `focus_own_window_over_game` does. A Wayland client can't
/// un-minimize itself (winit ignores it), and a minimized surface gets no
/// frame callbacks, so the app's own event loop stalls until something
/// outside it brings the window back - this is that something. Hyprland has
/// no minimize at all, so there's nothing to undo there.
pub fn unminimize_own_window() -> bool {
    match compositor() {
        Compositor::Kwin => {
            kdotool_lines(&["search", "--name", "^Hebnix$", "windowactivate", "windowraise"])
                .is_some()
        }
        Compositor::I3 | Compositor::X11 => match x11::own_window(std::process::id(), OWN_TITLE) {
            Some(own) if x11::is_hidden(own) => x11::activate(own),
            _ => false,
        },
        // no minimize on Hyprland, niri or Sway (i3 and Sway park windows on
        // the scratchpad instead, which Hebnix doesn't use)
        Compositor::Hyprland | Compositor::Niri | Compositor::Sway | Compositor::Other => false,
    }
}

/// pixel size of the monitor RL is on, falls back to the first monitor
/// reported by the compositor, or a 1920x1080 guess if nothing is
/// available.
pub fn rocket_league_monitor_size() -> (i32, i32) {
    match compositor() {
        Compositor::Hyprland => {
            if let Some(json) = hyprctl_json("monitors") {
                if let Some(arr) = json.as_array() {
                    // prefer the monitor RL's window is actually on
                    if let Some(rect) = get_rocket_league_window_rect() {
                        let (cx, cy) = ((rect.0 + rect.2) / 2, (rect.1 + rect.3) / 2);
                        for mon in arr {
                            let x = mon.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                            let y = mon.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                            let w = mon.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
                            let h = mon.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
                            if cx >= x && cx < x + w && cy >= y && cy < y + h {
                                return (w, h);
                            }
                        }
                    }
                    if let Some(mon) = arr.first() {
                        let w = mon.get("width").and_then(Value::as_i64).unwrap_or(1920) as i32;
                        let h = mon.get("height").and_then(Value::as_i64).unwrap_or(1080) as i32;
                        return (w, h);
                    }
                }
            }
        }
        Compositor::Kwin => {
            // schema verified against KDE::ConfigSerializer's own source:
            // {"outputs":[{"pos":{"x":,"y":},"size":{"width":,"height":},...}], ...}
            if let Some(json) = kscreen_doctor_json() {
                if let Some(arr) = json.get("outputs").and_then(Value::as_array) {
                    let get_wh = |o: &Value| -> Option<(i32, i32, i32, i32)> {
                        let pos = o.get("pos")?;
                        let size = o.get("size")?;
                        Some((
                            pos.get("x")?.as_i64()? as i32,
                            pos.get("y")?.as_i64()? as i32,
                            size.get("width")?.as_i64()? as i32,
                            size.get("height")?.as_i64()? as i32,
                        ))
                    };
                    if let Some(rect) = get_rocket_league_window_rect() {
                        let (cx, cy) = ((rect.0 + rect.2) / 2, (rect.1 + rect.3) / 2);
                        for out in arr {
                            if let Some((x, y, w, h)) = get_wh(out) {
                                if cx >= x && cx < x + w && cy >= y && cy < y + h {
                                    return (w, h);
                                }
                            }
                        }
                    }
                    for out in arr {
                        if out.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
                            if let Some((_, _, w, h)) = get_wh(out) {
                                return (w, h);
                            }
                        }
                    }
                }
            }
        }
        Compositor::Niri => {
            let output = niri_rl_window().and_then(|rl| niri_rl_output_rect(&rl));
            if let Some((_, _, w, h)) =
                output.or_else(|| niri_output_rects().first().map(|(_, rect)| *rect))
            {
                return (w, h);
            }
        }
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => {
            if let Some((l, t, r, b)) = rocket_league_monitor_rect_for(c) {
                return (r - l, b - t);
            }
        }
        Compositor::Other => warn_unsupported_compositor_once(),
    }
    (1920, 1080)
}

fn rocket_league_monitor_rect_for(compositor: Compositor) -> Option<(i32, i32, i32, i32)> {
    let rl_rect = generic_windows(compositor).and_then(|windows| {
        generic_rl_window(&windows, cached_rl_pid()).map(|w| w.rect)
    });
    output_for(&generic_outputs(compositor), rl_rect)
}

/// (left, top, right, bottom) of the monitor RL is on, in desktop
/// coordinates. Only known on the Sway, i3 and X11 backends; the Wayland
/// overlays (layer-shell) are placed by the compositor instead.
pub fn rocket_league_monitor_rect() -> Option<(i32, i32, i32, i32)> {
    match compositor() {
        c @ (Compositor::Sway | Compositor::I3 | Compositor::X11) => {
            rocket_league_monitor_rect_for(c)
        }
        _ => None,
    }
}

/// true when our own window's focus can be asked of the window manager
/// (Sway, i3, X11), which is when `is_pid_focused(own pid)` is trustworthy
pub fn can_track_own_window() -> bool {
    matches!(compositor(), Compositor::Sway | Compositor::I3 | Compositor::X11)
}

/// keep our own window above the game (Hyprland and KWin handle this
/// elsewhere). X11: `_NET_WM_STATE_ABOVE`. Sway/i3: floating windows are
/// drawn above tiled ones.
pub fn set_own_always_on_top(own_pid: u32, on_top: bool) -> bool {
    match compositor() {
        c @ (Compositor::Sway | Compositor::I3) => {
            on_top && i3ipc::run_command(&format!("{} floating enable", own_criteria(c, own_pid)))
        }
        Compositor::X11 => x11::own_window(own_pid, OWN_TITLE)
            .is_some_and(|window| x11::set_above(window, on_top)),
        _ => false,
    }
}

/// a second Hebnix was started: bring the running one forward
pub fn raise_other_instance(own_pid: u32) -> bool {
    match compositor() {
        c @ (Compositor::Sway | Compositor::I3) => {
            i3ipc::run_command(&format!("[title=\"^{OWN_TITLE}$\"] focus"))
                || i3ipc::run_command(&format!("{} focus", own_criteria(c, own_pid)))
        }
        Compositor::X11 => x11::windows()
            .and_then(|windows| {
                windows
                    .into_iter()
                    .find(|w| w.title == OWN_TITLE && w.pid != Some(own_pid))
            })
            .is_some_and(|w| x11::activate(w.id)),
        _ => false,
    }
}

/// is the cursor inside the RL window.
pub fn is_cursor_inside_rl_window() -> bool {
    let Some((left, top, right, bottom)) = get_rocket_league_window_rect() else {
        return false;
    };
    let Some((x, y)) = cursor_position() else {
        return false;
    };
    left <= x && x <= right && top <= y && y <= bottom
}

/// the cursor relative to the RL window's top-left, in the same units as
/// get_rocket_league_window_rect. None when either is unknown.
pub fn cursor_in_rl_window() -> Option<(i32, i32)> {
    let (left, top, _, _) = get_rocket_league_window_rect()?;
    let (x, y) = cursor_position()?;
    Some((x - left, y - top))
}

/// the cursor in the compositor's global coordinates. None where the
/// compositor doesn't say (niri, Sway).
pub fn cursor_position() -> Option<(i32, i32)> {
    let (x, y) = match compositor() {
        Compositor::Hyprland => {
            let json = hyprctl_json("cursorpos")?;
            (
                json.get("x").and_then(Value::as_i64)? as i32,
                json.get("y").and_then(Value::as_i64)? as i32,
            )
        }
        Compositor::Kwin => {
            // "X=123" / "Y=456" / "SCREEN=0" / "WINDOW=..." (--shell mode),
            // one per line, verified against kdotool's own template source.
            let lines = kdotool_lines(&["getmouselocation", "--shell"])?;
            let get = |prefix: &str| -> Option<i32> {
                lines
                    .iter()
                    .find_map(|l| l.strip_prefix(prefix))
                    .and_then(|v| v.parse().ok())
            };
            (get("X=")?, get("Y=")?)
        }
        Compositor::I3 | Compositor::X11 => x11::pointer()?,
        // no cursor position over niri's or Sway's IPC
        Compositor::Niri | Compositor::Sway | Compositor::Other => return None,
    };
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_with(vars: &[&str], desktop: &str) -> Compositor {
        detect(|name| vars.contains(&name), desktop)
    }

    #[test]
    fn picks_the_backend_from_the_session_environment() {
        use Compositor::*;
        assert!(detect_with(&["HYPRLAND_INSTANCE_SIGNATURE", "WAYLAND_DISPLAY"], "Hyprland") == Hyprland);
        assert!(detect_with(&["NIRI_SOCKET", "WAYLAND_DISPLAY"], "niri") == Niri);
        assert!(detect_with(&["SWAYSOCK", "WAYLAND_DISPLAY", "DISPLAY"], "sway") == Sway);
        assert!(detect_with(&["WAYLAND_DISPLAY"], "KDE") == Kwin);
        assert!(detect_with(&["DISPLAY"], "KDE") == X11);
        assert!(detect_with(&["DISPLAY", "I3SOCK"], "") == I3);
        assert!(detect_with(&["DISPLAY"], "i3") == I3);
        assert!(detect_with(&["DISPLAY"], "X-Cinnamon") == X11);
        assert!(detect_with(&["DISPLAY"], "XFCE") == X11);
        assert!(detect_with(&["WAYLAND_DISPLAY"], "GNOME") == Other);
        assert!(detect_with(&[], "") == Other);
    }

    fn win(pid: Option<u32>, class: &str, title: &str, focused: bool) -> GenWin {
        GenWin {
            pid,
            class: class.into(),
            title: title.into(),
            rect: (0, 0, 10, 10),
            focused,
            workspace: None,
            desktop: None,
        }
    }

    #[test]
    fn recognises_the_rocket_league_window() {
        let windows = vec![
            win(Some(1), "firefox", "Rocket League - YouTube", false),
            win(Some(2), "steam_app_252950", "Rocket League (64-bit, DX11, Cooked)", true),
        ];
        assert_eq!(generic_rl_window(&windows, None).and_then(|w| w.pid), Some(2));
        // the pid wins when it is known
        assert_eq!(generic_rl_window(&windows, Some(1)).and_then(|w| w.pid), Some(1));
        assert!(!is_rl_window(&windows[0]));
    }

    #[test]
    fn picks_the_output_the_game_is_on() {
        let outputs = [(0, 0, 1920, 1080), (1920, 0, 3840, 1080)];
        assert_eq!(output_for(&outputs, Some((2000, 100, 3000, 900))), Some(outputs[1]));
        assert_eq!(output_for(&outputs, None), Some(outputs[0]));
        assert_eq!(output_for(&[], None), None);
    }

    #[test]
    fn quotes_workspace_names_for_i3_commands() {
        assert_eq!(quote_workspace("1"), "\"1\"");
        assert_eq!(quote_workspace("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn parses_a_niri_window_with_and_without_layout() {
        let windows: Vec<NiriWindow> = serde_json::from_value(serde_json::json!([
            {
                "id": 12, "title": "Rocket League (64-bit, DX11, Cooked)", "app_id": "steam_app_252950",
                "pid": 4242, "workspace_id": 3, "is_focused": true, "is_floating": false,
                "is_urgent": false,
                "layout": {
                    "pos_in_scrolling_layout": [1, 1], "tile_size": [2560.0, 1440.0],
                    "window_size": [2560, 1440], "tile_pos_in_workspace_view": [0.0, 0.0],
                    "window_offset_in_tile": [0.0, 0.0]
                }
            },
            { "id": 13, "title": "Hebnix", "app_id": "Hebnix", "pid": 99, "workspace_id": 3,
              "is_focused": false, "is_floating": true }
        ]))
        .expect("niri Windows reply should parse");
        assert_eq!(windows[0].pid, Some(4242));
        assert_eq!(windows[0].layout.as_ref().and_then(|l| l.window_size), Some([2560, 1440]));
        assert!(windows[1].layout.is_none());
    }
}
