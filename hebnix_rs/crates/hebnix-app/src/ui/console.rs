//! Developer console: scrollback, command input with history and
//! autocomplete suggestions.

use crate::i18n::t;
use eframe::egui::{self, Key, Modifiers};

const MAX_LINES: usize = 2000;

pub struct ConsoleLine {
    /// local wall-clock time the line was written, "HH:MM:SS"
    pub time: String,
    pub text: String,
}

impl ConsoleLine {
    pub(crate) fn display(&self, timestamps: bool) -> String {
        if timestamps {
            format!("[{}] {}", self.time, self.text)
        } else {
            self.text.clone()
        }
    }
}

/// local time as "HH:MM:SS". `fallback` is the offset captured at startup,
/// used when the local offset can't be read (e.g. multi-threaded on unix).
fn timestamp_now(fallback: time::UtcOffset) -> String {
    let now = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc().to_offset(fallback));
    format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
}

pub struct ConsoleState {
    pub lines: Vec<ConsoleLine>,
    pub input: String,
    pub history: Vec<String>,
    pub history_index: usize,
    pub suggestions: Vec<String>,
    pub suggestion_index: Option<usize>,
    last_input: String,
    stick_to_bottom: bool,
    utc_offset: time::UtcOffset,
}

const BASE_COMMANDS: [&str; 11] = [
    "help",
    "info",
    "server",
    "webview",
    "plugin load ",
    "plugin reload ",
    "plugin unload ",
    "plugins list",
    "clear",
    "quit",
    "restart",
];

/// longest prefix shared by every candidate
fn common_prefix(items: &[String]) -> String {
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for item in iter {
        let shared = prefix
            .chars()
            .zip(item.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let cut = prefix
            .char_indices()
            .nth(shared)
            .map_or(prefix.len(), |(i, _)| i);
        prefix.truncate(cut);
    }
    prefix
}

impl Default for ConsoleState {
    fn default() -> Self {
        let utc_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let time = timestamp_now(utc_offset);
        let lines = [
            "[Console] Hebnix Developer Console Initialized.",
            "[Console] Type 'info' for diagnostic details.",
            "[Console] Type 'help' for a list of commands.",
        ]
        .into_iter()
        .map(|text| ConsoleLine {
            time: time.clone(),
            text: text.to_string(),
        })
        .collect();
        Self {
            lines,
            input: String::new(),
            history: Vec::new(),
            history_index: 0,
            suggestions: Vec::new(),
            suggestion_index: None,
            last_input: String::new(),
            stick_to_bottom: true,
            utc_offset,
        }
    }
}

impl ConsoleState {
    pub fn write(&mut self, message: impl Into<String>) {
        let time = timestamp_now(self.utc_offset);
        for line in message.into().lines() {
            // Mirror every console line into hebnix.log for post-mortems.
            tracing::info!(target: "console", "{line}");
            self.lines.push(ConsoleLine {
                time: time.clone(),
                text: line.to_string(),
            });
        }
        if self.lines.len() > MAX_LINES {
            let excess = self.lines.len() - MAX_LINES;
            self.lines.drain(0..excess);
        }
        self.stick_to_bottom = true;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// whole scrollback as plain text, one line per entry
    pub fn text(&self, timestamps: bool) -> String {
        self.lines
            .iter()
            .map(|line| line.display(timestamps))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn update_suggestions(&mut self, plugin_names: &[String]) {
        self.suggestions.clear();
        self.suggestion_index = None;
        if self.input.is_empty() {
            return;
        }
        for cmd in BASE_COMMANDS {
            if cmd.starts_with(&self.input) {
                self.suggestions.push(cmd.to_string());
            }
        }
        if self.input.starts_with("plugin ") {
            let parts: Vec<&str> = self.input.split(' ').collect();
            if parts.len() >= 2 {
                let action = parts[1];
                for name in plugin_names {
                    let full = format!("plugin {action} {name}");
                    if full.starts_with(&self.input) && !self.suggestions.contains(&full) {
                        self.suggestions.push(full);
                    }
                }
            }
        }
        self.suggestions.truncate(5);
    }

    #[cfg(windows)]
    pub(crate) fn edit_web_input(&mut self, value: String, plugin_names: Option<&[String]>) {
        self.input = value;
        self.last_input = self.input.clone();
        if let Some(plugin_names) = plugin_names {
            self.update_suggestions(plugin_names);
        } else {
            self.suggestions.clear();
            self.suggestion_index = None;
        }
    }

    pub(crate) fn submit(&mut self) -> Option<String> {
        let raw = self.input.trim().to_owned();
        if raw.is_empty() {
            return None;
        }
        self.write(format!("> {raw}"));
        self.history.push(raw.clone());
        self.history_index = self.history.len();
        self.input.clear();
        self.last_input.clear();
        self.suggestions.clear();
        self.suggestion_index = None;
        Some(raw)
    }

    /// render the console. returns a command line when the user submits one.
    /// `timestamps` is the System setting that prefixes each line with its time.
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        plugin_names: &[String],
        timestamps: bool,
    ) -> Option<String> {
        let mut submitted: Option<String> = None;

        // Toolbar: copy-to-clipboard. The horizontal row keeps it one line tall;
        // a bare right_to_left layout would claim all remaining height.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(t("console-copy"))
                    .on_hover_text(t("console-copy-tooltip"))
                    .clicked()
                {
                    ui.ctx().copy_text(self.text(timestamps));
                }
            });
        });

        // Reserve space for the input row at the bottom.
        let input_height = 32.0;
        let log_height = (ui.available_height() - input_height - 8.0).max(60.0);

        egui::Frame::group(ui.style())
            .fill(ui.visuals().extreme_bg_color)
            .show(ui, |ui| {
                ui.set_min_height(log_height);
                ui.set_max_height(log_height);
                egui::ScrollArea::vertical()
                    .id_salt("console_scroll")
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.stick_to_bottom)
                    .show(ui, |ui| crate::panel::followed(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        for line in &self.lines {
                            if timestamps {
                                // dim timestamp, normal text, in one wrapping label
                                let mut job = egui::text::LayoutJob::default();
                                let font = egui::FontId::monospace(12.0);
                                job.append(
                                    &format!("[{}] ", line.time),
                                    0.0,
                                    egui::TextFormat::simple(
                                        font.clone(),
                                        ui.visuals().weak_text_color(),
                                    ),
                                );
                                job.append(
                                    &line.text,
                                    0.0,
                                    egui::TextFormat::simple(font, ui.visuals().text_color()),
                                );
                                ui.add(egui::Label::new(job).wrap());
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&line.text).monospace().size(12.0),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    }));
            });
        self.stick_to_bottom = false;

        ui.add_space(4.0);

        // Keyboard handling before the TextEdit consumes events.
        let input_id = egui::Id::new("console_input");
        let has_focus = ui.ctx().memory(|m| m.has_focus(input_id));

        let mut apply_suggestion: Option<String> = None;
        if has_focus {
            let up = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
            let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
            let tab = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));

            if tab && !self.suggestions.is_empty() {
                let prefix = common_prefix(&self.suggestions);
                apply_suggestion = if prefix.chars().count() > self.input.chars().count() {
                    Some(prefix)
                } else {
                    self.suggestions
                        .get(self.suggestion_index.unwrap_or(0))
                        .cloned()
                };
            }

            if !self.suggestions.is_empty() {
                if up {
                    self.suggestion_index = Some(match self.suggestion_index {
                        Some(0) | None => self.suggestions.len() - 1,
                        Some(i) => i - 1,
                    });
                }
                if down {
                    self.suggestion_index = Some(match self.suggestion_index {
                        None => 0,
                        Some(i) if i + 1 >= self.suggestions.len() => 0,
                        Some(i) => i + 1,
                    });
                }
            } else if !self.history.is_empty() {
                if up && self.history_index > 0 {
                    self.history_index -= 1;
                    self.input = self.history[self.history_index].clone();
                    self.last_input = self.input.clone();
                }
                if down {
                    if self.history_index + 1 < self.history.len() {
                        self.history_index += 1;
                        self.input = self.history[self.history_index].clone();
                    } else {
                        self.history_index = self.history.len();
                        self.input.clear();
                    }
                    self.last_input = self.input.clone();
                }
            }
        }

        let response = ui.add(
            egui::TextEdit::singleline(&mut self.input)
                .id(input_id)
                .hint_text(t("render-enter-system-command"))
                .font(egui::TextStyle::Monospace)
                .lock_focus(!self.suggestions.is_empty()) // else tab should walk focus off the input
                .desired_width(f32::INFINITY),
        );

        // Suggestions popup above the input.
        if has_focus && !self.suggestions.is_empty() {
            let popup_height = self.suggestions.len() as f32 * 20.0 + 8.0;
            let popup_pos = response.rect.left_top() - egui::vec2(0.0, popup_height + 4.0);
            egui::Area::new(egui::Id::new("console_suggestions"))
                .fixed_pos(popup_pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(response.rect.width());
                        for (i, s) in self.suggestions.clone().iter().enumerate() {
                            let selected = self.suggestion_index == Some(i);
                            let label = ui.selectable_label(
                                selected,
                                egui::RichText::new(s).monospace().size(12.0),
                            );
                            if label.clicked() {
                                apply_suggestion = Some(s.clone());
                            }
                        }
                    });
                });
        }

        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            if let Some(i) = self.suggestion_index {
                // Enter with an active suggestion applies it.
                apply_suggestion = self.suggestions.get(i).cloned();
            } else {
                submitted = self.submit();
            }
            response.request_focus();
        }

        if let Some(s) = apply_suggestion {
            self.input = s;
            self.last_input = self.input.clone();
            self.suggestions.clear();
            self.suggestion_index = None;
            response.request_focus();
            // Move the cursor to the end of the applied suggestion.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), input_id) {
                let ccursor = egui::text::CCursor::new(self.input.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                state.store(ui.ctx(), input_id);
            }
        }

        if self.input != self.last_input {
            self.last_input = self.input.clone();
            self.update_suggestions(plugin_names);
        }

        submitted
    }
}
