//! Independent Freeplay settings, applied through the Workshop travel patch.
use super::lan_mutators;
use crate::patcher::{workshop_host, workshop_menu};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PRACTICE: &str =
    "?HbxSoloPractice?Playtest?NumPublicConnections=1?NumOpenPublicConnections=0?Lan?Listen";
const HEATSEEKER: &str = "TAGame.GameInfo_GodBall_TA";

#[derive(Default)]
pub(crate) struct State {
    install: PathBuf,
    enabled: bool,
    picks: Vec<usize>,
    status: String,
    session_check: Option<(Instant, SessionOrigin)>,
    patch_job: Option<std::sync::mpsc::Receiver<Result<bool, String>>>,
    package_stamp: Option<(u64, std::time::SystemTime)>,
    patch_probe: Option<std::sync::mpsc::Receiver<bool>>,
    patch_ready: Option<bool>,
}

fn launch_settings(picks: &[usize]) -> (String, &'static str) {
    let mut tags: Vec<_> = lan_mutators::travel_tags(picks)
        .filter(|tag| *tag != "FreePlay")
        .collect();
    if lan_mutators::heatseeker(picks) {
        tags.extend(["BotsNone", "PlayerCount2"]);
        if !tags
            .iter()
            .any(|tag| matches!(*tag, "10Minutes" | "20Minutes" | "UnlimitedTime"))
        {
            tags.push("UnlimitedTime");
        }
        return (tags.join(","), PRACTICE);
    }
    tags.insert(0, "Freeplay");
    (tags.join(","), "")
}

fn panel(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.heading(egui::RichText::new(title).size(22.0));
        ui.separator();
        content(ui);
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionOrigin {
    Freeplay,
    WorkshopOrLan,
    Other,
}

/// The marker survives travel into a listen server and identifies who launched it.
fn session_origin(current: &str) -> SessionOrigin {
    use SessionOrigin::*;
    let Some((map, query)) = current.split_once('?') else {
        return Other;
    };
    if map.is_empty() || map.eq_ignore_ascii_case("MENU_Main_p") {
        return Other;
    }
    let query: Vec<_> = query.split('?').collect();
    let has = |name: &str| query.iter().any(|part| part.eq_ignore_ascii_case(name));
    let option = |name: &str| {
        query
            .iter()
            .filter_map(|part| part.split_once('='))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    };
    let freeplay = option("GameTags").is_some_and(|tags| {
        tags.split(',')
            .any(|tag| tag.eq_ignore_ascii_case("Freeplay"))
    });
    let local_map = map
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        || (Path::new(map).is_absolute() && Path::new(map).is_file() && has("Workshop"));
    if local_map && has("HbxSoloPractice") {
        Freeplay
    } else if has("Workshop") || has("Lan") || has("Listen") {
        WorkshopOrLan
    } else if local_map && freeplay {
        Freeplay
    } else {
        Other
    }
}

pub(super) fn current_session_origin() -> SessionOrigin {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return SessionOrigin::Other;
    }
    hebnix_sdk::log::current_map_url()
        .as_deref()
        .map_or(SessionOrigin::Other, session_origin)
}

impl State {
    pub(crate) fn patch_busy(&self) -> bool {
        self.patch_job.is_some()
    }

    pub(crate) fn render(&mut self, ui: &mut egui::Ui, rl_path: &str) {
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        let previous_picks = self.picks.clone();
        panel(ui, "Freeplay", |ui| {
            let closed = cooked.is_dir() && !hebnix_sdk::process::is_rocket_league_running();
            let mut enabled = self.enabled && self.patch_ready != Some(false);
            if ui
                .add_enabled(
                    closed && !self.patch_busy(),
                    egui::Checkbox::new(&mut enabled, "Enable Freeplay Mutators"),
                )
                .on_disabled_hover_text(
                    "Close Rocket League before enabling or disabling the Freeplay patch.",
                )
                .changed()
            {
                let (tx, rx) = std::sync::mpsc::channel();
                let cooked = cooked.clone();
                let ctx = ui.ctx().clone();
                let (tags, options) = launch_settings(&self.picks);
                self.patch_job = Some(rx);
                self.status = "Updating Freeplay patch…".into();
                std::thread::spawn(move || {
                    let result =
                        workshop_host::set_freeplay_patch(&cooked, enabled).and_then(|()| {
                            workshop_menu::store_solo_preferences(
                                &cooked,
                                enabled,
                                &format!("{tags}{options}"),
                                !options.is_empty(),
                            )?;
                            Ok(enabled)
                        });
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
            }
            if self.enabled && self.patch_ready == Some(false) {
                ui.weak("TAGame.upk no longer contains the current Freeplay patch. Close Rocket League and enable Freeplay Mutators again.");
            }
            ui.add_enabled_ui(self.enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    lan_mutators::render_picks(ui, "freeplay-mode", &mut self.picks, |s| s.label == "Game Mode");
                    if lan_mutators::heatseeker(&self.picks) {
                        ui.weak("Use a Soccar arena and pick a team when the Heatseeker practice lobby opens.");
                    }
                });
            });
            let interval = Duration::from_millis(500);
            if self
                .session_check
                .is_none_or(|(checked, _)| checked.elapsed() >= interval)
            {
                self.session_check = Some((Instant::now(), current_session_origin()));
            }
            ui.ctx().request_repaint_after(interval);
            if self
                .session_check
                .is_some_and(|(_, origin)| origin == SessionOrigin::WorkshopOrLan)
            {
                ui.weak("This session uses Maps > In-game menu controls.");
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.enabled, egui::Button::new("Reset to defaults"))
                    .clicked()
                {
                    self.picks.fill(0);
                }
            });
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            panel(&mut columns[0], "Ball", |ui| {
                ui.add_enabled_ui(self.enabled, |ui| {
                    lan_mutators::render_picks(ui, "freeplay-ball", &mut self.picks, |s| {
                        s.label.starts_with("Ball") || s.label == "Number Of Balls"
                    });
                });
            });
            let rules = &mut columns[1];
            egui::ScrollArea::vertical()
                .id_salt("freeplay-rules-scroll")
                .max_height(rules.available_height())
                .show(rules, |ui| {
                    panel(ui, "Cars and Rules", |ui| {
                        ui.add_enabled_ui(self.enabled, |ui| {
                            lan_mutators::render_picks(
                                ui,
                                "freeplay-rules",
                                &mut self.picks,
                                |s| {
                                    !s.label.starts_with("Ball")
                                        && !matches!(
                                            s.label,
                                            "Game Mode"
                                                | "Free Play"
                                                | "Number Of Balls"
                                                | "Bot Loadouts"
                                        )
                                },
                            );
                        });
                    });
                });
        });
        if self.enabled && self.picks != previous_picks {
            let (tags, options) = launch_settings(&self.picks);
            self.status = match workshop_menu::store_solo_preferences(
                &cooked,
                true,
                &format!("{tags}{options}"),
                !options.is_empty(),
            ) {
                Ok(()) => "Changes saved for the next Freeplay launch.".into(),
                Err(error) => {
                    self.picks = previous_picks;
                    error
                }
            };
        }
    }

    pub(crate) fn poll_patch(&mut self, ctx: &egui::Context, rl_path: &str) {
        if let Some(job) = &self.patch_job {
            match job.try_recv() {
                Ok(result) => {
                    self.patch_job = None;
                    self.package_stamp = None;
                    self.patch_probe = None;
                    self.status = match result {
                        Ok(enabled) => {
                            self.enabled = enabled;
                            self.patch_ready = Some(enabled);
                            if enabled {
                                "Freeplay mutators enabled.".into()
                            } else {
                                "Freeplay mutators disabled.".into()
                            }
                        }
                        Err(error) => error,
                    };
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.patch_job = None;
                    self.status = "The Freeplay patch operation stopped.".into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let cooked = Path::new(rl_path).join("TAGame/CookedPCConsole");
        if self.install != cooked && !self.patch_busy() {
            self.install = cooked.clone();
            let (enabled, stored) = workshop_menu::read_solo_preferences(&cooked);
            self.enabled = enabled;
            (self.picks, _) =
                lan_mutators::parse(stored.split('?').next().unwrap_or_default().split(','));
            self.status.clear();
            self.session_check = None;
            self.package_stamp = None;
            self.patch_probe = None;
            self.patch_ready = None;
        }
        if self.enabled && !self.patch_busy() {
            let live = crate::patcher::game_path::resolve(&self.install, "TAGame.upk");
            let stamp = std::fs::metadata(live)
                .ok()
                .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
            if stamp != self.package_stamp {
                self.package_stamp = stamp;
                self.patch_ready = stamp.is_none().then_some(false);
                self.patch_probe = stamp.map(|_| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let cooked = self.install.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(workshop_host::freeplay_patch_intact(&cooked));
                        ctx.request_repaint();
                    });
                    rx
                });
            }
            if let Some(ready) = self
                .patch_probe
                .as_ref()
                .and_then(|job| job.try_recv().ok())
            {
                self.patch_ready = Some(ready);
                self.patch_probe = None;
            }
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeplay_apply_skips_other_sessions() {
        assert!(!State::default().enabled);
        let normal = "stadium_p?Game=TAGame.GameInfo_Soccar_TA?GameTags=Freeplay";
        assert!(session_origin(normal) == SessionOrigin::Freeplay);
        let practice = format!("stadium_p?Game={HEATSEEKER}{PRACTICE}");
        assert!(session_origin(&practice) == SessionOrigin::Freeplay);
        assert!(
            session_origin(&practice.replace("?NumOpenPublicConnections=0", ""))
                == SessionOrigin::Freeplay
        );
        assert!(
            session_origin(&practice.replace("?HbxSoloPractice", ""))
                == SessionOrigin::WorkshopOrLan
        );
        assert!(session_origin(&format!("{normal}?Workshop")) == SessionOrigin::WorkshopOrLan);
        assert!(session_origin(&format!("{normal}?Lan?Listen")) == SessionOrigin::WorkshopOrLan);
        assert!(session_origin(&format!("1.2.3.4:7777/{normal}")) == SessionOrigin::Other);
    }

    #[test]
    fn selecting_soccar_clears_heatseeker_launch_options() {
        let (mut picks, _) = lan_mutators::parse(["Ball_God"].into_iter());
        assert_eq!(launch_settings(&picks).1, PRACTICE);
        picks.fill(0);
        let (tags, options) = launch_settings(&picks);
        assert_eq!(tags, "Freeplay");
        assert!(options.is_empty());
    }
}
