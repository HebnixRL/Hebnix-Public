//! Chat export. Quick chats and the join/leave notices reach the screen through
//! HUDBase_TA.AddChatMessage(GFxChatMessage GFxMessage); typed text chat goes
//! through GFxData_TextModerationManager_TA.HandleMessageReceived, which builds
//! a GFxChatMessage local `NewMessage`. The patch adds
//!
//!     LogInternal("HBXCHAT" $ Tab $ Message.PlayerName $ Tab $ Message.Message);
//!
//! before each of those functions returns, which the game writes to Launch.log
//! as a `ScriptLog:` line. `watch` follows the log and appends each message to
//! chat.jsonl in Hebnix's folder, one JSON object per line, for plugins to read.
//! The functions are relocated, so Hebnix keeps their original payloads in
//! Backups/ChatExport and puts them back to restore. Colours rebuilds TAGame.upk
//! from a pristine copy and calls `reapply`.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PACKAGE: &str = "TAGame.upk";
/// A function that shows a chat message, and where it keeps the GFxChatMessage.
struct Target {
    function: &'static str,
    /// The parameter or local holding the message.
    message: &'static str,
    /// How the script reads it: by-reference parameter or local variable.
    read: u8,
}

const TARGETS: [Target; 2] = [
    Target { function: "HUDBase_TA.AddChatMessage", message: "HUDBase_TA.AddChatMessage.GFxMessage", read: PARAMETER_READ },
    Target {
        function: "GFxData_TextModerationManager_TA.HandleMessageReceived",
        message: "GFxData_TextModerationManager_TA.HandleMessageReceived.NewMessage",
        read: LOCAL_READ,
    },
];
const MESSAGE_STRUCT: &str = "GFxData_Chat_TA.GFxChatMessage";
const PLAYER_NAME: &str = "GFxData_Chat_TA.GFxChatMessage.PlayerName";
const MESSAGE: &str = "GFxData_Chat_TA.GFxChatMessage.Message";
/// Return, Nothing, EndOfScript: how every function here ends.
const END: [u8; 3] = [0x04, 0x0b, 0x4c];
/// The line's marker in Launch.log. Hebnix reads what follows the tabs.
const MARKER: &str = "HBXCHAT\t";

/// Opcodes as the game's own scripts use them: LogInternal (0xe7) and string
/// concatenation (0xf8) calls, a string constant, a struct member read.
const LOG: u8 = 0xe7;
const CONCAT: u8 = 0xf8;
const STRING: u8 = 0x1f;
const MEMBER: u8 = 0x35;
/// A by-reference parameter, how AddChatMessage reads GFxMessage.
const PARAMETER_READ: u8 = 0x46;
/// A local variable, how HandleMessageReceived reads NewMessage.
const LOCAL_READ: u8 = 0x2b;
const END_PARMS: u8 = 0x16;
/// The two optional LogInternal parameters, left empty.
const EMPTY: u8 = 0x48;

fn text(script: &mut Script, value: &str) {
    script.raw(&[STRING]).raw(value.as_bytes()).raw(&[0x00]);
}

/// `<message>.<field>` in the shape of the game's own reads of the struct.
fn member(script: &mut Script, field: i32, structure: i32, read: u8, message: i32) {
    script.raw(&[MEMBER]).object(field).object(structure).raw(&[0x00, 0x00, read]).object(message);
}

/// The log call, built like GameEvent_TA.InitGame's
/// `LogInternal("MatchTypeClass=(" $ x $ ")")`.
fn statement(read: u8, parameter: i32, structure: i32, name: i32, message: i32) -> Script {
    let mut out = Script::new();
    out.raw(&[LOG, CONCAT, CONCAT, CONCAT]);
    text(&mut out, MARKER);
    member(&mut out, name, structure, read, parameter);
    out.raw(&[END_PARMS]);
    text(&mut out, "\t");
    out.raw(&[END_PARMS]);
    member(&mut out, message, structure, read, parameter);
    out.raw(&[END_PARMS, EMPTY, EMPTY, END_PARMS]);
    out
}

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found"))
}

fn function_index(upk: &UpkPackage, target: &Target) -> Result<usize, String> {
    object(upk, target.function).and_then(|index| {
        usize::try_from(index - 1).map_err(|_| format!("{} is not an export", target.function))
    })
}

fn payload(upk: &UpkPackage, index: usize) -> Vec<u8> {
    let export = &upk.exports[index];
    upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec()
}

fn is_patched(upk: &UpkPackage, target: &Target) -> Result<bool, String> {
    let index = function_index(upk, target)?;
    Ok(Script::of_function(&payload(upk, index))?
        .bytes()
        .windows(MARKER.len())
        .any(|w| w == MARKER.as_bytes()))
}

/// Add the log call before the function returns.
fn apply_target(upk: &mut UpkPackage, target: &Target) -> Result<(), String> {
    let index = function_index(upk, target)?;
    let call = statement(
        target.read,
        object(upk, target.message)?,
        object(upk, MESSAGE_STRUCT)?,
        object(upk, PLAYER_NAME)?,
        object(upk, MESSAGE)?,
    );
    let mut script = Script::of_function(&payload(upk, index))?;
    if !script.bytes().ends_with(&END) {
        return Err(format!("{} changed layout in this Rocket League version", target.function));
    }
    script.split_off(END.len());
    script.append(&call).raw(&END);
    replace_script(upk, target.function, &script)
}

/// Patch every target that is not patched yet.
pub fn apply(upk: &mut UpkPackage) -> Result<(), String> {
    for target in &TARGETS {
        if !is_patched(upk, target)? {
            apply_target(upk, target)?;
        }
    }
    Ok(())
}

/// Whether each target is patched.
fn states(upk: &UpkPackage) -> Result<Vec<bool>, String> {
    TARGETS.iter().map(|target| is_patched(upk, target)).collect()
}

#[derive(Serialize, Deserialize)]
struct Saved {
    build: String,
    /// AddChatMessage's export payload before the patch (the first version
    /// of this feature saved only that one).
    #[serde(default)]
    original: Vec<u8>,
    /// Export payload of each patched function before the patch, by function.
    #[serde(default)]
    originals: Vec<(String, Vec<u8>)>,
}

impl Saved {
    fn original_of(&self, function: &str) -> Option<&[u8]> {
        self.originals
            .iter()
            .find(|(name, _)| name == function)
            .map(|(_, bytes)| bytes.as_slice())
            .or_else(|| (function == TARGETS[0].function && !self.original.is_empty()).then_some(self.original.as_slice()))
    }
}

fn saved_path(backups: &Path) -> PathBuf {
    backups.join("ChatExport").join("state.json")
}

fn read_saved(backups: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(saved_path(backups)).ok()?).ok()
}

pub fn enabled(backups: &Path) -> bool {
    read_saved(backups).is_some()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the chat export.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy loads and
/// has (or lacks) the patch.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.chat.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp)
        .and_then(|saved| states(&saved))
        .map(|states| states.iter().all(|state| *state == patched));
    if check != Ok(true) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

/// Turn the export on.
pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    backup_guard::check_install(cooked, backups)?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if states(&upk)?.iter().all(|state| *state) {
        return Ok(());
    }
    let build = backup_guard::build_id(cooked)?;
    // an install from before text chat was exported has AddChatMessage saved already
    let mut saved = read_saved(backups).filter(|saved| saved.build == build).unwrap_or(Saved {
        build,
        original: Vec::new(),
        originals: Vec::new(),
    });
    for target in &TARGETS {
        if !is_patched(&upk, target)? {
            let original = payload(&upk, function_index(&upk, target)?);
            saved.originals.retain(|(name, _)| name != target.function);
            saved.originals.push((target.function.to_string(), original));
        }
    }
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)?;
    let path = saved_path(backups);
    std::fs::create_dir_all(path.parent().unwrap_or(backups)).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec(&saved).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Turn the export off.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let Some(saved) = read_saved(backups) else {
        return Ok(());
    };
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if states(&upk)?.iter().any(|state| *state) {
        if saved.build != backup_guard::build_id(cooked)? {
            return Err("The chat export backup belongs to another game build. Verify the game files.".into());
        }
        for target in &TARGETS {
            if is_patched(&upk, target)? {
                let original = saved
                    .original_of(target.function)
                    .ok_or_else(|| format!("No saved copy of {}. Verify the game files.", target.function))?;
                upk.replace_export_payload(function_index(&upk, target)?, original)?;
            }
        }
        install(cooked, &live, &upk, false)?;
    }
    let _ = std::fs::remove_dir_all(backups.join("ChatExport"));
    Ok(())
}

/// Put the export back after another feature rewrote TAGame.upk from a backup.
pub fn reapply(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !enabled(backups) {
        return Ok(());
    }
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if states(&upk)?.iter().all(|state| *state) {
        return Ok(());
    }
    ensure_closed()?;
    apply(&mut upk)?;
    install(cooked, &live, &upk, true)
}

/// Where the exported messages go: one JSON object per line with `time` (Unix
/// seconds when Hebnix read it), `player` and `message`.
pub fn output_path() -> PathBuf {
    crate::config::base_dir().join("chat").join("chat.jsonl")
}

/// The chat in a Launch.log chunk as JSON lines. Names and messages come from
/// other players, so only lines the game itself logged as ScriptLog count.
fn json_lines(chunk: &str, time: u64) -> Vec<String> {
    chunk
        .lines()
        .filter_map(|line| line.split_once("] ScriptLog: ").map(|(_, rest)| rest))
        .filter_map(|rest| rest.strip_prefix(MARKER))
        .filter_map(|rest| rest.split_once('\t'))
        .map(|(player, message)| serde_json::json!({ "time": time, "player": player, "message": message }).to_string())
        .collect()
}

/// Follow Launch.log and append the chat to `output_path`. Safe to call more
/// than once; a thread starts the first time.
pub fn watch() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let log = dirs::document_dir().map(|d| d.join("My Games/Rocket League/TAGame/Logs/Launch.log"));
            // chat from an earlier game session was already exported
            let mut seen = log.as_ref().and_then(|l| std::fs::metadata(l).ok()).map_or(0, |m| m.len());
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(log) = &log else { continue };
                let Ok(len) = std::fs::metadata(log).map(|m| m.len()) else { continue };
                // a new game launch starts a new log
                if len < seen {
                    seen = 0;
                }
                if len == seen {
                    continue;
                }
                let Ok(mut file) = std::fs::File::open(log) else { continue };
                if file.seek(SeekFrom::Start(seen)).is_err() {
                    continue;
                }
                let mut bytes = Vec::new();
                if file.read_to_end(&mut bytes).is_err() {
                    continue;
                }
                // a line the game is still writing waits for the next pass
                let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { continue };
                seen += end as u64 + 1;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let lines = json_lines(&String::from_utf8_lossy(&bytes[..end]), now);
                if lines.is_empty() {
                    continue;
                }
                let path = output_path();
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
                if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(out, "{}", lines.join("\n"));
                }
            }
        });
    });
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

    /// Draw the toggle and where the chat goes.
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
            ui.checkbox(&mut want, t("game-mod-chat-export")).on_hover_text(t("game-mod-chat-export-description"));
        });
        ui.separator();
        ui.label(t("game-mod-chat-export-description"));
        ui.small(output_path().display().to_string());
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if want {
                    enable(&cooked, &backups).map(|_| t("game-mod-chat-export-on"))
                } else {
                    restore(&cooked, &backups).map(|_| t("game-mod-chat-export-off"))
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

    #[test]
    fn statement_is_a_log_call_over_two_struct_member_reads() {
        let script = statement(PARAMETER_READ, 100, 200, 300, 400);
        let mut expected = vec![0xe7, 0xf8, 0xf8, 0xf8, 0x1f];
        expected.extend_from_slice(b"HBXCHAT\t");
        expected.push(0x00);
        for (field, tail) in [(300i32, vec![0x16, 0x1f, 0x09, 0x00, 0x16]), (400, vec![0x16, 0x48, 0x48, 0x16])] {
            expected.push(0x35);
            expected.extend_from_slice(&field.to_le_bytes());
            expected.extend_from_slice(&200i32.to_le_bytes());
            expected.extend_from_slice(&[0x00, 0x00, 0x46]);
            expected.extend_from_slice(&100i32.to_le_bytes());
            expected.extend_from_slice(&tail);
        }
        assert_eq!(script.bytes(), &expected[..]);
    }

    #[test]
    fn only_the_games_own_script_log_lines_become_chat() {
        let log = "[0150.10] ScriptLog: HBXCHAT\tAnna\thello\tthere\n\
                   [0150.20] Log: HBXCHAT\tMallory\tnot script log\n\
                   [0150.30] ScriptLog: something else\n\
                   [0150.40] ScriptLog: HBXCHAT\tBob\tgg";
        assert_eq!(
            json_lines(log, 7),
            [
                r#"{"time":7,"player":"Anna","message":"hello\tthere"}"#,
                r#"{"time":7,"player":"Bob","message":"gg"}"#,
            ]
        );
    }

    /// Patch a copy of the real package, reload it and look for the log call.
    /// Run with HEBNIX_UPK_TEST_SOURCE set to a TAGame.upk.
    #[test]
    #[ignore]
    fn real_package() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        apply(&mut upk).unwrap();
        let out = std::env::temp_dir().join("hebnix_chat_test.upk");
        upk.save_fast(&out).unwrap();
        let reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(states(&reloaded).unwrap().iter().all(|state| *state));
    }

    /// Enable then restore on a temp copy of the real package, which must be
    /// the vanilla TAGame.upk, and check the function comes back byte for byte.
    #[test]
    #[ignore]
    fn enable_restore() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let root = std::env::temp_dir().join("hebnix_chat_install");
        let _ = std::fs::remove_dir_all(&root);
        let cooked = root.join("TAGame").join("CookedPCConsole");
        let backups = cooked.join("Backups");
        std::fs::create_dir_all(&cooked).unwrap();
        std::fs::create_dir_all(root.join("TAGame/Binaries/Win64")).unwrap();
        std::fs::write(root.join("TAGame/Binaries/Win64/RocketLeague.exe"), b"exe").unwrap();
        std::fs::copy(&source, cooked.join("TAGame.upk")).unwrap();
        let load = || UpkPackage::load(&cooked.join("TAGame.upk")).unwrap();
        let vanilla: Vec<Vec<u8>> = {
            let upk = load();
            TARGETS.iter().map(|t| payload(&upk, function_index(&upk, t).unwrap())).collect()
        };
        enable(&cooked, &backups).unwrap();
        let upk = load();
        assert!(enabled(&backups) && states(&upk).unwrap().iter().all(|state| *state));
        restore(&cooked, &backups).unwrap();
        let upk = load();
        assert!(!enabled(&backups));
        for (t, original) in TARGETS.iter().zip(&vanilla) {
            assert_eq!(&payload(&upk, function_index(&upk, t).unwrap()), original);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
