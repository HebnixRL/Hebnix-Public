//! Live overrides for each local boost's existing particle/material parameters.
//! Keep originals per actor and parameter; never edit shared product paint assets.
use super::*;

fn count(array: Script) -> Script { let mut s = raw(&[0x57, 1]); s.append(&array); s }
fn at(index: Script, array: Script) -> Script {
    let mut s = raw(&[0x57, 0, 0]); s.append(&index).append(&array); s
}
fn join(a: Script, b: Script) -> Script { native(0xf8, &[a, b]) }
fn identity(p: &UpkPackage, target: Script) -> R<Script> {
    // Names can be reused after an FX actor is destroyed; include its spawn time.
    Ok(join(join(cast(0x56, target), text(".")), cast(FLOAT_TO_STRING,
        var(p, 0x01, "Engine.Actor.CreationTime")?)))
}

// Counters live in private engine-rooted data, independent of save providers.
// Absolute branch addresses use Script's in-memory sizes, including object refs.
fn each(p: &UpkPackage, base: usize, key: &str, array: Script,
    build: impl FnOnce(usize, Script) -> R<Script>) -> R<Script> {
    repeat(p, base, key, count(array.clone()), |base, index| build(base, at(index, array)))
}
fn each_owned(p: &UpkPackage, base: usize, key: &str, owner: Script, array: Script,
    build: impl FnOnce(usize, Script) -> R<Script>) -> R<Script> {
    let valid = native(0xce, &[owner, raw(&[0x2a])]);
    let body = each(p, base + 3 + valid.mem_size(), key, array, build)?;
    guard(base, valid, body)
}
pub(super) fn repeat(p: &UpkPackage, base: usize, key: &str, length: Script,
    build: impl FnOnce(usize, Script) -> R<Script>) -> R<Script> {
    // Save providers can disappear during startup/teardown. A saved counter
    // then reads zero forever. Bootstrap once and skip safely if unavailable.
    let missing = native(0xcf, &[super::items::loop_state(p)?, raw(&[0x2a])]);
    let mut init = raw(&[0x0e]);
    init.object(obj(p, "UISavedValues_TA.Get.ReturnValue")?).append(&get(p, "HbxControl.init")?);
    let mut s = guard(base, missing, init)?;
    let ready = native(0xce, &[super::items::loop_state(p)?, raw(&[0x2a])]);
    let at = base + s.mem_size();
    let mut loop_body = assign(super::items::loop_counter(p, key, true)?, float(0.));
    let start = at + 3 + ready.mem_size() + loop_body.mem_size();
    let index = cast(0x44, super::items::loop_counter(p, key, false)?);
    let condition = native(0x96, &[index.clone(), length]);
    let mut body = build(start + 3 + condition.mem_size(), index)?;
    body.append(&assign(super::items::loop_counter(p, key, true)?,
        native(0xae, &[super::items::loop_counter(p, key, false)?, float(1.)])));
    let mut back = raw(&[0x06]);
    back.raw(&u16::try_from(start).map_err(|_| "Boost loop address overflow")?.to_le_bytes());
    body.append(&back);
    loop_body.append(&guard(start, condition, body)?);
    s.append(&guard(at, ready, loop_body)?);
    Ok(s)
}

pub(super) fn refresh(p: &UpkPackage, base: usize, mesh: Script, menu: bool) -> R<Script> {
    let previous_key = join(text("HbxBoostMode."), cast(0x56, mesh.clone()));
    let active = super::items::choose(flag(p, "boost.enabled")?, raw(&[0x27]),
        native(0xcc, &[get_dynamic(p, previous_key)?, text("1")]))?;
    let key = join(text("HbxBoostAppliedState."), cast(0x56, mesh.clone()));
    let stamp = join(join(join(super::items::vanity_stamp(p, "boost")?, get(p, "HbxLoadoutGeneration")?), text("|")), get(p, "HbxBoostNativeGeneration")?);
    let changed = native(0xc0, &[stamp.clone(), get_dynamic(p, key.clone())?]);
    let needed = super::items::choose(active, changed, raw(&[0x28]))?;
    let valid = super::items::choose(native(0xce, &[mesh.clone(), raw(&[0x2a])]), needed, raw(&[0x28]))?;
    // Native events invalidate this stamp below. Solid boost colours do not
    // need a continuous scan of actor/dispenser/particle parameter arrays.
    let mut body = refresh_inner(p, base + 3 + valid.mem_size(), mesh, menu)?;
    body.append(&set_dynamic(p, key, stamp)?);
    guard(base, valid, body)
}
fn refresh_inner(p: &UpkPackage, base: usize, mesh: Script, menu: bool) -> R<Script> {
    let mut s = transition(p, base, mesh.clone(), menu)?;
    s.append(&set(p, "HbxBoostLocalPass", text("1"))?);
    // Shift's live update path uses the preview actor's effect in the garage,
    // and the boost component's effect in a match. The preview mesh's effect is
    // a separate auxiliary target and is only a fallback.
    let actor = if menu {
        let preview = preview_actor(p)?;
        let effect = field(p, preview, "CarPreviewActor_TA.BoostFX")?;
        let fallback = field(p, dynamic(p, "CarPreviewMeshComponent_TA", mesh.clone())?, "CarPreviewMeshComponent_TA.BoostFX")?;
        super::items::choose(native(0xce, &[effect.clone(), raw(&[0x2a])]), effect, fallback)?
    } else {
        let boost = field(p, var(p, 0x01, "PlayerController_TA.Car")?, "Vehicle_TA.BoostComponent")?;
        field(p, boost, "CarComponent_TA.FXActor")?
    };
    s.append(&context(p, dynamic(p, "FXActor_Boost_TA", actor)?,
        call(p, "FXActor_Boost_TA.HandleUpdatedPaintSettings", &[mesh.clone()])?, None)?);
    let actors = field(p, mesh.clone(), "CarMeshComponentBase_TA.FXActors")?;
    s.append(&each_owned(p, base + s.mem_size(), "HbxBoostActorIndex", mesh.clone(), actors, |_, actor| {
        let boost = dynamic(p, "FXActor_Boost_TA", actor)?;
        // Context safely skips None and FX actors of other kinds.
        context(p, boost, call(p, "FXActor_Boost_TA.HandleUpdatedPaintSettings", &[mesh])?, None)
    })?);
    s.append(&set(p, "HbxBoostLocalPass", text("0"))?);
    Ok(s)
}
fn transition(p: &UpkPackage, base: usize, mesh: Script, menu: bool) -> R<Script> {
    let key = join(text("HbxBoostMode."), cast(0x56, mesh));
    let was_enabled = native(0xcc, &[get_dynamic(p, key.clone())?, text("1")]);
    let enabled = flag(p, "boost.enabled")?;
    let changed = super::items::choose(enabled.clone(), native(0x81, &[was_enabled.clone()]), was_enabled)?;
    let start = base + 3 + changed.mem_size();
    let mut reset = Script::new();
    if menu {
        let preview = preview_actor(p)?;
        let effect = field(p, preview.clone(), "CarPreviewActor_TA.BoostFX")?;
        let present = native(0xce, &[effect, raw(&[0x2a])]);
        let disabled = native(0x81, &[enabled.clone()]);
        let mut rebuild = context(p, preview.clone(), call(p, "CarPreviewActor_TA.ClearBoostFX", &[])?, None)?;
        rebuild.append(&context(p, preview, call(p, "CarPreviewActor_TA.InitBoostFX", &[])?, None)?);
        reset.append(&guard(start, disabled.clone(), guard(start + 3 + disabled.mem_size(), present, rebuild)?)?);
    } else {
        let component = field(p, var(p, 0x01, "PlayerController_TA.Car")?, "Vehicle_TA.BoostComponent")?;
        reset.append(&context(p, component, call(p, "CarComponent_TA.ReInitFX", &[])?, None)?);
    }
    reset.append(&set_dynamic(p, key, super::items::choose(enabled, text("1"), text("0"))?)?);
    guard(base, changed, reset)
}

fn parameter(p: &UpkPackage, base: usize, pair: Script, vector: bool, dispenser: Script, particle: bool) -> R<Script> {
    let structure = if particle { "Engine.ParticleSystemComponent.ParticleSysParam" } else if vector { "ProjectX.ParameterDispenser_X.VectorParamPair" }
        else { "ProjectX.ParameterDispenser_X.ColorParamPair" };
    let key = member(p, structure, if particle { "Name" } else { "Key" }, pair.clone(), false)?;
    let value = member(p, structure, if particle { "Vector" } else { "Value" }, pair.clone(), false)?;
    let prefix = join(join(join(text(if particle { "HbxBoostOriginal.P." } else if vector { "HbxBoostOriginal.V." } else { "HbxBoostOriginal.C." }),
        identity(p, dispenser.clone())?), text(".")), cast(0x57, key.clone()));
    let mut leading = set(p, "HbxBoostParameterKey", prefix)?;
    let base = base + leading.mem_size();
    let prefix = get(p, "HbxBoostParameterKey")?;
    let marker = join(prefix.clone(), text(".saved"));
    let cached = native(0xcc, &[get_dynamic(p, marker.clone())?, text("1")]);
    let channels = if vector { ["X", "Y", "Z", ""] } else { ["R", "G", "B", ""] };
    let ty = if vector { "Core.Object.Vector" } else { "Core.Object.LinearColor" };
    let mut capture = Script::new();
    for channel in channels.iter().filter(|c| !c.is_empty()) {
        capture.append(&set_dynamic(p, join(prefix.clone(), text(&format!(".{channel}"))),
            cast(FLOAT_TO_STRING, member(p, ty, channel, value.clone(), false)?))?);
    }
    capture.append(&set_dynamic(p, marker.clone(), text("1"))?);
    let enabled = flag(p, "boost.enabled")?;
    let mut apply = guard(base + 3 + enabled.mem_size(), native(0x81, &[cached.clone()]), capture)?;
    for (channel, rgb_channel) in channels[..3].iter().zip(["R", "G", "B"]) {
        let destination = member(p, structure, if particle { "Vector" } else { "Value" }, pair.clone(), true)?;
        apply.append(&assign(member(p, ty, channel, destination, true)?,
            cast(STRING_TO_FLOAT, get(p, &format!("HbxBoostLive.{rgb_channel}"))?)));
    }
    let setter = if vector { "SetVectorParameter" } else { "SetLinearColorParameter" };
    let propagate = context(p, dispenser, virtual_call(p, setter, &[key, value])?, None)?;
    apply.append(&propagate);
    let mut s = guard(base, enabled.clone(), apply)?;
    let disabled = native(0x81, &[enabled]);
    let mut restore = Script::new();
    for channel in channels.iter().filter(|c| !c.is_empty()) {
        let destination = member(p, structure, if particle { "Vector" } else { "Value" }, pair.clone(), true)?;
        restore.append(&assign(member(p, ty, channel, destination, true)?,
            cast(STRING_TO_FLOAT, get_dynamic(p, join(prefix.clone(), text(&format!(".{channel}"))))?)));
    }
    restore.append(&propagate).append(&set_dynamic(p, marker, text("0"))?);
    let restore = guard(base + s.mem_size() + 3 + disabled.mem_size(), cached, restore)?;
    s.append(&guard(base + s.mem_size(), disabled, restore)?);
    leading.append(&s);
    Ok(leading)
}

pub(super) fn actor(p: &UpkPackage, base: usize) -> R<Script> {
    let local = native(0xcc, &[get(p, "HbxBoostLocalPass")?, text("1")]);
    let active_key = join(text("HbxBoostActive."), identity(p, raw(&[0x17]))?);
    let enabled = flag(p, "boost.enabled")?;
    let previous = native(0xcc, &[get_dynamic(p, active_key.clone())?, text("1")]);
    let mut active = raw(&[0x45]);
    active.append(&enabled).raw(&1u16.to_le_bytes()).raw(&[0x27])
        .raw(&(previous.mem_size() as u16).to_le_bytes()).append(&previous);
    let active_base = base + 3 + local.mem_size();
    let start = active_base + 3 + active.mem_size();
    let mut body = Script::new();
    let mut colour = Script::new();
    for (index, channel) in ["R", "G", "B"].into_iter().enumerate() {
        colour.append(&set(p, &format!("HbxBoostLive.{channel}"), cast(FLOAT_TO_STRING,
            paint_channel(p, "boost", index)?))?);
    }
    body.append(&guard(start, enabled.clone(), colour)?);
    // BoostMesh_TA is a Core.Object, not an FXActor_X. Its animated materials
    // are painted by BoostMesh_TA.ApplyPaint through the native paint route;
    // interpreting its memory as an FX parameter dispenser corrupts the VM.
    for vector in [false, true] {
        let dispenser = var(p, 0x01, "ProjectX.FXActor_X.Parameters")?;
        let array = field(p, dispenser.clone(), if vector { "ProjectX.ParameterDispenser_X.VectorParams" }
            else { "ProjectX.ParameterDispenser_X.ColorParams" })?;
        body.append(&each_owned(p, start + body.mem_size(), "HbxBoostParamIndex", dispenser.clone(), array, |base, pair| {
            if !vector { return parameter(p, base, pair, false, dispenser.clone(), false); }
            // Vector parameters also carry positions and velocities. Touch colour/tint only.
            let key = member(p, "ProjectX.ParameterDispenser_X.VectorParamPair", "Key", pair.clone(), false)?;
            let name = virtual_call(p, "Caps", &[cast(0x57, key)])?;
            let mut condition = raw(&[0x28]);
            for word in ["COLOR", "COLOUR", "TINT", "PAINT", "PRIMARY", "SECONDARY", "ACCENT"] {
                let found = native(0x9b, &[virtual_call(p, "InStr", &[name.clone(), text(word)])?, int(-1)]);
                let mut either = raw(&[0x45]);
                either.append(&found).raw(&1u16.to_le_bytes()).raw(&[0x27])
                    .raw(&u16::try_from(condition.mem_size()).map_err(|_| "Boost colour condition overflow")?.to_le_bytes()).append(&condition);
                condition = either;
            }
            let inner = parameter(p, base + 3 + condition.mem_size(), pair, true, dispenser.clone(), false)?;
            guard(base, condition, inner)
        })?);
    }
    // Several stock boosts keep colour vectors on the PSC rather than the dispenser.
    // Read only existing instance parameters so disabling never leaves a new override behind.
    let attachments = var(p, 0x01, "ProjectX.FXActor_X.Attachments")?;
    body.append(&each(p, start + body.mem_size(), "HbxBoostAttachmentIndex", attachments, |base, attachment| {
        let component = member(p, "ProjectX.FXActor_X.FXAttachment", "Component", attachment, false)?;
        let psc = dynamic(p, "Engine.ParticleSystemComponent", component)?;
        let array = field(p, psc.clone(), "Engine.ParticleSystemComponent.InstanceParameters")?;
        each_owned(p, base, "HbxBoostParamIndex", psc.clone(), array, |base, pair| {
            let kind = member(p, "Engine.ParticleSystemComponent.ParticleSysParam", "ParamType", pair.clone(), false)?;
            // PSPT_Vector = 3. Scalar, random-vector, actor and material parameters stay intact.
            let vector = native(0x9a, &[cast(0x3a, kind), int(3)]);
            let name = member(p, "Engine.ParticleSystemComponent.ParticleSysParam", "Name", pair.clone(), false)?;
            let name = virtual_call(p, "Caps", &[cast(0x57, name)])?;
            let mut named = raw(&[0x28]);
            for word in ["COLOR", "COLOUR", "TINT", "PAINT", "PRIMARY", "SECONDARY", "ACCENT"] {
                let found = native(0x9b, &[virtual_call(p, "InStr", &[name.clone(), text(word)])?, int(-1)]);
                let mut either = raw(&[0x45]);
                either.append(&found).raw(&1u16.to_le_bytes()).raw(&[0x27])
                    .raw(&u16::try_from(named.mem_size()).map_err(|_| "Boost particle condition overflow")?.to_le_bytes()).append(&named);
                named = either;
            }
            let named_base = base + 3 + vector.mem_size();
            let apply = parameter(p, named_base + 3 + named.mem_size(), pair, true, psc.clone(), true)?;
            guard(base, vector, guard(named_base, named, apply)?)
        })
    })?);
    let mut remember = raw(&[0x45]);
    remember.append(&enabled).raw(&3u16.to_le_bytes()).append(&text("1"))
        .raw(&3u16.to_le_bytes()).append(&text("0"));
    body.append(&set_dynamic(p, active_key, remember)?);
    let mut out = guard(base, local.clone(), guard(active_base, active, body)?)?;
    let generation = cast(0x44, cast(STRING_TO_FLOAT, get(p, "HbxBoostNativeGeneration")?));
    out.append(&guard(base + out.mem_size(), native(0x81, &[local]), set(p, "HbxBoostNativeGeneration",
        cast(INT_TO_STRING, native(0x92, &[generation, int(1)])))?)?);
    Ok(out)
}

pub(super) fn prepare_imports(p: &mut UpkPackage) -> R<Vec<(usize, Vec<u8>, Vec<u8>)>> {
    let mut edits = Vec::new();
    for (outer, name, kind) in [
        ("Engine.ParticleSystemComponent", "InstanceParameters", "ArrayProperty"),
        ("Engine.ParticleSystemComponent", "ParticleSysParam", "ScriptStruct"),
        ("Engine.ParticleSystemComponent.ParticleSysParam", "Name", "NameProperty"),
        ("Engine.ParticleSystemComponent.ParticleSysParam", "ParamType", "ByteProperty"),
        ("Engine.ParticleSystemComponent.ParticleSysParam", "Vector", "StructProperty"),
        ("ProjectX.FXActor_X", "Attachments", "ArrayProperty"),
        ("ProjectX.FXActor_X", "FXAttachment", "ScriptStruct"),
        ("ProjectX.FXActor_X.FXAttachment", "Component", "ComponentProperty"),
        ("Engine.Actor", "CreationTime", "FloatProperty"),
        ("Core.Object.LinearColor", "R", "FloatProperty"),
        ("Core.Object.LinearColor", "G", "FloatProperty"),
        ("Core.Object.LinearColor", "B", "FloatProperty"),
        ("Core.Object.LinearColor", "A", "FloatProperty"),
        ("Core.Object.Vector", "X", "FloatProperty"),
        ("ProjectX.FXActor_X", "Parameters", "ComponentProperty"),
        ("ProjectX.ParameterDispenser_X", "ColorParams", "ArrayProperty"),
        ("ProjectX.ParameterDispenser_X", "VectorParams", "ArrayProperty"),
        ("ProjectX.ParameterDispenser_X", "ColorParamPair", "ScriptStruct"),
        ("ProjectX.ParameterDispenser_X", "VectorParamPair", "ScriptStruct"),
        ("ProjectX.ParameterDispenser_X.ColorParamPair", "Key", "NameProperty"),
        ("ProjectX.ParameterDispenser_X.ColorParamPair", "Value", "StructProperty"),
        ("ProjectX.ParameterDispenser_X.VectorParamPair", "Key", "NameProperty"),
        ("ProjectX.ParameterDispenser_X.VectorParamPair", "Value", "StructProperty"),
    ] {
        if p.find_object(&format!("{outer}.{name}")).is_some() { continue; }
        let parent = p.find_object(outer).ok_or_else(|| format!("Missing boost parameter type {outer}"))?;
        let slot = p.empty_import_slots().into_iter().next().ok_or("No blank boost parameter import slot")?;
        let (before, after) = p.bind_native_field(slot, parent, kind, name)?;
        edits.push((slot, before, after));
    }
    Ok(edits)
}
