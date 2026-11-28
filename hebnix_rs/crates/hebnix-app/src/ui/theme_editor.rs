//! Theme editor on the Theme settings page: the equipped theme's font and colours.
//! The first edit saves a copy, so the original theme file is never changed.
use super::*;

const PALETTE: [(&str, &str); 14] = [
    ("accent", "Accent"),
    ("hyperlink", "Links & highlight"),
    ("window_bg", "Window background"),
    ("panel_bg", "Panel background"),
    ("widget_bg", "Widget background"),
    ("extreme_bg", "Input background"),
    ("faint_bg", "Faint background"),
    ("text_edit_bg", "Text box background"),
    ("code_bg", "Code background"),
    ("text", "Text"),
    ("weak_text", "Weak text"),
    ("selection_text", "Selection outline"),
    ("warn", "Warning"),
    ("error", "Error"),
];
const STATES: [(&str, &str); 5] = [
    ("noninteractive", "Static"),
    ("inactive", "Idle"),
    ("hovered", "Hovered"),
    ("active", "Pressed"),
    ("open", "Open"),
];
const STATE_KEYS: [(&str, &str); 4] = [
    ("bg_fill", "Fill"),
    ("weak_bg_fill", "Weak fill"),
    ("bg_stroke", "Border"),
    ("fg_stroke", "Text"),
];
/// Seconds between saves while a picker is being dragged.
const SAVE_EVERY: f64 = 0.15;

#[derive(Default)]
pub(super) struct ThemeEditor {
    /// The theme `table` was read from.
    name: String,
    table: toml::Table,
    dirty: bool,
    saved_at: f64,
    error: Option<String>,
    font_filter: String,
    google_query: String,
    /// A font download running in the background: the family and the host it came from.
    download: Option<std::sync::mpsc::Receiver<(String, Result<String, String>)>>,
}

fn is_edited_copy(name: &str) -> bool {
    name.contains(" (edited")
}

fn copy_name(themes_dir: &std::path::Path, name: &str) -> String {
    (1..)
        .map(|n| if n == 1 { format!("{name} (edited)") } else { format!("{name} (edited {n})") })
        .find(|candidate| !themes_dir.join(format!("{candidate}.toml")).exists())
        .expect("a free copy name")
}

fn get_path<'a>(table: &'a toml::Table, path: &[&str]) -> Option<&'a toml::Value> {
    let (last, parents) = path.split_last()?;
    let mut table = table;
    for key in parents {
        table = table.get(*key)?.as_table()?;
    }
    table.get(*last)
}

fn set_path(table: &mut toml::Table, path: &[&str], value: toml::Value) {
    let (last, parents) = path.split_last().expect("a non-empty key path");
    let mut table = table;
    for key in parents {
        let entry = table.entry(*key).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        table = entry.as_table_mut().expect("just made a table");
    }
    table.insert((*last).to_owned(), value);
}

/// What the key shows right now, for keys the theme file leaves out.
fn current_colour(visuals: &egui::Visuals, section: &str, key: &str) -> egui::Color32 {
    let widget = |state: &str| match state {
        "noninteractive" => &visuals.widgets.noninteractive,
        "inactive" => &visuals.widgets.inactive,
        "hovered" => &visuals.widgets.hovered,
        "active" => &visuals.widgets.active,
        _ => &visuals.widgets.open,
    };
    match (section, key) {
        ("colors", "accent") => visuals.selection.bg_fill,
        ("colors", "hyperlink") => visuals.hyperlink_color,
        ("colors", "window_bg") => visuals.window_fill,
        ("colors", "panel_bg") => visuals.panel_fill,
        ("colors", "widget_bg") => visuals.widgets.inactive.bg_fill,
        ("colors", "extreme_bg") => visuals.extreme_bg_color,
        ("colors", "faint_bg") => visuals.faint_bg_color,
        ("colors", "text_edit_bg") => visuals.text_edit_bg_color(),
        ("colors", "code_bg") => visuals.code_bg_color,
        ("colors", "text") => visuals.text_color(),
        ("colors", "weak_text") => visuals.weak_text_color(),
        ("colors", "selection_text") => visuals.selection.stroke.color,
        ("colors", "warn") => visuals.warn_fg_color,
        ("colors", _) => visuals.error_fg_color,
        (state, "bg_fill") => widget(state).bg_fill,
        (state, "weak_bg_fill") => widget(state).weak_bg_fill,
        (state, "bg_stroke") => widget(state).bg_stroke.color,
        (state, _) => widget(state).fg_stroke.color,
    }
}

/// An RGB picker for one theme key. Keeps the key's alpha, since the picker has none.
fn colour_button(ui: &mut egui::Ui, table: &mut toml::Table, path: &[&str], fallback: egui::Color32) -> bool {
    let current = get_path(table, path)
        .and_then(toml::Value::as_str)
        .and_then(theme::parse_color)
        .unwrap_or(fallback);
    let [r, g, b, a] = current.to_srgba_unmultiplied();
    let mut rgb = [r, g, b];
    let changed = ui.color_edit_button_srgb(&mut rgb).changed();
    if changed {
        let [r, g, b] = rgb;
        let hex = if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") };
        set_path(table, path, toml::Value::String(hex));
    }
    changed
}

/// Copies a picked .ttf/.otf into the fonts folder and returns the name themes use for it.
fn import_font(fonts_dir: &std::path::Path) -> Result<Option<String>, String> {
    let dialog = rfd::FileDialog::new().set_title("Import font").add_filter("Fonts", &["ttf", "otf"]);
    let Some(path) = winutil::parent_file_dialog(dialog).pick_file() else {
        return Ok(None);
    };
    let (Some(name), Some(file)) = (path.file_stem().and_then(|stem| stem.to_str()), path.file_name()) else {
        return Err("The font's file name is not valid text.".into());
    };
    std::fs::create_dir_all(fonts_dir).map_err(|e| format!("Could not create the fonts folder: {e}"))?;
    std::fs::copy(&path, fonts_dir.join(file)).map_err(|e| format!("Could not copy the font: {e}"))?;
    Ok(Some(name.to_owned()))
}

impl HebnixApp {
    pub(super) fn theme_editor_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let current = self.config.settings.theme.clone();
        let editor = &mut self.theme_editor;
        if editor.name != current {
            editor.name = current.clone();
            editor.dirty = false;
            editor.error = None;
            editor.table = if current == "Dark" || current == "Light" {
                toml::Table::from_iter([("base".to_owned(), toml::Value::String(current.to_lowercase()))])
            } else {
                match std::fs::read_to_string(self.themes_dir.join(format!("{current}.toml")))
                    .map_err(|e| e.to_string())
                    .and_then(|text| text.parse::<toml::Table>().map_err(|e| e.to_string()))
                {
                    Ok(table) => table,
                    Err(error) => {
                        editor.error = Some(format!("Could not read the theme: {error}"));
                        toml::Table::new()
                    }
                }
            };
        }
        let readable = editor.error.is_none() || !editor.table.is_empty();
        let visuals = ctx.global_style().visuals.clone();
        let mut changed = false;
        let mut font_error = None;
        let mut offer: Option<(String, Option<crate::fonts::FontSource>)> = None;
        let mut google: Option<String> = None;
        egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.heading("Theme editor");
            ui.label(egui::RichText::new(if is_edited_copy(&editor.name) {
                "Changes save to this copy as you make them."
            } else {
                "Your first change saves a copy of this theme, so the original stays as it is."
            }).small().weak());
            if let Some(error) = &editor.error {
                ui.colored_label(visuals.warn_fg_color, error);
            }
            if !readable {
                return;
            }
            ui.add_space(6.0);
            let downloading = editor.download.is_some();
            ui.horizontal_wrapped(|ui| {
                ui.label("Font");
                let font = get_path(&editor.table, &["font"]).and_then(toml::Value::as_str).unwrap_or_default().to_owned();
                let mut chosen = font.clone();
                let missing = !font.is_empty() && !crate::fonts::installed(&self.fonts_dir, &font);
                let filter = editor.font_filter.to_lowercase();
                egui::ComboBox::from_id_salt("theme_editor_font")
                    .selected_text(if font.is_empty() { "Default" } else { font.as_str() })
                    .width(200.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen, String::new(), "Default");
                        // Hebnix's fonts folder (scanned only while open), then the fonts on this PC
                        let local = theme::list_fonts(&self.fonts_dir);
                        let system = crate::fonts::system_families().iter().filter(|name| !local.iter().any(|own| own.eq_ignore_ascii_case(name)));
                        for name in local.iter().chain(system).filter(|name| name.to_lowercase().contains(&filter)) {
                            ui.selectable_value(&mut chosen, name.clone(), name.as_str());
                        }
                    });
                ui.add(egui::TextEdit::singleline(&mut editor.font_filter).hint_text("Filter fonts").desired_width(120.0));
                if ui.button("Import font…").clicked() {
                    match import_font(&self.fonts_dir) {
                        Ok(Some(name)) => chosen = name,
                        Ok(None) => {}
                        Err(error) => font_error = Some(error),
                    }
                }
                if missing && !downloading {
                    ui.colored_label(visuals.warn_fg_color, "Not installed");
                    if ui.button("Download").clicked() {
                        let source = get_path(&editor.table, &["font_source"]).cloned().and_then(|value| value.try_into().ok());
                        offer = Some((font.clone(), source));
                    }
                }
                if downloading {
                    ui.spinner();
                    ui.label("Downloading…");
                }
                if chosen != font {
                    if chosen.is_empty() {
                        editor.table.remove("font");
                    } else {
                        editor.table.insert("font".into(), toml::Value::String(chosen));
                    }
                    changed = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut editor.google_query).hint_text("Google Fonts family, e.g. Inter").desired_width(220.0));
                let query = editor.google_query.trim().to_owned();
                if ui.add_enabled(!downloading && !query.is_empty(), egui::Button::new("Get from Google Fonts")).clicked() {
                    editor.table.insert("font".into(), toml::Value::String(query.clone()));
                    editor.google_query.clear();
                    google = Some(query);
                    changed = true;
                }
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Colours").strong());
            egui::Grid::new("theme_editor_palette").num_columns(4).spacing([16.0, 8.0]).show(ui, |ui| {
                for (index, (key, label)) in PALETTE.iter().enumerate() {
                    ui.label(*label);
                    changed |= colour_button(ui, &mut editor.table, &["colors", key], current_colour(&visuals, "colors", key));
                    if index % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Widget states").id_salt("theme_editor_widgets").show(ui, |ui| {
                egui::Grid::new("theme_editor_widget_grid").spacing([16.0, 8.0]).show(ui, |ui| {
                    ui.label("");
                    for (_, label) in STATE_KEYS {
                        ui.label(egui::RichText::new(label).weak());
                    }
                    ui.end_row();
                    for (state, label) in STATES {
                        ui.label(label);
                        for (key, _) in STATE_KEYS {
                            changed |= colour_button(ui, &mut editor.table, &["widgets", state, key], current_colour(&visuals, state, key));
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if let Some(error) = font_error {
            self.theme_editor.error = Some(error);
        }
        if let Some((font, source)) = offer {
            self.offer_font(&ctx, font, source, true);
        }
        if let Some(family) = google {
            // typed in by the user, which is the approval
            self.start_font_download(family.clone(), vec![crate::fonts::Fetch::Google(family)], &ctx);
        }
        let now = ctx.input(|i| i.time);
        if changed {
            self.theme_editor.dirty = true;
        }
        if self.theme_editor.dirty {
            if now - self.theme_editor.saved_at >= SAVE_EVERY {
                self.save_theme_edit(&ctx, now);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(SAVE_EVERY));
            }
        }
    }

    /// Offers to download the equipped theme's font when it isn't installed.
    pub(super) fn offer_theme_font(&mut self, ctx: &egui::Context) {
        if let Some((font, source)) = theme::theme_font(&self.themes_dir, &self.config.settings.theme) {
            self.offer_font(ctx, font, source, false);
        }
    }

    /// Asks before downloading `font`. Without `ask_again`, a download the user turned down stays quiet.
    fn offer_font(&mut self, ctx: &egui::Context, font: String, source: Option<crate::fonts::FontSource>, ask_again: bool) {
        use crate::fonts::Fetch;
        if self.theme_editor.download.is_some() || crate::fonts::installed(&self.fonts_dir, &font) {
            return;
        }
        // a theme without a source still gets a Google Fonts lookup by name
        let fetches = source.map(|source| source.fetches()).filter(|fetches| !fetches.is_empty()).unwrap_or_else(|| vec![Fetch::Google(font.clone())]);
        let key = fetches.iter().map(Fetch::key).collect::<Vec<_>>().join("|");
        if !ask_again && self.config.settings.declined_font_downloads.contains(&key) {
            return;
        }
        let places = fetches.iter().map(Fetch::describe).collect::<Vec<_>>().join(", or else ");
        let dialog = rfd::MessageDialog::new()
            .set_title("Download theme font")
            .set_level(rfd::MessageLevel::Info)
            .set_description(format!(
                "This theme uses the font \"{font}\", which isn't installed.\n\nDownload it from {places}?\n\nHebnix only keeps a .ttf or .otf font file from the download, in its fonts folder."
            ))
            .set_buttons(rfd::MessageButtons::YesNo);
        #[cfg(target_os = "windows")]
        let dialog = winutil::parent_message_dialog(dialog);
        if dialog.show() != rfd::MessageDialogResult::Yes {
            if !self.config.settings.declined_font_downloads.contains(&key) {
                self.config.settings.declined_font_downloads.push(key);
                self.save_config();
            }
            return;
        }
        self.start_font_download(font, fetches, ctx);
    }

    fn start_font_download(&mut self, family: String, fetches: Vec<crate::fonts::Fetch>, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let fonts_dir = self.fonts_dir.clone();
        let ctx = ctx.clone();
        self.console.write(format!("[Fonts] Downloading {family}…"));
        std::thread::spawn(move || {
            let mut errors = Vec::new();
            for fetch in &fetches {
                match crate::fonts::download(fetch, &family, &fonts_dir) {
                    Ok(host) => {
                        let _ = tx.send((family, Ok(host)));
                        ctx.request_repaint();
                        return;
                    }
                    Err(error) => errors.push(format!("{}: {error}", fetch.describe())),
                }
            }
            let _ = tx.send((family, Err(errors.join(" "))));
            ctx.request_repaint();
        });
        self.theme_editor.download = Some(rx);
    }

    /// Picks up a finished font download and reloads the theme so the font shows.
    pub(super) fn poll_font_download(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.theme_editor.download else {
            return;
        };
        let (family, result) = match rx.try_recv() {
            Ok(done) => done,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => ("the font".into(), Err("The download stopped.".into())),
        };
        self.theme_editor.download = None;
        match result {
            Ok(host) => {
                self.console.write(format!("[Fonts] Installed {family} (downloaded from {host})"));
                let theme_name = self.config.settings.theme.clone();
                self.change_theme(ctx, &theme_name);
            }
            Err(error) => {
                self.console.write(format!("[Fonts] Could not download {family}: {error}"));
                self.theme_editor.error = Some(format!("Could not download {family}: {error}"));
            }
        }
    }

    fn save_theme_edit(&mut self, ctx: &egui::Context, now: f64) {
        let editor = &mut self.theme_editor;
        editor.dirty = false;
        editor.saved_at = now;
        let target = if is_edited_copy(&editor.name) { editor.name.clone() } else { copy_name(&self.themes_dir, &editor.name) };
        let saved = toml::to_string(&editor.table)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                std::fs::create_dir_all(&self.themes_dir).map_err(|e| e.to_string())?;
                std::fs::write(self.themes_dir.join(format!("{target}.toml")), text).map_err(|e| e.to_string())
            });
        if let Err(error) = saved {
            editor.error = Some(format!("Could not save the theme: {error}"));
            return;
        }
        editor.error = None;
        if target != editor.name {
            editor.name = target.clone();
            self.theme_options = theme::list_themes(&self.themes_dir);
            self.change_theme(ctx, &target);
        } else {
            let _ = theme::apply_theme(ctx, &self.themes_dir, &self.fonts_dir, &target);
            theme::apply_window_opacity(ctx, self.config.settings.window_opacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_land_in_nested_tables_and_copies_get_fresh_names() {
        let mut table: toml::Table = "base = \"dark\"\n[colors]\naccent = \"#A177FF\"\n".parse().unwrap();
        set_path(&mut table, &["widgets", "hovered", "bg_fill"], toml::Value::String("#112233".into()));
        set_path(&mut table, &["colors", "text"], toml::Value::String("#FFFFFF".into()));
        let round: toml::Table = toml::to_string(&table).unwrap().parse().unwrap();
        assert_eq!(get_path(&round, &["widgets", "hovered", "bg_fill"]).and_then(toml::Value::as_str), Some("#112233"));
        assert_eq!(get_path(&round, &["colors", "accent"]).and_then(toml::Value::as_str), Some("#A177FF"));
        let parsed: crate::theme::ThemeFile = toml::from_str(&toml::to_string(&table).unwrap()).unwrap();
        assert_eq!(parsed.widgets.hovered.bg_fill.as_deref(), Some("#112233"));

        let dir = std::env::temp_dir().join(format!("hebnix-theme-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(copy_name(&dir, "glass"), "glass (edited)");
        std::fs::write(dir.join("glass (edited).toml"), "").unwrap();
        assert_eq!(copy_name(&dir, "glass"), "glass (edited 2)");
        assert!(is_edited_copy("glass (edited 2)") && !is_edited_copy("glass"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
