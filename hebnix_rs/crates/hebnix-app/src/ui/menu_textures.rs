//! Experimental GFx menu texture replacement. Patches go through the thumbnail
//! patcher's snapshot store, under Backups/MenuTexturePatcher.
use super::*;
use crate::patcher::thumbnail_patcher::{self as patches, MenuTexture};
use crate::patcher::upk_package::strip;
use image::RgbaImage;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};

type Job<T> = Option<Receiver<Result<T, String>>>;

#[derive(Default)]
pub(super) struct MenuTextureState {
    install: PathBuf,
    files: Vec<String>,
    file: Option<String>,
    textures: Option<Result<Vec<MenuTexture>, String>>,
    loading: Job<Vec<MenuTexture>>,
    selected: HashSet<usize>,
    replacement: Option<(String, Arc<[u8]>)>,
    pack: Vec<(String, Arc<[u8]>)>,
    pack_search: String,
    tile: bool,
    keep_alpha: bool,
    manifest: patches::Manifest,
    status: Option<Result<String, String>>,
    pending: Job<patches::Manifest>,
    revision: u64,
}

fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (sender, receiver) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = sender.send(job());
        ctx.request_repaint();
    });
    receiver
}

fn poll<T>(job: &mut Job<T>) -> Option<Result<T, String>> {
    let result = match job.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
        Err(_) => Err("Menu texture worker stopped".into()),
    };
    *job = None;
    Some(result)
}

fn image_bytes(ui: &mut egui::Ui, uri: String, bytes: &Arc<[u8]>, size: f32) -> egui::Response {
    ui.add(
        egui::Image::from_bytes(uri, bytes.clone())
            .fit_to_exact_size(egui::vec2(size, size))
            .maintain_aspect_ratio(true)
            .texture_options(egui::TextureOptions::NEAREST)
            .sense(egui::Sense::click()),
    )
}

/// Block and GUI textures from a Minecraft resource pack zip.
fn read_pack(path: &Path) -> Result<Vec<(String, Arc<[u8]>)>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let Some(short) = ["block/", "gui/"].iter().find_map(|dir| {
            name.strip_prefix("assets/minecraft/textures/")
                .filter(|rest| rest.starts_with(dir) && rest.ends_with(".png"))
        }) else {
            continue;
        };
        if entry.size() > 4 * 1024 * 1024 {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|e| e.to_string())?;
        out.push((short.trim_end_matches(".png").to_string(), bytes.into()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Fit the source to a target texture. Pixel art scales with nearest filtering;
/// tiling repeats it at 4x like Minecraft's dirt backgrounds.
fn compose(source: &RgbaImage, original: Option<&RgbaImage>, size: (u32, u32), tile: bool) -> RgbaImage {
    use image::imageops::{FilterType, overlay, resize};
    let (w, h) = (size.0.clamp(4, 1024), size.1.clamp(4, 1024));
    // Animated Minecraft textures stack frames vertically; keep the first.
    let source = if source.height() > source.width() && source.height() % source.width() == 0 {
        image::imageops::crop_imm(source, 0, 0, source.width(), source.width()).to_image()
    } else {
        source.clone()
    };
    let filter = if source.width().max(source.height()) <= 128 {
        FilterType::Nearest
    } else {
        FilterType::Triangle
    };
    let mut out = if tile {
        let tile = resize(&source, source.width() * 4, source.height() * 4, filter);
        let mut out = RgbaImage::new(w, h);
        for y in (0..h).step_by(tile.height() as usize) {
            for x in (0..w).step_by(tile.width() as usize) {
                overlay(&mut out, &tile, x as i64, y as i64);
            }
        }
        out
    } else {
        resize(&source, w, h, filter)
    };
    if let Some(original) = original {
        let mask = resize(original, w, h, FilterType::Triangle);
        for (pixel, mask) in out.pixels_mut().zip(mask.pixels()) {
            pixel[3] = (pixel[3] as u16 * mask[3] as u16 / 255) as u8;
        }
    }
    out
}

impl HebnixApp {
    pub(super) fn render_menu_textures_page(&mut self, ui: &mut egui::Ui) {
        let cooked = PathBuf::from(&self.config.settings.rl_path)
            .join("TAGame")
            .join("CookedPCConsole");
        let backups = cooked.join("Backups");
        let state = &mut self.menu_textures;
        if let Some(result) = poll(&mut state.loading) {
            state.textures = Some(result);
        }
        if let Some(result) = poll(&mut state.pending) {
            state.status = Some(result.map(|_| "Saved. Restart Rocket League to see it.".into()));
            state.revision += 1;
            state.textures = None;
            state.manifest = patches::menu_load(&backups).unwrap_or_default();
        }
        if state.install != cooked && state.pending.is_none() {
            state.install = cooked.clone();
            state.keep_alpha = true;
            state.file = None;
            state.textures = None;
            state.files = fs::read_dir(&cooked)
                .map(|dir| {
                    dir.flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| {
                            let n = n.to_ascii_lowercase();
                            n.starts_with("gfx") && n.ends_with(".upk") && !n.starts_with("gfxsounds")
                        })
                        .collect()
                })
                .unwrap_or_default();
            state.files.sort_by_key(|n| n.to_ascii_lowercase());
            state.manifest = patches::menu_load(&backups).unwrap_or_default();
        }

        egui::ScrollArea::vertical()
            .id_salt("menu_textures_page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("Menu Textures (experimental)");
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Replaces Texture2D exports inside GFx menu UPKs. Close Rocket League first. Restore All undoes everything.",
                );
                if let Some(status) = &state.status {
                    match status {
                        Ok(message) => ui.label(message),
                        Err(error) => ui.colored_label(ui.visuals().error_fg_color, error),
                    };
                }
                if state.pending.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Patching…");
                    });
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    let label = state.file.clone().unwrap_or_else(|| "Choose a menu package".into());
                    egui::ComboBox::from_id_salt("menu_textures_file")
                        .selected_text(label)
                        .height(500.0)
                        .show_ui(ui, |ui| {
                            for file in &state.files {
                                let count = state
                                    .manifest
                                    .packages
                                    .iter()
                                    .find(|p| p.file.eq_ignore_ascii_case(file))
                                    .map_or(0, |p| p.patches.len());
                                let text = if count > 0 { format!("{file}  [{count} patched]") } else { file.clone() };
                                if ui.selectable_label(state.file.as_ref() == Some(file), text).clicked() {
                                    state.file = Some(file.clone());
                                    state.textures = None;
                                    state.selected.clear();
                                }
                            }
                        });
                    if ui.button("Reload").clicked() {
                        state.textures = None;
                    }
                    let active = state.manifest.count();
                    if ui
                        .add_enabled(active > 0, egui::Button::new(format!("Restore All ({active})")))
                        .clicked()
                    {
                        let (cooked, backups) = (cooked.clone(), backups.clone());
                        state.status = None;
                        state.pending = Some(spawn(ui.ctx(), move || patches::menu_restore_all(&cooked, &backups)));
                    }
                });
                ui.separator();

                // Replacement source.
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Replacement:");
                    if ui.button("Image file…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                            .pick_file()
                    {
                        match fs::read(&path) {
                            Ok(bytes) => {
                                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                                state.replacement = Some((name, bytes.into()));
                            }
                            Err(error) => state.status = Some(Err(error.to_string())),
                        }
                    }
                    if ui.button("Minecraft resource pack…").clicked() {
                        let mut dialog = rfd::FileDialog::new().add_filter("Resource pack", &["zip"]);
                        if let Some(dir) = std::env::var_os("APPDATA")
                            .map(|d| PathBuf::from(d).join("PrismLauncher").join("instances"))
                            .filter(|d| d.is_dir())
                        {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(path) = dialog.pick_file() {
                            match read_pack(&path) {
                                Ok(pack) => state.pack = pack,
                                Err(error) => state.status = Some(Err(error)),
                            }
                        }
                    }
                    if let Some((name, bytes)) = &state.replacement {
                        image_bytes(ui, format!("bytes://menu-replacement/{name}"), bytes, 48.0);
                        ui.label(name);
                    }
                    ui.checkbox(&mut state.tile, "Tile (4x)");
                    ui.checkbox(&mut state.keep_alpha, "Keep original alpha");
                });
                if !state.pack.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} pack textures", state.pack.len()));
                        ui.text_edit_singleline(&mut state.pack_search)
                            .on_hover_text("Filter, e.g. dirt, stone, planks, gui/title");
                    });
                    let search = state.pack_search.to_lowercase();
                    ui.horizontal_wrapped(|ui| {
                        for (name, bytes) in state.pack.iter().filter(|(n, _)| n.contains(&search)).take(80) {
                            if image_bytes(ui, format!("bytes://menu-pack/{name}"), bytes, 40.0)
                                .on_hover_text(name)
                                .clicked()
                            {
                                state.replacement = Some((name.clone(), bytes.clone()));
                            }
                        }
                    });
                }
                ui.separator();

                let Some(file) = state.file.clone() else {
                    ui.weak("Pick a GFx package. GFX_MainMenu_SF, GFX_StartMenu_SF and GFX_PlayMenu_SF are good starts.");
                    return;
                };
                if state.textures.is_none() && state.loading.is_none() {
                    let path = cooked.join(&file);
                    state.loading = Some(spawn(ui.ctx(), move || patches::menu_textures(&path)));
                }
                let Some(textures) = &state.textures else {
                    ui.spinner();
                    return;
                };
                let textures = match textures {
                    Ok(textures) => textures,
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        return;
                    }
                };
                let patched: HashSet<usize> = state
                    .manifest
                    .packages
                    .iter()
                    .filter(|p| p.file.eq_ignore_ascii_case(&file))
                    .flat_map(|p| p.patches.iter().map(|p| p.export))
                    .collect();
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} textures, {} selected", textures.len(), state.selected.len()));
                    if ui.button("Select all").clicked() {
                        state.selected = textures.iter().filter(|t| t.png.is_ok()).map(|t| t.export).collect();
                    }
                    if ui.button("Clear").clicked() {
                        state.selected.clear();
                    }
                    let ready = !state.selected.is_empty() && state.replacement.is_some();
                    if ui.add_enabled(ready, egui::Button::new("Apply to selected")).clicked() {
                        let (name, bytes) = state.replacement.clone().unwrap();
                        let jobs: Vec<_> = textures
                            .iter()
                            .filter(|t| state.selected.contains(&t.export))
                            .map(|t| (t.export, t.size, t.png.as_ref().ok().cloned()))
                            .collect();
                        let (tile, keep_alpha) = (state.tile, state.keep_alpha);
                        let (cooked, backups, file) = (cooked.clone(), backups.clone(), file.clone());
                        state.status = None;
                        state.pending = Some(spawn(ui.ctx(), move || {
                            let source = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
                            let mut changes = Vec::new();
                            for (export, size, original) in jobs {
                                let original = original
                                    .filter(|_| keep_alpha)
                                    .and_then(|png| image::load_from_memory(&png).ok())
                                    .map(|i| i.to_rgba8());
                                changes.push((export, name.clone(), compose(&source, original.as_ref(), size, tile)));
                            }
                            patches::menu_apply(&cooked, &backups, &file, changes)
                        }));
                    }
                    let restorable: Vec<usize> = state.selected.intersection(&patched).copied().collect();
                    if ui
                        .add_enabled(!restorable.is_empty(), egui::Button::new(format!("Restore selected ({})", restorable.len())))
                        .clicked()
                    {
                        let (cooked, backups, file) = (cooked.clone(), backups.clone(), file.clone());
                        state.status = None;
                        state.pending = Some(spawn(ui.ctx(), move || patches::menu_restore(&cooked, &backups, &file, &restorable)));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for texture in textures {
                        let selected = state.selected.contains(&texture.export);
                        let stroke = if selected {
                            egui::Stroke::new(2.0, ui.visuals().selection.stroke.color)
                        } else if patched.contains(&texture.export) {
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 200, 120))
                        } else {
                            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
                        };
                        egui::Frame::group(ui.style()).stroke(stroke).show(ui, |ui| {
                            ui.set_width(110.0);
                            ui.vertical(|ui| {
                                let clicked = match &texture.png {
                                    Ok(png) => {
                                        let bytes: Arc<[u8]> = png.as_slice().into();
                                        let uri = format!("bytes://menu-tex/{}/{}/{file}/{}", state.revision, crc32fast::hash(png), texture.export);
                                        image_bytes(ui, uri, &bytes, 100.0).clicked()
                                    }
                                    Err(error) => {
                                        ui.weak("unreadable").on_hover_text(error);
                                        false
                                    }
                                };
                                let name = strip(&texture.name).to_string();
                                ui.add(egui::Label::new(egui::RichText::new(&name).small()).truncate())
                                    .on_hover_text(format!("{name}\nexport #{}", texture.export));
                                ui.weak(egui::RichText::new(format!("{}x{}", texture.size.0, texture.size.1)).small());
                                if clicked && texture.png.is_ok() && !state.selected.remove(&texture.export) {
                                    state.selected.insert(texture.export);
                                }
                            });
                        });
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_tiles_crops_animation_and_keeps_alpha() {
        // 16x32 = two animation frames: red over blue.
        let mut source = RgbaImage::from_pixel(16, 32, image::Rgba([255, 0, 0, 255]));
        for y in 16..32 {
            for x in 0..16 {
                source.put_pixel(x, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        let mut original = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
        original.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let out = compose(&source, Some(&original), (256, 128), true);
        assert_eq!(out.dimensions(), (256, 128));
        assert_eq!(out.get_pixel(200, 100)[0], 255, "first frame only");
        assert_eq!(out.get_pixel(0, 0)[3], 0, "original alpha kept");
        assert_eq!(out.get_pixel(128, 64)[3], 255);
        let stretched = compose(&source, None, (4000, 2), false);
        assert_eq!(stretched.dimensions(), (1024, 4));
    }
}
