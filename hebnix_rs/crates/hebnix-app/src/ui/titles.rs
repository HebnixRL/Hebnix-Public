//! Native title customisation. The patcher's transaction and restore paths stay separate.
use super::*;
use serde::{Deserialize, Serialize};

pub(super) fn hex_color([r, g, b]: [u8; 3]) -> String {
    format!("{r:02X}{g:02X}{b:02X}")
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct TitleEditorState {
    editing: Option<usize>,
    // An explicit switch saves the current draft. Returning to an entry restores it.
    drafts: Vec<(Option<usize>, TitleDraft)>,
    library_owned: bool,
    #[serde(skip)]
    library_cache: Option<LibraryCache>,
    #[serde(skip)]
    restore_confirmation: bool,
    #[serde(skip)]
    page_open: bool,
}

#[derive(Clone)]
struct LibraryCache {
    query: String,
    owned_only: bool,
    owned: std::collections::HashSet<String>,
    count: usize,
    indices: Arc<[usize]>,
}

impl TitleEditorState {
    pub(super) fn leave_page(&mut self) {
        self.page_open = false;
    }

    pub(super) fn invalidate_catalog(&mut self) {
        self.library_cache = None;
    }

    fn switch(
        &mut self,
        index: Option<usize>,
        current: TitleDraft,
        rules: &[TitleSpoofSettings],
    ) -> Option<TitleDraft> {
        if index == self.editing {
            return None;
        }
        self.drafts.retain(|(key, _)| *key != self.editing);
        self.drafts.push((self.editing, current));
        let next = self
            .drafts
            .iter()
            .find(|(key, _)| *key == index)
            .map(|(_, draft)| draft.clone())
            .or_else(|| index.and_then(|i| rules.get(i)).map(TitleDraft::from))
            .unwrap_or_else(|| TitleDraft {
                text: String::new(),
                color: [232; 3],
                glow: false,
                glow_color: [232; 3],
                target: None,
            });
        self.editing = index;
        Some(next)
    }

    fn library_indices(
        &mut self,
        catalog: &[TitleCatalogEntry],
        owned: &std::collections::HashSet<String>,
        query: String,
    ) -> Arc<[usize]> {
        if self.library_cache.as_ref().is_none_or(|cache| {
            cache.query != query
                || cache.owned_only != self.library_owned
                || cache.count != catalog.len()
                || (self.library_owned && &cache.owned != owned)
        }) {
            let indices = catalog
                .iter()
                .enumerate()
                .filter(|(_, title)| {
                    (!self.library_owned || owned.contains(&title.id.to_ascii_lowercase()))
                        && (query.is_empty()
                            || title.text.to_ascii_lowercase().contains(&query)
                            || title.id.to_ascii_lowercase().contains(&query))
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
                .into();
            self.library_cache = Some(LibraryCache {
                query,
                owned_only: self.library_owned,
                owned: if self.library_owned {
                    owned.clone()
                } else {
                    Default::default()
                },
                count: catalog.len(),
                indices,
            });
        }
        self.library_cache.as_ref().unwrap().indices.clone()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct TitleDraft {
    text: String,
    color: [u8; 3],
    glow: bool,
    glow_color: [u8; 3],
    target: Option<String>,
}

impl From<&TitleSpoofSettings> for TitleDraft {
    fn from(rule: &TitleSpoofSettings) -> Self {
        Self {
            text: rule.text.clone(),
            color: parse_hex_color(&rule.color),
            glow: rule.glow,
            glow_color: parse_hex_color(rule.glow_color.as_deref().unwrap_or(&rule.color)),
            target: rule.target_id.clone(),
        }
    }
}

fn same_target(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

/// Cached native galleys stay sharp at the current DPI. Only glyphs are repeated
/// for the measured halo approximation; no title-coloured surface is ever painted.
fn title_text(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    glow: Option<Color32>,
    height: f32,
) -> egui::Response {
    title_text_sized(
        ui,
        text,
        color,
        glow,
        egui::vec2(ui.available_width(), height),
    )
}

fn title_text_sized(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    glow: Option<Color32>,
    size: egui::Vec2,
) -> egui::Response {
    let height = size.y;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let icons = crate::patcher::title_icons::loaded(ui.ctx());
        let mut display = String::with_capacity(text.len());
        let mut inline = Vec::new();
        let mut remaining = text;
        while let Some(start) = remaining.find('{') {
            display.push_str(&remaining[..start]);
            remaining = &remaining[start..];
            if let Some(end) = remaining.find('}') {
                let token = &remaining[..=end];
                if let Some(icon) = icons.get(token) {
                    inline.push((display.chars().count(), icon));
                    display.push('\u{2003}');
                    remaining = &remaining[end + 1..];
                    continue;
                }
            }
            display.push('{');
            remaining = &remaining[1..];
        }
        display.push_str(remaining);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple(
            display,
            font.clone(),
            Color32::PLACEHOLDER,
            (rect.width() - 12.0).max(1.0),
        );
        job.wrap.max_rows = if height >= font.size * 2.5 { 2 } else { 1 };
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let pos = egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0);
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        if let Some(glow) = glow {
            // Fixed work per visible row; offsets are in points and rasterized at native DPI.
            for (radius, alpha) in [(3.0, 12), (2.0, 22), (1.0, 36)] {
                let halo = Color32::from_rgba_unmultiplied(glow.r(), glow.g(), glow.b(), alpha);
                for direction in 0..8 {
                    let angle = direction as f32 * std::f32::consts::TAU / 8.0;
                    painter.galley(
                        pos + egui::vec2(angle.cos(), angle.sin()) * radius,
                        galley.clone(),
                        halo,
                    );
                }
            }
        }
        for (index, icon) in inline {
            let cursor = galley.pos_from_cursor(egui::text::CCursor::new(index));
            let next = galley.pos_from_cursor(egui::text::CCursor::new(index + 1));
            // Elided tokens have no character slot and must not paint past the ellipsis.
            if galley.elided && cursor == next {
                continue;
            }
            let size = icon.size_vec2();
            let limit = egui::vec2(
                (next.left() - cursor.left()).max(font.size),
                font.size * 1.15,
            );
            let scale = (limit.x / size.x).min(limit.y / size.y);
            let bounds = egui::Rect::from_center_size(
                pos + cursor.center().to_vec2() + egui::vec2(limit.x / 2.0, 0.0),
                size * scale,
            );
            painter.image(
                icon.id(),
                bounds,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.galley(pos, galley, color);
    }
    response.on_hover_text(text)
}

fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

fn column(ui: &mut egui::Ui, width: f32, height: f32, content: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            content(ui);
        },
    );
}

impl HebnixApp {
    fn title_draft(&self) -> TitleDraft {
        TitleDraft {
            text: self.spoofer_title.clone(),
            color: self.spoofer_title_color,
            glow: self.spoofer_title_glow,
            glow_color: self.spoofer_title_glow_color,
            target: self.spoofer_title_target.clone(),
        }
    }

    fn load_title_draft(&mut self, draft: TitleDraft) {
        self.spoofer_title = draft.text;
        self.spoofer_title_color = draft.color;
        self.spoofer_title_glow = draft.glow;
        self.spoofer_title_glow_color = draft.glow_color;
        self.spoofer_title_target = draft.target;
    }

    fn switch_title_editor(&mut self, index: Option<usize>) {
        let draft = self.title_draft();
        if let Some(next) = self.title_editor.switch(index, draft, &self.spoofer_titles) {
            self.load_title_draft(next);
        }
    }

    fn remove_title_entry(&mut self, index: usize) {
        if self.title_editor.editing == Some(index) {
            self.title_editor.editing = None;
        } else if self.title_editor.editing.is_some_and(|i| i > index) {
            self.title_editor.editing = self.title_editor.editing.map(|i| i - 1);
        }
        self.title_editor
            .drafts
            .retain(|(key, _)| *key != Some(index));
        for (key, _) in &mut self.title_editor.drafts {
            if key.is_some_and(|i| i > index) {
                *key = key.map(|i| i - 1);
            }
        }
        self.spoofer_titles.remove(index);
    }

    fn title_rules(&self) -> Vec<crate::patcher::title_patcher::TitleOverride> {
        self.spoofer_titles
            .iter()
            .map(|title| {
                let packed = |value: &str| {
                    let [r, g, b] = parse_hex_color(value);
                    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
                };
                crate::patcher::title_patcher::TitleOverride {
                    target_id: title.target_id.clone(),
                    original: title
                        .target_id
                        .as_ref()
                        .and_then(|id| {
                            self.title_catalog
                                .iter()
                                .find(|entry| entry.id.eq_ignore_ascii_case(id))
                        })
                        .map(|entry| crate::patcher::title_patcher::TitleAppearance {
                            text: entry.text.clone(),
                            color: entry.color.as_deref().map(packed),
                            glow_color: entry.glow.as_deref().map(packed).unwrap_or(0),
                        }),
                    text: title.text.trim().to_string(),
                    color: packed(&title.color),
                    glow: title.glow,
                    glow_color: title.glow_color.as_deref().map(packed),
                }
            })
            .collect()
    }

    pub(super) fn render_titles_page(&mut self, ui: &mut egui::Ui) {
        if !self.title_editor.page_open {
            self.title_editor.page_open = true;
            self.reload_catalogs(ui.ctx());
        }
        crate::patcher::title_icons::prepare(
            ui.ctx(),
            std::path::Path::new(&self.config.settings.rl_path),
        );
        self.render_titles_content(ui);
        self.render_title_restore_confirmation(ui.ctx());
    }

    fn render_titles_content(&mut self, ui: &mut egui::Ui) {
        // Tolerate settings edited outside the app without losing the current draft.
        if self
            .title_editor
            .editing
            .is_some_and(|i| i >= self.spoofer_titles.len())
        {
            self.title_editor.editing = None;
        }
        self.title_editor
            .drafts
            .retain(|(key, _)| key.is_none_or(|i| i < self.spoofer_titles.len()));
        self.title_patcher.poll();
        crate::theme::page_heading(ui, "Titles");
        self.owned_inventory.show(ui);
        let mut changed = !self.spoofer_title_enabled;
        self.spoofer_title_enabled = true;
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!(
                "{} owned titles in the selected save",
                self.owned_inventory.title_ids.len()
            ));
            changed |= ui
                .checkbox(&mut self.title_only_owned, "Show only owned replacements")
                .changed();
        });
        if self.title_patcher.busy() {
            ui.label("Title patch operation in progress. Editing will resume when it finishes.");
        }
        ui.add_space(6.0);
        // Columns always divide the available viewport. Only the two lists scroll.
        let total = ui.available_width();
        let height = ui.available_height().max(0.0);
        let gap = (total * 0.018).clamp(8.0, 18.0).floor();
        let usable = (total - gap * 2.0).max(0.0);
        let left = (usable * 0.31).floor();
        let middle = (usable * 0.34).floor();
        let right = (usable - left - middle).floor();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.horizontal_top(|ui| {
                column(ui, left, height, |ui| {
                    self.render_title_library(ui, &mut changed)
                });
                column(ui, middle, height, |ui| {
                    self.render_title_editor(ui, &mut changed)
                });
                column(ui, right, height, |ui| {
                    self.render_applied_titles(ui, &mut changed)
                });
            });
        });
        if changed {
            self.save_config();
        }
    }

    fn render_title_library(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        ui.heading("Copy Title");
        ui.label("Browse titles and copy one to edit.");
        ui.add(
            egui::TextEdit::singleline(&mut self.spoofer_title_copy_filter)
                .hint_text("Search titles")
                .desired_width(f32::INFINITY),
        );
        egui::ComboBox::from_id_salt("title_library_filter")
            .selected_text(if self.title_editor.library_owned {
                "Owned Titles"
            } else {
                "All Titles"
            })
            .show_ui(ui, |ui| {
                *changed |= ui
                    .selectable_value(&mut self.title_editor.library_owned, false, "All Titles")
                    .changed();
                *changed |= ui
                    .selectable_value(&mut self.title_editor.library_owned, true, "Owned Titles")
                    .changed();
            });
        ui.separator();
        if let Some(error) = &self.catalogs_error {
            ui.label(error);
            if ui
                .add_enabled(
                    !self.catalogs_loading,
                    egui::Button::new("Refresh title library"),
                )
                .clicked()
            {
                self.reload_catalogs(ui.ctx());
            }
        }
        if self.title_catalog.is_empty() {
            ui.label(if self.catalogs_loading {
                "Loading title library…"
            } else {
                "Title library unavailable."
            });
            return;
        }
        let query = self.spoofer_title_copy_filter.trim().to_ascii_lowercase();
        let visible = self.title_editor.library_indices(
            &self.title_catalog,
            &self.owned_inventory.title_ids,
            query,
        );
        if visible.is_empty() {
            ui.label("No matching titles. Try All Titles or refresh owned items.");
        }
        let row_height = (egui::TextStyle::Body.resolve(ui.style()).size + 10.0).max(26.0);
        let copy_font = egui::TextStyle::Button.resolve(ui.style());
        let copy_width = ui
            .painter()
            .layout_no_wrap("Copy".into(), copy_font.clone(), Color32::WHITE)
            .size()
            .x
            .ceil()
            + ui.spacing().button_padding.x * 2.0;
        let mut copied = None;
        egui::ScrollArea::vertical()
            .id_salt("title_library_rows")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, visible.len(), |ui, range| {
                for row in range {
                    let title = &self.title_catalog[visible[row]];
                    ui.push_id(&title.id, |ui| {
                        ui.horizontal(|ui| {
                            let preview_width =
                                (ui.available_width() - copy_width - ui.spacing().item_spacing.x)
                                    .max(1.0);
                            title_text_sized(
                                ui,
                                &title.text,
                                title.game_color(),
                                title.glow.as_deref().map(|s| rgb(parse_hex_color(s))),
                                egui::vec2(preview_width, row_height),
                            );
                            if ui
                                .add_enabled(
                                    !self.title_patcher.busy(),
                                    egui::Button::new(
                                        egui::RichText::new("Copy").font(copy_font.clone()),
                                    )
                                    .min_size(egui::vec2(copy_width, row_height)),
                                )
                                .on_hover_text(format!("Copy {} into the editor", title.text))
                                .clicked()
                            {
                                copied = Some(title.clone());
                            }
                        });
                    });
                }
            });
        if let Some(title) = copied {
            self.spoofer_title_copy = Some(title.id);
            self.spoofer_title = title.text.chars().take(64).collect();
            self.spoofer_title_color = title
                .color
                .as_deref()
                .map(parse_hex_color)
                .unwrap_or([255; 3]);
            self.spoofer_title_glow = title.glow.is_some();
            self.spoofer_title_glow_color = title
                .glow
                .as_deref()
                .map(parse_hex_color)
                .unwrap_or(self.spoofer_title_color);
            *changed = true;
        }
    }

    fn matching_title_entry(&self) -> Option<usize> {
        self.spoofer_titles.iter().position(|rule| {
            same_target(
                rule.target_id.as_deref(),
                self.spoofer_title_target.as_deref(),
            )
        })
    }

    pub(super) fn select_title_target(&mut self, target: Option<String>) {
        let existing = self
            .spoofer_titles
            .iter()
            .position(|rule| same_target(rule.target_id.as_deref(), target.as_deref()));
        if existing.is_some() {
            self.switch_title_editor(existing);
        } else if self.title_editor.editing.is_some() {
            let current = self.title_draft();
            let previous = self.title_editor.editing;
            self.title_editor.drafts.retain(|(key, _)| *key != previous);
            self.title_editor.drafts.push((previous, current.clone()));
            // Restore the separate new-title draft if one exists; otherwise keep
            // the current appearance ready for the newly selected target.
            if let Some((_, draft)) = self
                .title_editor
                .drafts
                .iter()
                .find(|(key, _)| key.is_none())
                .cloned()
            {
                self.load_title_draft(draft);
            }
            self.title_editor.editing = None;
        }
        self.spoofer_title_target = target;
    }

    fn render_title_editor(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        ui.heading("Edit Title");
        ui.add_space(4.0);
        ui.add_enabled_ui(!self.title_patcher.busy(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Custom title");
                ui.weak(format!("{} / 64", self.spoofer_title.chars().count()));
            });
            *changed |= ui
                .add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut self.spoofer_title)
                        .char_limit(64)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Text colour");
                *changed |= ui
                    .color_edit_button_srgb(&mut self.spoofer_title_color)
                    .changed();
                egui::ComboBox::from_id_salt("title_add_symbol")
                    .selected_text("Add Symbol")
                    .width(ui.available_width().min(150.0))
                    .height(260.0)
                    .show_ui(ui, |ui| {
                        let icons = crate::patcher::title_icons::loaded(ui.ctx());
                        let mut symbols: Vec<_> = icons.iter().collect();
                        symbols.sort_by(|(a, _), (b, _)| a.cmp(b));
                        if symbols.is_empty() {
                            ui.weak("Title symbols are not available yet.");
                        }
                        let length = self.spoofer_title.chars().count();
                        for (token, icon) in symbols {
                            let fits = length + token.chars().count() <= 64;
                            let dimensions = icon.size_vec2();
                            let scale = (20.0 / dimensions.x).min(20.0 / dimensions.y);
                            let response = ui.add_enabled(
                                fits,
                                egui::Button::image_and_text(
                                    egui::Image::new((icon.id(), dimensions * scale)),
                                    token.as_str(),
                                )
                                .frame(false)
                                .min_size(egui::vec2(ui.available_width(), 26.0)),
                            );
                            if response
                                .on_disabled_hover_text(
                                    "This symbol would exceed the 64-character title limit.",
                                )
                                .clicked()
                            {
                                self.spoofer_title.push_str(token);
                                *changed = true;
                                ui.close();
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                *changed |= ui.checkbox(&mut self.spoofer_title_glow, "Glow").changed();
                *changed |= ui
                    .add_enabled_ui(self.spoofer_title_glow, |ui| {
                        ui.color_edit_button_srgb(&mut self.spoofer_title_glow_color)
                            .on_hover_text("Glow colour")
                    })
                    .inner
                    .changed();
            });
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(ui.visuals().extreme_bg_color)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    title_text(
                        ui,
                        &self.spoofer_title,
                        rgb(self.spoofer_title_color),
                        self.spoofer_title_glow
                            .then(|| rgb(self.spoofer_title_glow_color)),
                        40.0,
                    );
                });
            ui.add_space(12.0);
            ui.label("Title to replace");
            let selected = self
                .spoofer_title_target
                .as_ref()
                .map(|id| {
                    self.title_catalog
                        .iter()
                        .find(|t| t.id.eq_ignore_ascii_case(id))
                        .map(|t| t.text.as_str())
                        .unwrap_or(id)
                })
                .unwrap_or(if self.title_only_owned {
                    "Choose an owned title"
                } else {
                    "All titles (fallback)"
                })
                .to_string();
            let mut target = self.spoofer_title_target.clone();
            egui::ComboBox::from_id_salt("title_spoof_target")
                .width((ui.available_width() - 8.0).max(1.0))
                .truncate()
                .selected_text(selected)
                .height(240.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.spoofer_title_filter)
                            .hint_text("Search replacement titles")
                            .desired_width(ui.available_width()),
                    );
                    if !self.title_only_owned {
                        if ui
                            .selectable_value(&mut target, None, "All titles (fallback)")
                            .clicked()
                        {
                            ui.close();
                        }
                    }
                    let query = self.spoofer_title_filter.trim().to_ascii_lowercase();
                    for title in &self.title_catalog {
                        if (!self.title_only_owned
                            || self
                                .owned_inventory
                                .title_ids
                                .contains(&title.id.to_ascii_lowercase()))
                            && (query.is_empty()
                                || title.text.to_ascii_lowercase().contains(&query))
                        {
                            ui.push_id(&title.id, |ui| {
                                let size = egui::vec2(
                                    ui.available_width(),
                                    (egui::TextStyle::Body.resolve(ui.style()).size + 10.0)
                                        .max(26.0),
                                );
                                let selected = target
                                    .as_deref()
                                    .is_some_and(|id| id.eq_ignore_ascii_case(&title.id));
                                let response =
                                    ui.add_sized(size, egui::Button::selectable(selected, ""));
                                let mut preview = ui.new_child(
                                    egui::UiBuilder::new()
                                        .id_salt("title_option_preview")
                                        .max_rect(response.rect)
                                        .layout(egui::Layout::top_down(egui::Align::Min)),
                                );
                                title_text_sized(
                                    &mut preview,
                                    &title.text,
                                    title.game_color(),
                                    title
                                        .glow
                                        .as_deref()
                                        .map(|value| rgb(parse_hex_color(value))),
                                    response.rect.size(),
                                );
                                if response.clicked() {
                                    target = Some(title.id.clone());
                                    ui.close();
                                }
                            });
                        }
                    }
                });
            if target != self.spoofer_title_target {
                self.select_title_target(target);
                *changed = true;
            }
            let existing = self.matching_title_entry();
            let owned = self.spoofer_title_target.as_ref().is_some_and(|id| {
                self.owned_inventory
                    .title_ids
                    .contains(&id.to_ascii_lowercase())
            });
            let reason = if self.spoofer_title.trim().is_empty() {
                Some("Enter your title text.")
            } else if self.spoofer_title.chars().count() > 64 || self.spoofer_title.contains('\0') {
                Some("Use 1–64 characters without null characters.")
            } else if self.title_only_owned && !owned {
                Some("Choose an owned title, or turn off the owned-only filter.")
            } else if existing.is_none() && self.spoofer_titles.len() >= 64 {
                Some("The list supports up to 64 replacements.")
            } else {
                None
            };
            ui.add_space(8.0);
            let label = if existing.is_some() {
                "Edit Title"
            } else {
                "Add Title"
            };
            let action = egui::Button::new(egui::RichText::new(label).strong())
                .min_size(egui::vec2(ui.available_width(), 34.0));
            if ui.add_enabled(reason.is_none(), action).clicked() {
                let entry = TitleSpoofSettings {
                    text: self.spoofer_title.trim().to_string(),
                    color: hex_color(self.spoofer_title_color),
                    glow: self.spoofer_title_glow,
                    glow_color: Some(hex_color(self.spoofer_title_glow_color)),
                    target_id: self.spoofer_title_target.clone(),
                };
                if let Some(index) = existing {
                    self.spoofer_titles[index] = entry;
                    self.title_editor.editing = Some(index);
                } else {
                    self.spoofer_titles.push(entry);
                    self.title_editor.editing = Some(self.spoofer_titles.len() - 1);
                }
                self.title_editor.drafts.retain(|(key, _)| {
                    *key != self.title_editor.editing && (existing.is_some() || key.is_some())
                });
                *changed = true;
            }
            if let Some(reason) = reason {
                ui.weak(reason);
            }
        });
    }

    fn render_applied_titles(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        ui.heading("Applied Titles");
        ui.weak("Apply your list to update this client.");
        ui.add_space(4.0);
        let footer_height = (egui::TextStyle::Body.resolve(ui.style()).size * 6.5).max(108.0);
        let list_height = (ui.available_height() - footer_height).max(40.0);
        let mut edit = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("applied_title_rows")
            .max_height(list_height)
            .min_scrolled_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.spoofer_titles.is_empty() {
                    ui.weak("No title replacements added yet");
                }
                let font = egui::TextStyle::Body.resolve(ui.style()).size;
                let button_font = egui::TextStyle::Button.resolve(ui.style());
                let button_width = |label: &str| {
                    ui.painter()
                        .layout_no_wrap(label.to_owned(), button_font.clone(), Color32::WHITE)
                        .size()
                        .x
                        .ceil()
                        + ui.spacing().button_padding.x * 2.0
                };
                let edit_width = button_width("Edit");
                let remove_width = button_width("Remove");
                for (index, rule) in self.spoofer_titles.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        // Bound the row itself. A right-to-left layout in an unbounded
                        // scroll child expands to the viewport's entire height.
                        let target = rule
                            .target_id
                            .as_ref()
                            .map(|id| {
                                self.title_catalog
                                    .iter()
                                    .find(|title| title.id.eq_ignore_ascii_case(id))
                                    .map(|title| title.text.as_str())
                                    .unwrap_or(id)
                            })
                            .unwrap_or("All titles (fallback)");
                        let warning = rule.target_id.as_ref().and_then(|id| {
                            if !self.title_catalog.is_empty()
                                && !self
                                    .title_catalog
                                    .iter()
                                    .any(|t| t.id.eq_ignore_ascii_case(id))
                            {
                                Some("Target unavailable in the current catalog")
                            } else if !self
                                .owned_inventory
                                .title_ids
                                .contains(&id.to_ascii_lowercase())
                            {
                                Some("Not owned in the selected save")
                            } else {
                                None
                            }
                        });
                        let line = (font + 8.0).max(ui.spacing().interact_size.y);
                        let height = line * 2.0
                            + ui.spacing().item_spacing.y
                            + warning.map_or(0.0, |_| font + 4.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                title_text(
                                    ui,
                                    &rule.text,
                                    rgb(parse_hex_color(&rule.color)),
                                    rule.glow.then(|| {
                                        rgb(parse_hex_color(
                                            rule.glow_color.as_deref().unwrap_or(&rule.color),
                                        ))
                                    }),
                                    line,
                                );
                                ui.horizontal(|ui| {
                                    let width = (ui.available_width()
                                        - edit_width
                                        - remove_width
                                        - ui.spacing().item_spacing.x * 2.0)
                                        .max(1.0);
                                    title_text_sized(
                                        ui,
                                        target,
                                        ui.visuals().weak_text_color(),
                                        None,
                                        egui::vec2(width, line),
                                    );
                                    ui.add_enabled_ui(!self.title_patcher.busy(), |ui| {
                                        if ui
                                            .add_sized(
                                                [edit_width, line],
                                                egui::Button::new(
                                                    egui::RichText::new("Edit")
                                                        .font(button_font.clone()),
                                                ),
                                            )
                                            .on_hover_text("Edit this replacement")
                                            .clicked()
                                        {
                                            edit = Some(index);
                                        }
                                        if ui
                                            .add_sized(
                                                [remove_width, line],
                                                egui::Button::new(
                                                    egui::RichText::new("Remove")
                                                        .font(button_font.clone()),
                                                ),
                                            )
                                            .on_hover_text("Remove this replacement")
                                            .clicked()
                                        {
                                            remove = Some(index);
                                        }
                                    });
                                });
                                if let Some(warning) = warning {
                                    ui.small(warning);
                                }
                            },
                        );
                        ui.separator();
                    });
                }
            });
        if let Some(index) = edit {
            self.switch_title_editor(Some(index));
            *changed = true;
        }
        if let Some(index) = remove {
            self.remove_title_entry(index);
            *changed = true;
        }
        ui.add_space(6.0);
        let rules = self.title_rules();
        let validation = crate::patcher::title_patcher::validate_rules(&rules).err();
        let cooked = std::path::Path::new(&self.config.settings.rl_path)
            .join("TAGame")
            .join("CookedPCConsole");
        if let Some(matches) = self.title_patcher.applied_matches(&cooked, &rules) {
            if matches {
                ui.colored_label(
                    egui::Color32::from_rgb(100, 210, 140),
                    egui::RichText::new("Applied").strong(),
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(235, 185, 90),
                    "Changes pending — apply to update",
                );
            }
        }
        let closed = !self.last_rl_open && !self.first_status;
        let ready = closed && !self.title_patcher.busy();
        ui.horizontal(|ui| {
            let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            if ui
                .add_enabled_ui(ready && validation.is_none(), |ui| {
                    ui.add_sized([width, 28.0], egui::Button::new("Apply"))
                })
                .inner
                .on_hover_text("Apply title changes")
                .clicked()
            {
                let cooked = std::path::Path::new(&self.config.settings.rl_path)
                    .join("TAGame")
                    .join("CookedPCConsole");
                self.title_patcher.start(
                    self.base_dir.clone(),
                    cooked,
                    Some(rules),
                    ui.ctx().clone(),
                );
            }
            if ui
                .add_enabled_ui(ready, |ui| {
                    ui.add_sized([width, 28.0], egui::Button::new("Restore"))
                })
                .inner
                .on_hover_text("Restore original title display")
                .clicked()
            {
                self.title_editor.restore_confirmation = true;
            }
        });
        egui::ScrollArea::vertical().id_salt("titles_patch_status").min_scrolled_height(0.0).max_height(ui.available_height().max(0.0)).auto_shrink([false, false]).show(ui, |ui| {
            if !closed { ui.weak(if self.first_status { "Checking whether Rocket League is running…" } else { "Close Rocket League to Apply or Restore." }); }
            else if let Some(error) = validation { if !self.spoofer_titles.is_empty() { ui.label(error); } }
            if self.title_patcher.busy() {
                ui.horizontal(|ui| {
                    let spinner_size = 16.0;
                    let width = (ui.available_width() - spinner_size - ui.spacing().item_spacing.x).max(1.0);
                    ui.allocate_ui_with_layout(egui::vec2(width, spinner_size), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.label(&self.title_patcher.status);
                    });
                    ui.add(egui::Spinner::new().size(spinner_size));
                });
            } else if !self.title_patcher.status.is_empty() {
                ui.label(&self.title_patcher.status);
            }
            if !spoofer::is_admin() && self.title_patcher.status.contains("os error 5") {
                ui.label("Windows denied file access. Restart as administrator, then retry Apply or Restore.");
                if ui.button("Restart as administrator for title patching").clicked() {
                    self.save_config();
                    if spoofer::spawn_elevated_relaunch() {
                        self.spoofer_mgr.shutdown(); self.workshop.suspend_multiplayer(); std::process::exit(0);
                    }
                    self.title_patcher.status = "Could not restart as administrator.".into();
                }
            }
        });
    }

    fn render_title_restore_confirmation(&mut self, ctx: &egui::Context) {
        if !self.title_editor.restore_confirmation {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        let ready = !self.last_rl_open && !self.first_status && !self.title_patcher.busy();
        let modal = egui::Modal::new(egui::Id::new("restore_title_changes")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading("Restore title changes?");
            ui.label("Restore the original title display on this client? Your replacement list will be kept.");
            if !ready { ui.label("Close Rocket League and wait for the current operation to finish before restoring."); }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                confirm = ui.add_enabled(ready, egui::Button::new("Restore titles")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.title_editor.restore_confirmation = false;
        }
        if confirm {
            self.title_editor.restore_confirmation = false;
            let cooked = std::path::Path::new(&self.config.settings.rl_path)
                .join("TAGame")
                .join("CookedPCConsole");
            self.title_patcher
                .start(self.base_dir.clone(), cooked, None, ctx.clone());
        }
    }
}
