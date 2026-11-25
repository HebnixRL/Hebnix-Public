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
                else { movie::install_instant_suite(&current, &original, descriptor.patched)? }.ok_or("Live movie code block not found")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_paste_template_has_live_gate() {
        let template = include_bytes!("../../../assets/chat-paste/chat_paste_ui.swf");
        let (patched, _) = movie::rewrite(template, &abc::live_chat_paste).unwrap().expect("chat hook");
        let contains = std::cell::Cell::new(false);
        movie::rewrite(&patched, &|data| { if data.windows(b"HEBNIX_LIVE_CHAT_PASTE".len()).any(|w| w == b"HEBNIX_LIVE_CHAT_PASTE") { contains.set(true); } Ok(None) }).unwrap();
        assert!(contains.get());
    }
}
