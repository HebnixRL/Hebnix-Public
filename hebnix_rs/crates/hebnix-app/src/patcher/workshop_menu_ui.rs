//! The Workshop menu's own Flash movie, with the LAN controls added: a tickbox
//! per map, a search box, and right-click / Y to host. The movie is stored as
//! the RawData of GFX_WorkshopMenu.WorkshopMenu in GFX_WorkshopMenu_SF.upk.
//! Only the vanilla movie is patched; the original package is kept in the
//! backups folder and put back on restore.
use super::{game_path, upk_package::UpkPackage};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const PACKAGE: &str = "GFX_WorkshopMenu_SF.upk";
const EXPORT: &str = "GFX_WorkshopMenu.WorkshopMenu";
/// the patched movie, built from the vanilla one (see assets/workshop-menu/README.md)
const PATCHED: &[u8] = include_bytes!("../../assets/workshop-menu/workshop_menu_ui.swf");
/// sha256 of the vanilla movie this was built from
const VANILLA: &str = "c0fcdf7ae15e167708ce92a52a01fe9d448e3718231f793dc1f114b1c4aa0ec4";

/// the movie as the uncompressed FWS form, with its length field matching its size
fn normal(swf: &[u8]) -> Vec<u8> {
    let mut swf = swf.to_vec();
    if swf.len() >= 8 {
        swf[..3].copy_from_slice(b"FWS");
        let len = swf.len() as u32;
        swf[4..8].copy_from_slice(&len.to_le_bytes());
    }
    swf
}

fn digest(swf: &[u8]) -> String {
    Sha256::digest(normal(swf)).iter().map(|b| format!("{b:02x}")).collect()
}

/// where the movie sits in the package: export index, property tag offset,
/// start of the array data and its size
fn locate(upk: &UpkPackage) -> Result<(usize, usize, usize, usize), String> {
    let index = upk
        .find_object(EXPORT)
        .ok_or_else(|| format!("{EXPORT} not found in {PACKAGE}"))? as usize
        - 1;
    let (props, _) = upk.serialized_props(&upk.exports[index])?;
    let raw = props
        .iter()
        .find(|p| p.name == "RawData" && p.tag_type == "ArrayProperty")
        .ok_or("The Workshop menu movie has no RawData")?;
    if raw.value_offset != raw.tag_offset + 24 {
        return Err("The Workshop menu movie has an unexpected layout".into());
    }
    Ok((index, raw.tag_offset, raw.value_offset, raw.size))
}

fn movie(upk: &UpkPackage) -> Result<Vec<u8>, String> {
    let (index, _, value, size) = locate(upk)?;
    let export = &upk.exports[index];
    let bytes = &upk.image[export.serial_offset..export.serial_offset + export.serial_size];
    bytes
        .get(value + 4..value + size)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "The Workshop menu movie is truncated".to_string())
}

/// the package with the movie swapped, saved to a new file
fn with_movie(upk: &mut UpkPackage, swf: &[u8], out: &Path) -> Result<(), String> {
    let (index, tag, value, size) = locate(upk)?;
    let export = &upk.exports[index];
    let old = upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec();
    let mut swf = normal(swf);
    swf[..3].copy_from_slice(b"GFX");
    let len = swf.len() as u32;
    let mut bytes = old[..value].to_vec();
    bytes[tag + 16..tag + 20].copy_from_slice(&(len + 4).to_le_bytes());
    bytes.extend(len.to_le_bytes());
    bytes.extend(swf);
    bytes.extend_from_slice(&old[value + size..]);
    upk.replace_title_export(index, &bytes)?;
    upk.save(out)
}

/// whether the live package holds the patched movie
pub fn status(cooked: &Path) -> bool {
    UpkPackage::load(&game_path::resolve(cooked, PACKAGE))
        .and_then(|upk| movie(&upk))
        .is_ok_and(|swf| digest(&swf) == digest(PATCHED))
}

/// not named like the package: the game scans subfolders and would load a
/// same-named backup instead of the live file ("Ambiguous package name")
fn backup_path(backups: &Path) -> std::path::PathBuf {
    let old = backups.join(PACKAGE);
    let backup = backups.join(format!("{PACKAGE}.bak"));
    if old.is_file() {
        if backup.is_file() {
            let _ = fs::remove_file(old);
        } else {
            let _ = fs::rename(old, &backup);
        }
    }
    backup
}

/// Put the patched movie in, keeping the original package in the backups
/// folder. A movie that is neither the vanilla one nor ours (a game update) is
/// left alone, since the patch was built against the vanilla one.
pub fn apply(cooked: &Path, backups: &Path) -> Result<(), String> {
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    let backup = backup_path(backups);
    let current = digest(&movie(&upk)?);
    if current == digest(PATCHED) {
        return Ok(());
    }
    if current != VANILLA {
        return Err(format!(
            "the game's Workshop menu movie is not the version Hebnix patches (sha256 {current})"
        ));
    }
    fs::create_dir_all(backups).map_err(|e| e.to_string())?;
    if !backup.is_file() {
        fs::copy(&live, &backup).map_err(|e| format!("Could not back up {PACKAGE}: {e}"))?;
    }
    let temp = cooked.join(format!("{PACKAGE}.workshop.tmp"));
    let result = with_movie(&mut upk, PATCHED, &temp).and_then(|()| {
        let saved = UpkPackage::load(&temp)?;
        if digest(&movie(&saved)?) == digest(PATCHED) {
            fs::copy(&temp, &live).map(|_| ()).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
        } else {
            Err(format!("Patched {PACKAGE} failed validation"))
        }
    });
    let _ = fs::remove_file(&temp);
    result
}

/// Put the original package back. Nothing to do when the movie is not ours.
pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    if !status(cooked) {
        let _ = fs::remove_file(backup_path(backups));
        return Ok(());
    }
    let backup = backup_path(backups);
    let original = UpkPackage::load(&backup).map_err(|_| format!("No backup of {PACKAGE} to restore"))?;
    if digest(&movie(&original)?) != VANILLA {
        return Err(format!("The backup of {PACKAGE} is not the vanilla movie"));
    }
    fs::copy(&backup, game_path::resolve(cooked, PACKAGE))
        .map_err(|e| format!("Could not restore {PACKAGE}: {e}"))?;
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_form_ignores_magic_and_length() {
        let mut gfx = PATCHED.to_vec();
        gfx[..3].copy_from_slice(b"GFX");
        gfx[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(digest(&gfx), digest(PATCHED));
    }

    #[test]
    fn embedded_movie_is_not_the_vanilla_one() {
        assert_ne!(digest(PATCHED), VANILLA);
        assert_eq!(&PATCHED[..3], b"FWS");
    }

    /// needs a vanilla GFX_WorkshopMenu_SF.upk: HEBNIX_VANILLA_UPK points at it
    #[test]
    #[ignore]
    fn apply_and_restore_on_a_copy() {
        let vanilla = std::env::var("HEBNIX_VANILLA_UPK").expect("set HEBNIX_VANILLA_UPK");
        let dir = std::env::temp_dir().join(format!("hebnix-ui-{}", std::process::id()));
        fs::create_dir_all(dir.join("Backups")).unwrap();
        fs::copy(vanilla, dir.join(PACKAGE)).unwrap();
        let original = digest(&movie(&UpkPackage::load(&dir.join(PACKAGE)).unwrap()).unwrap());
        apply(&dir, &dir.join("Backups")).unwrap();
        assert!(status(&dir));
        // a backup named like the package makes the game load it instead of the live file
        assert!(!dir.join("Backups").join(PACKAGE).exists());
        fs::copy(backup_path(&dir.join("Backups")), dir.join("Backups").join(PACKAGE)).unwrap();
        apply(&dir, &dir.join("Backups")).unwrap();
        assert!(!dir.join("Backups").join(PACKAGE).exists());
        restore(&dir, &dir.join("Backups")).unwrap();
        assert!(!status(&dir));
        assert_eq!(digest(&movie(&UpkPackage::load(&dir.join(PACKAGE)).unwrap()).unwrap()), original);
        let _ = fs::remove_dir_all(dir);
    }
}
