//! Celebration. After a match the game moves GameEvent_Soccar_TA into its
//! PrePodiumSpotlight state, which lines the cars up on the podium. Freeplay is
//! GameInfo_Soccar_TA with a Freeplay tag, so it has the same state, and a
//! command that runs
//!
//!     GameEvent_Soccar_TA(GetGameEvent()).GotoGameState('PrePodiumSpotlight');
//!
//! starts the podium on demand. The command does nothing outside freeplay, and
//! a second press ends the podium with the game's own freeplay reset. It is a
//! block of the dispatcher (dispatcher.rs) with a key action of its own
//! (actions.rs).
use super::forfeit_command::{KEYS, set_binding};
use super::script_patch::Script;
use super::upk_package::UpkPackage;
use super::{actions, backup_guard, dispatcher, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const DEFAULT_KEY: &str = "K";
/// The key action slot (see actions.rs), also the dispatcher block id.
const SLOT: u32 = 2;
/// Freeplay is GameInfo_Soccar_TA with a Freeplay tag, so its event is a plain soccar event.
const EVENT_CLASS: &str = "GameEvent_Soccar_TA";
/// SoccarGameEvent is never set in freeplay, so the event comes from the PRI.
const GET_EVENT: &str = "PlayerController_TA.GetGameEvent";
const GOTO: &str = "GameEvent_TA.GotoGameState";
const STATE: &str = "PrePodiumSpotlight";
/// The next state, entered at once so both state changes finish before the HUD name is set.
const PODIUM_STATE: &str = "PodiumSpotlight";
const LOG_MARKER: &str = "HBXCELEBRATE";
/// True in freeplay, offline or online.
const IS_FREEPLAY: &str = "GameEvent_TA.IsPlayingFreeplay";
/// The event's freeplay reset. The key's own command asserts on the ball, which the podium has removed.
const RESET: &str = "GameEvent_TA.PlayerResetTraining";

const MATCH_WINNER: &str = "GameEvent_Soccar_TA.MatchWinner";
const PODIUM_TIME: &str = "GameEvent_Soccar_TA.PodiumTime";
const ALLOW_EXPLOSION: &str = "GameEvent_Soccar_TA.bAllowWinnersExplosion";
const PLAYER_TEAM: &str = "Engine.PlayerReplicationInfo.Team";
const PLAYER_PRI: &str = "PlayerController_TA.PRI";
const HUD_CLASS: &str = "GFxHUD_TA";
const MY_HUD: &str = "Engine.PlayerController.myHUD";
const EVENT_DATA: &str = "GFxHUD_TA.GameEventData";
const STATE_BINDING: &str = "GFxData_GameEvent_TA.GameState";
/// The state name the menu code sees afterwards, so it does not open the end menu.
const SHOWN_STATE: &str = "Active";
/// How long the podium stays up, in seconds. When the time runs out the game
/// ends the match and the end of match menu opens, so this is long.
const PODIUM_SECONDS: f32 = 3600.0;

/// Object references the body needs.
struct Refs {
    class: i32,
    get_event: i32,
    goto: i32,
    is_freeplay: i32,
    reset: i32,
    state: i32,
    podium_state: i32,
    match_winner: i32,
    podium_time: i32,
    allow_explosion: i32,
    team: i32,
    pri: i32,
    hud_class: i32,
    my_hud: i32,
    event_data: i32,
    state_binding: i32,
    shown_state: i32,
}

/// `Context(object).call`, in the game's shape: Context, no fail flag, the
/// object expression, skip size of the call, return property, size, the call.
fn context(object: &Script, retprop: i32, call: &Script) -> Script {
    let mut out = Script::new();
    out.raw(&[0x5e, 0x19, 0x00]).append(object).raw(&(call.mem_size() as u16).to_le_bytes());
    out.object(retprop).raw(&[0x00]).append(call);
    out
}

/// `object.member`, as the game writes it in `PC.ConfiguredClientInputRate`; a bool
/// member is wrapped in a BoolVariable (2d), as in `GFxData.bPendingIdleKick`.
fn member(object: &Script, property: i32, boolean: bool) -> Script {
    let mut access = Script::new();
    if boolean {
        access.raw(&[0x2d]);
    }
    access.raw(&[0x01]).object(property);
    context(object, property, &access)
}

/// `GameEvent_Soccar_TA(GetGameEvent())`.
fn event(refs: &Refs) -> Script {
    let mut out = Script::new();
    out.raw(&[0x2e]).object(refs.class).raw(&[0x1c]).object(refs.get_event).raw(&[0x16]);
    out
}

/// A jump opcode with its target word to be filled in by `land`; the word's
/// position goes in `jumps` for the dispatcher to shift.
fn jump(out: &mut Script, op: u8, jumps: &mut Vec<usize>) -> usize {
    out.raw(&[op]);
    let at = out.len();
    out.raw(&[0, 0]);
    jumps.push(at);
    at
}

/// Point the jump at `at` to the end of what is written so far (memory form,
/// counted from the start of the body).
fn land(out: &mut Script, at: usize) {
    let target = out.mem_size() as u16;
    out.set_word(at, target);
}

/// A call of a final function with no parameters.
fn call_of(function: i32) -> Script {
    let mut call = Script::new();
    call.raw(&[0x1c]).object(function).raw(&[0x16]);
    call
}

/// What the command runs, with LogInternal(marker) first to show in the log.
/// GameEvent is `GameEvent_Soccar_TA(GetGameEvent())`.
///
///     if (GameEvent.IsPlayingFreeplay())
///     {
///         if (GameEvent.bAllowWinnersExplosion)   // nothing running yet: start
///         {
///             GameEvent.MatchWinner = PRI.Team;   // the podium lines up the winners' cars
///             GameEvent.PodiumTime = 3600.0;      // and ends the match when its time is up
///             GameEvent.bAllowWinnersExplosion = false; // the explosion podium needs a goal; also the flag
///             GameEvent.GotoGameState('PrePodiumSpotlight');
///             GameEvent.GotoGameState('PodiumSpotlight');
///             GFxHUD_TA(myHUD).GameEventData.GameState = 'Active'; // the menu code reads this
///         }
///         else                                    // running: end it
///         {
///             GameEvent.bAllowWinnersExplosion = true;
///             GameEvent.PlayerResetTraining();
///         }
///     }
///
/// Returns the body and the positions of its jump targets.
fn body(refs: &Refs) -> (Script, Vec<usize>) {
    let mut jumps = Vec::new();
    let mut out = Script::new();
    out.raw(&[0xe7, 0x1f]).raw(LOG_MARKER.as_bytes()).raw(&[0x00, 0x48, 0x48, 0x16]);
    let outer = jump(&mut out, 0x07, &mut jumps);
    out.append(&context(&event(refs), 0, &call_of(refs.is_freeplay)));
    let inner = jump(&mut out, 0x07, &mut jumps);
    out.append(&member(&event(refs), refs.allow_explosion, true));
    let mut own_pri = Script::new();
    own_pri.raw(&[0x01]).object(refs.pri);
    out.raw(&[0x0f]).append(&member(&event(refs), refs.match_winner, false)).append(&member(&own_pri, refs.team, false));
    out.raw(&[0x0f]).append(&member(&event(refs), refs.podium_time, false)).raw(&[0x1e]).raw(&PODIUM_SECONDS.to_le_bytes());
    out.raw(&[0x14]).append(&member(&event(refs), refs.allow_explosion, true)).raw(&[0x28]);
    for state in [refs.state, refs.podium_state] {
        let mut call = Script::new();
        call.raw(&[0x1c]).object(refs.goto).raw(&[0x21]).name(state).raw(&[0x16]);
        out.append(&context(&event(refs), 0, &call));
    }
    let mut hud = Script::new();
    hud.raw(&[0x2e]).object(refs.hud_class).raw(&[0x01]).object(refs.my_hud);
    let data = member(&hud, refs.event_data, false);
    out.raw(&[0x0f]).append(&member(&data, refs.state_binding, false)).raw(&[0x21]).name(refs.shown_state);
    let skip = jump(&mut out, 0x06, &mut jumps);
    land(&mut out, inner);
    out.raw(&[0x14]).append(&member(&event(refs), refs.allow_explosion, true)).raw(&[0x27]);
    out.append(&context(&event(refs), 0, &call_of(refs.reset)));
    land(&mut out, skip);
    land(&mut out, outer);
    (out, jumps)
}

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found"))
}

fn name(upk: &UpkPackage, text: &str) -> Result<i32, String> {
    upk.names.iter().position(|n| n == text).map(|i| i as i32).ok_or_else(|| format!("{text} is missing from {PACKAGE}"))
}

fn is_patched(upk: &UpkPackage) -> bool {
    dispatcher::has(upk, SLOT as u8) && actions::has(upk, SLOT)
}

/// Add the dispatcher block and the key action that runs it.
pub fn apply(upk: &mut UpkPackage, backups: &Path, build: &str) -> Result<(), String> {
    let state = name(upk, STATE)?;
    let refs = Refs {
        class: object(upk, EVENT_CLASS)?,
        get_event: object(upk, GET_EVENT)?,
        goto: object(upk, GOTO)?,
        is_freeplay: object(upk, IS_FREEPLAY)?,
        reset: object(upk, RESET)?,
        state,
        podium_state: name(upk, PODIUM_STATE)?,
        match_winner: object(upk, MATCH_WINNER)?,
        podium_time: object(upk, PODIUM_TIME)?,
        allow_explosion: object(upk, ALLOW_EXPLOSION)?,
        team: object(upk, PLAYER_TEAM)?,
        pri: object(upk, PLAYER_PRI)?,
        hud_class: object(upk, HUD_CLASS)?,
        my_hud: object(upk, MY_HUD)?,
        event_data: object(upk, EVENT_DATA)?,
        state_binding: object(upk, STATE_BINDING)?,
        shown_state: name(upk, SHOWN_STATE)?,
    };
    let (script, jumps) = body(&refs);
    dispatcher::install(upk, backups, build, SLOT as u8, &script, &jumps)?;
    actions::remove(upk, SLOT)?;
    actions::add(upk, SLOT, &format!("{} {SLOT}", dispatcher::COMMAND))
}

/// The Action name TAInput.ini binds the key to.
fn action() -> String {
    actions::action_name(SLOT)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("Celebration").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub fn enabled(backups: &Path) -> bool {
    read_saved(backups).is_some()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the celebration command.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the command.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.celebration.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).map(|saved| is_patched(&saved));
    if check != Ok(patched) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        return Ok(());
    }
    let build = backup_guard::build_id(cooked)?;
    apply(&mut upk, backups, &build)?;
    install(cooked, &live, &upk, true)?;
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&Saved { build }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if actions::has(&upk, SLOT) || dispatcher::has(&upk, SLOT as u8) {
        let build = backup_guard::build_id(cooked)?;
        if saved.build != build {
            return Err("The celebration backup belongs to another game build. Verify the game files.".into());
        }
        actions::remove(&mut upk, SLOT)?;
        dispatcher::remove(&mut upk, backups, &build, SLOT as u8)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("Celebration"));
    Ok(())
}

/// Put the command back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk) {
        return Ok(());
    }
    ensure_closed()?;
    apply(&mut upk, backups, &backup_guard::build_id(cooked)?)?;
    install(cooked, &live, &upk, true)
}

/// Unbind the key, for uninstall.
pub fn unbind() -> Result<(), String> {
    set_binding(&action(), &[], None)
}

/// Page state: the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Draw the toggle and key picker. Returns true when `key` changed.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path, key: &mut String) -> bool {
        use crate::i18n::t;
        use eframe::egui;
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.status = result.unwrap_or_else(|e| e);
                    self.job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            }
        }
        if key.is_empty() {
            *key = DEFAULT_KEY.into();
        }
        let on = enabled(backups);
        let mut want = on;
        let mut key_changed = false;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-celebration")).on_hover_text(t("game-mod-celebration-description"));
            ui.label(t("game-mod-celebration-description"));
            ui.horizontal(|ui| {
                ui.label(t("game-mod-celebration-key"));
                egui::ComboBox::from_id_salt("celebration_key").selected_text(key.as_str()).show_ui(ui, |ui| {
                    for option in KEYS {
                        key_changed |= ui.selectable_value(key, option.to_string(), option).changed();
                    }
                });
            });
        });
        if want != on || (key_changed && on) {
            let (cooked, backups, key) = (cooked.to_path_buf(), backups.to_path_buf(), key.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if !want {
                    restore(&cooked, &backups).and_then(|_| unbind()).map(|_| t("game-mod-celebration-off"))
                } else {
                    enable(&cooked, &backups).and_then(|_| set_binding(&action(), &[], Some(&key))).map(|_| t("game-mod-celebration-on"))
                };
                let _ = tx.send(result);
            });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
        key_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> Refs {
        Refs { class: 100, get_event: 200, goto: 300, state: 7, podium_state: 8, is_freeplay: 1300, reset: 1400, match_winner: 400, podium_time: 500, allow_explosion: 800, team: 600, pri: 700, hud_class: 900, my_hud: 1000, event_data: 1100, state_binding: 1200, shown_state: 9 }
    }

    /// Each reference appears, the memory size counts them at 8 bytes, the
    /// body ends with the reset call, and the three jumps land where the
    /// `if` and `else` end (the outer one at the very end of the body).
    #[test]
    fn body_has_every_reference_and_its_jumps_land_inside() {
        let (script, jumps) = body(&refs());
        for reference in [100i32, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400] {
            assert!(script.bytes().windows(4).any(|w| w == reference.to_le_bytes()), "{reference}");
        }
        assert_eq!(script.mem_size(), script.len() + 4 * script.refs());
        assert!(script.bytes().starts_with(&[0xe7, 0x1f]));
        let mut tail = vec![0x1c];
        tail.extend_from_slice(&1400i32.to_le_bytes());
        tail.push(0x16);
        assert!(script.bytes().ends_with(&tail));
        assert_eq!(jumps.len(), 3);
        let target = |at: usize| u16::from_le_bytes([script.bytes()[at], script.bytes()[at + 1]]) as usize;
        assert_eq!(target(jumps[0]), script.mem_size());
        assert_eq!(target(jumps[2]), script.mem_size());
        assert!(target(jumps[1]) < script.mem_size());
        // the else part (a LetBool and the reset call) is what follows the inner target
        assert_eq!(script.mem_size() - target(jumps[1]), 1 + member(&event(&refs()), 800, true).mem_size() + 1 + context(&event(&refs()), 0, &call_of(1400)).mem_size());
    }

    /// `object.member` is the game's own shape: skip = size of `01 <property>`
    /// in memory (9), then the property as return property, a 0 byte, `01 <property>`.
    #[test]
    fn member_matches_the_games_own_assignment_target() {
        let mut object = Script::new();
        object.raw(&[0x2b]).object(0x71dc);
        let script = member(&object, 0xd95e, false);
        let mut expected = vec![0x5e, 0x19, 0x00, 0x2b];
        expected.extend_from_slice(&0x71dci32.to_le_bytes());
        expected.extend_from_slice(&[0x09, 0x00]);
        expected.extend_from_slice(&0xd95ei32.to_le_bytes());
        expected.extend_from_slice(&[0x00, 0x01]);
        expected.extend_from_slice(&0xd95ei32.to_le_bytes());
        assert_eq!(script.bytes(), &expected[..]);
    }

    /// IdleKickSystem_TA.HideIdleWarning, `GFxData.bPendingIdleKick = false`:
    /// 14 <member> 28 with the member 5e 19 00 2b <local> 0a 00 <prop> 00 2d 01 <prop>.
    #[test]
    fn bool_member_matches_the_games_own_assignment_target() {
        let mut object = Script::new();
        object.raw(&[0x2b]).object(0xb53b);
        let script = member(&object, 0x85de, true);
        let expected = [0x5e, 0x19, 0x00, 0x2b, 0x3b, 0xb5, 0, 0, 0x0a, 0x00, 0xde, 0x85, 0, 0, 0x00, 0x2d, 0x01, 0xde, 0x85, 0, 0];
        assert_eq!(script.bytes(), &expected[..]);
    }


    /// Enable both commands on a temp copy of the real package, turn one off
    /// and then the other, and check the package ends exactly as it began.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a vanilla TAGame.upk.
    /// The command applies to a vanilla package (every name resolves), survives
    /// a save and load, and its jumps stay valid next to a second block.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a vanilla TAGame.upk.
    #[test]
    #[ignore]
    fn applies_next_to_another_block_and_removes() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let backups = std::env::temp_dir().join("hebnix_celebration_apply_test");
        let _ = std::fs::remove_dir_all(&backups);
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        let mut other = Script::new();
        other.raw(&[0x0b, 0x0b]);
        dispatcher::install(&mut upk, &backups, "b", 1, &other, &[]).unwrap();
        apply(&mut upk, &backups, "b").unwrap();
        dispatcher::install(&mut upk, &backups, "b", 3, &other, &[]).unwrap();
        let out = std::env::temp_dir().join("hebnix_celebration_apply_test.upk");
        upk.save_fast(&out).unwrap();
        let mut reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(is_patched(&reloaded));
        dispatcher::remove(&mut reloaded, &backups, "b", 1).unwrap();
        dispatcher::remove(&mut reloaded, &backups, "b", 3).unwrap();
        assert!(is_patched(&reloaded));
        dispatcher::remove(&mut reloaded, &backups, "b", SLOT as u8).unwrap();
        assert!(!dispatcher::has(&reloaded, SLOT as u8));
    }

    #[test]
    #[ignore]
    fn both_commands_enable_and_restore() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let root = std::env::temp_dir().join("hebnix_celebration_install");
        let _ = std::fs::remove_dir_all(&root);
        let cooked = root.join("TAGame").join("CookedPCConsole");
        let backups = cooked.join("Backups");
        std::fs::create_dir_all(&cooked).unwrap();
        std::fs::create_dir_all(root.join("TAGame/Binaries/Win64")).unwrap();
        std::fs::write(root.join("TAGame/Binaries/Win64/RocketLeague.exe"), b"exe").unwrap();
        std::fs::copy(&source, cooked.join("TAGame.upk")).unwrap();
        let load = || UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        let paths = ["PlayerController_TA.NetClientInputRate", "PlayerInput_TA.GetDefaultActions"];
        let snapshot = |upk: &UpkPackage| -> Vec<Vec<u8>> {
            paths
                .iter()
                .map(|p| {
                    let e = &upk.exports[object(upk, p).unwrap() as usize - 1];
                    upk.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec()
                })
                .collect()
        };
        let vanilla = snapshot(&load());
        super::super::forfeit_command::enable(&cooked, &backups).unwrap();
        enable(&cooked, &backups).unwrap();
        assert!(is_patched(&load()) && enabled(&backups));
        restore(&cooked, &backups).unwrap();
        assert!(!is_patched(&load()));
        assert!(dispatcher::has(&load(), 1) && actions::has(&load(), 1));
        super::super::forfeit_command::restore(&cooked, &backups).unwrap();
        assert_eq!(snapshot(&load()), vanilla);
        assert!(!backups.join("Dispatcher").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
