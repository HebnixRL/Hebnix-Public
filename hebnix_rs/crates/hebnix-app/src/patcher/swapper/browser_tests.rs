//! Headless interaction/layout coverage; all disk writes use isolated fixtures.
use super::*;

struct BrowserFixture {
    root: PathBuf,
    ctx: egui::Context,
    state: SwapperState,
    category: SwapCategory,
    width: f32,
    textures: HashMap<egui::TextureId, egui::ColorImage>,
    recolour: bool,
    owned_ids: HashSet<i64>,
}

impl BrowserFixture {
    fn new(theme: &str, category: SwapCategory, manual: bool, width: f32) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hebnix-items-ui-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(root.join("Backups")).unwrap();
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let themes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        crate::theme::apply_theme(&ctx, &themes, &themes, theme).unwrap();
        let mut state = SwapperState::new(&root);
        state.walkthrough_mode = !manual;
        state.walkthrough_category = Some(category);
        state.owned_only = false;
        state.catalogs.insert(
            category,
            Arc::new(
                (0..35)
                    .map(|i| SwapItem {
                        name: format!("Test item {i}"),
                        upk: format!("test-{i}.upk"),
                        path: None,
                        thumbnail: None,
                        body_preview: None,
                        audio_bnk: None,
                        upk_type: "simple".into(),
                        product_id: Some(i),
                        car_key: Some("UNIVERSAL".into()),
                        car_name: Some("Universal".into()),
                        car_product_id: None,
                        shared_package: false,
                    })
                    .collect(),
            ),
        );
        state.resolutions.insert(
            category,
            Resolution {
                path: root.clone(),
                generation: 0,
                items: Some(Arc::new(vec![
                    ResolvedItem {
                        available: true,
                        thumbnail: None
                    };
                    35
                ])),
                cars: Some(Arc::new(vec![(
                    "UNIVERSAL".into(),
                    "Universal".into(),
                    None,
                )])),
            },
        );
        Self {
            root,
            ctx,
            state,
            category,
            width,
            textures: HashMap::new(),
            recolour: false,
            owned_ids: HashSet::from([2]),
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        let (tx, _rx) = unbounded();
        let cooked = &self.root;
        let backups = cooked.join("Backups");
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(self.width, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if self.recolour {
                        ui.heading(t("nav-item-colours"));
                        ui.horizontal_wrapped(|ui| {
                            self.state.render_recolour_controls(ui, &tx);
                        });
                        self.state
                            .render_recolour(ui, cooked, &backups, &tx, &self.owned_ids);
                    } else {
                        self.state.render_tab(
                            ui,
                            self.category,
                            cooked,
                            &backups,
                            &tx,
                            &self.owned_ids,
                        );
                    }
                    assert!(
                        ui.min_rect().right() <= self.width + 1.0,
                        "Content overflow at width {}, {:?}, walkthrough={}: {:?}",
                        self.width,
                        self.category,
                        self.state.walkthrough_mode,
                        ui.min_rect()
                    );
                });
            },
        );
        for (id, delta) in &output.textures_delta.set {
            let egui::ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let texture = self.textures.get_mut(id).unwrap();
                for row in 0..image.height() {
                    let start = (y + row) * texture.width() + x;
                    texture.pixels[start..start + image.width()].copy_from_slice(
                        &image.pixels[row * image.width()..(row + 1) * image.width()],
                    );
                }
            } else {
                self.textures.insert(*id, image.as_ref().clone());
            }
        }
        output
    }

    fn settle(&mut self) -> egui::FullOutput {
        self.frame(vec![]);
        self.frame(vec![])
    }

    fn click(&mut self, text: &str) {
        let output = self.settle();
        let pos = texts(&output)
            .into_iter()
            .rev()
            .find(|(_, value, _)| value == text)
            .unwrap_or_else(|| panic!("Missing button: {text}"))
            .0
            .center();
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
    }
}

impl Drop for BrowserFixture {
    fn drop(&mut self) {
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let root = self.root.canonicalize().unwrap();
        assert!(root.starts_with(&temp) && root != temp);
        fs::remove_dir_all(root).unwrap();
    }
}

fn texts(output: &egui::FullOutput) -> Vec<(egui::Rect, String, egui::Rect)> {
    fn visit(
        shape: &egui::Shape,
        clip: egui::Rect,
        found: &mut Vec<(egui::Rect, String, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(text) => found.push((
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                text.galley.text().into(),
                clip,
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, clip, found);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, shape.clip_rect, &mut result);
    }
    result
}

#[test]
fn items_browser_all_categories_modes_and_themes_fit_content() {
    for theme in ["Dark", "Ocean Studio Dark"] {
        for category in SwapCategory::ALL {
            for manual in [false, true] {
                for width in [600.0, 834.0, 1034.0] {
                    let mut fixture = BrowserFixture::new(theme, category, manual, width);
                    let before = fixture.ctx.style_of(fixture.ctx.theme());
                    let output = fixture.settle();
                    let rendered = texts(&output);
                    let expected_columns = match (width as u32, manual) {
                        (600, false) => 3,
                        (834, false) => 5,
                        (1034, false) => 6,
                        (600, true) => 2,
                        (834, true) => 3,
                        (1034, true) => 4,
                        _ => unreachable!(),
                    };
                    let expected_pages = 35_usize.div_ceil(expected_columns * 3);
                    assert!(
                        rendered
                            .iter()
                            .any(|(_, text, _)| text
                                .contains(&format!("Page 1 of {expected_pages}"))),
                        "A page must contain three rows: {theme} {category:?} {manual} {width}"
                    );
                    assert!(
                        rendered
                            .iter()
                            .any(|(_, text, _)| text.contains("Test item 0")),
                        "{theme} {category:?} {manual}"
                    );
                    for key in [
                        "tab-walkthrough-mode",
                        "tab-manual-mode",
                        "ball-show-applied",
                        "app-restore-all",
                        "app-reload-catalogs",
                        "spoofer-clear",
                    ] {
                        let (rect, _, clip) = rendered
                            .iter()
                            .find(|(_, text, _)| text == &t(key))
                            .unwrap();
                        assert!(
                            clip.expand(1.0).contains_rect(*rect),
                            "Clipped control {key}: {rect:?} in {clip:?}"
                        );
                    }
                    // Scoped presentation must not mutate the shell's styles.
                    assert_eq!(
                        format!("{before:?}"),
                        format!("{:?}", fixture.ctx.style_of(fixture.ctx.theme()))
                    );
                }
            }
        }
    }
}

#[test]
fn recolour_browser_shares_three_row_layout_search_and_owned_selection() {
    for theme in ["Dark", "Ocean Studio Dark"] {
        for (width, columns) in [(600.0, 3), (834.0, 5), (1034.0, 6)] {
            let mut f = BrowserFixture::new(theme, SwapCategory::Bodies, false, width);
            f.recolour = true;
            f.owned_ids = (0..35).collect();
            f.state.recolour_category = SwapCategory::Bodies;
            let output = f.settle();
            let expected_pages = 35_usize.div_ceil(columns * 3);
            assert!(
                texts(&output)
                    .iter()
                    .any(|(_, text, _)| text.contains(&format!("Page 1 of {expected_pages}")))
            );
            f.click("Test item 0");
            assert_eq!(
                f.state.recolour_selected,
                Some(item_identity(&f.state.catalogs[&SwapCategory::Bodies][0]))
            );
            assert!(f.state.active.is_empty());
            f.state.recolour_search = "no matches".into();
            assert!(
                texts(&f.settle())
                    .iter()
                    .any(|(_, text, _)| text.contains("(0 items)"))
            );
            f.click(&t("spoofer-clear"));
            assert!(f.state.recolour_search.is_empty());
            f.owned_ids.clear();
            f.settle();
            assert!(f.state.recolour_selected.is_none());
        }
    }
}

#[test]
fn items_browser_walkthrough_selection_search_ownership_and_modes() {
    for theme in ["Dark", "Ocean Studio Dark"] {
        let mut f = BrowserFixture::new(theme, SwapCategory::Bodies, false, 834.0);
        f.click("Test item 0");
        assert_eq!(f.state.walkthrough_source, Some(0));
        assert!(f.state.active.is_empty());
        f.click(&t("ball-next"));
        assert_eq!(f.state.walkthrough_step, WalkthroughStep::SelectTarget);
        f.state.owned_only = true;
        let rendered = texts(&f.settle());
        assert!(rendered.iter().any(|(_, text, _)| text == "Test item 2"));
        assert!(!rendered.iter().any(|(_, text, _)| text == "Test item 1"));
        f.click("Test item 2");
        assert_eq!(f.state.walkthrough_target, Some(2));
        assert!(f.state.active.is_empty());
        f.state
            .walkthrough_search
            .insert(SwapCategory::Bodies, "no match".into());
        let rendered = texts(&f.settle());
        assert!(
            rendered
                .iter()
                .any(|(_, text, _)| text.contains("(0 items)"))
        );
        assert!(f.state.walkthrough_target.is_none());
        f.click(&t("spoofer-clear"));
        assert!(f.state.walkthrough_search[&SwapCategory::Bodies].is_empty());
        f.click(&t("tab-manual-mode"));
        assert!(!f.state.walkthrough_mode);
        assert!(f.state.walkthrough_source.is_none());
        assert!(f.state.walkthrough_target.is_none());
    }
}

#[test]
fn items_browser_applied_uses_manifest_and_pagination_uses_filtered_count() {
    let category = SwapCategory::Bodies;
    let mut f = BrowserFixture::new("Ocean Studio Dark", category, false, 834.0);
    let source = &f.state.catalogs[&category][0];
    let target = &f.state.catalogs[&category][1];
    let active = ActiveSwap {
        category: category.slug().into(),
        source_name: source.name.clone(),
        source_upk: source.upk.clone(),
        target_name: target.name.clone(),
        target_upk: target.upk.clone(),
        target_bnk: None,
        paint: SwapPaint::Default,
        target_thumbnail: None,
        source_asset: None,
        target_asset: None,
        asset_override: false,
    };
    fs::write(f.root.join("Backups/test-1.upk.bak"), b"fixture").unwrap();
    fs::write(
        SwapperState::manifest_path(&f.root.join("Backups")),
        serde_json::to_vec(&vec![active]).unwrap(),
    )
    .unwrap();
    let rendered = texts(&f.settle());
    assert_eq!(
        rendered
            .iter()
            .filter(|(_, text, _)| text == "Applied")
            .count(),
        1
    );
    f.state.view_patched = true;
    let rendered = texts(&f.settle());
    assert!(
        rendered
            .iter()
            .any(|(_, text, _)| text.contains("(1 items)"))
    );
    f.state.view_patched = false;
    f.click("›|");
    assert_eq!(f.state.page[&category], 2);
    f.click("|‹");
    assert_eq!(f.state.page[&category], 0);
    f.click("2");
    assert_eq!(f.state.page[&category], 1);
    f.state.page.insert(category, 2);
    let rendered = texts(&f.settle());
    assert!(
        rendered
            .iter()
            .any(|(_, text, _)| text.contains("Page 3 of 3") && text.contains("(35 items)"))
    );
    f.state.search_input.insert(category, "Test item 0".into());
    let rendered = texts(&f.settle());
    assert_eq!(f.state.page[&category], 0);
    assert!(
        rendered
            .iter()
            .any(|(_, text, _)| text.contains("Page 1 of 1"))
    );
    assert_eq!(f.state.active.len(), 1);
}

#[test]
fn items_browser_long_names_preview_states_and_keyboard_selection() {
    for theme in ["Dark", "Ocean Studio Dark"] {
        for manual in [false, true] {
            let category = SwapCategory::Bodies;
            let mut f = BrowserFixture::new(theme, category, manual, 600.0);
            let items = Arc::make_mut(f.state.catalogs.get_mut(&category).unwrap());
            items[0].name =
                "A very long car body name with Unicode 雪 and enough words to require truncation"
                    .into();
            items[1].name =
                "A very long replacement name that must stay inside the replacement selector"
                    .into();
            f.state.walkthrough_source = Some(0);
            let output = f.settle();
            let rendered = texts(&output);
            assert!(
                rendered
                    .iter()
                    .any(|(_, text, _)| text.contains("A very long car body name"))
            );
            if manual {
                fn rectangles(shape: &egui::Shape, result: &mut Vec<egui::epaint::RectShape>) {
                    match shape {
                        egui::Shape::Rect(rect) => result.push(rect.clone()),
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                rectangles(shape, result);
                            }
                        }
                        _ => {}
                    }
                }
                let mut rects = Vec::new();
                for shape in &output.shapes {
                    rectangles(&shape.shape, &mut rects);
                }
                let surface = f.ctx.style_of(f.ctx.theme()).visuals.faint_bg_color;
                let selector_text = rendered
                    .iter()
                    .find(|(_, text, _)| text.contains("A very long replacement name"))
                    .expect("long replacement is selected")
                    .0;
                let card = rects
                    .iter()
                    .find(|shape| {
                        shape.fill == surface
                            && shape.rect.height() > 200.0
                            && shape.rect.contains(selector_text.center())
                    })
                    .expect("replacement has a containing card");
                let dropdown = rects
                    .iter()
                    .filter(|shape| {
                        shape.rect.height() < 60.0 && shape.rect.contains_rect(selector_text)
                    })
                    .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
                    .expect("replacement has a closed dropdown background");
                assert!(
                    card.rect.shrink(1.0).contains_rect(dropdown.rect),
                    "Dropdown exceeds item tile: {:?} vs {:?}",
                    dropdown.rect,
                    card.rect
                );
            }

            let item = f.state.catalogs[&category][0].clone();
            assert!(matches!(
                f.state.browser_preview(
                    category,
                    &item,
                    &ResolvedItem {
                        available: true,
                        thumbnail: None
                    }
                ),
                browser::Preview::Missing
            ));
            let resolved = ResolvedItem {
                available: true,
                thumbnail: Some("preview.upk".into()),
            };
            let key = thumbnail_key(category, "preview.upk", &item);
            assert!(matches!(
                f.state.browser_preview(category, &item, &resolved),
                browser::Preview::Loading
            ));
            f.state
                .failed_thumbnails
                .insert(key.clone(), "fixture extraction failure".into());
            assert!(matches!(
                f.state.browser_preview(category, &item, &resolved),
                browser::Preview::Failed(_)
            ));
            f.state.failed_thumbnails.remove(&key);
            f.state.thumbnails.insert(
                key,
                Some(Arc::from(
                    include_bytes!("../../../assets/hebnix.png").as_slice(),
                )),
            );
            assert!(matches!(
                f.state.browser_preview(category, &item, &resolved),
                browser::Preview::Ready(_)
            ));

            if !manual {
                let id = egui::Id::new(("walkthrough_source", category, item_identity(&item)));
                f.state.walkthrough_source = None;
                f.ctx.memory_mut(|memory| memory.request_focus(id));
                f.frame(vec![egui::Event::Key {
                    key: egui::Key::Space,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }]);
                assert_eq!(f.state.walkthrough_source, Some(0));
                assert!(f.state.active.is_empty());
            }
        }
    }
}

// Optional visual QA of the actual egui meshes, using existing image support.
// No window, game installation, graphics driver or extra dependency is needed.
#[test]
#[ignore = "set HEBNIX_ITEMS_UI_RENDER_DIR to export headless content previews"]
fn items_browser_export_previews() {
    let dir =
        PathBuf::from(std::env::var_os("HEBNIX_ITEMS_UI_RENDER_DIR").expect("render directory"));
    fs::create_dir_all(&dir).unwrap();
    for theme in ["Dark", "Ocean Studio Dark"] {
        for manual in [false, true] {
            let mut f = BrowserFixture::new(theme, SwapCategory::Bodies, manual, 834.0);
            f.state.walkthrough_source = Some(0);
            let output = f.settle();
            let mut canvas = image::RgbaImage::from_pixel(834, 700, image::Rgba([20, 20, 20, 255]));
            let edge = |a: egui::Pos2, b: egui::Pos2, p: egui::Pos2| {
                (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
            };
            for clipped in f.ctx.tessellate(output.shapes, output.pixels_per_point) {
                let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else {
                    continue;
                };
                let texture = &f.textures[&mesh.texture_id];
                for triangle in mesh.indices.chunks_exact(3) {
                    let v = triangle
                        .iter()
                        .map(|&i| mesh.vertices[i as usize])
                        .collect::<Vec<_>>();
                    let area = edge(v[0].pos, v[1].pos, v[2].pos);
                    if area.abs() < 0.0001 {
                        continue;
                    }
                    let rect = egui::Rect::from_points(&[v[0].pos, v[1].pos, v[2].pos])
                        .intersect(clipped.clip_rect)
                        .intersect(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(834.0, 700.0),
                        ));
                    if !rect.is_positive() {
                        continue;
                    }
                    for y in
                        rect.top().floor().max(0.0) as u32..rect.bottom().ceil().min(700.0) as u32
                    {
                        for x in rect.left().floor().max(0.0) as u32
                            ..rect.right().ceil().min(834.0) as u32
                        {
                            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                            let w = [
                                edge(v[1].pos, v[2].pos, p) / area,
                                edge(v[2].pos, v[0].pos, p) / area,
                                edge(v[0].pos, v[1].pos, p) / area,
                            ];
                            if w.iter().any(|&w| w < 0.0) {
                                continue;
                            }
                            let u = (0..3).map(|i| v[i].uv.x * w[i]).sum::<f32>();
                            let t = (0..3).map(|i| v[i].uv.y * w[i]).sum::<f32>();
                            let tx = (u * texture.width() as f32 - 0.5)
                                .clamp(0.0, (texture.width() - 1) as f32);
                            let ty = (t * texture.height() as f32 - 0.5)
                                .clamp(0.0, (texture.height() - 1) as f32);
                            let (ix, iy) = (tx.floor() as usize, ty.floor() as usize);
                            let texels = [
                                texture[(ix, iy)],
                                texture[((ix + 1).min(texture.width() - 1), iy)],
                                texture[(ix, (iy + 1).min(texture.height() - 1))],
                                texture[(
                                    (ix + 1).min(texture.width() - 1),
                                    (iy + 1).min(texture.height() - 1),
                                )],
                            ];
                            let (fx, fy) = (tx.fract(), ty.fract());
                            let tw = [
                                (1.0 - fx) * (1.0 - fy),
                                fx * (1.0 - fy),
                                (1.0 - fx) * fy,
                                fx * fy,
                            ];
                            let mut source = [0.0; 4];
                            for channel in 0..4 {
                                source[channel] = (0..3)
                                    .map(|i| v[i].color.to_array()[channel] as f32 * w[i])
                                    .sum::<f32>()
                                    / 255.0
                                    * (0..4)
                                        .map(|i| texels[i].to_array()[channel] as f32 * tw[i])
                                        .sum::<f32>();
                            }
                            let dest = canvas.get_pixel_mut(x, y);
                            for channel in 0..3 {
                                dest.0[channel] = (source[channel]
                                    + dest.0[channel] as f32 * (1.0 - source[3] / 255.0))
                                    .clamp(0.0, 255.0)
                                    as u8;
                            }
                        }
                    }
                }
            }
            canvas
                .save(dir.join(format!(
                    "{}-{}.png",
                    theme.replace(' ', "-"),
                    if manual { "manual" } else { "walkthrough" }
                )))
                .unwrap();
        }
    }
}

#[test]
fn recolour_category_tabs_exclude_audio_and_reset_item_selection() {
    for theme in ["Dark", "Ocean"] {
        let mut fixture = BrowserFixture::new(theme, SwapCategory::Bodies, false, 834.0);
        fixture.state.recolour_selected = Some("previous-item".into());
        fixture.state.recolour_status = Some(Err("previous-error".into()));
        fixture.state.recolour_page = 2;
        let frame = |fixture: &mut BrowserFixture, events| {
            fixture.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(834.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::secondary_navigation(ui, "recolour_categories_test").show(
                        ui,
                        |ui| {
                            fixture.state.render_recolour_categories(ui);
                        },
                    );
                    egui::CentralPanel::default().show(ui, |_| {});
                },
            )
        };
        frame(&mut fixture, vec![]);
        let output = frame(&mut fixture, vec![]);
        let rendered = texts(&output);
        for category in SwapCategory::ALL {
            assert_eq!(
                rendered
                    .iter()
                    .any(|(_, text, _)| text == &category.label()),
                category.supports_recolour()
            );
        }
        let pos = rendered
            .iter()
            .find(|(_, text, _)| text == &SwapCategory::Wheels.label())
            .unwrap()
            .0
            .center();
        frame(
            &mut fixture,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &mut fixture,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(fixture.state.recolour_category, SwapCategory::Wheels);
        assert_eq!(fixture.state.recolour_page, 0);
        assert!(fixture.state.recolour_selected.is_none());
        assert!(fixture.state.recolour_status.is_none());
    }
}

#[test]
fn owned_save_selector_is_below_the_swapper_title() {
    for theme in ["Dark", "Ocean"] {
        let mut fixture = BrowserFixture::new(theme, SwapCategory::Bodies, false, 1034.0);
        let mut inventory = crate::owned_inventory::OwnedInventory::empty_for_ui_tests();
        inventory.ids = fixture.owned_ids.clone();
        let (tx, _rx) = unbounded();
        let output = fixture.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1034.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    fixture.state.render_owned_tab(
                        ui,
                        SwapCategory::Bodies,
                        &fixture.root,
                        &fixture.root.join("Backups"),
                        &tx,
                        &mut inventory,
                    );
                });
            },
        );
        let rendered = texts(&output);
        let heading = rendered
            .iter()
            .find(|(_, text, _)| text == &SwapCategory::Bodies.label())
            .unwrap()
            .0;
        let selector = rendered
            .iter()
            .find(|(_, text, _)| text == "Owned items from save:")
            .unwrap()
            .0;
        assert!(
            heading.bottom() <= selector.top(),
            "Inventory selector must follow the page title"
        );
    }
}
