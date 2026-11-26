use crate::patcher::{game_path, title_patcher::{abc, movie}, upk_package::UpkPackage, workshop_menu_ui::Movie};
use std::path::Path;
type R<T> = Result<T, String>;
pub const FILES: &[(&str, &str)] = &[("GFX_Hud_SF.upk", "GFX_Hud.HUD"), ("GFX_WinnerMenu_SF.upk", "GFX_WinnerMenu.WinnerMenu")];
pub fn prepare(cooked: &Path, file: &str, baseline: Option<&[u8]>) -> R<(String, Vec<u8>, Vec<u8>)> {
    let (_, export) = FILES.iter().find(|(name, _)| *name == file).ok_or("Unknown live movie package")?;
    let mut package = UpkPackage::load(&game_path::resolve(cooked, file))?;
    let index = package.find_object(export).filter(|i| *i > 0).ok_or("Missing live movie export")? as usize - 1;
    if let Some(baseline) = baseline { package.replace_export_payload(index, baseline)?; }
    let entry = &package.exports[index];
    let before = package.image[entry.serial_offset..entry.serial_offset + entry.serial_size].to_vec();
    let descriptor = if file == FILES[0].0 {
        Movie { package: FILES[0].0, export: FILES[0].1, patched: include_bytes!("../../../assets/chat-paste/chat_paste_ui.swf"), vanilla: "a3011bd5268df871dc8fca3c7d7145269aa841cb91cc8ef4ff3ca7a6de3d33ac" }
    } else {
        Movie { package: FILES[1].0, export: FILES[1].1, patched: include_bytes!("../../../assets/instant-suite/winner_menu_ui.swf"), vanilla: "28f28cd0d1f94f3d4926a381a903514b4b14d69b90ad73a1676cd1b19cf60069" }
    };
    let current = descriptor.movie(&package)?;
    let (contains_hook, mut desired) = if file == FILES[0].0 {
        let found = std::cell::Cell::new(false);
        movie::rewrite(&current, &|data| { if abc::has_chat_paste(data)? { found.set(true); } Ok(None) })?;
        (found.get(), current.clone())
    } else {
        let found = std::cell::Cell::new(false);
        movie::rewrite(&current, &|data| { if abc::instant_suite_hook(data)?.is_some() { found.set(true); } Ok(None) })?;
        (found.get(), current.clone())
    };
    if !contains_hook {
        if crate::patcher::workshop_menu_ui::digest(&current) == descriptor.vanilla { desired = descriptor.patched.to_vec(); }
        else {
            let original = descriptor.recover_original(cooked, &cooked.join("Backups"), &package).ok_or("Cannot identify the original movie for live Game Mods")?;
            desired = if file == FILES[0].0 { movie::install_chat_paste(&current, &original, descriptor.patched)? }
                else { movie::apply_template(&current, &original, descriptor.patched)? }.ok_or("Live movie code block not found")?;
        }
    }
    if file == FILES[0].0 {
        if let Some((updated, _)) = movie::rewrite(&desired, &abc::live_chat_paste)? { desired = updated; }
    } else if let Some((updated, _)) = movie::rewrite(&desired, &|data| abc::instant_suite_bridge(data, true))? { desired = updated; }
    descriptor.replace_movie(&mut package, &desired)?;
    let entry = &package.exports[index];
    let after = package.image[entry.serial_offset..entry.serial_offset + entry.serial_size].to_vec();
    Ok((export.to_string(), before, after))
}

/// Rebase only the journal-owned movie delta onto a changed export. Named ABC
/// blocks and their template edits are verified; unrelated tags/blocks survive.
pub fn rebase_owned(
    package: &mut UpkPackage, file: &str, before: &[u8], after: &[u8],
) -> R<(Vec<u8>, Vec<u8>)> {
    let (file, path) = FILES.iter().find(|(name, _)| *name == file).copied().ok_or("Unknown live movie")?;
    let descriptor = Movie { package: file, export: path, patched: &[], vanilla: "" };
    let index = package.find_object(path).filter(|i| *i > 0).ok_or("Missing live movie export")? as usize - 1;
    let entry = &package.exports[index];
    let current = package.image[entry.serial_offset..entry.serial_offset + entry.serial_size].to_vec();
    let result = (|| {
        package.replace_export_payload(index, before)?;
        let stock_movie = descriptor.movie(package)?;
        package.replace_export_payload(index, after)?;
        let live_movie = descriptor.movie(package)?;
        package.replace_export_payload(index, &current)?;
        let current_movie = descriptor.movie(package)?;
        let disabled = movie::apply_template(&current_movie, &live_movie, &stock_movie)?.unwrap_or_else(|| current_movie.clone());
        let enabled = movie::apply_template(&current_movie, &stock_movie, &live_movie)?.unwrap_or(current_movie);
        let mut exports = Vec::new();
        for desired in [disabled, enabled] {
            package.replace_export_payload(index, &current)?;
            descriptor.replace_movie(package, &desired)?;
            let entry = &package.exports[index];
            exports.push(package.image[entry.serial_offset..entry.serial_offset + entry.serial_size].to_vec());
        }
        Ok((exports.remove(0), exports.remove(0)))
    })();
    // The caller stages either variant only after journalling both results.
    package.replace_export_payload(index, &current)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_hud_recovery_preserves_an_independent_movie_tag() {
        use std::io::Read;
        let before = include_bytes!("../../../assets/chat-paste/chat_paste_ui.swf");
        let (after, _) = movie::rewrite(before, &abc::live_chat_paste).unwrap().unwrap();
        let mut current = after[..8].to_vec();
        if matches!(&after[..3], b"CFX" | b"CWS") {
            flate2::read::ZlibDecoder::new(&after[8..]).read_to_end(&mut current).unwrap();
        } else { current.extend_from_slice(&after[8..]); }
        current[..3].copy_from_slice(b"FWS");
        let nbits = current[8] >> 3;
        let mut at = 8 + (5 + 4 * nbits as usize).div_ceil(8) + 4;
        loop {
            let tag_at = at;
            let tag = u16::from_le_bytes(current[at..at + 2].try_into().unwrap()); at += 2;
            let mut size = (tag & 63) as usize;
            if size == 63 { size = u32::from_le_bytes(current[at..at + 4].try_into().unwrap()) as usize; at += 4; }
            if tag >> 6 == 0 {
                let text = b"independent-hud-edit";
                let mut metadata = ((77u16 << 6) | text.len() as u16).to_le_bytes().to_vec();
                metadata.extend_from_slice(text);
                current.splice(tag_at..tag_at, metadata);
                break;
            }
            at += size;
        }
        let length = current.len() as u32; current[4..8].copy_from_slice(&length.to_le_bytes());
        let disabled = movie::apply_template(&current, &after, before).unwrap().unwrap();
        assert!(disabled.windows(b"independent-hud-edit".len()).any(|w| w == b"independent-hud-edit"));
        let enabled = movie::apply_template(&disabled, before, &after).unwrap().unwrap();
        assert_eq!(enabled, current);
    }

    #[test]
    fn chat_paste_template_has_live_gate() {
        let template = include_bytes!("../../../assets/chat-paste/chat_paste_ui.swf");
        let (patched, _) = movie::rewrite(template, &abc::live_chat_paste).unwrap().expect("chat hook");
        let contains = std::cell::Cell::new(false);
        movie::rewrite(&patched, &|data| { if data.windows(b"HEBNIX_LIVE_CHAT_PASTE".len()).any(|w| w == b"HEBNIX_LIVE_CHAT_PASTE") { contains.set(true); } Ok(None) }).unwrap();
        assert!(contains.get());
    }
}
