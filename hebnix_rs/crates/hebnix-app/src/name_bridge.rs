//! UI for the experimental mid-match name bridge. Package work stays off-frame.
use crate::patcher::profile_pictures::name_bridge as patch;
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct State {
    game: PathBuf,
    enabled: bool,
    checked: bool,
    name: String,
    status: String,
    operation: Option<mpsc::Receiver<Result<bool, String>>>,
    pending: Option<Instant>,
}
impl State {
    fn start(&mut self, change: Option<bool>) {
        let game = self.game.clone();
        let (tx, rx) = mpsc::channel();
        self.operation = Some(rx);
        self.status = match change {
            Some(true) => "Installing bridge…",
            Some(false) => "Removing bridge…",
            None => "Checking bridge…",
        }
        .into();
        std::thread::spawn(move || {
            let result = match change {
                Some(on) => patch::set_enabled(&game, on).and_then(|_| patch::installed(&game)),
                None => patch::installed(&game),
            };
            let _ = tx.send(result);
        });
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, game: &Path) {
        if self.game != game && self.operation.is_none() {
            self.game = game.to_path_buf();
            self.checked = false;
            self.enabled = false;
            self.pending = None;
        }
        if !self.checked && self.operation.is_none() {
            self.checked = true;
            self.start(None);
        }
        if let Some(rx) = &self.operation {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Bridge worker stopped unexpectedly.".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.operation = None;
                match result {
                    Ok(enabled) => {
                        self.enabled = enabled;
                        self.status = if enabled {
                            "Bridge installed. Start a fresh game session to load the hook."
                        } else {
                            "Bridge disabled."
                        }
                        .into();
                    }
                    Err(error) => self.status = error,
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if let Some(sent) = self.pending {
            if !patch::request_path(&self.game).exists() {
                self.pending = None;
                self.status = "Request file consumed. Check your local scoreboard/nameplate. This package has no verified client-to-server rename call.".into();
            } else if sent.elapsed() >= Duration::from_secs(10) {
                self.status = "Still waiting for the game to consume the request. Join a match in a fresh session with the bridge installed.".into();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }

        ui.heading("Name bridge test");
        ui.label("Trial changing your local display name during an offline or LAN match.");
        ui.add_space(12.0);
        let running = hebnix_sdk::process::is_rocket_league_running();
        let busy = self.operation.is_some() || self.game != game;
        ui.horizontal(|ui| {
            let label = if self.enabled {
                "Disable bridge"
            } else {
                "Enable bridge"
            };
            if ui
                .add_enabled(!busy && !running, egui::Button::new(label))
                .on_disabled_hover_text("Close Rocket League before changing the package hook.")
                .clicked()
            {
                self.pending = None;
                self.start(Some(!self.enabled));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Check installation"))
                .clicked()
            {
                self.start(None);
            }
        });
        ui.weak("Enable with Rocket League closed, then launch it and enter an offline or LAN match. Remove the hook with the game closed.");
        ui.add_space(16.0);
        ui.label("Test name");
        let input = ui.add(
            egui::TextEdit::singleline(&mut self.name)
                .hint_text("Enter a name")
                .char_limit(32)
                .desired_width(340.0),
        );
        let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let can_send = self.enabled && running && !busy && !self.name.trim().is_empty();
        ui.horizontal(|ui| {
            if (ui
                .add_enabled(can_send, egui::Button::new("Apply test name"))
                .clicked()
                || enter)
                && can_send
            {
                match patch::send(&self.game, &self.name) {
                    Ok(()) => {
                        self.pending = Some(Instant::now());
                        self.status = "Request sent. Waiting for the game to consume it…".into();
                    }
                    Err(error) => self.status = error,
                }
            }
        });
        ui.add_space(12.0);
        ui.label(&self.status);
        ui.add_space(12.0);
        ui.weak("Targets the inspected current Epic package. This trial updates the local player record and UI; other players may continue to see the server's name. The existing Display Name setting is separate.");
    }
}
