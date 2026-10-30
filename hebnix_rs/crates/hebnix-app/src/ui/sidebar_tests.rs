//! Regression checks for the narrow sidebar shown in the theme feedback.
use eframe::egui;
use crate::{theme, toast::ToastCenter, ui::toasts::{self, ToastView}};

struct SidebarFrame {
    output: egui::FullOutput,
    action: egui::Rect,
    notifications: egui::Rect,
    status: egui::Rect,
    pages: egui::Rect,
    bounds: egui::Rect,
    selected: Option<usize>,
}
fn frame(ctx: &egui::Context, p: &theme::Presentation, labels: &[String],
    view: &mut ToastView, center: &mut ToastCenter, events: Vec<egui::Event>) -> SidebarFrame {
    let mut result = SidebarFrame { output: Default::default(), action: egui::Rect::NOTHING,
        notifications: egui::Rect::NOTHING, status: egui::Rect::NOTHING,
        pages: egui::Rect::NOTHING, bounds: egui::Rect::NOTHING, selected: None };
    result.output = ctx.run_ui(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1250.0,700.0))),
        events, ..Default::default()
    }, |ui| theme::show_shell(ui, p, |ui, part| {
        if let theme::ShellPart::Navigation(_) = part {
            result.bounds = ui.clip_rect();
            theme::sidebar(ui, |ui, part| match part {
                theme::SidebarPart::Footer => {
                    result.action = ui.scope(|ui| {
                        result.selected = p.sidebar_actions(ui,labels);
                    }).response.rect;
                    ui.add_space(6.0);
                    result.notifications = ui.horizontal(|ui| {
                        toasts::render_bell(ui, view, center);
                        ui.add(egui::Label::new(egui::RichText::new("Notifications").small()).wrap());
                    }).response.rect;
                    result.status = ui.add(egui::Label::new(
                        egui::RichText::new("✓ Rocket League Connected").small()).wrap()).rect;
                }
                theme::SidebarPart::Pages => {
                    result.pages = ui.available_rect_before_wrap();
                    p.brand(ui);
                    egui::ScrollArea::vertical().id_salt("test_sidebar_pages").auto_shrink([false,false])
                        .show(ui, |ui| {
                            for tab in super::Tab::ALL {
                                p.nav_item(ui,false,tab.config_key(),&tab.label(),Some(ui.available_width()));
                            }
                        });
                }
            });
        }
    }));
    result
}
fn text_shapes<'a>(shape: &'a egui::Shape, result: &mut Vec<&'a egui::epaint::TextShape>) {
    match shape {
        egui::Shape::Text(text) => result.push(text),
        egui::Shape::Vec(shapes) => for shape in shapes { text_shapes(shape,result); },
        _ => {}
    }
}
fn text_pos(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    let mut texts = Vec::new();
    for shape in &output.shapes { text_shapes(&shape.shape,&mut texts); }
    let text = texts.into_iter().find(|t| t.galley.text() == label)
        .unwrap_or_else(|| panic!("Missing text: {label}"));
    text.pos + text.galley.size()*0.5
}
fn chevron_pos(output: &egui::FullOutput) -> egui::Pos2 {
    fn find(shape: &egui::Shape) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::LineSegment {points, ..}
                if (points[1]-points[0]-egui::vec2(3.5,3.5)).length()<0.01 =>
                    Some(points[1]-egui::vec2(0.0,2.0)),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(find),
            _ => None,
        }
    }
    output.shapes.iter().find_map(|s|find(&s.shape)).expect("Missing vector dropdown icon")
}
fn setup(width: f32) -> (egui::Context, theme::Presentation, ToastView, ToastCenter) {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
    theme::apply_theme(&ctx,&dir,&dir,"Ocean Studio Dark").unwrap();
    let mut p = (*theme::presentation(&ctx).unwrap()).clone();
    p.layout.navigation_width = width;
    let mut center = ToastCenter::default();
    center.push("example","Example","Unread notification",Default::default());
    (ctx,p,ToastView::default(),center)
}
#[test]
fn sidebar_footer_controls_fit_without_overlap_or_elided_labels() {
    for width in [96.0,184.0,320.0] {
        for font_size in [14.0,22.0] {
            for actions in [vec!["Start Rocket League"],vec!["Restart Rocket League","Close Rocket League","Reload Plugins"]] {
                let (ctx,p,mut view,mut center)=setup(width);
                ctx.style_mut_of(ctx.theme(),|s| {
                    s.text_styles.insert(egui::TextStyle::Button,egui::FontId::proportional(font_size));
                });
                let labels: Vec<String> = actions.iter().map(|s|s.to_string()).collect();
                for _ in 0..3 { frame(&ctx,&p,&labels,&mut view,&mut center,vec![]); }
                let result=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
                for rect in [result.action,result.notifications,result.status] {
                    assert!(result.bounds.expand(1.0).contains_rect(rect), "Sidebar {width}/{font_size}: control {rect:?} escapes {:?}",result.bounds);
                }
                assert!(result.action.bottom()<=result.notifications.top(), "Action overlaps notifications");
                assert!(result.notifications.bottom()<=result.status.top(), "Notifications overlap status");
                assert!(result.pages.bottom()<=result.action.top(), "Navigation overlaps footer");
                let mut texts=Vec::new();
                for shape in &result.output.shapes { text_shapes(&shape.shape,&mut texts); }
                for label in [labels[0].as_str(),"Notifications","✓ Rocket League Connected","1"] {
                    let text=texts.iter().find(|t|t.galley.text()==label)
                        .unwrap_or_else(||panic!("Sidebar {width}/{font_size} missing {label}"));
                    assert!(!text.galley.elided,"Sidebar {width}/{font_size} truncates {label}");
                }
            }
        }
    }
}
#[test]
fn sidebar_primary_and_menu_actions_dispatch_once() {
    let (ctx,p,mut view,mut center)=setup(184.0);
    let labels=vec!["Restart Rocket League".into(),"Close Rocket League".into(),"Reload Plugins".into()];
    for _ in 0..3 { frame(&ctx,&p,&labels,&mut view,&mut center,vec![]); }
    let output=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
    let primary=text_pos(&output.output,&labels[0]);
    let click=|pos,pressed|vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {
        pos,button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE
    }];
    assert_eq!(frame(&ctx,&p,&labels,&mut view,&mut center,click(primary,true)).selected,None);
    assert_eq!(frame(&ctx,&p,&labels,&mut view,&mut center,click(primary,false)).selected,Some(0));
    let output=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
    let menu=chevron_pos(&output.output);
    frame(&ctx,&p,&labels,&mut view,&mut center,click(menu,true));
    assert_eq!(frame(&ctx,&p,&labels,&mut view,&mut center,click(menu,false)).selected,None);
    let output=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
    let secondary=text_pos(&output.output,&labels[1]);
    frame(&ctx,&p,&labels,&mut view,&mut center,click(secondary,true));
    assert_eq!(frame(&ctx,&p,&labels,&mut view,&mut center,click(secondary,false)).selected,Some(1));
    assert_eq!(frame(&ctx,&p,&labels,&mut view,&mut center,vec![]).selected,None);
}

#[test]
fn themed_toolbar_keeps_navigation_actions_bell_and_status_inline() {
    for name in ["Ocean Studio Dark", "Hebnix 2"] {
    for width in [700.0,1250.0,2000.0] {
        let (ctx,_,mut view,mut center)=setup(184.0);
        let dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
        theme::apply_theme(&ctx,&dir,&dir,name).unwrap();
        let mut p=(*theme::presentation(&ctx).unwrap()).clone();
        p.layout.shell=theme::Shell::Topbar;
        let labels=vec!["Restart Rocket League".into(),"Close Rocket League".into()];
        let mut rows=Vec::new();
        let mut toolbar=egui::Rect::NOTHING;
        let mut bounds=egui::Rect::NOTHING;
        for _ in 0..4 {
            rows.clear();
            let _ = ctx.run_ui(egui::RawInput {
                screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,700.0))),
                ..Default::default()
            }, |ui|theme::show_shell(ui,&p,|ui,part|if let theme::ShellPart::Navigation(false)=part {
                bounds=ui.available_rect_before_wrap();
                toolbar=theme::toolbar(ui,|ui| {
                    for key in ["Console","Maps","Spoofer","Colours","Items","Settings","Plugins","Replays","About"] {
                        rows.push(p.nav_item(ui,false,key,key,None).rect);
                    }
                    ui.add_space(8.0);
                    p.toolbar_actions(ui,|ui| {
                        rows.push(ui.scope(|ui|{p.inline_actions(ui,&labels);}).response.rect);
                        rows.push(ui.scope(|ui|{toasts::render_bell(ui,&mut view,&mut center);}).response.rect);
                        rows.push(ui.add(egui::Label::new("Waiting for Rocket League…").wrap_mode(egui::TextWrapMode::Extend)).rect);
                    });
                });
            }));
        }
        for row in &rows { assert!(row.top()<=rows[0].center().y && row.bottom()>=rows[0].center().y,"Control moved to a second row at {width}: {rows:?}"); }
        for pair in rows.windows(2) { assert!(pair[0].right()<=pair[1].left(),"Toolbar controls overlap"); }
        assert!(toolbar.height()<55.0,"Toolbar unexpectedly wraps: {toolbar:?}");
        if width==700.0 { assert!(toolbar.width()>width,"Narrow toolbar must scroll horizontally"); }
        if width>700.0 && p.layout.right_align_actions {
            assert!((rows.last().unwrap().right()-bounds.right()).abs()<1.0,"Actions must end at the right edge: {rows:?}, bounds {bounds:?}");
        }
    }
    }
}

#[test]
fn joined_toolbar_actions_share_an_outline_and_keep_both_click_targets() {
    let (ctx,_,_,_)=setup(184.0);
    let dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes");
    theme::apply_theme(&ctx,&dir,&dir,"Hebnix 2").unwrap();
    let p=theme::presentation(&ctx).unwrap();
    let labels=vec!["Restart Rocket League".into(),"Close Rocket League".into()];
    let render=|events| {
        let mut selected=None;
        let output=ctx.run_ui(egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1250.0,700.0))),
            events,..Default::default()
        },|ui| {theme::toolbar(ui,|ui| {p.toolbar_actions(ui,|ui| {selected=p.inline_actions(ui,&labels);});});});
        (output,selected)
    };
    for _ in 0..3 { render(vec![]); }
    let (output,_)=render(vec![]);
    let primary=text_pos(&output,&labels[0]);
    let menu=chevron_pos(&output);
    fn rects<'a>(shape:&'a egui::Shape,out:&mut Vec<&'a egui::epaint::RectShape>) {
        match shape {egui::Shape::Rect(rect)=>out.push(rect),egui::Shape::Vec(shapes)=>for shape in shapes {rects(shape,out)},_=>{}}
    }
    let mut painted=Vec::new();
    for shape in &output.shapes {rects(&shape.shape,&mut painted);}
    let segment=|pos|painted.iter().find(|r|r.rect.contains(pos)&&r.rect.height()==26.0&&r.rect.width()<300.0&&r.stroke==egui::Stroke::NONE).unwrap();
    let main=segment(primary);
    let arrow=segment(menu);
    assert_eq!(main.rect.right(),arrow.rect.left(),"Split action must have no gap");
    assert_eq!(main.corner_radius.ne,0);
    assert_eq!(main.corner_radius.se,0);
    assert_eq!(arrow.corner_radius.nw,0);
    assert_eq!(arrow.corner_radius.sw,0);
    assert!(painted.iter().any(|r|r.rect==main.rect.union(arrow.rect)&&r.stroke.width>0.0),"One outline must enclose both segments");
    let click=|pos,pressed|vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE}];
    render(click(primary,true));
    assert_eq!(render(click(primary,false)).1,Some(0));
    assert_eq!(render(vec![]).1,None);
    render(click(menu,true));
    assert_eq!(render(click(menu,false)).1,None);
    let (output,_)=render(vec![]);
    let secondary=text_pos(&output,&labels[1]);
    render(click(secondary,true));
    assert_eq!(render(click(secondary,false)).1,Some(1));
    assert_eq!(render(vec![]).1,None);
}

#[test]
fn normal_sidebar_launch_label_stays_on_one_line() {
    let (ctx,p,mut view,mut center)=setup(184.0);
    let labels=vec!["Start Rocket League".into()];
    let output=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
    let mut texts=Vec::new();
    for shape in &output.output.shapes { text_shapes(&shape.shape,&mut texts); }
    let text=texts.iter().find(|t|t.galley.text()==labels[0]).unwrap();
    assert_eq!(text.galley.rows.len(),1);
}

#[test]
fn sidebar_header_is_text_and_page_icons_render_as_images() {
    let (ctx,p,mut view,mut center)=setup(184.0);
    let labels=vec!["Start Rocket League".into()];
    for _ in 0..3 { frame(&ctx,&p,&labels,&mut view,&mut center,vec![]); }
    let output=frame(&ctx,&p,&labels,&mut view,&mut center,vec![]);
    fn images(shape:&egui::Shape)->usize {
        match shape {
            egui::Shape::Mesh(mesh) if !mesh.indices.is_empty()=>1,
            egui::Shape::Rect(rect) if rect.brush.is_some()=>1,
            egui::Shape::Vec(shapes)=>shapes.iter().map(images).sum(),
            _=>0,
        }
    }
    let image_count:usize=output.output.shapes.iter().map(|s|images(&s.shape)).sum();
    assert_eq!(image_count,super::Tab::ALL.len()+1,"Expected one icon per navigation page and the bell, with no header image");
    let _=text_pos(&output.output,"Hebnix");
}

