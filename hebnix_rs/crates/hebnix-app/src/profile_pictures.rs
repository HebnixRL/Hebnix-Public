//! Profile picture resolution for the native avatar integration.
//!
//! The HTTP resolver lives separately from the game package patcher in
//! `crate::patcher::profile_pictures`.

mod service;
use eframe::egui;

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    enabled: bool,
    imported_owner: Option<String>,
}

enum Completion {
    Started(Option<service::Runtime>, String),
    Enabled(bool, Option<service::Runtime>, String),
    Imported(String),
    Uploaded(String),
}

pub(crate) struct State {
    root: PathBuf,
    settings: Settings,
    runtime: Option<service::Runtime>,
    pending: Option<mpsc::Receiver<Result<Completion, String>>>,
    initialized: bool,
    status: String,
    owner: Option<String>,
    last_account_check: Option<Instant>,
    preview: Option<egui::TextureHandle>,
}

fn epic_from_primary(primary: &str) -> Option<String> {
    let mut parts = primary.split('|');
    let platform = parts.next()?;
    let account = parts.next()?;
    if !platform.eq_ignore_ascii_case("epic")
        || !service::valid_epic(account)
        || parts.next()?.parse::<u8>().is_err()
        || parts.next().is_some()
    {
        return None;
    }
    Some(account.to_ascii_lowercase())
}
fn current_epic() -> Option<String> {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return None;
    }
    epic_from_primary(&hebnix_sdk::log::local_player_id()?)
}

impl State {
    pub(crate) fn new(base: &Path) -> Self {
        let root = base.join("profile_pictures");
        let settings = fs::read(root.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .unwrap_or_default();
        Self {
            root,
            settings,
            runtime: None,
            pending: None,
            initialized: false,
            status: String::new(),
            owner: None,
            last_account_check: None,
            preview: None,
        }
    }
    fn pictures(&self) -> PathBuf {
        self.root.join("pictures")
    }
    fn save(&mut self) {
        let result = (|| {
            fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
            fs::write(
                self.root.join("settings.json"),
                serde_json::to_vec_pretty(&self.settings).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            self.status = format!("Could not save profile picture settings: {error}");
        }
    }
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<Completion, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }
    pub(crate) fn tick(&mut self, ctx: &egui::Context, game: &Path) {
        if !self.initialized {
            self.initialized = true;
            if self.settings.enabled {
                let game = game.to_owned();
                let pictures = self.pictures();
                self.spawn(ctx, move || {
                    if !crate::patcher::profile_pictures::installed(&game)? {
                        return Ok(Completion::Started(None, "Native avatar hooks need updating. Close Rocket League and choose Update native avatar patch.".into()));
                    }
                    Ok(Completion::Started(Some(service::Runtime::start(&pictures)?), "Profile picture service is running.".into()))
                });
            }
        }
        if self
            .last_account_check
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3))
        {
            self.last_account_check = Some(Instant::now());
            let owner = current_epic();
            if self.owner != owner {
                self.owner = owner;
                self.preview = None;
            }
        }
        let result = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Profile picture operation stopped unexpectedly.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = result {
            self.pending = None;
            match result {
                Ok(Completion::Started(runtime, message)) => {
                    self.runtime = runtime;
                    self.status = message;
                }
                Ok(Completion::Enabled(enabled, runtime, message)) => {
                    self.settings.enabled = enabled;
                    if !enabled || runtime.is_some() {
                        self.runtime = runtime;
                    }
                    self.status = message;
                    self.save();
                }
                Ok(Completion::Imported(owner)) => {
                    self.settings.imported_owner = Some(owner);
                    self.preview = None;
                    if let Some(runtime) = &self.runtime {
                        runtime.invalidate();
                    }
                    self.status = "Picture imported locally. Use Upload to share it with other players. Restart Rocket League to refresh native avatars.".into();
                    self.save();
                }
                Ok(Completion::Uploaded(url)) => {
                    if let Some(runtime) = &self.runtime {
                        runtime.invalidate();
                    }
                    self.status = format!(
                        "Picture uploaded successfully. Restart Rocket League to refresh native avatars. {url}"
                    );
                }
                Err(error) => self.status = error,
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, game: &Path) {
        egui::ScrollArea::vertical().id_salt("profile_pictures_page").show(ui, |ui| {
            ui.heading("Profile Pictures");
            ui.label("Show player profile pictures in Rocket League menus and matches.");
            ui.add_space(12.0);
            let busy = self.pending.is_some();
            let running = hebnix_sdk::process::is_rocket_league_running();
            ui.label(if self.runtime.is_some() { "Picture service: running" } else { "Picture service: stopped" });
            let mut enabled = self.settings.enabled;
            if ui.add_enabled(!busy && !running, egui::Checkbox::new(&mut enabled, "Enable native profile pictures")).changed() {
                let game = game.to_owned();
                let pictures = self.pictures();
                // Keep the current listener until a successful disable. Enabling
                // reserves the port before changing the game package.
                let has_runtime = self.runtime.is_some();
                self.spawn(ui.ctx(), move || {
                    let runtime = if enabled && !has_runtime { Some(service::Runtime::start(&pictures)?) } else { None };
                    let message = crate::patcher::profile_pictures::set_enabled(&game, enabled)?;
                    Ok(Completion::Enabled(enabled, runtime, message))
                });
            }
            if running { ui.label("Close Rocket League to enable or disable the native avatar patch."); }
            if self.settings.enabled && !running && !busy && ui.button("Update native avatar patch").clicked() {
                let game = game.to_owned();
                let pictures = self.pictures();
                let has_runtime = self.runtime.is_some();
                self.spawn(ui.ctx(), move || {
                    let runtime = if !has_runtime { Some(service::Runtime::start(&pictures)?) } else { None };
                    let message = crate::patcher::profile_pictures::set_enabled(&game, true)?;
                    Ok(Completion::Enabled(true, runtime, message))
                });
            }
            if self.settings.enabled && self.runtime.is_none() && !busy && ui.button("Start picture service").clicked() {
                let game = game.to_owned();
                let pictures = self.pictures();
                self.spawn(ui.ctx(), move || {
                    if !crate::patcher::profile_pictures::installed(&game)? {
                        return Err("Close Rocket League and choose Update native avatar patch first.".into());
                    }
                    Ok(Completion::Started(Some(service::Runtime::start(&pictures)?), "Profile picture service is running.".into()))
                });
            }
            ui.label("Keep Hebnix running while playing. No Workshop menu or map is required.");
            ui.separator();
            ui.heading("Your Epic picture");
            if let Some(owner) = self.owner.clone() {
                ui.label(format!("Epic account: {owner}"));
                let path = self.pictures().join(format!("{owner}.jpg"));
                if self.preview.is_none() && path.is_file() {
                    if let Ok(image) = image::open(&path) {
                        let rgba = image.to_rgba8();
                        self.preview = Some(ui.ctx().load_texture("own_profile_picture",
                            egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()),
                            egui::TextureOptions::LINEAR));
                    }
                }
                if let Some(preview) = &self.preview { ui.image((preview.id(), egui::vec2(128.0, 128.0))); }
                if ui.add_enabled(!busy, egui::Button::new("Import picture…")).clicked() {
                    let pictures = self.pictures();
                    let owner = owner.clone();
                    self.spawn(ui.ctx(), move || {
                        let file = rfd::FileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                            .pick_file().ok_or("Import canceled.".to_string())?;
                        let mut bytes = Vec::new();
                        fs::File::open(file).map_err(|e| e.to_string())?.take(6 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                        let jpeg = service::normalize_image(&bytes)?;
                        if current_epic().as_deref() != Some(&owner) { return Err("The active Epic account changed. Import the picture again.".into()); }
                        fs::create_dir_all(&pictures).map_err(|e| e.to_string())?;
                        fs::write(pictures.join(format!("{owner}.jpg")), jpeg).map_err(|e| e.to_string())?;
                        Ok(Completion::Imported(owner))
                    });
                }
                let can_upload = !busy && self.settings.imported_owner.as_deref() == Some(&owner) && path.is_file();
                if ui.add_enabled(can_upload, egui::Button::new("Upload picture to Hebnix")).clicked() {
                    self.spawn(ui.ctx(), move || {
                        let bytes = fs::read(path).map_err(|e| e.to_string())?;
                        if current_epic().as_deref() != Some(&owner) { return Err("The active Epic account changed. Upload canceled.".into()); }
                        Ok(Completion::Uploaded(service::upload(&owner, bytes)?))
                    });
                }
                ui.label("Upload shares your imported picture through pubapi.hebnix.com for other players using Hebnix.");
            } else {
                ui.label("Start Rocket League on your Epic account to import or upload your picture.");
            }
            ui.label("Epic, Steam, PlayStation, Xbox and Switch pictures use req.hebnix.com. Accounts without a picture keep the normal icon.");
            if busy { ui.spinner(); }
            if !self.status.is_empty() { ui.add_space(8.0); ui.label(&self.status); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploads_require_an_epic_primary_id() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(epic_from_primary(&format!("Epic|{id}|0")), Some(id.into()));
        for value in [
            format!("Steam|{id}|0"),
            "Epic|../avatar|0".into(),
            format!("Epic|{id}|0|extra"),
        ] {
            assert_eq!(epic_from_primary(&value), None);
        }
    }
}
