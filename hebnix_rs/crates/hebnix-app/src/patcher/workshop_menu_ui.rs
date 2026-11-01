//! The game's own Flash menus, with the LAN controls added. The Workshop menu
//! gets a tickbox per map, a search box, and right-click / Y to host; the LAN
//! browser gets a sync button (right-click / Y) that fetches the hosted map.
//! A movie is stored as the RawData of an export in its GFX package. Only the
//! vanilla movie is patched; the original package is kept in the backups
//! folder and put back on restore.
use super::{game_path, upk_package::UpkPackage};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

struct Movie {
    package: &'static str,
    export: &'static str,
    /// the patched movie, built from the vanilla one (see assets/*/README.md)
    patched: &'static [u8],
    /// sha256 of the vanilla movie this was built from
    vanilla: &'static str,
}

const MOVIES: [Movie; 2] = [
    Movie {
        package: "GFX_WorkshopMenu_SF.upk",
        export: "GFX_WorkshopMenu.WorkshopMenu",
        patched: include_bytes!("../../assets/workshop-menu/workshop_menu_ui.swf"),
        vanilla: "c0fcdf7ae15e167708ce92a52a01fe9d448e3718231f793dc1f114b1c4aa0ec4",
    },
    Movie {
        package: "GFX_ServerBrowserMenu_SF.upk",
        export: "GFX_ServerBrowserMenu.ServerBrowserMenu",
        patched: include_bytes!("../../assets/lan-browser/lan_browser_ui.swf"),
        vanilla: "49cb7605a97ae0a5da10b975fca1220f0a61d281fca98c976c260613a65913ac",
    },
];

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

impl Movie {
    /// where the movie sits in the package: export index, property tag offset,
    /// start of the array data and its size
    fn locate(&self, upk: &UpkPackage) -> Result<(usize, usize, usize, usize), String> {
        let index = upk
            .find_object(self.export)
            .ok_or_else(|| format!("{} not found in {}", self.export, self.package))? as usize
            - 1;
        let (props, _) = upk.serialized_props(&upk.exports[index])?;
        let raw = props
            .iter()
            .find(|p| p.name == "RawData" && p.tag_type == "ArrayProperty")
            .ok_or_else(|| format!("{} has no RawData", self.package))?;
        if raw.value_offset != raw.tag_offset + 24 {
            return Err(format!("{} has an unexpected layout", self.package));
        }
        Ok((index, raw.tag_offset, raw.value_offset, raw.size))
    }

    fn movie(&self, upk: &UpkPackage) -> Result<Vec<u8>, String> {
        let (index, _, value, size) = self.locate(upk)?;
        let export = &upk.exports[index];
        let bytes = &upk.image[export.serial_offset..export.serial_offset + export.serial_size];
        bytes
            .get(value + 4..value + size)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("{} is truncated", self.package))
    }

    /// the package with the movie swapped, saved to a new file
    fn with_movie(&self, upk: &mut UpkPackage, swf: &[u8], out: &Path) -> Result<(), String> {
        let (index, tag, value, size) = self.locate(upk)?;
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
        upk.save_fast(out)
    }

    /// whether the live package holds the patched movie
    fn status(&self, cooked: &Path) -> bool {
        UpkPackage::load(&game_path::resolve(cooked, self.package))
            .and_then(|upk| self.movie(&upk))
            .is_ok_and(|swf| digest(&swf) == digest(self.patched))
    }

    /// not named like the package: the game scans subfolders and would load a
    /// same-named backup instead of the live file ("Ambiguous package name")
    fn backup_path(&self, backups: &Path) -> std::path::PathBuf {
        let old = backups.join(self.package);
        let backup = backups.join(format!("{}.bak", self.package));
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
    fn apply(&self, cooked: &Path, backups: &Path) -> Result<(), String> {
        let live = game_path::resolve(cooked, self.package);
        let mut upk = UpkPackage::load(&live)?;
        let backup = self.backup_path(backups);
        let current = digest(&self.movie(&upk)?);
        if current == digest(self.patched) {
            return Ok(());
        }
        if current != self.vanilla {
            return Err(format!(
                "the game's {} movie is not the version Hebnix patches (sha256 {current})",
                self.package
            ));
        }
        fs::create_dir_all(backups).map_err(|e| e.to_string())?;
        if !backup.is_file() {
            fs::copy(&live, &backup).map_err(|e| format!("Could not back up {}: {e}", self.package))?;
        }
        let temp = cooked.join(format!("{}.workshop.tmp", self.package));
        let result = self.with_movie(&mut upk, self.patched, &temp).and_then(|()| {
            let saved = UpkPackage::load(&temp)?;
            if digest(&self.movie(&saved)?) == digest(self.patched) {
                fs::copy(&temp, &live)
                    .map(|_| ())
                    .map_err(|e| format!("Could not write {}: {e}", self.package))
            } else {
                Err(format!("Patched {} failed validation", self.package))
            }
        });
        let _ = fs::remove_file(&temp);
        result
    }

    /// Put the original package back. Nothing to do when the movie is not ours.
    fn restore(&self, cooked: &Path, backups: &Path) -> Result<(), String> {
        let backup = self.backup_path(backups);
        if !self.status(cooked) {
            let _ = fs::remove_file(backup);
            return Ok(());
        }
        let original =
            UpkPackage::load(&backup).map_err(|_| format!("No backup of {} to restore", self.package))?;
        if digest(&self.movie(&original)?) != self.vanilla {
            return Err(format!("The backup of {} is not the vanilla movie", self.package));
        }
        fs::copy(&backup, game_path::resolve(cooked, self.package))
            .map_err(|e| format!("Could not restore {}: {e}", self.package))?;
        let _ = fs::remove_file(backup);
        Ok(())
    }
}

/// every menu is tried even when an earlier one fails; the errors come back together
fn each(op: impl Fn(&Movie) -> Result<(), String>) -> Result<(), String> {
    let errors: Vec<String> = MOVIES.iter().filter_map(|movie| op(movie).err()).collect();
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

pub fn apply(cooked: &Path, backups: &Path) -> Result<(), String> {
    each(|movie| movie.apply(cooked, backups))
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    each(|movie| movie.restore(cooked, backups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_form_ignores_magic_and_length() {
        for movie in &MOVIES {
            let mut gfx = movie.patched.to_vec();
            gfx[..3].copy_from_slice(b"GFX");
            gfx[4..8].copy_from_slice(&0u32.to_le_bytes());
            assert_eq!(digest(&gfx), digest(movie.patched));
        }
    }

    #[test]
    fn embedded_movies_are_not_the_vanilla_ones() {
        for movie in &MOVIES {
            assert_ne!(digest(movie.patched), movie.vanilla);
            assert_eq!(&movie.patched[..3], b"FWS");
        }
    }

    /// needs vanilla GFX_WorkshopMenu_SF.upk and GFX_ServerBrowserMenu_SF.upk:
    /// HEBNIX_VANILLA_UPKS points at the folder holding them
    #[test]
    #[ignore]
    fn apply_and_restore_on_a_copy() {
        let vanilla = std::env::var("HEBNIX_VANILLA_UPKS").expect("set HEBNIX_VANILLA_UPKS");
        let dir = std::env::temp_dir().join(format!("hebnix-ui-{}", std::process::id()));
        let backups = dir.join("Backups");
        fs::create_dir_all(&backups).unwrap();
        for movie in &MOVIES {
            fs::copy(Path::new(&vanilla).join(movie.package), dir.join(movie.package)).unwrap();
        }
        let originals: Vec<String> = MOVIES
            .iter()
            .map(|m| digest(&m.movie(&UpkPackage::load(&dir.join(m.package)).unwrap()).unwrap()))
            .collect();
        apply(&dir, &backups).unwrap();
        for movie in &MOVIES {
            assert!(movie.status(&dir));
            // a backup named like the package makes the game load it instead of the live file
            assert!(!backups.join(movie.package).exists());
            fs::copy(movie.backup_path(&backups), backups.join(movie.package)).unwrap();
        }
        apply(&dir, &backups).unwrap();
        for movie in &MOVIES {
            assert!(!backups.join(movie.package).exists());
        }
        restore(&dir, &backups).unwrap();
        for (movie, original) in MOVIES.iter().zip(originals) {
            assert!(!movie.status(&dir));
            assert_eq!(digest(&movie.movie(&UpkPackage::load(&dir.join(movie.package)).unwrap()).unwrap()), original);
        }
        let _ = fs::remove_dir_all(dir);
    }
}
