//! Presentation helpers scoped to the Item Swapper working area.
use super::*;

pub(super) use crate::ui::browser::{ROWS, TILE_HEIGHT, frame, pagination, search};

pub(super) fn columns(ui: &egui::Ui, manual: bool) -> usize {
    crate::ui::browser::columns(ui, if manual { 224.0 } else { 150.0 })
}

pub(super) fn replacement_text(ui: &egui::Ui, label: &str, width: f32) -> Arc<egui::Galley> {
    // ComboBox::width is a minimum. Bound the galley itself so even a very long
    // selected name cannot make the closed selector wider than its card.
    let text_width = (width
        - ui.spacing().button_padding.x * 2.0
        - ui.spacing().icon_width
        - ui.spacing().icon_spacing
        - 4.0)
        .max(1.0);
    let mut job = egui::text::LayoutJob::simple(
        label.into(),
        egui::TextStyle::Button.resolve(ui.style()),
        ui.visuals().text_color(),
        text_width,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

pub(super) fn name(ui: &mut egui::Ui, label: &str, upk: &str) {
    ui.add(egui::Label::new(egui::RichText::new(label).strong()).truncate())
        .on_hover_text(format!("{label}\n{upk}"));
}

pub(super) fn applied(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(t("items-browser-applied"))
            .small()
            .color(ui.visuals().selection.stroke.color),
    );
}

pub(super) fn hover_outline(ui: &egui::Ui, rect: egui::Rect) {
    let focused = ui
        .memory(|memory| memory.focused())
        .and_then(|id| ui.ctx().read_response(id))
        .is_some_and(|response| rect.contains_rect(response.rect));
    if focused || ui.rect_contains_pointer(rect) {
        ui.painter().rect_stroke(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            egui::Stroke::new(
                if focused { 2.0 } else { 1.0 },
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Inside,
        );
    }
}

pub(super) enum Preview {
    Ready(Arc<[u8]>),
    Loading,
    Missing,
    Failed(String),
}

impl SwapperState {
    pub(super) fn browser_preview(
        &self,
        category: SwapCategory,
        item: &SwapItem,
        resolved: &ResolvedItem,
    ) -> Preview {
        let Some(filename) = resolved.thumbnail.as_deref() else {
            return Preview::Missing;
        };
        let key = thumbnail_key(category, filename, item);
        if let Some(error) = self.failed_thumbnails.get(&key) {
            Preview::Failed(error.clone())
        } else if let Some(Some(bytes)) = self.thumbnails.get(&key) {
            Preview::Ready(bytes.clone())
        } else {
            Preview::Loading
        }
    }
}

pub(super) fn preview(
    ui: &mut egui::Ui,
    id: &str,
    preview: Preview,
    height: f32,
    fill_tile: bool,
) {
    let size = egui::vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| match preview {
            Preview::Ready(bytes) => {
                let image = egui::Image::from_bytes(
                    format!("bytes://swapper-browser/{id}/{:08x}", crc32fast::hash(&bytes)),
                    bytes,
                );
                let image = if fill_tile {
                    crate::ui::browser::cover_image(ui, image, size)
                } else {
                    image.fit_to_exact_size(size)
                };
                ui.add(image);
            }
            Preview::Loading => {
                ui.spinner()
                    .on_hover_text(t("items-browser-preview-loading"));
            }
            Preview::Missing => {
                ui.weak(t("items-browser-preview-missing"));
            }
            Preview::Failed(error) => {
                ui.weak(t("items-browser-preview-failed"))
                    .on_hover_text(error);
            }
        },
    );
}

pub(super) fn tile(
    ui: &mut egui::Ui,
    id: egui::Id,
    category: SwapCategory,
    item: &SwapItem,
    image: Preview,
    selected: bool,
    is_applied: bool,
    height: f32,
) -> egui::Response {
    let label = item_label(category, item);
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
    let style = frame(ui, selected);
    ui.painter()
        .rect_filled(rect, style.corner_radius, style.fill);
    let banner_height = (ui.text_style_height(&egui::TextStyle::Body) + 12.0).max(28.0);
    let banner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 1.0, rect.bottom() - banner_height - 1.0),
        rect.right_bottom() - egui::vec2(1.0, 1.0),
    );
    ui.painter()
        .rect_filled(banner, style.corner_radius, ui.visuals().extreme_bg_color);
    let image_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(6.0, 6.0),
        egui::pos2(rect.right() - 6.0, banner.top() - 4.0),
    );
    let mut image_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt((id, "preview"))
            .max_rect(image_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    image_ui.set_clip_rect(ui.clip_rect().intersect(image_rect));
    preview(
        &mut image_ui,
        &item_identity(item),
        image,
        image_rect.height(),
        true,
    );
    let mut banner_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt((id, "name"))
            .max_rect(banner.shrink2(egui::vec2(6.0, 2.0)))
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::TopDown,
            )),
    );
    name(&mut banner_ui, &label, &item.upk);
    if selected || is_applied {
        let mut badges = ui.new_child(
            egui::UiBuilder::new()
                .id_salt((id, "status"))
                .max_rect(rect.shrink(6.0))
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        for key in [
            selected.then_some("items-browser-selected"),
            is_applied.then_some("items-browser-applied"),
        ]
        .into_iter()
        .flatten()
        {
            egui::Frame::new()
                .fill(ui.visuals().widgets.inactive.bg_fill)
                .inner_margin(egui::Margin::symmetric(4, 2))
                .corner_radius(3)
                .show(&mut badges, |ui| {
                    ui.label(
                        egui::RichText::new(t(key))
                            .small()
                            .color(ui.visuals().selection.stroke.color),
                    );
                });
        }
    }
    ui.painter().rect_stroke(
        rect,
        style.corner_radius,
        style.stroke,
        egui::StrokeKind::Inside,
    );
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{label}\n{}", item.upk));
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &label,
        )
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            egui::Stroke::new(
                if response.has_focus() { 2.0 } else { 1.0 },
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Inside,
        );
    }
    response
}
