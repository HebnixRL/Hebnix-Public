use super::*;
use base64::Engine;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(crate) struct SettingsFrame {
    pub(crate) widgets: Vec<Value>,
    pub(crate) inputs: HashSet<String>,
}

struct Scope {
    ctx: egui::Context,
    frames: Vec<Vec<Value>>,
    counters: HashMap<String, usize>,
    inputs: HashSet<String>,
    action: Option<(String, Value)>,
    expanded: HashSet<String>,
}

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

pub(crate) fn capture(
    ctx: &egui::Context, action: Option<(String, Value)>, expanded: HashSet<String>,
    run: impl FnOnce() -> mlua::Result<()>,
) -> Result<SettingsFrame, String> {
    SCOPE.with_borrow_mut(|scope| *scope = Some(Scope {
        ctx: ctx.clone(), frames: vec![Vec::new()], counters: HashMap::new(),
        inputs: HashSet::new(), action, expanded,
    }));
    let result = run();
    let scope = SCOPE.with_borrow_mut(Option::take).expect("settings render scope");
    result.map_err(|error| error.to_string())?;
    Ok(SettingsFrame {
        widgets: scope.frames.into_iter().next().unwrap_or_default(),
        inputs: scope.inputs,
    })
}

pub(super) fn active() -> bool {
    SCOPE.with_borrow(Option::is_some)
}

fn id(scope: &mut Scope, kind: &str, key: &str) -> String {
    let prefix = format!("{kind}:{key}");
    let count = scope.counters.entry(prefix.clone()).or_default();
    let id = format!("{prefix}:{count}");
    *count += 1;
    id
}

pub(super) fn widget(kind: &str, key: &str, mut data: Value, interactive: bool) -> Option<Option<Value>> {
    SCOPE.with_borrow_mut(|scope| {
        let scope = scope.as_mut()?;
        let id = id(scope, kind, key);
        data["type"] = json!(kind);
        data["id"] = json!(id);
        if interactive { scope.inputs.insert(id.clone()); }
        let action = if interactive && scope.action.as_ref().is_some_and(|(target, _)| *target == id) {
            scope.action.take().map(|(_, value)| value)
        } else {
            None
        };
        scope.frames.last_mut()?.push(data);
        Some(action)
    })
}

pub(super) fn container(
    kind: &str, key: &str, mut data: Value, body: impl FnOnce() -> mlua::Result<()>,
) -> mlua::Result<bool> {
    let element = SCOPE.with_borrow_mut(|scope| {
        let scope = scope.as_mut()?;
        let id = id(scope, kind, key);
        let shown = kind != "collapsing" || scope.expanded.contains(&id);
        if kind == "collapsing" { scope.inputs.insert(id.clone()); }
        scope.frames.push(Vec::new());
        Some((id, shown))
    });
    let Some((id, shown)) = element else { return Ok(false) };
    let result = if shown { body() } else { Ok(()) };
    SCOPE.with_borrow_mut(|scope| {
        let scope = scope.as_mut().expect("settings render scope");
        let children = scope.frames.pop().unwrap_or_default();
        data["type"] = json!(kind);
        data["id"] = json!(id);
        data["children"] = json!(children);
        if kind == "collapsing" { data["open"] = json!(shown); }
        scope.frames.last_mut().expect("settings parent").push(data);
    });
    result.map(|()| true)
}

pub(super) fn copy(text: &str) -> bool {
    SCOPE.with_borrow(|scope| {
        if let Some(scope) = scope {
            scope.ctx.copy_text(text.to_owned());
            true
        } else { false }
    })
}

pub(super) fn image(key: &str, bytes: &[u8], opts: &Option<Table>) -> bool {
    if !active() { return false; }
    let mime = match image::guess_format(bytes).ok() {
        Some(image::ImageFormat::Png) => "image/png",
        Some(image::ImageFormat::Jpeg) => "image/jpeg",
        Some(image::ImageFormat::Gif) => "image/gif",
        Some(image::ImageFormat::WebP) => "image/webp",
        Some(image::ImageFormat::Bmp) => "image/bmp",
        _ if std::str::from_utf8(bytes).is_ok_and(|text| text.trim_start().starts_with("<svg")) => "image/svg+xml",
        _ => return false,
    };
    let image = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    let tint = opts.as_ref().and_then(|table| table.get::<String>("tint").ok());
    widget("image", key, json!({
        "image": image, "width": opt_num(opts, "width"), "height": opt_num(opts, "height"),
        "x": opt_num(opts, "x"), "y": opt_num(opts, "y"), "tint": tint,
    }), false);
    true
}
