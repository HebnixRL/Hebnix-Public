//! Independently assembled UnrealScript hooks. No new native-backed fields.
use super::super::{script_patch::{Script, function_payload}, upk_package::UpkPackage};
// Bump whenever generated hooks or their Game Mods/scoreboard dependencies change.
pub const REVISION: u32 = 29;
#[path = "boost.rs"]
mod boost;
#[path = "items.rs"]
mod items;
// ECastToken values, distinct from the game's remapped expression tokens.
const STRING_TO_FLOAT: u8 = 0x4c;
const FLOAT_TO_STRING: u8 = 0x55;
// Verified against stock assignments from IntProperty to StrProperty.
// 0x54 is BoolToString and turns registry indices into "True"/"False".
const INT_TO_STRING: u8 = 0x53;
pub const TARGETS: &[&str] = &[
    "UISavedValues_TA.Get", "PlayerController_TA.PlayerTick", "PlayerController_Menu_TA.PlayerTick",
    "CarMeshComponent_TA.InitWheelControls", "CarMeshComponentBase_TA.SetMaterialColors",
    "Team_TA.UpdateColors",
    "GFxShell_TA.GetKeyForAction",
    "Ball_God_TA.GetMaxSpeedColor", "Ball_God_TA.GetActiveTeamColor",
    "Team_TA.GetColors", // Retire the earlier lookup hook during upgrades.
    "Team_TA.SetDefaultColors",
    "Ball_TA.SetExplosionFXActor",
    "FXActor_Boost_TA.HandleUpdatedPaintSettings",
    "CarMeshComponentBase_TA.ApplyPaintSettingsToObject",
    "PlayerVanity_TA.ApplyColor",
    "ProductAttribute_Painted_TA.ApplyPaintToObject",
    "Ball_TA.InitExplosionFX",
    "CarMeshComponentBase_TA.ApplyAttributes",
    "ExplosionPreviewer_TA.ApplyExplosionParameters",
    "ProductOverride_ParticleSystemColorParameter_TA.GetPaintToApply",
    "CarMeshComponent_TA.SetLoadout",
];
type R<T> = Result<T, String>;
/// Import records are just names: assembling a valid reference does not prove
/// that the game can resolve it. Check the owning packages before installation.
pub(super) fn verify_dependencies(cooked: &std::path::Path) -> R<()> {
    for (file, dependencies) in [
        ("Core.upk", &[("Object.GetSingleton", "Function"), ("Object.FindObject", "Function"),
            ("Object.PathName", "Function"), ("Object.IsPendingKill", "Function"),
            ("Object.TimeStamp", "Function"), ("Object.TimeStamp.ReturnValue", "StrProperty"),
            ("FileSystem.SaveStringToFile", "Function"), ("Object.Color.R", "ByteProperty"),
            ("Object.Color.G", "ByteProperty"), ("Object.Color.B", "ByteProperty"), ("Object.Color.A", "ByteProperty")][..]),
        ("Engine.upk", &[("ObjectReferencer", "Class"), ("ObjectReferencer.ReferencedObjects", "ArrayProperty"),
            ("GameEngine", "Class"), ("GameEngine.ObjectReferencers", "ArrayProperty"),
            ("ActorComponent.Owner", "ObjectProperty"), ("Actor.bDeleteMe", "BoolProperty"),
            ("MeshComponent.GetNumElements", "Function"), ("MeshComponent.GetNumElements.ReturnValue", "IntProperty"),
            ("MeshComponent.CreateAndSetMaterialInstanceConstant", "Function"), ("MeshComponent.SetMaterial", "Function"),
            ("MaterialInstanceConstant", "Class"), ("MaterialInstanceConstant.SetParent", "Function"),
            ("MaterialInstanceConstant.GetVectorParameterValue", "Function"),
            ("MaterialInstanceConstant.GetVectorParameterValue.ReturnValue", "BoolProperty"),
            ("MaterialInstanceConstant.SetVectorParameterValue", "Function"),
            ("MaterialInstance", "Class"), ("MaterialInstance.Parent", "ObjectProperty"),
            ("MaterialInstanceConstant.VectorParameterValues", "ArrayProperty"),
            ("MaterialInstanceConstant.ScalarParameterValues", "ArrayProperty"),
            ("MaterialInstanceConstant.TextureParameterValues", "ArrayProperty"),
            ("MaterialInstanceConstant.FontParameterValues", "ArrayProperty"),
            ("MeshComponent.CreateAndSetMaterialInstanceConstant.ReturnValue", "ObjectProperty"),
            ("MeshComponent.SetLinearColorParameter", "Function")][..]),
        ("ProjectX.upk", &[("FXActor_X.AttachmentActor", "ObjectProperty"),
            ("FXActor_X.IsStateActive", "Function"), ("FXActor_X.IsStateActive.ReturnValue", "BoolProperty"),
            ("FXActor_X.RemoveStateForceUpdate", "Function"), ("ParameterDispenser_X.FloatParams", "ArrayProperty")][..]),
    ] {
        let path = super::super::game_path::resolve(cooked, file);
        let package = UpkPackage::load(&path).map_err(|error| format!("Cannot verify live colour dependencies in {file}: {error}"))?;
        for (path, kind) in dependencies {
            let index = package.find_object(path).filter(|index| *index > 0)
                .ok_or_else(|| format!("Live colours require {file}:{path}; installation left the game untouched"))? as usize - 1;
            if super::super::upk_package::strip(&package.class_of(&package.exports[index])) != *kind {
                return Err(format!("Live colour dependency {file}:{path} changed type; installation left the game untouched"));
            }
        }
        if file == "Core.upk" {
            for (path, expected) in [("Object.EqualEqual_ObjectObject", 0xcf_u16), ("Object.NotEqual_ObjectObject", 0xce),
                ("Object.EqualEqual_StrStr", 0xcc), ("Object.NotEqual_StrStr", 0xc0), ("Object.Less_IntInt", 0x96)] {
                let index = package.find_object(path).filter(|index| *index > 0).ok_or_else(|| format!("Missing native operator {path}"))? as usize - 1;
                let export = &package.exports[index];
                let bytes = &package.image[export.serial_offset..export.serial_offset + export.serial_size];
                let code = Script::of_function(bytes)?;
                let at = 48 + code.len();
                let actual = bytes.get(at..at + 2).ok_or("Truncated native operator metadata")?;
                if u16::from_le_bytes(actual.try_into().unwrap()) != expected {
                    return Err(format!("Native operator {path} changed; live cosmetic installation left the game untouched"));
                }
            }
        }
    }
    Ok(())
}
thread_local! {
    static LOOP_SLOTS: std::cell::RefCell<std::collections::HashMap<String, usize>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static OBJECTS: std::cell::RefCell<std::collections::HashMap<String, i32>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static OWNER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
pub(super) fn raw(bytes: &[u8]) -> Script { Script::from_parts(bytes.to_vec(), 0) }
fn obj(p: &UpkPackage, path: &str) -> R<i32> {
    OWNER.with(|owner| { let identity = p as *const UpkPackage as usize; if owner.replace(identity) != identity { OBJECTS.with(|c| c.borrow_mut().clear()); } });
    if let Some(index) = OBJECTS.with(|c| c.borrow().get(&path.to_ascii_lowercase()).copied()) { return Ok(index); }
    let index = p.find_object(path).ok_or_else(|| format!("Live cosmetics requires {path}"))?;
    OBJECTS.with(|c| c.borrow_mut().insert(path.to_ascii_lowercase(), index));
    Ok(index)
}
pub(super) fn var(p: &UpkPackage, opcode: u8, path: &str) -> R<Script> { let mut s = raw(&[opcode]); s.object(obj(p, path)?); Ok(s) }
pub(super) fn text(value: &str) -> Script { let mut s = raw(&[0x1f]); s.raw(value.as_bytes()).raw(&[0]); s }
fn path_text(value: &std::path::Path) -> Script {
    let mut script = raw(&[0x34]);
    for character in value.to_string_lossy().encode_utf16().chain(std::iter::once(0)) { script.raw(&character.to_le_bytes()); }
    script
}
pub(super) fn int(value: i32) -> Script { let mut s = raw(&[0x1d]); s.raw(&value.to_le_bytes()); s }
fn float(value: f32) -> Script { let mut s = raw(&[0x1e]); s.raw(&value.to_le_bytes()); s }
pub(super) fn native(op: u8, args: &[Script]) -> Script { let mut s = raw(&[op]); for a in args { s.append(a); } s.raw(&[0x16]); s }
// Core metadata: EqualEqual_ObjectObject = 0xcf; 0xcd is GotoState.
pub(super) fn object_eq(args: &[Script]) -> Script { native(0xcf, args) }
pub(super) fn cast(op: u8, value: Script) -> Script { let mut s = raw(&[0x38, op]); s.append(&value); s }
pub(super) fn call(p: &UpkPackage, function: &str, args: &[Script]) -> R<Script> {
    let mut s = var(p, 0x1c, function)?; for a in args { s.append(a); } s.raw(&[0x16]); Ok(s)
}
pub(super) fn virtual_call(p: &UpkPackage, function: &str, args: &[Script]) -> R<Script> {
    let name = p.names.iter().position(|n| n == function).ok_or_else(|| format!("Missing runtime function name {function}"))?;
    let mut s = raw(&[0x1b]); s.name(name as i32); for a in args { s.append(a); } s.raw(&[0x16]); Ok(s)
}
pub(super) fn context(p: &UpkPackage, target: Script, body: Script, property: Option<&str>) -> R<Script> {
    let mut s = raw(&[0x19, 0]); s.append(&target).raw(&u16::try_from(body.mem_size()).map_err(|_| "Context overflow")?.to_le_bytes());
    s.object(property.map(|path| obj(p, path)).transpose()?.unwrap_or(0)).raw(&[0]).append(&body); Ok(s)
}
pub(super) fn field(p: &UpkPackage, target: Script, path: &str) -> R<Script> { context(p, target, var(p, 0x01, path)?, Some(path)) }
pub(super) fn static_call(p: &UpkPackage, class: &str, body: Script, property: Option<&str>) -> R<Script> {
    let mut s = raw(&[0x5e, 0x12, 0, 0x20]); s.object(obj(p, class)?).raw(&u16::try_from(body.mem_size()).map_err(|_| "Static context overflow")?.to_le_bytes());
    s.object(property.map(|path| obj(p, path)).transpose()?.unwrap_or(0)).raw(&[0]).append(&body); Ok(s)
}
pub(super) fn dynamic(p: &UpkPackage, class: &str, target: Script) -> R<Script> { let mut s = var(p, 0x2e, class)?; s.append(&target); Ok(s) }
pub(super) fn assign(left: Script, right: Script) -> Script { let mut s = raw(&[0x0f]); s.append(&left).append(&right); s }
fn set_bool(p: &UpkPackage, target: Script, property: &str, enabled: bool) -> R<Script> {
    let mut bool_var = raw(&[0x2d]); bool_var.append(&var(p, 0x01, property)?);
    let mut s = raw(&[0x14]); s.append(&context(p, target, bool_var, Some(property))?).raw(&[if enabled { 0x27 } else { 0x28 }]); Ok(s)
}
pub(super) fn ret(value: Script) -> Script { let mut s = raw(&[0x04]); s.append(&value); s }
pub(super) fn guard(base: usize, condition: Script, body: Script) -> R<Script> {
    let end = u16::try_from(base + 3 + condition.mem_size() + body.mem_size()).map_err(|_| format!("Runtime branch overflow at {base} (condition {}, body {})", condition.mem_size(), body.mem_size()))?;
    let mut s = raw(&[0x07]); s.raw(&end.to_le_bytes()).append(&condition).append(&body); Ok(s)
}
fn member(p: &UpkPackage, structure: &str, name: &str, target: Script, write: bool) -> R<Script> {
    let mut s = raw(&[0x35]); s.object(obj(p, &format!("{structure}.{name}"))?).object(obj(p, structure)?).raw(&[0, u8::from(write)]).append(&target); Ok(s)
}
pub(super) fn element(index: i32, array: Script) -> Script { let mut s = raw(&[0x57, 0, 0]); s.append(&int(index)).append(&array); s }
fn saved(p: &UpkPackage) -> R<Script> {
    let engine = static_call(p, "Engine.Engine", call(p, "Engine.Engine.GetEngine", &[])?, Some("Engine.Engine.GetEngine.ReturnValue"))?;
    let player = dynamic(p, "LocalPlayer_TA", element(0, field(p, engine, "Engine.Engine.GamePlayers")?))?;
    let manager = field(p, player, "LocalPlayer_TA.SaveGameManager")?;
    let provider = field(p, manager, "SaveGameManager_TA.SaveObjectProvider")?;
    dynamic(p, "UISavedValues_TA", context(p, provider, call(p, "Core.ObjectProvider.Get", &[var(p, 0x20, "UISavedValues_TA")?])?, Some("Core.ObjectProvider.Get.ReturnValue"))?)
}
fn settings_ready(p: &UpkPackage) -> R<Script> {
    let engine = static_call(p, "Engine.Engine", call(p, "Engine.Engine.GetEngine", &[])?, Some("Engine.Engine.GetEngine.ReturnValue"))?;
    let players = field(p, engine, "Engine.Engine.GamePlayers")?;
    let mut count = raw(&[0x57, 1]); count.append(&players);
    let player = dynamic(p, "LocalPlayer_TA", element(0, players))?;
    let manager = field(p, player.clone(), "LocalPlayer_TA.SaveGameManager")?;
    let provider = field(p, manager.clone(), "SaveGameManager_TA.SaveObjectProvider")?;
    let valid = items::choose(native(0xce, &[provider, raw(&[0x2a])]), native(0xce, &[saved(p)?, raw(&[0x2a])]), raw(&[0x28]))?;
    let valid = items::choose(native(0xce, &[manager, raw(&[0x2a])]), valid, raw(&[0x28]))?;
    let valid = items::choose(native(0xce, &[player, raw(&[0x2a])]), valid, raw(&[0x28]))?;
    items::choose(native(0x97, &[count, int(0)]), valid, raw(&[0x28]))
}
pub(super) fn get_self(p: &UpkPackage, key: Script) -> R<Script> { call(p, "UISavedValues_TA.Get", &[cast(0x60, key)]) }
pub(super) fn set_self(p: &UpkPackage, key: &str, value: Script) -> R<Script> { call(p, "UISavedValues_TA.Set", &[cast(0x60, text(key)), value]) }
pub(super) fn get(p: &UpkPackage, key: &str) -> R<Script> { context(p, saved(p)?, get_self(p, text(key))?, Some("UISavedValues_TA.Get.ReturnValue")) }
fn get_dynamic(p: &UpkPackage, key: Script) -> R<Script> { context(p, saved(p)?, get_self(p, key)?, Some("UISavedValues_TA.Get.ReturnValue")) }
fn set_dynamic(p: &UpkPackage, key: Script, value: Script) -> R<Script> {
    context(p, saved(p)?, call(p, "UISavedValues_TA.Set", &[cast(0x60, key), value])?, None)
}
pub(super) fn set(p: &UpkPackage, key: &str, value: Script) -> R<Script> { context(p, saved(p)?, set_self(p, key, value)?, None) }
pub fn flag(p: &UpkPackage, key: &str) -> R<Script> { Ok(native(0xcc, &[get(p, &format!("HbxLive.{key}"))?, text("1")])) }
fn value(p: &UpkPackage, key: &str, min: f32, max: f32) -> R<Script> {
    virtual_call(p, "FClamp", &[cast(STRING_TO_FLOAT, get(p, &format!("HbxLive.{key}"))?), float(min), float(max)])
}
fn append_original(p: &UpkPackage, path: &str, build: impl FnOnce(usize) -> R<Script>) -> R<Vec<u8>> {
    let i = obj(p, path)? as usize - 1; let e = &p.exports[i]; let old = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let mut s = Script::of_function(old)?;
    if !s.bytes().ends_with(&[0x04, 0x0b, 0x4c]) { return Err(format!("Unsupported function ending: {path}")); }
    let body = build(s.mem_size() - 3).map_err(|error| format!("{path}: {error}"))?;
    validate(&body, s.mem_size() - 3)?;
    s.insert_before_tail(3, &body); function_payload(old, &s)
}
fn store_cache(p: &UpkPackage, output: &Script) -> R<Script> {
    let condition = native(0xc0, &[output.clone(), get_self(p, text("HbxLiveCache"))?]);
    let yes = text("1"); let no = text("0");
    let mut changed = raw(&[0x45]);
    changed.append(&condition).raw(&(yes.mem_size() as u16).to_le_bytes()).append(&yes)
        .raw(&(no.mem_size() as u16).to_le_bytes()).append(&no);
    let mut code = set_self(p, "HbxLiveChanged", changed)?;
    code.append(&set_self(p, "HbxLiveCache", output.clone())?);
    Ok(code)
}

fn getter(p: &UpkPackage, settings_path: &std::path::Path, control_path: &std::path::Path) -> R<Vec<u8>> {
    let path = TARGETS[0]; let i = obj(p, path)? as usize - 1; let e = &p.exports[i]; let old = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let mut s = Script::of_function(old)?;
    // Preserve the stock array lookup, its return and all original jump offsets.
    let n = s.len();
    if n < 10 || s.bytes()[n - 10..n - 7] != [0x04, 0x1f, 0] { return Err("Saved-value getter is already modified or unsupported".into()); }
    s.cut(n - 10, 10, 1);
    let (start, start_refs, start_memory) = (s.len(), s.refs(), s.mem_size());
    let key = cast(0x57, var(p, 0x2b, "UISavedValues_TA.Get.Key")?);
    // Bootstrap transient state without persisting a saved-value key. Otherwise
    // the original lookup would bypass this branch on the next game session.
    let init = native(0xcc, &[key.clone(), text("HbxControl.init")]);
    let at = s.mem_size();
    let mut initialize = items::ensure_holder(p, at + 3 + init.mem_size())?;
    initialize.append(&ret(text("")));
    s.append(&guard(at, init, initialize)?);
    let wheels = native(0xcc, &[key.clone(), text("HbxWheel.refresh")]);
    let at = s.mem_size();
    let mut update = refresh_wheel_colours(p, at + 3 + wheels.mem_size())?;
    update.append(&ret(text("")));
    s.append(&guard(at, wheels, update)?);
    let output = var(p, 0x2b, "UISavedValues_TA.Get.ReturnValue")?;
    let index = var(p, 0x2b, "UISavedValues_TA.Get.Idx")?;
    let mut refresh = assign(output.clone(), text(""));
    let loader = virtual_call(p, "LoadFileToString", &[path_text(settings_path), output.clone()])?;
    refresh.append(&static_call(p, "Core.FileSystem", loader, None)?);
    refresh.append(&assign(output.clone(), native(0xc9, &[output.clone(), text("\r"), text(""), raw(&[0x48])])));
    let valid_header = native(0xcc, &[virtual_call(p, "Left", &[output.clone(), int(19)])?, text("\nHbxLive.version=1\n")]);
    // Invalid/missing/truncated files replace the cache with empty defaults.
    let mut invalid = assign(output.clone(), text(""));
    invalid.append(&store_cache(p, &output)?).append(&ret(text("")));
    refresh.append(&guard(s.mem_size() + 3 + native(0xcc, &[key.clone(), text("HbxLive.refresh")]).mem_size() + refresh.mem_size(), native(0x81, &[valid_header]), invalid)?);
    let mut invalidate = assign(output.clone(), text(""));
    invalidate.append(&store_cache(p, &output)?).append(&ret(text("")));
    let refresh_base = s.mem_size() + 3 + native(0xcc, &[key.clone(), text("HbxLive.refresh")]).mem_size();
    let footer = virtual_call(p, "InStr", &[output.clone(), text("\nHbxLive.end=1\n")])?;
    refresh.append(&guard(refresh_base + refresh.mem_size(), native(0x9a, &[footer, int(-1)]), invalidate.clone())?);
    refresh.append(&guard(refresh_base + refresh.mem_size(), native(0x81, &[native(0x96, &[native(0xef, &[output.clone()]), int(8193)])]), invalidate)?);
    refresh.append(&store_cache(p, &output)?).append(&ret(text("")));
    s.append(&guard(s.mem_size(), native(0xcc, &[key.clone(), text("HbxLive.refresh")]), refresh)?);
    // A fixed, one-shot quit request lets the game exit itself when Windows
    // denies TerminateProcess. The file cannot supply an arbitrary command.
    let request_key = native(0xcc, &[key.clone(), text("HbxControl.quit")]);
    let at = s.mem_size();
    let start_control = at + 3 + request_key.mem_size();
    let mut control = assign(output.clone(), text(""));
    control.append(&static_call(p, "Core.FileSystem", virtual_call(p, "LoadFileToString", &[path_text(control_path), output.clone()])?, None)?);
    let session = items::control_session(p)?;
    let request_prefix = native(0xf8, &[native(0xf8, &[text("quit:"), session.clone()]), text(":")]);
    let prefix_ok = items::choose(native(0xc0, &[session, text("")]),
        native(0xcc, &[virtual_call(p, "Left", &[output.clone(), native(0xef, &[request_prefix.clone()])])?, request_prefix]), raw(&[0x28]))?;
    let short = native(0x96, &[native(0xef, &[output.clone()]), int(81)]);
    let fresh = native(0xc0, &[output.clone(), get_self(p, text("HbxQuitLast"))?]);
    let valid = items::choose(prefix_ok, items::choose(short, fresh, raw(&[0x28]))?, raw(&[0x28]))?;
    let mut accept = set_self(p, "HbxQuitLast", output.clone())?;
    accept.append(&ret(text("1")));
    control.append(&guard(start_control + control.mem_size(), valid, accept)?).append(&ret(text("0")));
    s.append(&guard(at, request_key, control)?);
    let prefix = native(0xcc, &[virtual_call(p, "Left", &[key.clone(), int(8)])?, text("HbxLive.")]);
    let mut parse = assign(output.clone(), get_self(p, text("HbxLiveCache"))?);
    let needle = native(0xf8, &[native(0xf8, &[text("\n"), key]), text("=")]);
    parse.append(&assign(index.clone(), virtual_call(p, "InStr", &[output.clone(), needle.clone()])?));
    let body_base = s.mem_size() + 3 + prefix.mem_size();
    parse.append(&guard(body_base + parse.mem_size(), native(0x9a, &[index.clone(), int(-1)]), ret(text("0")))?);
    parse.append(&assign(output.clone(), virtual_call(p, "Mid", &[output.clone(), native(0x92, &[index, native(0xef, &[needle])]), int(32)])?));
    parse.append(&assign(output.clone(), virtual_call(p, "Left", &[output.clone(), virtual_call(p, "InStr", &[output.clone(), text("\n")])?])?));
    let key_name = cast(0x57, var(p, 0x2b, "UISavedValues_TA.Get.Key")?);
    let binding = native(0xcc, &[virtual_call(p, "Left", &[key_name, int(13)])?, text("HbxLive.bind.")]);
    parse.append(&guard(body_base + parse.mem_size(), binding, ret(output.clone()))?);
    // Accept only a bounded decimal representation, so NaN/Inf and arbitrary
    // strings can never reach rotation, translation or material parameters.
    let index = var(p, 0x2b, "UISavedValues_TA.Get.Idx")?;
    parse.append(&assign(index.clone(), int(0)));
    let loop_start = body_base + parse.mem_size();
    let ch = virtual_call(p, "Mid", &[output.clone(), index.clone(), int(1)])?;
    let invalid = native(0x9a, &[virtual_call(p, "InStr", &[text("0123456789.-"), ch])?, int(-1)]);
    let condition = native(0x96, &[index.clone(), native(0xef, &[output.clone()])]);
    let mut loop_body = guard(loop_start + 3 + condition.mem_size(), invalid, ret(text("0")))?;
    loop_body.append(&assign(index.clone(), native(0x92, &[index, int(1)]))).raw(&[0x06]).raw(&u16::try_from(loop_start).map_err(|_| "Parse loop overflow")?.to_le_bytes());
    parse.append(&guard(loop_start, condition, loop_body)?).append(&ret(output.clone()));
    s.append(&guard(s.mem_size(), prefix, parse)?);
    s.append(&ret(text(""))).raw(&[0x04, 0x3a]).object(obj(p, "UISavedValues_TA.Get.ReturnValue")?).raw(&[0x4c]);
    validate(&Script::from_parts(s.bytes()[start..].to_vec(), s.refs() - start_refs), start_memory)?;
    function_payload(old, &s)
}

fn validate(script: &Script, base: usize) -> R<()> { checked_jumps(script, base).map(|_| ()) }
fn checked_jumps(script: &Script, base: usize) -> R<Vec<(usize, usize)>> {
    struct Check<'a> { b: &'a [u8], at: usize, refs: usize, base: usize, branches: Vec<(usize, usize)>, starts: Vec<usize> }
    impl Check<'_> {
        fn take(&mut self, n: usize) -> R<&[u8]> {
            let end = self.at.checked_add(n).ok_or("Bytecode overflow")?;
            let bytes = self.b.get(self.at..end).ok_or("Truncated live bytecode")?;
            self.at = end; Ok(bytes)
        }
        fn reference(&mut self) -> R<()> { self.take(4)?; self.refs += 1; Ok(()) }
        fn position(&self) -> usize { self.base + self.at + 4 * self.refs }
        fn expr(&mut self, depth: usize) -> R<()> {
            if depth > 96 { return Err("Live bytecode nesting limit exceeded".into()); }
            let at = self.at;
            match self.take(1)?[0] {
                0x01 | 0x20 | 0x2b | 0x3a | 0x46 => self.reference()?,
                0x04 | 0x5e => self.expr(depth + 1)?,
                0x2d => {
                    // BoolVariable's operand is a variable selector, not a
                    // general expression. A Context here makes the VM treat
                    // bytecode bytes as a UBoolProperty pointer and crash.
                    if !matches!(self.b.get(self.at), Some(0x01 | 0x2b | 0x46)) {
                        return Err(format!("Invalid boolean variable operand at {at}"));
                    }
                    self.expr(depth + 1)?;
                },
                0x11 => { for _ in 0..5 { self.expr(depth + 1)?; } },
                0x1a => { self.expr(depth + 1)?; self.expr(depth + 1)?; },
                0x0f | 0x14 => { self.expr(depth + 1)?; self.expr(depth + 1)?; },
                0x0e => { self.reference()?; self.expr(depth + 1)?; },
                0x06 => { let offset = self.at; let target = u16::from_le_bytes(self.take(2)?.try_into().unwrap()); self.branches.push((offset, target as usize)); },
                0x07 => { let offset = self.at; let target = u16::from_le_bytes(self.take(2)?.try_into().unwrap()); self.branches.push((offset, target as usize)); self.expr(depth + 1)?; },
                0x12 | 0x19 => {
                    self.take(1)?; self.expr(depth + 1)?;
                    let skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                    self.reference()?; self.take(1)?;
                    let start = self.position(); self.expr(depth + 1)?;
                    if self.position() - start != skip { return Err(format!("Live context skip mismatch at {at}")); }
                },
                0x1b => { self.take(8)?; self.params(depth + 1)?; },
                0x1c => { self.reference()?; self.params(depth + 1)?; },
                0x1d | 0x1e => { self.take(4)?; },
                0x21 => { self.take(8)?; },
                0x1f => while self.take(1)?[0] != 0 {},
                0x34 => while self.take(2)? != [0, 0] {},
                0x24 | 0x2c => { self.take(1)?; },
                0x2e => { self.reference()?; self.expr(depth + 1)?; },
                0x35 => {
                    self.reference()?; self.reference()?;
                    let flags = self.take(2)?;
                    if flags[0] == 0 && matches!(self.b.get(self.at), Some(0x1b | 0x1c | 0x45)) {
                        return Err(format!("Struct temporary requires a result buffer at {at}"));
                    }
                    self.expr(depth + 1)?;
                },
                0x38 => { self.take(1)?; self.expr(depth + 1)?; },
                0x45 => {
                    self.expr(depth + 1)?;
                    for _ in 0..2 {
                        let skip = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
                        let start = self.position(); self.expr(depth + 1)?;
                        if self.position() - start != skip { return Err("Conditional memory size mismatch".into()); }
                    }
                },
                0x57 => {
                    match self.take(1)?[0] {
                        0 => { self.take(1)?; self.expr(depth + 1)?; self.expr(depth + 1)?; },
                        1 => self.expr(depth + 1)?,
                        _ => return Err("Unexpected array expression".into()),
                    }
                },
                0x0b | 0x17 | 0x25..=0x28 | 0x2a | 0x48 | 0x4c => {},
                0xcd => return Err("GotoState is not an object comparison; refusing unsafe cosmetic bytecode".into()),
                0x80..=0xff => self.params(depth + 1)?,
                op => return Err(format!("Unsupported live bytecode {op:02x} at {at}")),
            }
            Ok(())
        }
        fn params(&mut self, depth: usize) -> R<()> {
            while self.b.get(self.at) != Some(&0x16) { self.expr(depth + 1)?; }
            self.take(1)?; Ok(())
        }
    }
    let mut check = Check { b: script.bytes(), at: 0, refs: 0, base, branches: vec![], starts: vec![] };
    while check.at < check.b.len() { check.starts.push(check.position()); check.expr(0)?; }
    check.starts.push(check.position());
    if check.refs != script.refs() { return Err("Live bytecode reference count mismatch".into()); }
    if let Some((_, target)) = check.branches.iter().find(|(_, target)| !check.starts.contains(target)) { return Err(format!("Live branch target {target} is not an instruction boundary")); }
    Ok(check.branches)
}
/// Insert a private fast path without invalidating the original absolute jumps.
fn prefix_original(p: &UpkPackage, path: &str, prefix: Script) -> R<Vec<u8>> {
    let export = &p.exports[obj(p, path)? as usize - 1];
    let old = &p.image[export.serial_offset..export.serial_offset + export.serial_size];
    let script = prepend_checked(Script::of_function(old)?, prefix)?;
    function_payload(old, &script)
}
fn prepend_checked(mut original: Script, prefix: Script) -> R<Script> {
    for (offset, target) in checked_jumps(&original, 0)? {
        original.set_word(offset, u16::try_from(target + prefix.mem_size()).map_err(|_| "Prefixed native branch overflow")?);
    }
    let mut script = prefix;
    script.append(&original);
    validate(&script, 0)?;
    Ok(script)
}
fn wheel(p: &UpkPackage, base: usize) -> R<Script> {
    let local = raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone();
    let condition = items::choose(settings_ready(p)?, local, raw(&[0x28]))?;
    let mut body = Script::new();
    for i in 0..4 {
        let array = element(i, var(p, 0x01, "CarMeshComponent_TA.WheelControls")?);
        let roll = member(p, "CarMeshComponent_TA.WheelSkelControlSet", "RollControl", array.clone(), false)?;
        let translation = member(p, "CarMeshComponent_TA.WheelSkelControlSet", "TranslationControl", array, false)?;
        let sign = if i % 2 == 0 { 1. } else { -1. };
        let mut rotate = set_bool(p, translation.clone(), "Engine.SkelControlSingleBone.bApplyRotation", true)?;
        rotate.append(&set_bool(p, translation.clone(), "Engine.SkelControlSingleBone.bAddRotation", true)?);
        rotate.append(&assign(field(p, translation.clone(), "Engine.SkelControlSingleBone.BoneRotationSpace")?, raw(&[0x24, 2])));
        let rotation = field(p, translation.clone(), "Engine.SkelControlSingleBone.BoneRotation")?;
        rotate.append(&assign(member(p, "Core.Object.Rotator", "Roll", rotation, true)?, cast(0x44, native(0xab, &[value(p, "camber", -30., 30.)?, float(sign * 65536. / 360.)]))));
        let mut move_wheel = assign(field(p, roll.clone(), "Engine.SkelControlSingleBone.BoneTranslationSpace")?, raw(&[0x24, 2]));
        move_wheel.append(&set_bool(p, roll.clone(), "Engine.SkelControlSingleBone.bApplyTranslation", true)?);
        move_wheel.append(&set_bool(p, roll.clone(), "Engine.SkelControlSingleBone.bAddTranslation", true)?);
        let pos = field(p, roll.clone(), "Engine.SkelControlSingleBone.BoneTranslation")?;
        move_wheel.append(&assign(member(p, "Core.Object.Vector", "Y", pos.clone(), true)?, native(0xab, &[value(p, "lateral", -15., 15.)?, float(sign)])));
        move_wheel.append(&assign(member(p, "Core.Object.Vector", "Z", pos, true)?, value(p, "vertical", -15., 15.)?));
        let mut count = raw(&[0x57, 1]); count.append(&var(p, 0x01, "CarMeshComponent_TA.WheelControls")?);
        let present = native(0x97, &[count, int(i)]);
        let at = base + 3 + condition.mem_size() + body.mem_size();
        let inner = at + 3 + present.mem_size();
        let mut update = guard(inner, native(0xce, &[translation, raw(&[0x2a])]), rotate)?;
        update.append(&guard(inner + update.mem_size(), native(0xce, &[roll, raw(&[0x2a])]), move_wheel)?);
        body.append(&guard(at, present, update)?);
    }
    guard(base, condition, body)
}
fn materials(p: &UpkPackage, base: usize) -> R<Script> {
    let local = items::choose(settings_ready(p)?, raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone(), raw(&[0x28]))?;
    let mut body = Script::new();
    for (key, color, full) in [("primary", "TeamColor", "TeamColorFull"), ("secondary", "CustomColor", "CustomColorFull"), ("trim", "TrimColor", "TrimColorFull"), ("wheels", "CustomColor", "CustomColorFull")] {
        let enabled = flag(p, &format!("{key}.enabled"))?;
        let wheel_pass = native(0xcc, &[get(p, "HbxWheelColourPass")?, text("1")]);
        let pass = if key == "wheels" { wheel_pass } else { native(0x81, &[wheel_pass]) };
        let condition = enabled;
        let rgb = paint_rgb(p, key)?;
        let apply = call(p, "CarMeshComponentBase_TA.SetMaterialColorParams", &[
            var(p, 0x2b, "CarMeshComponentBase_TA.SetMaterialColors.MatInst")?, rgb,
            cast(0x60, text(color)), cast(0x60, text(full)),
        ])?;
        let start = base + 3 + local.mem_size() + body.mem_size();
        let apply = guard(start + 3 + condition.mem_size(), pass, apply)?;
        body.append(&guard(start, condition, apply)?);
    }
    // Restore the real material callback used by the working pre-FPS hooks.
    // refresh_mesh owns the per-car 10 Hz clock: subsequent material callbacks
    // return before scanning or applying any registered paints.
    body.append(&items::refresh_mesh(p, base + 3 + local.mem_size() + body.mem_size())?);
    // Wheels/trim also have a direct material path, independent of inventory
    // paint capture. Report that path after refresh_mesh resets its counters.
    for key in ["trim", "wheels"] {
        let wheel_pass = native(0xcc, &[get(p, "HbxWheelColourPass")?, text("1")]);
        let pass = if key == "wheels" { wheel_pass } else { native(0x81, &[wheel_pass]) };
        let material = var(p, 0x2b, "CarMeshComponentBase_TA.SetMaterialColors.MatInst")?;
        let active = items::choose(flag(p, &format!("{key}.enabled"))?,
            items::choose(pass, native(0xce, &[material, raw(&[0x2a])]), raw(&[0x28]))?, raw(&[0x28]))?;
        let mut report = set(p, &format!("HbxNativeFound.{key}"), text("1"))?;
        report.append(&set(p, &format!("HbxNativeApplied.{key}"), text("1"))?);
        body.append(&guard(base + 3 + local.mem_size() + body.mem_size(), active, report)?);
    }
    guard(base, local, body)
}
fn rgb(p: &UpkPackage, key: &str) -> R<Script> {
    call(p, "Core.Object.MakeLinearColor", &[value(p, &format!("{key}.r"), 0., 1.)?, value(p, &format!("{key}.g"), 0., 1.)?, value(p, &format!("{key}.b"), 0., 1.)?, float(1.)])
}
// Keep channel expressions scalar: projecting a member from a temporary struct
// without a VM temporary buffer makes native MakeLinearColor write to null.
fn paint_channel(p: &UpkPackage, key: &str, index: usize) -> R<Script> {
    let time = cast(STRING_TO_FLOAT, get(p, "HbxRainbowTime")?);
    let phase = [0., -std::f32::consts::TAU / 3., std::f32::consts::TAU / 3.][index];
    let rainbow = native(0xae, &[float(0.5), native(0xab, &[float(0.5), native(0xbb, &[native(0xae, &[native(0xab, &[time, float(std::f32::consts::TAU / 6.)]), float(phase)])])])]);
    let fixed = value(p, &format!("{key}.{}", ["r", "g", "b"][index]), 0., 1.)?;
    let mut result = raw(&[0x45]);
    result.append(&flag(p, &format!("{key}.rainbow"))?)
        .raw(&u16::try_from(rainbow.mem_size()).map_err(|_| "Rainbow expression overflow")?.to_le_bytes()).append(&rainbow)
        .raw(&u16::try_from(fixed.mem_size()).map_err(|_| "Colour expression overflow")?.to_le_bytes()).append(&fixed);
    Ok(result)
}
fn paint_rgb(p: &UpkPackage, key: &str) -> R<Script> {
    call(p, "Core.Object.MakeLinearColor", &[paint_channel(p, key, 0)?, paint_channel(p, key, 1)?, paint_channel(p, key, 2)?, float(1.)])
}
pub(super) fn preview_actor(p: &UpkPackage) -> R<Script> {
    preview_actor_for(p, raw(&[0x17]))
}
pub(super) fn preview_actor_for(p: &UpkPackage, controller: Script) -> R<Script> {
    // Same owner path used by Shift: it works on the main menu as well as in
    // the garage, without relying on a garage data page having been opened.
    let game = dynamic(p, "GameInfo_GFxMenu_TA", field(p, field(p, controller.clone(), "Engine.Actor.WorldInfo")?, "Engine.WorldInfo.Game")?)?;
    let previews = field(p, game, "GameInfo_GFxMenu_TA.CarPreviewSet")?;
    let player = dynamic(p, "Engine.LocalPlayer", field(p, controller, "Engine.PlayerController.Player")?)?;
    context(p, previews, call(p, "CarPreviewSet_TA.GetPlayerCarPreviewActor", &[player])?, Some("CarPreviewSet_TA.GetPlayerCarPreviewActor.ReturnValue"))
}
fn heatseeker_getter(p: &UpkPackage, path: &str, rainbow_only: bool) -> R<Vec<u8>> {
    let e = &p.exports[obj(p, path)? as usize - 1];
    let before = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let original = Script::of_function(before)?;
    // These vanilla getters each have exactly one absolute jump. Relocate it
    // explicitly; retain every stock return path when the option is disabled.
    let (size, memory, jump) = if rainbow_only { (133, 205, 57u16) } else { (112, 180, 159u16) };
    let marker = [0x07, jump as u8, (jump >> 8) as u8];
    let positions: Vec<_> = original.bytes().windows(3).enumerate().filter_map(|(i, b)| (b == marker).then_some(i)).collect();
    if original.len() != size || original.mem_size() != memory || positions.len() != 1 {
        return Err(format!("Heatseeker getter changed: {path}. Refusing to overwrite it"));
    }
    let mut script = guard(0, flag(p, "heatseeker.enabled")?, ret(paint_rgb(p, "heatseeker")?))?;
    if rainbow_only {
        let inner = guard(3 + flag(p, "heatseeker.enabled")?.mem_size(), flag(p, "heatseeker.rainbow")?, ret(paint_rgb(p, "heatseeker")?))?;
        script = guard(0, flag(p, "heatseeker.enabled")?, inner)?;
    }
    validate(&script, 0)?;
    let mut bytes = original.bytes().to_vec();
    let destination = u16::try_from(jump as usize + script.mem_size()).map_err(|_| "Heatseeker jump overflow")?;
    bytes[positions[0] + 1..positions[0] + 3].copy_from_slice(&destination.to_le_bytes());
    script.append(&Script::from_parts(bytes, original.refs()));
    function_payload(before, &script)
}
fn team_colours(p: &UpkPackage, base: usize, owner: &str) -> R<Script> {
    let outer_base = base;
    let ready = settings_ready(p)?;
    let base = base + 3 + ready.mem_size();
    let mut body = Script::new();
    for (group, local) in [("stadium", "NewColors"), ("hud", "NewFontColor")] {
        let out = format!("{owner}.{local}");
        let enabled = flag(p, &format!("{group}.enabled"))?;
        let outer_base = base + body.mem_size();
        let mut teams = Script::new();
        for (index, team) in [(0, "blue"), (1, "orange")] {
            let matches = native(0x9a, &[var(p, 0x01, "Engine.TeamInfo.TeamIndex")?, int(index)]);
            let color = rgb(p, &format!("{group}.{team}"))?;
            let output = var(p, 0x2b, &out)?;
            let mut apply = Script::new();
            if group == "stadium" {
                let mut length = raw(&[0x57, 0x01]); length.append(&output);
                apply.append(&assign(length, int(3)));
                for row in 0..3 { apply.append(&assign(element(row, output.clone()), color.clone())); }
            } else { apply.append(&assign(output, color)); }
            teams.append(&guard(outer_base + 3 + enabled.mem_size() + teams.mem_size(), matches, apply)?);
        }
        body.append(&guard(outer_base, enabled, teams)?);
    }
    guard(outer_base, ready, body)
}
fn update_team_colours(p: &UpkPackage) -> R<Vec<u8>> {
    let path = "Team_TA.UpdateColors";
    let e = &p.exports[obj(p, path)? as usize - 1];
    let before = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let original = Script::of_function(before)?;
    // Vanilla begins GetColors(NewColors, NewFontColor). Override those local
    // results before the existing setters, delegates and Kismet notification.
    let bytes = original.bytes();
    // Exact vanilla bytecode captured from the supported game package. Checking
    // the complete body avoids ambiguity in name-table and object lookup aliases.
    const STOCK: &[u8] = &[0x1b, 0x29, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2b, 0xb3, 0x62, 0x00, 0x00, 0x2b, 0xb2, 0x62, 0x00, 0x00, 0x16, 0x0f, 0x01, 0x24, 0x62, 0x00, 0x00, 0x2b, 0xb3, 0x62, 0x00, 0x00, 0x1c, 0x9e, 0x62, 0x00, 0x00, 0x2b, 0xb2, 0x62, 0x00, 0x00, 0x48, 0x16, 0xe7, 0xf8, 0xf8, 0xa8, 0xf8, 0xf8, 0xa8, 0xa8, 0x38, 0x56, 0x17, 0x38, 0x57, 0x1c, 0xe3, 0xf8, 0xff, 0xff, 0x16, 0x16, 0x1f, 0x53, 0x74, 0x72, 0x69, 0x6e, 0x67, 0x28, 0x4e, 0x65, 0x77, 0x43, 0x6f, 0x6c, 0x6f, 0x72, 0x73, 0x29, 0x3d, 0x28, 0x00, 0x16, 0x38, 0x65, 0x2b, 0xb2, 0x62, 0x00, 0x00, 0x16, 0x1f, 0x29, 0x00, 0x16, 0x1f, 0x46, 0x6f, 0x6e, 0x74, 0x43, 0x6f, 0x6c, 0x6f, 0x72, 0x3d, 0x28, 0x00, 0x16, 0x38, 0x65, 0x01, 0x24, 0x62, 0x00, 0x00, 0x16, 0x1f, 0x29, 0x00, 0x16, 0x21, 0x9f, 0x2b, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x16, 0x42, 0x00, 0x0a, 0x62, 0x00, 0x00, 0xae, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17, 0x16, 0x1c, 0xb6, 0x62, 0x00, 0x00, 0x16, 0x1c, 0xd8, 0x62, 0x00, 0x00, 0x01, 0x70, 0xf2, 0xff, 0xff, 0x2b, 0xb2, 0x62, 0x00, 0x00, 0x16, 0x1c, 0x96, 0x7e, 0x01, 0x00, 0x16, 0x04, 0x0b, 0x4c];
    if original.mem_size() != 241 || bytes != STOCK {
        return Err(format!("Team colour update differs from the supported stock function ({} bytes, {} memory). Recovery retained the journal", original.len(), original.mem_size()));
    }
    // Virtual call: opcode + FName (8), two local object references (5 each),
    // and end-parameters. Keep the original call and its original name index.
    let offset = 20;
    let refs = 2;
    let mut lookup = Script::from_parts(bytes[..offset].to_vec(), refs);
    let overrides = team_colours(p, lookup.mem_size(), path)?;
    validate(&overrides, lookup.mem_size())?;
    lookup.append(&overrides);
    // The stock remainder has no absolute branches; all original notifications
    // and material updates remain in their original order.
    lookup.append(&Script::from_parts(original.bytes()[offset..].to_vec(), original.refs() - refs));
    function_payload(before, &lookup)
}
fn default_team_colours(p: &UpkPackage) -> R<Vec<u8>> {
    let path = "Team_TA.SetDefaultColors";
    let e = &p.exports[obj(p, path)? as usize - 1];
    let before = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let original = Script::of_function(before)?;
    // The menu updates team archetypes through SetDefaultColors rather than
    // UpdateColors. Preserve its stock lookup, texture and shader updates.
    const STOCK: &[u8] = &[0x1b,0x29,0x53,0,0,0,0,0,0,0x5b,0x0b,0x2b,0xa1,0x62,0,0,0x5b,0x0b,0x2b,0xa0,0x62,0,0,0x16,0x1c,0x9e,0x62,0,0,0x2b,0xa0,0x62,0,0,0x48,0x16,0x1c,0xd8,0x62,0,0,0x01,0x70,0xf2,0xff,0xff,0x2b,0xa0,0x62,0,0,0x16,0x04,0x0b,0x4c];
    if original.bytes() != STOCK || original.mem_size() != 83 {
        return Err("Menu team colour function changed. Refusing to overwrite it".into());
    }
    let mut script = Script::from_parts(original.bytes()[..24].to_vec(), 2);
    let overrides = team_colours(p, script.mem_size(), path)?;
    validate(&overrides, script.mem_size())?;
    script.append(&overrides);
    script.append(&Script::from_parts(original.bytes()[24..].to_vec(), original.refs() - 2));
    function_payload(before, &script)
}
fn shell_bridge(p: &UpkPackage) -> R<Vec<u8>> {
    let path = TARGETS[6];
    let e = &p.exports[obj(p, path)? as usize - 1];
    let before = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
    let original = Script::of_function(before)?;
    if original.len() != 75 || original.refs() != 11 { return Err("Shell key lookup changed. Live bridge left it untouched".into()); }
    let action = cast(0x57, var(p, 0x2b, "GFxShell_TA.GetKeyForAction.Action")?);
    let mut script = Script::new();
    script.append(&guard(0, native(0xcc, &[virtual_call(p, "Left", &[action.clone(), int(8)])?, text("HbxLive.")]), ret(get_dynamic(p, action.clone())?))?);
    let instant = native(0xcc, &[action, text("GetSCT")]);
    let base = script.mem_size() + 3 + instant.mem_size();
    let mut body = guard(base, native(0x81, &[flag(p, "mods.instant-suite")?]), ret(text("0")))?;
    let actor = field(p, var(p, 0x01, "ProjectX.GFxShell_X.Player")?, "Engine.Player.Actor")?;
    let event = context(p, dynamic(p, "PlayerController_TA", actor)?, call(p, "PlayerController_TA.GetGameEvent", &[])?, Some("PlayerController_TA.GetGameEvent.ReturnValue"))?;
    let playlist = context(p, event, call(p, "GameEvent_TA.GetPlaylist", &[])?, Some("GameEvent_TA.GetPlaylist.ReturnValue"))?;
    let playlist_id = field(p, playlist, "ProjectX.GameSettingPlaylist_X.PlaylistId")?;
    for (id, setting) in [(0, "private"), (6, "private"), (22, "tournaments"), (34, "tournaments")] {
        let matches = native(0x9a, &[playlist_id.clone(), int(id)]);
        let reject = guard(base + body.mem_size() + 3 + matches.mem_size(), native(0x81, &[flag(p, &format!("instant.{setting}"))?]), ret(text("0")))?;
        body.append(&guard(base + body.mem_size(), matches, reject)?);
    }
    for mode in 1..=4 {
        let matches = native(0xcc, &[get(p, "HbxLive.instant.mode")?, text(&mode.to_string())]);
        body.append(&guard(base + body.mem_size(), matches, ret(text(&format!("9999999999999{mode}"))))?);
    }
    body.append(&ret(text("0")));
    script.append(&guard(script.mem_size(), instant, body)?);
    validate(&script, 0)?;
    script.append(&original);
    function_payload(before, &script)
}
fn tick(p: &UpkPackage, base: usize, menu: bool, report: &std::path::Path) -> R<Script> {
    let ready = settings_ready(p)?;
    let body = tick_ready(p, base + 3 + ready.mem_size(), menu, report)?;
    guard(base, ready, body)
}
fn tick_ready(p: &UpkPackage, base: usize, menu: bool, report: &std::path::Path) -> R<Script> {
    let time = field(p, var(p, 0x01, "Engine.Actor.WorldInfo")?, "Engine.WorldInfo.RealTimeSeconds")?;
    // Shared across menu/game and local players, ten file reads a second.
    let next = cast(STRING_TO_FLOAT, get(p, "HbxLiveNextRead")?);
    let reset = context(p, saved(p)?, set_self(p, "HbxLiveNextRead", text("0"))?, None)?;
    let mut body = guard(base, native(0xb0, &[time.clone(), native(0xaf, &[next.clone(), float(1.)])]), reset)?;
    let condition = native(0xb1, &[time.clone(), next]);
    let mut poll = set_dynamic(p, text("HbxRainbowTime"), cast(FLOAT_TO_STRING, time.clone()))?;
    poll.append(&context(p, saved(p)?, set_self(p, "HbxLiveNextRead", cast(FLOAT_TO_STRING, native(0xae, &[time, float(0.1)])))?, None)?);
    let mut refresh = raw(&[0x0e]); refresh.object(obj(p, "UISavedValues_TA.Get.ReturnValue")?).append(&get(p, "HbxLive.refresh")?);
    poll.append(&refresh);
    let quit = native(0xcc, &[get(p, "HbxControl.quit")?, text("1")]);
    let mut exit = raw(&[0x0e]);
    exit.object(obj(p, "Engine.Actor.ConsoleCommand.ReturnValue")?).append(&virtual_call(p, "ConsoleCommand", &[text("quit"), raw(&[0x28])])?);
    poll.append(&guard(base + body.mem_size() + 3 + condition.mem_size() + poll.mem_size(), quit, exit)?);
    let poll_base = base + body.mem_size() + 3 + condition.mem_size();
    if menu {
        // Refresh once per file change or menu-world change, including disable
        // and malformed-file fallback. Do not disturb match team instances.
        let world = var(p, 0x01, "Engine.Actor.WorldInfo")?;
        let signature = native(0xf8, &[cast(0x56, world), get(p, "HbxLiveCache")?]);
        let changed = native(0xc0, &[get(p, "HbxMenuTeamColoursApplied")?, signature.clone()]);
        let mut update = static_call(p, "GameData_TA", call(p, "GameData_TA.SetDefaultTeamColors", &[])?, None)?;
        update.append(&set_dynamic(p, text("HbxMenuTeamColoursApplied"), signature)?);
        poll.append(&guard(poll_base + poll.mem_size(), changed, update)?);
    } else {
        let event = dynamic(p, "GameEvent_Team_TA", call(p, "PlayerController_TA.GetGameEvent", &[])?)?;
        let teams = field(p, event.clone(), "GameEvent_Team_TA.Teams")?;
        let available = native(0xce, &[event.clone(), raw(&[0x2a])]);
        let signature = native(0xf8, &[cast(0x56, event), get(p, "HbxLiveCache")?]);
        let changed = native(0xc0, &[get(p, "HbxTeamColoursApplied")?, signature.clone()]);
        let mut count = raw(&[0x57, 0x01]); count.append(&teams);
        let ready = items::choose(available, native(0x96, &[int(1), count]), raw(&[0x28]))?;
        let outer_base = poll_base + poll.mem_size();
        let mut update = Script::new();
        for i in 0..2 {
            let team = dynamic(p, "Team_TA", element(i, teams.clone()))?;
            update.append(&context(p, team, call(p, "Team_TA.UpdateColors", &[])?, None)?);
        }
        update.append(&set_dynamic(p, text("HbxTeamColoursApplied"), signature)?);
        let changed_body = guard(outer_base + 3 + ready.mem_size(), changed, update)?;
        poll.append(&guard(outer_base, ready, changed_body)?);
    }
    if !menu {
        let event = dynamic(p, "GameEvent_Soccar_TA", call(p, "PlayerController_TA.GetGameEvent", &[])?)?;
        let available = native(0xce, &[event.clone(), raw(&[0x2a])]);
        let balls = field(p, event, "GameEvent_Soccar_TA.GameBalls")?;
        let mut count = raw(&[0x57, 0x01]); count.append(&balls);
        let ball = dynamic(p, "Ball_God_TA", element(0, balls))?;
        // Refresh even after disabling, so the original colour is restored.
        let ready = items::choose(available, native(0x96, &[int(0), count]), raw(&[0x28]))?;
        let ready = items::choose(ready, native(0xce, &[ball.clone(), raw(&[0x2a])]), raw(&[0x28]))?;
        poll.append(&guard(poll_base + poll.mem_size(), ready,
            context(p, ball, call(p, "Ball_God_TA.UpdateColor", &[])?, None)?)?);
    }
    poll.append(&super::game_mods::bindings(p, poll_base + poll.mem_size())?);
    body.append(&guard(base + body.mem_size(), condition, poll)?);
    // A shared file poll must not prevent other local controllers from updating
    // their own car. Each controller gets an independent cosmetic refresh clock.
    let render_key = native(0xf8, &[text("HbxRenderNext."), cast(0x56, raw(&[0x17]))]);
    let render_time = field(p, var(p, 0x01, "Engine.Actor.WorldInfo")?, "Engine.WorldInfo.RealTimeSeconds")?;
    let render_next = cast(STRING_TO_FLOAT, get_dynamic(p, render_key.clone())?);
    body.append(&guard(base + body.mem_size(), native(0xb0, &[render_time.clone(), native(0xaf, &[render_next.clone(), float(1.)])]), set_dynamic(p, render_key.clone(), text("0"))?)?);
    let render_condition = native(0xb1, &[render_time.clone(), render_next]);
    let mut render = set_dynamic(p, render_key, cast(FLOAT_TO_STRING, native(0xae, &[render_time, float(0.1)])))?;
    let mesh = local_mesh(p, menu)?;
    // Garage preview meshes derive from the base mesh class. Casting them to
    // the in-match subclass suppressed the entire live render pass in menus.
    let mesh = dynamic(p, "CarMeshComponentBase_TA", mesh)?;
    let render_condition = items::choose(native(0xce, &[mesh.clone(), raw(&[0x2a])]), render_condition, raw(&[0x28]))?;
    let boost_base = base + body.mem_size() + 3 + render_condition.mem_size() + render.mem_size();
    render.append(&boost::refresh(p, boost_base, mesh.clone(), menu)?);
    let alignment_key = native(0xf8, &[text("HbxAlignmentRefresh."), cast(0x56, mesh.clone())]);
    let mut alignment_signature = get(p, "HbxLoadoutGeneration")?;
    for key in ["camber", "vertical", "lateral"] {
        alignment_signature = native(0xf8, &[native(0xf8, &[alignment_signature, text("|")]), get(p, &format!("HbxLive.{key}"))?]);
    }
    let changed = native(0xc0, &[alignment_signature.clone(), get_dynamic(p, alignment_key.clone())?]);
    let mut alignment = context(p, dynamic(p, "CarMeshComponent_TA", mesh.clone())?, call(p, "CarMeshComponent_TA.InitWheelControls", &[])?, None)?;
    alignment.append(&set_dynamic(p, alignment_key, alignment_signature)?);
    render.append(&guard(base + body.mem_size() + 3 + render_condition.mem_size() + render.mem_size(), changed, alignment)?);
    // SetMeshMaterialColors takes the mesh to update, not the component itself.
    render.append(&context(p, mesh.clone(), call(p, "CarMeshComponentBase_TA.SetMeshMaterialColors", &[mesh])?, None)?);
    // Wheel meshes are separate components. Refresh each one's materials using
    // the stock material iterator, then apply its independent paint override.
    // The marker is scoped to synchronous calls and always cleared afterwards.
    render.append(&set(p, "HbxWheelMesh", call(p, "Core.Object.PathName", &[local_mesh(p, menu)?])?)?);
    let mut refresh_wheels = raw(&[0x0e]);
    refresh_wheels.object(obj(p, "UISavedValues_TA.Get.ReturnValue")?).append(&get(p, "HbxWheel.refresh")?);
    render.append(&refresh_wheels);
    // Factory paint is applied after the parameter fallback. When disabled,
    // this final pass restores the real equipped variant rather than a cached
    // fallback value that may have been captured after another paint pass.
    let item_base = base + body.mem_size() + 3 + render_condition.mem_size() + render.mem_size();
    render.append(&items::refresh_vanity(p, item_base)?);
    body.append(&guard(base + body.mem_size(), render_condition, render)?);
    let time = field(p, var(p, 0x01, "Engine.Actor.WorldInfo")?, "Engine.WorldInfo.RealTimeSeconds")?;
    body.append(&items::diagnostics(p, base + body.mem_size(), time, path_text(report))?);
    Ok(body)
}
// Keep wheel iteration in its own dispatch branch. Both controller tick
// functions have native 16-bit branch addresses and must remain below 64K.
fn refresh_wheel_colours(p: &UpkPackage, base: usize) -> R<Script> {
    let car_mesh = dynamic(p, "CarMeshComponentBase_TA", call(p, "Core.Object.FindObject", &[
        get(p, "HbxWheelMesh")?, var(p, 0x20, "CarMeshComponentBase_TA")?])?)?;
    let ready = native(0xce, &[car_mesh.clone(), raw(&[0x2a])]);
    let body = refresh_wheel_colours_ready(p, base + 3 + ready.mem_size(), car_mesh)?;
    guard(base, ready, body)
}
fn refresh_wheel_colours_ready(p: &UpkPackage, base: usize, car_mesh: Script) -> R<Script> {
    let mut out = Script::new();
    let wheels = field(p, car_mesh.clone(), "CarMeshComponentBase_TA.WheelMeshes")?;
    let wheel_key = native(0xf8, &[text("HbxWheelMaterials."), cast(0x56, car_mesh.clone())]);
    let active_key = native(0xf8, &[wheel_key.clone(), text(".active")]);
    let was_active = native(0xcc, &[get_dynamic(p, active_key.clone())?, text("1")]);
    let mut stamp = native(0xf8, &[items::vanity_stamp(p, "wheels")?, get(p, "HbxLoadoutGeneration")?]);
    // Native loadout changes replace the wheel set together. Track the first
    // component plus count as well as generation, including delayed startup.
    let mut count = raw(&[0x57, 1]); count.append(&wheels);
    let first = items::choose(native(0x97, &[count.clone(), int(0)]), cast(0x56, element(0, wheels.clone())), text(""))?;
    stamp = native(0xf8, &[native(0xf8, &[stamp, cast(INT_TO_STRING, count)]), first]);
    // Evaluate the preview/component expression once instead of duplicating
    // it in both the comparison and commit (UnrealScript has a 64K limit).
    out.append(&set(p, "HbxWheelPendingStamp", stamp)?);
    let stamp = get(p, "HbxWheelPendingStamp")?;
    let changed = native(0xc0, &[stamp.clone(), get_dynamic(p, wheel_key.clone())?]);
    let needed = items::choose(flag(p, "wheels.enabled")?, changed, was_active)?;
    let wheel_base = base + out.mem_size();
    let mut update = set_dynamic(p, text("HbxWheelColourPass"), text("1"))?;
    for i in 0..4 {
        let mut count = raw(&[0x57, 1]); count.append(&wheels);
        let condition = native(0x96, &[int(i), count]);
        let apply = context(p, car_mesh.clone(), call(p, "CarMeshComponentBase_TA.SetMeshMaterialColors", &[element(i, wheels.clone())])?, None)?;
        update.append(&guard(wheel_base + 3 + needed.mem_size() + update.mem_size(), condition, apply)?);
    }
    update.append(&set_dynamic(p, text("HbxWheelColourPass"), text("0"))?);
    update.append(&set_dynamic(p, wheel_key, stamp)?);
    update.append(&set_dynamic(p, active_key, items::choose(flag(p, "wheels.enabled")?, text("1"), text("0"))?)?);
    out.append(&guard(wheel_base, needed, update)?);
    Ok(out)
}
fn local_mesh(p: &UpkPackage, menu: bool) -> R<Script> {
    let owner = if menu { preview_actor(p)? } else { var(p, 0x01, "PlayerController_TA.Car")? };
    let field_name = if menu { "CarPreviewActor_TA.CarMesh" } else { "Vehicle_TA.CarMesh" };
    items::choose(native(0xce, &[owner.clone(), raw(&[0x2a])]), field(p, owner, field_name)?, raw(&[0x2a]))
}

pub fn generate(p: &UpkPackage) -> R<Vec<(String, Vec<u8>)>> {
    generate_with_paths(p, std::path::Path::new(r"..\..\TAGame\HbxLiveCosmetics.ini"), std::path::Path::new(r"..\..\TAGame\HbxLiveRuntime.ini"))
}
pub fn generate_with_paths(p: &UpkPackage, settings: &std::path::Path, report: &std::path::Path) -> R<Vec<(String, Vec<u8>)>> {
    LOOP_SLOTS.with(|slots| slots.borrow_mut().clear());
    OWNER.with(|owner| owner.set(p as *const UpkPackage as usize));
    OBJECTS.with(|c| {
        let mut objects = c.borrow_mut(); objects.clear();
        let import_count = p.import_debug().len();
        for index in (1..=p.exports.len() as i32).chain((1..=import_count as i32).map(|i| -i)) {
            objects.entry(p.asset_path(index).to_ascii_lowercase()).or_insert(index);
        }
    });
    let control = report.parent().ok_or("Missing live-control directory")?.join("quit.request");
    let mut out = vec![(TARGETS[0].to_string(), getter(p, settings, &control)?)];
    out.push((TARGETS[1].into(), append_original(p, TARGETS[1], |base| tick(p, base, false, report))?));
    out.push((TARGETS[2].into(), append_original(p, TARGETS[2], |base| tick(p, base, true, report))?));
    out.push((TARGETS[3].into(), append_original(p, TARGETS[3], |base| wheel(p, base))?));
    out.push((TARGETS[4].into(), append_original(p, TARGETS[4], |base| materials(p, base))?));
    out.push((TARGETS[5].into(), update_team_colours(p)?));
    out.push((TARGETS[6].into(), shell_bridge(p)?));
    out.push((TARGETS[7].into(), heatseeker_getter(p, TARGETS[7], false)?));
    out.push((TARGETS[8].into(), heatseeker_getter(p, TARGETS[8], true)?));
    let retired = &p.exports[obj(p, TARGETS[9])? as usize - 1];
    out.push((TARGETS[9].into(), p.image[retired.serial_offset..retired.serial_offset + retired.serial_size].to_vec()));
    out.push((TARGETS[10].into(), default_team_colours(p)?));
    out.push((TARGETS[15].into(), append_original(p, TARGETS[15], |base| items::capture(p, base))?));
    out.push((TARGETS[16].into(), append_original(p, TARGETS[16], |base| items::goal(p, base))?));
    out.push((TARGETS[17].into(), append_original(p, TARGETS[17], |base| items::attributes(p, base))?));
    out.push((TARGETS[18].into(), append_original(p, TARGETS[18], |base| items::goal_preview(p, base))?));
    out.push((TARGETS[19].into(), items::particle_override_getter(p)?));
    out.push((TARGETS[20].into(), prefix_original(p, TARGETS[20], items::reset_loadout(p, 0)?)?));
    out.push((TARGETS[13].into(), items::paint_route(p)?));
    out.push((TARGETS[14].into(), append_original(p, TARGETS[14], |base| items::vanity(p, base))?));
    out.push((TARGETS[12].into(), append_original(p, TARGETS[12], |base| boost::actor(p, base))?));
    out.push((TARGETS[11].into(), append_original(p, TARGETS[11], |base| {
        let actor = field(p, var(p, 0x2b, "Ball_TA.SetExplosionFXActor.InExplosion")?, "ProjectX.Explosion_X.FXActor")?;
        let scale = value(p, "goal-explosion.scale", 0.1, 10.)?;
        // With no override (including unreadable settings), retain the original actor scale.
        let apply = context(p, actor, call(p, "Engine.Actor.SetDrawScale", &[scale])?, None)?;
        guard(base, flag(p, "goal-explosion.enabled")?, apply)
    })?));
    Ok(out)
}

pub fn prepare_names(p: &mut UpkPackage) -> R<Vec<(usize, String, String)>> {
    let mut used = vec![false; p.names.len()];
    let wanted = ["GetVectorParameterValue", "SetVectorParameterValue", "VectorParameterValues", "ScalarParameterValues", "TextureParameterValues", "FontParameterValues", "GameEngine", "ObjectReferencers", "SaveStringToFile", "AttachmentActor", "ObjectReferencer", "ReferencedObjects", "IsPendingKill", "FindObject", "PathName", "ParticleSysParam", "ParamType", "InstanceParameters", "FXAttachment", "CreationTime", "ColorParams", "VectorParams", "ColorParamPair", "VectorParamPair", "Value", "Key", "Parameters", "SetLinearColorParameter", "SetVectorParameter", "Caps", "LoadFileToString", "InStr", "Left", "Mid", "FClamp", "SetBind", "GetBind", "bApplyRotation", "bAddRotation", "bApplyTranslation", "bAddTranslation", "BoneRotationSpace", "BoneTranslationSpace", "BoneRotation", "BoneTranslation"];
    for (i, name) in p.names.iter().enumerate() {
        if wanted.contains(&name.as_str()) || name.starts_with("Hbx") || name.starts_with("HBX")
            || ["TransformedLeftX", "SpecialEventData", "TargetFind", "ImportJSON"].contains(&name.as_str())
            || name.eq_ignore_ascii_case("LoadFileToString") { used[i] = true; }
    }
    for export in &p.exports {
        used[export.object_name.name_index] = true;
        let bytes = &p.image[export.serial_offset..export.serial_offset + export.serial_size];
        for window in bytes.windows(8) {
            let index = u32::from_le_bytes(window[..4].try_into().unwrap()) as usize;
            let instance = u32::from_le_bytes(window[4..].try_into().unwrap());
            if index < used.len() && (instance <= 1000 || instance == u32::MAX) { used[index] = true; }
        }
    }
    for (a, b, c) in p.import_debug() {
        for part in [&a, &b, &c].into_iter().flat_map(|s| s.split('.')) {
            if let Some(index) = p.names.iter().position(|n| n == super::super::upk_package::strip(part)) { used[index] = true; }
        }
    }
    let mut edits = Vec::new();
    for name in wanted {
        if p.names.iter().any(|s| s == name) { continue; }
        if name == "LoadFileToString" {
            if let Some(index) = p.names.iter().position(|s| s == "TransformedLeftX") {
                edits.push((index, p.names[index].clone(), name.to_string()));
                p.rename_name(index, name)?;
                used[index] = true;
                continue;
            }
        }
        let index = p.names.iter().enumerate().filter(|(i, s)| *i > 256 && !used[*i] && s.len() > 4 && s.bytes().all(|b| b.is_ascii_alphanumeric())
            && !["TransformedLeftX", "SpecialEventData", "TargetFind", "ImportJSON"].contains(&s.as_str()))
            .min_by_key(|(_, s)| s.len().abs_diff(name.len())).map(|(i, _)| i).ok_or_else(|| format!("No unused name slot for {name}"))?;
        let delta = name.len() as isize - p.names[index].len() as isize;
        let mut replacements = vec![(index, name.to_string())];
        if delta != 0 {
            let donor = p.names.iter().enumerate().find(|(i, s)| *i != index && *i > 256 && !used[*i]
                && s.bytes().all(|b| b.is_ascii_alphanumeric()) && s.len() as isize - delta >= 24)
                .map(|(i, _)| i).ok_or_else(|| format!("No name-size donor for {name}"))?;
            let size = (p.names[donor].len() as isize - delta) as usize;
            let replacement = format!("HbxLiveUnused{donor:0width$}", width = size - 13);
            replacements.push((donor, replacement));
            used[donor] = true;
        }
        // Inert reflection names are shared infrastructure. Leave them available
        // after removing our hooks, because other Hebnix patches may reuse them.
        for (i, new) in &replacements { edits.push((*i, p.names[*i].clone(), new.clone())); }
        p.rename_names_same_size(&replacements)?;
        used[index] = true;
    }
    Ok(edits)
}
pub fn prepare_imports(p: &mut UpkPackage) -> R<Vec<(usize, Vec<u8>, Vec<u8>)>> {
    let outer = p.find_object("Engine.SkelControlSingleBone").ok_or("Missing native wheel controller class")?;
    let mut edits = Vec::new();
    for (name, kind) in [("bApplyRotation", "BoolProperty"), ("bAddRotation", "BoolProperty"), ("bApplyTranslation", "BoolProperty"), ("bAddTranslation", "BoolProperty"), ("BoneRotationSpace", "ByteProperty"), ("BoneTranslationSpace", "ByteProperty"), ("BoneRotation", "StructProperty"), ("BoneTranslation", "StructProperty")] {
        if p.find_object(&format!("Engine.SkelControlSingleBone.{name}")).is_some() { continue; }
        let slot = p.empty_import_slots().into_iter().next().ok_or("No blank native wheel import slot")?;
        let (before, after) = p.bind_native_field(slot, outer, kind, name)?;
        edits.push((slot, before, after));
    }
    edits.extend(boost::prepare_imports(p)?);
    edits.extend(items::prepare_imports(p)?);
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_paths_preserve_unicode_in_native_string_constants() {
        let value = std::path::Path::new("C:\\Users\\测试\\Hebnix\\runtime.ini");
        let script = path_text(value);
        assert_eq!(script.bytes()[0], 0x34);
        let units: Vec<_> = script.bytes()[1..].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|v| *v != 0).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), value.to_string_lossy());
        validate(&script, 0).unwrap();
    }
    #[test]
    fn prefixing_a_native_function_rebases_memory_addresses_not_file_offsets() {
        let mut reference = raw(&[0x20]);
        reference.object(1);
        let condition = not_null_for_test(reference);
        let mut original = guard(0, condition, ret(int(10))).unwrap();
        original.append(&ret(int(20))).raw(&[0x4c]);
        let old_jumps = checked_jumps(&original, 0).unwrap();
        let prefix = guard(0, raw(&[0x28]), ret(int(30))).unwrap();
        let prefix_memory = prefix.mem_size();
        let prefix_bytes = prefix.len();
        let rebased = prepend_checked(original, prefix).unwrap();
        let jumps = checked_jumps(&rebased, 0).unwrap();
        for (offset, target) in old_jumps {
            assert!(jumps.contains(&(offset + prefix_bytes, target + prefix_memory)));
        }
        fn not_null_for_test(value: Script) -> Script { native(0xce, &[value, raw(&[0x2a])]) }
    }
    #[test]
    fn object_equality_cannot_dispatch_a_state_change() {
        let equality = object_eq(&[raw(&[0x2a]), raw(&[0x2a])]);
        assert_eq!(equality.bytes(), &[0xcf, 0x2a, 0x2a, 0x16]);
        validate(&equality, 0).unwrap();
        let unsafe_state_change = native(0xcd, &[raw(&[0x2a]), raw(&[0x2a])]);
        assert!(validate(&unsafe_state_change, 0).unwrap_err().contains("GotoState"));
    }
    #[test]
    fn struct_projection_rejects_native_returns_without_a_temporary_buffer() {
        let mut projection = raw(&[0x35]);
        projection.object(1).object(2).raw(&[0, 0]);
        let mut call = raw(&[0x1c]);
        call.object(3).raw(&[0x16]);
        let mut invalid = projection.clone();
        invalid.append(&call);
        assert!(validate(&invalid, 0).unwrap_err().contains("Struct temporary requires a result buffer"));
        let mut property = raw(&[0x01]);
        property.object(3);
        projection.append(&property);
        validate(&projection, 0).unwrap();
    }
    #[test]
    fn boolean_reads_require_a_property_selector_inside_the_context() {
        // The v11 encoding could pass structural validation but made the VM
        // interpret Context bytecode as a UBoolProperty pointer at startup.
        assert!(validate(&raw(&[0x2d, 0x19]), 0).unwrap_err().contains("Invalid boolean variable operand"));
        let mut value = raw(&[0x2d, 0x01]);
        value.object(1);
        validate(&value, 0).unwrap();
        let mut contextual = raw(&[0x19, 0, 0x17]);
        contextual.raw(&(value.mem_size() as u16).to_le_bytes()).object(1).raw(&[0]).append(&value);
        validate(&contextual, 0).unwrap();
    }
    #[test]
    #[ignore = "read-only cast audit against an explicitly supplied game package"]
    fn audit_game_cast_types() {
        let p = UpkPackage::load(std::path::Path::new(&std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap())).unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut integer_strings = 0;
        for e in &p.exports {
            if !p.class_of(e).starts_with("Function") { continue; }
            let data = &p.image[e.serial_offset..e.serial_offset + e.serial_size];
            let Ok(s) = Script::of_function(data) else { continue; };
            for w in s.bytes().windows(13) {
                if w[0] != 0x0f || w[1] != 0x2b || w[6] != 0x38 || ![0x2b, 0x46, 0x01].contains(&w[8]) { continue; }
                let lhs = i32::from_le_bytes(w[2..6].try_into().unwrap()); let rhs = i32::from_le_bytes(w[9..13].try_into().unwrap());
                if lhs <= 0 || rhs <= 0 || lhs as usize > p.exports.len() || rhs as usize > p.exports.len() { continue; }
                let a = p.class_of(&p.exports[lhs as usize - 1]); let b = p.class_of(&p.exports[rhs as usize - 1]);
                if a.starts_with("StrProperty") && b.starts_with("IntProperty") {
                    assert_eq!(w[7], INT_TO_STRING, "integer registry cast differs from stock {}", p.asset_path(e.table_index as i32 + 1));
                    integer_strings += 1;
                    seen.insert(w[7]);
                }
                if ((a.starts_with("FloatProperty") && b.starts_with("StrProperty")) || (a.starts_with("StrProperty") && b.starts_with("FloatProperty"))) && seen.insert(w[7]) {
                    println!("CAST {:02x} {} -> {} in {}", w[7], b, a, p.asset_path(e.table_index as i32 + 1));
                }
            }
        }
        assert!(!seen.is_empty());
        assert!(integer_strings > 0, "no stock integer-to-string conversion was audited");
    }
    #[test]
    #[ignore = "requires an explicitly supplied unmodified local TAGame package"]
    fn generate_against_local_package() {
        let path = std::env::var("HEBNIX_UPK_TEST_SOURCE").unwrap();
        verify_dependencies(std::path::Path::new(&path).parent().unwrap()).unwrap();
        let mut package = UpkPackage::load(std::path::Path::new(&path)).unwrap();
        for name in ["LoadFileToString", "InStr", "Left", "Mid", "FClamp"] { println!("NAME {name}: {:?}", package.names.iter().position(|s| s == name)); }
        for index in [0xb53c, 0xb540, 0x3143, 0x3144] { println!("MATERIAL NAME {index}: {:?}", package.names.get(index)); }
        prepare_names(&mut package).unwrap();
        println!("BLANK IMPORTS {}", package.empty_import_slots().len());
        prepare_imports(&mut package).unwrap();
        let boost_hook = boost::actor(&package, 0).unwrap();
        let mut invalid_owner = vec![0x01];
        invalid_owner.extend_from_slice(&package.find_object("FXActor_Boost_TA.BoostMesh").unwrap().to_le_bytes());
        assert!(!boost_hook.bytes().windows(invalid_owner.len()).any(|w| w == invalid_owner),
            "BoostMesh_TA derives from Object; it must not be read as an FXActor parameter dispenser");
        for script in [items::capture(&package, 0).unwrap(), items::refresh_mesh(&package, 0).unwrap()] {
            let mut singleton = vec![0x1c];
            singleton.extend_from_slice(&package.find_object("Core.Object.GetSingleton").unwrap().to_le_bytes());
            assert!(!script.bytes().windows(singleton.len()).any(|w| w == singleton), "paint registry must be constructed and rooted, not looked up as a registered singleton");
            let mut roots = vec![0x01];
            roots.extend_from_slice(&package.find_object("Engine.GameEngine.ObjectReferencers").unwrap().to_le_bytes());
            assert!(script.bytes().windows(roots.len()).any(|w| w == roots), "private paint registry must be retained by the engine");
            validate(&script, 0).unwrap();
        }
        let generated = generate(&package).unwrap();
        let scoped = items::references(&package).unwrap();
        assert!(scoped.bytes().windows(".HbxLivePaintRecords".len()).any(|w| w == b".HbxLivePaintRecords"));
        assert!(!scoped.bytes().windows(".HbxLiveColourReferences".len()).any(|w| w == b".HbxLiveColourReferences"),
            "hot paint loops must read only the current mesh's bucket");
        let control = items::control_session(&package).unwrap();
        assert!(control.bytes().windows(".HbxLiveColourReferences".len()).any(|w| w == b".HbxLiveColourReferences"),
            "the quit session must remain game-wide, independent of the current car bucket");
        // A missing save provider used to pin a loop counter at zero forever.
        // Loop-local writes must target transient float fields, never Set/Get.
        let counter = items::loop_counter(&package, "HbxRegressionLoop", true).unwrap();
        assert!(counter.bytes().windows(".HbxLiveColourReferences.HbxControlSession".len())
            .any(|w| w == b".HbxLiveColourReferences.HbxControlSession"));
        for forbidden in ["UISavedValues_TA.Set", "UISavedValues_TA.Get"] {
            let mut call = vec![0x1c];
            call.extend_from_slice(&package.find_object(forbidden).unwrap().to_le_bytes());
            assert!(!counter.bytes().windows(call.len()).any(|w| w == call), "loop counters must survive a missing save provider");
        }
        assert!(LOOP_SLOTS.with(|slots| slots.borrow().len()) <= 48);
        let route = &generated.iter().find(|(path, _)| path == "CarMeshComponentBase_TA.ApplyPaintSettingsToObject").unwrap().1;
        let mut unsafe_material_ctor = vec![0x20];
        unsafe_material_ctor.extend_from_slice(&package.find_object("Engine.MaterialInstanceConstant").unwrap().to_le_bytes());
        assert!(!route.windows(unsafe_material_ctor.len()).any(|w| w == unsafe_material_ctor),
            "renderer materials must be created by the engine factory, never script new/template cloning");
        let mut reparent = vec![0x1c];
        reparent.extend_from_slice(&package.find_object("Engine.MaterialInstanceConstant.SetParent").unwrap().to_le_bytes());
        assert!(!route.windows(reparent.len()).any(|w| w == reparent), "attachment recolouring must not reparent shaders or change their texture/static-switch inheritance");
        let material_path = "CarMeshComponentBase_TA.SetMaterialColors";
        let material_export = &package.exports[package.find_object(material_path).unwrap() as usize - 1];
        let stock_material = Script::of_function(&package.image[material_export.serial_offset..material_export.serial_offset + material_export.serial_size]).unwrap();
        let material = Script::of_function(&generated.iter().find(|(path, _)| path == material_path).unwrap().1).unwrap();
        assert!(material.bytes().starts_with(&stock_material.bytes()[..stock_material.len() - 3]),
            "retain the working stock material entry point, without a synthetic None-material dispatch");
        for (path, payload) in &generated {
            for removed in ["HbxRefreshSignature", "HbxPaintRefresh.", "HbxBoostRefresh.", "HbxAttachmentMaterialDomain"] {
                assert!(!payload.windows(removed.len()).any(|w| w == removed.as_bytes()), "{path} retained the post-FPS paint cache {removed}");
            }
        }
        // Export/property order is NOT UFunction argument order. Compare our
        // replay against the stock closure's native call convention.
        let replay = &generated.iter().find(|(path, _)| path == "CarMeshComponentBase_TA.SetMaterialColors").unwrap().1;
        let mut call_marker = vec![0x1c];
        call_marker.extend_from_slice(&package.find_object("CarMeshComponentBase_TA.ApplyPaintSettingsToObject").unwrap().to_le_bytes());
        let start = replay.windows(call_marker.len()).position(|w| w == call_marker).unwrap() + call_marker.len();
        let mut previous = 0;
        for name in ["ProductPaint", "PaintSettings", "ObjectToPaint", "Product"] {
            let mut property = vec![0x01];
            property.extend_from_slice(&package.find_object(&format!("__CarMeshComponentBase_TA__ApplyPaintToObject_0x1.{name}")).unwrap().to_le_bytes());
            let offset = replay[start..].windows(property.len()).position(|w| w == property).unwrap();
            assert!(offset >= previous, "native paint argument {name} is out of order");
            previous = offset + property.len();
        }
        for (function, required) in [
            ("CarMeshComponentBase_TA.ApplyPaintSettingsToObject", "HbxLivePaint_"),
            ("ProductAttribute_Painted_TA.ApplyPaintToObject", "HbxLiveCapture_"),
            ("PlayerVanity_TA.ApplyColor", "HbxVanityOriginal."),
            ("CarMeshComponentBase_TA.ApplyAttributes", "HbxCaptureHasPaint"),
            ("Ball_TA.InitExplosionFX", "HbxGoalHasPaint"),
            ("ExplosionPreviewer_TA.ApplyExplosionParameters", "HbxPreviewHasPaint"),
            ("CarMeshComponentBase_TA.ApplyPaintSettingsToObject", "HbxLiveOriginalFX"),
            ("CarMeshComponentBase_TA.ApplyPaintSettingsToObject", "HbxLiveOriginalMaterials"),
            ("ProductOverride_ParticleSystemColorParameter_TA.GetPaintToApply", "HbxItemOverridePass"),
            ("UISavedValues_TA.Get", "HbxControl.quit"),
            ("UISavedValues_TA.Get", "HbxQuitLast"),
            ("CarMeshComponent_TA.SetLoadout", "HbxLoadoutGeneration"),
            ("CarMeshComponentBase_TA.SetMaterialColors", "HbxChangedPaintLastApplied."),
            ("PlayerController_TA.PlayerTick", ".lastApplied"),
            ("PlayerController_Menu_TA.PlayerTick", ".lastApplied"),
            ("CarMeshComponentBase_TA.SetMaterialColors", "HbxNativeRefreshCount"),
            ("CarMeshComponentBase_TA.SetMaterialColors", "HbxNativePaintReplay"),
            ("CarMeshComponentBase_TA.SetMaterialColors", "HbxLivePaintRecords"),
            ("ProductAttribute_Painted_TA.ApplyPaintToObject", "HbxLivePaintRecords"),
            ("PlayerController_TA.PlayerTick", "HbxControl.quit"),
            ("PlayerController_Menu_TA.PlayerTick", "HbxControl.quit"),
        ] {
            let bytes = &generated.iter().find(|(path, _)| path == function).unwrap().1;
            assert!(bytes.windows(required.len()).any(|w| w == required.as_bytes()), "{function} missing private paint state");
        }
        for (function, required) in [
            ("Ball_TA.InitExplosionFX", "ProjectX.Explosion_X.FXActor"),
            ("Ball_TA.SetExplosionFXActor", "ProjectX.Explosion_X.FXActor"),
            ("ProductAttribute_Painted_TA.ApplyPaintToObject", "Engine.ObjectReferencer.ReferencedObjects"),
            ("ProductAttribute_Painted_TA.ApplyPaintToObject", "CarMeshComponentBase_TA.bLocalPlayer"),
            ("CarMeshComponentBase_TA.ApplyAttributes", "CarMeshComponentBase_TA.ApplyAttributes.Attributes"),
        ] {
            let bytes = &generated.iter().find(|(path, _)| path == function).unwrap().1;
            let mut reference = vec![if required == "CarMeshComponentBase_TA.ApplyAttributes.Attributes" { 0x2b } else { 0x01 }];
            reference.extend_from_slice(&package.find_object(required).unwrap().to_le_bytes());
            assert!(bytes.windows(reference.len()).any(|w| w == reference), "{function} must reach {required}");
        }
        // Active boost actors are owned separately from the generic mesh FX list.
        // This catches a refresh that assembles successfully but never reaches them.
        for (function, fields) in [
            ("PlayerController_TA.PlayerTick", &["Vehicle_TA.BoostComponent", "CarComponent_TA.FXActor"][..]),
            ("PlayerController_Menu_TA.PlayerTick", &["CarPreviewActor_TA.BoostFX", "GameInfo_GFxMenu_TA.CarPreviewSet"][..]),
        ] {
            let bytes = &generated.iter().find(|(path, _)| path == function).unwrap().1;
            let script = Script::of_function(bytes).unwrap();
            for field in fields {
                let mut reference = vec![0x01];
                reference.extend_from_slice(&package.find_object(field).unwrap().to_le_bytes());
                assert!(script.bytes().windows(reference.len()).any(|w| w == reference), "{function} must refresh {field}");
            }
        }
        let originals: Vec<_> = generated.iter().map(|(path, _)| {
            let index = package.find_object(path).unwrap() as usize - 1;
            let e = &package.exports[index];
            (index, package.image[e.serial_offset..e.serial_offset + e.serial_size].to_vec())
        }).collect();
        for ((path, bytes), (index, _)) in generated.iter().zip(&originals) {
            println!("GENERATED {path}: {}", bytes.len());
            package.replace_export_payload(*index, bytes).unwrap();
        }
        let staged = std::env::temp_dir().join(format!("hebnix-live-roundtrip-{}.upk", rand::random::<u64>()));
        std::fs::write(&staged, package.repacked_fast().unwrap()).unwrap();
        let mut loaded = UpkPackage::load(&staged).unwrap();
        for ((path, bytes), (index, before)) in generated.iter().zip(&originals) {
            let e = &loaded.exports[*index];
            assert_eq!(&loaded.image[e.serial_offset..e.serial_offset + e.serial_size], bytes, "{path}");
            loaded.replace_export_payload(*index, before).unwrap();
        }
        std::fs::write(&staged, loaded.repacked_fast().unwrap()).unwrap();
        let restored = UpkPackage::load(&staged).unwrap();
        for (index, before) in &originals {
            let e = &restored.exports[*index];
            assert_eq!(&restored.image[e.serial_offset..e.serial_offset + e.serial_size], before);
        }
        std::fs::remove_file(staged).unwrap();
    }
}
