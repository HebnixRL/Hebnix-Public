//! Controller side panel (Steam Deck / gamepad): pad polling and docked window geometry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui;
use hebnix_sdk::input::{dinput_name_to_xinput_mask, get_dinput_inputs, get_xinput_state};

use crate::messages::AppMsg;

const DPAD_UP: u16 = 0x0001;
const DPAD_DOWN: u16 = 0x0002;
const DPAD_LEFT: u16 = 0x0004;
const DPAD_RIGHT: u16 = 0x0008;
const LS: u16 = 0x0040;
const RS: u16 = 0x0080;
const LB: u16 = 0x0100;
const RB: u16 = 0x0200;
const A: u16 = 0x1000;
const B: u16 = 0x2000;
/// the triggers are analog on xinput; these two mask bits xinput leaves unused stand for them
const LT: u16 = 0x0400;
const RT: u16 = 0x0800;
const TRIGGER_THRESHOLD: u8 = 100;
/// both stick clicks together open and close the panel
const CHORD: u16 = LS | RS;
const STICK_DEADZONE: i16 = 16000;
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_RATE: Duration = Duration::from_millis(100);
/// how long the panel takes to slide in
const SLIDE: Duration = Duration::from_millis(220);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    Up,
    Down,
    Left,
    Right,
    Accept,
    Back,
    PrevPage,
    NextPage,
    /// L2 / R2: page through a list, like the map browser's pages
    PrevListPage,
    NextListPage,
}

/// logical buttons held right now, as xinput masks, from any xinput or dinput pad
fn held() -> u16 {
    let mut mask = 0;
    for index in 0..4 {
        if let Some(state) = get_xinput_state(index) {
            mask |= state.buttons;
            if state.left_trigger > TRIGGER_THRESHOLD {
                mask |= LT;
            }
            if state.right_trigger > TRIGGER_THRESHOLD {
                mask |= RT;
            }
            // left stick doubles as a d-pad
            if state.thumb_lx > STICK_DEADZONE {
                mask |= DPAD_RIGHT;
            } else if state.thumb_lx < -STICK_DEADZONE {
                mask |= DPAD_LEFT;
            }
            if state.thumb_ly > STICK_DEADZONE {
                mask |= DPAD_UP;
            } else if state.thumb_ly < -STICK_DEADZONE {
                mask |= DPAD_DOWN;
            }
        }
    }
    for name in get_dinput_inputs() {
        if let Some(bit) = dinput_name_to_xinput_mask(&name) {
            mask |= bit;
        }
        match name.as_str() {
            "l2" => mask |= LT,
            "r2" => mask |= RT,
            _ => {}
        }
    }
    mask
}

fn event_for(bit: u16) -> Option<PadEvent> {
    Some(match bit {
        DPAD_UP => PadEvent::Up,
        DPAD_DOWN => PadEvent::Down,
        DPAD_LEFT => PadEvent::Left,
        DPAD_RIGHT => PadEvent::Right,
        A => PadEvent::Accept,
        B => PadEvent::Back,
        LB => PadEvent::PrevPage,
        RB => PadEvent::NextPage,
        LT => PadEvent::PrevListPage,
        RT => PadEvent::NextListPage,
        _ => return None,
    })
}

/// set when keyboard focus moved since the last frame, so scroll areas bring it into view
static FOCUS_MOVED: AtomicBool = AtomicBool::new(false);

/// Call at the end of a scroll area's content: if the pad just moved focus to a widget inside
/// it, scroll that widget into view. Egui doesn't do this for keyboard focus.
pub fn follow_focus(ui: &egui::Ui) {
    if !FOCUS_MOVED.load(Ordering::Relaxed) {
        return;
    }
    let ctx = ui.ctx();
    if let Some(rect) = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .map(|response| response.rect)
    {
        if ui.min_rect().intersects(rect) {
            ui.scroll_to_rect(rect, None);
        }
    }
}

/// A scroll area's content with `follow_focus` after it. A scroll target only reaches the
/// area it was set in, so nested areas (a page's own list inside the panel) each need this.
pub fn followed<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let inner = add(ui);
    follow_focus(ui);
    inner
}

/// Pad poller. The chord always works; the other buttons only queue while the panel is open,
/// so playing the game never wakes the UI.
pub struct Pad {
    active: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<PadEvent>>>,
}

impl Pad {
    pub fn spawn(tx: Sender<AppMsg>, ctx: egui::Context) -> Self {
        let active = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (thread_active, thread_events) = (Arc::clone(&active), Arc::clone(&events));
        std::thread::Builder::new()
            .name("panel-pad".into())
            .spawn(move || {
                let mut previous = held();
                let mut repeat: Option<(u16, Instant)> = None;
                loop {
                    std::thread::sleep(Duration::from_millis(16));
                    let now = held();
                    let rising = now & !previous;
                    previous = now;
                    if now & CHORD == CHORD && rising & CHORD != 0 {
                        crate::ui_work::send_control(&tx, &ctx, AppMsg::TogglePanel);
                        continue;
                    }
                    if !thread_active.load(Ordering::Relaxed) {
                        repeat = None;
                        continue;
                    }
                    let mut fresh = Vec::new();
                    for bit in [DPAD_UP, DPAD_DOWN, DPAD_LEFT, DPAD_RIGHT, A, B, LB, RB, LT, RT] {
                        if rising & bit != 0 {
                            fresh.extend(event_for(bit));
                            if bit & (DPAD_UP | DPAD_DOWN | DPAD_LEFT | DPAD_RIGHT) != 0 {
                                repeat = Some((bit, Instant::now() + REPEAT_DELAY));
                            }
                        }
                    }
                    if let Some((bit, due)) = repeat {
                        if now & bit == 0 {
                            repeat = None;
                        } else if Instant::now() >= due {
                            fresh.extend(event_for(bit));
                            repeat = Some((bit, Instant::now() + REPEAT_RATE));
                        }
                    }
                    if !fresh.is_empty() {
                        if let Ok(mut queue) = thread_events.lock() {
                            queue.extend(fresh);
                        }
                        ctx.request_repaint();
                    }
                }
            })
            .ok();
        Self { active, events }
    }

    pub fn set_active(&self, on: bool) {
        self.active.store(on, Ordering::Relaxed);
        if !on {
            self.drain();
        }
    }

    pub fn drain(&self) -> Vec<PadEvent> {
        self.events
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }
}

/// Where and how the panel opens, from the Hebnix settings.
#[derive(Clone, Copy)]
pub struct Look {
    pub left: bool,
    /// share of the game window's width, 0.2 to 0.5
    pub width: f32,
    pub animate: bool,
}

/// Docked window state: the panel sits on one edge of the game window and puts the
/// normal window back where it was on the way out.
pub struct Panel {
    pub active: bool,
    pub pad: Pad,
    /// open page, or None for the card list
    pub page: Option<usize>,
    /// page steps and back presses seen by the input hook, taken by the next frame
    pub pending_page: i32,
    pub pending_back: bool,
    /// L2 / R2 steps through the open list's pages, taken by the next frame
    pub pending_list_page: i32,
    /// D-pad moves waiting for a frame: egui takes one focus move per frame
    pub pending_nav: Vec<PadEvent>,
    last_focus: Option<egui::Id>,
    /// the up/down just issued, the widget it started from and that widget's rect
    check: Option<(PadEvent, egui::Id, egui::Rect)>,
    saved: Option<(egui::Pos2, egui::Vec2)>,
    /// the slide-in under way: when it started, where from and where to
    slide: Option<(Instant, egui::Pos2, egui::Pos2)>,
}

impl Panel {
    pub fn new(tx: Sender<AppMsg>, ctx: egui::Context) -> Self {
        Self {
            active: false,
            pad: Pad::spawn(tx, ctx),
            page: None,
            pending_page: 0,
            pending_back: false,
            pending_list_page: 0,
            pending_nav: Vec::new(),
            last_focus: None,
            check: None,
            saved: None,
            slide: None,
        }
    }

    pub fn enter(&mut self, ctx: &egui::Context, look: Look) {
        let ppp = ctx.pixels_per_point();
        let share = look.width.clamp(0.2, 0.5);
        // the area the panel docks into, in points: the game window, or the monitor
        let (area_min, area_size) = match hebnix_sdk::process::get_rocket_league_window_rect() {
            Some((left, top, right, bottom)) => (
                egui::pos2(left as f32, top as f32) / ppp,
                egui::vec2((right - left) as f32, (bottom - top) as f32) / ppp,
            ),
            None => (
                egui::Pos2::ZERO,
                ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1280.0, 800.0)),
            ),
        };
        let width = (area_size.x * share).clamp(320.0, area_size.x.max(320.0));
        let size = egui::vec2(width, area_size.y);
        let pos = if look.left { area_min } else { egui::pos2(area_min.x + area_size.x - width, area_min.y) };
        // slides in from the edge it docks to
        let from = egui::pos2(if look.left { pos.x - width } else { pos.x + width }, pos.y);
        self.slide = (look.animate && !self.active).then(|| (Instant::now(), from, pos));
        if !self.active {
            self.saved = ctx.input(|i| {
                let viewport = i.viewport();
                Some((viewport.outer_rect?.min, viewport.inner_rect?.size()))
            });
        }
        self.active = true;
        self.pad.set_active(true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(320.0, 240.0)));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.slide.map_or(pos, |(_, from, _)| from)));
    }

    /// Move the window one step along the slide-in, if one is under way.
    fn step_slide(&mut self, ctx: &egui::Context) {
        let Some((start, from, to)) = self.slide else { return };
        let t = (start.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
        // ease out: fast at first, settling into place like a drawer
        let eased = 1.0 - (1.0 - t).powi(3);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(from.lerp(to, eased)));
        if t >= 1.0 {
            self.slide = None;
        } else {
            ctx.request_repaint();
        }
    }

    /// Start of a panel frame: note whether focus moved last frame, check that an up or down
    /// really went up or down, then take one queued move.
    /// Moves go through `move_focus` instead of arrow keys, because a text box keeps the arrow
    /// keys for itself and the pad could never leave it. Egui's own up/down search sometimes
    /// skips a widget or wraps to the top in these layouts, so when it lands anywhere but
    /// above/below where we were, the move is redone in reading order from the old widget.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.step_slide(ctx);
        let rect_of = |id: egui::Id| ctx.read_response(id).map(|response| response.rect);
        let mut focused = ctx.memory(|m| m.focused());
        if let Some((direction, from, from_rect)) = self.check.take() {
            let went_right_way = focused != Some(from)
                && focused.and_then(rect_of).is_some_and(|now| match direction {
                    PadEvent::Up => now.center().y < from_rect.center().y - 1.0,
                    _ => now.center().y > from_rect.center().y + 1.0,
                });
            if !went_right_way {
                let order = if direction == PadEvent::Up {
                    egui::FocusDirection::Previous
                } else {
                    egui::FocusDirection::Next
                };
                ctx.memory_mut(|m| {
                    m.request_focus(from);
                    m.move_focus(order);
                });
                focused = Some(from);
            }
        }
        FOCUS_MOVED.store(focused != self.last_focus, Ordering::Relaxed);
        self.last_focus = focused;
        if self.pending_nav.is_empty() {
            return;
        }
        let event = self.pending_nav.remove(0);
        let direction = match (event, focused) {
            (_, None) => egui::FocusDirection::Next,
            (PadEvent::Up, _) => egui::FocusDirection::Up,
            (PadEvent::Down, _) => egui::FocusDirection::Down,
            (PadEvent::Left, _) => egui::FocusDirection::Left,
            _ => egui::FocusDirection::Right,
        };
        if let (PadEvent::Up | PadEvent::Down, Some(from)) = (event, focused) {
            self.check = rect_of(from).map(|rect| (event, from, rect));
        }
        ctx.memory_mut(|m| m.move_focus(direction));
        ctx.request_repaint();
    }

    pub fn leave(&mut self, ctx: &egui::Context) {
        if !self.active {
            return;
        }
        self.active = false;
        self.slide = None;
        self.pad.set_active(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
            crate::app::MIN_WIDTH,
            crate::app::MIN_HEIGHT,
        )));
        if let Some((pos, size)) = self.saved.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
        }
    }
}

/// Turn pad moves into key events before egui starts its frame, and tell egui the window is
/// focused. Egui drops keyboard focus whenever its window isn't the active one, so without
/// this the pad can't select anything unless Windows actually gave Hebnix the foreground.
/// Returns the page, back and list page requests, which the app handles itself.
pub fn feed(
    ctx: &egui::Context,
    raw: &mut egui::RawInput,
    events: &[PadEvent],
    nav: &mut Vec<PadEvent>,
) -> (i32, bool, i32) {
    if let Some(viewport) = raw.viewports.get_mut(&raw.viewport_id) {
        viewport.focused = Some(true);
    }
    raw.events.push(egui::Event::WindowFocused(true));
    let (mut page, mut back, mut list_page) = (0, false, 0);
    for event in events {
        let key = match event {
            PadEvent::Up | PadEvent::Down | PadEvent::Left | PadEvent::Right => {
                nav.push(*event);
                continue;
            }
            PadEvent::Accept => egui::Key::Enter,
            // an open dropdown closes first, like Escape on a keyboard
            PadEvent::Back if egui::Popup::is_any_open(ctx) => egui::Key::Escape,
            PadEvent::Back => {
                back = true;
                continue;
            }
            PadEvent::PrevPage => {
                page -= 1;
                continue;
            }
            PadEvent::NextPage => {
                page += 1;
                continue;
            }
            PadEvent::PrevListPage => {
                list_page -= 1;
                continue;
            }
            PadEvent::NextListPage => {
                list_page += 1;
                continue;
            }
        };
        for pressed in [true, false] {
            raw.events.push(egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    if !events.is_empty() {
        // focus moves when egui ends this frame, so one more frame is needed to draw it
        // even while the window is not the one Windows considers active
        ctx.request_repaint();
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    (page, back, list_page)
}
