//! layout helpers so rows don't clip when a translation runs longer than english.

use eframe::egui::{self, Color32};

/// Shared gap between vertical subtab separators and their content.
pub fn subtab_content_frame() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin {
        left: 5,
        ..Default::default()
    })
}

/// Frame a full-width section inside a page or scroll area.
pub fn bordered_panel(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        content(ui);
    });
}

/// Reserve one label column while keeping the text against its left edge.
pub fn settings_label(
    ui: &mut egui::Ui,
    width: f32,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, 20.0));
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

/// width for a settings page's label column: the widest label on that page,
/// never narrower than `min` (the old fixed width), capped so one huge label
/// can't eat the row. keeps the inputs next to the labels lined up.
pub fn label_column_width(ui: &egui::Ui, labels: &[String], min: f32) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let widest = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label.clone(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    (widest + 8.0).clamp(min, min.max(320.0))
}
