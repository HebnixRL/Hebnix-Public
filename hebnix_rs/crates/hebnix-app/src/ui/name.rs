//! Theme-native display name editor and recent-name history.
use super::*;

impl HebnixApp {
    pub(super) fn render_name_page(&mut self, ui: &mut egui::Ui) {
        let height_id = ui.make_persistent_id("name_content_height");
        egui::ScrollArea::vertical()
            .id_salt("name_page_scroll")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let available_width = ui.available_width();
                let margin = ui
                    .spacing()
                    .item_spacing
                    .x
                    .max(12.0)
                    .min(available_width / 4.0);
                let content_width = (available_width - 2.0 * margin).clamp(0.0, 760.0);
                let previous_height = ui
                    .data_mut(|data| data.get_temp::<f32>(height_id))
                    .unwrap_or(0.0);
                // Measure the real themed content, including wrapped guidance. A second
                // layout pass centers it correctly after font, scale, or window changes.
                let top = ((viewport.height() - previous_height) / 2.0).max(0.0);
                let origin =
                    ui.cursor().min + egui::vec2((available_width - content_width) / 2.0, top);
                let response = ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt("name_content")
                        .max_rect(egui::Rect::from_min_size(
                            origin,
                            egui::vec2(content_width, 0.0),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        ui.set_width(content_width);
                        self.render_name_content(ui, content_width);
                    },
                );
                let height = response.response.rect.height();
                if (height - previous_height).abs() > 0.5 {
                    ui.data_mut(|data| data.insert_temp(height_id, height));
                    ui.ctx().request_discard("Name content height changed");
                }
            });
    }

    fn render_name_content(&mut self, ui: &mut egui::Ui, content_width: f32) {
        let spacing = ui.spacing().item_spacing;
        let padding = ui.spacing().button_padding;
        let field_height =
            36.0_f32.max(ui.text_style_height(&egui::TextStyle::Body) + 2.0 * padding.y);
        let button_height = 32.0_f32
            .max(ui.spacing().interact_size.y)
            .max(ui.text_style_height(&egui::TextStyle::Button) + 2.0 * padding.y);
        let column_gap = spacing.x.min(content_width / 4.0);
        let button_width = (content_width - 2.0 * column_gap) / 3.0;
        // Keep this frame's buttons stable when focus loss commits a new name.
        let recent_names = self.spoofer_username_history.clone();
        let mut completed = false;
        let mut chosen_name = None;

        crate::theme::page_heading(ui, "Name");
        ui.weak("Choose your display name");
        ui.add_space(spacing.y * 2.0);

        ui.scope(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "{} / {}",
                            self.spoofer_username.chars().count(),
                            spoofer::MAX_NAME_LENGTH,
                        ))
                        .small()
                        .weak(),
                    )
                    .extend(),
                );
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Checkbox::new(
                            &mut self.spoofer_username_enabled,
                            "Display Name",
                        ))
                        .changed()
                    {
                        if self.spoofer_username_enabled {
                            self.remember_username_spoof();
                        }
                        self.save_config();
                        self.evaluate_proxies();
                    }
                });
            });

            ui.scope(|ui| {
                let text_response = ui.add_sized(
                    [content_width, field_height],
                    egui::TextEdit::singleline(&mut self.spoofer_username)
                        .id_salt("display_name_input")
                        .char_limit(spoofer::MAX_NAME_LENGTH)
                        .desired_width(content_width)
                        .horizontal_align(egui::Align::Center)
                        .vertical_align(egui::Align::Center),
                );
                if text_response.changed() {
                    self.spoofer_mgr.set_username(
                        if self.http_spoofer_ready() && self.spoofer_username_enabled {
                            &self.spoofer_username
                        } else {
                            ""
                        },
                    );
                }
                completed = text_response.lost_focus();

                ui.add_space(spacing.y * 2.0);
                ui.label("Previous names");
                if recent_names.is_empty() {
                    ui.weak("No previous names yet.");
                } else {
                    egui::Grid::new("name_history_grid")
                        .num_columns(3)
                        .min_col_width(button_width)
                        .max_col_width(button_width)
                        .spacing(egui::vec2(column_gap, spacing.y))
                        .show(ui, |ui| {
                            for index in 0..USERNAME_HISTORY_LIMIT {
                                if let Some(name) = recent_names.get(index) {
                                    ui.push_id(name, |ui| {
                                        if ui
                                            .add_sized(
                                                [button_width, button_height],
                                                egui::Button::new(name)
                                                    .truncate()
                                                    .selected(name == &self.spoofer_username),
                                            )
                                            .on_hover_text(name)
                                            .clicked()
                                        {
                                            chosen_name = Some(name.clone());
                                        }
                                    });
                                } else {
                                    ui.allocate_space(egui::vec2(button_width, button_height));
                                }
                                if index % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                }
            });
        });

        // Process completion after drawing history so committing the draft cannot
        // shift another name under a click in this same frame.
        if completed {
            self.remember_username_spoof();
            self.save_config();
        }
        if let Some(name) = chosen_name {
            self.spoofer_username = name;
            self.spoofer_mgr.set_username(
                if self.http_spoofer_ready() && self.spoofer_username_enabled {
                    &self.spoofer_username
                } else {
                    ""
                },
            );
            self.remember_username_spoof();
            self.save_config();
        }

        ui.add_space(spacing.y * 2.0);
        ui.label(
            egui::RichText::new("Names are saved automatically.")
                .small()
                .weak(),
        );
    }
}
