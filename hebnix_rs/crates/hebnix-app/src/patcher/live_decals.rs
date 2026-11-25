use std::io::{Cursor, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::upk_package::UpkPackage;
use super::workshop_menu_ui::Movie;
use super::{backup_guard, game_path, instant_suite, title_patcher, upk_history};

const GARAGE: Movie = Movie {
    package: "GFx_GarageMenu_SF.upk",
    export: "GFx_GarageMenu.GarageMenu",
    patched: &[],
    vanilla: "",
};
const INSTALL: &[u8] = include_bytes!("../../assets/decal-studio/garage-enable.bin");
const RESTORE: &[u8] = include_bytes!("../../assets/decal-studio/garage-restore.bin");
const RESTORE_V1: &[u8] = include_bytes!("../../assets/decal-studio/garage-restore-v1.bin");
const RESTORE_V2: &[u8] = include_bytes!("../../assets/decal-studio/garage-restore-v2.bin");
const LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GaragePatch {
    Original,
    Previous(&'static [u8]),
    Current,
}

fn apply_delta(abc: &[u8], delta: &[u8]) -> Result<Option<(Vec<u8>, usize)>, String> {
    if Sha256::digest(abc).as_slice() != &delta[..32] {
        return Ok(None);
    }
    let mut instructions = Vec::new();
    flate2::read::ZlibDecoder::new(&delta[64..])
        .take(LIMIT as u64)
        .read_to_end(&mut instructions)
        .map_err(|error| error.to_string())?;
    let mut reader = Cursor::new(instructions.as_slice());
    let mut patched = Vec::new();
    let read_num = |reader: &mut Cursor<&[u8]>| -> Result<usize, String> {
        let mut word = [0; 4];
        reader.read_exact(&mut word).map_err(|e| e.to_string())?;
        Ok(u32::from_le_bytes(word) as usize)
    };
    while (reader.position() as usize) < instructions.len() {
        let mut opcode = [0];
        reader.read_exact(&mut opcode).map_err(|e| e.to_string())?;
        let length = read_num(&mut reader)?;
        if length > LIMIT - patched.len() {
            return Err("Garage patch exceeds its output limit".into());
        }
        match opcode[0] {
            0 => {
                let start = reader.position() as usize;
                let literal = instructions
                    .get(start..start + length)
                    .ok_or("Truncated garage delta")?;
                patched.extend_from_slice(literal);
                reader.set_position((start + length) as u64);
            }
            1 => {
                let start = read_num(&mut reader)?;
                patched.extend_from_slice(
                    abc.get(start..start + length)
                        .ok_or("Invalid garage delta offset")?,
                );
            }
            _ => return Err("Unknown garage delta operation".into()),
        }
    }
    if Sha256::digest(&patched).as_slice() != &delta[32..64] {
        return Err("Garage patch failed integrity verification".into());
    }
    Ok(Some((patched, 1)))
}

fn inspect(movie: &[u8]) -> Result<GaragePatch, String> {
    let known = std::cell::Cell::new(None);
    title_patcher::movie::rewrite(movie, &|data| {
        let hash = Sha256::digest(data);
        let hash = hash.as_slice();
        if hash == &INSTALL[..32] {
            known.set(Some(GaragePatch::Original));
        } else if hash == &RESTORE[..32] {
            known.set(Some(GaragePatch::Current));
        } else if let Some(delta) = [RESTORE_V1, RESTORE_V2]
            .into_iter()
            .find(|delta| hash == &delta[..32])
        {
            known.set(Some(GaragePatch::Previous(delta)));
        }
        Ok(None)
    })?;
    known
        .get()
        .ok_or_else(|| "This game build has a different garage ActionScript block.".into())
}

pub(crate) fn status(cooked: &Path) -> Result<bool, String> {
    let garage = UpkPackage::load(&game_path::resolve(cooked, GARAGE.package))?;
    Ok(inspect(&GARAGE.movie(&garage)?)? != GaragePatch::Original)
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the decal changer patch.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn set_enabled(cooked: &Path, backups: &Path, enable: bool) -> Result<bool, String> {
    ensure_closed()?;
    backup_guard::check_install(cooked, backups)?;
    let live = game_path::resolve(cooked, GARAGE.package);
    let original = std::fs::read(&live).map_err(|e| e.to_string())?;
    let mut garage = UpkPackage::from_bytes(original.clone())?;
    let movie = GARAGE.movie(&garage)?;
    let installed = inspect(&movie)?;
    let goal = if enable {
        GaragePatch::Current
    } else {
        GaragePatch::Original
    };
    let marker = backups.join("DecalStudioBridge");
    if installed != goal {
        let (edited, _) = title_patcher::movie::rewrite(&movie, &|data| {
            if let GaragePatch::Previous(delta) = installed {
                let Some((original, _)) = apply_delta(data, delta)? else {
                    return Ok(None);
                };
                if enable {
                    apply_delta(&original, INSTALL)
                } else {
                    Ok(Some((original, 1)))
                }
            } else {
                apply_delta(data, if enable { INSTALL } else { RESTORE })
            }
        })?
        .ok_or("Could not find the expected garage ActionScript")?;
        GARAGE.replace_movie(&mut garage, &edited)?;
        let staged = garage.repacked_fast()?;
        if inspect(&GARAGE.movie(&UpkPackage::from_bytes(staged.clone())?)?)? != goal {
            return Err("Staged garage package failed verification".into());
        }
        if enable {
            let tagame = UpkPackage::load(&game_path::resolve(cooked, "TAGame.upk"))?;
            if !instant_suite::has_native_bridge(&tagame) {
                std::fs::create_dir_all(backups).map_err(|e| e.to_string())?;
                upk_history::replace(&marker, b"Decal Studio installed the shared Shell bridge")?;
                instant_suite::install_bridge(cooked, true)?;
            }
        }
        ensure_closed()?;
        upk_history::install_linked_from(cooked, &live, &original, &staged, "Live decal changer")?;
    }
    if !enable && marker.is_file() {
        if !instant_suite::status(cooked, backups) {
            instant_suite::install_bridge(cooked, false)?;
        }
        std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    }
    Ok(enable)
}

pub(crate) fn preview(cooked: &Path, product: Option<i32>) -> Result<(), String> {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return Err("Launch Rocket League without EAC and open Customize Car first.".into());
    }
    match hebnix_sdk::log::easy_anti_cheat_enabled() {
        Some(false) => write_preview(cooked, product),
        Some(true) => Err("Rocket League was launched with Easy Anti-Cheat. Close it and choose Launch without EAC in Epic Games Launcher.".into()),
        None => Err("Waiting for Rocket League to report its launch mode. Open Customize Car and try again.".into()),
    }
}

fn write_preview(cooked: &Path, product: Option<i32>) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let value = product.map_or_else(|| "clear".into(), |id| id.to_string());
    let command = format!("DE:{}:{value}", now.as_millis());
    let path = cooked
        .parent()
        .ok_or("Cannot locate the TAGame folder")?
        .join("HbxInstant.txt");
    upk_history::replace(&path, command.as_bytes())
}

#[test]
#[ignore = "requires HEBNIX_DECAL_PROBE_COOKED to point at local Rocket League packages"]
fn staged_garage_roundtrip() {
    let source = std::path::PathBuf::from(std::env::var_os("HEBNIX_DECAL_PROBE_COOKED").unwrap());
    let root = std::env::temp_dir().join(format!("hebnix-live-decals-{}", rand::random::<u64>()));
    let cooked = root.join("TAGame").join("CookedPCConsole");
    let backups = cooked.join("Backups");
    let binaries = root.join("Binaries").join("Win64");
    std::fs::create_dir_all(&cooked).unwrap();
    std::fs::create_dir_all(&binaries).unwrap();
    std::fs::write(binaries.join("RocketLeague.exe"), b"stage fixture").unwrap();
    for file in [GARAGE.package, "TAGame.upk"] {
        std::fs::copy(game_path::resolve(&source, file), cooked.join(file)).unwrap();
    }
    status(&cooked).unwrap();
    assert!(set_enabled(&cooked, &backups, true).unwrap());
    assert!(status(&cooked).unwrap());
    let garage = UpkPackage::load(&cooked.join(GARAGE.package)).unwrap();
    assert!(inspect(&GARAGE.movie(&garage).unwrap()).unwrap() == GaragePatch::Current);
    write_preview(&cooked, Some(123)).unwrap();
    assert!(
        std::fs::read_to_string(root.join("TAGame").join("HbxInstant.txt"))
            .unwrap()
            .ends_with(":123")
    );
    assert!(!set_enabled(&cooked, &backups, false).unwrap());
    assert!(!status(&cooked).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
