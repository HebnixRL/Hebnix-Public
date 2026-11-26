//! Exercise the production view with a small host, without starting app services
//! or permitting file writes. Ownership, title settings and patch validation are real.
use super::{TitleCatalogEntry, parse_hex_color};
use crate::spoofer;
use crate::spoofer::rules::TitleSpoofSettings;
use serde_json::Value;
use eframe::egui::{self, Color32};
use std::{path::PathBuf, sync::Arc};

#[path = "titles_snapshot.rs"]
mod snapshot;
#[path = "titles.rs"]
mod view;

struct Settings {
    rl_path: String,
    owned_items_only: bool,
}
struct Config {
    settings: Settings,
}
struct Workshop;
impl Workshop {
    fn suspend_multiplayer(&self) {}
}
struct HebnixApp {
    spoofer_title: String,
    spoofer_title_color: [u8; 3],
    spoofer_title_glow: bool,
    spoofer_title_glow_color: [u8; 3],
    spoofer_title_target: Option<String>,
    spoofer_title_copy: Option<String>,
    spoofer_title_copy_filter: String,
    spoofer_title_filter: String,
    spoofer_title_enabled: bool,
    spoofer_titles: Vec<TitleSpoofSettings>,
    title_editor: view::TitleEditorState,
    title_catalog: Vec<TitleCatalogEntry>,
    title_only_owned: bool,
    title_patcher: crate::patcher::title_patcher::TitlePatcherState,
    owned_inventory: crate::owned_inventory::OwnedInventory,
    last_rl_open: bool,
    first_status: bool,
    catalogs_loading: bool,
    catalogs_error: Option<String>,
    config: Config,
    base_dir: PathBuf,
    spoofer_mgr: Arc<spoofer::SpooferManager>,
    workshop: Workshop,
    saves: usize,
}
impl HebnixApp {
    fn swapper_cooked_path(&self) -> PathBuf {
        PathBuf::from(&self.config.settings.rl_path).join("TAGame/CookedPCConsole")
    }
    fn save_config(&mut self) {
        self.saves += 1;
    }
    fn reload_catalogs(&mut self, _: &egui::Context) {}
}

fn fixture() -> HebnixApp {
    let (tx, _) = crossbeam_channel::unbounded();
    let mut owned = crate::owned_inventory::OwnedInventory::default();
    owned.title_ids.extend(["xp_0".into(), "xp_1".into()]);
    HebnixApp {
        spoofer_title: "New {literal} title".into(),
        spoofer_title_color: [36, 221, 148],
        spoofer_title_glow: true,
        spoofer_title_glow_color: [232, 68, 173],
        spoofer_title_target: Some("XP_0".into()),
        spoofer_title_copy: None,
        spoofer_title_copy_filter: String::new(),
        spoofer_title_filter: String::new(),
        spoofer_title_enabled: true,
        spoofer_titles: Vec::new(),
        title_editor: Default::default(),
        title_catalog: (0..900)
            .map(|i| TitleCatalogEntry {
                id: format!("XP_{i}"),
                text: format!("Library title {i}"),
                color: (i == 0).then(|| "AABBCC".into()),
                glow: (i == 0).then(|| "FF459F".into()),
            })
            .collect(),
        title_only_owned: true,
        title_patcher: Default::default(),
        owned_inventory: owned,
        last_rl_open: false,
        first_status: true,
        catalogs_loading: false,
        catalogs_error: None,
        config: Config {
            settings: Settings {
                rl_path: String::new(),
                owned_items_only: true,
            },
        },
        base_dir: PathBuf::new(),
        spoofer_mgr: Arc::new(spoofer::SpooferManager::new(PathBuf::new(), tx)),
        workshop: Workshop,
        saves: 0,
    }
}

fn frame(ctx: &egui::Context, host: &mut HebnixApp, events: Vec<egui::Event>) -> egui::FullOutput {
    frame_at(ctx, host, events, egui::vec2(1250.0, 730.0)).0
}

fn frame_at(
    ctx: &egui::Context,
    host: &mut HebnixApp,
    events: Vec<egui::Event>,
    size: egui::Vec2,
) -> (egui::FullOutput, egui::Rect, egui::Rect) {
    let mut bounds = egui::Rect::NOTHING;
    let mut content = egui::Rect::NOTHING;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            egui::Panel::left("main_fixture_nav")
                .exact_size(184.0)
                .show(ui, |_| {});
            egui::Panel::left("secondary_fixture_nav")
                .exact_size(175.0)
                .show(ui, |_| {});
            egui::CentralPanel::default().show(ui, |ui| {
                bounds = ui.available_rect_before_wrap();
                host.render_titles_page(ui);
                content = ui.min_rect();
            });
        },
    );
    (output, bounds, content)
}

fn texts(output: &egui::FullOutput) -> Vec<(&egui::epaint::TextShape, egui::Rect)> {
    fn collect<'a>(
        shape: &'a egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<(&'a egui::epaint::TextShape, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(text) => out.push((text, clip)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, clip, out);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, shape.clip_rect, &mut result);
    }
    result
}

fn click(ctx: &egui::Context, host: &mut HebnixApp, label: &str, occurrence: usize) {
    let output = frame(ctx, host, vec![]);
    let (text, _) = texts(&output)
        .into_iter()
        .filter(|(t, _)| t.galley.text() == label)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("Missing button {label} #{occurrence}"));
    let pos = text.pos + text.galley.size() * 0.5;
    for pressed in [true, false] {
        frame(
            ctx,
            host,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn setup() -> (egui::Context, HebnixApp) {
    let ctx = egui::Context::default();
    let mut host = fixture();
    for _ in 0..4 {
        frame(&ctx, &mut host, vec![]);
    }
    (ctx, host)
}

#[test]
fn title_ui_copy_uses_known_metadata_and_keeps_unknown_attributes() {
    let (ctx, mut host) = setup();
    click(&ctx, &mut host, "Copy", 0);
    assert_eq!(host.spoofer_title, "Library title 0");
    assert_eq!(host.spoofer_title_color, [0xaa, 0xbb, 0xcc]);
    assert_eq!(host.spoofer_title_glow_color, [0xff, 0x45, 0x9f]);
    click(&ctx, &mut host, "Copy", 1);
    assert_eq!(host.spoofer_title, "Library title 1");
    assert_eq!(host.spoofer_title_color, [0xaa, 0xbb, 0xcc]);
    assert_eq!(host.spoofer_title_glow_color, [0xff, 0x45, 0x9f]);
}

#[test]
fn title_ui_target_selects_edit_or_add_and_preserves_drafts() {
    let (ctx, mut host) = setup();
    click(&ctx, &mut host, "Add Title", 0);
    assert_eq!(host.spoofer_titles.len(), 1);
    host.spoofer_title = "Unsaved entry {literal}".into();
    host.select_title_target(Some("XP_1".into()));
    host.spoofer_title = "New unsaved draft".into();
    host.select_title_target(Some("XP_0".into()));
    assert_eq!(host.spoofer_title, "Unsaved entry {literal}");
    click(&ctx, &mut host, "Edit Title", 1);
    assert_eq!(host.spoofer_titles.len(), 1);
    assert_eq!(host.spoofer_titles[0].text, "Unsaved entry {literal}");
    assert_eq!(host.spoofer_titles[0].glow_color.as_deref(), Some("E844AD"));
    host.select_title_target(Some("XP_1".into()));
    assert_eq!(host.spoofer_title, "New unsaved draft");
    click(&ctx, &mut host, "Add Title", 0);
    assert_eq!(host.spoofer_titles.len(), 2);
    // Case variants still update the existing rule rather than create duplicates.
    host.spoofer_title_target = Some("xp_0".into());
    host.spoofer_title = "Updated existing".into();
    click(&ctx, &mut host, "Edit Title", 1);
    assert_eq!(host.spoofer_titles.len(), 2);
    assert_eq!(host.spoofer_titles[0].text, "Updated existing");
    click(&ctx, &mut host, "Remove", 0);
    assert_eq!(host.spoofer_titles.len(), 1);
    assert_eq!(host.spoofer_titles[0].text, "New unsaved draft");
    assert_eq!(host.spoofer_title, "Updated existing");
}

#[test]
fn title_ui_glow_toggle_preserves_colour_and_only_adds_glyph_shapes() {
    let (ctx, mut host) = setup();
    let output = frame(&ctx, &mut host, vec![]);
    let before = texts(&output)
        .iter()
        .filter(|(t, _)| t.galley.text() == host.spoofer_title)
        .count();
    assert_eq!(before, 26, "24 halo passes, main glyphs and text input");
    click(&ctx, &mut host, "Glow", 0);
    assert!(!host.spoofer_title_glow);
    assert_eq!(host.spoofer_title_glow_color, [232, 68, 173]);
    let output = frame(&ctx, &mut host, vec![]);
    assert_eq!(
        texts(&output)
            .iter()
            .filter(|(t, _)| t.galley.text() == host.spoofer_title)
            .count(),
        2
    );
    click(&ctx, &mut host, "Glow", 0);
    assert!(host.spoofer_title_glow);
}

#[test]
fn title_ui_three_columns_and_patch_actions_fit_at_native_and_high_dpi() {
    for dpi in [1.0, 1.5, 2.0] {
        let (ctx, mut host) = setup();
        ctx.set_pixels_per_point(dpi);
        for _ in 0..4 {
            frame(&ctx, &mut host, vec![]);
        }
        let output = frame(&ctx, &mut host, vec![]);
        let mut headings = Vec::new();
        for label in [
            "Copy Title",
            "Edit Title",
            "Applied Titles",
            "Apply",
            "Restore",
        ] {
            let (text, clip) = texts(&output)
                .into_iter()
                .find(|(t, _)| t.galley.text() == label)
                .expect(label);
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                clip.expand(1.0).contains_rect(rect),
                "{label} clipped at DPI {dpi}: {rect:?} / {clip:?}"
            );
            assert!(!text.galley.elided, "{label} truncated");
            if label.ends_with("Title") || label == "Applied Titles" {
                headings.push(rect);
            }
        }
        assert!(
            headings[0].right() < headings[1].left() && headings[1].right() < headings[2].left()
        );
        assert!((headings[0].top() - headings[2].top()).abs() < 1.0);
        let library_rows = texts(&output)
            .iter()
            .filter(|(t, _)| t.galley.text().starts_with("Library title"))
            .count();
        assert!(
            library_rows < 50,
            "The entire library must not render while scrolling"
        );
    }
}

#[test]
fn title_ui_columns_fit_viewport_width_without_page_scrolling() {
    for width in [1100.0, 1250.0, 1637.0] {
        for font_size in [14.0, 22.0] {
            let ctx = egui::Context::default();
            ctx.style_mut_of(ctx.theme(), |style| {
                style
                    .text_styles
                    .insert(egui::TextStyle::Body, egui::FontId::proportional(font_size));
            });
            let mut host = fixture();
            // A long saved target name must truncate inside its selector.
            host.title_catalog[0].text =
                "A very long original title name that must fit inside the replacement selector"
                    .into();
            for _ in 0..4 {
                frame_at(&ctx, &mut host, vec![], egui::vec2(width, 730.0));
            }
            let (output, bounds, content) =
                frame_at(&ctx, &mut host, vec![], egui::vec2(width, 730.0));
            assert!(
                content.right() <= bounds.right() + 1.0,
                "Columns overflow at {width}/{font_size}: {content:?} / {bounds:?}"
            );
            assert!(
                content.bottom() <= bounds.bottom() + 1.0,
                "Page grows beyond its viewport at {width}/{font_size}: {content:?} / {bounds:?}"
            );
            for label in [
                "Copy Title",
                "Edit Title",
                "Applied Titles",
                "Add Title",
                "Apply",
                "Restore",
            ] {
                let (text, clip) = texts(&output)
                    .into_iter()
                    .find(|(t, _)| t.galley.text() == label)
                    .expect(label);
                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    clip.expand(1.0).contains_rect(rect),
                    "{label} clipped at {width}/{font_size}"
                );
            }
            assert!(
                !texts(&output)
                    .iter()
                    .any(|(t, _)| t.galley.text().contains("Local UPK changes affect"))
            );
        }
    }
}

#[test]
fn title_ui_legacy_style_defaults_and_editor_state_round_trip() {
    let old: TitleSpoofSettings =
        serde_json::from_str(r#"{"text":"Old","color":"AABBCC","glow":true,"target_id":"XP_0"}"#)
            .unwrap();
    assert!(old.glow_color.is_none());
    let (ctx, mut host) = setup();
    host.spoofer_titles.push(old);
    click(&ctx, &mut host, "Edit", 0);
    assert_eq!(host.spoofer_title_glow_color, [0xaa, 0xbb, 0xcc]);
    host.spoofer_title = "Keep my changes".into();
    host.select_title_target(Some("XP_1".into()));
    let state = serde_json::to_value(&host.title_editor).unwrap();
    host.title_editor = serde_json::from_value(state).unwrap();
    click(&ctx, &mut host, "Edit", 0);
    assert_eq!(host.spoofer_title, "Keep my changes");
}

#[test]
fn title_ui_rows_are_compact_and_patch_actions_share_a_line() {
    let (ctx, mut host) = setup();
    host.spoofer_titles = (0..3)
        .map(|i| TitleSpoofSettings {
            text: format!("Replacement {i}"),
            color: "E8E8E8".into(),
            glow: false,
            glow_color: None,
            target_id: Some(format!("XP_{i}")),
        })
        .collect();
    host.owned_inventory.title_ids.insert("xp_2".into());
    for _ in 0..3 {
        frame(&ctx, &mut host, vec![]);
    }
    let output = frame(&ctx, &mut host, vec![]);
    let position = |label: &str| {
        texts(&output)
            .into_iter()
            .find(|(t, _)| t.galley.text() == label)
            .unwrap()
            .0
            .pos
    };
    let a = position("Replacement 0");
    let b = position("Replacement 1");
    assert!((b.y - a.y) < 90.0, "Applied rows expanded: {a:?} {b:?}");
    let a = position("Library title 1");
    let b = position("Library title 2");
    assert!(
        (b.y - a.y) < 38.0,
        "Copy rows have excess spacing: {a:?} {b:?}"
    );
    assert!((position("Apply").y - position("Restore").y).abs() < 1.0);
    for index in 0..3 {
        let edits: Vec<_> = texts(&output)
            .into_iter()
            .filter(|(t, _)| t.galley.text() == "Edit")
            .collect();
        let removes: Vec<_> = texts(&output)
            .into_iter()
            .filter(|(t, _)| t.galley.text() == "Remove")
            .collect();
        assert!((edits[index].0.pos.y - removes[index].0.pos.y).abs() < 1.0);
        assert!(edits[index].0.pos.x < removes[index].0.pos.x);
    }
    assert!(!texts(&output).iter().any(|(t, _)| {
        ["New title", "Edit title replacements", "Save changes"].contains(&t.galley.text())
    }));
}

#[test]
fn title_ui_restore_requires_confirmation_and_rechecks_closed_game() {
    let (ctx, mut host) = setup();
    host.first_status = false;
    click(&ctx, &mut host, "Restore", 0);
    let output = frame(&ctx, &mut host, vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(t, _)| t.galley.text() == "Restore title changes?")
    );
    assert!(!host.title_patcher.busy());
    host.last_rl_open = true;
    click(&ctx, &mut host, "Restore titles", 0);
    assert!(
        !host.title_patcher.busy(),
        "An open game must block confirmation too"
    );
    click(&ctx, &mut host, "Cancel", 0);
    let output = frame(&ctx, &mut host, vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(t, _)| t.galley.text() == "Restore title changes?")
    );
}

#[test]
#[ignore = "writes a visual QA artifact; optionally reads installed icons"]
fn title_ui_render_snapshot() {
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("examples/fonts/Barlow-Medium.ttf")).unwrap();
    fonts
        .font_data
        .insert("Barlow".into(), egui::FontData::from_owned(bytes).into());
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "Barlow".into());
    ctx.set_fonts(fonts);
    let mut host = fixture();
    host.config.settings.rl_path = std::env::var("HEBNIX_TITLE_ICON_DIR").unwrap_or_default();
    host.spoofer_title = "S10 {Silver} Tournament Winner".into();
    host.spoofer_title_color = [210, 210, 220];
    host.spoofer_title_glow = false;
    host.title_catalog[0].text = "Veteran".into();
    host.title_catalog[1].text = "S10 {Silver} Tournament Winner".into();
    host.title_catalog[2].text = "Choo Choo {Train}".into();
    host.title_catalog[3].text = "{flag_au} Rocket League".into();
    host.title_catalog[4].text = "S10 {Champion} Tournament Winner".into();
    host.title_catalog[5].text = "S10 {GrandChampion} Tournament Winner".into();
    host.title_catalog.truncate(6);
    host.spoofer_titles = vec![
        TitleSpoofSettings {
            text: host.spoofer_title.clone(),
            color: "D2D2DC".into(),
            glow: false,
            glow_color: None,
            target_id: Some("XP_0".into()),
        },
        TitleSpoofSettings {
            text: "Choo Choo {Train}".into(),
            color: "60D0F0".into(),
            glow: false,
            glow_color: None,
            target_id: Some("XP_1".into()),
        },
    ];
    let mut frames = Vec::new();
    let start = std::time::Instant::now();
    loop {
        frames.push(frame(&ctx, &mut host, vec![]));
        if (frames.len() > 4 && !crate::patcher::title_icons::loaded(&ctx).is_empty())
            || start.elapsed().as_secs() > 5
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for _ in 0..3 {
        frames.push(frame(&ctx, &mut host, vec![]));
    }
    snapshot::save(&ctx, &frames, &root.join("target/titles-refined-ui.png"));
}
