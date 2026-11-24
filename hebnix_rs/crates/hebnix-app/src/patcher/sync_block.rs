//! Keeps Psyonix's inventory sync from replacing items added to the save. Two
//! in-place edits to TAGame.upk, neither changing any export length:
//! `SaveData_TA.HandleOnlineProductsReceived` returns at once, and
//! `ProductDatabase_TA.IsDefaultProduct` is always true. The edited bytes are
//! specific to one game build, so a build that differs is refused.
use super::upk_package::UpkPackage;
use super::{backup_guard, game_path};
use std::path::Path;

const PACKAGE: &str = "TAGame.upk";
const HANDLER: &str = "SaveData_TA.HandleOnlineProductsReceived";
const DEFAULT_PRODUCT: &str = "ProductDatabase_TA.IsDefaultProduct";

/// `IsDefaultProduct` body that decides default status; replaced by 28 `True` tokens.
const DEFAULT_CHECK: [u8; 28] = [
    0x9A, 0x38, 0x3A, 0x5E, 0x19, 0x00, 0x2B, 0xC4, 0x61, 0x00, 0x00, 0x09, 0x00, 0x37, 0x9C, 0x00,
    0x00, 0x00, 0x01, 0x37, 0x9C, 0x00, 0x00, 0x38, 0x3A, 0x24, 0x00, 0x16,
];
const TRUE: u8 = 0x27;
/// The 3 object references in the replaced span took 4 extra bytes each in memory.
const MEMORY_SIZE_AT: usize = 40;
const MEMORY_SIZE_ORIGINAL: u32 = 0x45;
const MEMORY_SIZE_PATCHED: u32 = MEMORY_SIZE_ORIGINAL - 12;
/// `Let` in the handler's first statement, which becomes `Return`.
const HANDLER_BYTE_AT: usize = 48;
const LET: u8 = 0x0F;
const RETURN: u8 = 0x04;

struct Spots {
    default_export: usize,
    default_offset: usize,
    handler_byte: usize,
}

fn export_bytes<'a>(upk: &'a UpkPackage, path: &str) -> Result<(usize, &'a [u8]), String> {
    let index = upk.find_object(path).filter(|i| *i > 0).ok_or_else(|| format!("{path} was not found"))?;
    let export = &upk.exports[index as usize - 1];
    Ok((export.serial_offset, &upk.image[export.serial_offset..export.serial_offset + export.serial_size]))
}

/// `Some(true)` patched, `Some(false)` original, `None` for an unknown build.
fn state(upk: &UpkPackage) -> Option<(bool, Spots)> {
    let (handler_start, handler) = export_bytes(upk, HANDLER).ok()?;
    let (default_start, default) = export_bytes(upk, DEFAULT_PRODUCT).ok()?;
    let memory = u32::from_le_bytes(default.get(MEMORY_SIZE_AT..MEMORY_SIZE_AT + 4)?.try_into().ok()?);
    let handler_byte = *handler.get(HANDLER_BYTE_AT)?;
    let original = default.windows(28).position(|w| w == DEFAULT_CHECK);
    let patched = default.windows(28).position(|w| w.iter().all(|b| *b == TRUE));
    let spots = |at| Spots {
        default_export: default_start,
        default_offset: at,
        handler_byte: handler_start + HANDLER_BYTE_AT,
    };
    match (original, patched) {
        (Some(at), _) if memory == MEMORY_SIZE_ORIGINAL && handler_byte == LET => Some((false, spots(at))),
        (_, Some(at)) if memory == MEMORY_SIZE_PATCHED && handler_byte == RETURN => Some((true, spots(at))),
        _ => None,
    }
}

pub fn active(cooked: &Path) -> bool {
    UpkPackage::load(&game_path::resolve(cooked, PACKAGE)).ok().and_then(|upk| state(&upk)).is_some_and(|(on, _)| on)
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the inventory sync block.".into())
    } else {
        Ok(())
    }
}

fn set(cooked: &Path, backups: &Path, on: bool) -> Result<(), String> {
    ensure_closed()?;
    backup_guard::check_install(cooked, backups)?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    let (patched, spots) = state(&upk).ok_or("This game build does not match the sync block. Update Hebnix or verify the game files.")?;
    if patched == on {
        return Ok(());
    }
    let (check, byte, memory) = if on {
        ([TRUE; 28], RETURN, MEMORY_SIZE_PATCHED)
    } else {
        (DEFAULT_CHECK, LET, MEMORY_SIZE_ORIGINAL)
    };
    upk.patch(spots.default_export + spots.default_offset, &check)?;
    upk.patch(spots.default_export + MEMORY_SIZE_AT, &memory.to_le_bytes())?;
    upk.patch(spots.handler_byte, &[byte])?;
    let temp = cooked.join("TAGame.upk.sync_block.tmp");
    upk.save_fast(&temp)?;
    let verified = UpkPackage::load(&temp).ok().and_then(|saved| state(&saved)).map(|(now, _)| now);
    if verified != Some(on) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Staged {PACKAGE} failed validation."));
    }
    let copied = std::fs::copy(&temp, &live);
    let _ = std::fs::remove_file(&temp);
    copied.map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
}

pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    set(cooked, backups, true)
}

pub fn disable(cooked: &Path, backups: &Path) -> Result<(), String> {
    set(cooked, backups, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a real install: set HEBNIX_TEST_COOKED to the CookedPCConsole folder.
    #[test]
    fn reads_the_state_of_an_installed_package() {
        let Ok(cooked) = std::env::var("HEBNIX_TEST_COOKED") else { return };
        let upk = UpkPackage::load(&game_path::resolve(Path::new(&cooked), PACKAGE)).unwrap();
        assert!(state(&upk).is_some(), "build not recognised");
        println!("sync block active: {}", state(&upk).unwrap().0);
    }
}
