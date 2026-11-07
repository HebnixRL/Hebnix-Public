use super::{HebnixApp, ItemsSubTab, PluginsSubTab, Tab};
use crate::i18n::t;
use eframe::egui;
use hebnix_sdk::stats::{EventData, StatsEvent, TeamState, UpdateStateData};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum HomeTab {
    #[default]
    LiveMatch,
    Plugins,
    Maps,
    Swaps,
}

struct MatchEvent {
    clock: Option<(i64, bool)>,
    message: String,
}

#[derive(Default)]
pub(super) struct HomeState {
    tab: HomeTab,
    snapshot: Option<UpdateStateData>,
    updated: Option<Instant>,
    match_guid: Option<String>,
    clock: Option<(i64, bool)>,
    events: VecDeque<MatchEvent>,
    ended: bool,
}

impl HomeState {
    fn clear_match(&mut self) {
        *self = Self {
            tab: self.tab,
            ..Self::default()
        };
    }

    pub(super) fn event(&mut self, event: &StatsEvent) {
        if event.event_type == "MatchDestroyed" {
            self.clear_match();
            return;
        }
        if event.event_type == "MatchCreated"
            || event
                .match_guid
                .as_ref()
                .is_some_and(|guid| self.match_guid.as_ref() != Some(guid))
        {
            self.clear_match();
        }
        if event.match_guid.is_some() {
            self.match_guid = event.match_guid.clone();
        }
        let message = match &event.data {
            EventData::UpdateState(state) => {
                self.clock = Some((state.game.time_seconds, state.game.overtime));
                self.snapshot = Some(state.clone());
                self.updated = Some(Instant::now());
                None
            }
            EventData::ClockUpdatedSeconds(clock) => {
                self.clock = Some((clock.time_seconds, clock.overtime));
                if let Some(state) = &mut self.snapshot {
                    state.game.time_seconds = clock.time_seconds;
                    state.game.overtime = clock.overtime;
                }
                None
            }
            EventData::GoalScored(goal) => Some(format!(
                "{} · {} · {:.0} km/h",
                t("home-goal"),
                goal.scorer.name,
                goal.goal_speed
            )),
            EventData::Statfeed(stat) => {
                Some(format!("{} · {}", stat.main_target.name, stat.stat_type))
            }
            EventData::MatchEnded(_) => {
                self.ended = true;
                Some(t("home-match-ended"))
            }
            EventData::Simple if event.event_type == "MatchStarted" => {
                Some(t("home-match-started"))
            }
            _ => None,
        };
        if let Some(message) = message {
            self.events.push_front(MatchEvent {
                clock: self.clock,
                message,
            });
            self.events.truncate(256);
        }
    }
}

fn match_clock(clock: Option<(i64, bool)>) -> String {
    let Some((seconds, overtime)) = clock else {
        return "—:—".into();
    };
    let seconds = seconds.unsigned_abs();
    format!(
        "{}{}:{:02}",
        if overtime { "+" } else { "" },
        seconds / 60,
        seconds % 60
    )
}

fn team_colour(team: &TeamState) -> egui::Color32 {
    let raw = team.color_primary.trim().trim_start_matches('#');
    let raw = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if (raw.len() == 6 || raw.len() == 8) && raw.is_ascii() {
        if let Ok(rgb) = u32::from_str_radix(&raw[..6], 16) {
            return egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        }
    }
    if team.team_num == 0 {
        egui::Color32::from_rgb(40, 115, 210)
    } else {
        egui::Color32::from_rgb(225, 120, 30)
    }
}

fn readable_text(background: egui::Color32) -> egui::Color32 {
    let linear = |channel: u8| {
        let v = f32::from(channel) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(background.r())
        + 0.7152 * linear(background.g())
        + 0.0722 * linear(background.b());
    if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

fn team_scoreboard(ui: &mut egui::Ui, state: &UpdateStateData, team: &TeamState) {
    let colour = team_colour(team);
    let text_colour = readable_text(colour);
    let name = if team.name.is_empty() {
        t(if team.team_num == 0 {
            "home-blue-team"
        } else {
            "home-orange-team"
        })
    } else {
        team.name.clone()
    };
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(
            colour.r(), colour.g(), colour.b(), 210,
        ))
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(name)
                            .color(text_colour)
                            .size(22.0)
                            .strong(),
                    )
                    .wrap(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(team.score.to_string())
                            .color(text_colour)
                            .size(28.0)
                            .strong(),
                    );
                });
            });
        });
    let surface = ui.visuals().panel_fill;
    let mix = |a: u8, b: u8| (f32::from(a) * 0.14 + f32::from(b) * 0.86).round() as u8;
    let body = egui::Color32::from_rgb(
        mix(colour.r(), surface.r()),
        mix(colour.g(), surface.g()),
        mix(colour.b(), surface.b()),
    );
    let text_colour = readable_text(body);
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(
            body.r(), body.g(), body.b(), 190,
        ))
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::horizontal()
                .id_salt(("home_team_players", team.team_num))
                .show(ui, |ui| {
                    egui::Grid::new(("home_team_scoreboard", team.team_num))
                        .striped(false)
                        .spacing([14.0, 8.0])
                        .show(ui, |ui| {
                            for key in [
                                "home-player",
                                "home-score",
                                "home-goals",
                                "home-assists",
                                "home-saves",
                                "home-shots",
                            ] {
                                ui.label(egui::RichText::new(t(key)).color(text_colour).strong());
                            }
                            ui.end_row();
                            let mut players: Vec<_> = state
                                .players
                                .iter()
                                .filter(|p| p.team_num == team.team_num)
                                .collect();
                            players.sort_by_key(|p| std::cmp::Reverse(p.score));
                            for player in players {
                                ui.label(egui::RichText::new(&player.name).color(text_colour));
                                for value in [
                                    player.score,
                                    player.goals,
                                    player.assists,
                                    player.saves,
                                    player.shots,
                                ] {
                                    ui.label(
                                        egui::RichText::new(value.to_string()).color(text_colour),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
        });
}

fn live_match(ui: &mut egui::Ui, home: &HomeState, connected: bool, running: bool) {
    let height = ui.available_height().max(0.0);
    let spacing = ui.spacing().item_spacing.x;
    let events_width = (ui.available_width() * 0.30)
        .clamp(170.0, 320.0)
        .min(ui.available_width() * 0.45);
    let scoreboard_width = (ui.available_width() - events_width - spacing).max(0.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(scoreboard_width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(scoreboard_width);
                egui::ScrollArea::vertical()
                    .id_salt("home_live_scoreboard")
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading(t("home-live-match"));
                        if connected && let Some(state) = &home.snapshot {
                            ui.heading(match_clock(Some((
                                state.game.time_seconds,
                                state.game.overtime,
                            ))));
                            ui.label(&state.game.arena);
                            if home.ended || state.game.has_winner {
                                ui.label(t("home-match-ended"));
                            }
                            if state.game.replay {
                                ui.label(t("home-replay"));
                            }
                            if let Some(updated) = home.updated
                                && updated.elapsed().as_secs() >= 5
                            {
                                ui.weak(format!(
                                    "{}: {}s",
                                    t("home-last-update"),
                                    updated.elapsed().as_secs()
                                ));
                            }
                            ui.add_space(8.0);
                            let mut teams: Vec<_> = state.game.teams.iter().collect();
                            teams.sort_by_key(|team| team.team_num);
                            for team in teams {
                                team_scoreboard(ui, state, team);
                                ui.add_space(10.0);
                            }
                            // Keep players visible while the API is still assembling team details.
                            let unassigned: Vec<_> = state
                                .players
                                .iter()
                                .filter(|p| {
                                    !state
                                        .game
                                        .teams
                                        .iter()
                                        .any(|team| team.team_num == p.team_num)
                                })
                                .collect();
                            if !unassigned.is_empty() {
                                ui.strong(t("home-spectator"));
                                for player in unassigned {
                                    ui.label(&player.name);
                                }
                            }
                        } else {
                            ui.label(t(if !running {
                                "home-launch-hint"
                            } else if !connected {
                                "home-api-hint"
                            } else {
                                "home-match-hint"
                            }));
                        }
                    });
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(events_width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(events_width);
                ui.heading(t("home-events"));
                egui::ScrollArea::vertical()
                    .id_salt("home_match_events")
                    .max_height(ui.available_height().max(0.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if !connected || home.events.is_empty() {
                            ui.label(t("home-no-events"));
                        } else {
                            for event in &home.events {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.strong(match_clock(event.clock));
                                    ui.add(egui::Label::new(&event.message).wrap());
                                });
                            }
                        }
                    });
            },
        );
    });
}

impl HebnixApp {
    pub(super) fn render_home(&mut self, ui: &mut egui::Ui) {
        let connected = self.last_rl_open && self.stats.connected();
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        ui.heading(t("home-title"));
        ui.add_space(8.0);
        let green = if ui.visuals().dark_mode {
            egui::Color32::LIGHT_GREEN
        } else {
            egui::Color32::from_rgb(0, 120, 50)
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong("Rocket League");
                ui.colored_label(
                    if self.last_rl_open {
                        green
                    } else {
                        ui.visuals().weak_text_color()
                    },
                    t(if self.last_rl_open {
                        "home-running"
                    } else {
                        "home-stopped"
                    }),
                );
                ui.separator();
                ui.strong("Stats API");
                ui.colored_label(
                    if connected {
                        green
                    } else {
                        ui.visuals().weak_text_color()
                    },
                    t(if connected {
                        "home-connected"
                    } else {
                        "home-disconnected"
                    }),
                );
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for (tab, key) in [
                (HomeTab::LiveMatch, "home-live-match"),
                (HomeTab::Plugins, "home-active-plugins"),
                (HomeTab::Maps, "home-active-maps"),
                (HomeTab::Swaps, "home-active-swaps"),
            ] {
                ui.selectable_value(&mut self.home.tab, tab, t(key));
            }
        });
        ui.separator();
        match self.home.tab {
            HomeTab::LiveMatch => live_match(ui, &self.home, connected, self.last_rl_open),
            HomeTab::Plugins => {
                if ui.button(t("home-install-plugins")).clicked() {
                    self.install_modal = super::InstallModal {
                        open: true,
                        ..Default::default()
                    };
                }
                egui::ScrollArea::vertical()
                    .id_salt("home_active_plugins")
                    .max_height(ui.available_height().max(0.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let plugins: Vec<_> = self
                            .plugin_mgr
                            .plugins
                            .iter()
                            .filter(|p| p.enabled && p.load_error.is_none())
                            .map(|p| {
                                (
                                    p.slug.clone(),
                                    p.display_name().to_string(),
                                    p.has_settings(),
                                )
                            })
                            .collect();
                        if plugins.is_empty() {
                            ui.label(t("home-no-plugins"));
                        }
                        for (slug, name, has_settings) in plugins {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.strong(name);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if has_settings && ui.button(t("tab-settings")).clicked() {
                                                self.selected_settings_plugin = Some(slug);
                                                self.plugins_subtab = PluginsSubTab::Settings;
                                                self.tab = Tab::Plugins;
                                            }
                                        },
                                    );
                                });
                            });
                        }
                    });
            }
            HomeTab::Maps => {
                if ui.button(t("home-install-map")).clicked() {
                    self.workshop.open_import();
                    self.tab = Tab::Workshop;
                }
                egui::ScrollArea::vertical()
                    .id_salt("home_active_maps")
                    .max_height(ui.available_height().max(0.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut maps = self
                            .workshop
                            .manager
                            .active_maps()
                            .into_iter()
                            .collect::<Vec<_>>();
                        maps.sort_by(|a, b| a.0.cmp(&b.0));
                        if maps.is_empty() {
                            ui.label(t("home-no-maps"));
                        }
                        for (target, map) in maps {
                            let name = map.get("name").and_then(|v| v.as_str()).unwrap_or(&target);
                            ui.label(name);
                        }
                    });
            }
            HomeTab::Swaps => {
                if ui.button(t("home-swap-items")).clicked() {
                    self.items_subtab = ItemsSubTab::Swapper;
                    self.tab = Tab::Items;
                }
                let cooked_pc = std::path::PathBuf::from(&self.config.settings.rl_path)
                    .join("TAGame")
                    .join("CookedPCConsole");
                let backups_dir = cooked_pc.join("Backups");
                egui::ScrollArea::vertical()
                    .id_salt("home_active_swaps")
                    .max_height(ui.available_height().max(0.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.swapper.active_count(&backups_dir) == 0 {
                            ui.label(t("home-no-swaps"));
                        }
                        self.swapper
                            .render_active_swaps(ui, &cooked_pc, &backups_dir, &self.tx);
                    });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hebnix_sdk::stats::{ClockUpdatedSecondsData, PlayerState};

    #[test]
    fn clears_previous_match_without_switching_tabs_and_bounds_history() {
        let mut home = HomeState {
            tab: HomeTab::Plugins,
            ..Default::default()
        };
        let mut update = StatsEvent::simple("UpdateState");
        update.match_guid = Some("first".into());
        update.data = EventData::UpdateState(UpdateStateData::default());
        home.event(&update);
        for _ in 0..300 {
            home.event(&StatsEvent::simple("MatchStarted"));
        }
        assert_eq!(home.events.len(), 256);
        update.match_guid = Some("second".into());
        home.event(&update);
        assert!(home.events.is_empty());
        assert!(home.snapshot.is_some());
        assert!(home.tab == HomeTab::Plugins);
        home.event(&StatsEvent::simple("MatchDestroyed"));
        assert!(home.snapshot.is_none());
        assert!(home.tab == HomeTab::Plugins);
    }

    #[test]
    fn event_timestamps_use_the_match_clock_and_preserve_overtime() {
        let mut home = HomeState::default();
        let mut clock = StatsEvent::simple("ClockUpdatedSeconds");
        clock.data = EventData::ClockUpdatedSeconds(ClockUpdatedSecondsData {
            time_seconds: 150,
            overtime: false,
        });
        home.event(&clock);
        home.event(&StatsEvent::simple("MatchStarted"));
        clock.data = EventData::ClockUpdatedSeconds(ClockUpdatedSecondsData {
            time_seconds: 17,
            overtime: true,
        });
        home.event(&clock);
        home.event(&StatsEvent::simple("MatchStarted"));
        assert_eq!(match_clock(home.events[0].clock), "+0:17");
        assert_eq!(match_clock(home.events[1].clock), "2:30");
        assert_eq!(match_clock(None), "—:—");
    }

    #[test]
    fn team_text_remains_readable_for_white_black_and_custom_team_colours() {
        assert_eq!(readable_text(egui::Color32::WHITE), egui::Color32::BLACK);
        assert_eq!(readable_text(egui::Color32::BLACK), egui::Color32::WHITE);
        let mut team = TeamState {
            color_primary: "#ffffff".into(),
            ..Default::default()
        };
        assert_eq!(team_colour(&team), egui::Color32::WHITE);
        team.color_primary = "0x000000".into();
        assert_eq!(team_colour(&team), egui::Color32::BLACK);
        team.color_primary = "not a colour".into();
        assert_ne!(team_colour(&team), egui::Color32::TRANSPARENT);
    }

    fn text_shapes(shape: &egui::Shape, output: &mut Vec<(egui::Rect, String, egui::Color32)>) {
        match shape {
            egui::Shape::Text(text) => output.push((
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                text.galley.text().into(),
                text.fallback_color,
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text_shapes(shape, output);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn live_scoreboard_and_events_fit_the_window_and_events_stay_on_the_right() {
        for size in [egui::vec2(1000.0, 650.0), egui::vec2(650.0, 450.0)] {
            let ctx = egui::Context::default();
            let mut state = UpdateStateData::default();
            state.game.teams = vec![
                TeamState {
                    name: "Blue fixture".into(),
                    team_num: 0,
                    color_primary: "ffffff".into(),
                    ..Default::default()
                },
                TeamState {
                    name: "Orange fixture".into(),
                    team_num: 1,
                    color_primary: "000000".into(),
                    ..Default::default()
                },
            ];
            state.players = (0..40)
                .map(|i| PlayerState {
                    name: format!("Player {i}"),
                    team_num: i % 2,
                    ..Default::default()
                })
                .collect();
            let home = HomeState {
                snapshot: Some(state),
                events: (0..100)
                    .map(|i| MatchEvent {
                        clock: Some((150, false)),
                        message: format!("Fixture event {i}"),
                    })
                    .collect(),
                ..Default::default()
            };
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        live_match(ui, &home, true, true);
                        assert!(
                            ui.min_rect().right() <= size.x + 1.0,
                            "Content overflowed the window width: {:?}",
                            ui.min_rect()
                        );
                        assert!(
                            ui.min_rect().bottom() <= size.y + 1.0,
                            "Content overflowed the window height: {:?}",
                            ui.min_rect()
                        );
                    });
                },
            );
            let mut texts = Vec::new();
            for shape in &output.shapes {
                text_shapes(&shape.shape, &mut texts);
            }
            let scoreboard = texts
                .iter()
                .find(|(_, text, _)| text == &t("home-live-match"))
                .unwrap()
                .0;
            let events = texts
                .iter()
                .find(|(_, text, _)| text == &t("home-events"))
                .unwrap()
                .0;
            assert!(events.left() > scoreboard.right());
            assert!(texts.iter().any(|(_, text, _)| text == "2:30"));
            assert!(!texts.iter().any(|(_, text, _)| text == &t("home-team")));
            assert_eq!(
                texts
                    .iter()
                    .filter(|(_, text, _)| text == "Blue fixture")
                    .count(),
                1
            );
        }
    }
}
