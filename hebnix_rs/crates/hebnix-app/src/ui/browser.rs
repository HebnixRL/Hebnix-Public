//! Shared content-only browsing layout. Does not style application navigation.
use crate::i18n::{t, t_args};
use eframe::egui;

pub(crate) const ROWS: usize = 3;

pub(crate) fn frame(ui: &egui::Ui, selected: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(if selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        })
        .stroke(if selected {
            egui::Stroke::new(2.0, ui.visuals().selection.stroke.color)
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        })
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(10)
}

pub(crate) fn fallback_image(base_dir: &std::path::Path) -> std::sync::Arc<[u8]> {
    std::fs::read(base_dir.join("assets").join("hebnix.png"))
        .unwrap_or_else(|_| include_bytes!("../../assets/hebnix.png").to_vec())
        .into()
}

pub(crate) fn columns_for_width(width: f32, gap: f32, minimum: f32) -> usize {
    ((width + gap) / (minimum + gap)).floor().clamp(1.0, 6.0) as usize
}

pub(crate) fn columns(ui: &egui::Ui, minimum: f32) -> usize {
    columns_for_width(
        ui.available_width() - ui.spacing().scroll.allocated_width(),
        ui.spacing().item_spacing.x,
        minimum,
    )
}

pub(crate) fn tile_height(ui: &egui::Ui) -> f32 {
    ((ui.available_height() - ui.spacing().item_spacing.y * (ROWS - 1) as f32) / ROWS as f32)
        .clamp(164.0, 260.0)
}

pub(crate) fn header(ui: &mut egui::Ui, title: String, actions: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        crate::theme::page_heading(ui, title);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
            actions,
        );
    });
}

pub(crate) fn pagination(ui: &mut egui::Ui, page: &mut usize, count: usize, page_size: usize) {
    let pages = count.div_ceil(page_size).max(1);
    *page = (*page).min(pages - 1);
    ui.horizontal_wrapped(|ui| {
        ui.weak(t_args(
            "tab-page-page-of-total-pages-filtered",
            &[
                ("page", (*page + 1).to_string().into()),
                ("total_pages", pages.to_string().into()),
                ("filtered", count.to_string().into()),
            ],
        ));
        if ui
            .add_enabled(*page > 0, egui::Button::new("|‹"))
            .on_hover_text(t("items-browser-first-page"))
            .clicked()
        {
            *page = 0;
        }
        if ui
            .add_enabled(*page > 0, egui::Button::new(t("ball-previous")))
            .clicked()
        {
            *page -= 1;
        }
        // A bounded window keeps pagination usable with large catalogues.
        let start = page.saturating_sub(1).min(pages.saturating_sub(3));
        for index in start..(start + 3).min(pages) {
            if ui
                .selectable_label(*page == index, (index + 1).to_string())
                .clicked()
            {
                *page = index;
            }
        }
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new(t("ball-next")))
            .clicked()
        {
            *page += 1;
        }
        if ui
            .add_enabled(*page + 1 < pages, egui::Button::new("›|"))
            .on_hover_text(t("items-browser-last-page"))
            .clicked()
        {
            *page = pages - 1;
        }
    });
    ui.add_space(6.0);
}

pub(crate) fn search(ui: &mut egui::Ui, input: &mut String, hint: String) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        // Reserve room for Clear; the field itself owns the search affordance.
        let clear_width = ui
            .painter()
            .layout_no_wrap(
                t("spoofer-clear"),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().text_color(),
            )
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
            + ui.spacing().item_spacing.x;
        let mut focused = false;
        let field = egui::Frame::new()
            .fill(ui.visuals().text_edit_bg_color())
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
                    let center = rect.center() - egui::vec2(2.0, 2.0);
                    let stroke = egui::Stroke::new(1.4, ui.visuals().weak_text_color());
                    ui.painter().circle_stroke(center, 4.5, stroke);
                    ui.painter().line_segment(
                        [center + egui::vec2(3.0, 3.0), center + egui::vec2(7.0, 7.0)],
                        stroke,
                    );
                    let width = (ui.available_width() - clear_width - 40.0).max(60.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(input)
                            .hint_text(hint)
                            .frame(egui::Frame::NONE)
                            .desired_width(width),
                    );
                    changed |= response.changed();
                    focused = response.has_focus();
                });
            });
        if focused {
            ui.painter().rect_stroke(
                field.response.rect,
                ui.visuals().widgets.inactive.corner_radius,
                egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if ui.button(t("spoofer-clear")).clicked() {
            input.clear();
            changed = true;
        }
    });
    changed
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::{
        config::{Config, PatchSource},
        patcher,
    };

    #[test]
    fn item_patching_browser_headers_fit_both_themes() {
        let root =
            std::env::temp_dir().join(format!("hebnix-patching-browser-{}", rand::random::<u64>()));
        let themes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        let (tx, _rx) = crossbeam_channel::unbounded();
        let cars_dir = root.join("cars/fixture");
        std::fs::create_dir_all(&cars_dir).unwrap();
        let mut upk = 0x9E2A83C1u32.to_le_bytes().to_vec();
        upk.extend(868u16.to_le_bytes());
        upk.extend(33u16.to_le_bytes());
        std::fs::write(cars_dir.join("fixture.upk"), upk).unwrap();
        let patches: serde_json::Map<String, serde_json::Value> = (0..24)
            .map(|index| {
                (
                    format!("Custom car {index:02}"),
                    serde_json::json!({"BodyID": 23, "MeshPath": "fixture.Body"}),
                )
            })
            .collect();
        std::fs::write(
            cars_dir.join("cars.json"),
            serde_json::to_vec(&patches).unwrap(),
        )
        .unwrap();
        for theme in ["Dark", "Ocean Studio Dark", "Hebnix 2"] {
            for width in [600.0, 834.0, 1034.0] {
                let ctx = egui::Context::default();
                crate::theme::apply_theme(&ctx, &themes, &themes, theme).unwrap();
                let mut config = Config::default();
                config.patcher.ball_source = PatchSource::Custom;
                config.patcher.boost_source = PatchSource::Custom;
                config.patcher.decal_source = PatchSource::Custom;
                config.patcher.car_source = PatchSource::Custom;
                let mut ball = patcher::ball::PatcherState::new(&root, &config);
                let mut boost = patcher::boost_patcher::BoostPatcherState::new(&root, &config);
                let mut decal = patcher::decal_patcher::DecalPatcherState::new(&root, &config);
                let mut car = patcher::car_patcher::CarPatcherState::new(&root, &config);
                car.set_catalog(serde_json::json!({"bodies": [{
                    "id": 23, "name": "Octane", "upk_path": "Body_Octane_SF.upk"
                }]}))
                .unwrap();
                let mut inventory = crate::owned_inventory::OwnedInventory::empty_for_ui_tests();
                let cooked = root.join("CookedPCConsole");
                let backups = cooked.join("Backups");
                for tab in 0..4 {
                    let mut output = egui::FullOutput::default();
                    for _ in 0..2 {
                        output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 700.0),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                egui::CentralPanel::default().show(ui, |ui| {
                                    match tab {
                                        0 => ball.render_ball_tab(
                                            ui,
                                            &cooked,
                                            &cooked.join("Mutators_Balls_SF.upk"),
                                            &backups,
                                            &tx,
                                            &ctx,
                                            &mut config,
                                        ),
                                        1 => boost.render_tab(
                                            ui,
                                            &cooked,
                                            &backups,
                                            &tx,
                                            &ctx,
                                            &mut config,
                                        ),
                                        2 => decal.render_tab(
                                            ui,
                                            &cooked,
                                            &backups,
                                            &tx,
                                            &ctx,
                                            &mut config,
                                            &mut inventory,
                                        ),
                                        _ => car.render_tab(
                                            ui,
                                            &ctx,
                                            &cooked,
                                            &backups,
                                            &tx,
                                            &mut config,
                                        ),
                                    }
                                    assert!(
                                        ui.min_rect().right() <= width + 1.0,
                                        "Patching header {tab} overflows at {theme}/{width}: {:?}",
                                        ui.min_rect()
                                    );
                                });
                            },
                        );
                    }
                    fn text(shape: &egui::Shape, needle: &str) -> bool {
                        match shape {
                            egui::Shape::Text(t) => t.galley.text() == needle,
                            egui::Shape::Vec(v) => v.iter().any(|s| text(s, needle)),
                            _ => false,
                        }
                    }
                    for key in ["spoofer-clear", "btn-refresh", "ball-import-zip"] {
                        assert!(
                            output.shapes.iter().any(|s| text(&s.shape, &t(key))),
                            "Missing control {key} in patching header {tab}"
                        );
                    }
                    if tab == 2 || tab == 3 {
                        assert!(
                            output
                                .shapes
                                .iter()
                                .any(|s| text(&s.shape, &t("ball-restore-original")))
                        );
                    }
                    if tab == 2 {
                        let find = |label: &str| {
                            output
                                .shapes
                                .iter()
                                .find_map(|shape| match &shape.shape {
                                    egui::Shape::Text(text) if text.galley.text() == label => {
                                        Some(text.visual_bounding_rect())
                                    }
                                    _ => None,
                                })
                                .unwrap()
                        };
                        let save = find("Owned items from save:");
                        let checkbox = find("Show only owned items");
                        assert!(
                            checkbox.right() > width - 35.0,
                            "Owned filter should align to the right"
                        );
                        if width >= 834.0 {
                            assert!(
                                (save.center().y - checkbox.center().y).abs() < 6.0,
                                "Owned controls should share the top row"
                            );
                        }
                    }
                    if tab == 3 {
                        let labels: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Text(text)
                                    if text.galley.text().starts_with("Custom car ") =>
                                {
                                    Some((text.visual_bounding_rect(), shape.clip_rect))
                                }
                                _ => None,
                            })
                            .collect();
                        let columns = columns_for_width(width - 16.0 - 6.0, 8.0, 224.0);
                        assert_eq!(
                            labels.len(),
                            columns * ROWS,
                            "Three full car rows at {theme}/{width}"
                        );
                        for (rect, clip) in labels {
                            assert!(
                                clip.expand(1.0).contains_rect(rect),
                                "Car label clipped at {theme}/{width}: {rect:?} / {clip:?}"
                            );
                        }
                        for shape in &output.shapes {
                            if let egui::Shape::Text(text) = &shape.shape {
                                if text.galley.text() == t("ball-apply")
                                    || text.galley.text() == t("presets-delete")
                                    || text.galley.text() == "Octane"
                                {
                                    assert!(
                                        shape
                                            .clip_rect
                                            .expand(1.0)
                                            .contains_rect(text.visual_bounding_rect()),
                                        "Car footer clipped at {theme}/{width}: {} {:?} / {:?}",
                                        text.galley.text(),
                                        text.visual_bounding_rect(),
                                        shape.clip_rect
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(&temp) && resolved != temp);
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
