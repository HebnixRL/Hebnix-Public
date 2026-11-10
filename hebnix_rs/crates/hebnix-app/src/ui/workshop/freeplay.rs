//! Independent Freeplay settings, applied through the Workshop travel patch.
use super::lan_mutators;
use crate::patcher::{workshop_host, workshop_menu};
use eframe::egui;
use hebnix_sdk::stats::WsStatsClient;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    session_check: Option<(Instant, bool)>,
}

struct Restart {
    cooked: PathBuf,
    menu: String,
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

/// Recognize local Freeplay and the solo Heatseeker session launched from it.
fn is_freeplay(current: &str) -> bool {
    let Some((map, query)) = current.split_once('?') else {
        return false;
    };
    if map.is_empty() || map.eq_ignore_ascii_case("MENU_Main_p") {
        return false;
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
    let practice = option("Game").is_some_and(|game| game.eq_ignore_ascii_case(HEATSEEKER))
        && has("HbxSoloPractice")
        && has("Lan")
        && has("Listen")
        && option("NumOpenPublicConnections") == Some("0");
    let local_map = map
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        || (Path::new(map).is_absolute() && Path::new(map).is_file() && has("Workshop"));
    local_map && (practice || (freeplay && !has("Lan") && !has("Listen")))
}

fn active_freeplay() -> Option<String> {
    if !hebnix_sdk::process::is_rocket_league_running() {
        return None;
    }
    hebnix_sdk::log::current_map_url().filter(|url| is_freeplay(url))
}

fn restart_target(current: &str, tags: &str, options: &str) -> Option<String> {
    let (map, query) = current.split_once('?')?;
    if map.contains(['"', '|', '\r', '\n', ';']) {
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
    Some(if map.contains(char::is_whitespace) {
        format!("\"{target}\"")
    } else {
        target
    })
}

impl State {
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
                self.session_check = Some((Instant::now(), active_freeplay().is_some()));
            }
            ui.ctx().request_repaint_after(interval);
            let can_apply = self.restart.is_none()
                && cooked.is_dir()
                && self.session_check.is_some_and(|(_, active)| active);
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
                "Close Rocket League and use Update Workshop patch to enable Freeplay restart."
                    .into(),
            );
        }
        if !stats.connected() {
            return Err("Connect Rocket League's Stats API to restart Freeplay.".into());
        }
        let (tags, options) = launch_settings(&self.picks);
        let (active_tags, active_options) = if self.enabled {
            (tags.as_str(), options)
        } else {
            ("Freeplay", "")
        };
        let target = restart_target(&current, active_tags, active_options)
            .ok_or("Could not prepare the current Freeplay map for restart.")?;
        workshop_menu::store_solo_preferences(
            cooked,
            self.enabled,
            &format!("{tags}{options}"),
            lan_mutators::heatseeker(&self.picks),
        )?;
        let request = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let menu = format!("MENU_Main_p?HbxFreeplayRestart={request}");
        workshop_host::request_freeplay_travel(cooked, stats, &menu)?;
        self.restart = Some(Restart {
            cooked: cooked.to_owned(),
            menu,
            target,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        Ok("Restarting Freeplay...".into())
    }

    pub(crate) fn poll_restart(
        &mut self,
        ctx: &egui::Context,
        stats: &WsStatsClient,
        rl_path: &str,
    ) {
        let Some(restart) = &self.restart else { return };
        let cooked = Path::new(rl_path).join("TAGame").join("CookedPCConsole");
        if Instant::now() >= restart.deadline
            || cooked != restart.cooked
            || !hebnix_sdk::process::is_rocket_league_running()
        {
            self.restart = None;
            self.status =
                "Freeplay restart stopped. Reopen Freeplay to use the saved settings.".into();
        } else if stats.connected() && hebnix_sdk::log::map_load_finished(&restart.menu) {
            let result =
                workshop_host::request_freeplay_travel(&restart.cooked, stats, &restart.target);
            self.restart = None;
            self.status = result
                .map(|_| "Freeplay reload requested.".into())
                .unwrap_or_else(|error| error);
        } else {
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
        assert!(is_freeplay(normal));
        let practice = format!("stadium_p?Game={HEATSEEKER}{PRACTICE}");
        assert!(is_freeplay(&practice));
        assert!(!is_freeplay(&practice.replace("?HbxSoloPractice", "")));
        assert!(!is_freeplay(&format!("{normal}?Lan?Listen")));
        assert!(!is_freeplay(&format!("1.2.3.4:7777/{normal}")));
    }
}
