//! Equipped-item colours through Rocket League's native paint application path.
//! Each car owns private paint instances; shared catalog paints remain untouched.
use super::*;

pub(super) fn choose(condition: Script, yes: Script, no: Script) -> R<Script> {
    let mut code = raw(&[0x45]);
    code.append(&condition).raw(&u16::try_from(yes.mem_size()).map_err(|_| "Paint choice overflow")?.to_le_bytes()).append(&yes)
        .raw(&u16::try_from(no.mem_size()).map_err(|_| "Paint choice overflow")?.to_le_bytes()).append(&no);
    Ok(code)
}
fn join(a: Script, b: Script) -> Script { native(0xf8, &[a, b]) }
fn lookup(p: &UpkPackage, domain: Script, suffix: &str) -> R<Script> {
    get_dynamic(p, join(join(text("HbxLive."), domain), text(suffix)))
}
fn enabled(p: &UpkPackage, domain: Script) -> R<Script> {
    Ok(native(0xcc, &[lookup(p, domain, ".enabled")?, text("1")]))
}
fn colour_channel(p: &UpkPackage, domain: Script, index: usize) -> R<Script> {
    let time = cast(STRING_TO_FLOAT, get(p, "HbxRainbowTime")?);
    let phase = [0., -std::f32::consts::TAU / 3., std::f32::consts::TAU / 3.][index];
    let rainbow = native(0xae, &[float(0.5), native(0xab, &[float(0.5), native(0xbb, &[native(0xae, &[native(0xab, &[time, float(std::f32::consts::TAU / 6.)]), float(phase)])])])]);
    let fixed = virtual_call(p, "FClamp", &[cast(STRING_TO_FLOAT, lookup(p, domain.clone(), [".r", ".g", ".b"][index])?), float(0.), float(1.)])?;
    let allowed = choose(native(0xc0, &[domain.clone(), text("goal_explosion")]),
        native(0xcc, &[lookup(p, domain, ".rainbow")?, text("1")]), raw(&[0x28]))?;
    choose(allowed, rainbow, fixed)
}
pub(super) fn fixed_element(index: i32, array: Script) -> Script {
    // ProductPaint.Colors is a fixed array, not a dynamic array.
    let mut code = raw(&[0x1a]); code.append(&int(index)).append(&array); code
}

fn asset_domain(p: &UpkPackage, asset: Script) -> R<Script> {
    let product = context(p, asset, call(p, "ProductAsset_TA.GetProduct", &[])?, Some("ProductAsset_TA.GetProduct.ReturnValue"))?;
    let slot = field(p, product, "ProductTemplate_TA.Slot")?;
    let mut domain = text("");
    for (name, key) in [("Body", "trim"), ("Skin", "decal"), ("Wheels", "wheels"), ("Boost", "boost"), ("Antenna", "antenna"), ("Hat", "topper"), ("SupersonicTrail", "trail"), ("GoalExplosion", "goal_explosion")] {
        domain = choose(object_eq(&[slot.clone(), var(p, 0x20, &format!("ProductSlots.{name}"))?]), text(key), domain)?;
    }
    Ok(domain)
}
fn asset_settings(p: &UpkPackage, asset: Script) -> R<Script> {
    dynamic(p, "ProductAttribute_PaintSettings_TA", context(p, asset,
        call(p, "ProductAsset_TA.GetAttribute", &[var(p, 0x20, "ProductAttribute_PaintSettings_TA")?])?,
        Some("ProductAsset_TA.GetAttribute.ReturnValue"))?)
}

/// The factory call is valid only for painted inventory instances. Unpainted
/// records deliberately retain None, rather than inventing a factory paint.
pub(super) fn paint_route(p: &UpkPackage) -> R<Vec<u8>> {
    let owner = "CarMeshComponentBase_TA.ApplyPaintSettingsToObject";
    let export = &p.exports[obj(p, owner)? as usize - 1];
    let old = &p.image[export.serial_offset..export.serial_offset + export.serial_size];
    let args = ["PaintSettings", "ProductPaint", "ObjectToPaint", "Asset"].map(|name| var(p, 0x2b, &format!("{owner}.{name}"))).into_iter().collect::<R<Vec<_>>>()?;
    let original = static_call(p, "ProductAttribute_Painted_TA", call(p, "ProductAttribute_Painted_TA.ApplyPaintToObject", &args)?, None)?;
    // A replay already has a captured factory paint. Restoring it immediately
    // before applying the live paint doubles material/FX rebuilds per frame.
    let live_replay = choose(native(0xcc, &[get(p, "HbxNativePaintReplay")?, text("1")]),
        choose(not_null(args[0].clone()), enabled(p, asset_domain(p, args[3].clone())?)?, raw(&[0x28]))?, raw(&[0x28]))?;
    let restore = choose(not_null(args[1].clone()), native(0x81, &[live_replay]), raw(&[0x28]))?;
    let mut script = guard(0, restore, original)?;
    script.append(&paint(p, script.mem_size())?).append(&ret(raw(&[0x0b]))).raw(&[0x4c]);
    validate(&script, 0)?;
    function_payload(old, &script)
}

pub(super) fn paint(p: &UpkPackage, base: usize) -> R<Script> {
    let owner = "CarMeshComponentBase_TA.ApplyPaintSettingsToObject";
    let asset = var(p, 0x2b, &format!("{owner}.Asset"))?;
    let target = var(p, 0x2b, &format!("{owner}.ObjectToPaint"))?;
    let settings = var(p, 0x2b, &format!("{owner}.PaintSettings"))?;
    let paint = var(p, 0x2b, &format!("{owner}.ProductPaint"))?;
    let domain = asset_domain(p, asset.clone())?;
    let local = choose(settings_ready(p)?, raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone(), raw(&[0x28]))?;
    let mut body = set(p, "HbxPaintMesh", call(p, "Core.Object.PathName", &[raw(&[0x17])])?)?;
    body.append(&set(p, "HbxItemNativeKey", domain)?);
    let key = get(p, "HbxItemNativeKey")?;
    // Keep material snapshots and unpainted FX state separate from inventory
    // paints. Restoring None through ApplyPaintToObject is not a restoration.
    body.append(&attachment_materials(p, base + 3 + local.mem_size() + body.mem_size(), target.clone(), key.clone())?);
    body.append(&unpainted_fx(p, base + 3 + local.mem_size() + body.mem_size(), target.clone(), paint.clone(), key.clone())?);
    let detected = set_dynamic(p, join(text("HbxNativeFound."), key.clone()), text("1"))?;
    body.append(&guard(base + 3 + local.mem_size() + body.mem_size(), not_null(settings.clone()), detected)?);
    let condition = enabled(p, key.clone())?;
    let active_base = base + 3 + local.mem_size() + body.mem_size();
    let mut apply = Script::new();
    // Clone the currently equipped factory paint as the constructor template.
    // Its emissive/material metadata is retained, and its colour array is private.
    let paint_id = choose(native(0xce, &[paint.clone(), raw(&[0x2a])]),
        context(p, paint.clone(), call(p, "ProductPaint_TA.GetID", &[])?, Some("ProductPaint_TA.GetID.ReturnValue"))?, int(0))?;
    let name = join(join(join(text("HbxLivePaint_"), key.clone()), join(text("_"), cast(INT_TO_STRING, paint_id))),
        join(text("_"), get(p, "HbxLoadoutGeneration")?));
    let path = join(join(call(p, "Core.Object.PathName", &[raw(&[0x17])])?, text(".")), name.clone());
    let cached = dynamic(p, "ProductPaint_TA", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "ProductPaint_TA")?])?)?;
    let mut create = raw(&[0x11]);
    create.raw(&[0x17]).append(&name).append(&int(0)).append(&var(p, 0x20, "ProductPaint_TA")?).append(&paint);
    apply.append(&assign(paint.clone(), choose(native(0xce, &[cached.clone(), raw(&[0x2a])]), cached, create)?));
    for (index, channel) in ["R", "G", "B"].into_iter().enumerate() {
        apply.append(&set(p, &format!("HbxItemNative.{channel}"), cast(FLOAT_TO_STRING,
            colour_channel(p, key.clone(), index)?))?);
    }
    let rgb = call(p, "Core.Object.MakeLinearColor", &[
        cast(STRING_TO_FLOAT, get(p, "HbxItemNative.R")?), cast(STRING_TO_FLOAT, get(p, "HbxItemNative.G")?),
        cast(STRING_TO_FLOAT, get(p, "HbxItemNative.B")?), float(1.)])?;
    let colors = field(p, paint.clone(), "ProductPaint_TA.Colors")?;
    let index = obj(p, "ProductPaint_TA.Colors")? as usize - 1;
    let export = &p.exports[index];
    let count = u32::from_le_bytes(p.image[export.serial_offset + 16..export.serial_offset + 20].try_into().unwrap());
    if count == 0 || count > 32 { return Err("Unsupported native paint colour array".into()); }
    for i in 0..count { apply.append(&assign(fixed_element(i as i32, colors.clone()), rgb.clone())); }
    apply.append(&static_call(p, "ProductAttribute_Painted_TA", call(p, "ProductAttribute_Painted_TA.ApplyPaintToObject", &[settings.clone(), paint, target, asset])?, None)?);
    apply.append(&particle_overrides(p, active_base + 3 + condition.mem_size() + 3 + not_null(settings.clone()).mem_size() + apply.mem_size(),
        settings.clone(), var(p, 0x2b, &format!("{owner}.ObjectToPaint"))?)?);
    apply.append(&set_dynamic(p, join(text("HbxNativeApplied."), key), text("1"))?);
    // Native painting selects the effect's Painted attachment state, including
    // the parameter-driven particle templates used by Classic/Dueling Dragons.
    let has_settings = native(0xce, &[settings, raw(&[0x2a])]);
    let apply = guard(active_base + 3 + condition.mem_size(), has_settings, apply)?;
    body.append(&guard(active_base, condition, apply)?);
    guard(base, local, body)
}

const PAINT_CLOSURE: &str = "__CarMeshComponentBase_TA__ApplyPaintToObject_0x1";
fn length(array: Script) -> Script { let mut code = raw(&[0x57, 1]); code.append(&array); code }
fn element_at(index: Script, array: Script) -> Script { let mut code = raw(&[0x57, 0, 0]); code.append(&index).append(&array); code }
fn engine(p: &UpkPackage) -> R<Script> {
    dynamic(p, "Engine.GameEngine", static_call(p, "Engine.Engine", call(p, "Engine.Engine.GetEngine", &[])?, Some("Engine.Engine.GetEngine.ReturnValue"))?)
}
fn holder(p: &UpkPackage) -> R<Script> {
    let path = join(call(p, "Core.Object.PathName", &[engine(p)?])?, text(".HbxLiveColourReferences"));
    dynamic(p, "Engine.ObjectReferencer", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "Engine.ObjectReferencer")?])?)
}
pub(super) fn global_references(p: &UpkPackage) -> R<Script> { field(p, holder(p)?, "Engine.ObjectReferencer.ReferencedObjects") }
fn paint_mesh(p: &UpkPackage) -> R<Script> {
    dynamic(p, "CarMeshComponentBase_TA", call(p, "Core.Object.FindObject", &[get(p, "HbxPaintMesh")?, var(p, 0x20, "CarMeshComponentBase_TA")?])?)
}
fn bucket(p: &UpkPackage) -> R<Script> {
    let path = join(get(p, "HbxPaintMesh")?, text(".HbxLivePaintRecords"));
    dynamic(p, "Engine.ObjectReferencer", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "Engine.ObjectReferencer")?])?)
}
pub(super) fn references(p: &UpkPackage) -> R<Script> { field(p, bucket(p)?, "Engine.ObjectReferencer.ReferencedObjects") }
// Separate from the session Label used to authenticate quit requests. These
// fixed-array floats are transient loop locals, never saved through the UI.
pub(super) fn loop_state(p: &UpkPackage) -> R<Script> {
    let path = join(call(p, "Core.Object.PathName", &[engine(p)?])?, text(".HbxLiveColourReferences.HbxControlSession"));
    dynamic(p, "ProductPaint_TA", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "ProductPaint_TA")?])?)
}
pub(super) fn loop_counter(p: &UpkPackage, key: &str, write: bool) -> R<Script> {
    let slot = LOOP_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut(); let next = slots.len();
        *slots.entry(key.to_owned()).or_insert(next)
    });
    // ProductPaint.Colors has 12 LinearColor entries (48 independent floats).
    if slot >= 48 { return Err("Live loop counter storage exhausted".into()); }
    member(p, "Core.Object.LinearColor", ["R", "G", "B", "A"][slot % 4],
        fixed_element((slot / 4) as i32, field(p, loop_state(p)?, "ProductPaint_TA.Colors")?), write)
}
pub(super) fn control_session(p: &UpkPackage) -> R<Script> {
    let refs = global_references(p)?;
    let value = field(p, dynamic(p, "ProductPaint_TA", element_at(int(0), refs.clone()))?, "ProductPaint_TA.Label")?;
    choose(not_null(holder(p)?), choose(native(0x97, &[length(refs), int(0)]), value, text(""))?, text(""))
}

/// Some effects (Cure, for example) use named per-paint overrides rather than
/// CustomColor. Reuse the asset's own routing and substitute RGB only during
/// this synchronous call; never edit its shared override objects or palette.
fn particle_overrides(p: &UpkPackage, base: usize, settings: Script, target: Script) -> R<Script> {
    let groups = field(p, settings, "ProductAttribute_PaintSettings_TA.PaintsToOverride")?;
    let available = native(0x97, &[length(groups.clone()), int(0)]);
    let start = base + 3 + available.mem_size();
    let overrides = member(p, "_Types_TA.PaintWithOverride", "Overrides", element_at(int(0), groups), false)?;
    let mut body = set(p, "HbxItemOverridePass", text("1"))?;
    body.append(&super::boost::repeat(p, start + body.mem_size(), "HbxItemOverrideIndex", length(overrides.clone()), |base, index| {
        let colour = dynamic(p, "ProductOverride_ParticleSystemColorParameter_TA", element_at(index, overrides))?;
        guard(base, not_null(colour.clone()), context(p, colour,
            call(p, "ProductOverride_ParticleSystemColorParameter_TA.ApplyToObject", &[target])?, None)?)
    })?);
    body.append(&set(p, "HbxItemOverridePass", text("0"))?);
    guard(base, available, body)
}
pub(super) fn particle_override_getter(p: &UpkPackage) -> R<Vec<u8>> {
    let active = native(0xcc, &[get(p, "HbxItemOverridePass")?, text("1")]);
    let rgb = call(p, "Core.Object.MakeLinearColor", &[
        cast(STRING_TO_FLOAT, get(p, "HbxItemNative.R")?), cast(STRING_TO_FLOAT, get(p, "HbxItemNative.G")?),
        cast(STRING_TO_FLOAT, get(p, "HbxItemNative.B")?), float(1.)])?;
    prefix_original(p, "ProductOverride_ParticleSystemColorParameter_TA.GetPaintToApply", guard(0, active, ret(rgb))?)
}

fn baseline(p: &UpkPackage, target: Script, suffix: &str) -> R<Script> {
    let path = join(call(p, "Core.Object.PathName", &[target])?, text(suffix));
    dynamic(p, "Engine.ObjectReferencer", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "Engine.ObjectReferencer")?])?)
}
fn baseline_refs(p: &UpkPackage, target: Script, suffix: &str) -> R<Script> {
    field(p, baseline(p, target, suffix)?, "Engine.ObjectReferencer.ReferencedObjects")
}
fn create_baseline(p: &UpkPackage, base: usize, target: Script, suffix: &str) -> R<Script> {
    let missing = native(0x81, &[not_null(baseline(p, target.clone(), suffix)?)]);
    let roots = references(p)?;
    let mut create = assign(length(roots.clone()), native(0x92, &[length(roots.clone()), int(1)]));
    let mut object = raw(&[0x11]);
    object.append(&target).append(&text(suffix.trim_start_matches('.'))).append(&int(0))
        .append(&var(p, 0x20, "Engine.ObjectReferencer")?).raw(&[0x2a]);
    create.append(&assign(element_at(native(0x93, &[length(roots.clone()), int(1)]), roots), object));
    guard(base, missing, create)
}

/// Attachments without a Painted inventory attribute still expose CustomColor
/// in their materials. Give the equipped component private MICs, retaining the
/// exact original material references for disable/equipment changes.
fn attachment_materials(p: &UpkPackage, base: usize, target: Script, domain: Script) -> R<Script> {
    const SUFFIX: &str = ".HbxLiveOriginalMaterials";
    let mesh = dynamic(p, "Engine.MeshComponent", target)?;
    let attachment = choose(native(0xcc, &[domain.clone(), text("antenna")]), raw(&[0x27]), native(0xcc, &[domain.clone(), text("topper")]))?;
    let active = enabled(p, domain.clone())?;
    let tracked = choose(active.clone(), raw(&[0x27]), not_null(baseline(p, mesh.clone(), SUFFIX)?))?;
    let valid = choose(not_null(bucket(p)?), choose(not_null(mesh.clone()), choose(attachment, tracked, raw(&[0x28]))?, raw(&[0x28]))?, raw(&[0x28]))?;
    let start = base + 3 + valid.mem_size();
    let mut body = create_baseline(p, start, mesh.clone(), SUFFIX)?;
    body.append(&set_dynamic(p, join(text("HbxNativeFound."), domain.clone()), text("1"))?);
    let refs = baseline_refs(p, mesh.clone(), SUFFIX)?;
    let at = start + body.mem_size();
    let begin = at + 3 + active.mem_size();
    let material_ready = context(p, mesh.clone(), call(p, "Engine.MeshComponent.GetMaterial", &[int(0)])?, Some("Engine.MeshComponent.GetMaterial.ReturnValue"))?;
    let empty = choose(native(0x96, &[length(refs.clone()), int(2)]), not_null(material_ready), raw(&[0x28]))?;
    let count = context(p, mesh.clone(), call(p, "Engine.MeshComponent.GetNumElements", &[])?, Some("Engine.MeshComponent.GetNumElements.ReturnValue"))?;
    let mut snapshot = assign(length(refs.clone()), native(0x92, &[native(0x94, &[count.clone(), int(2)]), int(1)]));
    snapshot.append(&assign(element_at(int(0), refs.clone()), mesh.clone()));
    snapshot.append(&super::boost::repeat(p, begin + 3 + empty.mem_size() + snapshot.mem_size(), "HbxAttachmentMaterialIndex", count.clone(), |_, index| {
        let original = context(p, mesh.clone(), call(p, "Engine.MeshComponent.GetMaterial", &[index.clone()])?, Some("Engine.MeshComponent.GetMaterial.ReturnValue"))?;
        let mut save = assign(element_at(native(0x92, &[index.clone(), int(1)]), refs.clone()), original);
        let data_slot = element_at(native(0x92, &[native(0x92, &[index.clone(), int(1)]), count.clone()]), refs.clone());
        let live = context(p, mesh.clone(), call(p, "Engine.MeshComponent.CreateAndSetMaterialInstanceConstant", &[index.clone()])?, Some("Engine.MeshComponent.CreateAndSetMaterialInstanceConstant.ReturnValue"))?;
        save.append(&assign(data_slot.clone(), live));
        let mut data = raw(&[0x11]);
        data.append(&mesh).append(&text("")).append(&int(0)).append(&var(p, 0x20, "ProductPaint_TA")?).raw(&[0x2a]);
        save.append(&assign(data_slot.clone(), data));
        let colours = field(p, dynamic(p, "ProductPaint_TA", data_slot)?, "ProductPaint_TA.Colors")?;
        let material = dynamic(p, "Engine.MaterialInstanceConstant", context(p, mesh.clone(), call(p, "Engine.MeshComponent.GetMaterial", &[index])?, Some("Engine.MeshComponent.GetMaterial.ReturnValue"))?)?;
        for (i, name) in ["CustomColor", "CustomColorFull"].into_iter().enumerate() {
            // Native getter writes the WHOLE LinearColor into persistent data.
            // No shader, texture, static switch, parent, or material array edits.
            let get = context(p, material.clone(), call(p, "Engine.MaterialInstanceConstant.GetVectorParameterValue", &[
                cast(0x60, text(name)), fixed_element(i as i32, colours.clone())])?, Some("Engine.MaterialInstanceConstant.GetVectorParameterValue.ReturnValue"))?;
            let mut discard = raw(&[0x0e]);
            discard.object(obj(p, "Engine.MaterialInstanceConstant.GetVectorParameterValue.ReturnValue")?).append(&get);
            save.append(&discard);
        }
        Ok(save)
    })?);
    let mut apply = guard(begin, empty, snapshot)?;
    // Unpainted toppers/antennas use this fallback instead of the native
    // OverrideMeshMaterial callback. Bind their existing MICs as well, so
    // their rainbows do not depend on replaying the attachment paint path.
    apply.append(&super::boost::repeat(p, begin + apply.mem_size(), "HbxAttachmentAnimationIndex", count.clone(), |base, index| {
        let material = dynamic(p, "Engine.MaterialInstanceConstant", context(p, mesh.clone(),
            call(p, "Engine.MeshComponent.GetMaterial", &[index])?, Some("Engine.MeshComponent.GetMaterial.ReturnValue"))?)?;
        let mut bind = Script::new();
        for parameter in ["CustomColor", "CustomColorFull"] {
            bind.append(&super::animation::bind(p, paint_mesh(p)?, material.clone(), domain.clone(), cast(0x60, text(parameter)), None)?);
        }
        guard(base, not_null(material), bind)
    })?);
    let rgb = call(p, "Core.Object.MakeLinearColor", &[
        colour_channel(p, domain.clone(), 0)?, colour_channel(p, domain.clone(), 1)?, colour_channel(p, domain.clone(), 2)?, float(1.)])?;
    for name in ["CustomColor", "CustomColorFull"] {
        apply.append(&context(p, mesh.clone(), call(p, "Engine.MeshComponent.SetLinearColorParameter", &[cast(0x60, text(name)), rgb.clone()])?, None)?);
    }
    apply.append(&set_dynamic(p, join(text("HbxNativeApplied."), domain), text("1"))?);
    body.append(&guard(at, active.clone(), apply)?);
    let disabled = native(0x81, &[active]);
    let at = start + body.mem_size();
    let nonempty = native(0x97, &[length(refs.clone()), int(1)]);
    let begin = at + 3 + disabled.mem_size() + 3 + nonempty.mem_size();
    let mut restore = restore_attachment_snapshot(p, begin, mesh, refs.clone())?;
    restore.append(&assign(length(refs), int(0)));
    body.append(&guard(at, disabled.clone(), guard(at + 3 + disabled.mem_size(), nonempty, restore)?)?);
    guard(base, valid, body)
}

fn restore_attachment_snapshot(p: &UpkPackage, base: usize, mesh: Script, refs: Script) -> R<Script> {
    let count = native(0x95, &[native(0x93, &[length(refs.clone()), int(1)]), int(2)]);
    super::boost::repeat(p, base, "HbxAttachmentRestoreIndex", count.clone(), |base, index| {
        let original = dynamic(p, "Engine.MaterialInterface", element_at(native(0x92, &[index.clone(), int(1)]), refs.clone()))?;
        let material = dynamic(p, "Engine.MaterialInstanceConstant", original.clone())?;
        let data = dynamic(p, "ProductPaint_TA", element_at(native(0x92, &[native(0x92, &[index.clone(), int(1)]), count]), refs))?;
        let valid = choose(not_null(material.clone()), not_null(data.clone()), raw(&[0x28]))?;
        let mut restore = Script::new();
        for (i, name) in ["CustomColor", "CustomColorFull"].into_iter().enumerate() {
            let colour = fixed_element(i as i32, field(p, data.clone(), "ProductPaint_TA.Colors")?);
            restore.append(&context(p, material.clone(), call(p, "Engine.MaterialInstanceConstant.SetVectorParameterValue", &[cast(0x60, text(name)), colour])?, None)?);
        }
        let mut code = guard(base, valid, restore)?;
        code.append(&context(p, mesh, call(p, "Engine.MeshComponent.SetMaterial", &[index, original])?, None)?);
        Ok(code)
    })
}
/// Unpainted FX need their native Painted attachment state as well as RGB.
/// Snapshot the actor's parameter arrays before the first override, then remove
/// only the state we added and restore those arrays when disabled.
fn unpainted_fx(p: &UpkPackage, base: usize, target: Script, _factory: Script, domain: Script) -> R<Script> {
    const SUFFIX: &str = ".HbxLiveOriginalFX";
    let fx = dynamic(p, "FXActor_TA", target)?;
    let active = enabled(p, domain.clone())?;
    let tracked = choose(active.clone(), raw(&[0x27]), not_null(baseline(p, fx.clone(), SUFFIX)?))?;
    // Some stock FX expose a non-null default paint factory even though their
    // visible colour still lives in the actor's parameter dispenser. Apply
    // this fallback to every tracked local FX actor; the normal painted route
    // may run as well, but both write the same requested colour.
    let valid = choose(not_null(bucket(p)?), choose(not_null(fx.clone()), tracked, raw(&[0x28]))?, raw(&[0x28]))?;
    let start = base + 3 + valid.mem_size();
    let mut body = create_baseline(p, start, fx.clone(), SUFFIX)?;
    let refs = baseline_refs(p, fx.clone(), SUFFIX)?;
    let at = start + body.mem_size();
    let begin = at + 3 + active.mem_size();
    let empty = native(0x9a, &[length(refs.clone()), int(0)]);
    let parameters = field(p, fx.clone(), "ProjectX.FXActor_X.Parameters")?;
    let state = field(p, fx.clone(), "FXActor_TA.PaintedState")?;
    let mut snapshot = assign(length(refs.clone()), int(3));
    snapshot.append(&assign(element_at(int(0), refs.clone()), fx.clone()));
    let mut clone = raw(&[0x11]);
    // This is a component, not an Actor: a UObject template clone is valid and
    // copies the native parameter arrays without spawning a second effect.
    clone.append(&fx).append(&text("HbxLiveOriginalParameters")).append(&int(0))
        .append(&var(p, 0x20, "ProjectX.ParameterDispenser_X")?).append(&parameters);
    snapshot.append(&assign(element_at(int(1), refs.clone()), choose(not_null(parameters.clone()), clone, raw(&[0x2a]))?));
    let was_active = context(p, fx.clone(), call(p, "ProjectX.FXActor_X.IsStateActive", &[state.clone()])?, Some("ProjectX.FXActor_X.IsStateActive.ReturnValue"))?;
    snapshot.append(&assign(element_at(int(2), refs.clone()), choose(was_active, state.clone(), raw(&[0x2a]))?));
    body.append(&guard(at, active.clone(), guard(begin, empty, snapshot)?)?);
    // Stock trails and several topper/boost effects have no Painted inventory
    // attribute. Their colour lives directly in the existing dispenser, so the
    // normal ProductPaint path above has nothing to apply. Recolour those
    // existing colour parameters in place; the snapshot restores them exactly.
    let at = start + body.mem_size();
    let rgb = call(p, "Core.Object.MakeLinearColor", &[
        colour_channel(p, domain.clone(), 0)?,
        colour_channel(p, domain.clone(), 1)?,
        colour_channel(p, domain.clone(), 2)?,
        float(1.),
    ])?;
    let colours = field(p, parameters.clone(), "ProjectX.ParameterDispenser_X.ColorParams")?;
    let mut recolour = super::boost::repeat(p, at + 3 + active.mem_size(), "HbxUnpaintedColourIndex", length(colours.clone()), |_base, index| {
        let pair = element_at(index, colours.clone());
        let key = member(p, "ProjectX.ParameterDispenser_X.ColorParamPair", "Key", pair.clone(), false)?;
        let value = member(p, "ProjectX.ParameterDispenser_X.ColorParamPair", "Value", pair.clone(), true)?;
        let mut apply = assign(value.clone(), rgb.clone());
        apply.append(&context(p, parameters.clone(), virtual_call(p, "SetLinearColorParameter", &[key, value])?, None)?);
        Ok(apply)
    })?;
    recolour.append(&context(p, fx.clone(), call(p, "ProjectX.FXActor_X.AddState", &[state.clone()])?, None)?);
    recolour.append(&set_dynamic(p, join(text("HbxNativeFound."), domain.clone()), text("1"))?);
    recolour.append(&set_dynamic(p, join(text("HbxNativeApplied."), domain.clone()), text("1"))?);
    body.append(&guard(at, active.clone(), recolour)?);
    let disabled = native(0x81, &[active]);
    let at = start + body.mem_size();
    let nonempty = native(0x97, &[length(refs.clone()), int(0)]);
    let begin = at + 3 + disabled.mem_size() + 3 + nonempty.mem_size();
    let saved = dynamic(p, "ProjectX.ParameterDispenser_X", element_at(int(1), refs.clone()))?;
    let mut restore = Script::new();
    for field_name in ["ColorParams", "VectorParams", "FloatParams"] {
        restore.append(&assign(field(p, field(p, fx.clone(), "ProjectX.FXActor_X.Parameters")?, &format!("ProjectX.ParameterDispenser_X.{field_name}"))?,
            field(p, saved.clone(), &format!("ProjectX.ParameterDispenser_X.{field_name}"))?));
    }
    let mut restore = guard(begin, not_null(saved), restore)?;
    restore.append(&context(p, fx.clone(), call(p, "ProjectX.FXActor_X.RemoveStateForceUpdate", &[state.clone()])?, None)?);
    let originally_active = not_null(element_at(int(2), refs.clone()));
    restore.append(&guard(begin + restore.mem_size(), originally_active,
        context(p, fx, call(p, "ProjectX.FXActor_X.AddState", &[state])?, None)?)?);
    restore.append(&assign(length(refs), int(0)));
    body.append(&guard(at, disabled.clone(), guard(at + 3 + disabled.mem_size(), nonempty, restore)?)?);
    guard(base, valid, body)
}
pub(super) fn ensure_holder(p: &UpkPackage, base: usize) -> R<Script> {
    let available = not_null(engine(p)?);
    let missing = native(0x81, &[not_null(holder(p)?)]);
    let roots = field(p, engine(p)?, "Engine.GameEngine.ObjectReferencers")?;
    let mut create = assign(length(roots.clone()), native(0x92, &[length(roots.clone()), int(1)]));
    let mut object = raw(&[0x11]);
    object.append(&engine(p)?).append(&text("HbxLiveColourReferences")).append(&int(0))
        .append(&var(p, 0x20, "Engine.ObjectReferencer")?).raw(&[0x2a]);
    create.append(&assign(element_at(native(0x93, &[length(roots.clone()), int(1)]), roots), object));
    // Reserve one private object for a per-game session token. Unlike saved
    // values this object is recreated on game restart, so an interrupted quit
    // request cannot close a later game session.
    let refs = global_references(p)?;
    create.append(&assign(length(refs.clone()), int(1)));
    let mut session = raw(&[0x11]);
    session.append(&holder(p)?).append(&text("HbxControlSession")).append(&int(0))
        .append(&var(p, 0x20, "ProductPaint_TA")?).raw(&[0x2a]);
    create.append(&assign(element_at(int(0), refs.clone()), session));
    let session = dynamic(p, "ProductPaint_TA", element_at(int(0), refs))?;
    create.append(&assign(field(p, session, "ProductPaint_TA.Label")?,
        static_call(p, "Core.Object", call(p, "Core.Object.TimeStamp", &[])?, Some("Core.Object.TimeStamp.ReturnValue"))?));
    // GetSingleton is a lookup, not an arbitrary-object constructor. Retain our
    // private referencer in the engine's native GC-aware root array instead.
    guard(base, available.clone(), guard(base + 3 + available.mem_size(), missing, create)?)
}

fn ensure_bucket(p: &UpkPackage, base: usize) -> R<Script> {
    let mut out = ensure_holder(p, base)?;
    let valid = choose(not_null(holder(p)?), not_null(paint_mesh(p)?), raw(&[0x28]))?;
    let missing = native(0x81, &[not_null(bucket(p)?)]);
    let at = base + out.mem_size();
    let start = at + 3 + valid.mem_size() + 3 + missing.mem_size();
    let roots = global_references(p)?;
    let mut create = set(p, "HbxPaintBucketSlot", text("0"))?;
    create.append(&super::boost::repeat(p, start + create.mem_size(), "HbxPaintBucketIndex", length(roots.clone()), |base, index| {
        let entry = element_at(index.clone(), roots.clone());
        let available = object_eq(&[entry, raw(&[0x2a])]);
        let unset = native(0x9a, &[number(p, "HbxPaintBucketSlot")?, int(0)]);
        guard(base, available.clone(), guard(base + 3 + available.mem_size(), unset,
            set(p, "HbxPaintBucketSlot", cast(INT_TO_STRING, native(0x92, &[index, int(1)])))?)?)
    })?);
    let mut append = set(p, "HbxPaintBucketSlot", cast(INT_TO_STRING, native(0x92, &[length(roots.clone()), int(1)])))?;
    append.append(&assign(length(roots.clone()), number(p, "HbxPaintBucketSlot")?));
    create.append(&guard(start + create.mem_size(), native(0x9a, &[number(p, "HbxPaintBucketSlot")?, int(0)]), append)?);
    let mut object = raw(&[0x11]);
    object.append(&paint_mesh(p)?).append(&text("HbxLivePaintRecords")).append(&int(0)).append(&var(p, 0x20, "Engine.ObjectReferencer")?).raw(&[0x2a]);
    create.append(&assign(element_at(native(0x93, &[number(p, "HbxPaintBucketSlot")?, int(1)]), roots), object));
    out.append(&guard(at, valid.clone(), guard(at + 3 + valid.mem_size(), missing, create)?)?);
    Ok(out)
}

fn prune_buckets(p: &UpkPackage, base: usize) -> R<Script> {
    let roots = global_references(p)?;
    super::boost::repeat(p, base, "HbxPaintBucketPruneIndex", length(roots.clone()), |base, index| {
        let entry = element_at(index, roots);
        let old = dynamic(p, "Engine.ObjectReferencer", entry.clone())?;
        let owner = field(p, old.clone(), "Core.Object.Outer")?;
        let actor = actor_of(p, owner.clone())?;
        let deleting = choose(not_null(actor.clone()), bool_field(p, actor, "Engine.Actor.bDeleteMe")?, raw(&[0x28]))?;
        let stale = choose(not_null(old), choose(dead(p, owner)?, raw(&[0x27]), deleting)?, raw(&[0x28]))?;
        guard(base, stale, assign(entry, raw(&[0x2a])))
    })
}
fn number(p: &UpkPackage, key: &str) -> R<Script> { Ok(cast(0x44, cast(STRING_TO_FLOAT, get(p, key)?))) }
fn not_null(value: Script) -> Script { native(0xce, &[value, raw(&[0x2a])]) }
fn bool_field(p: &UpkPackage, target: Script, property: &str) -> R<Script> {
    // EX_BoolVariable consumes a variable selector and a UBoolProperty reference,
    // not an arbitrary expression. Evaluate the object context outside it.
    let mut value = raw(&[0x2d]);
    value.append(&var(p, 0x01, property)?);
    context(p, target, value, Some(property))
}
fn dead(p: &UpkPackage, value: Script) -> R<Script> {
    choose(not_null(value.clone()), context(p, value, call(p, "Core.Object.IsPendingKill", &[])?, Some("Core.Object.IsPendingKill.ReturnValue"))?, raw(&[0x27]))
}


fn actor_of(p: &UpkPackage, target: Script) -> R<Script> {
    let actor = dynamic(p, "Engine.Actor", target.clone())?;
    let component = dynamic(p, "Engine.ActorComponent", target)?;
    choose(not_null(actor.clone()), actor, choose(not_null(component.clone()), field(p, component, "Engine.ActorComponent.Owner")?, raw(&[0x2a]))?)
}
fn mesh_of_actor(p: &UpkPackage, actor: Script) -> R<Script> {
    let vehicle = dynamic(p, "Vehicle_TA", actor.clone())?;
    let preview = dynamic(p, "CarPreviewActor_TA", actor)?;
    choose(not_null(vehicle.clone()), field(p, vehicle, "Vehicle_TA.CarMesh")?,
        choose(not_null(preview.clone()), field(p, preview, "CarPreviewActor_TA.CarMesh")?, raw(&[0x2a]))?)
}
fn source_mesh(p: &UpkPackage, target: Script) -> R<Script> {
    let forced_name = get(p, "HbxNativeForcedOwner")?;
    let forced = dynamic(p, "CarMeshComponentBase_TA", call(p, "Core.Object.FindObject", &[forced_name, var(p, 0x20, "CarMeshComponentBase_TA")?])?)?;
    let actor = actor_of(p, target.clone())?;
    let owner = choose(not_null(actor.clone()), mesh_of_actor(p, field(p, actor.clone(), "Engine.Actor.Owner")?)?, raw(&[0x2a]))?;
    let direct_owner = mesh_of_actor(p, actor)?;
    let fx = dynamic(p, "ProjectX.FXActor_X", target.clone())?;
    let attached = choose(not_null(fx.clone()), mesh_of_actor(p, field(p, fx, "ProjectX.FXActor_X.AttachmentActor")?)?, raw(&[0x2a]))?;
    let boost = dynamic(p, "FXActor_Boost_TA", target.clone())?;
    let boost_mesh = choose(not_null(boost.clone()), field(p, boost, "FXActor_Boost_TA.CarMesh")?, raw(&[0x2a]))?;
    let direct = dynamic(p, "CarMeshComponentBase_TA", target)?;
    let mut mesh = raw(&[0x2a]);
    for candidate in [owner, attached, direct_owner, boost_mesh, direct] {
        mesh = choose(not_null(candidate.clone()), candidate, mesh)?;
    }
    choose(not_null(forced.clone()), forced, mesh)
}
/// Capture the actual native paint call, including boosts and goals. A private
/// copy of its arguments is retained in a GC-aware reference holder. Never save
/// a replacement paint into the original factory callback.
pub(super) fn capture(p: &UpkPackage, base: usize) -> R<Script> {
    let owner = "ProductAttribute_Painted_TA.ApplyPaintToObject";
    let asset = var(p, 0x2b, &format!("{owner}.Asset"))?;
    let target = var(p, 0x2b, &format!("{owner}.Target"))?;
    let settings = var(p, 0x2b, &format!("{owner}.PaintSettings"))?;
    let factory = var(p, 0x2b, &format!("{owner}.Paint"))?;
    capture_arguments(p, base, asset, target, settings, factory)
}
fn capture_arguments(p: &UpkPackage, base: usize, asset: Script, target: Script, settings: Script, factory: Script) -> R<Script> {
    let replay = native(0xcc, &[get(p, "HbxNativePaintReplay")?, text("1")]);
    let private_paint = native(0x9b, &[virtual_call(p, "InStr", &[cast(0x56, factory.clone()), text(".HbxLivePaint_")])?, int(-1)]);
    let recognized = native(0xc0, &[asset_domain(p, asset.clone())?, text("")]);
    let source = choose(not_null(asset.clone()), choose(not_null(target.clone()), recognized, raw(&[0x28]))?, raw(&[0x28]))?;
    let local_mesh = source_mesh(p, target.clone())?;
    // Remote/auxiliary meshes never receive local colours. Do not let their
    // startup paint calls consume the local registry's capacity or scan time.
    let is_local = choose(not_null(local_mesh.clone()), bool_field(p, local_mesh, "CarMeshComponentBase_TA.bLocalPlayer")?, raw(&[0x28]))?;
    let source = choose(source, is_local, raw(&[0x28]))?;
    let usable = choose(settings_ready(p)?, choose(replay, raw(&[0x28]), choose(private_paint, raw(&[0x28]), source)?)?, raw(&[0x28]))?;
    let outer_start = base + 3 + usable.mem_size();
    let mut prefix = set(p, "HbxPaintMesh", call(p, "Core.Object.PathName", &[source_mesh(p, target.clone())?])?)?;
    prefix.append(&ensure_bucket(p, outer_start + prefix.mem_size())?);
    let ready = not_null(bucket(p)?);
    let start = outer_start + prefix.mem_size() + 3 + ready.mem_size();
    let product = context(p, asset.clone(), call(p, "ProductAsset_TA.GetProduct", &[])?, Some("ProductAsset_TA.GetProduct.ReturnValue"))?;
    let slot = field(p, product, "ProductTemplate_TA.Slot")?;
    let seen = join(join(join(text("HbxNativePaint.Index."), cast(0x56, target.clone())), text(".")), cast(0x56, slot.clone()));
    let refs = references(p)?;
    let mut body = set(p, "HbxNativeRootSlot", get_dynamic(p, seen.clone())?)?;
    let current = element_at(native(0x93, &[number(p, "HbxNativeRootSlot")?, int(1)]), refs.clone());
    let old_callback = dynamic(p, PAINT_CLOSURE, current)?;
    let old_target = field(p, old_callback.clone(), &format!("{PAINT_CLOSURE}.ObjectToPaint"))?;
    let same_target = choose(not_null(old_callback), object_eq(&[old_target, target.clone()]), raw(&[0x28]))?;
    let positive = native(0x97, &[number(p, "HbxNativeRootSlot")?, int(0)]);
    let valid_slot = choose(positive, native(0x96, &[number(p, "HbxNativeRootSlot")?, native(0x92, &[length(refs.clone()), int(1)])]), raw(&[0x28]))?;
    body.append(&guard(start + body.mem_size(), native(0x81, &[valid_slot.clone()]), set(p, "HbxNativeRootSlot", text("0"))?)?);
    let reset_base = start + body.mem_size() + 3 + valid_slot.mem_size();
    body.append(&guard(start + body.mem_size(), valid_slot,
        guard(reset_base, native(0x81, &[same_target]), set(p, "HbxNativeRootSlot", text("0"))?)?)?);
    let needs_slot = native(0x9a, &[number(p, "HbxNativeRootSlot")?, int(0)]);
    let scan_base = start + body.mem_size() + 3 + needs_slot.mem_size();
    let mut scan = super::boost::repeat(p, scan_base, "HbxNativeRootIndex", length(refs.clone()), |base, index| {
        let entry = element_at(index.clone(), refs.clone());
        let callback = dynamic(p, PAINT_CLOSURE, entry.clone())?;
        let old_target = field(p, callback.clone(), &format!("{PAINT_CLOSURE}.ObjectToPaint"))?;
        let actor = actor_of(p, old_target.clone())?;
        let deleted_actor = choose(not_null(actor.clone()), bool_field(p, actor, "Engine.Actor.bDeleteMe")?, raw(&[0x28]))?;
        // This private referencer retains callbacks and restoration snapshots.
        // Check each class before reading its fields; failed casts are not roots.
        let baseline = dynamic(p, "Engine.ObjectReferencer", entry.clone())?;
        let baseline_refs = field(p, baseline.clone(), "Engine.ObjectReferencer.ReferencedObjects")?;
        let baseline_target = element_at(int(0), baseline_refs.clone());
        let baseline_actor = actor_of(p, baseline_target.clone())?;
        let baseline_deleted = choose(not_null(baseline_actor.clone()), bool_field(p, baseline_actor, "Engine.Actor.bDeleteMe")?, raw(&[0x28]))?;
        let baseline_dead = choose(native(0x97, &[length(baseline_refs), int(0)]),
            choose(baseline_deleted, raw(&[0x27]), dead(p, baseline_target)?)?, dead(p, field(p, baseline.clone(), "Core.Object.Outer")?)?)?;
        let baseline_stale = choose(not_null(baseline), baseline_dead, raw(&[0x28]))?;
        let stale = choose(not_null(callback.clone()), choose(dead(p, callback)?, raw(&[0x27]), choose(deleted_actor, raw(&[0x27]), dead(p, old_target)?)?)?, baseline_stale)?;
        let mut code = guard(base, stale, assign(entry.clone(), raw(&[0x2a])))?;
        let candidate = dynamic(p, PAINT_CLOSURE, entry.clone())?;
        let old_asset = field(p, candidate.clone(), &format!("{PAINT_CLOSURE}.Product"))?;
        let old_target = field(p, candidate.clone(), &format!("{PAINT_CLOSURE}.ObjectToPaint"))?;
        let same = choose(not_null(candidate), choose(object_eq(&[old_target, target.clone()]), object_eq(&[old_asset, asset.clone()]), raw(&[0x28]))?, raw(&[0x28]))?;
        let at = base + code.mem_size();
        code.append(&guard(at, same, set(p, "HbxNativeRootSlot", cast(INT_TO_STRING, native(0x92, &[index.clone(), int(1)])))?)?);
        let empty = object_eq(&[entry, raw(&[0x2a])]);
        let unfound = native(0x9a, &[number(p, "HbxNativeRootSlot")?, int(0)]);
        let set = set(p, "HbxNativeRootSlot", cast(INT_TO_STRING, native(0x92, &[index, int(1)])))?;
        let next = base + code.mem_size();
        code.append(&guard(next, empty.clone(), guard(next + 3 + empty.mem_size(), unfound, set)?)?);
        Ok(code)
    })?;
    let unfound = native(0x9a, &[number(p, "HbxNativeRootSlot")?, int(0)]);
    let room = native(0x96, &[length(refs.clone()), int(512)]);
    let mut append = set(p, "HbxNativeRootSlot", cast(INT_TO_STRING, native(0x92, &[length(refs.clone()), int(1)])))?;
    append.append(&assign(length(refs.clone()), number(p, "HbxNativeRootSlot")?));
    let next = scan_base + scan.mem_size();
    scan.append(&guard(next, unfound.clone(), guard(next + 3 + unfound.mem_size(), room, append)?)?);
    body.append(&guard(start + body.mem_size(), needs_slot, scan)?);
    let valid = native(0x97, &[number(p, "HbxNativeRootSlot")?, int(0)]);
    let store_base = start + body.mem_size() + 3 + valid.mem_size();
    let entry = element_at(native(0x93, &[number(p, "HbxNativeRootSlot")?, int(1)]), refs);
    let callback = dynamic(p, PAINT_CLOSURE, entry.clone())?;
    let mut create = raw(&[0x11]);
    // Several targets can share one product slot (wheel components, FX, etc.).
    // Give each retained record a distinct name, not just the slot's name.
    create.append(&source_mesh(p, target.clone())?).append(&join(text("HbxLiveCapture_"), get(p, "HbxNativeRootSlot")?)).append(&int(0)).append(&var(p, 0x20, PAINT_CLOSURE)?).raw(&[0x2a]);
    let mut store = guard(store_base, native(0x81, &[not_null(callback.clone())]), assign(entry, create))?;
    // A native recapture means the target may have reset its material/FX state.
    // Replay passes are excluded above, so invalidate only on real game changes.
    store.append(&set_dynamic(p, join(text("HbxChangedPaintLastApplied."), cast(0x56, callback.clone())), text(""))?);
    store.append(&set(p, "HbxPaintCaptureGeneration", cast(INT_TO_STRING,
        native(0x92, &[number(p, "HbxPaintCaptureGeneration")?, int(1)])))?);
    // Unpainted registration only runs when there is no Painted attribute.
    // Clear a previous variant if the same component/asset is reused unpainted.
    store.append(&assign(field(p, callback.clone(), &format!("{PAINT_CLOSURE}.ProductPaint"))?, factory));
    for (name, value) in [("Product", asset), ("ObjectToPaint", target), ("PaintSettings", settings)] {
        store.append(&assign(field(p, callback.clone(), &format!("{PAINT_CLOSURE}.{name}"))?, value));
    }
    store.append(&set_dynamic(p, seen, get(p, "HbxNativeRootSlot")?)?);
    body.append(&guard(start + body.mem_size(), valid, store)?);
    prefix.append(&guard(outer_start + prefix.mem_size(), ready, body)?);
    guard(base, usable, prefix)
}
fn refresh_paints(p: &UpkPackage, base: usize, mesh: Script) -> R<Script> {
    let refs = references(p)?;
    let mut out = ensure_bucket(p, base)?;
    out.append(&set(p, "HbxNativeRefreshPasses", cast(INT_TO_STRING, native(0x92, &[number(p, "HbxNativeRefreshPasses")?, int(1)])))?);
    out.append(&set(p, "HbxNativePaintReplay", text("1"))?);
    for key in ["trim", "decal", "wheels", "boost", "antenna", "topper", "trail", "goal_explosion"] {
        out.append(&set(p, &format!("HbxNativeFound.{key}"), text("0"))?);
        out.append(&set(p, &format!("HbxNativeApplied.{key}"), text("0"))?);
    }
    out.append(&set(p, "HbxNativeRefreshCount", cast(INT_TO_STRING, length(refs.clone())))?);
    let ready = not_null(bucket(p)?);
    let at = base + out.mem_size();
    let loop_body = super::boost::repeat(p, at + 3 + ready.mem_size(), "HbxNativeRefreshIndex", number(p, "HbxNativeRefreshCount")?, |base, index| {
        // Snapshot the length; painting can append restoration references.
        let mut leading = Script::new();
        let entry = element_at(index, refs);
        let callback = dynamic(p, PAINT_CLOSURE, entry.clone())?;
        let target = field(p, callback.clone(), &format!("{PAINT_CLOSURE}.ObjectToPaint"))?;
        let actor = actor_of(p, target.clone())?;
        let deleting = choose(not_null(actor.clone()), bool_field(p, actor, "Engine.Actor.bDeleteMe")?, raw(&[0x28]))?;
        let alive = choose(not_null(callback.clone()), choose(dead(p, target.clone())?, raw(&[0x28]), native(0x81, &[deleting]))?, raw(&[0x28]))?;
        let stale = choose(not_null(callback.clone()), native(0x81, &[alive.clone()]), raw(&[0x28]))?;
        leading.append(&guard(base, stale, assign(entry, raw(&[0x2a])))?);
        let base = base + leading.mem_size();
        // The callback's Within outer is the capturing car mesh. This remains
        // valid for trails, topper meshes and FX whose Actor.Owner is unset.
        let owns = object_eq(&[field(p, callback.clone(), "Core.Object.Outer")?, mesh.clone()]);
        let mut args = Vec::new();
        // Match the engine's closure call, not the property export order.
        // UFunction parameters are linked in ProductPaint, PaintSettings,
        // ObjectToPaint, Asset order; reversing them passes None as Asset.
        for name in ["ProductPaint", "PaintSettings", "ObjectToPaint", "Product"] {
            args.push(field(p, callback.clone(), &format!("{PAINT_CLOSURE}.{name}"))?);
        }
        // Reapplying the same native goal paint repeatedly re-enters its FX
        // state/particle setup. A one-shot explosion needs changes, not a
        // continuous car-material refresh (especially through goal replays).
        // Every native paint replay can rebuild materials/FX, not just goals.
        // Static paints apply once; animated paints advance on their colour clock.
        let domain_value = asset_domain(p, field(p, callback.clone(), &format!("{PAINT_CLOSURE}.Product"))?)?;
        let domain = get(p, "HbxReplayDomain")?;
        let goal = native(0xcc, &[domain.clone(), text("goal_explosion")]);
        let wheel = native(0xcc, &[domain.clone(), text("wheels")]);
        let is_enabled = enabled(p, domain.clone())?;
        let stamp_key = join(text("HbxChangedPaintLastApplied."), cast(0x56, callback));
        // Wheels animate existing MIC parameters in their dedicated pass.
        // Other native paints need at most four updates per second.
        let material_only = choose(native(0xcc, &[domain.clone(), text("trim")]), raw(&[0x27]),
            choose(native(0xcc, &[domain.clone(), text("decal")]), raw(&[0x27]),
            choose(native(0xcc, &[domain.clone(), text("antenna")]), raw(&[0x27]), native(0xcc, &[domain.clone(), text("topper")]))?)?)?;
        let one_shot = choose(goal, raw(&[0x27]), choose(wheel, raw(&[0x27]), material_only)?)?;
        let frame = cast(INT_TO_STRING, cast(0x44, native(0xab, &[
            cast(STRING_TO_FLOAT, get(p, "HbxRainbowTime")?), float(4.)])));
        let phase = choose(one_shot, text(""), frame)?;
        let animated = native(0xcc, &[lookup(p, domain.clone(), ".rainbow")?, text("1")]);
        let stamp = join(join(get(p, "HbxLiveCache")?, choose(animated, phase, text(""))?), get(p, "HbxLoadoutGeneration")?);
        let mesh_target = dynamic(p, "Engine.MeshComponent", target.clone())?;
        let material = choose(not_null(mesh_target.clone()), context(p, mesh_target,
            call(p, "Engine.MeshComponent.GetMaterial", &[int(0)])?, Some("Engine.MeshComponent.GetMaterial.ReturnValue"))?, raw(&[0x2a]))?;
        let stamp = join(join(stamp, text("|")), cast(0x56, material));
        let changed = native(0xc0, &[stamp.clone(), get_dynamic(p, stamp_key.clone())?]);
        let active_key = join(stamp_key.clone(), text(".active"));
        let was_active = native(0xcc, &[get_dynamic(p, active_key.clone())?, text("1")]);
        // Disabled paints never enter the live repaint path, except once
        // to undo a previously applied override on the still-existing effect.
        let needs_update = choose(is_enabled.clone(), changed, was_active)?;
        let needed = needs_update;
        let at = base + 3 + alive.mem_size() + 3 + owns.mem_size();
        let mut update = set(p, "HbxReplayDomain", domain_value)?;
        let mut report = set_dynamic(p, join(text("HbxNativeFound."), domain.clone()), text("1"))?;
        report.append(&set_dynamic(p, join(text("HbxNativeApplied."), domain),
            choose(is_enabled.clone(), text("1"), text("0"))?)?);
        update.append(&report);
        let mut apply = context(p, mesh, call(p, "CarMeshComponentBase_TA.ApplyPaintSettingsToObject", &args)?, None)?;
        let mut remember = set_dynamic(p, stamp_key, stamp)?;
        remember.append(&set_dynamic(p, active_key,
            choose(is_enabled, text("1"), text("0"))?)?);
        apply.append(&remember);
        update.append(&guard(at + update.mem_size(), needed, apply)?);
        leading.append(&guard(base, alive.clone(), guard(base + 3 + alive.mem_size(), owns, update)?)?);
        Ok(leading)
    })?;
    out.append(&guard(at, ready, loop_body)?);
    out.append(&set(p, "HbxNativePaintReplay", text("0"))?);
    Ok(out)
}

pub(super) fn refresh_mesh(p: &UpkPackage, base: usize) -> R<Script> {
    let mesh = raw(&[0x17]);
    let time = mesh_time(p)?;
    let clock = join(text("HbxNativeRefreshNext."), cast(0x56, mesh.clone()));
    let next = cast(STRING_TO_FLOAT, get_dynamic(p, clock.clone())?);
    let rewind = native(0xb0, &[time.clone(), native(0xaf, &[next, float(1.)])]);
    let mut out = guard(base, rewind, set_dynamic(p, clock.clone(), text("0"))?)?;
    let due = native(0xb1, &[time.clone(), cast(STRING_TO_FLOAT, get_dynamic(p, clock.clone())?)]);
    let start = base + out.mem_size() + 3 + due.mem_size();
    let mut update = set_dynamic(p, clock, cast(FLOAT_TO_STRING, native(0xae, &[time, float(0.1)])))?;
    update.append(&set(p, "HbxPaintMesh", call(p, "Core.Object.PathName", &[mesh.clone()])?)?);
    let enabled = flag(p, "items.enabled")?;
    let previous_key = join(text("HbxItemsWereActive."), cast(0x56, mesh.clone()));
    let previous = native(0xcc, &[get_dynamic(p, previous_key.clone())?, text("1")]);
    let active = choose(enabled.clone(), raw(&[0x27]), previous)?;
    let at = start + update.mem_size();
    let mut paint = reconcile_mesh(p, at + 3 + active.mem_size())?;
    paint.append(&refresh_paints(p, at + 3 + active.mem_size() + paint.mem_size(), mesh)?);
    paint.append(&set_dynamic(p, previous_key, choose(enabled, text("1"), text("0"))?)?);
    update.append(&guard(at, active, paint)?);
    out.append(&guard(base + out.mem_size(), due, update)?);
    Ok(out)
}
fn mesh_time(p: &UpkPackage) -> R<Script> {
    // Preview components can exist before ActorComponent.Owner is assigned.
    // The controller supplies a valid clock before dispatching paint work.
    Ok(cast(STRING_TO_FLOAT, get(p, "HbxRainbowTime")?))
}
pub(super) fn inherited_added(p: &UpkPackage, base: usize) -> R<Script> {
    let local = raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone();
    // Our reconciliation is already capturing these associations. Do not
    // reset its re-entrancy clock from inside that synchronous pass.
    let external = native(0xc0, &[get(p, "HbxPaintDispatch")?, text("1")]);
    let ready = choose(settings_ready(p)?, choose(local, external, raw(&[0x28]))?, raw(&[0x28]))?;
    let mut update = set(p, "HbxInheritedGeneration", cast(INT_TO_STRING,
        native(0x92, &[number(p, "HbxInheritedGeneration")?, int(1)])))?;
    for prefix in ["HbxNativeRefreshNext.", "HbxNativeReconcile."] {
        update.append(&set_dynamic(p, join(text(prefix), cast(0x56, raw(&[0x17]))), text("0"))?);
    }
    guard(base, ready, update)
}
/// Preview decal changes rebuild materials without calling the match-only
/// SetLoadout hook. Retire callbacks for the previous skin and reconcile the
/// current associations on the next scheduled pass, independent of RGB edits.
pub(super) fn materials_changed(p: &UpkPackage, base: usize) -> R<Script> {
    let local = raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone();
    let ready = choose(settings_ready(p)?, local, raw(&[0x28]))?;
    let start = base + 3 + ready.mem_size();
    let mut out = set(p, "HbxPaintMesh", call(p, "Core.Object.PathName", &[raw(&[0x17])])?)?;
    out.append(&set(p, "HbxLoadoutGeneration", cast(INT_TO_STRING, native(0x92, &[number(p, "HbxLoadoutGeneration")?, int(1)])))?);
    for prefix in ["HbxNativeRefreshNext.", "HbxNativeReconcile."] {
        out.append(&set_dynamic(p, join(text(prefix), cast(0x56, raw(&[0x17]))), text("0"))?);
    }
    let valid = not_null(bucket(p)?);
    let at = start + out.mem_size();
    let array = references(p)?;
    let clear = super::boost::repeat(p, at + 3 + valid.mem_size(), "HbxMaterialResetIndex", length(array.clone()), |base, index| {
        let entry = element_at(index, array);
        guard(base, not_null(dynamic(p, PAINT_CLOSURE, entry.clone())?), assign(entry, raw(&[0x2a])))
    })?;
    out.append(&guard(at, valid, clear)?);
    guard(base, ready, out)
}
/// Asset initialization can precede SaveGameManager readiness. Revisit the
/// engine's own asset/object associations so missing captures recover later,
/// including components recreated by a preview or loadout change.
fn reconcile_mesh(p: &UpkPackage, base: usize) -> R<Script> {
    let clock = join(text("HbxNativeReconcile."), cast(0x56, raw(&[0x17])));
    let time = mesh_time(p)?;
    let next = cast(STRING_TO_FLOAT, get_dynamic(p, clock.clone())?);
    let reset = native(0xb0, &[time.clone(), native(0xaf, &[next.clone(), float(2.)])]);
    let mut code = guard(base, reset, set_dynamic(p, clock.clone(), text("0"))?)?;
    let due = native(0xb1, &[time.clone(), cast(STRING_TO_FLOAT, get_dynamic(p, clock.clone())?)]);
    let at = base + code.mem_size();
    let start = at + 3 + due.mem_size();
    let mut reconcile = set_dynamic(p, clock, cast(FLOAT_TO_STRING, native(0xae, &[time, float(2.)])))?;
    let objects = var(p, 0x01, "CarMeshComponentBase_TA.ObjectsWithInheritedSettings")?;
    // ApplyAttributes adds inherited associations itself. Never iterate a
    // growing length or reapply an already captured product every two seconds.
    reconcile.append(&set(p, "HbxNativeReconcileCount", cast(INT_TO_STRING, length(objects.clone())))?);
    reconcile.append(&super::boost::repeat(p, start + reconcile.mem_size(), "HbxNativeReconcileIndex", number(p, "HbxNativeReconcileCount")?, |base, index| {
        let pair = element_at(index, objects);
        let asset = member(p, "CarMeshComponentBase_TA.InheritedObjects", "Asset", pair.clone(), false)?;
        let target = member(p, "CarMeshComponentBase_TA.InheritedObjects", "ObjectToEdit", pair, false)?;
        let product = context(p, asset.clone(), call(p, "ProductAsset_TA.GetProduct", &[])?, Some("ProductAsset_TA.GetProduct.ReturnValue"))?;
        let slot = field(p, product, "ProductTemplate_TA.Slot")?;
        let seen = join(join(join(text("HbxNativePaint.Index."), cast(0x56, target.clone())), text(".")), cast(0x56, slot));
        let index = cast(0x44, cast(STRING_TO_FLOAT, get_dynamic(p, seen)?));
        let in_range = choose(native(0x97, &[index.clone(), int(0)]), native(0x96, &[index.clone(), native(0x92, &[length(references(p)?), int(1)])]), raw(&[0x28]))?;
        let callback = dynamic(p, PAINT_CLOSURE, element_at(native(0x93, &[index, int(1)]), references(p)?))?;
        let same_target = object_eq(&[field(p, callback.clone(), &format!("{PAINT_CLOSURE}.ObjectToPaint"))?, target.clone()]);
        let same_asset = object_eq(&[field(p, callback.clone(), &format!("{PAINT_CLOSURE}.Product"))?, asset.clone()]);
        // Paint settings may finish loading after the association/callback.
        // A retained callback with None settings is not ready forever: recapture
        // once the asset exposes its actual settings, without repaint polling.
        let same_settings = object_eq(&[field(p, callback.clone(), &format!("{PAINT_CLOSURE}.PaintSettings"))?, asset_settings(p, asset.clone())?]);
        let captured = choose(in_range, choose(not_null(callback), choose(same_target,
            choose(same_asset, same_settings, raw(&[0x28]))?, raw(&[0x28]))?, raw(&[0x28]))?, raw(&[0x28]))?;
        let missing = native(0x81, &[captured]);
        let valid = choose(not_null(asset.clone()), choose(not_null(target.clone()), missing, raw(&[0x28]))?, raw(&[0x28]))?;
        guard(base, valid, call(p, "CarMeshComponentBase_TA.ApplyAttributes", &[asset, target])?)
    })?);
    code.append(&guard(at, due, reconcile)?);
    Ok(code)
}

/// SetLoadout rebuilds the native materials and attachments. Retiring the old
/// callbacks first prevents a removed decal from repainting the new body.
pub(super) fn reset_loadout(p: &UpkPackage, base: usize) -> R<Script> {
    let ready = settings_ready(p)?;
    guard(base, ready.clone(), reset_loadout_ready(p, base + 3 + ready.mem_size())?)
}
fn reset_loadout_ready(p: &UpkPackage, base: usize) -> R<Script> {
    let mut prefix = set(p, "HbxPaintMesh", call(p, "Core.Object.PathName", &[raw(&[0x17])])?)?;
    let base = base + prefix.mem_size();
    let local = raw(&[0x2d]).append(&var(p, 0x01, "CarMeshComponentBase_TA.bLocalPlayer")?).clone();
    let ready = choose(settings_ready(p)?, choose(local, not_null(bucket(p)?), raw(&[0x28]))?, raw(&[0x28]))?;
    let refs = references(p)?;
    let start = base + 3 + ready.mem_size();
    let mut clear = set(p, "HbxLoadoutResetCount", cast(INT_TO_STRING, length(refs.clone())))?;
    clear.append(&super::boost::repeat(p, start + clear.mem_size(), "HbxLoadoutResetIndex", length(refs.clone()), |base, index| {
        let entry = element_at(index, refs.clone());
        let snapshot = dynamic(p, "Engine.ObjectReferencer", entry.clone())?;
        let target = field(p, snapshot.clone(), "Core.Object.Outer")?;
        let actor = actor_of(p, target.clone())?;
        let owner = var(p, 0x01, "Engine.ActorComponent.Owner")?;
        let actor_owned = choose(not_null(actor.clone()), choose(object_eq(&[actor.clone(), owner.clone()]), raw(&[0x27]),
            object_eq(&[field(p, actor, "Engine.Actor.Owner")?, owner]))?, raw(&[0x28]))?;
        let belongs = choose(object_eq(&[target.clone(), raw(&[0x17])]), raw(&[0x27]), actor_owned)?;
        let owns_snapshot = choose(not_null(snapshot.clone()), belongs, raw(&[0x28]))?;
        let callback = dynamic(p, PAINT_CLOSURE, entry.clone())?;
        let owns_callback = choose(not_null(callback.clone()), object_eq(&[field(p, callback, "Core.Object.Outer")?, raw(&[0x17])]), raw(&[0x28]))?;
        let saved = field(p, snapshot, "Engine.ObjectReferencer.ReferencedObjects")?;
        let mesh = dynamic(p, "Engine.MeshComponent", target)?;
        let has_materials = choose(not_null(mesh.clone()), native(0x97, &[length(saved.clone()), int(1)]), raw(&[0x28]))?;
        let restore_base = base + 3 + owns_snapshot.mem_size();
        let restore = restore_attachment_snapshot(p, restore_base + 3 + has_materials.mem_size(), mesh, saved.clone())?;
        let mut restore = guard(restore_base, has_materials, restore)?;
        restore.append(&assign(length(saved), int(0)));
        let mut clear = guard(base, owns_snapshot.clone(), restore)?;
        let owned = choose(owns_snapshot, raw(&[0x27]), owns_callback)?;
        clear.append(&guard(base + clear.mem_size(), owned, assign(entry, raw(&[0x2a])))?);
        Ok(clear)
    })?);
    // Keep the per-tick scan proportional to current equipment rather than
    // the highest number of objects seen during the entire game session.
    clear.append(&super::boost::repeat(p, start + clear.mem_size(), "HbxLoadoutTrimIndex", number(p, "HbxLoadoutResetCount")?, |base, _| {
        let last = element_at(native(0x93, &[length(refs.clone()), int(1)]), refs.clone());
        let removable = choose(native(0x97, &[length(refs.clone()), int(1)]), object_eq(&[last, raw(&[0x2a])]), raw(&[0x28]))?;
        guard(base, removable, assign(length(refs.clone()), native(0x93, &[length(refs.clone()), int(1)])))
    })?);
    clear.append(&set(p, "HbxLoadoutGeneration", cast(INT_TO_STRING, native(0x92, &[number(p, "HbxLoadoutGeneration")?, int(1)])))?);
    prefix.append(&guard(base, ready, clear)?);
    Ok(prefix)
}

fn vanity_prefix(p: &UpkPackage, target: Script) -> R<Script> {
    let id = field(p, target.clone(), "PlayerVanity_TA.ProductID")?;
    let instance = field(p, target.clone(), "PlayerVanity_TA.InstanceID")?;
    Ok(join(join(join(join(text("HbxVanityOriginal."), cast(0x56, target)), text(".")), cast(INT_TO_STRING, id)), instance))
}
// UI vanity assets and one-shot explosions rebuild their layers/state on
// paint application. Unlike car materials, solid colours need no refresh.
pub(super) fn vanity_stamp(p: &UpkPackage, key: &str) -> R<Script> {
    // The validated settings cache changes only when the file changes. Read
    // it once rather than expanding five provider lookups into both tick hooks.
    // Changes to another setting can cause one harmless extra update.
    let mut stamp = get(p, "HbxLiveCache")?;
    // Four expensive UI/one-shot FX updates per second for rainbow, independent
    // of the 10 Hz continuous car-material clock.
    if key == "goal_explosion" { return Ok(stamp); }
    let phase = cast(INT_TO_STRING, cast(0x44, native(0xab, &[
        cast(STRING_TO_FLOAT, get(p, "HbxRainbowTime")?), float(4.)])));
    stamp = join(join(stamp, text("|")), choose(flag(p, &format!("{key}.rainbow"))?, phase, text(""))?);
    Ok(stamp)
}
pub(super) fn refresh_vanity(p: &UpkPackage, base: usize) -> R<Script> {
    let mut out = set(p, "HbxVanityLocalPass", text("1"))?;
    for (field_name, key) in [("PlayerBanner", "banner"), ("PlayerBorder", "avatar_border")] {
        let target = var(p, 0x01, &format!("PlayerControllerBase_TA.{field_name}"))?;
        let exists = not_null(target.clone());
        let start = base + out.mem_size() + 3 + exists.mem_size();
        let mut update = Script::new();
        let paintable = field(p, target.clone(), "PlayerVanity_TA.PaintableLayer")?;
        let tintable = field(p, target.clone(), "PlayerVanity_TA.TintableLayer")?;
        let colourable = bool_field(p, target.clone(), "PlayerVanity_TA.bColorable")?;
        let supports = choose(not_null(paintable), raw(&[0x27]), choose(not_null(tintable), raw(&[0x27]), colourable)?)?;
        update.append(&set(p, &format!("HbxNativeFound.{key}"), choose(supports, text("1"), text("0"))?)?);
        let marker = join(vanity_prefix(p, target.clone())?, text(".saved"));
        let previous = native(0xcc, &[get_dynamic(p, marker)?, text("1")]);
        let enabled = flag(p, &format!("{key}.enabled"))?;
        update.append(&set(p, &format!("HbxNativeApplied.{key}"),
            choose(enabled.clone(), choose(previous.clone(), text("1"), text("0"))?, text("0"))?)?);
        let stamp_key = join(vanity_prefix(p, target.clone())?, text(".lastApplied"));
        let changed = native(0xc0, &[vanity_stamp(p, key)?, get_dynamic(p, stamp_key.clone())?]);
        let active = choose(enabled, changed, previous)?;
        let colour = field(p, target.clone(), "PlayerVanity_TA.ColorValue")?;
        let mut refresh = context(p, target.clone(), call(p, "PlayerVanity_TA.ApplyColor", &[colour])?, None)?;
        // Commit only after the scoped update; native changes outside this
        // pass invalidate it in the ApplyColor hook.
        refresh.append(&set_dynamic(p, stamp_key, vanity_stamp(p, key)?)?);
        update.append(&guard(start + update.mem_size(), active, refresh)?);
        out.append(&guard(base + out.mem_size(), exists, update)?);
    }
    out.append(&set(p, "HbxVanityLocalPass", text("0"))?);
    Ok(out)
}
pub(super) fn vanity(p: &UpkPackage, base: usize) -> R<Script> {
    let local = native(0xcc, &[get(p, "HbxVanityLocalPass")?, text("1")]);
    let start = base + 3 + local.mem_size();
    let mut body = Script::new();
    for (class, key) in [("PlayerBanner_TA", "banner"), ("PlayerAvatarBorder_TA", "avatar_border")] {
        let matches = native(0xce, &[dynamic(p, class, raw(&[0x17]))?, raw(&[0x2a])]);
        let at = start + body.mem_size() + 3 + matches.mem_size();
        let mut part = set(p, "HbxVanityOriginalKey", vanity_prefix(p, raw(&[0x17]))?)?;
        let prefix = get(p, "HbxVanityOriginalKey")?;
        let marker = join(prefix.clone(), text(".saved"));
        let cached = native(0xcc, &[get_dynamic(p, marker.clone())?, text("1")]);
        let active = flag(p, &format!("{key}.enabled"))?;
        let mut capture = Script::new();
        for field in ["ColorValue", "PaintColor"] {
            for channel in ["R", "G", "B", "A"] {
                let value = member(p, "Core.Object.Color", channel, var(p, 0x01, &format!("PlayerVanity_TA.{field}"))?, false)?;
                capture.append(&set_dynamic(p, join(prefix.clone(), text(&format!(".{field}.{channel}"))), cast(FLOAT_TO_STRING, cast(0x3c, value)))?);
            }
        }
        capture.append(&set_dynamic(p, join(prefix.clone(), text(".ColorID")), cast(INT_TO_STRING, var(p, 0x01, "PlayerVanity_TA.ColorID")?))?);
        capture.append(&set_dynamic(p, marker.clone(), text("1"))?);
        let mut apply = guard(at + part.mem_size() + 3 + active.mem_size(), native(0x81, &[cached.clone()]), capture)?;
        let rgb = call(p, "Core.Object.LinearColorToColor", &[paint_rgb(p, key)?])?;
        apply.append(&assign(var(p, 0x01, "PlayerVanity_TA.ColorValue")?, rgb.clone()));
        apply.append(&assign(var(p, 0x01, "PlayerVanity_TA.PaintColor")?, rgb));
        apply.append(&set_bool(p, raw(&[0x17]), "PlayerVanity_TA.bIsDirty", true)?);
        apply.append(&set(p, &format!("HbxNativeApplied.{key}"), text("1"))?);
        part.append(&guard(at + part.mem_size(), active.clone(), apply)?);
        let disabled = native(0x81, &[active]);
        let mut restore = Script::new();
        for field in ["ColorValue", "PaintColor"] {
            for channel in ["R", "G", "B", "A"] {
                let destination = member(p, "Core.Object.Color", channel, var(p, 0x01, &format!("PlayerVanity_TA.{field}"))?, true)?;
                restore.append(&assign(destination, cast(0x43, cast(STRING_TO_FLOAT, get_dynamic(p, join(prefix.clone(), text(&format!(".{field}.{channel}"))))?))));
            }
        }
        restore.append(&assign(var(p, 0x01, "PlayerVanity_TA.ColorID")?, cast(0x44, cast(STRING_TO_FLOAT, get_dynamic(p, join(prefix.clone(), text(".ColorID")))?))));
        restore.append(&set_dynamic(p, marker, text("0"))?);
        restore.append(&set_bool(p, raw(&[0x17]), "PlayerVanity_TA.bIsDirty", true)?);
        let cached_base = at + part.mem_size() + 3 + disabled.mem_size();
        part.append(&guard(at + part.mem_size(), disabled, guard(cached_base, cached, restore)?)?);
        body.append(&guard(start + body.mem_size(), matches, part)?);
    }
    let mut out = guard(base, local.clone(), body)?;
    // Native changes on the same vanity instance must invalidate the stamp.
    // Keep this here instead of serialising its colour in every controller tick.
    let external = native(0x81, &[local]);
    let key = join(vanity_prefix(p, raw(&[0x17]))?, text(".lastApplied"));
    out.append(&guard(base + out.mem_size(), external, set_dynamic(p, key, text(""))?)?);
    Ok(out)
}

pub(super) fn diagnostics(p: &UpkPackage, base: usize, time: Script, report_path: Script) -> R<Script> {
    let next = cast(STRING_TO_FLOAT, get(p, "HbxDiagnosticNext")?);
    let reset = native(0xb0, &[time.clone(), native(0xaf, &[next.clone(), float(2.)])]);
    let mut out = guard(base, reset, set(p, "HbxDiagnosticNext", text("0"))?)?;
    let due = native(0xb1, &[time.clone(), cast(STRING_TO_FLOAT, get(p, "HbxDiagnosticNext")?)]);
    let at = base + out.mem_size() + 3 + due.mem_size();
    let mut write = ensure_holder(p, at)?;
    write.append(&prune_buckets(p, at + write.mem_size())?);
    write.append(&set(p, "HbxDiagnosticNext", cast(FLOAT_TO_STRING, native(0xae, &[time, float(1.)])))?);
    let mut lines = vec![text(&format!("HbxLiveRuntime.version={}\n", REVISION))];
    lines.push(join(text("session="), join(control_session(p)?, text("\n"))));
    let registry = choose(not_null(bucket(p)?), length(references(p)?), int(0))?;
    lines.push(join(text("registry.entries="), join(cast(INT_TO_STRING, registry), text("\n"))));
    let meshes = choose(not_null(holder(p)?), native(0x93, &[length(global_references(p)?), int(1)]), int(0))?;
    lines.push(join(text("registry.meshes="), join(cast(INT_TO_STRING, meshes), text("\n"))));
    lines.push(join(text("paint.refreshes="), join(get(p, "HbxNativeRefreshPasses")?, text("\n"))));
    for key in ["trim", "decal", "wheels", "boost", "antenna", "topper", "trail", "goal_explosion", "banner", "avatar_border"] {
        for (suffix, source) in [("found", "HbxNativeFound"), ("applied", "HbxNativeApplied")] {
            lines.push(join(text(&format!("{key}.{suffix}=")), join(get(p, &format!("{source}.{key}"))?, text("\n"))));
        }
    }
    lines.push(text("HbxLiveRuntime.end=1\n"));
    // A balanced expression avoids excessive VM recursion as categories grow.
    while lines.len() > 1 {
        lines = lines.chunks(2).map(|pair| if pair.len() == 2 { join(pair[0].clone(), pair[1].clone()) } else { pair[0].clone() }).collect();
    }
    let report = lines.pop().unwrap();
    let save = static_call(p, "Core.FileSystem", call(p, "Core.FileSystem.SaveStringToFile", &[report_path, report])?, None)?;
    let mut discard = raw(&[0x0e]);
    discard.object(obj(p, "Core.FileSystem.SaveStringToFile.ReturnValue")?).append(&save);
    write.append(&discard);
    out.append(&guard(base + out.mem_size(), due, write)?);
    Ok(out)
}

/// GC roots must be pruned even when optional file diagnostics are disabled.
pub(super) fn maintenance(p: &UpkPackage, base: usize, time: Script) -> R<Script> {
    let next = cast(STRING_TO_FLOAT, get(p, "HbxRegistryMaintenanceNext")?);
    let mut out = guard(base, native(0xb0, &[time.clone(), native(0xaf, &[next.clone(), float(2.)])]),
        set(p, "HbxRegistryMaintenanceNext", text("0"))?)?;
    let due = native(0xb1, &[time.clone(), next]);
    let start = base + out.mem_size() + 3 + due.mem_size();
    let mut prune = set(p, "HbxRegistryMaintenanceNext", cast(FLOAT_TO_STRING, native(0xae, &[time, float(2.)])))?;
    prune.append(&ensure_holder(p, start + prune.mem_size())?);
    prune.append(&prune_buckets(p, start + prune.mem_size())?);
    out.append(&guard(base + out.mem_size(), due, prune)?);
    Ok(out)
}

pub(super) fn prepare_imports(p: &mut UpkPackage) -> R<Vec<(usize, Vec<u8>, Vec<u8>)>> {
    let mut edits = Vec::new();
    // These are script classes with fixed reflected layouts. Enforce their
    // construction constraints instead of weakening shared UClass metadata.
    for (class, within) in [(PAINT_CLOSURE, "CarMeshComponentBase_TA"), ("ProductPaint_TA", "Core.Object")] {
        let index = p.find_object(class).filter(|index| *index > 0).ok_or_else(|| format!("Missing native paint class {class}"))? as usize - 1;
        let export = &p.exports[index];
        let bytes = &p.image[export.serial_offset..export.serial_offset + export.serial_size];
        let expected = p.find_object(within).ok_or_else(|| format!("Missing native paint outer {within}"))?;
        if bytes.len() < 78 || i32::from_le_bytes(bytes[74..78].try_into().unwrap()) != expected {
            return Err(format!("{class} construction layout changed; live item colours left the package untouched"));
        }
    }
    for (parent, kind, name) in [
        ("Engine", "Class", "ObjectReferencer"),
        ("Engine", "Class", "GameEngine"),
        ("Engine.GameEngine", "ArrayProperty", "ObjectReferencers"),
        ("Engine.ObjectReferencer", "ArrayProperty", "ReferencedObjects"),
        ("Core.Object", "ObjectProperty", "Outer"),
        ("ProjectX.FXActor_X", "ObjectProperty", "AttachmentActor"),
        ("Engine.ActorComponent", "ObjectProperty", "Owner"),
        ("Engine.Actor", "BoolProperty", "bDeleteMe"),
        ("Core.Object", "Function", "IsPendingKill"),
        ("Core.Object", "Function", "TimeStamp"),
        ("Core.Object.TimeStamp", "StrProperty", "ReturnValue"),
        ("Engine.MeshComponent", "Function", "GetNumElements"),
        ("Engine", "Class", "MaterialInstanceConstant"),
        ("Engine.MaterialInstanceConstant", "Function", "SetParent"),
        ("Engine.MaterialInstanceConstant", "Function", "GetVectorParameterValue"),
        ("Engine.MaterialInstanceConstant.GetVectorParameterValue", "BoolProperty", "ReturnValue"),
        ("Engine.MaterialInstanceConstant", "Function", "SetVectorParameterValue"),
        ("Engine", "Class", "MaterialInstance"),
        ("Engine.MaterialInstance", "ObjectProperty", "Parent"),
        ("Engine.MaterialInstanceConstant", "ArrayProperty", "VectorParameterValues"),
        ("Engine.MaterialInstanceConstant", "ArrayProperty", "ScalarParameterValues"),
        ("Engine.MaterialInstanceConstant", "ArrayProperty", "TextureParameterValues"),
        ("Engine.MaterialInstanceConstant", "ArrayProperty", "FontParameterValues"),
        ("Engine.MeshComponent.CreateAndSetMaterialInstanceConstant", "ObjectProperty", "ReturnValue"),
        ("Engine.MeshComponent.GetNumElements", "IntProperty", "ReturnValue"),
        ("Engine.MeshComponent", "Function", "CreateAndSetMaterialInstanceConstant"),
        ("Engine.MeshComponent", "Function", "SetMaterial"),
        ("ProjectX.ParameterDispenser_X", "ArrayProperty", "FloatParams"),
        ("ProjectX.FXActor_X", "Function", "IsStateActive"),
        ("ProjectX.FXActor_X.IsStateActive", "BoolProperty", "ReturnValue"),
        ("ProjectX.FXActor_X", "Function", "RemoveStateForceUpdate"),
    ] {
        if p.find_object(&format!("{parent}.{name}")).is_some() { continue; }
        let outer = p.find_object(parent).ok_or_else(|| format!("Missing native colour lifetime type {parent}"))?;
        let slot = p.empty_import_slots().into_iter().next().ok_or("No native colour lifetime import slot")?;
        let (before, after) = p.bind_native_field(slot, outer, kind, name)?;
        edits.push((slot, before, after));
    }
    for channel in ["R", "G", "B", "A"] {
        if p.find_object(&format!("Core.Object.Color.{channel}")).is_some() { continue; }
        let parent = p.find_object("Core.Object.Color").ok_or("Missing native Color struct")?;
        let slot = p.empty_import_slots().into_iter().next().ok_or("No native UI colour import slot")?;
        let (before, after) = p.bind_native_field(slot, parent, "ByteProperty", channel)?;
        edits.push((slot, before, after));
    }
    for (parent, kind, name) in [
        ("Core.Object", "Function", "FindObject"),
        ("Core.Object", "Function", "PathName"),
        ("Core.FileSystem", "Function", "SaveStringToFile"),
        ("Core.FileSystem.SaveStringToFile", "BoolProperty", "ReturnValue"),
    ] {
        if p.find_object(&format!("{parent}.{name}")).is_some() { continue; }
        let outer = p.find_object(parent).ok_or_else(|| format!("Missing colour diagnostic import {parent}"))?;
        let slot = p.empty_import_slots().into_iter().next().ok_or("No native paint function import slot")?;
        let (before, after) = p.bind_native_field(slot, outer, kind, name)?;
        edits.push((slot, before, after));
    }
    Ok(edits)
}

pub(super) fn goal(p: &UpkPackage, base: usize) -> R<Script> {
    let scorer = var(p, 0x2b, "Ball_TA.InitExplosionFX.Scorer")?;
    let local = context(p, scorer.clone(), call(p, "PRI_TA.IsLocalPlayerPRI", &[])?, Some("PRI_TA.IsLocalPlayerPRI.ReturnValue"))?;
    let fx = field(p, var(p, 0x2b, "Ball_TA.InitExplosionFX.InExplosion")?, "ProjectX.Explosion_X.FXActor")?;
    let car = field(p, scorer, "PRI_TA.Car")?;
    let mesh = field(p, car, "Vehicle_TA.CarMesh")?;
    let valid = choose(not_null(fx.clone()), not_null(mesh.clone()), raw(&[0x28]))?;
    let start = base + 3 + local.mem_size() + 3 + valid.mem_size();
    let mut save = set_dynamic(p, join(text("HbxGoalOwner."), cast(0x56, fx.clone())), cast(0x56, mesh.clone()))?;
    save.append(&set(p, "HbxNativeForcedOwner", call(p, "Core.Object.PathName", &[mesh])?)?);
    let data = var(p, 0x2b, "Ball_TA.InitExplosionFX.OnlineProduct")?;
    let attributes = member(p, "_Types_TA.ClientLoadoutOnlineProduct", "Attributes", data, false)?;
    let mut count = raw(&[0x57, 1]); count.append(&attributes);
    save.append(&set(p, "HbxGoalHasPaint", text("0"))?);
    save.append(&super::boost::repeat(p, start + save.mem_size(), "HbxGoalAttributeIndex", count, |base, index| {
        let attribute = dynamic(p, "ProductAttribute_Painted_TA", element_at(index, attributes))?;
        let asset = var(p, 0x2b, "Ball_TA.InitExplosionFX.PlayerGoalExplosion")?;
        let mut apply = set(p, "HbxGoalHasPaint", text("1"))?;
        apply.append(&context(p, attribute.clone(), call(p, "ProductAttribute_Painted_TA.ApplyToObject", &[asset, fx.clone()])?, None)?);
        guard(base, not_null(attribute), apply)
    })?);
    let asset = var(p, 0x2b, "Ball_TA.InitExplosionFX.PlayerGoalExplosion")?;
    let unpainted = native(0xcc, &[get(p, "HbxGoalHasPaint")?, text("0")]);
    let at = start + save.mem_size();
    save.append(&guard(at, unpainted.clone(), capture_arguments(p, at + 3 + unpainted.mem_size(), asset.clone(), fx, asset_settings(p, asset)?, raw(&[0x2a]))?)?);
    save.append(&set(p, "HbxNativeForcedOwner", text(""))?);
    guard(base, local.clone(), guard(base + 3 + local.mem_size(), valid, save)?)
}

pub(super) fn goal_preview(p: &UpkPackage, base: usize) -> R<Script> {
    let owner = "ExplosionPreviewer_TA.ApplyExplosionParameters";
    let fx = var(p, 0x2b, &format!("{owner}.Instance"))?;
    let product = var(p, 0x2b, &format!("{owner}.OnlineProduct"))?;
    let pc = var(p, 0x01, "ExplosionPreviewer_TA.PC")?;
    let mesh = field(p, preview_actor_for(p, pc.clone())?, "CarPreviewActor_TA.CarMesh")?;
    // Stock/default explosions may have no OnlineProduct inventory instance.
    let valid = choose(not_null(pc), choose(not_null(mesh.clone()), not_null(fx.clone()), raw(&[0x28]))?, raw(&[0x28]))?;
    let start = base + 3 + valid.mem_size();
    let mut out = set(p, "HbxNativeForcedOwner", call(p, "Core.Object.PathName", &[mesh.clone()])?)?;
    let attributes = field(p, product.clone(), "OnlineProduct_TA.Attributes")?;
    out.append(&set(p, "HbxPreviewHasPaint", text("0"))?);
    let present = not_null(product);
    let at = start + out.mem_size();
    out.append(&guard(at, present.clone(), super::boost::repeat(p, at + 3 + present.mem_size(), "HbxPreviewAttributeIndex", length(attributes.clone()), |base, index| {
        let attribute = dynamic(p, "ProductAttribute_Painted_TA", element_at(index, attributes))?;
        let asset = var(p, 0x01, "ExplosionPreviewer_TA.ExplosionProduct")?;
        let mut apply = set(p, "HbxPreviewHasPaint", text("1"))?;
        apply.append(&context(p, attribute.clone(), call(p, "ProductAttribute_Painted_TA.ApplyToObject", &[asset, fx.clone()])?, None)?);
        guard(base, not_null(attribute), apply)
    })?)?);
    let asset = var(p, 0x01, "ExplosionPreviewer_TA.ExplosionProduct")?;
    let unpainted = native(0xcc, &[get(p, "HbxPreviewHasPaint")?, text("0")]);
    let at = start + out.mem_size();
    out.append(&guard(at, unpainted.clone(), capture_arguments(p, at + 3 + unpainted.mem_size(), asset.clone(), fx.clone(), asset_settings(p, asset.clone())?, raw(&[0x2a]))?)?);
    // A preview can finish before the next controller refresh. Apply while
    // its FX instance is still alive instead of waiting for the registry pass.
    out.append(&context(p, mesh, call(p, "CarMeshComponentBase_TA.ApplyPaintSettingsToObject", &[
        raw(&[0x2a]), asset_settings(p, asset.clone())?, fx, asset])?, None)?);
    out.append(&set(p, "HbxNativeForcedOwner", text(""))?);
    guard(base, valid, out)
}

/// Capture after the mesh's native attribute pass, using its known owner rather
/// than guessing from FX attachments that may not have been connected yet.
pub(super) fn attributes(p: &UpkPackage, base: usize) -> R<Script> {
    let owner = "CarMeshComponentBase_TA.ApplyAttributes";
    let asset = var(p, 0x2b, &format!("{owner}.Asset"))?;
    let target = var(p, 0x2b, &format!("{owner}.Target"))?;
    let list = var(p, 0x2b, &format!("{owner}.Attributes"))?;
    let idle = choose(settings_ready(p)?, native(0xc0, &[get(p, "HbxCaptureAttributesBusy")?, text("1")]), raw(&[0x28]))?;
    let start = base + 3 + idle.mem_size();
    let mut out = set(p, "HbxCaptureAttributesBusy", text("1"))?;
    out.append(&set(p, "HbxNativeForcedOwner", call(p, "Core.Object.PathName", &[raw(&[0x17])])?)?);
    out.append(&set(p, "HbxCaptureHasPaint", text("0"))?);
    out.append(&super::boost::repeat(p, start + out.mem_size(), "HbxCaptureAttributeIndex", length(list.clone()), |base, index| {
        let attribute = dynamic(p, "ProductAttribute_Painted_TA", element_at(index, list))?;
        let mut apply = set(p, "HbxCaptureHasPaint", text("1"))?;
        apply.append(&context(p, attribute.clone(), call(p, "ProductAttribute_Painted_TA.ApplyToObject", &[asset.clone(), target.clone()])?, None)?);
        guard(base, not_null(attribute), apply)
    })?);
    // Empty Attributes is normal for stock and unpainted products. Register
    // those too; waiting for a Painted attribute silently excludes these items.
    let unpainted = native(0xcc, &[get(p, "HbxCaptureHasPaint")?, text("0")]);
    let at = start + out.mem_size();
    out.append(&guard(at, unpainted.clone(), capture_arguments(p, at + 3 + unpainted.mem_size(), asset.clone(), target, asset_settings(p, asset)?, raw(&[0x2a]))?)?);
    out.append(&set(p, "HbxNativeForcedOwner", text(""))?);
    out.append(&set(p, "HbxCaptureAttributesBusy", text("0"))?);
    guard(base, idle, out)
}
