use super::HebnixApp;
use crate::i18n::t;
use crate::patcher::{backup_guard, game_update::Recovery};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct State {
    checked: Option<PathBuf>,
    next_check: Option<Instant>,
    pending: bool,
    recovery: Option<Recovery>,
    work: Option<VecDeque<Item>>,
    waiting: Option<Item>,
    error: Option<String>,
}

enum Item {
    Swap(serde_json::Value),
    Car(String, String),
    Decal(String, String),
    Ball(String),
    Boost(String),
    Anthems,
    BallVisual(usize, usize, bool),
    Alignment(String, crate::patcher::wheel_alignment::Settings),
    Names(String, Vec<crate::patcher::item_name::Patch>),
    Thumbnails(crate::patcher::thumbnail_patcher::Package),
}

impl HebnixApp {
    pub(super) fn check_game_update(&mut self) -> bool {
        let cooked = Path::new(&self.config.settings.rl_path).join("TAGame/CookedPCConsole");
        let state = &mut self.game_update;
        if state.checked.as_ref() != Some(&cooked) {
            *state = State::default();
            state.checked = Some(cooked.clone());
        }
        if state.pending || state.next_check.is_some_and(|at| at > Instant::now()) {
            return state.pending;
        }
        if self.patcher_ball.processing_target.is_some()
            || self.patcher_boost.processing_target.is_some()
            || self.patcher_decal.processing_target.is_some()
            || self.patcher_anthem.busy()
            || self.colours.is_busy()
            || self.wheel_alignment.busy()
            || self.speed_patch.busy()
        {
            return false;
        }
        self.game_update.next_check = Some(Instant::now() + Duration::from_secs(30));
        let result = Recovery::load(&cooked).and_then(|recovery| {
            if recovery.is_some() || backup_guard::update_required(&cooked)? {
                self.game_update.pending = true;
                self.game_update.recovery = recovery;
            }
            Ok(())
        });
        if let Err(error) = result {
            self.game_update.pending = true;
            self.game_update.error = Some(error);
        }
        self.game_update.pending
    }

    pub(super) fn render_game_update(&mut self, ctx: &egui::Context) {
        let cooked = Path::new(&self.config.settings.rl_path).join("TAGame/CookedPCConsole");
        let mut patch = false;
        let mut clear = false;
        let mut reload = false;
        let running = hebnix_sdk::process::is_rocket_league_running();
        egui::Modal::new(egui::Id::new("rocket_league_package_update")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("game-update-title"));
            ui.label(t("game-update-choice"));
            if running {
                ui.label(t("game-update-close-game"));
            }
            if let Some(error) = &self.game_update.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if self.game_update.work.is_some() {
                ui.spinner();
                ui.label(t("game-update-working"));
            } else {
                if !self.catalogs_loaded {
                    ui.horizontal(|ui| {
                        reload = ui.add_enabled(
                            !self.catalogs_loading,
                            egui::Button::new(t("game-update-reload-catalogs")),
                        ).clicked();
                        if self.catalogs_loading {
                            ui.spinner();
                            ui.label(t("game-update-loading-catalogs"));
                        }
                    });
                    if !self.catalogs_loading && let Some(error) = &self.catalogs_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                }
                ui.add_enabled_ui(!running, |ui| {
                    ui.horizontal(|ui| {
                        let clearing = self
                            .game_update
                            .recovery
                            .as_ref()
                            .is_some_and(|recovery| recovery.clearing);
                        patch = ui
                            .add_enabled(
                                !clearing && (!self.catalogs_loading || self.catalogs_loaded),
                                egui::Button::new(t("game-update-repatch")),
                            )
                            .clicked();
                        clear = ui.button(t("game-update-clear")).clicked();
                    });
                });
            }
        });

        if reload {
            self.reload_catalogs(ctx);
        }
        let result = if patch || clear {
            self.begin_game_update(&cooked, patch, ctx)
        } else if self.game_update.work.is_some() {
            self.continue_game_update(&cooked, ctx)
        } else {
            return;
        };
        if let Err(error) = result {
            self.game_update.error = Some(error);
            self.game_update.work = None;
            self.game_update.waiting = None;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn begin_game_update(
        &mut self,
        cooked: &Path,
        patch: bool,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.game_update.recovery.is_none() {
            let recovery = match Recovery::load(cooked)? {
                Some(recovery) => recovery,
                None => Recovery::capture(cooked, self.config.patcher.clone())?,
            };
            recovery.save(cooked)?;
            self.game_update.recovery = Some(recovery);
        }
        let mut recovery = self.game_update.recovery.take().unwrap();
        let result = (|| {
            if patch
                && !self.catalogs_loaded
                && (!recovery.swaps.is_empty()
                    || !recovery.items.active_cars.is_empty()
                    || !recovery.items.active_decals.is_empty())
            {
                if !self.catalogs_loading {
                    self.reload_catalogs(ctx);
                }
                self.game_update.error = None;
                return Ok(());
            }
            recovery.rebase_if_needed(cooked, patch)?;
            if !recovery.prepared {
                self.clear_game_update_items()?;
            }
            recovery.prepare(cooked, patch)?;
            if !patch {
                recovery.clearing = true;
                recovery.save(cooked)?;
                crate::patcher::uninstall::restore_repatched_items(&self.base_dir, cooked)?;
                self.clear_game_update_items()?;
                backup_guard::delete_backups(cooked)?;
                return self.finish_game_update(cooked);
            }
            let mut work = VecDeque::new();
            work.extend(
                recovery
                    .items
                    .active_cars
                    .iter()
                    .map(|(key, name)| Item::Car(key.clone(), name.clone())),
            );
            work.extend(
                recovery
                    .items
                    .active_decals
                    .iter()
                    .map(|(key, name)| Item::Decal(key.clone(), name.clone())),
            );
            work.extend(recovery.items.active_ball.iter().cloned().map(Item::Ball));
            work.extend(recovery.items.active_boost.iter().cloned().map(Item::Boost));
            work.extend(
                recovery
                    .alignments
                    .iter()
                    .cloned()
                    .map(|(name, settings)| Item::Alignment(name, settings)),
            );
            work.extend(
                recovery
                    .ball_visuals
                    .iter()
                    .map(|&(source, donor, colours)| Item::BallVisual(source, donor, colours)),
            );
            work.extend(
                recovery
                    .item_names
                    .iter()
                    .cloned()
                    .map(|(file, patches)| Item::Names(file, patches)),
            );
            work.extend(
                recovery
                    .thumbnails
                    .packages
                    .iter()
                    .cloned()
                    .map(Item::Thumbnails),
            );
            work.extend(recovery.swaps.iter().cloned().map(Item::Swap));
            if !recovery.items.active_anthems.is_empty() {
                work.push_back(Item::Anthems);
            }
            self.game_update.work = Some(work);
            self.game_update.error = None;
            Ok(())
        })();
        if self.game_update.pending {
            self.game_update.recovery = Some(recovery);
        }
        result
    }

    fn clear_game_update_items(&mut self) -> Result<(), String> {
        let items = &mut self.config.patcher;
        items.active_ball = None;
        items.active_boost = None;
        items.active_cars.clear();
        items.active_decals.clear();
        items.active_anthems.clear();
        self.patcher_ball.active_ball = None;
        self.patcher_boost.active_boost = None;
        self.patcher_car.active_cars.clear();
        self.patcher_decal.active_decals.clear();
        self.patcher_anthem.active_anthems.clear();
        self.presets.pending_ball = None;
        self.presets.pending_boost = None;
        self.presets.pending_decals.clear();
        self.presets.pending_cars = None;
        self.persist_game_update_items()
    }

    fn persist_game_update_items(&mut self) -> Result<(), String> {
        self.config
            .save(&self.base_dir)
            .map_err(|error| error.to_string())?;
        let root = Path::new(&self.config.settings.rl_path);
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "game_path": root,
            "patcher": self.config.patcher,
        }))
        .map_err(|error| error.to_string())?;
        crate::patcher::game_update::write_item_state(root, &bytes)
    }

    fn finish_game_update(&mut self, cooked: &Path) -> Result<(), String> {
        self.persist_game_update_items()?;
        Recovery::finish(cooked)?;
        self.game_update = State::default();
        Ok(())
    }

    fn continue_game_update(&mut self, cooked: &Path, ctx: &egui::Context) -> Result<(), String> {
        let recovery = self.game_update.recovery.as_ref().unwrap();
        if backup_guard::build_id(cooked)? != recovery.build {
            return Err("Rocket League changed again during repatching.".into());
        }
        if hebnix_sdk::process::is_rocket_league_running() {
            return Ok(());
        }
        self.patcher_ball.poll_ops(&self.tx, ctx, &mut self.config);
        self.patcher_boost.poll_ops(&self.tx, &mut self.config);
        self.patcher_decal
            .handle_op_result(&self.tx, &mut self.config);
        self.patcher_anthem.poll(&mut self.config, &self.tx);
        if self.patcher_ball.processing_target.is_some()
            || self.patcher_boost.processing_target.is_some()
            || self.patcher_decal.processing_target.is_some()
            || self.patcher_anthem.busy()
        {
            return Ok(());
        }
        let items = &self.config.patcher;
        let anthems = &recovery.items.active_anthems;
        let applied = |item: &Item| match item {
            Item::Ball(name) => items.active_ball.as_ref() == Some(name),
            Item::Boost(name) => items.active_boost.as_ref() == Some(name),
            Item::Car(key, name) => items.active_cars.get(key) == Some(name),
            Item::Decal(key, name) => items.active_decals.get(key) == Some(name),
            Item::Anthems => &items.active_anthems == anthems,
            Item::Swap(_)
            | Item::BallVisual(..)
            | Item::Alignment(..)
            | Item::Names(..)
            | Item::Thumbnails(_) => false,
        };
        if let Some(item) = self.game_update.waiting.take() {
            if !applied(&item) {
                return Err("An item could not be repatched. See the console for details, then retry or clear active items.".into());
            }
            self.persist_game_update_items()?;
            return Ok(());
        }
        let Some(item) = self.game_update.work.as_mut().unwrap().pop_front() else {
            if backup_guard::update_required(cooked)? {
                return Err("Rocket League files changed again during repatching. Verify the game files and retry.".into());
            }
            return self.finish_game_update(cooked);
        };
        if applied(&item) {
            return Ok(());
        }
        let backups = cooked.join("Backups");
        match &item {
            Item::Swap(value) => {
                let report =
                    self.swapper
                        .apply_preset_swaps(std::slice::from_ref(value), cooked, &backups);
                if report.applied != 1 {
                    return Err(report.skipped.join("\n"));
                }
                return Ok(());
            }
            Item::Car(key, name) => {
                self.patcher_car.apply_preset(
                    name,
                    key,
                    cooked,
                    &backups,
                    &self.tx,
                    &mut self.config,
                )?;
            }
            Item::Decal(key, name) => {
                let (car, skin) = key.split_once('|').ok_or("Invalid decal target")?;
                self.patcher_decal
                    .apply_decal_to_skin(name, car, skin, 1.0, cooked, &backups, &self.tx, ctx)?;
            }
            Item::Ball(name) => self
                .patcher_ball
                .begin_apply_named(name, cooked, &backups, &self.tx, ctx)?,
            Item::Boost(name) => self
                .patcher_boost
                .begin_apply_named(name, cooked, &backups, &self.tx, ctx)?,
            Item::Anthems => self.patcher_anthem.repatch(anthems.clone(), cooked, ctx),
            Item::Thumbnails(package) => {
                for patch in &package.patches {
                    crate::patcher::thumbnail_patcher::apply(
                        cooked,
                        &backups,
                        &package.file,
                        &patch.kind,
                        &patch.name,
                        &patch.png()?,
                    )?;
                }
                return Ok(());
            }
            Item::Alignment(file, settings) => {
                return crate::patcher::game_update::apply_alignment(cooked, file, *settings);
            }
            Item::Names(file, patches) => {
                return crate::patcher::game_update::apply_names(cooked, file, patches);
            }
            Item::BallVisual(source, donor, colours) => {
                if crate::patcher::ball_visual::active(&backups)?
                    .iter()
                    .any(|swap| swap.source == *source)
                {
                    return Ok(());
                }
                return crate::patcher::ball_visual::apply_with_colours(
                    cooked, &backups, *source, *donor, *colours,
                );
            }
        }
        self.game_update.waiting = Some(item);
        Ok(())
    }
}
