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
        .position(|n| n == "LoadFileToString")
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
        let before = payload(&upk).unwrap().1.bytes().to_vec();
        add(&mut upk).unwrap();
        let out = std::env::temp_dir().join("hebnix_host_test.upk");
        upk.save_fast(&out).unwrap();
        let mut reloaded = UpkPackage::load(&out).unwrap();
        let _ = std::fs::remove_file(out);
        assert!(has(&reloaded));
        remove(&mut reloaded).unwrap();
        assert!(!has(&reloaded));
        assert_eq!(payload(&reloaded).unwrap().1.bytes(), &before[..]);
    }
}
