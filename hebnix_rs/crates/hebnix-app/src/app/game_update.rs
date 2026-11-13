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
    paused: bool,
    failed: VecDeque<Item>,
    force_all: bool,
    recovery: Option<Recovery>,
    work: Option<VecDeque<Item>>,
    waiting: Option<Item>,
    waiting_for_catalogs: bool,
    refreshed_catalogs: bool,
    clear_requested: bool,
    finishing: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

#[derive(Clone)]
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
    pub(super) fn check_game_update(&mut self, ctx: &egui::Context) {
        let cooked = Path::new(&self.config.settings.rl_path).join("TAGame/CookedPCConsole");
        let state = &mut self.game_update;
        if state.checked.as_ref() != Some(&cooked) {
            *state = State::default();
            state.checked = Some(cooked.clone());
        }
        if state.pending && !state.paused {
            self.advance_game_update(&cooked, ctx);
            return;
        }
        if state.pending || state.next_check.is_some_and(|at| at > Instant::now()) {
            return;
        }
        if self.patch_jobs_busy() {
            return;
        }
        self.game_update.next_check = Some(Instant::now() + Duration::from_secs(30));
        match Recovery::load(&cooked).and_then(|recovery| {
            if recovery.is_some() || backup_guard::update_required(&cooked)? {
                Ok(Some(recovery))
            } else {
                Ok(None)
            }
        }) {
            Ok(Some(recovery)) => {
                self.game_update.pending = true;
                self.game_update.recovery = recovery;
                self.console.write("[Recovery] Rocket League files changed or a repatch is unfinished. Close Rocket League and type repatch in the Console.");
                self.game_update.paused = true;
            }
            Ok(None) => {}
            Err(error) => self.game_update_failed(error),
        }
    }

    pub(super) fn patch_jobs_busy(&self) -> bool {
        self.patcher_ball.processing_target.is_some()
            || self.patcher_boost.processing_target.is_some()
            || self.patcher_decal.processing_target.is_some()
            || self.patcher_anthem.busy()
            || self.colours.is_busy()
            || self.wheel_alignment.busy()
            || self.speed_patch.busy()
            || self.title_patcher.busy()
            || self.rank_patch.busy()
            || self.instant_suite.busy()
            || self.goal_explosion.busy()
            || self.goal_explosion_volume.busy()
            || self.forfeit_command.busy()
            || self.celebration.busy()
            || self.bots_freeplay.busy()
            || self.freeplay.patch_busy()
            || self.chat_export.busy()
            || self.lan_commands.busy()
            || self.chat_paste.busy()
            || self.swapper.busy()
            || self.scoreboard.busy()
            || self.profile_pictures.busy()
            || self.workshop.maintenance_busy()
            || self.replay_history_patch.busy()
    }

    pub(super) fn recovery_running(&self) -> bool {
        self.game_update.pending && !self.game_update.paused
    }

    pub(super) fn render_game_update_controls(&mut self, ui: &mut egui::Ui) {
        if !self.game_update.pending {
            return;
        }
        if self.recovery_running() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t(if self.game_update.waiting_for_catalogs {
                    "game-update-loading-catalogs"
                } else {
                    "game-update-working"
                }));
            });
            if hebnix_sdk::process::is_rocket_league_running() {
                ui.label(t("game-update-close-game"));
            }
            return;
        }
        ui.label(t("game-update-title"));
        let running = hebnix_sdk::process::is_rocket_league_running();
        if running {
            ui.label(t("game-update-close-game"));
        }
        ui.add_enabled_ui(!running, |ui| {
            ui.horizontal(|ui| {
                if ui.button(t("game-update-repatch")).clicked() {
                    self.repatch_from_console(ui.ctx(), false);
                }
                if ui.button(t("game-update-clear")).clicked() {
                    self.start_game_update(ui.ctx(), false, true);
                }
            });
        });
    }

    fn advance_game_update(&mut self, cooked: &Path, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_millis(200));
        if let Some(job) = &self.game_update.finishing {
            match job.try_recv() {
                Ok(Ok(())) => self.complete_game_update(),
                Ok(Err(error)) => {
                    self.game_update.finishing = None;
                    self.game_update_failed(error);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.game_update.finishing = None;
                    self.game_update_failed("The recovery worker stopped before finishing.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            return;
        }
        if hebnix_sdk::process::is_rocket_league_running() {
            return;
        }
        let result = if self.game_update.waiting_for_catalogs {
            if self.catalogs_loading {
                return;
            }
            self.game_update.waiting_for_catalogs = false;
            if self.catalogs_loaded && self.catalogs_error.is_none() {
                self.game_update.refreshed_catalogs = true;
                self.begin_game_update(cooked, true, ctx)
            } else {
                Err(self.catalogs_error.clone().unwrap_or_else(|| "Could not load item catalogs.".into()))
            }
        } else if self.game_update.work.is_some() {
            self.continue_game_update(cooked, ctx)
        } else {
            self.begin_game_update(cooked, !self.game_update.clear_requested, ctx)
        };
        if let Err(error) = result {
            self.game_update_failed(error);
        }
    }

    fn game_update_failed(&mut self, error: String) {
        self.game_update.pending = true;
        self.game_update.paused = true;
        self.console.write(format!("[Recovery] {error}"));
        self.console.write("[Recovery] Close Rocket League, then type repatch here to retry.");
    }

    pub(super) fn repatch_from_console(&mut self, ctx: &egui::Context, all: bool) {
        self.start_game_update(ctx, all, false);
    }

    fn start_game_update(&mut self, ctx: &egui::Context, all: bool, clear: bool) {
        if self.recovery_running() {
            return;
        }
        if hebnix_sdk::process::is_rocket_league_running() {
            self.console.write("[Recovery] Close Rocket League before running repatch.");
            return;
        }
        if self.patch_jobs_busy() {
            self.console.write("[Recovery] Wait for the current patch to finish.");
            return;
        }
        let cooked = Path::new(&self.config.settings.rl_path).join("TAGame/CookedPCConsole");
        if self.game_update.checked.as_ref() != Some(&cooked) {
            self.game_update = State::default();
            self.game_update.checked = Some(cooked.clone());
        }
        let resume = (|| -> Result<bool, String> {
            if !all && !clear && !self.game_update.pending
                && Recovery::load(&cooked)?.is_none()
                && !backup_guard::update_required(&cooked)?
            {
                self.console.write("[Recovery] No items need repatching.");
                return Ok(false);
            }
            self.game_update.pending = true;
            self.game_update.paused = false;
            self.game_update.clear_requested = clear
                || self.game_update.recovery.as_ref().is_some_and(|recovery| recovery.clearing)
                || Recovery::load(&cooked)?.is_some_and(|recovery| recovery.clearing);
            self.game_update.checked = Some(cooked.clone());
            self.game_update.waiting_for_catalogs = false;
            if !all && !self.game_update.clear_requested && self.game_update.work.is_some()
                && self.game_update.recovery.as_ref().is_some_and(|recovery| {
                    backup_guard::build_id(&cooked).is_ok_and(|build| build == recovery.build)
                })
                && !backup_guard::update_required(&cooked)?
            {
                return Ok(true);
            }
            self.game_update.work = None;
            self.game_update.waiting = None;
            self.game_update.failed.clear();
            self.game_update.force_all = all;
            self.game_update.refreshed_catalogs = false;
            if all && !self.game_update.clear_requested && self.game_update.recovery.is_none()
                && Recovery::load(&cooked)?.is_none()
                && !backup_guard::update_required(&cooked)?
            {
                let mut recovery = Recovery::capture(&cooked, self.config.patcher.clone())?;
                recovery.prepared = true;
                recovery.save(&cooked)?;
                self.game_update.recovery = Some(recovery);
            }
            Ok(true)
        })();
        match resume {
            Ok(true) => ctx.request_repaint(),
            Ok(false) => {}
            Err(error) => self.game_update_failed(error),
        }
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
                && !self.game_update.refreshed_catalogs
                && (!self.catalogs_loaded || !recovery.prepared)
                && (!recovery.swaps.is_empty()
                    || !recovery.items.active_cars.is_empty()
                    || !recovery.items.active_decals.is_empty()
                    || !recovery.items.active_anthems.is_empty())
            {
                if !self.catalogs_loading {
                    self.reload_catalogs(ctx);
                }
                self.game_update.waiting_for_catalogs = true;
                return Ok(());
            }
            recovery.rebase_if_needed(cooked, patch)?;
            recovery.prepare(cooked, patch)?;
            if recovery.reset_pending {
                self.clear_game_update_items()?;
                recovery.reset_pending = false;
                if let Err(error) = recovery.save(cooked) {
                    recovery.reset_pending = true;
                    return Err(error);
                }
            }
            if !patch {
                recovery.clearing = true;
                recovery.save(cooked)?;
                crate::patcher::uninstall::restore_repatched_items(&self.base_dir, cooked)?;
                self.clear_game_update_items()?;
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
        let cooked = cooked.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("game-update-recovery".into())
            .spawn(move || { let _ = tx.send(Recovery::finish(&cooked)); })
            .map_err(|error| error.to_string())?;
        self.game_update.finishing = Some(rx);
        Ok(())
    }

    fn complete_game_update(&mut self) {
        self.colours.invalidate_active();
        self.console.write(if self.game_update.clear_requested {
            "[Recovery] Active item patches cleared."
        } else {
            "[Recovery] Repatching completed."
        });
        self.game_update = State::default();
    }

    fn continue_game_update(&mut self, cooked: &Path, ctx: &egui::Context) -> Result<(), String> {
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
        let recovery = self.game_update.recovery.as_ref().unwrap();
        if backup_guard::build_id(cooked)? != recovery.build {
            return Err("Rocket League changed again during repatching.".into());
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
                self.record_failed_item(item, "The patch did not complete. See the preceding console output.");
                return Ok(());
            }
            self.persist_game_update_items()?;
            self.game_update.work.as_mut().unwrap().pop_front();
            return Ok(());
        }
        let Some(item) = self.game_update.work.as_ref().unwrap().front().cloned() else {
            if !self.game_update.failed.is_empty() {
                let failed = std::mem::take(&mut self.game_update.failed);
                let count = failed.len();
                self.game_update.work = Some(failed);
                return Err(format!("{count} item patch(es) failed. The other items have finished."));
            }
            if backup_guard::update_required(cooked)? {
                return Err("Rocket League files changed again during repatching. Verify the game files and retry.".into());
            }
            return self.finish_game_update(cooked);
        };
        if !self.game_update.force_all && applied(&item) {
            self.game_update.work.as_mut().unwrap().pop_front();
            return Ok(());
        }
        if let Err(error) = self.apply_recovery_item(cooked, ctx, item.clone()) {
            self.record_failed_item(item, &error);
        }
        Ok(())
    }

    fn record_failed_item(&mut self, item: Item, error: &str) {
        let name = match &item {
            Item::Swap(value) => value.get("target_upk").and_then(serde_json::Value::as_str).unwrap_or("Item swap"),
            Item::Car(_, name) | Item::Decal(_, name) | Item::Ball(name) | Item::Boost(name)
            | Item::Alignment(name, _) | Item::Names(name, _) => name,
            Item::Anthems => "Anthems",
            Item::BallVisual(..) => "Ball appearance",
            Item::Thumbnails(package) => &package.file,
        };
        self.console.write(format!("[Recovery] {name}: {error}"));
        self.game_update.work.as_mut().unwrap().pop_front();
        self.game_update.failed.push_back(item);
    }

    fn apply_recovery_item(&mut self, cooked: &Path, ctx: &egui::Context, item: Item) -> Result<(), String> {
        let backups = cooked.join("Backups");
        match &item {
            Item::Swap(value) => {
                let report =
                    self.swapper
                        .apply_preset_swaps(std::slice::from_ref(value), cooked, &backups);
                if report.applied != 1 {
                    return Err(report.skipped.join("\n"));
                }
                self.game_update.work.as_mut().unwrap().pop_front();
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
            Item::Anthems => {
                let anthems = self.game_update.recovery.as_ref().unwrap().items.active_anthems.clone();
                self.patcher_anthem.repatch(anthems, cooked, ctx);
            }
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
                self.game_update.work.as_mut().unwrap().pop_front();
                return Ok(());
            }
            Item::Alignment(file, settings) => {
                crate::patcher::game_update::apply_alignment(cooked, file, *settings, self.game_update.force_all)?;
                self.game_update.work.as_mut().unwrap().pop_front();
                return Ok(());
            }
            Item::Names(file, patches) => {
                crate::patcher::game_update::apply_names(cooked, file, patches)?;
                self.game_update.work.as_mut().unwrap().pop_front();
                return Ok(());
            }
            Item::BallVisual(source, donor, colours) => {
                if !self.game_update.force_all && crate::patcher::ball_visual::active(&backups)?
                    .iter()
                    .any(|swap| swap.source == *source)
                {
                    self.game_update.work.as_mut().unwrap().pop_front();
                    return Ok(());
                }
                crate::patcher::ball_visual::apply_with_colours(
                    cooked, &backups, *source, *donor, *colours,
                )?;
                self.game_update.work.as_mut().unwrap().pop_front();
                return Ok(());
            }
        }
        self.game_update.waiting = Some(item);
        Ok(())
    }
}
