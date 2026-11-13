//! The game's own Flash menus, with the LAN controls added. The Workshop menu
//! gets a tickbox per map, a search box, and right-click / Y to host; the LAN
//! browser gets a sync button (right-click / Y) that fetches the hosted map.
//! A movie is stored as the RawData of an export in its GFX package. Only the
//! vanilla movie is patched; the original package is kept in the backups
//! folder and put back on restore.
use super::{backup_guard, game_path, upk_package::UpkPackage};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub(super) struct Movie {
    pub(super) package: &'static str,
    pub(super) export: &'static str,
    /// the patched movie, built from the vanilla one (see assets/*/README.md)
    pub(super) patched: &'static [u8],
    /// sha256 of the vanilla movie this was built from
    pub(super) vanilla: &'static str,
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

pub(super) fn managed_movie(file: &str, upk: &UpkPackage, index: usize) -> Option<&'static Movie> {
    MOVIES.iter().find(|movie| {
        file.eq_ignore_ascii_case(movie.package)
            && upk.find_object(movie.export) == i32::try_from(index + 1).ok()
    })
}

/// the movie as the uncompressed FWS form, with its length field matching its size
fn normal(swf: &[u8]) -> Vec<u8> {
    let mut swf = swf.to_vec();
    if swf.len() >= 8 {
        if matches!(&swf[..3], b"CFX" | b"CWS") {
            swf[..3].copy_from_slice(b"CWS");
        } else {
            swf[..3].copy_from_slice(b"FWS");
            let len = swf.len() as u32;
            swf[4..8].copy_from_slice(&len.to_le_bytes());
        }
    }
    swf
}

pub(super) fn digest(swf: &[u8]) -> String {
    Sha256::digest(normal(swf))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Movie {
    fn menu_only(&self) -> Result<Option<&'static [u8]>, String> {
        if self.package != MOVIES[0].package {
            return Ok(None);
        }
        static MENU_ONLY: std::sync::OnceLock<Result<Vec<u8>, String>> = std::sync::OnceLock::new();
        MENU_ONLY
            .get_or_init(|| {
                super::title_patcher::movie::rewrite(
                    self.patched,
                    &super::title_patcher::abc::disable_workshop_downloader,
                )?
                .map(|(movie, _)| movie)
                .ok_or_else(|| "Workshop downloader method was not found".into())
            })
            .as_deref()
            .map(Some)
            .map_err(Clone::clone)
    }

    fn is_patched(&self, current: &str) -> bool {
        current == digest(self.patched)
            || self
                .menu_only()
                .ok()
                .flatten()
                .is_some_and(|swf| current == digest(swf))
    }

    pub(super) fn has_known_movie(&self, upk: &UpkPackage) -> bool {
        self.movie(upk).is_ok_and(|swf| {
            let current = digest(&swf);
            current == self.vanilla || self.is_patched(&current)
        })
    }

    /// Append-only updates can leave the original movie in the package.
    /// Only recover bytes that match this patch's exact vanilla digest.
    fn retained_vanilla(&self, upk: &UpkPackage) -> Option<Vec<u8>> {
        for (at, signature) in upk.image.windows(3).enumerate() {
            if signature != b"GFX" && signature != b"FWS" {
                continue;
            }
            let Some(header) = upk.image.get(at..at + 8) else {
                continue;
            };
            let size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
            if size < 8 {
                continue;
            }
            let Some(bytes) = at.checked_add(size).and_then(|end| upk.image.get(at..end)) else {
                continue;
            };
            if digest(bytes) == self.vanilla {
                return Some(bytes.to_vec());
            }
        }
        None
    }

    /// where the movie sits in the package: export index, property tag offset,
    /// start of the array data and its size
    fn locate(&self, upk: &UpkPackage) -> Result<(usize, usize, usize, usize), String> {
        let index = upk
            .find_object(self.export)
            .and_then(|object| usize::try_from(object).ok()?.checked_sub(1))
            .filter(|&index| index < upk.exports.len())
            .ok_or_else(|| format!("{} not found in {}", self.export, self.package))?;
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

    pub(super) fn movie(&self, upk: &UpkPackage) -> Result<Vec<u8>, String> {
        let (index, _, value, size) = self.locate(upk)?;
        let export = &upk.exports[index];
        let bytes = &upk.image[export.serial_offset..export.serial_offset + export.serial_size];
        bytes
            .get(value + 4..value + size)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("{} is truncated", self.package))
    }

    /// the package with the movie swapped, saved to a new file
    pub(super) fn with_movie(
        &self,
        upk: &mut UpkPackage,
        swf: &[u8],
        out: &Path,
    ) -> Result<(), String> {
        let (index, tag, value, size) = self.locate(upk)?;
        let export = &upk.exports[index];
        let old =
            upk.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec();
        let mut swf = normal(swf);
        let signature = if swf.starts_with(b"CWS") { b"CFX" } else { b"GFX" };
        swf[..3].copy_from_slice(signature);
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
    pub(super) fn status(&self, cooked: &Path) -> bool {
        UpkPackage::load(&game_path::resolve(cooked, self.package))
            .and_then(|upk| self.movie(&upk))
            .is_ok_and(|swf| self.is_patched(&digest(&swf)))
    }

    /// one marker per movie, so one movie's check can't vouch for the other's backup
    pub(super) fn marker(&self) -> String {
        format!("{}.build.sha256", self.package)
    }

    fn is_backup(&self, name: &str) -> bool {
        name == format!("{}.bak", self.package)
    }

    /// not named like the package: the game scans subfolders and would load a
    /// same-named backup instead of the live file ("Ambiguous package name")
    fn backup_path(&self, backups: &Path) -> std::path::PathBuf {
        let old = backups.join(self.package);
        let backup = backups.join(format!("{}.bak", self.package));
        if old.is_file() && !backup.is_file() {
            let _ = fs::rename(old, &backup);
        }
        backup
    }

    pub(super) fn clean_movie(&self, path: &Path) -> Option<Vec<u8>> {
        let package = UpkPackage::load(path).ok()?;
        let movie = self.movie(&package).ok()?;
        (digest(&movie) == self.vanilla).then_some(movie)
    }

    pub(super) fn recover_original(&self, cooked: &Path, backups: &Path, live: &UpkPackage) -> Option<Vec<u8>> {
        let name = format!("{}.bak", self.package);
        self.clean_movie(&backups.join(&name))
            .or_else(|| self.retained_vanilla(live))
            .or_else(|| {
                let recovery = super::game_update::Recovery::load(cooked).ok()??;
                self.clean_movie(&recovery.archive(cooked).join(&name))
            })
            .or_else(|| self.clean_movie(&super::title_patcher::original_movie_package(cooked, self.package)))
    }

    /// Recover only the movie, keeping the live package's other exports.
    pub(super) fn apply(&self, cooked: &Path, backups: &Path) -> Result<(), String> {
        let live = game_path::resolve(cooked, self.package);
        let mut upk = UpkPackage::load(&live)?;
        let backup = self.backup_path(backups);
        let current = self.movie(&upk)?;
        let current_hash = digest(&current);
        let desired = if super::workshop_menu::downloader_on(cooked) {
            self.patched
        } else {
            self.menu_only()?.unwrap_or(self.patched)
        };
        let already_current = current_hash == digest(desired);
        if already_current
            && backup_guard::check(cooked, backups, &self.marker(), |name| self.is_backup(name))
                .is_ok()
            && self.clean_movie(&backup).is_some()
        {
            return Ok(());
        }
        let vanilla = if current_hash == self.vanilla {
            Some(current)
        } else {
            self.clean_movie(&backup)
                .or_else(|| self.clean_movie(&backups.join(self.package)))
                .or_else(|| self.retained_vanilla(&upk))
                .or_else(|| {
                    let recovery = super::game_update::Recovery::load(cooked).ok()??;
                    if !self.is_patched(&current_hash) {
                        return None;
                    }
                    self.clean_movie(&recovery.archive(cooked).join(format!("{}.bak", self.package)))
                })
                .or_else(|| {
                    let (index, _, offset, _) = self.locate(&upk).ok()?;
                    let movie = super::title_patcher::legacy_workshop_movie(
                        cooked, self.package, &upk, index, offset,
                    )?;
                    (digest(&movie) == self.vanilla).then_some(movie)
                })
                .or_else(|| self.clean_movie(&super::title_patcher::original_movie_package(cooked, self.package)))
        }
        .ok_or_else(|| format!(
            "{} has no recoverable original movie compatible with this patch (installed movie SHA-256 {}). With Rocket League closed, verify the game files in your launcher, then use Update Workshop patch.",
            self.package, current_hash
        ))?;

        backup_guard::prepare(cooked, backups, &self.marker(), |name| self.is_backup(name))?;
        if self.clean_movie(&backup).is_none() {
            let temp = backup.with_extension("recovery.tmp");
            let saved = if current_hash == self.vanilla {
                fs::copy(&live, &temp)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                self.with_movie(&mut upk, &vanilla, &temp)
            };
            let result = saved.and_then(|()| {
                self.clean_movie(&temp)
                    .ok_or_else(|| format!("Recovered {} failed validation", self.package))?;
                fs::rename(&temp, &backup).map_err(|e| {
                    format!("Could not save the restore copy of {}: {e}", self.package)
                })
            });
            let _ = fs::remove_file(&temp);
            result?;
        }
        if already_current {
            Ok(())
        } else {
            self.install_movie(cooked, &mut upk, desired)
        }
    }

    pub(super) fn install_movie(
        &self,
        cooked: &Path,
        upk: &mut UpkPackage,
        desired: &[u8],
    ) -> Result<(), String> {
        let live = game_path::resolve(cooked, self.package);
        let temp = cooked.join(format!("{}.workshop.tmp", self.package));
        let before = fs::read(&live).map_err(|error| error.to_string())?;
        let result = self.with_movie(upk, desired, &temp).and_then(|()| {
            let saved = UpkPackage::load(&temp)?;
            if digest(&self.movie(&saved)?) == digest(desired) {
                if fs::read(&live).map_err(|error| error.to_string())? != before {
                    return Err(format!("{} changed while preparing the menu patch", self.package));
                }
                fs::rename(&temp, &live)
                    .map_err(|e| format!("Could not write {}: {e}", self.package))
            } else {
                Err(format!("Patched {} failed validation", self.package))
            }
        });
        let _ = fs::remove_file(&temp);
        result
    }

    /// Restore only the backed-up movie and keep other package exports.
    pub(super) fn restore(&self, cooked: &Path, backups: &Path) -> Result<(), String> {
        let backup = self.backup_path(backups);
        if !self.status(cooked) {
            return Ok(());
        }
        let mut live = UpkPackage::load(&game_path::resolve(cooked, self.package))?;
        let vanilla = self.recover_original(cooked, backups, &live)
            .ok_or_else(|| format!("No verified original menu for {} to restore", self.package))?;
        self.install_movie(cooked, &mut live, &vanilla)?;
        let _ = fs::remove_file(backup);
        let _ = fs::remove_file(backups.join(self.marker()));
        Ok(())
    }
}

pub(super) fn update_downloader(cooked: &Path, backups: &Path) -> Result<(), String> {
    let _guard = super::title_patcher::UI_PATCH_LOCK
        .lock()
        .map_err(|_| "UI patch lock unavailable")?;
    MOVIES[0].apply(cooked, backups)
}

/// every menu is tried even when an earlier one fails; the errors come back together
fn each(op: impl Fn(&Movie) -> Result<(), String>) -> Result<(), String> {
    let errors: Vec<String> = MOVIES.iter().filter_map(|movie| op(movie).err()).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub fn apply(cooked: &Path, backups: &Path) -> Result<(), String> {
    let _guard = super::title_patcher::UI_PATCH_LOCK
        .lock()
        .map_err(|_| "UI patch lock unavailable")?;
    each(|movie| movie.apply(cooked, backups))
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let _guard = super::title_patcher::UI_PATCH_LOCK
        .lock()
        .map_err(|_| "UI patch lock unavailable")?;
    each(|movie| movie.restore(cooked, backups))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validate an actual legacy package without modifying the installation.
    #[test]
    #[ignore]
    fn legacy_browser_upgrade_and_restore() {
        let source = std::env::var("HEBNIX_LEGACY_BROWSER").unwrap();
        let movie = &MOVIES[1];
        let dir =
            std::env::temp_dir().join(format!("hebnix-browser-upgrade-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let live = dir.join(movie.package);
        fs::copy(source, &live).unwrap();
        let package = UpkPackage::load(&live).unwrap();
        assert!(movie.retained_vanilla(&package).is_some());
        let backups = dir.join("Backups");
        movie.apply(&dir, &backups).unwrap();
        assert!(movie.status(&dir));
        movie.apply(&dir, &backups).unwrap();
        movie.restore(&dir, &backups).unwrap();
        let restored = UpkPackage::load(&live).unwrap();
        assert_eq!(digest(&movie.movie(&restored).unwrap()), movie.vanilla);
        fs::remove_dir_all(dir).unwrap();
    }

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
            fs::copy(
                Path::new(&vanilla).join(movie.package),
                dir.join(movie.package),
            )
            .unwrap();
        }
        let originals: Vec<String> = MOVIES
            .iter()
            .map(|m| {
                digest(
                    &m.movie(&UpkPackage::load(&dir.join(m.package)).unwrap())
                        .unwrap(),
                )
            })
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
            assert_eq!(
                digest(
                    &movie
                        .movie(&UpkPackage::load(&dir.join(movie.package)).unwrap())
                        .unwrap()
                ),
                original
            );
        }
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
#[test]
fn workshop_movie_has_distinct_downloader_and_menu_only_variants() {
    let movie = &MOVIES[0];
    let menu_only = movie.menu_only().unwrap().unwrap();
    assert_ne!(digest(menu_only), digest(movie.patched));
    assert!(movie.is_patched(&digest(menu_only)));
    assert!(movie.is_patched(&digest(movie.patched)));
    assert!(MOVIES[1].menu_only().unwrap().is_none());
}
