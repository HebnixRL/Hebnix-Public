//! Bind existing, engine-created MICs once; animation never rebuilds paints.
use super::*;
use super::items::{choose, fixed_element, global_references, ensure_holder};

const CHANNELS: &[&str] = &["primary", "secondary", "trim", "decal", "wheels", "antenna", "topper", "boost", "trail"];
fn join(a: Script, b: Script) -> Script { native(0xf8, &[a, b]) }
fn length(array: Script) -> Script { let mut s = raw(&[0x57, 1]); s.append(&array); s }
fn at(index: Script, array: Script) -> Script { let mut s = raw(&[0x57, 0, 0]); s.append(&index).append(&array); s }
fn fixed(index: Script, array: Script) -> Script { let mut s = raw(&[0x1a]); s.append(&index).append(&array); s }
fn nonnull(s: Script) -> Script { native(0xce, &[s, raw(&[0x2a])]) }
fn registry(p: &UpkPackage, mesh: Script) -> R<Script> {
    dynamic(p, "Engine.ObjectReferencer", call(p, "Core.Object.FindObject", &[
        join(call(p, "Core.Object.PathName", &[mesh])?, text(".HbxLiveAnimation")), var(p, 0x20, "Engine.ObjectReferencer")?])?)
}
fn refs(p: &UpkPackage, mesh: Script) -> R<Script> { field(p, registry(p, mesh)?, "Engine.ObjectReferencer.ReferencedObjects") }
fn current_mesh(p: &UpkPackage) -> R<Script> {
    dynamic(p, "CarMeshComponentBase_TA", call(p, "Core.Object.FindObject", &[get(p, "HbxAnimationMesh")?, var(p, 0x20, "CarMeshComponentBase_TA")?])?)
}
fn ensure(p: &UpkPackage, base: usize, mesh: Script) -> R<Script> {
    let mut out = ensure_holder(p, base)?;
    let missing = native(0x81, &[nonnull(registry(p, mesh.clone())?)]);
    let roots = global_references(p)?;
    let mut create = assign(length(roots.clone()), native(0x92, &[length(roots.clone()), int(1)]));
    let mut object = raw(&[0x11]);
    object.append(&mesh).append(&text("HbxLiveAnimation")).append(&int(0)).append(&var(p, 0x20, "Engine.ObjectReferencer")?).raw(&[0x2a]);
    create.append(&assign(at(native(0x93, &[length(roots.clone()), int(1)]), roots), object));
    let array = refs(p, mesh.clone())?;
    create.append(&assign(length(array.clone()), int(1)));
    let mut data = raw(&[0x11]);
    data.append(&registry(p, mesh)?).append(&text("ColourFrame")).append(&int(0)).append(&var(p, 0x20, "ProductPaint_TA")?).raw(&[0x2a]);
    create.append(&assign(at(int(0), array), data));
    out.append(&guard(base + out.mem_size(), missing, create)?);
    Ok(out)
}
/// Dispatch through Get to keep native 16-bit function addresses bounded.
pub(super) fn bind(p: &UpkPackage, mesh: Script, material: Script, domain: Script, parameter: Script, full: Option<&str>) -> R<Script> {
    let mut out = set(p, "HbxAnimationMesh", call(p, "Core.Object.PathName", &[mesh])?)?;
    out.append(&set(p, "HbxAnimationMaterial", call(p, "Core.Object.PathName", &[material])?)?);
    out.append(&set(p, "HbxAnimationDomain", domain)?);
    out.append(&set(p, "HbxAnimationParameter", cast(0x57, parameter))?);
    out.append(&set(p, "HbxAnimationFull", text(full.unwrap_or("")))?);
    let mut discard = raw(&[0x0e]);
    discard.object(obj(p, "UISavedValues_TA.Get.ReturnValue")?).append(&get(p, "HbxAnimation.bind")?);
    out.append(&discard);
    Ok(out)
}
pub(super) fn register(p: &UpkPackage, base: usize) -> R<Script> {
    let mesh = current_mesh(p)?;
    let material = dynamic(p, "Engine.MaterialInstanceConstant", call(p, "Core.Object.FindObject", &[get(p, "HbxAnimationMaterial")?, var(p, 0x20, "Engine.MaterialInstanceConstant")?])?)?;
    let valid = choose(nonnull(mesh.clone()), nonnull(material.clone()), raw(&[0x28]))?;
    let start = base + 3 + valid.mem_size();
    let mut out = ensure(p, start, mesh.clone())?;
    let array = refs(p, mesh)?;
    let domain = get(p, "HbxAnimationDomain")?;
    let parameter = get(p, "HbxAnimationParameter")?;
    let name = join(join(join(text("HbxColourBinding_"), domain.clone()), text("_")), parameter.clone());
    let path = join(join(call(p, "Core.Object.PathName", &[material.clone()])?, text(".")), name.clone());
    let data = dynamic(p, "ProductPaint_TA", call(p, "Core.Object.FindObject", &[path, var(p, 0x20, "ProductPaint_TA")?])?)?;
    let found = nonnull(data.clone());
    let slot = cast(0x44, field(p, data.clone(), "ProductPaint_TA.EmissiveAmount")?);
    let present = choose(found.clone(), choose(native(0x97, &[slot.clone(), int(0)]),
        choose(native(0x96, &[slot.clone(), length(array.clone())]), object_eq(&[at(slot.clone(), array.clone()), data.clone()]), raw(&[0x28]))?, raw(&[0x28]))?, raw(&[0x28]))?;
    let missing = choose(native(0x96, &[length(array.clone()), int(513)]), native(0x81, &[present]), raw(&[0x28]))?;
    let mut object = raw(&[0x11]);
    object.append(&material.clone()).append(&name).append(&int(0)).append(&var(p, 0x20, "ProductPaint_TA")?).raw(&[0x2a]);
    // FindObject expressions observe this object immediately after construction.
    let mut append = assign(length(array.clone()), native(0x92, &[length(array.clone()), int(2)]));
    append.append(&assign(at(native(0x93, &[length(array.clone()), int(2)]), array.clone()), material));
    append.append(&assign(at(native(0x93, &[length(array.clone()), int(1)]), array.clone()), choose(found, data.clone(), object)?));
    append.append(&assign(field(p, data.clone(), "ProductPaint_TA.EmissiveAmount")?, cast(0x3f, native(0x93, &[length(array), int(1)]))));
    append.append(&assign(field(p, data.clone(), "ProductPaint_TA.EmissiveParameterName")?, cast(0x60, parameter)));
    let mut channel = int(-1);
    for (i, key) in CHANNELS.iter().enumerate() { channel = choose(native(0xcc, &[domain.clone(), text(key)]), int(i as i32), channel)?; }
    append.append(&assign(field(p, data.clone(), "ProductPaint_TA.FinishType")?, cast(0x3d, channel)));
    append.append(&assign(field(p, data, "ProductPaint_TA.Label")?, get(p, "HbxAnimationFull")?));
    out.append(&guard(start + out.mem_size(), missing, append)?);
    guard(base, valid, out)
}
pub(super) fn clear(p: &UpkPackage, base: usize) -> R<Script> {
    let mesh = raw(&[0x17]);
    guard(base, nonnull(registry(p, mesh.clone())?), assign(length(refs(p, mesh)?), int(1)))
}
pub(super) fn update(p: &UpkPackage, base: usize) -> R<Script> {
    let mesh = current_mesh(p)?;
    let available = nonnull(registry(p, mesh.clone())?);
    let start = base + 3 + available.mem_size();
    let array = refs(p, mesh)?;
    let frame = dynamic(p, "ProductPaint_TA", at(int(0), array.clone()))?;
    let colours = field(p, frame, "ProductPaint_TA.Colors")?;
    let mut out = Script::new();
    // All channels use the same gradient. Calculate three sines once per
    // frame, then share this value across every active channel/material.
    let rainbow = fixed_element(11, colours.clone());
    out.append(&assign(rainbow.clone(), call(p, "Core.Object.MakeLinearColor", &[
        rainbow_channel(p, 0)?, rainbow_channel(p, 1)?, rainbow_channel(p, 2)?, float(1.)])?));
    for (i, key) in CHANNELS.iter().enumerate() {
        let active = choose(flag(p, &format!("{key}.enabled"))?, flag(p, &format!("{key}.rainbow"))?, raw(&[0x28]))?;
        let colour = fixed_element(i as i32, colours.clone());
        // A=0 is an inactive binding; read flags once per channel, not per MIC.
        out.append(&assign(member(p, "Core.Object.LinearColor", "A", colour.clone(), true)?, float(0.)));
        out.append(&guard(start + out.mem_size(), active, assign(colour, rainbow.clone()))?);
    }
    let count = native(0x95, &[native(0x93, &[length(array.clone()), int(1)]), int(2)]);
    out.append(&boost::repeat(p, start + out.mem_size(), "HbxAnimationIndex", count, |base, index| {
        let material = dynamic(p, "Engine.MaterialInstanceConstant", at(native(0x92, &[native(0x94, &[index.clone(), int(2)]), int(1)]), array.clone()))?;
        let data = dynamic(p, "ProductPaint_TA", at(native(0x92, &[native(0x94, &[index, int(2)]), int(2)]), array.clone()))?;
        let channel = cast(0x3a, field(p, data.clone(), "ProductPaint_TA.FinishType")?);
        let colour = fixed(channel.clone(), colours);
        let active = choose(nonnull(material.clone()), choose(nonnull(data.clone()),
            choose(native(0x96, &[channel, int(CHANNELS.len() as i32)]), native(0xb1, &[member(p, "Core.Object.LinearColor", "A", colour.clone(), false)?, float(0.5)]), raw(&[0x28]))?, raw(&[0x28]))?, raw(&[0x28]))?;
        let full = field(p, data.clone(), "ProductPaint_TA.Label")?;
        let parameter = field(p, data, "ProductPaint_TA.EmissiveParameterName")?;
        let paired = native(0xc0, &[full.clone(), text("")]);
        let start = base + 3 + active.mem_size();
        // Keep the game's normal correction/full-bright calculation for car
        // colours. This helper changes two vectors; it does not rebuild a MIC.
        let mut apply = guard(start, paired.clone(), context(p, current_mesh(p)?, call(p, "CarMeshComponentBase_TA.SetMaterialColorParams", &[
            material.clone(), colour.clone(), parameter.clone(), cast(0x60, full)])?, None)?)?;
        apply.append(&guard(start + apply.mem_size(), native(0x81, &[paired]), context(p, material,
            call(p, "Engine.MaterialInstanceConstant.SetVectorParameterValue", &[parameter, colour])?, None)?)?);
        guard(base, active, apply)
    })?);
    guard(base, available, out)
}
pub(super) fn capture_native(p: &UpkPackage, base: usize) -> R<Script> {
    let owner = "ProductAttribute_Painted_TA.OverrideMeshMaterial";
    let material = var(p, 0x2b, &format!("{owner}.MIC"))?;
    let paint = var(p, 0x2b, &format!("{owner}.Paint"))?;
    let settings = var(p, 0x2b, &format!("{owner}.PaintSettings"))?;
    let private = native(0x9b, &[virtual_call(p, "InStr", &[cast(0x56, paint), text(".HbxLivePaint_")])?, int(-1)]);
    let valid = choose(private, nonnull(material.clone()), raw(&[0x28]))?;
    let mesh = dynamic(p, "CarMeshComponentBase_TA", call(p, "Core.Object.FindObject", &[get(p, "HbxPaintMesh")?, var(p, 0x20, "CarMeshComponentBase_TA")?])?)?;
    guard(base, valid, bind(p, mesh, material, get(p, "HbxItemNativeKey")?, field(p, settings, "ProductAttribute_PaintSettings_TA.PaintParameterName")?, None)?)
}
