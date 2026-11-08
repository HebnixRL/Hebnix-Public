//! LAN chat commands. When you host a LAN match, the host can type
//!
//!     !map <map name>     change the map (for example `!map Stadium_P`)
//!     !kick <player name> kick that player
//!
//! in the text chat. The server runs TextChatActor_TA.ServerSendMessageToPlayers
//! for every message; the patch adds the command parser before it returns:
//!
//!     if (OwnerPC.IsLocalPlayerController()) {   // only the host's own messages
//!         if (Left(text, 5) ~= "!map ") ConsoleCommand("servertravel " $ Mid(text, 5, 255) $ <LAN options>);
//!         if (Left(text, 6) ~= "!kick ") foreach AllControllers(class'PlayerController_X', PC)
//!             if (PC.PlayerReplicationInfo.PlayerName ~= Mid(text, 6, 255)) PC.KickPlayerForReason(..);
//!     }
//!
//! The function is relocated, so Hebnix keeps its original payload in
//! Backups/LanCommands and puts it back to restore. Colours rebuilds TAGame.upk
//! from a pristine copy and calls `reapply`.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "TAGame.upk";
const FUNCTION: &str = "TextChatActor_TA.ServerSendMessageToPlayers";
/// Return, Nothing, EndOfScript: how every function here ends.
const END: [u8; 3] = [0x04, 0x0b, 0x4c];
/// A string only the patch adds to the function.
const MARKER: &str = "servertravel ";
/// What the game's own LAN host travels with, so a new map keeps the LAN match.
const OPTIONS: &str = "?game=TAGame.GameInfo_Soccar_TA?Lan?listen";
/// The reason the kicked player sees.
const KICK_REASON: &str = "TAGame.Lan.HostExited";

/// The objects the script refers to.
struct Refs {
    /// The function's message parameter, its MessageObject, that one's Payload, and the text inside.
    staged: i32,
    message_object: i32,
    payload: i32,
    text: i32,
    payload_struct: i32,
    /// TextChatActor_TA.OwnerPC: the controller of the player who sent the message.
    owner: i32,
    local_return: i32,
    local_name: i32,
    console_return: i32,
    console_name: i32,
    /// WorldInfo.GetWorldInfo().AllControllers(class, out PC) and the loop variable.
    world: i32,
    world_return: i32,
    get_world: i32,
    all_controllers: i32,
    controller_class: i32,
    pc: i32,
    pri: i32,
    player_name: i32,
    kick: i32,
}

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found"))
}

fn name(upk: &UpkPackage, text: &str) -> Result<i32, String> {
    upk.names.iter().position(|n| n == text).map(|i| i as i32).ok_or_else(|| format!("{text} is missing from {PACKAGE}"))
}

fn refs(upk: &UpkPackage) -> Result<Refs, String> {
    let o = |path: &str| object(upk, path);
    Ok(Refs {
        staged: o("TextChatActor_TA.ServerSendMessageToPlayers.StagedMessage")?,
        message_object: o("StagedMessageData.MessageObject")?,
        payload: o("ProjectX.SignedMessageObject_X.Payload")?,
        text: o("ProjectX._Types_X.MessagePayload.MessageText")?,
        payload_struct: o("ProjectX._Types_X.MessagePayload")?,
        owner: o("TextChatActor_TA.OwnerPC")?,
        local_return: o("Engine.Controller.IsLocalPlayerController.ReturnValue")?,
        local_name: name(upk, "IsLocalPlayerController")?,
        console_return: o("Engine.Actor.ConsoleCommand.ReturnValue")?,
        console_name: name(upk, "ConsoleCommand")?,
        world: o("Engine.WorldInfo")?,
        world_return: o("Engine.WorldInfo.GetWorldInfo.ReturnValue")?,
        get_world: o("Engine.WorldInfo.GetWorldInfo")?,
        all_controllers: o("Engine.WorldInfo.AllControllers")?,
        controller_class: o("ProjectX.PlayerController_X")?,
        pc: o("TextChatActor_TA.ServerSendMessageToPlayers.PC")?,
        pri: o("Engine.Controller.PlayerReplicationInfo")?,
        player_name: o("Engine.PlayerReplicationInfo.PlayerName")?,
        kick: o("ProjectX.PlayerController_X.KickPlayerForReason")?,
    })
}

fn literal(value: &str) -> Script {
    let mut out = Script::new();
    out.raw(&[0x1f]).raw(value.as_bytes()).raw(&[0x00]);
    out
}

/// `<object>.<member>` as the game's scripts read a member of another object:
/// Context, no fail flag, the object, skip size, return property, the member.
fn member_of(object: &Script, member: i32) -> Script {
    let mut out = Script::new();
    out.raw(&[0x19, 0x00]).append(object).raw(&[0x09, 0x00]).object(member).raw(&[0x00, 0x01]).object(member);
    out
}

fn local(variable: i32) -> Script {
    let mut out = Script::new();
    out.raw(&[0x2b]).object(variable);
    out
}

/// `StagedMessage.MessageObject.Payload.MessageText`, read like the relay in the function does.
fn message_text(r: &Refs) -> Script {
    let payload = member_of(&member_of(&local(r.staged), r.message_object), r.payload);
    let mut out = Script::new();
    out.raw(&[0x5e, 0x35]).object(r.text).object(r.payload_struct).raw(&[0x00, 0x00]).append(&payload);
    out
}

/// `int(byte n)`
fn byte_int(n: u8) -> Script {
    let mut out = Script::new();
    out.raw(&[0x38, 0x3a, 0x24, n]);
    out
}

/// `Left(text, n) ~= prefix`
fn starts_with(r: &Refs, n: u8, prefix: &str) -> Script {
    let mut out = Script::new();
    out.raw(&[0xbf, 0x80]).append(&message_text(r)).append(&byte_int(n)).raw(&[0x16]);
    out.append(&literal(prefix)).raw(&[0x16]);
    out
}

/// `Mid(text, n, 255)`
fn rest_of(r: &Refs, n: u8) -> Script {
    let mut out = Script::new();
    out.raw(&[0xd0]).append(&message_text(r)).append(&byte_int(n)).append(&byte_int(255)).raw(&[0x16]);
    out
}

/// `OwnerPC.IsLocalPlayerController()`: a virtual call by name, like the game's own.
fn sent_by_host(r: &Refs) -> Script {
    let mut call = Script::new();
    call.raw(&[0x1b]).name(r.local_name).raw(&[0x16]);
    let mut out = Script::new();
    out.raw(&[0x5e, 0x19, 0x00, 0x01]).object(r.owner).raw(&(call.mem_size() as u16).to_le_bytes());
    out.object(r.local_return).raw(&[0x00]).append(&call);
    out
}

/// `ConsoleCommand("servertravel " $ Mid(text, 5, 255) $ OPTIONS, false)`
fn travel(r: &Refs) -> Script {
    let mut out = Script::new();
    out.raw(&[0x0e]).object(r.console_return).raw(&[0x1b]).name(r.console_name);
    out.raw(&[0xf8, 0xf8]).append(&literal(MARKER)).append(&rest_of(r, 5)).raw(&[0x16]);
    out.append(&literal(OPTIONS)).raw(&[0x16, 0x28, 0x16]);
    out
}

/// The kick loop, placed at memory offset `at` of the function:
///
///     foreach WorldInfo.GetWorldInfo().AllControllers(class'PlayerController_X', PC)
///         if (PC.PlayerReplicationInfo.PlayerName ~= Mid(text, 6, 255)) PC.KickPlayerForReason(reason);
///
/// Its jump targets are absolute, so `at` matters; the size does not depend on it.
fn kick_loop(r: &Refs, at: usize) -> Script {
    let mut head = Script::new();
    head.raw(&[0x2f, 0x5e, 0x19, 0x00, 0x12, 0x00, 0x20]).object(r.world).raw(&[0x0a, 0x00]).object(r.world_return);
    head.raw(&[0x00, 0x1c]).object(r.get_world).raw(&[0x16]);
    let mut all = Script::new();
    all.raw(&[0x1c]).object(r.all_controllers).raw(&[0x20]).object(r.controller_class).append(&local(r.pc)).raw(&[0x16]);
    let mut player_name = Script::new();
    player_name.raw(&[0x5e, 0x19, 0x00]).append(&member_of(&local(r.pc), r.pri));
    player_name.raw(&[0x09, 0x00]).object(r.player_name).raw(&[0x00, 0x01]).object(r.player_name);
    let mut cond = Script::new();
    cond.raw(&[0xbf]).append(&player_name).append(&rest_of(r, 6)).raw(&[0x16]);
    let mut call = Script::new();
    call.raw(&[0x1c]).object(r.kick).append(&literal(KICK_REASON)).raw(&[0x48, 0x16]);
    let mut kick = Script::new();
    kick.raw(&[0x5e, 0x19, 0x00]).append(&local(r.pc)).raw(&(call.mem_size() as u16).to_le_bytes());
    kick.object(0).raw(&[0x00]).append(&call);

    // the loop body starts after the head, its skip size, return property, flag, and the iterator call
    let start = head.mem_size() + 2 + 8 + 1 + all.mem_size() + 2;
    let end = start + 3 + cond.mem_size() + kick.mem_size();
    let skip = all.mem_size() + 2 + (end - start) + 2;
    let mut out = Script::new();
    out.append(&head).raw(&(skip as u16).to_le_bytes()).object(0).raw(&[0x00]).append(&all);
    out.raw(&((at + end + 1) as u16).to_le_bytes());
    out.raw(&[0x07]).raw(&((at + end) as u16).to_le_bytes()).append(&cond).append(&kick);
    // IteratorNext, IteratorPop
    out.raw(&[0x31, 0x30]);
    out
}

/// The parser, for code that starts at memory offset `base` of the function.
fn commands(r: &Refs, base: usize) -> Script {
    let (host, map_check, kick_check) = (sent_by_host(r), starts_with(r, 5, "!map "), starts_with(r, 6, "!kick "));
    let travel = travel(r);
    let after_map = base + (3 + host.mem_size()) + (3 + map_check.mem_size()) + travel.mem_size();
    let loop_at = after_map + 3 + kick_check.mem_size();
    let looped = kick_loop(r, loop_at);
    let end = loop_at + looped.mem_size();
    let jump = |target: usize| {
        let mut out = Script::new();
        out.raw(&[0x07]).raw(&(target as u16).to_le_bytes());
        out
    };
    let mut out = Script::new();
    out.append(&jump(end)).append(&host);
    out.append(&jump(after_map)).append(&map_check).append(&travel);
    out.append(&jump(end)).append(&kick_check).append(&looped);
    out
}

fn function_index(upk: &UpkPackage) -> Result<usize, String> {
    object(upk, FUNCTION).and_then(|index| usize::try_from(index - 1).map_err(|_| format!("{FUNCTION} is not an export")))
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

fn is_patched(upk: &UpkPackage) -> Result<bool, String> {
    let index = function_index(upk)?;
    Ok(Script::of_function(&payload(upk, index))?.bytes().windows(MARKER.len()).any(|w| w == MARKER.as_bytes()))
}

/// Add the parser before the function returns.
pub fn apply(upk: &mut UpkPackage) -> Result<(), String> {
    if is_patched(upk)? {
        return Ok(());
    }
    let r = refs(upk)?;
    let mut script = Script::of_function(&payload(upk, function_index(upk)?))?;
    if !script.bytes().ends_with(&END) {
        return Err(format!("{FUNCTION} changed layout in this Rocket League version"));
    }
    script.split_off(END.len());
    let code = commands(&r, script.mem_size());
    script.append(&code).raw(&END);
    replace_script(upk, FUNCTION, &script)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    /// The function's export payload before the patch.
    original: Vec<u8>,
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("LanCommands").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub fn enabled(backups: &Path) -> bool {
    read_saved(backups).is_some()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the LAN commands.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the patch.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.lan.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).and_then(|saved| is_patched(&saved));
    if check != Ok(patched) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

/// Turn the commands on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk)? {
        return Ok(());
    }
    let saved = Saved { build: backup_guard::build_id(cooked)?, original: payload(&upk, function_index(&upk)?) };
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)?;
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Turn the commands off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk)? {
        if saved.build != backup_guard::build_id(cooked)? {
            return Err("The LAN commands backup belongs to another game build. Verify the game files.".into());
        }
        upk.replace_export_payload(function_index(&upk)?, &saved.original)?;
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("LanCommands"));
    Ok(())
}

/// Put the commands back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if is_patched(&upk)? {
        return Ok(());
    }
    ensure_closed()?;
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)
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

    /// Draw the toggle.
    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path) {
        use crate::i18n::t;
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
        let on = enabled(backups);
        let mut want = on;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-lan-commands")).on_hover_text(t("game-mod-lan-commands-description"));
        });
        ui.label(t("game-mod-lan-commands-description"));
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if want {
                    enable(&cooked, &backups).map(|_| t("game-mod-lan-commands-on"))
                } else {
                    restore(&cooked, &backups).map(|_| t("game-mod-lan-commands-off"))
                };
                let _ = tx.send(result);
            });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Refs {
        Refs {
            staged: 1, message_object: 2, payload: 3, text: 4, payload_struct: 5, owner: 6, local_return: 7, local_name: 8,
            console_return: 9, console_name: 10, world: 11, world_return: 12, get_world: 13, all_controllers: 14,
            controller_class: 15, pc: 16, pri: 17, player_name: 18, kick: 19,
        }
    }

    /// Every jump out of the parser lands exactly after it, wherever it is placed.
    #[test]
    fn commands_end_where_the_jumps_say() {
        for base in [0usize, 700] {
            let script = commands(&fake(), base);
            let end = (base + script.mem_size()) as u16;
            assert_eq!(&script.bytes()[..3], &[0x07, end as u8, (end >> 8) as u8]);
        }
    }

    #[test]
    fn kick_loop_size_does_not_depend_on_its_place() {
        assert_eq!(kick_loop(&fake(), 0).mem_size(), kick_loop(&fake(), 900).mem_size());
    }

    /// Patch a copy of the real package, reload it and look for the parser.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        apply(&mut upk).unwrap();
        let out = std::env::temp_dir().join("hebnix_lan_test.upk");
        upk.save_fast(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(is_patched(&reloaded).unwrap());
    }
}
