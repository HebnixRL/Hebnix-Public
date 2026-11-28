//! Inline title images read from the player's game, never bundled or patched.
use super::{
    cosmetic_thumbnail,
    upk_package::{UpkPackage, strip},
};
use eframe::egui;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub type Icons = HashMap<String, egui::TextureHandle>;
type Pixels = Vec<(String, image::RgbaImage)>;

fn token_for_asset(name: &str) -> Option<String> {
    let token = match name {
        "tournament_title_icon_bronze" => "Bronze",
        "tournament_title_icon_silver" => "Silver",
        "tournament_title_icon_gold" => "Gold",
        "tournament_title_icon_platinum" => "Platinum",
        "tournament_title_icon_diamond" => "Diamond",
        "tournament_title_icon_champion" => "Champion",
        "tournament_title_icon_grandChampion" => "GrandChampion",
        "tournament_title_icon_supersonicLegend" => "Legend",
        name if name.starts_with("flag_") => name,
        name if name.starts_with("TitleIcon_Stat") => &name[14..],
        name if name.starts_with("TitleIcon_") => &name[10..],
        _ => return None,
    };
    Some(format!("{{{token}}}"))
}

pub fn extract(install: &Path) -> Result<Pixels, String> {
    let directory = install.join("TAGame").join("CookedPCConsole");
    let package = UpkPackage::load(&directory.join("Startup.upk"))?;
    let mut icons = Vec::new();
    for export in &package.exports {
        let Some(token) = token_for_asset(strip(&package.name_of(export.object_name))) else {
            continue;
        };
        if !package.class_of(export).starts_with("Texture2D") {
            continue;
        }
        match cosmetic_thumbnail::texture(&package, export, &directory) {
            Ok(pixels) => icons.push((token, pixels)),
            Err(error) => tracing::debug!("Title icon {token}: {error}"),
        }
    }
    if icons.is_empty() {
        return Err("No inline title icons were readable in Startup.upk.".into());
    }
    Ok(icons)
}

#[derive(Default)]
struct Loader {
    install: Option<PathBuf>,
    result: Option<crossbeam_channel::Receiver<Result<Pixels, String>>>,
    icons: Arc<Icons>,
    #[cfg(windows)]
    web_icons: Arc<std::collections::BTreeMap<String, String>>,
}

fn key() -> egui::Id {
    egui::Id::new("hebnix_inline_title_icons")
}

/// Poll/upload only on the UI thread; package decoding runs once per install in
/// the background. One context-local cache serves every title preview.
pub fn prepare(ctx: &egui::Context, install: &Path) {
    let cache = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<Loader>>>(key())
            .clone()
    });
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.install.as_deref() != Some(install) {
        cache.install = Some(install.to_path_buf());
        cache.icons = Arc::default();
        #[cfg(windows)]
        { cache.web_icons = Arc::default(); }
        cache.result = None;
        if !install.as_os_str().is_empty() {
            let (tx, rx) = crossbeam_channel::bounded(1);
            cache.result = Some(rx);
            let install = install.to_path_buf();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(extract(&install));
                ctx.request_repaint();
            });
        }
    }
    if let Some(result) = cache.result.as_ref().and_then(|rx| rx.try_recv().ok()) {
        cache.result = None;
        match result {
            Ok(images) => {
                #[cfg(windows)]
                {
                    use base64::Engine;
                    cache.web_icons = Arc::new(images.iter().filter_map(|(token, image)| {
                        let pixels = image::imageops::thumbnail(image, 32, 32);
                        let png = cosmetic_thumbnail::preview_png(pixels).ok()?;
                        Some((token.clone(), format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))))
                    }).collect());
                }
                cache.icons = Arc::new(
                    images
                        .into_iter()
                        .map(|(token, image)| {
                            let size = [image.width() as usize, image.height() as usize];
                            let image =
                                egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                            let texture = ctx.load_texture(
                                format!("title-icon-{token}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            );
                            (token, texture)
                        })
                        .collect(),
                );
            }
            Err(error) => tracing::debug!("Inline title icons unavailable: {error}"),
        }
    }
}

#[cfg(windows)]
pub(crate) fn web_loaded(ctx: &egui::Context) -> Arc<std::collections::BTreeMap<String, String>> {
    ctx.data(|data| data.get_temp::<Arc<Mutex<Loader>>>(key()))
        .map(|cache| cache.lock().unwrap_or_else(|error| error.into_inner()).web_icons.clone())
        .unwrap_or_default()
}

pub fn loaded(ctx: &egui::Context) -> Arc<Icons> {
    ctx.data(|data| data.get_temp::<Arc<Mutex<Loader>>>(key()))
        .map(|cache| {
            cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .icons
                .clone()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_icon_tokens_match_game_symbols() {
        for (name, expected) in [
            ("TitleIcon_Train", "{Train}"),
            ("flag_au", "{flag_au}"),
            ("TitleIcon_StatAerialGoal", "{AerialGoal}"),
            ("tournament_title_icon_supersonicLegend", "{Legend}"),
        ] {
            assert_eq!(token_for_asset(name).as_deref(), Some(expected));
        }
        assert!(token_for_asset("Alerts_TitleStrip").is_none());
    }
}
