use super::HebnixApp;
use crate::i18n::t;
use eframe::egui;
use hebnix_sdk::stats::{EventData, StatsEvent, TeamState, UpdateStateData};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum HomeTab {
    #[default]
    Dashboard,
    LiveMatch,
    Tracker,
}

struct MatchEvent {
    clock: Option<(i64, bool)>,
    message: String,
}

#[derive(Default)]
pub(super) struct HomeState {
    pub(super) tab: HomeTab,
    snapshot: Option<UpdateStateData>,
    updated: Option<Instant>,
    match_guid: Option<String>,
    clock: Option<(i64, bool)>,
    events: VecDeque<MatchEvent>,
    ended: bool,
}

impl HomeState {
    pub(super) fn clear_match(&mut self) {
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
            colour.r(),
            colour.g(),
            colour.b(),
            210,
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
            body.r(),
            body.g(),
            body.b(),
            190,
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
                    .show(ui, |ui| crate::panel::followed(ui, |ui| {
                        ui.heading(t("home-live-match"));
                        if connected && let Some(state) = &home.snapshot {
                            ui.heading(match_clock(Some((
                                state.game.time_seconds,
                                state.game.overtime,
                            ))));
                            ui.label(crate::discord_presence::display_map_name(&state.game.arena));
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
                    }));
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
                    .show(ui, |ui| crate::panel::followed(ui, |ui| {
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
                    }));
            },
        );
    });
}

impl HebnixApp {
    #[cfg(windows)]
    pub(super) fn live_match_web_data(&self) -> serde_json::Value {
        use serde_json::json;
        let running = self.last_rl_open;
        let connected = running && self.stats.connected();
        let game = self.home.snapshot.as_ref().filter(|_| connected).map(|state| {
            let mut teams: Vec<_> = state.game.teams.iter().collect();
            teams.sort_by_key(|team| team.team_num);
            let teams: Vec<_> = teams.into_iter().map(|team| {
                let colour = team_colour(team);
                let name = if team.name.is_empty() {
                    t(if team.team_num == 0 { "home-blue-team" } else { "home-orange-team" })
                } else {
                    team.name.clone()
                };
                let mut players: Vec<_> = state.players.iter().filter(|player| player.team_num == team.team_num).collect();
                players.sort_by_key(|player| std::cmp::Reverse(player.score));
                json!({
                    "name": name, "score": team.score,
                    "colour": format!("#{:02x}{:02x}{:02x}", colour.r(), colour.g(), colour.b()),
                    "players": players.iter().map(|p| {
                        json!([p.name, p.score, p.goals, p.assists, p.saves, p.shots])
                    }).collect::<Vec<_>>(),
                })
            }).collect();
            json!({
                "clock": match_clock(Some((state.game.time_seconds, state.game.overtime))),
                "arena": crate::discord_presence::display_map_name(&state.game.arena),
                "ended": (self.home.ended || state.game.has_winner).then(|| t("home-match-ended")),
                "replay": state.game.replay.then(|| t("home-replay")),
                "stale": self.home.updated.map(|time| time.elapsed().as_secs()).filter(|seconds| *seconds >= 5)
                    .map(|seconds| format!("{}: {seconds}s", t("home-last-update"))),
                "teams": teams,
                "spectators": state.players.iter().filter(|player| {
                    !state.game.teams.iter().any(|team| team.team_num == player.team_num)
                }).map(|player| &player.name).collect::<Vec<_>>(),
            })
        });
        let columns = [
            "home-player",
            "home-score",
            "home-goals",
            "home-assists",
            "home-saves",
            "home-shots",
        ].map(|key| t(key));
        json!({
            "dashboard": self.home.tab == HomeTab::Dashboard,
            "running": running,
            "connected": connected,
            "live_cosmetics": crate::patcher::live_cosmetics::status_snapshot(),
            "running_label": t(if running { "home-running" } else { "home-stopped" }),
            "connection_label": t(if connected { "home-connected" } else { "home-disconnected" }),
            "waiting": t(if !running { "home-launch-hint" } else if !connected { "home-api-hint" } else { "home-match-hint" }),
            "events_title": t("home-events"),
            "empty_events": t("home-no-events"),
            "spectator_label": t("home-spectator"),
            "columns": columns,
            "game": game,
            "events": self.home.events.iter().filter(|_| connected)
                .map(|event| json!({ "clock": match_clock(event.clock), "message": event.message }))
                .collect::<Vec<_>>(),
        })
    }

    /// Dashboard in the web version's look: a status card with large values, then a card with the primary action.
    fn modern_dashboard(&mut self, ui: &mut egui::Ui, connected: bool) {
        let green = egui::Color32::from_rgb(110, 220, 140);
        let visuals = ui.visuals().clone();
        let border = visuals.text_color().gamma_multiply(0.25);
        let card = egui::Frame::NONE
            .fill(visuals.faint_bg_color.gamma_multiply(0.6))
            .stroke(egui::Stroke::new(1.0, border))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(18, 14));
        let live = crate::patcher::live_cosmetics::status_snapshot();
        let live_colour = match live["colour"].as_str() {
            Some("success") => green,
            Some("danger") => egui::Color32::LIGHT_RED,
            _ => egui::Color32::YELLOW,
        };
        let stat = |ui: &mut egui::Ui, label: &str, value: &str, colour: egui::Color32, tip: &str| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(label).size(12.5).color(visuals.weak_text_color()));
                ui.add_space(1.0);
                ui.label(egui::RichText::new(value).size(18.0).strong().color(colour)).on_hover_text(tip);
            });
        };
        let weak = visuals.text_color();
        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (running, running_colour) = (
                    t(if self.last_rl_open { "home-running" } else { "home-stopped" }),
                    if self.last_rl_open { green } else { weak },
                );
                stat(ui, "Rocket League", &running, running_colour, "");
                ui.add_space(28.0);
                ui.separator();
                ui.add_space(28.0);
                let (api, api_colour) = (
                    t(if connected { "home-connected" } else { "home-disconnected" }),
                    if connected { green } else { weak },
                );
                stat(ui, "Stats API", &api, api_colour, "");
                ui.add_space(28.0);
                ui.separator();
                ui.add_space(28.0);
                stat(ui, "Live Cosmetics", live["label"].as_str().unwrap_or("Installing"), live_colour, live["detail"].as_str().unwrap_or_default());
            });
        });
        ui.add_space(12.0);
        let running = self.last_rl_open;
        let accent = visuals.hyperlink_color;
        let bright = (u32::from(accent.r()) * 3 + u32::from(accent.g()) * 6 + u32::from(accent.b())) / 10 > 150;
        let ink = if bright { egui::Color32::from_rgb(16, 18, 28) } else { egui::Color32::WHITE };
        let mut start = false;
        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let label = egui::RichText::new(if running { "Stop Rocket League  ■" } else { "Start Rocket League  ▶" }).size(15.0).color(ink);
            let button = egui::Button::new(label).fill(accent).stroke(egui::Stroke::NONE)
                .corner_radius(6).min_size(egui::vec2(0.0, 38.0));
            start = ui.add(button).clicked();
        });
        if start {
            self.execute_action_button_action(if running {
                crate::config::ActionButtonAction::CloseRocketLeague
            } else {
                crate::config::ActionButtonAction::StartRocketLeague
            }, ui.ctx());
        }
    }

    pub(super) fn render_home(&mut self, ui: &mut egui::Ui, modern_navigation: bool) {
        let connected = self.last_rl_open && self.stats.connected();
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        if !modern_navigation && crate::theme::presentation(ui.ctx()).is_none_or(|theme| theme.layout.show_page_heading) {
            ui.heading(t("home-title"));
            ui.add_space(8.0);
        }
        if !modern_navigation {
            ui.horizontal_wrapped(|ui| {
                for (tab, key) in [
                    (HomeTab::Dashboard, "home-dashboard"),
                    (HomeTab::LiveMatch, "home-live-match"),
                    (HomeTab::Tracker, "home-tracker"),
                ] {
                    ui.selectable_value(&mut self.home.tab, tab, t(key));
                }
            });
            ui.separator();
        }
        match self.home.tab {
            HomeTab::Dashboard if modern_navigation => self.modern_dashboard(ui, connected),
            HomeTab::Dashboard => {
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
                        ui.separator();
                        ui.strong("Live Cosmetics");
                        let live = crate::patcher::live_cosmetics::status_snapshot();
                        let colour = match live["colour"].as_str() {
                            Some("success") => green,
                            Some("danger") => egui::Color32::LIGHT_RED,
                            _ => egui::Color32::YELLOW,
                        };
                        ui.colored_label(colour, live["label"].as_str().unwrap_or("Installing"))
                            .on_hover_text(live["detail"].as_str().unwrap_or_default());
                    });
                });
                ui.add_space(8.0);
                let running = self.last_rl_open;
                if ui.button(if running { "Stop Rocket League  ■" } else { "Start Rocket League  ▶" }).clicked() {
                    self.execute_action_button_action(if running {
                        crate::config::ActionButtonAction::CloseRocketLeague
                    } else {
                        crate::config::ActionButtonAction::StartRocketLeague
                    }, ui.ctx());
                }
            }
            HomeTab::LiveMatch => live_match(ui, &self.home, connected, self.last_rl_open),
            #[cfg(windows)]
            HomeTab::Tracker if modern_navigation => self.home_tracker.show_modern(ui),
            HomeTab::Tracker => self.home_tracker.show(ui),
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
            tab: HomeTab::Tracker,
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
        assert!(home.tab == HomeTab::Tracker);
        home.event(&StatsEvent::simple("MatchDestroyed"));
        assert!(home.snapshot.is_none());
        assert!(home.tab == HomeTab::Tracker);
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
