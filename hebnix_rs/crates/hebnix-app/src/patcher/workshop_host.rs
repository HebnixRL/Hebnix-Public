//! Start a Workshop map from Hebnix. The stats socket already delivers every
//! command to MatchStatsExporter_TA.ProcessSocketCommand, so a block added to
//! its end answers {"Command":"HebnixHost"}:
//!
//!     if (CommandJson == "{\"Command\":\"HebnixHost\"}") {
//!         class'FileSystem'.static.LoadFileToString("..\..\TAGame\HbxRun.txt", CommandJson);
//!         GFxData_Workshop_TA(DataStore.GetObject(...)).LoadWorkshopMap(CommandJson);
//!     }
//!
//! HbxRun.txt holds the item path of the map's LAN slot, the same text a
//! right-click in the in-game list loads, so the host travels to the slot copy
//! and the game tags come from HbxHost.txt as they do there. CommandJson, an
//! out string the command is done with, is the scratch string. The data object is
//! made when it does not exist yet, as the match history bridge does. Needs the
//! Workshop menu's renamed LoadFileToString. The block goes after the function's
//! own call, so the history bridge (which inserts at the start) is untouched.
use super::script_patch::{Script, replace_script};
use super::upk_package::UpkPackage;
use super::game_path;
use std::path::Path;

const PACKAGE: &str = "TAGame.upk";
const COMMAND: &str = "MatchStatsExporter_TA.ProcessSocketCommand";
const WORKSHOP: &str = "GFxData_Workshop_TA";
/// The stats socket message Hebnix sends, and the file with the path to load.
pub const MESSAGE: &str = "HebnixHost";
const REQUEST: &str = "{\"Command\":\"HebnixHost\"}";
pub const RUN_FILE: &str = "HbxRun.txt";
const RUN_PATH: &str = r"..\..\TAGame\HbxRun.txt";
/// Return, Nothing, EndOfScript: how the function ends.
const END: usize = 3;
const FREEPLAY_REQUEST: &str = "{\"Command\":\"HebnixRestartFreeplayV3\"}";
const V2_FREEPLAY_REQUEST: &str = "{\"Command\":\"HebnixRestartFreeplayV2\"}";
const OLD_FREEPLAY_REQUEST: &str = "{\"Command\":\"HebnixRestartFreeplay\"}";
const FREEPLAY_FILE: &str = "HbxFreeplayRun.txt";
const FREEPLAY_ENABLED: &str = "Freeplay.enabled";
const LAN_NAME: &str = "HbxLanName.txt";
const LAN_PASSWORD: &str = "HbxLanPassword.txt";

fn object(upk: &UpkPackage, path: &str) -> Result<i32, String> {
    upk.find_object(path).ok_or_else(|| format!("{path} was not found in {PACKAGE}"))
}

fn field(upk: &UpkPackage, op: u8, path: &str) -> Result<Script, String> {
    let mut s = Script::new();
    s.raw(&[op]).object(object(upk, path)?);
    Ok(s)
}

/// A final-function call: the function, its arguments, end.
fn call(upk: &UpkPackage, path: &str, args: &[Script]) -> Result<Script, String> {
    let mut s = field(upk, 0x1c, path)?;
    for arg in args {
        s.append(arg);
    }
    s.raw(&[0x16]);
    Ok(s)
}

/// Context: `target.body`, the size of the body in memory, the return property.
fn context(upk: &UpkPackage, target: Script, body: Script, property: Option<&str>) -> Result<Script, String> {
    let size = u16::try_from(body.mem_size()).map_err(|_| "The host block's context is too long".to_string())?;
    let mut s = Script::new();
    s.raw(&[0x19, 0]).append(&target).raw(&size.to_le_bytes());
    s.object(property.map(|p| object(upk, p)).transpose()?.unwrap_or(0));
    s.raw(&[0x00]).append(&body);
    Ok(s)
}

fn native(opcode: u8, args: &[Script]) -> Script {
    let mut s = Script::new();
    s.raw(&[opcode]);
    for arg in args {
        s.append(arg);
    }
    s.raw(&[0x16]);
    s
}

fn string(text: &str) -> Script {
    let mut s = Script::new();
    s.raw(&[0x1f]).raw(text.as_bytes()).raw(&[0x00]);
    s
}

/// The player's data store: PrimaryPC.GetLocalPlayer() then GFxShell_TA.GetShell(..).DataStore.
fn store(upk: &UpkPackage) -> Result<Script, String> {
    let player = context(
        upk,
        field(upk, 0x01, "MatchStatsExporter_TA.PrimaryPC")?,
        call(upk, "PlayerControllerBase_TA.GetLocalPlayer", &[])?,
        Some("PlayerControllerBase_TA.GetLocalPlayer.ReturnValue"),
    )?;
    context(
        upk,
        call(upk, "GFxShell_TA.GetShell", &[player])?,
        field(upk, 0x01, "ProjectX.GFxShell_X.DataStore")?,
        Some("ProjectX.GFxShell_X.DataStore"),
    )
}

/// GFxData_Workshop_TA(store.GetObject(class'GFxData_Workshop_TA'))
fn workshop(upk: &UpkPackage) -> Result<Script, String> {
    let get = context(
        upk,
        store(upk)?,
        call(upk, "ProjectX.GFxDataStore_X.GetObject", &[field(upk, 0x20, WORKSHOP)?, native_nothing()])?,
        Some("ProjectX.GFxDataStore_X.GetObject.ReturnValue"),
    )?;
    let mut s = field(upk, 0x2e, WORKSHOP)?;
    s.append(&get);
    Ok(s)
}

/// An omitted optional argument.
fn native_nothing() -> Script {
    let mut s = Script::new();
    s.raw(&[0x48]);
    s
}

fn jump(target: usize, condition: &Script) -> Result<Script, String> {
    let target = u16::try_from(target).map_err(|_| "The host block's jump is past the script limit".to_string())?;
    let mut s = Script::new();
    s.raw(&[0x07]).raw(&target.to_le_bytes()).append(condition);
    Ok(s)
}

/// The block, its jump targets counted from the start of the function: `base`
/// is where it starts, in memory form.
fn block(upk: &UpkPackage, base: usize) -> Result<Script, String> {
    let command = field(upk, 0x2b, &format!("{COMMAND}.CommandJson"))?;
    let load_file = upk
        .names
        .iter()
        .position(|n| matches!(n.as_str(), "LoadFileToString" | "TransformedLeftX"))
        .map(|i| [(i as i32).to_le_bytes(), [0; 4]].concat())
        .ok_or("The Workshop menu is not enabled: LoadFileToString is missing")?;
    let file_system = object(upk, "Core.FileSystem")?;

    let request = native(204, &[command.clone(), string(REQUEST)]);
    let is_none = native(207, &[workshop(upk)?, Script::from_parts(vec![0x2a], 0)]);
    let create = context(
        upk,
        store(upk)?,
        call(upk, "ProjectX.GFxDataStore_X.CreateObject", &[field(upk, 0x20, WORKSHOP)?, native_nothing(), native_nothing()])?,
        Some("ProjectX.GFxDataStore_X.CreateObject.ReturnValue"),
    )?;
    // ClassContext(class'FileSystem'), skip, no return property, then
    // LoadFileToString(path, CommandJson)
    let mut read = Script::new();
    read.raw(&[0x1b]).raw(&load_file).append(&string(RUN_PATH)).append(&command).raw(&[0x16]);
    let mut load = Script::new();
    load.raw(&[0x5e, 0x12, 0x00, 0x20]).object(file_system);
    load.raw(&u16::try_from(read.mem_size()).map_err(|e| e.to_string())?.to_le_bytes());
    // no return property (still an object reference in memory), then the call
    load.object(0).raw(&[0x00]).append(&read);
    let start = context(upk, workshop(upk)?, call(upk, &format!("{WORKSHOP}.LoadWorkshopMap"), &[command])?, None)?;

    // positions in memory form, from the block's start
    let outer = 3 + request.mem_size();
    let inner = 3 + is_none.mem_size();
    let created = base + outer + inner + create.mem_size();
    let finish = base + outer + inner + create.mem_size() + load.mem_size() + start.mem_size();

    let mut out = Script::new();
    out.append(&jump(finish, &request)?);
    out.append(&jump(created, &is_none)?);
    out.append(&create).append(&load).append(&start);
    Ok(out)
}

fn payload(upk: &UpkPackage) -> Result<(usize, Script), String> {
    let index = object(upk, COMMAND).ok().filter(|i| *i > 0).ok_or("ProcessSocketCommand was not found")? as usize - 1;
    let export = &upk.exports[index];
    let bytes = &upk.image[export.serial_offset..export.serial_offset + export.serial_size];
    Ok((index, Script::of_function(bytes)?))
}

fn load_text(upk: &UpkPackage, filename: &str, destination: &Script) -> Result<Script, String> {
    let loader = upk.names.iter().position(|n| matches!(n.as_str(), "LoadFileToString" | "TransformedLeftX"))
        .ok_or("The Workshop file loader is missing")? as i32;
    let path = format!(r"..\..\TAGame\{filename}");
    let mut read = Script::new();
    read.raw(&[0x1b]).name(loader).append(&string(&path)).append(destination).raw(&[0x16]);
    let mut load = Script::new();
    load.raw(&[0x5e, 0x12, 0, 0x20]).object(object(upk, "Core.FileSystem")?)
        .raw(&(read.mem_size() as u16).to_le_bytes()).object(0).raw(&[0]).append(&read);
    Ok(load)
}

fn freeplay_block(upk: &UpkPackage, base: usize, version: u8) -> Result<Script, String> {
    let legacy = version == 1;
    let command = field(upk, 0x2b, &format!("{COMMAND}.CommandJson"))?;
    let load = load_text(upk, FREEPLAY_FILE, &command)?;
    let mut open = native(
        if legacy { 0xf8 } else { 0xa8 },
        &[string(if legacy { "open " } else { "Open" }), command.clone()],
    );
    if version == 3 {
        // The file holds Disconnect or a complete Open/Start travel command.
        open = command.clone();
    }
    let execute = if legacy {
        // Retained to recognize and remove the first restart hook.
        let console = upk.names.iter().position(|n| n == "ConsoleCommand")
            .ok_or("ConsoleCommand is missing")? as i32;
        let mut execute = Script::new();
        execute.raw(&[0x0e]).object(object(upk, "Engine.Actor.ConsoleCommand.ReturnValue")?)
            .raw(&[0x1b]).name(console).append(&open).raw(&[0x28, 0x16]);
        context(upk, field(upk, 0x01, "MatchStatsExporter_TA.PrimaryPC")?, execute, None)?
    } else {
        // Match TravelManager.Open: call Travel with the Open command and
        // let the game choose its own command user.
        let travel = call(upk, "TravelManager_TA.Travel", &[open, native_nothing()])?;
        let mut execute = Script::new();
        execute.raw(&[0x5e, 0x12, 0, 0x20]).object(object(upk, "TravelManager_TA")?)
            .raw(&(travel.mem_size() as u16).to_le_bytes()).object(0).raw(&[0]).append(&travel);
        execute
    };
    let request = native(204, &[command, string(match version {
        1 => OLD_FREEPLAY_REQUEST,
        2 => V2_FREEPLAY_REQUEST,
        _ => FREEPLAY_REQUEST,
    })]);
    let end = base + 3 + request.mem_size() + load.mem_size() + execute.mem_size();
    let mut block = jump(end, &request)?;
    block.append(&load).append(&execute);
    Ok(block)
}

pub(super) fn edit_freeplay_restart(upk: &mut UpkPackage, on: bool) -> Result<bool, String> {
    let (_, mut script) = payload(upk)?;
    let installed = freeplay_version(&script);
    if installed == on.then_some(3) {
        return Ok(false);
    }
    if !script.bytes().ends_with(&[0x04, 0x0b, 0x4c]) {
        return Err(format!("{COMMAND} changed layout in this Rocket League version"));
    }
    if let Some(version) = installed {
        let probe = freeplay_block(upk, 0, version)?;
        let start = script.len().checked_sub(END + probe.len()).ok_or("Freeplay restart hook is missing")?;
        let block = freeplay_block(upk, script.mem_size() - END - probe.mem_size(), version)?;
        if script.bytes().get(start..start + block.len()) != Some(block.bytes()) {
            return Err("The Freeplay restart hook changed layout".into());
        }
        script.cut(start, block.len(), block.refs());
    }
    if on {
        let block = freeplay_block(upk, script.mem_size() - END, 3)?;
        script.insert_before_tail(END, &block);
    }
    replace_script(upk, COMMAND, &script)?;
    Ok(true)
}

/// Temporarily remove the restart hook while another feature changes the
/// shared dispatcher. Return its variant so it can be put back unchanged.
fn freeplay_version(script: &Script) -> Option<u8> {
    [(3, FREEPLAY_REQUEST), (2, V2_FREEPLAY_REQUEST), (1, OLD_FREEPLAY_REQUEST)]
        .into_iter().find_map(|(version, request)| {
            script.bytes().windows(request.len()).any(|bytes| bytes == request.as_bytes()).then_some(version)
        })
}

pub(crate) fn detach_freeplay_restart(upk: &mut UpkPackage) -> Result<Option<u8>, String> {
    let (_, script) = payload(upk)?;
    let version = freeplay_version(&script);
    edit_freeplay_restart(upk, false)?;
    Ok(version)
}

pub(crate) fn attach_freeplay_restart(upk: &mut UpkPackage, version: u8) -> Result<(), String> {
    let (_, mut script) = payload(upk)?;
    if !script.bytes().ends_with(&[0x04, 0x0b, 0x4c]) {
        return Err(format!("{COMMAND} changed layout in this Rocket League version"));
    }
    let block = freeplay_block(upk, script.mem_size() - END, version)?;
    script.insert_before_tail(END, &block);
    replace_script(upk, COMMAND, &script)
}

pub const FREEPLAY_MESSAGE: &str = "HebnixRestartFreeplayV3";

pub fn request_freeplay_travel(
    cooked: &Path,
    stats: &hebnix_sdk::stats::WsStatsClient,
    command: &str,
) -> Result<(), String> {
    if !stats.connected() {
        return Err("Connect Rocket League's Stats API to restart Freeplay.".into());
    }
    let path = cooked.parent().unwrap_or(cooked).join(FREEPLAY_FILE);
    super::upk_history::replace(&path, command.as_bytes())
        .map_err(|error| format!("Could not prepare Freeplay travel: {error}"))?;
    stats.send_command(hebnix_sdk::stats::WsCommand::new(FREEPLAY_MESSAGE))
        .map_err(|_| "Could not queue Freeplay travel.".into())
}

pub fn freeplay_ready(cooked: &Path) -> bool {
    UpkPackage::load(&game_path::resolve(cooked, PACKAGE)).is_ok_and(|upk| {
        payload(&upk).is_ok_and(|(_, script)| script.bytes().windows(FREEPLAY_REQUEST.len())
            .any(|window| window == FREEPLAY_REQUEST.as_bytes()))
    })
}

pub(super) fn has_freeplay_restart(upk: &UpkPackage) -> bool {
    payload(upk).is_ok_and(|(_, script)| freeplay_version(&script).is_some())
}

pub(crate) fn set_freeplay_patch(cooked: &Path, enabled: bool) -> Result<(), String> {
    let _guard = super::profile_pictures::PACKAGE_EDIT.lock().map_err(|error| error.to_string())?;
    ensure_closed()?;
    let backups = cooked.join("Backups");
    super::backup_guard::check_install(cooked, &backups)?;
    let live = game_path::resolve(cooked, PACKAGE);
    let before = std::fs::read(&live).map_err(|error| error.to_string())?;
    let mut upk = UpkPackage::from_bytes(before.clone())?;
    let workshop = super::workshop_menu::status(cooked)?;
    let restart = edit_freeplay_restart(&mut upk, enabled)?;
    let solo = super::workshop_menu::edit_solo_freeplay(&mut upk, enabled, workshop)?;
    if restart || solo {
        super::workshop_menu::rename_names(&mut upk, workshop)?;
        let after = upk.repacked_fast()?;
        let mut staged = UpkPackage::from_bytes(after.clone())?;
        if edit_freeplay_restart(&mut staged, enabled)?
            || super::workshop_menu::edit_solo_freeplay(&mut staged, enabled, workshop)?
        {
            return Err("Freeplay patch validation failed.".into());
        }
        ensure_closed()?;
        super::upk_history::install_linked_from(cooked, &live, &before, &after, "Freeplay mutators")?;
    }
    if enabled {
        super::workshop_menu::migrate_solo_preferences(cooked)?;
        super::upk_history::replace(&backups.join(FREEPLAY_ENABLED), b"on")?;
    } else if backups.join(FREEPLAY_ENABLED).is_file() {
        std::fs::remove_file(backups.join(FREEPLAY_ENABLED)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn recover_freeplay(cooked: &Path, archive: &Path) -> Result<(), String> {
    if archive.join(FREEPLAY_ENABLED).is_file() {
        set_freeplay_patch(cooked, true)?;
    }
    Ok(())
}

/// Override the LAN settings returned to both the server browser and the
/// login validator. The profile's private-match settings stay on disk as-is.
pub(super) fn edit_lan_credentials(upk: &mut UpkPackage, on: bool) -> Result<bool, String> {
    const FUNCTION: &str = "ListenServer_TA.GetCustomMatchSettings";
    const SETTINGS: &str = "ProjectX._Types_X.CustomMatchSettings";
    const EXPRESSION_LEN: usize = 76;
    const EXPRESSION_REFS: usize = 10;
    let index = object(upk, FUNCTION)? as usize - 1;
    let export = &upk.exports[index];
    let current = Script::of_function(&upk.image[export.serial_offset..export.serial_offset + export.serial_size])?;
    let result = field(upk, 0x2b, &format!("{FUNCTION}.ReturnValue"))?;
    let mut trailer = Script::new();
    trailer.raw(&[0x04, 0x3a]).object(object(upk, &format!("{FUNCTION}.ReturnValue"))?).raw(&[0x4c]);
    let mut assign = Script::new();
    assign.raw(&[0x0f]).append(&result);
    let mut suffix = Script::new();
    for (member, filename) in [("ServerName", LAN_NAME), ("Password", LAN_PASSWORD)] {
        let mut destination = Script::new();
        destination.raw(&[0x35]).object(object(upk, &format!("{SETTINGS}.{member}"))?)
            .object(object(upk, SETTINGS)?).raw(&[0, 1]).append(&result);
        suffix.append(&load_text(upk, filename, &destination)?);
    }
    suffix.raw(&[0x04]).append(&result).append(&trailer);

    let bytes = current.bytes();
    let vanilla = bytes.len() == 1 + EXPRESSION_LEN + trailer.len()
        && current.refs() == EXPRESSION_REFS + trailer.refs()
        && bytes.starts_with(&[0x04, 0x5e, 0x19, 0x00])
        && bytes.ends_with(trailer.bytes());
    let patched = bytes.len() == assign.len() + EXPRESSION_LEN + suffix.len()
        && bytes.starts_with(assign.bytes()) && bytes.ends_with(suffix.bytes());
    if !vanilla && !patched {
        return Err(format!("{FUNCTION} changed layout in this Rocket League version"));
    }
    if on == patched {
        return Ok(false);
    }
    let start = if vanilla { 1 } else { assign.len() };
    let expression = Script::from_parts(bytes[start..start + EXPRESSION_LEN].to_vec(), EXPRESSION_REFS);
    let mut replacement = Script::new();
    if on {
        replacement.append(&assign).append(&expression).append(&suffix);
    } else {
        replacement.raw(&[0x04]).append(&expression).append(&trailer);
    }
    replace_script(upk, FUNCTION, &replacement)?;
    Ok(true)
}

pub fn read_lan_credentials(cooked: &Path) -> Option<(String, String)> {
    let directory = cooked.parent().unwrap_or(cooked);
    let password = std::fs::read_to_string(directory.join(LAN_PASSWORD)).ok()?;
    let name = std::fs::read_to_string(directory.join(LAN_NAME)).unwrap_or_default();
    Some((name, password))
}

pub fn store_lan_credentials(cooked: &Path, credentials: Option<(&str, &str)>) -> Result<(), String> {
    let directory = cooked.parent().unwrap_or(cooked);
    let name = credentials.map(|(name, _)| name).filter(|name| !name.is_empty());
    let password = credentials.map(|(_, password)| password);
    if name.is_some_and(|name| name.chars().any(char::is_control))
        || password.is_some_and(|password| password.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '?' | '#' | '"' | '|' | ';' | '&')
        }))
    {
        return Err("Use a password without spaces or URL separators.".into());
    }
    for (file, value) in [(LAN_NAME, name), (LAN_PASSWORD, password)] {
        let path = directory.join(file);
        let result = match value {
            Some(value) => std::fs::write(path, value),
            None => std::fs::remove_file(path).or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(error) }
            }),
        };
        result.map_err(|error| format!("Could not save LAN match settings: {error}"))?;
    }
    Ok(())
}

/// Whether the block is in the function.
pub fn has(upk: &UpkPackage) -> bool {
    payload(upk).is_ok_and(|(_, script)| script.bytes().windows(REQUEST.len()).any(|w| w == REQUEST.as_bytes()))
}

fn add(upk: &mut UpkPackage) -> Result<(), String> {
    if has(upk) {
        return Ok(());
    }
    let (_, mut script) = payload(upk)?;
    if script.bytes().ends_with(&[0x04, 0x0b, 0x4c]) {
        let base = script.mem_size() - END;
        let block = block(upk, base)?;
        script.insert_before_tail(END, &block);
        return replace_script(upk, COMMAND, &script);
    }
    Err(format!("{COMMAND} changed layout in this Rocket League version"))
}

fn remove(upk: &mut UpkPackage) -> Result<(), String> {
    if !has(upk) {
        return Ok(());
    }
    let (_, mut script) = payload(upk)?;
    let probe = block(upk, 0)?;
    // the block was written for its place, so find its start by its length
    let start = script.len().checked_sub(END + probe.len()).ok_or("The host block is not where it was put")?;
    let base = script.mem_size() - END - probe.mem_size();
    let here = block(upk, base)?;
    if script.bytes().get(start..start + here.len()) != Some(here.bytes()) {
        return Err("The host block is not where it was put".into());
    }
    script.cut(start, here.len(), here.refs());
    replace_script(upk, COMMAND, &script)
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the host command.".into())
    } else {
        Ok(())
    }
}

/// Write `upk` over the live package after proving the staged copy has (or lacks) the block.
fn install(cooked: &Path, live: &Path, upk: &UpkPackage, patched: bool) -> Result<(), String> {
    let temp = cooked.join("TAGame.upk.host.tmp");
    upk.save_fast(&temp)?;
    let check = UpkPackage::load(&temp).map(|saved| has(&saved));
    if check != Ok(patched) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation: {check:?}"));
    }
    let copied = std::fs::copy(&temp, live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

/// Add the command to the live package; a game that is running keeps its old copy.
pub fn enable(cooked: &Path) -> Result<(), String> {
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if has(&upk) {
        return Ok(());
    }
    ensure_closed()?;
    add(&mut upk)?;
    install(cooked, &live, &upk, true)
}

/// Take the command out of the live package.
pub fn restore(cooked: &Path) -> Result<(), String> {
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    if !has(&upk) {
        return Ok(());
    }
    ensure_closed()?;
    remove(&mut upk)?;
    install(cooked, &live, &upk, false)
}

/// Where Hebnix writes the path for the command to load.
pub fn run_file(cooked: &Path) -> std::path::PathBuf {
    cooked.parent().unwrap_or(cooked).join(RUN_FILE)
}

/// Whether the live package can take the command now.
pub fn ready(cooked: &Path) -> bool {
    UpkPackage::load(&game_path::resolve(cooked, PACKAGE)).is_ok_and(|upk| has(&upk))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Add and remove the block on a copy of a vanilla TAGame.upk with the
    /// Workshop menu's names (HEBNIX_UPK_TEST_SOURCE), and reload it between.
    #[test]
    #[ignore]
    fn add_then_remove_restores_the_function() {
        let source = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        let mut upk = UpkPackage::load(Path::new(&source)).unwrap();
        super::super::workshop_menu::rename_names(&mut upk, true).unwrap();
        super::super::workshop_menu::edit_solo_freeplay(&mut upk, true, false).unwrap();
        assert!(!super::super::workshop_menu::edit_solo_freeplay(&mut upk, true, false).unwrap());
        edit_freeplay_restart(&mut upk, false).unwrap();
        remove(&mut upk).unwrap();
        edit_lan_credentials(&mut upk, false).unwrap();
        let before = payload(&upk).unwrap().1.bytes().to_vec();
        let credentials = |upk: &UpkPackage| {
            let e = &upk.exports[upk.find_object("ListenServer_TA.GetCustomMatchSettings").unwrap() as usize - 1];
            upk.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec()
        };
        let credentials_before = credentials(&upk);
        add(&mut upk).unwrap();
        let (_, mut old_restart) = payload(&upk).unwrap();
        let legacy = freeplay_block(&upk, old_restart.mem_size() - END, 1).unwrap();
        old_restart.insert_before_tail(END, &legacy);
        replace_script(&mut upk, COMMAND, &old_restart).unwrap();
        assert!(edit_freeplay_restart(&mut upk, true).unwrap());
        assert!(edit_lan_credentials(&mut upk, true).unwrap());
        let out = std::env::temp_dir().join("hebnix_host_test.upk");
        upk.save_fast(&out).unwrap();
        let mut reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(has(&reloaded));
        assert!(!super::super::workshop_menu::edit_solo_freeplay(&mut reloaded, true, false).unwrap());
        assert!(!super::super::workshop_menu::edit_solo_freeplay(&mut reloaded, true, true).unwrap());
        super::super::workshop_menu::rename_names(&mut reloaded, false).unwrap();
        assert!(reloaded.names.iter().any(|name| name == "LoadFileToString"));
        assert!(has_freeplay_restart(&reloaded));
        assert!(!edit_freeplay_restart(&mut reloaded, true).unwrap());
        assert!(!edit_lan_credentials(&mut reloaded, true).unwrap());
        assert!(edit_freeplay_restart(&mut reloaded, false).unwrap());
        assert!(edit_lan_credentials(&mut reloaded, false).unwrap());
        assert_eq!(credentials(&reloaded), credentials_before);
        remove(&mut reloaded).unwrap();
        assert!(!has(&reloaded));
        assert_eq!(payload(&reloaded).unwrap().1.bytes(), &before[..]);
    }
}
