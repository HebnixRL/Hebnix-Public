//! Native profile pictures and the user's saved avatar gallery.
pub(crate) mod accounts;
mod gallery;
mod service;
pub(crate) use service::download_profile_avatar;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A picture upload that has not finished. It is retried once this old.
const UPLOAD_RETRY_SECS: u64 = 2 * 60 * 60;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    imported_owner: Option<String>,
    history: Vec<String>,
    selected: Option<String>,
    active: HashMap<String, String>,
    account: Option<String>,
    pending_upload: Option<PendingUpload>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PendingUpload {
    owner: String,
    id: String,
    /// Unix seconds when the upload was started.
    since: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn upload_due(pending: &PendingUpload, now: u64) -> bool {
    now.saturating_sub(pending.since) >= UPLOAD_RETRY_SECS
}

#[cfg(test)]
mod pending_upload_tests {
    use super::*;

    #[test]
    fn upload_is_due_only_after_two_hours() {
        let pending = PendingUpload { owner: "epic:a".into(), id: "x".into(), since: 1_000 };
        assert!(!upload_due(&pending, 1_000 + UPLOAD_RETRY_SECS - 1));
        assert!(upload_due(&pending, 1_000 + UPLOAD_RETRY_SECS));
    }
}
enum Completion {
    Started(bool, bool, Result<Option<service::Runtime>, String>),
    Enabled(bool, Option<service::Runtime>),
    Imported(String),
    Uploaded { owner: String, id: String },
    Canceled,
}
pub(crate) struct State {
    root: PathBuf,
    settings: Settings,
    runtime: Option<service::Runtime>,
    pending: Option<mpsc::Receiver<Result<Completion, String>>>,
    game: PathBuf,
    enabled: bool,
    needs_update: bool,
    status: String,
    error: bool,
    owner: Option<accounts::Account>,
    save_accounts: Vec<accounts::Account>,
    account_scan: Option<mpsc::Receiver<Vec<accounts::Account>>>,
    last_save_scan: Option<Instant>,
    active_scan: Option<mpsc::Receiver<(String, Result<String, String>)>>,
    looked_up: HashSet<String>,
    prune_pending: bool,
    last_account_check: Option<Instant>,
    textures: HashMap<String, egui::TextureHandle>,
}
impl State {
    pub(crate) fn new(base: &Path) -> Self {
        let root = base.join("profile_pictures");
        let mut settings = fs::read(root.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default();
        settings
            .history
            .retain(|id| gallery::path(&root, id).is_ok_and(|p| p.is_file()));
        settings.history.truncate(gallery::LIMIT);
        settings.active.retain(|key, id| {
            accounts::Account::from_key(key).is_some()
                && gallery::path(&root, id).is_ok_and(|p| p.is_file())
        });
        if settings.history.is_empty() && settings.active.is_empty() {
            if let Some(owner) = settings
                .imported_owner
                .as_ref()
                .filter(|id| service::valid_epic(id))
            {
                let old = root.join("pictures").join(format!("{owner}.jpg"));
                if let Ok(id) = gallery::import(&root, &old) {
                    settings.selected = Some(id.clone());
                    gallery::remember(&mut settings.history, id);
                }
            }
        }
        if let (Some(owner), Some(id)) = (&settings.imported_owner, &settings.selected) {
            if service::valid_epic(owner) && gallery::path(&root, id).is_ok_and(|p| p.is_file()) {
                settings
                    .active
                    .entry(format!("epic:{owner}"))
                    .or_insert_with(|| id.clone());
            }
        }
        settings.selected = None;
        settings.imported_owner = None;
        if settings
            .pending_upload
            .as_ref()
            .is_some_and(|p| !gallery::path(&root, &p.id).is_ok_and(|path| path.is_file()))
        {
            settings.pending_upload = None;
        }
        let mut state = Self {
            root,
            settings,
            runtime: None,
            pending: None,
            game: PathBuf::new(),
            enabled: false,
            needs_update: false,
            status: String::new(),
            error: false,
            owner: None,
            save_accounts: Vec::new(),
            account_scan: None,
            last_save_scan: None,
            active_scan: None,
            looked_up: HashSet::new(),
            prune_pending: false,
            last_account_check: None,
            textures: HashMap::new(),
        };
        if !state.settings.history.is_empty() || !state.settings.active.is_empty() {
            state.save();
        }
        state
    }
    fn pictures(&self) -> PathBuf {
        self.root.join("pictures")
    }
    fn save(&mut self) {
        let result = (|| {
            fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
            let temporary = self.root.join("settings.tmp");
            fs::write(
                &temporary,
                serde_json::to_vec_pretty(&self.settings).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            fs::rename(temporary, self.root.join("settings.json")).map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            self.status = format!("Could not save your pictures: {error}");
            self.error = true;
        } else {
            self.prune_pending = true;
            self.prune_gallery();
        }
    }
    fn prune_gallery(&mut self) {
        if self.prune_pending && self.pending.is_none() && self.active_scan.is_none() {
            let keep: Vec<_> = self
                .settings
                .history
                .iter()
                .chain(self.settings.active.values())
                .cloned()
                .collect();
            gallery::prune(&self.root, &keep);
            self.prune_pending = false;
        }
    }
    fn remove_recent(&mut self, id: &str) {
        self.settings.history.retain(|old| old != id);
        self.save();
    }
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        work: impl FnOnce() -> Result<Completion, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.status = label.into();
        self.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }
    fn set_enabled(&mut self, ctx: &egui::Context, game: &Path, enabled: bool) {
        let game = game.to_owned();
        let pictures = self.pictures();
        let has_runtime = self.runtime.is_some();
        self.spawn(ctx, "Updating avatar support…", move || {
            let runtime = if enabled && !has_runtime {
                Some(service::Runtime::start(&pictures)?)
            } else {
                None
            };
            crate::patcher::profile_pictures::set_enabled(&game, enabled)?;
            Ok(Completion::Enabled(enabled, runtime))
        });
    }
    pub(crate) fn tick(&mut self, ctx: &egui::Context, game: &Path) {
        if self.game != game && self.pending.is_none() {
            self.game = game.to_owned();
            self.enabled = false;
            self.runtime = None;
            self.needs_update = false;
            if !game.as_os_str().is_empty() {
                let game = game.to_owned();
                let pictures = self.pictures();
                self.spawn(ctx, "", move || {
                    let (on, needs_update) = crate::patcher::profile_pictures::status(&game)?;
                    let runtime = if on {
                        service::Runtime::start(&pictures).map(Some)
                    } else {
                        Ok(None)
                    };
                    Ok(Completion::Started(on, needs_update, runtime))
                });
            }
        }
        if self.account_scan.is_none()
            && self
                .last_save_scan
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.last_save_scan = Some(Instant::now());
            let previous = self.save_accounts.clone();
            let ctx = ctx.clone();
            let (tx, rx) = mpsc::channel();
            self.account_scan = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(accounts::discover(&previous));
                ctx.request_repaint();
            });
        }
        if let Some(accounts) = self.account_scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.account_scan = None;
            self.save_accounts = accounts;
            self.last_account_check = None;
        }
        if self
            .last_account_check
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3))
        {
            self.last_account_check = Some(Instant::now());
            let running = hebnix_sdk::process::is_rocket_league_running();
            let owner = accounts::choose(
                &self.save_accounts,
                self.settings.account.as_deref(),
                running,
                accounts::current(),
            );
            if self.owner.as_ref().map(|a| &a.key) != owner.as_ref().map(|a| &a.key)
                && self.pending.is_none()
            {
                self.status.clear();
                self.error = false;
            }
            self.owner = owner;
        }
        if self.pending.is_none() {
            if let (Some(pending), Some(owner)) = (self.settings.pending_upload.clone(), self.owner.clone()) {
                if pending.owner == owner.key && upload_due(&pending, now_secs()) {
                    self.upload(ctx, pending.id);
                }
            }
        }
        if let Some((key, picture)) = self.active_scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.active_scan = None;
            if !self.settings.active.contains_key(&key) {
                if let Ok(id) = picture {
                    self.settings.active.insert(key, id);
                    self.save();
                }
            }
        }
        if self.active_scan.is_none() {
            if let Some(account) = self
                .owner
                .as_ref()
                .filter(|a| {
                    !self.settings.active.contains_key(&a.key) && !self.looked_up.contains(&a.key)
                })
                .cloned()
            {
                self.looked_up.insert(account.key.clone());
                let (tx, rx) = mpsc::channel();
                self.active_scan = Some(rx);
                let root = self.root.clone();
                let pictures = self.pictures();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let local = account.picture_file(&pictures);
                    let legacy = pictures.join(format!("{}.jpg", account.id));
                    let picture = if local.is_file() {
                        gallery::import(&root, &local)
                    } else if account.platform == "epic" && legacy.is_file() {
                        gallery::import(&root, &legacy)
                    } else {
                        service::account_picture(&account.platform, &account.id)
                            .and_then(|jpeg| gallery::store_jpeg(&root, &jpeg))
                    };
                    let _ = tx.send((account.key, picture));
                    ctx.request_repaint();
                });
            }
        }
        let result = self.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("The operation stopped. Please try again.".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = result {
            self.pending = None;
            match result {
                Ok(Completion::Started(on, needs_update, runtime)) => {
                    self.enabled = on;
                    self.needs_update = needs_update;
                    match runtime {
                        Ok(runtime) => {
                            self.runtime = runtime;
                            if needs_update {
                                self.status = "Close Rocket League to update avatar support.".into();
                            }
                        }
                        Err(error) => {
                            self.error = true;
                            self.status = error;
                        }
                    }
                }
                Ok(Completion::Enabled(enabled, runtime)) => {
                    self.enabled = enabled;
                    if !enabled || runtime.is_some() {
                        self.runtime = runtime;
                    }
                    self.needs_update = false;
                    self.status.clear();
                    self.save();
                }
                Ok(Completion::Imported(id)) => {
                    gallery::remember(&mut self.settings.history, id);
                    self.textures
                        .retain(|id, _| self.settings.history.contains(id));
                    self.status = "Picture added.".into();
                    self.save();
                }
                Ok(Completion::Uploaded { owner, id }) => {
                    self.settings.pending_upload = None;
                    self.settings.active.insert(owner, id.clone());
                    gallery::remember(&mut self.settings.history, id);
                    if let Some(runtime) = &self.runtime {
                        runtime.invalidate();
                    }
                    self.status =
                        "Picture uploaded. Restart Rocket League to refresh your in-game avatar."
                            .into();
                    self.save();
                }
                Ok(Completion::Canceled) => self.status.clear(),
                Err(error) => {
                    self.error = true;
                    self.status = error;
                }
            }
        }
        self.prune_gallery();
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    fn upload(&mut self, ctx: &egui::Context, id: String) {
        let Some(owner) = self.owner.clone() else {
            return;
        };
        self.settings.pending_upload = Some(PendingUpload {
            owner: owner.key.clone(),
            id: id.clone(),
            since: now_secs(),
        });
        self.save();
        let root = self.root.clone();
        let pictures = self.pictures();
        self.spawn(ctx, "Uploading picture…", move || {
            let jpeg = gallery::read(&root, &id)?;
            accounts::authorize(&owner)?;
            service::upload(&owner.id, jpeg.clone())?;
            fs::create_dir_all(&pictures).map_err(|e| e.to_string())?;
            fs::write(owner.picture_file(&pictures), jpeg).map_err(|e| e.to_string())?;
            Ok(Completion::Uploaded {
                owner: owner.key,
                id,
            })
        });
    }
    fn texture(&mut self, ctx: &egui::Context, id: &str) -> Option<egui::TextureId> {
        if !self.textures.contains_key(id) {
            let path = gallery::path(&self.root, id).ok()?;
            let rgba = image::open(path).ok()?.to_rgba8();
            let texture = ctx.load_texture(
                format!("avatar-{id}"),
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            );
            self.textures.insert(id.into(), texture);
        }
        self.textures.get(id).map(|t| t.id())
    }
    fn active_picture(&self) -> Option<String> {
        self.owner
            .as_ref()
            .and_then(|a| self.settings.active.get(&a.key))
            .cloned()
    }
    fn account_picker(&mut self, ui: &mut egui::Ui, running: bool, busy: bool) {
        if !self.save_accounts.is_empty() {
            let mut key = self.owner.as_ref().map(|a| a.key.clone());
            let old = key.clone();
            let label = self
                .owner
                .as_ref()
                .map(|a| a.name.as_str())
                .unwrap_or("Waiting for Rocket League…");
            ui.add_enabled_ui(!running && !busy, |ui| {
                egui::ComboBox::from_id_salt("avatar_account")
                    .selected_text(label)
                    .width(ui.available_width().min(280.0))
                    .show_ui(ui, |ui| {
                        for account in &self.save_accounts {
                            let platform = if account.platform == "epic" {
                                "Epic"
                            } else {
                                "Steam"
                            };
                            ui.selectable_value(
                                &mut key,
                                Some(account.key.clone()),
                                format!("{} ({platform})", account.name),
                            );
                        }
                    });
            });
            if key != old {
                self.settings.account = key;
                self.owner = accounts::choose(
                    &self.save_accounts,
                    self.settings.account.as_deref(),
                    false,
                    None,
                );
                self.status.clear();
                self.error = false;
                self.save();
            }
        } else if let Some(account) = &self.owner {
            ui.label(&account.name);
        } else {
            ui.label("Open Rocket League to upload");
        }
    }
    fn show_recent(&mut self, ui: &mut egui::Ui) {
        let busy = self.pending.is_some();
        ui.set_max_width(400.0);
        ui.vertical(|ui| {
            ui.strong("Recent pictures (click one to upload)");
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Import picture…"))
                    .clicked()
                {
                    let root = self.root.clone();
                    self.spawn(ui.ctx(), "", move || {
                        let dialog = rfd::FileDialog::new()
                            .set_title("Import profile picture")
                            .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]);
                        let Some(file) = crate::winutil::parent_file_dialog(dialog).pick_file()
                        else {
                            return Ok(Completion::Canceled);
                        };
                        gallery::import(&root, &file).map(Completion::Imported)
                    });
                }
            });
        });
        ui.add_space(10.0);
        let size = ((ui.available_width() - 16.0) / 3.0).clamp(48.0, 120.0);
        let active = self.active_picture();
        let mut clicked = None;
        let mut removed = None;
        let history = self.settings.history.clone();
        egui::Grid::new("profile_picture_gallery")
            .num_columns(3)
            .striped(false)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                let mut count = 0;
                for id in &history {
                    let Some(texture) = self.texture(ui.ctx(), id) else {
                        continue;
                    };
                    ui.push_id(id, |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                        let sense = if !busy && self.owner.is_some() {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        };
                        let response = ui.interact(rect, ui.id().with("upload"), sense);
                        ui.painter().rect_filled(rect, 5.0, egui::Color32::BLACK);
                        ui.painter().image(
                            texture,
                            rect.shrink(4.0),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        if active.as_ref() == Some(id) {
                            ui.painter().rect_stroke(
                                rect.shrink(1.0),
                                5.0,
                                egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                                egui::StrokeKind::Inside,
                            );
                        } else if response.hovered() {
                            ui.painter().rect_stroke(
                                rect.shrink(1.0),
                                5.0,
                                ui.visuals().widgets.hovered.bg_stroke,
                                egui::StrokeKind::Inside,
                            );
                        }
                        let close = egui::Rect::from_min_size(
                            rect.right_top() + egui::vec2(-24.0, 4.0),
                            egui::vec2(20.0, 20.0),
                        );
                        let close_response = ui.interact(
                            close,
                            ui.id().with("remove"),
                            if busy {
                                egui::Sense::hover()
                            } else {
                                egui::Sense::click()
                            },
                        );
                        close_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                !busy,
                                "Remove from recent pictures",
                            )
                        });
                        let close_clicked = close_response.clicked();
                        if !busy
                            && (ui.rect_contains_pointer(rect)
                                || response.has_focus()
                                || close_response.has_focus())
                        {
                            ui.painter().rect_filled(
                                close,
                                4.0,
                                egui::Color32::from_black_alpha(220),
                            );
                            ui.painter().text(
                                close.center(),
                                egui::Align2::CENTER_CENTER,
                                "×",
                                egui::FontId::proportional(17.0),
                                egui::Color32::WHITE,
                            );
                            close_response.on_hover_text("Remove from recent pictures");
                        }
                        if close_clicked {
                            removed = Some(id.clone());
                        } else if response.clicked() {
                            clicked = Some(id.clone());
                        }
                    });
                    count += 1;
                    if count % 3 == 0 {
                        ui.end_row();
                    }
                }
            });
        if let Some(id) = removed {
            self.remove_recent(&id);
        } else if let Some(id) = clicked {
            self.upload(ui.ctx(), id);
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, game: &Path) {
        egui::ScrollArea::vertical()
            .id_salt("profile_pictures_page")
            .show(ui, |ui| crate::panel::followed(ui, |ui| {
                ui.heading("Profile Pictures");
                ui.label("Choose a picture to share with other Hebnix players.");
                ui.label(egui::RichText::new("Click a picture under Recent pictures to upload it as your avatar.").strong());
                ui.add_space(14.0);
                let busy = self.pending.is_some();
                let running = hebnix_sdk::process::is_rocket_league_running();
                ui.columns(2, |columns| {
                    let left = &mut columns[0];
                    let mut enabled = self.enabled;
                    if left
                        .add_enabled(
                            !busy && !running && !game.as_os_str().is_empty(),
                            egui::Checkbox::new(&mut enabled, "Show player profile pictures"),
                        )
                        .on_disabled_hover_text("Close Rocket League to change avatar support.")
                        .changed()
                    {
                        self.set_enabled(left.ctx(), game, enabled);
                    }
                    left.add_space(8.0);
                    self.account_picker(left, running, busy);
                    if self.enabled
                        && self.needs_update
                        && left
                            .add_enabled(
                                !busy && !running,
                                egui::Button::new("Update avatar support"),
                            )
                            .clicked()
                    {
                        self.set_enabled(left.ctx(), game, true);
                    }
                    if self.enabled
                        && self.runtime.is_none()
                        && !self.needs_update
                        && !busy
                        && left.button("Retry connection").clicked()
                    {
                        let pictures = self.pictures();
                        self.spawn(left.ctx(), "Connecting…", move || {
                            Ok(Completion::Started(
                                true,
                                false,
                                service::Runtime::start(&pictures).map(Some),
                            ))
                        });
                    }
                    if let Some(id) = self.active_picture() {
                        if let Some(texture) = self.texture(left.ctx(), &id) {
                            left.add_space(16.0);
                            left.strong("Current picture");
                            left.add_space(8.0);
                            let size = left.available_width().min(250.0);
                            let (picture, _) = left
                                .allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                            left.painter().image(
                                texture,
                                picture,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            left.painter().rect_stroke(
                                picture.expand(2.0),
                                4.0,
                                egui::Stroke::new(2.0, left.visuals().selection.bg_fill),
                                egui::StrokeKind::Outside,
                            );
                        }
                    }
                    self.show_recent(&mut columns[1]);
                });
                if self.pending.is_some() {
                    ui.add_space(8.0);
                    ui.spinner();
                }
                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    if self.error {
                        ui.colored_label(ui.visuals().error_fg_color, &self.status);
                    } else {
                        ui.label(&self.status);
                    }
                }
            }));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_pictures_are_per_account_and_removal_only_changes_recents() {
        let root =
            std::env::temp_dir().join(format!("hebnix-account-pictures-{}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let mut state = State::new(&root);
        let mut ids = Vec::new();
        for color in [20, 220] {
            let source = root.join("source.png");
            image::RgbImage::from_pixel(250, 250, image::Rgb([color, 0, 0]))
                .save(&source)
                .unwrap();
            ids.push(gallery::import(&state.root, &source).unwrap());
        }
        let first = accounts::Account::from_key("epic:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let second = accounts::Account::from_key("steam:76561198000000000").unwrap();
        state.settings.history = ids.clone();
        state
            .settings
            .active
            .insert(first.key.clone(), ids[0].clone());
        state
            .settings
            .active
            .insert(second.key.clone(), ids[1].clone());
        state.owner = Some(first.clone());
        assert_eq!(state.active_picture(), Some(ids[0].clone()));
        state.owner = Some(second.clone());
        assert_eq!(state.active_picture(), Some(ids[1].clone()));
        state.remove_recent(&ids[1]);
        assert_eq!(state.settings.history, vec![ids[0].clone()]);
        assert_eq!(state.active_picture(), Some(ids[1].clone()));
        assert!(gallery::path(&state.root, &ids[1]).unwrap().is_file());
        let mut reopened = State::new(&root);
        reopened.owner = Some(second);
        assert_eq!(reopened.active_picture(), Some(ids[1].clone()));
        assert_eq!(reopened.settings.history, vec![ids[0].clone()]);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn gallery_survives_a_restart_with_legacy_settings() {
        let root =
            std::env::temp_dir().join(format!("hebnix-gallery-settings-{}", rand::random::<u64>()));
        let profiles = root.join("profile_pictures");
        fs::create_dir_all(profiles.join("pictures")).unwrap();
        let owner = "0123456789abcdef0123456789abcdef";
        image::RgbImage::from_pixel(128, 128, image::Rgb([0, 255, 0]))
            .save(profiles.join("pictures").join(format!("{owner}.jpg")))
            .unwrap();
        fs::write(
            profiles.join("settings.json"),
            serde_json::json!({"enabled":false,"imported_owner":owner}).to_string(),
        )
        .unwrap();
        let state = State::new(&root);
        assert_eq!(state.settings.history.len(), 1);
        assert_eq!(
            state.settings.active.get(&format!("epic:{owner}")),
            state.settings.history.first()
        );
        let restored = State::new(&root);
        assert_eq!(restored.settings.history, state.settings.history);
        assert_eq!(restored.settings.selected, state.settings.selected);
        fs::remove_dir_all(root).unwrap();
    }
}
