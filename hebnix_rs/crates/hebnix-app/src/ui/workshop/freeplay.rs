//! Independent Freeplay settings, applied through the Workshop travel patch.
use super::lan_mutators;
use crate::patcher::{workshop_host, workshop_menu};
use eframe::egui;
use hebnix_sdk::stats::WsStatsClient;
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
    restart: Option<Restart>,
    session_check: Option<(Instant, SessionOrigin)>,
    patch_job: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

struct Restart {
    cooked: PathBuf,
    target: String,
    deadline: Instant,
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
    hebnix_sdk::log::current_map_url().as_deref().map_or(SessionOrigin::Other, session_origin)
}

fn active_freeplay() -> Option<String> {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return None;
    }
    hebnix_sdk::log::current_map_url().filter(|url| session_origin(url) == SessionOrigin::Freeplay)
}

fn restart_target(current: &str, tags: &str, options: &str) -> Option<String> {
    let (map, query) = current.split_once('?')?;
    if map.contains(['"', '|', '\r', '\n', ';'])
        || (!options.is_empty() && query.split('?').any(|part| part.eq_ignore_ascii_case("Workshop")))
    {
        return None;
    }
    let game = if options.is_empty() {
        "TAGame.GameInfo_Soccar_TA"
    } else {
        HEATSEEKER
    };
    let mut target = format!("{map}?Game={game}?GameTags={tags}{options}");
    for flag in ["easy", "Workshop"] {
        if query.split('?').any(|part| part.eq_ignore_ascii_case(flag)) {
            target.push('?');
            target.push_str(flag);
        }
    }
    let target = if map.contains(char::is_whitespace) {
        format!("\"{target}\"")
    } else {
        target
    };
    let command = if options.is_empty() { "Open" } else { "Start" };
    Some(format!("{command} {target}"))
}

impl State {
    pub(crate) fn patch_busy(&self) -> bool {
        self.patch_job.is_some()
    }

    pub(crate) fn render(&mut self, ui: &mut egui::Ui, rl_path: &str, stats: &WsStatsClient) {
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        if self.install != cooked {
            self.install = cooked.clone();
            let (enabled, stored) = workshop_menu::read_solo_preferences(&cooked);
            self.enabled = enabled;
            (self.picks, _) =
                lan_mutators::parse(stored.split('?').next().unwrap_or_default().split(','));
            self.status.clear();
            self.session_check = None;
        }
        panel(ui, "Freeplay", |ui| {
            ui.horizontal(|ui| {
                let closed = cooked.is_dir() && !hebnix_sdk::process::is_rocket_league_running();
                for (label, enabled) in [("Install or update Freeplay patch", true), ("Remove Freeplay patch", false)] {
                    if ui.add_enabled(closed && !self.patch_busy(), egui::Button::new(label))
                        .on_disabled_hover_text("Close Rocket League before changing the Freeplay patch.")
                        .clicked()
                    {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let cooked = cooked.clone();
                        let ctx = ui.ctx().clone();
                        self.patch_job = Some(rx);
                        self.status = "Updating Freeplay patch…".into();
                        std::thread::spawn(move || {
                            let _ = tx.send(workshop_host::set_freeplay_patch(&cooked, enabled));
                            ctx.request_repaint();
                        });
                    }
                }
            });
            if ui
                .checkbox(&mut self.enabled, "Enable Freeplay Mutators")
                .changed()
            {
                self.status = "Apply to use this change.".into();
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
            let can_apply = self.restart.is_none()
                && cooked.is_dir()
                && self.session_check.is_some_and(|(_, origin)| origin == SessionOrigin::Freeplay);
            if self.session_check.is_some_and(|(_, origin)| origin == SessionOrigin::WorkshopOrLan) {
                ui.weak("This session uses Maps > In-game menu controls.");
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_apply, egui::Button::new("Apply and restart Freeplay"))
                    .on_disabled_hover_text(
                        "Open local Freeplay in Rocket League to apply changes.",
                    )
                    .clicked()
                {
                    self.status = self.apply(&cooked, stats).unwrap_or_else(|error| error);
                    ui.ctx().request_repaint();
                }
                if ui
                    .add_enabled(self.enabled, egui::Button::new("Reset to defaults"))
                    .clicked()
                {
                    self.picks.fill(0);
                    self.status = "Defaults selected. Apply to use them.".into();
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
    }

    fn apply(&mut self, cooked: &Path, stats: &WsStatsClient) -> Result<String, String> {
        let current =
            active_freeplay().ok_or("Open local Freeplay in Rocket League to apply changes.")?;
        if !workshop_host::freeplay_ready(cooked) {
            return Err(
                "Close Rocket League and select Install or update Freeplay patch on this tab."
                    .into(),
            );
        }
        let (tags, options) = launch_settings(&self.picks);
        let (active_tags, active_options) = if self.enabled {
            (tags.as_str(), options)
        } else {
            ("Freeplay", "")
        };
        let target = restart_target(&current, active_tags, active_options)
            .ok_or("Use a standard Soccar arena for Heatseeker, or choose another Freeplay map.")?;
        workshop_menu::store_solo_preferences(
            cooked,
            self.enabled,
            &format!("{tags}{options}"),
            lan_mutators::heatseeker(&self.picks),
        )?;
        let disconnect_sent = stats.connected()
            && workshop_host::request_freeplay_travel(cooked, stats, "Disconnect").is_ok();
        let now = Instant::now();
        self.restart = Some(Restart {
            cooked: cooked.to_owned(),
            target,
            deadline: now + Duration::from_secs(30),
        });
        Ok(if disconnect_sent {
            "Restarting Freeplay..."
        } else {
            "Settings saved. Leave the practice match through Rocket League’s menu to continue the restart."
        }.into())
    }

    pub(crate) fn poll_restart(
        &mut self,
        ctx: &egui::Context,
        stats: &WsStatsClient,
        rl_path: &str,
    ) {
        if let Some(job) = &self.patch_job {
            match job.try_recv() {
                Ok(result) => {
                    self.patch_job = None;
                    self.status = result.map(|()| "Freeplay patch updated.".into()).unwrap_or_else(|error| error);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.patch_job = None;
                    self.status = "The Freeplay patch operation stopped.".into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(restart) = &self.restart else { return };
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        if Instant::now() >= restart.deadline
            || cooked != restart.cooked
            || !hebnix_sdk::process::is_rocket_league_running()
        {
            self.restart = None;
            self.status =
                "Freeplay restart stopped. Reopen Freeplay to use the saved settings.".into();
            return;
        }
        let menu_ready = hebnix_sdk::log::map_load_finished("MENU_Main_p");
        if menu_ready && stats.connected() {
            let result =
                workshop_host::request_freeplay_travel(&restart.cooked, stats, &restart.target);
            self.restart = None;
            self.status = result
                .map(|_| "Freeplay reload requested.".into())
                .unwrap_or_else(|error| error);
        } else {
            if restart.deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(27) {
                self.status = if menu_ready {
                    "Waiting for Rocket League’s Stats API to reconnect.".into()
                } else {
                    "If the practice match is still open, leave it through Rocket League’s menu to continue the restart.".into()
                };
            }
            ctx.request_repaint_after(Duration::from_millis(250));
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
        assert!(session_origin(&practice.replace("?NumOpenPublicConnections=0", "")) == SessionOrigin::Freeplay);
        assert!(session_origin(&practice.replace("?HbxSoloPractice", "")) == SessionOrigin::WorkshopOrLan);
        assert!(session_origin(&format!("{normal}?Workshop")) == SessionOrigin::WorkshopOrLan);
        assert!(session_origin(&format!("{normal}?Lan?Listen")) == SessionOrigin::WorkshopOrLan);
        assert!(session_origin(&format!("1.2.3.4:7777/{normal}")) == SessionOrigin::Other);
    }

    #[test]
    fn heatseeker_restart_builds_a_local_lan_start() {
        let (picks, _) = lan_mutators::parse(["Ball_God", "UnlimitedTime"].into_iter());
        let (tags, options) = launch_settings(&picks);
        let current = "street_p?Game=TAGame.GameInfo_Soccar_TA?GameTags=Freeplay";
        let target = restart_target(current, &tags, options).unwrap();
        assert!(target.starts_with("Start street_p?Game=TAGame.GameInfo_GodBall_TA?GameTags=Ball_God,"));
        assert!(target.ends_with("?Lan?Listen"));
        assert!(!target.contains("GameTags=Freeplay"));
        assert_eq!(restart_target(current, "Freeplay", "").as_deref(), Some("Open street_p?Game=TAGame.GameInfo_Soccar_TA?GameTags=Freeplay"));
        let practice = target.strip_prefix("Start ").unwrap();
        assert!(session_origin(practice) == SessionOrigin::Freeplay);
        assert_eq!(restart_target(practice, "Freeplay", ""), restart_target(current, "Freeplay", ""));
    }
}
