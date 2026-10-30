//! Versioned presentation contract shared by full/Lite and the web theme builder.
use super::{ThemeFile, parse_color};
use eframe::egui::{self, Color32};
use serde::Deserialize;
use std::{
    path::{Component, Path},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    #[default]
    Classic,
    Topbar,
    Sidebar,
    Rail,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeLayout {
    pub shell: Shell,
    pub navigation_width: f32,
    pub content_padding: f32,
    pub show_page_heading: bool,
    pub right_align_actions: bool,
    pub joined_actions: bool,
    pub secondary_navigation_width: f32,
    pub navigation_order: Vec<String>,
}
impl Default for ThemeLayout {
    fn default() -> Self {
        Self {
            shell: Shell::Classic,
            navigation_width: 184.0,
            content_padding: 12.0,
            show_page_heading: true,
            right_align_actions: false,
            joined_actions: false,
            secondary_navigation_width: 200.0,
            navigation_order: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeTypography {
    pub heading: f32,
    pub body: f32,
    pub small: f32,
    pub button: f32,
}
impl Default for ThemeTypography {
    fn default() -> Self {
        Self {
            heading: 24.0,
            body: 14.0,
            small: 11.0,
            button: 14.0,
        }
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeComponents {
    pub map_card: MapCardStyle,
    pub navigation: NavigationStyle,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MapCardStyle {
    pub surface: String,
    pub border: String,
    pub selected_border: String,
    pub corner_radius: u8,
    pub padding: f32,
    pub min_width: f32,
    pub image_height: f32,
    pub height: f32,
    pub horizontal: bool,
}
impl Default for MapCardStyle {
    fn default() -> Self {
        Self {
            surface: "#18212B".into(),
            border: "#334155".into(),
            selected_border: "#2E86C1".into(),
            corner_radius: 8,
            padding: 10.0,
            min_width: 240.0,
            image_height: 140.0,
            height: 320.0,
            horizontal: false,
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NavigationStyle {
    pub icons: bool,
    pub surface: String,
    pub selected: String,
    pub text: String,
    pub height: f32,
    pub corner_radius: u8,
}
impl Default for NavigationStyle {
    fn default() -> Self {
        Self {
            icons: false,
            surface: "#16212C".into(),
            selected: "#2E86C1".into(),
            text: "#D6E4F0".into(),
            height: 34.0,
            corner_radius: 6,
        }
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemePages {
    pub workshop: WorkshopLayout,
}
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkshopPresentation {
    #[default]
    Grid,
    LibraryWithDetails,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkshopLayout {
    pub layout: WorkshopPresentation,
    pub details_width: f32,
    /// Preferred number of equal-width columns; zero chooses automatically.
    pub columns: u8,
}
impl Default for WorkshopLayout {
    fn default() -> Self {
        Self {
            layout: WorkshopPresentation::Grid,
            details_width: 300.0,
            columns: 0,
        }
    }
}
impl WorkshopLayout {
    pub fn grid_columns(&self, available: f32, gap: f32, min_width: f32) -> usize {
        let fitting = ((available + gap) / (min_width + gap)).floor().max(1.0) as usize;
        if self.columns == 0 { fitting } else { fitting.min(self.columns as usize) }
    }
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeAssets {
    pub background: Option<String>,
}

#[derive(Clone)]
pub struct Presentation {
    pub layout: ThemeLayout,
    pub components: ThemeComponents,
    pub pages: ThemePages,
    pub background: Option<(String, Arc<[u8]>)>,
}
#[derive(Clone)]
struct LegacyStyles {
    dark: Arc<egui::Style>,
    light: Arc<egui::Style>,
}
fn active_id() -> egui::Id {
    egui::Id::new("hebnix.theme.presentation.v2")
}
fn legacy_id() -> egui::Id {
    egui::Id::new("hebnix.theme.legacy.styles")
}

pub fn presentation(ctx: &egui::Context) -> Option<Arc<Presentation>> {
    ctx.data_mut(|data| data.get_temp::<Arc<Presentation>>(active_id()))
}
/// Render a page or subtab title according to the active theme's layout setting.
/// Legacy themes retain their existing headings.
pub fn page_heading(ui: &mut egui::Ui, title: impl Into<egui::RichText>) {
    if presentation(ui.ctx()).is_none_or(|p| p.layout.show_page_heading) {
        ui.heading(title);
    }
}

/// Validation is also used by the ZIP installer; unsupported versions never silently degrade.
pub fn parse_theme(text: &str) -> Result<ThemeFile, String> {
    let value: toml::Value =
        toml::from_str(text).map_err(|e| format!("Invalid theme TOML: {e}"))?;
    let version = value.get("schema_version");
    if version.is_some_and(|v| !matches!(v.as_integer(), Some(1 | 2))) {
        return Err("Unsupported theme schema; update Hebnix to open this theme".into());
    }
    let mut decoded = value.clone();
    if version.and_then(|v| v.as_integer()) != Some(2) {
        // These were unknown/ignored keys in the original format. Do not begin
        // interpreting them unless the author explicitly opts into version 2.
        if let Some(table) = decoded.as_table_mut() {
            for key in ["layout", "typography", "components", "pages", "assets"] {
                table.remove(key);
            }
        }
    }
    let file: ThemeFile = decoded
        .try_into()
        .map_err(|e| format!("Invalid theme TOML: {e}"))?;
    if file.schema_version != Some(2) {
        return Ok(file);
    }
    if !file.base.is_empty()
        && !file.base.eq_ignore_ascii_case("dark")
        && !file.base.eq_ignore_ascii_case("light")
    {
        return Err("Theme base must be dark or light".into());
    }
    for (label, value, min, max) in [
        (
            "navigation width",
            file.layout.navigation_width,
            64.0,
            320.0,
        ),
        ("content padding", file.layout.content_padding, 0.0, 32.0),
        ("secondary navigation width", file.layout.secondary_navigation_width, 120.0, 320.0),
        ("heading size", file.typography.heading, 12.0, 40.0),
        ("body size", file.typography.body, 10.0, 22.0),
        ("small size", file.typography.small, 9.0, 18.0),
        ("button size", file.typography.button, 10.0, 22.0),
        ("card padding", file.components.map_card.padding, 0.0, 24.0),
        ("card height", file.components.map_card.height, 180.0, 800.0),
        (
            "card minimum width",
            file.components.map_card.min_width,
            180.0,
            480.0,
        ),
        (
            "image height",
            file.components.map_card.image_height,
            60.0,
            240.0,
        ),
        (
            "navigation height",
            file.components.navigation.height,
            24.0,
            52.0,
        ),
        (
            "details width",
            file.pages.workshop.details_width,
            240.0,
            420.0,
        ),
    ] {
        if !value.is_finite() || !(min..=max).contains(&value) {
            return Err(format!("Invalid {label}: expected {min}–{max}"));
        }
    }
    if file.pages.workshop.columns > 8 {
        return Err("Invalid grid columns: expected 0 (automatic) or 1–8".into());
    }
    for color in [
        &file.components.map_card.surface,
        &file.components.map_card.border,
        &file.components.map_card.selected_border,
        &file.components.navigation.surface,
        &file.components.navigation.selected,
        &file.components.navigation.text,
    ] {
        if !color.is_ascii() || parse_color(color).is_none() {
            return Err(format!("Invalid component color '{color}'"));
        }
    }
    // Validate legacy fields for v2 as well; legacy loading stays permissive.
    if let Some(colors) = value.get("colors").and_then(|v| v.as_table()) {
        for (name, color) in colors {
            if !color
                .as_str()
                .is_some_and(|s| s.is_ascii() && parse_color(s).is_some())
            {
                return Err(format!("Invalid color '{name}'"));
            }
        }
    }
    if let Some(asset) = &file.assets.background {
        validate_asset_path(asset)?;
    }
    Ok(file)
}
pub fn validate_asset_path(path: &str) -> Result<(), String> {
    let relative = Path::new(path);
    if !path.starts_with("assets/")
        || path.contains('\\')
        || path.contains(':')
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        || !matches!(
            relative.extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg" | "webp")
        )
    {
        return Err("Background must be an image inside assets/ using a relative path".into());
    }
    Ok(())
}

pub(super) fn prepare_presentation(
    ctx: &egui::Context,
    file: Option<&ThemeFile>,
    dir: &Path,
) -> Result<(), String> {
    let next = if let Some(file) = file.filter(|f| f.schema_version == Some(2)) {
        let background = if let Some(path) = &file.assets.background {
            let root = dir.canonicalize().map_err(|e| e.to_string())?;
            let full = dir
                .join(path)
                .canonicalize()
                .map_err(|e| format!("Background '{path}': {e}"))?;
            if !full.starts_with(&root) {
                return Err("Background escapes theme directory".into());
            }
            let bytes = std::fs::read(full).map_err(|e| e.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("Background exceeds 8 MB".into());
            }
            let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|e| e.to_string())?;
            let (w, h) = reader
                .into_dimensions()
                .map_err(|e| format!("Invalid background image: {e}"))?;
            if w == 0 || h == 0 || w > 4096 || h > 4096 {
                return Err("Background dimensions must be at most 4096 × 4096".into());
            }
            image::load_from_memory(&bytes)
                .map_err(|e| format!("Invalid background image: {e}"))?;
            // Content-specific URI prevents stale textures after editing the same asset.
            Some((
                format!("bytes://theme/{path}/{:08x}", crc32fast::hash(&bytes)),
                Arc::from(bytes),
            ))
        } else {
            None
        };
        Some(Arc::new(Presentation {
            layout: file.layout.clone(),
            components: file.components.clone(),
            pages: file.pages.clone(),
            background,
        }))
    } else {
        None
    };
    // Restore both original styles before leaving/updating a v2 theme. Pure legacy -> legacy
    // follows the original code path unchanged, including application-specific style settings.
    if let Some(saved) = ctx.data_mut(|d| d.get_temp::<LegacyStyles>(legacy_id())) {
        ctx.set_style_of(egui::Theme::Dark, saved.dark);
        ctx.set_style_of(egui::Theme::Light, saved.light);
        ctx.data_mut(|d| d.remove::<LegacyStyles>(legacy_id()));
    }
    ctx.data_mut(|d| d.remove::<Arc<Presentation>>(active_id()));
    if let Some(next) = next {
        let saved = LegacyStyles {
            dark: ctx.style_of(egui::Theme::Dark),
            light: ctx.style_of(egui::Theme::Light),
        };
        ctx.data_mut(|d| {
            d.insert_temp(legacy_id(), saved);
            d.insert_temp(active_id(), next);
        });
        let base = if file.unwrap().base.eq_ignore_ascii_case("light") {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        };
        ctx.set_style_of(base, egui::Style::default());
    }
    Ok(())
}
pub(super) fn apply_presentation_style(ctx: &egui::Context, file: &ThemeFile) {
    if file.schema_version != Some(2) {
        return;
    }
    ctx.style_mut_of(ctx.theme(), |style| {
        for (role, size) in [
            (egui::TextStyle::Heading, file.typography.heading),
            (egui::TextStyle::Body, file.typography.body),
            (egui::TextStyle::Small, file.typography.small),
            (egui::TextStyle::Button, file.typography.button),
        ] {
            style
                .text_styles
                .insert(role, egui::FontId::proportional(size));
        }
    });
}
impl Presentation {
    /// Toolbar actions use their natural width; the toolbar itself scrolls if needed.
    pub fn inline_actions(&self, ui: &mut egui::Ui, labels: &[String]) -> Option<usize> {
        let primary = labels.first()?;
        let mut selected = None;
        ui.scope(|ui| {
            let joined = self.layout.joined_actions && labels.len() > 1;
            ui.spacing_mut().item_spacing.x = if joined { 0.0 } else { 4.0 };
            let radius = ui.visuals().widgets.inactive.corner_radius;
            let mut button = egui::Button::new(primary)
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .min_size(egui::vec2(0.0, self.components.navigation.height));
            if joined {
                button = button.corner_radius(egui::CornerRadius { ne: 0, se: 0, ..radius })
                    .stroke(egui::Stroke::NONE);
            }
            let main = ui.add(button);
            if main.clicked() {
                selected = Some(0);
            }
            if labels.len() > 1 {
                let arrow = dropdown_button_styled(ui, egui::vec2(24.0, main.rect.height()), joined);
                if joined {
                    // One outline around two hit targets: primary action and menu.
                    let response = main.clone().union(arrow.clone());
                    let stroke = ui.style().interact(&response).bg_stroke;
                    ui.painter().rect_stroke(response.rect, radius, stroke, egui::StrokeKind::Inside);
                    ui.painter().line_segment([
                        egui::pos2(main.rect.right(), main.rect.top() + 1.0),
                        egui::pos2(main.rect.right(), main.rect.bottom() - 1.0),
                    ], stroke);
                }
                action_menu(ui, &arrow, labels, &mut selected);
            }
        });
        selected
    }
    /// Measure the trailing group once per frame, then use the previous frame's
    /// width for the spacer. Actions are rendered and handled exactly once.
    pub fn toolbar_actions(&self, ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
        if !self.layout.right_align_actions {
            return ui.scope(render).response.rect;
        }
        let id = ui.make_persistent_id("theme_toolbar_actions_width");
        let width = ui.ctx().data_mut(|data| data.get_temp::<f32>(id)).unwrap_or(0.0);
        if width > 0.0 {
            ui.add_space((ui.available_width() - width).max(0.0));
        }
        let rect = ui.scope(render).response.rect;
        if (rect.width() - width).abs() > 0.1 {
            ui.ctx().data_mut(|data| data.insert_temp(id, rect.width()));
            ui.ctx().request_repaint();
        }
        rect
    }
    /// Sidebar actions have their own full row. Never inherit the toolbar's
    /// nested horizontal layout or split the primary label across sibling rows.
    pub fn sidebar_actions(&self, ui: &mut egui::Ui, labels: &[String]) -> Option<usize> {
        let primary = labels.first()?;
        let width = ui.available_width();
        let height = self.components.navigation.height;
        let mut selected = None;
        if labels.len() == 1 || width < 140.0 {
            if ui
                .add_sized([width, height], egui::Button::new(primary).wrap())
                .on_hover_text(primary)
                .clicked()
            {
                selected = Some(0);
            }
            if labels.len() > 1 {
                let arrow = dropdown_button(ui, egui::vec2(width, 24.0));
                action_menu(ui, &arrow, labels, &mut selected);
            }
            return selected;
        }
        let gap = 4.0;
        let menu_width = 24.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let main = ui
                .scope(|ui| {
                    // Compact padding lets normal action labels stay on one line in
                    // the sidebar. Larger fonts can still wrap rather than clipping.
                    ui.spacing_mut().button_padding.x = ui.spacing().button_padding.x.min(2.0);
                    ui.add_sized(
                        [width - menu_width - gap, height],
                        egui::Button::new(primary).wrap(),
                    )
                })
                .inner
                .on_hover_text(primary);
            if main.clicked() {
                selected = Some(0);
            }
            let arrow = dropdown_button(ui, egui::vec2(menu_width, main.rect.height()));
            action_menu(ui, &arrow, labels, &mut selected);
        });
        selected
    }
    pub fn nav_item(
        &self,
        ui: &mut egui::Ui,
        selected: bool,
        key: &str,
        label: &str,
        width: Option<f32>,
    ) -> egui::Response {
        let n = &self.components.navigation;
        let fill = surface_color(ui, if selected { &n.selected } else { &n.surface });
        // Let egui measure the complete button, including theme padding and
        // strokes. A guessed text width used as a maximum clips the labels.
        let text = egui::RichText::new(label).color(color(&n.text));
        let button = if n.icons {
            egui::Button::image_and_text(navigation_icon(key).fit_to_exact_size(egui::vec2(16.0,16.0))
                .tint(color(&n.text)), text)
        } else { egui::Button::new(text) };
        ui.add(
            button
                .fill(fill)
                .corner_radius(n.corner_radius)
                .selected(selected)
                .min_size(egui::vec2(width.unwrap_or(0.0), n.height))
                .wrap_mode(if width.is_some() {
                    egui::TextWrapMode::Wrap
                } else {
                    egui::TextWrapMode::Extend
                }),
        )
    }
    pub fn brand(&self, ui: &mut egui::Ui) {
        // Keep the header text-only. Theme icons belong to navigation controls.
        ui.heading("Hebnix");
    }
    pub fn paint_background(&self, ui: &mut egui::Ui) {
        if let Some((uri, bytes)) = &self.background {
            egui::Image::from_bytes(uri.clone(), bytes.clone())
                .tint(Color32::from_white_alpha(65))
                .paint_at(ui, ui.max_rect());
        }
    }
    pub fn ordered_pages<'a>(
        &self,
        available: &'a [(&'a str, &'a str)],
    ) -> Vec<(&'a str, &'a str)> {
        let mut result = Vec::new();
        for name in &self.layout.navigation_order {
            if let Some(&page) = available.iter().find(|(id, _)| *id == name) {
                if !result.contains(&page) {
                    result.push(page);
                }
            }
        }
        for &page in available {
            if !result.contains(&page) {
                result.push(page);
            }
        }
        result
    }
}
pub fn color(value: &str) -> Color32 {
    parse_color(value).unwrap_or(Color32::TRANSPARENT)
}

// Images, rather than Unicode glyphs, work with every selected theme font.
fn navigation_icon(key: &str) -> egui::Image<'static> {
    let source = match key {
        "Console" => egui::include_image!("../../assets/navigation/console.svg"),
        "Workshop" | "Maps" => egui::include_image!("../../assets/navigation/workshop.svg"),
        "Spoofer" => egui::include_image!("../../assets/navigation/spoofer.svg"),
        "Colours" => egui::include_image!("../../assets/navigation/colours.svg"),
        "Items" => egui::include_image!("../../assets/navigation/items.svg"),
        "Plugins" => egui::include_image!("../../assets/navigation/plugins.svg"),
        "Replays" => egui::include_image!("../../assets/navigation/replays.svg"),
        "Settings" => egui::include_image!("../../assets/navigation/settings.svg"),
        _ => egui::include_image!("../../assets/navigation/about.svg"),
    };
    egui::Image::new(source)
}

fn dropdown_button(ui: &mut egui::Ui, size: egui::Vec2) -> egui::Response {
    dropdown_button_styled(ui, size, false)
}

fn dropdown_button_styled(ui: &mut egui::Ui, size: egui::Vec2, joined: bool) -> egui::Response {
    let arrow = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding.x = 4.0;
            let mut button = egui::Button::new("");
            if joined {
                let radius = ui.visuals().widgets.inactive.corner_radius;
                button = button.corner_radius(egui::CornerRadius { nw: 0, sw: 0, ..radius })
                    .stroke(egui::Stroke::NONE);
            }
            ui.add_sized(size, button)
        })
        .inner
        .on_hover_text("More actions");
    arrow.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "More actions")
    });
    // Draw the chevron as geometry so every theme font can display it.
    let center = arrow.rect.center();
    let stroke = ui.style().interact(&arrow).fg_stroke;
    ui.painter().line_segment(
        [
            center + egui::vec2(-3.5, -1.5),
            center + egui::vec2(0.0, 2.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(0.0, 2.0),
            center + egui::vec2(3.5, -1.5),
        ],
        stroke,
    );
    arrow
}

fn action_menu(
    ui: &mut egui::Ui,
    arrow: &egui::Response,
    labels: &[String],
    selected: &mut Option<usize>,
) {
    egui::Popup::menu(arrow)
        .open_memory(arrow.clicked().then_some(egui::SetOpenCommand::Toggle))
        .width(ui.available_width().clamp(180.0, 360.0))
        .layout(egui::Layout::top_down(egui::Align::Min))
        .show(|ui| {
            for (index, label) in labels.iter().enumerate().skip(1) {
                if ui.add(egui::Button::new(label).wrap()).clicked() {
                    *selected = Some(index);
                    ui.close();
                }
            }
        });
}

/// Keep navigation and all toolbar controls in the same unwrapped row.
pub fn toolbar(ui: &mut egui::Ui, render: impl FnOnce(&mut egui::Ui)) -> egui::Rect {
    egui::ScrollArea::horizontal()
        .id_salt("theme_nav_scroll")
        .show(ui, |ui| ui.horizontal(render).response.rect)
        .inner
}

#[cfg(test)]
mod tests {
    #[test]
    fn toolbar_options_are_explicit_and_legacy_themes_ignore_them() {
        let defaults = super::parse_theme("schema_version = 2").unwrap();
        assert!(!defaults.layout.right_align_actions);
        assert!(!defaults.layout.joined_actions);
        let options = "[layout]\nright_align_actions = true\njoined_actions = true";
        let enabled = super::parse_theme(&format!("schema_version = 2\n{options}")).unwrap();
        assert!(enabled.layout.right_align_actions && enabled.layout.joined_actions);
        let legacy = super::parse_theme(options).unwrap();
        assert!(!legacy.layout.right_align_actions && !legacy.layout.joined_actions);
        for key in ["right_align_actions", "joined_actions"] {
            assert!(super::parse_theme(&format!("schema_version = 2\n[layout]\n{key} = \"yes\"")).is_err());
        }
    }
    #[test]
    fn tile_columns_share_width_and_reduce_on_narrow_windows() {
        let mut layout=WorkshopLayout::default();
        for (preferred,width,expected) in [(4,1100.0,4),(3,1100.0,3),(4,760.0,2),(4,200.0,1),(0,1500.0,5)] {
            layout.columns=preferred;
            assert_eq!(layout.grid_columns(width,10.0,250.0),expected);
        }
        for text in ["schema_version=2\n[pages.workshop]\ncolumns=9", "schema_version=2\n[pages.workshop]\ncolumns=3.5", "schema_version=2\n[components.map_card]\nheight=0"] {
            assert!(parse_theme(text).is_err());
        }
        let file=parse_theme("schema_version=2\n[pages.workshop]\ncolumns=3\n[components.map_card]\nheight=340").unwrap();
        assert_eq!(file.pages.workshop.columns,3);
        assert_eq!(file.components.map_card.height,340.0);
    }
    #[test]
    fn navigation_icons_are_an_optional_boolean_in_v2() {
        for icons in [true,false] {
            let theme=super::parse_theme(&format!("schema_version = 2\n[components.navigation]\nicons = {icons}")).unwrap();
            assert_eq!(theme.components.navigation.icons,icons);
        }
        assert!(!super::parse_theme("schema_version = 2").unwrap().components.navigation.icons);
        // The same table remains ignored for unversioned legacy themes.
        assert!(!super::parse_theme("[components.navigation]\nicons = true").unwrap().components.navigation.icons);
    }
    use super::*;
    #[test]
    #[ignore = "v2 example themes were never committed"]
    fn navigation_labels_are_never_elided_by_theme_padding() {
        fn check(shape: &egui::Shape, label: &str, found: &mut bool) {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    *found = true;
                    assert!(
                        !text.galley.elided,
                        "Navigation label was truncated: {label}"
                    );
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        check(shape, label, found);
                    }
                }
                _ => {}
            }
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        for name in [
            "Carbon Studio",
            "Ocean Workspace",
            "Crimson Arena",
            "Ocean Studio Dark",
        ] {
            let ctx = egui::Context::default();
            super::super::apply_theme(&ctx, &dir, &dir, name).unwrap();
            let p = presentation(&ctx).unwrap();
            // Include padding larger than the old hard-coded 20-pixel allowance,
            // a larger theme font, and a narrow sidebar with a translated label.
            ctx.style_mut_of(ctx.theme(), |style| {
                style.spacing.button_padding.x = 24.0;
                style
                    .text_styles
                    .insert(egui::TextStyle::Button, egui::FontId::proportional(22.0));
            });
            for label in [
                "Console",
                "Maps",
                "Spoofer",
                "Colours",
                "Items",
                "Settings",
                "Plugins",
                "Replays",
                "About",
                "Benachrichtigungseinstellungen",
            ] {
                for width in [None, Some(80.0)] {
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1250.0, 700.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    p.nav_item(ui, false, label, label, width);
                                });
                            });
                        },
                    );
                    let mut found = false;
                    for shape in &output.shapes {
                        check(&shape.shape, label, &mut found);
                    }
                    assert!(found, "No navigation text rendered for {name}: {label}");
                }
            }
        }
    }
    #[test]
    fn legacy_examples_have_no_presentation() {
        for text in [
            include_str!("../../../../examples/themes/Carbon.toml"),
            include_str!("../../../../examples/themes/Ocean.toml"),
            include_str!("../../../../examples/themes/Crimson.toml"),
        ] {
            let file = parse_theme(text).unwrap();
            assert_eq!(file.schema_version, None);
        }
        let legacy = parse_theme("base = 'dark'\n[layout]\nshell = 'previously ignored custom data'\n[assets]\nbackground = '../../ignored.png'").unwrap();
        assert_eq!(legacy.schema_version, None);
        assert!(legacy.assets.background.is_none());
    }
    #[test]
    #[ignore = "v2 example themes were never committed"]
    fn v2_examples_validate_and_switch_without_leaking() {
        let ctx = egui::Context::default();
        let dir = Path::new(".");
        let before = ctx.style_of(egui::Theme::Dark);
        for text in [
            &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/Carbon Studio.toml")).unwrap(),
            &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/Ocean Workspace.toml")).unwrap(),
            &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/Crimson Arena.toml")).unwrap(),
            &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/Ocean Studio Dark.toml")).unwrap(),
        ] {
            let file = parse_theme(text).unwrap();
            prepare_presentation(&ctx, Some(&file), dir).unwrap();
            assert!(presentation(&ctx).is_some());
            apply_presentation_style(&ctx, &file);
        }
        prepare_presentation(&ctx, None, dir).unwrap();
        assert!(presentation(&ctx).is_none());
        assert_eq!(ctx.style_of(egui::Theme::Dark).spacing, before.spacing);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).text_styles,
            before.text_styles
        );
    }
    #[test]
    fn rejects_unsupported_schema_and_unsafe_assets() {
        assert!(parse_theme("schema_version = 99").is_err());
        for path in [
            "../image.png",
            "assets/../../image.png",
            "C:/image.png",
            "assets/x.svg",
        ] {
            assert!(validate_asset_path(path).is_err());
        }
        assert!(parse_theme("schema_version = 2\n[layout]\nnavigation_width = nan").is_err());
        assert!(
            parse_theme("schema_version = 2\n[components.map_card]\nsurface = '🔥🔥'").is_err()
        );
    }
    #[test]
    fn navigation_order_cannot_remove_or_duplicate_pages() {
        let mut p = Presentation {
            layout: ThemeLayout::default(),
            components: ThemeComponents::default(),
            pages: ThemePages::default(),
            background: None,
        };
        p.layout.navigation_order = vec!["Settings".into(), "Settings".into(), "Unknown".into()];
        assert_eq!(
            p.ordered_pages(&[("Console", "Console"), ("Settings", "Settings")]),
            vec![("Settings", "Settings"), ("Console", "Console")]
        );
    }
    #[test]
    #[ignore = "v2 example themes were never committed"]
    fn switching_through_v2_restores_each_legacy_theme_style() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        for name in ["Dark", "Light", "Carbon", "Ocean", "Crimson"] {
            let ctx = egui::Context::default();
            super::super::apply_theme(&ctx, &dir, &dir, name).unwrap();
            let expected = format!("{:?}", ctx.style_of(ctx.theme()));
            for advanced in ["Carbon Studio", "Ocean Workspace", "Crimson Arena", "Hebnix 2"] {
                super::super::apply_theme(&ctx, &dir, &dir, advanced).unwrap();
                assert!(presentation(&ctx).is_some());
                super::super::apply_theme(&ctx, &dir, &dir, name).unwrap();
                assert!(presentation(&ctx).is_none());
                assert_eq!(
                    format!("{:?}", ctx.style_of(ctx.theme())),
                    expected,
                    "{advanced} -> {name}"
                );
            }
        }
    }
}

pub enum ShellPart {
    Navigation(bool),
    Content,
}
pub enum SidebarPart {
    Pages,
    Footer,
}

/// Compact secondary navigation is opt-in; legacy and existing themes keep
/// their original panel sizing, including egui's saved panel state.
pub fn secondary_navigation(ui: &egui::Ui, id: &'static str) -> egui::Panel {
    let panel=egui::Panel::left(id).resizable(false);
    if let Some(p)=presentation(ui.ctx()) {
        if (p.layout.secondary_navigation_width-200.0).abs()>0.01 {
            return panel.exact_size(p.layout.secondary_navigation_width);
        }
    }
    panel.default_size(200.0).size_range(200.0..=320.0)
}

/// Reserve the footer's actual measured height before laying out navigation.
/// This also works with wrapped actions/statuses and larger theme fonts.
pub fn sidebar(ui: &mut egui::Ui, mut render: impl FnMut(&mut egui::Ui, SidebarPart)) {
    egui::Panel::bottom("hebnix_theme_sidebar_footer")
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin {
            top: 8,
            ..Default::default()
        }))
        .show(ui, |ui| {
            ui.separator();
            render(ui, SidebarPart::Footer);
        });
    render(ui, SidebarPart::Pages);
}
/// App state stays with the owner; this function only arranges the two regions.
pub fn show_shell(
    ui: &mut egui::Ui,
    p: &Presentation,
    mut render: impl FnMut(&mut egui::Ui, ShellPart),
) {
    let padding = p.layout.content_padding.round() as i8;
    if matches!(p.layout.shell, Shell::Sidebar | Shell::Rail) {
        let width = if p.layout.shell == Shell::Rail {
            96.0
        } else {
            p.layout.navigation_width
        };
        egui::Panel::left("hebnix_themed_navigation")
            .resizable(false)
            .exact_size(width)
            .frame(
                egui::Frame::new()
                    .fill(surface_color(ui, &p.components.navigation.surface))
                    .inner_margin(8),
            )
            .show(ui, |ui| render(ui, ShellPart::Navigation(true)));
    } else {
        egui::Panel::top("hebnix_themed_navigation")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(surface_color(ui, &p.components.navigation.surface))
                    .inner_margin(8),
            )
            .show(ui, |ui| render(ui, ShellPart::Navigation(false)));
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(ui.visuals().panel_fill)
                .inner_margin(padding),
        )
        .show(ui, |ui| {
            p.paint_background(ui);
            render(ui, ShellPart::Content);
        });
}

/// Custom surfaces follow the existing window-opacity preference, without scaling twice.
pub fn surface_color(ui: &egui::Ui, value: &str) -> Color32 {
    let color = color(value);
    let opacity = ui
        .ctx()
        .data_mut(|d| d.get_temp::<f32>(egui::Id::new("hebnix.theme.opacity")))
        .unwrap_or(1.0);
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (color.a() as f32 * opacity) as u8,
    )
}
