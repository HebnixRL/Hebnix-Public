//! Local paint for visual cosmetic packages. Updates material defaults and the
//! model parameters reapplied when skins/attachments rebuild those materials.
//! Inventory attributes stay intact; shaders still need in-game visual checks.
use super::upk_package::{Prop, UpkPackage, strip};
use std::path::Path;
#[path = "painted_swap/runtime_parameters.rs"]
mod runtime_parameters;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SwapPaint {
    #[default]
    Default,
    Crimson,
    Red, // Kept for existing saved swaps.
    TitaniumWhite,
    Preset(u8),
    Custom([u8; 3]),
}

// ProductPaint_TA Colors[0] from the installed TAGame.upk. The picker displays
// sRGB HEX; the UPK stores linear colour. Metallic finishes remain asset-defined.
const PALETTE: [(&str, [f32; 3]); 18] = [
    ("Crimson", [0.6, 0.0, 0.0]),
    ("Lime", [0.5, 1.0, 0.0]),
    ("Black", [0.05, 0.05, 0.05]),
    ("Sky Blue", [0.02, 0.5, 0.8]),
    ("Cobalt", [0.08, 0.2, 1.0]),
    ("Burnt Sienna", [0.3, 0.07, 0.0]),
    ("Forest Green", [0.0, 0.5, 0.0]),
    ("Purple", [0.25, 0.0, 0.5]),
    ("Pink", [0.8, 0.2, 0.6]),
    ("Orange", [1.0, 0.3, 0.0]),
    ("Grey", [0.25, 0.25, 0.25]),
    ("Titanium White", [0.8, 0.8, 0.8]),
    ("Saffron", [0.75, 0.75, 0.0]),
    ("Gold", [0.8045591, 0.5542271, 0.1801443]),
    ("Rose Gold", [1.0, 0.66611695, 0.63204265]),
    ("White Gold", [0.74030423, 0.5621126, 0.40401086]),
    ("Onyx", [0.04943346, 0.04943346, 0.04943346]),
    ("Platinum", [0.78931373, 0.7817507, 0.7667436]),
];
impl SwapPaint {
    pub const ALL: [Self; 20] = [
        Self::Default,
        Self::Custom([255; 3]),
        Self::Crimson,
        Self::Preset(2),
        Self::Preset(3),
        Self::Preset(4),
        Self::Preset(5),
        Self::Preset(6),
        Self::Preset(7),
        Self::Preset(8),
        Self::Preset(9),
        Self::Preset(10),
        Self::Preset(11),
        Self::TitaniumWhite,
        Self::Preset(13),
        Self::Preset(14),
        Self::Preset(15),
        Self::Preset(16),
        Self::Preset(17),
        Self::Preset(18),
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Crimson => "Crimson",
            Self::Red | Self::Custom(_) => "Custom",
            Self::TitaniumWhite => "Titanium White",
            Self::Preset(id) => PALETTE
                .get(id.wrapping_sub(1) as usize)
                .map_or("Invalid paint", |p| p.0),
        }
    }
    pub fn rgb(self) -> [u8; 3] {
        if let Self::Custom(rgb) = self {
            return rgb;
        }
        if self == Self::Default {
            return [255; 3];
        }
        self.material()[..3]
            .try_into()
            .map(|rgb: [f32; 3]| {
                rgb.map(|v| {
                    let srgb = if v <= 0.0031308 {
                        12.92 * v
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    };
                    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
                })
            })
            .unwrap()
    }
    pub fn description(self) -> String {
        if self == Self::Default {
            return "Default (uncoloured)".into();
        }
        let [r, g, b] = self.rgb();
        format!("{} #{r:02X}{g:02X}{b:02X}", self.label())
    }
    fn material(self) -> [f32; 4] {
        let rgb = match self {
            Self::Crimson => PALETTE[0].1,
            Self::Red => [1.0, 0.0, 0.0],
            Self::TitaniumWhite => PALETTE[11].1,
            Self::Preset(id) => PALETTE
                .get(id.wrapping_sub(1) as usize)
                .map_or([0.0; 3], |p| p.1),
            Self::Custom(rgb) => rgb.map(|v| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }),
            Self::Default => [0.0; 3],
        };
        [rgb[0], rgb[1], rgb[2], 1.0]
    }
}

/// Opening the picker does not change paint. Only a changed RGB/valid HEX does.
pub fn controls(ui: &mut eframe::egui::Ui, paint: &mut SwapPaint) {
    use eframe::egui;
    ui.horizontal_wrapped(|ui| {
        let before = *paint;
        let rgb = paint.rgb();
        egui::ComboBox::from_id_salt("paint_preset")
            .width(110.0)
            .selected_text(paint.label())
            .show_ui(ui, |ui| {
                for option in SwapPaint::ALL {
                    let selected = if matches!(option, SwapPaint::Custom(_)) {
                        matches!(*paint, SwapPaint::Custom(_) | SwapPaint::Red)
                    } else {
                        *paint == option
                    };
                    if ui.selectable_label(selected, option.label()).clicked() {
                        *paint = if matches!(option, SwapPaint::Custom(_)) {
                            SwapPaint::Custom(rgb)
                        } else {
                            option
                        };
                    }
                }
            });
        let id = ui.id().with("paint_hex");
        let rgb = paint.rgb();
        let formatted = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
        let mut hex = ui
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| formatted.clone());
        if *paint != before {
            hex = formatted;
        }
        ui.horizontal(|ui| {
            let mut color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            let response = egui::color_picker::color_edit_button_srgba(
                ui,
                &mut color,
                egui::color_picker::Alpha::Opaque,
            );
            // Outline default with a slash so its uncoloured state remains clear,
            // while retaining the same fully clickable picker hitbox.
            if *paint == SwapPaint::Default {
                let rect = response.rect.shrink(2.0);
                let centre = rect.center();
                for (min, max, shade) in [
                    (rect.min, centre, 80),
                    (
                        egui::pos2(centre.x, rect.top()),
                        egui::pos2(rect.right(), centre.y),
                        140,
                    ),
                    (
                        egui::pos2(rect.left(), centre.y),
                        egui::pos2(centre.x, rect.bottom()),
                        140,
                    ),
                    (centre, rect.max, 80),
                ] {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(min, max),
                        0.0,
                        egui::Color32::from_gray(shade),
                    );
                }
                ui.painter().line_segment(
                    [response.rect.left_bottom(), response.rect.right_top()],
                    egui::Stroke::new(1.0, ui.visuals().text_color()),
                );
            }
            if response.changed() {
                let changed = [color.r(), color.g(), color.b()];
                if changed != rgb || *paint == SwapPaint::Default {
                    *paint = SwapPaint::Custom(changed);
                    hex = format!("#{:02X}{:02X}{:02X}", changed[0], changed[1], changed[2]);
                }
            }
            let edit = ui.add(
                egui::TextEdit::singleline(&mut hex)
                    .desired_width(72.0)
                    .char_limit(7),
            );
            if edit.changed() {
                if let Some(changed) = parse_hex(&hex) {
                    if changed != paint.rgb() || *paint == SwapPaint::Default {
                        *paint = SwapPaint::Custom(changed);
                    }
                }
            }
            if edit.lost_focus() {
                let rgb = paint.rgb();
                hex = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
            }
        });
        ui.data_mut(|d| d.insert_temp(id, hex));
    });
    ui.weak(paint.description());
}
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.len() != 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&text[..2], 16).ok()?,
        u8::from_str_radix(&text[2..4], 16).ok()?,
        u8::from_str_radix(&text[4..], 16).ok()?,
    ])
}
fn has_paint_template(donor: &str) -> bool {
    matches!(
        donor.to_ascii_lowercase().as_str(),
        "body_grain_sf.upk" | "boost_standard_sf.upk"
    )
}

fn named<'a>(props: &'a [Prop], name: &str, ty: &str) -> Result<&'a Prop, String> {
    props
        .iter()
        .find(|p| p.name == name && p.tag_type == ty)
        .ok_or_else(|| format!("Missing {name} ({ty}) in paint template"))
}

fn parameter_name(package: &UpkPackage, prop: &Prop, base: usize) -> Result<String, String> {
    if prop.size != 8 {
        return Err("Invalid paint parameter FName".into());
    }
    let index = usize::try_from(package.read_int(base + prop.value_offset)?)
        .map_err(|_| "Negative paint parameter name")?;
    package
        .names
        .get(index)
        .cloned()
        .ok_or_else(|| "Invalid paint parameter name".into())
}

fn vector_slot(
    package: &UpkPackage,
    props: &[Prop],
    base: usize,
    parameter: &str,
) -> Result<Vec<usize>, String> {
    let Some(array) = props
        .iter()
        .find(|p| p.name == "VectorParameterValues" && p.tag_type == "ArrayProperty")
    else {
        return Ok(Vec::new());
    };
    let start = base + array.value_offset;
    let end = start
        .checked_add(array.size)
        .ok_or("Paint array overflow")?;
    if array.size < 4 {
        return Err("Truncated paint vector array".into());
    }
    let count = package.read_int(start)?;
    if !(0..=1024).contains(&count) {
        return Err("Invalid paint vector count".into());
    }
    let mut at = start + 4;
    let mut found = Vec::new();
    for _ in 0..count {
        let (entry, next) = package.nested_props(at, end)?;
        let name = named(&entry, "ParameterName", "NameProperty")?;
        if parameter_name(package, name, 0)? == parameter {
            let value = named(&entry, "ParameterValue", "StructProperty")?;
            if value.struct_name != "LinearColor" || value.size != 16 {
                return Err("Invalid paint vector".into());
            }
            // Cooked MICs can contain duplicate parameter entries. Paint every
            // matching value so either engine lookup order returns the colour.
            found.push(value.value_offset);
        }
        at = next;
    }
    if at != end {
        return Err("Unexpected bytes in paint vector array".into());
    }
    Ok(found)
}

fn struct_array(package: &UpkPackage, prop: &Prop, base: usize) -> Result<Vec<Vec<Prop>>, String> {
    let start = base + prop.value_offset;
    let end = start.checked_add(prop.size).ok_or("Paint array overflow")?;
    let count = package.read_int(start)?;
    if prop.size < 4 || !(0..=4096).contains(&count) {
        return Err("Invalid paint struct array".into());
    }
    let mut at = start + 4;
    let mut entries = Vec::new();
    for _ in 0..count {
        let (fields, next) = package.nested_props(at, end)?;
        entries.push(fields);
        at = next;
    }
    if at != end {
        return Err("Unexpected bytes in paint struct array".into());
    }
    Ok(entries)
}

fn particle_colour(original: &[u8], colour: [f32; 4]) -> Result<Vec<u8>, String> {
    if original.len() != 12 {
        return Err("Invalid particle colour size".into());
    }
    let old: Vec<_> = original
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    if old.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("Invalid particle brightness".into());
    }
    let peak = old.into_iter().fold(0.0f32, f32::max);
    let peak = if peak == 0.0 { 1.0 } else { peak };
    // Keep the effect's brightness scale, including the chosen paint's lightness.
    // Dividing by the chosen colour's maximum turned Black, Grey and White into
    // the same white particle colour and amplified dark custom colours.
    Ok(colour[..3]
        .iter()
        .flat_map(|v| (v * peak).to_le_bytes())
        .collect())
}

fn fname(package: &UpkPackage, name: &str) -> Result<Vec<u8>, String> {
    let index = package
        .names
        .iter()
        .position(|n| n == name)
        .ok_or_else(|| format!("Missing {name} FName"))?;
    Ok([index as i32, 0]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect())
}

fn property(
    package: &UpkPackage,
    name: &str,
    ty: &str,
    structure: Option<&str>,
    value: &[u8],
) -> Result<Vec<u8>, String> {
    let mut bytes = fname(package, name)?;
    bytes.extend(fname(package, ty)?);
    bytes.extend((value.len() as i32).to_le_bytes());
    bytes.extend(0i32.to_le_bytes());
    if let Some(structure) = structure {
        bytes.extend(fname(package, structure)?);
    }
    bytes.extend_from_slice(value);
    Ok(bytes)
}

fn vector_parameter(
    package: &UpkPackage,
    parameter: &str,
    colour: &[u8],
) -> Result<Vec<u8>, String> {
    let mut entry = property(
        package,
        "ParameterName",
        "NameProperty",
        None,
        &fname(package, parameter)?,
    )?;
    entry.extend(property(
        package,
        "ParameterValue",
        "StructProperty",
        Some("LinearColor"),
        colour,
    )?);
    entry.extend(fname(package, "None")?);
    Ok(entry)
}

/// Paint an already identity-swapped package in staging. The donor name is the
/// original asset, even though the staged filename now identifies the owned item.
/// Known structures are required; unsupported layouts fail before writing.
pub fn bake(path: &Path, donor: &str, paint: SwapPaint) -> Result<usize, String> {
    bake_impl(path, donor, paint, true)
}

/// Audit the same patch plan without writing to the installed game package.
pub(crate) fn inspect_paint(path: &Path, donor: &str, paint: SwapPaint) -> Result<usize, String> {
    bake_impl(path, donor, paint, false)
}

fn bake_impl(path: &Path, donor: &str, paint: SwapPaint, write: bool) -> Result<usize, String> {
    if matches!(paint, SwapPaint::Preset(id) if id == 0 || id as usize > PALETTE.len()) {
        return Err("Invalid paint preset".into());
    }
    if paint == SwapPaint::Default {
        return Ok(0);
    }
    if !write {
        return Ok(bake_materials(path, donor, paint, false)?
            + runtime_parameters::bake(path, paint, false)?);
    }
    // Runtime parameter sources and material defaults form one transaction.
    // Skin/attachment rebuilding must see the same colour as the model's MIC.
    let staged = path.with_extension("paint-runtime.tmp");
    let result = (|| {
        std::fs::copy(path, &staged).map_err(|e| e.to_string())?;
        let materials = bake_materials(&staged, donor, paint, true)?;
        let runtime = runtime_parameters::bake(&staged, paint, true)?;
        std::fs::copy(&staged, path).map_err(|e| e.to_string())?;
        Ok(materials + runtime)
    })();
    let _ = std::fs::remove_file(&staged);
    result
}

fn bake_materials(
    path: &Path,
    donor: &str,
    paint: SwapPaint,
    write: bool,
) -> Result<usize, String> {
    if matches!(paint, SwapPaint::Preset(id) if id == 0 || id as usize > PALETTE.len()) {
        return Err("Invalid paint preset".into());
    }
    if paint == SwapPaint::Default {
        return Ok(0);
    }
    if !has_paint_template(donor) {
        return bake_declared_paint(path, paint, write);
    }
    let body = donor.eq_ignore_ascii_case("body_grain_sf.upk");
    let mut package = UpkPackage::load(path)?;
    let mut patches: Vec<(usize, Vec<u8>)> = Vec::new();
    let material_name = if body {
        "MIC_Body_Grain"
    } else {
        "MasterBoost_Standard_MIC"
    };
    let parameter = if body { "TrimColor" } else { "CustomColor" };
    let colors = paint.material();
    let mut materials = 0;
    let mut particles = 0;
    for export in &package.exports {
        let class = package.class_of(export);
        if strip(&class) == "MaterialInstanceConstant"
            && strip(&package.name_of(export.object_name)) == material_name
        {
            let (props, _) = package.serialized_props(export)?;
            let slots = vector_slot(&package, &props, export.serial_offset, parameter)?;
            if slots.is_empty() {
                return Err(format!("Material has no {parameter} vector"));
            }
            for at in slots {
                patches.push((at, colors.iter().flat_map(|v| v.to_le_bytes()).collect()));
            }
            materials += 1;
        }
        if !body && strip(&class) == "DistributionVectorParticleParameter" {
            // These distribution subobjects have a four-byte template index
            // following the net index. Require the observed template sentinel.
            if package.read_int(export.serial_offset + 4)? != -1 {
                return Err("Unsupported particle distribution template header".into());
            }
            let (props, _) = package.nested_props(
                export.serial_offset + 8,
                export.serial_offset + export.serial_size,
            )?;
            let name = named(&props, "ParameterName", "NameProperty")?;
            if matches!(
                parameter_name(&package, name, 0)?.as_str(),
                "CustomColor" | "InnerColor"
            ) {
                let constant = named(&props, "Constant", "StructProperty")?;
                if constant.struct_name != "Vector" || constant.size != 12 {
                    return Err("Unexpected particle colour layout".into());
                }
                let at = constant.value_offset;
                // Preserve the original HDR peak brightness while replacing hue.
                let old = &package.image[at..at + 12];
                let rgb = particle_colour(old, colors)?;
                patches.push((at, rgb));
                let none = package
                    .names
                    .iter()
                    .position(|name| name == "None")
                    .ok_or("Missing None FName")?;
                patches.push((
                    name.value_offset,
                    [none as i32, 0]
                        .into_iter()
                        .flat_map(i32::to_le_bytes)
                        .collect(),
                ));
                particles += 1;
            }
        }
    }
    if materials != 1 || (!body && particles != 3) {
        return Err(format!(
            "Unsupported donor layout: found {materials} paint materials and {particles} particle colour defaults"
        ));
    }
    if !write {
        return Ok(patches.len());
    }
    for (offset, bytes) in &patches {
        package.patch(*offset, bytes)?;
    }
    let temporary = path.with_extension("paint.tmp");
    let result = (|| {
        package.save(&temporary)?;
        let check = UpkPackage::load(&temporary)?;
        // Repacking may change physical offsets, but no export may change beyond
        // the planned float values. Includes meshes, textures and all native tails.
        for export in &package.exports {
            let start = export.serial_offset;
            let end = start + export.serial_size;
            if check.image.get(start..end) != package.image.get(start..end) {
                return Err("Paint package export round-trip mismatch".into());
            }
        }
        for (offset, bytes) in &patches {
            if check.image.get(*offset..*offset + bytes.len()) != Some(bytes.as_slice()) {
                return Err("Paint values did not survive package round-trip".into());
            }
        }
        std::fs::copy(&temporary, path).map_err(|e| e.to_string())?;
        Ok(patches.len())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

/// Recolour an owned item's pristine package without changing its identity.
/// All parsing and verification happens in staging before replacing the live UPK.
pub fn recolour_for_target(
    source: &Path,
    destination: &Path,
    donor_filename: &str,
    paint: SwapPaint,
) -> Result<(), String> {
    let parent = destination.parent().ok_or("No destination directory")?;
    let staging = parent.join(format!(
        ".hebnix-recolour-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let staged = staging.join(destination.file_name().ok_or("No destination filename")?);
    let result = (|| {
        std::fs::copy(source, &staged).map_err(|e| e.to_string())?;
        bake(&staged, donor_filename, paint)?;
        std::fs::copy(&staged, destination).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = std::fs::remove_file(staged.with_extension("paint.tmp"));
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_dir(&staging);
    result
}

/// Keep the existing identity patch and paint patch together in staging, so a
/// missing material or failed verification never installs a half-painted swap.
pub fn patch_for_target(
    source: &Path,
    target_backup: &Path,
    destination: &Path,
    base_dir: &Path,
    donor_asset_path: Option<&str>,
    target_asset_path: Option<&str>,
    donor_filename: &str,
    paint: SwapPaint,
) -> Result<(), String> {
    if paint == SwapPaint::Default {
        return crate::cosmetic_upk::patch_for_target(
            source,
            target_backup,
            destination,
            base_dir,
            donor_asset_path,
            target_asset_path,
        );
    }
    let parent = destination.parent().ok_or("No destination directory")?;
    let staging = parent.join(format!(
        ".hebnix-paint-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let staged = staging.join(destination.file_name().ok_or("No destination filename")?);
        crate::cosmetic_upk::patch_for_target(
            source,
            target_backup,
            &staged,
            base_dir,
            donor_asset_path,
            target_asset_path,
        )?;
        bake(&staged, donor_filename, paint)?;
        std::fs::copy(&staged, destination).map_err(|e| e.to_string())?;
        Ok(())
    })();
    // Only files created above, never an arbitrary recursive directory removal.
    if let Some(name) = destination.file_name() {
        let staged = staging.join(name);
        let _ = std::fs::remove_file(staged.with_extension("paint.tmp"));
        let _ = std::fs::remove_file(staged);
    }
    let _ = std::fs::remove_dir(&staging);
    result
}

/// These are colour controls, unlike vector parameters used for UVs, masks,
/// positions or team colours. Official paint settings take precedence.
fn local_colour_parameter(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "customcolor"
            | "trimcolor"
            | "paintcolor"
            | "paintedcolor"
            | "color"
            | "colour"
            | "diffusecolor"
            | "diffusetint"
            | "basecolor"
            | "tintcolor"
            | "colortint"
    )
}

fn has_imported_paint_settings(package: &UpkPackage) -> Result<bool, String> {
    for export in &package.exports {
        if !strip(&package.class_of(export)).starts_with("ProductAsset_") {
            continue;
        }
        let (props, _) = package.serialized_props(export)?;
        for array in props
            .iter()
            .filter(|p| p.name == "Attributes" && p.tag_type == "ArrayProperty")
        {
            let at = export.serial_offset + array.value_offset;
            let count = package.read_int(at)?;
            if count < 0 || array.size != 4 + count as usize * 4 {
                return Err("Invalid product attribute array".into());
            }
            for index in 0..count as usize {
                let reference = package.read_int(at + 4 + index * 4)?;
                if reference < 0
                    && package.reference_class(reference)?.1 == "ProductAttribute_PaintSettings_TA"
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

// Prefer the asset's paint settings. Without inventory paint metadata, use
// explicit material colour controls belonging to the visual product itself.
fn bake_declared_paint(path: &Path, paint: SwapPaint, write: bool) -> Result<usize, String> {
    use std::collections::{HashMap, HashSet};
    let mut package = UpkPackage::load(path)?;
    // Cooked objects can omit defaults and their property-name entries entirely.
    // Explicit paint overrides then need new tags, without reusing an unrelated
    // name index or changing any existing FName reference.
    let property_names = [
        "VectorParameterValues",
        "ParameterName",
        "ParameterValue",
        "NameProperty",
        "StructProperty",
        "ArrayProperty",
        "LinearColor",
        "DefaultValue",
        "Constant",
        "FloatProperty",
        "Vector",
        "CustomColor",
    ];
    if property_names
        .iter()
        .any(|name| !package.names.iter().any(|n| n == name))
    {
        if write {
            let prepared = path.with_extension("paint-names.tmp");
            let result = (|| {
                crate::cosmetic_upk::append_names(path, &prepared, &property_names)?;
                let changed = bake_declared_paint(&prepared, paint, true)?;
                std::fs::copy(&prepared, path).map_err(|e| e.to_string())?;
                Ok(changed)
            })();
            let _ = std::fs::remove_file(&prepared);
            return result;
        }
        // The audit plans the same appended indexes without saving a package.
        for name in property_names {
            if !package.names.iter().any(|n| n == name) {
                package.names.push(name.into());
            }
        }
    }
    let mut materials: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut parameter_names = HashSet::new();
    let mut runtime_material_parameters = HashSet::new();
    let mut painted_switches = HashSet::new();
    let mut material_overrides = Vec::new();
    for e in &package.exports {
        if strip(&package.class_of(e)) == "ProductOverride_ParticleSystemColorParameter_TA" {
            let (props, _) = package.serialized_props(e)?;
            for array in props
                .iter()
                .filter(|p| p.name == "ParameterOverrides" && p.tag_type == "ArrayProperty")
            {
                for fields in struct_array(&package, array, e.serial_offset)? {
                    if let Some(name) = fields
                        .iter()
                        .find(|p| p.name == "PaintParameterName" && p.tag_type == "NameProperty")
                    {
                        parameter_names.insert(parameter_name(&package, name, 0)?);
                    }
                }
            }
        } else if strip(&package.class_of(e)) == "ProductOverride_MaterialParameter_TA" {
            let (props, _) = package.serialized_props(e)?;
            let Some(name) = props
                .iter()
                .find(|p| p.name == "MaterialName" && p.tag_type == "NameProperty")
            else {
                continue;
            };
            let material_name = parameter_name(&package, name, e.serial_offset)?;
            for array in props
                .iter()
                .filter(|p| p.name == "VectorParameterValues" && p.tag_type == "ArrayProperty")
            {
                for fields in struct_array(&package, array, e.serial_offset)? {
                    let name = named(&fields, "ParameterName", "NameProperty")?;
                    let parameter = parameter_name(&package, name, 0)?;
                    let value = named(&fields, "ParameterValue", "StructProperty")?;
                    if value.struct_name != "LinearColor" || value.size != 16 {
                        return Err("Invalid material paint override".into());
                    }
                    material_overrides.push((material_name.clone(), parameter, value.value_offset));
                }
            }
        } else if strip(&package.class_of(e)) == "ProductOverride_ParticleSystemFloatParameter_TA" {
            let (props, _) = package.serialized_props(e)?;
            for array in props
                .iter()
                .filter(|p| p.name == "ParameterOverrides" && p.tag_type == "ArrayProperty")
            {
                for fields in struct_array(&package, array, e.serial_offset)? {
                    if let Some(key) = fields
                        .iter()
                        .find(|p| p.name == "Key" && p.tag_type == "NameProperty")
                    {
                        let key = parameter_name(&package, key, 0)?;
                        if key == "PaintedSwitch" {
                            painted_switches.insert(key);
                        }
                    }
                }
            }
        }
    }
    for e in &package.exports {
        if strip(&package.class_of(e)) != "ProductAttribute_PaintSettings_TA" {
            continue;
        }
        let (props, _) = package.serialized_props(e)?;
        let parameter = if let Some(p) = props
            .iter()
            .find(|p| p.name == "PaintParameterName" && p.tag_type == "NameProperty")
        {
            parameter_name(&package, p, e.serial_offset)?
        } else {
            "CustomColor".into()
        };
        parameter_names.insert(parameter.clone());
        if props
            .iter()
            .any(|p| p.name == "bPaintBody" && p.bool_value == Some(true))
        {
            // Body paint uses the body's trim MIC rather than MaterialGroups;
            // those groups commonly contain a null slot for runtime assignment.
            for material in &package.exports {
                if strip(&package.class_of(material)) == "MaterialInstanceConstant" {
                    let (fields, _) = package.serialized_props(material)?;
                    let paintable_parent = fields
                        .iter()
                        .find(|p| p.name == "Parent" && p.tag_type == "ObjectProperty")
                        .map(|p| {
                            package
                                .read_int(material.serial_offset + p.value_offset)
                                .map(|reference| {
                                    package
                                        .object_path(reference)
                                        .to_ascii_lowercase()
                                        .contains("body_paintable")
                                })
                        })
                        .transpose()?
                        .unwrap_or(false);
                    if !vector_slot(&package, &fields, material.serial_offset, &parameter)?
                        .is_empty()
                        || paintable_parent
                        || strip(&package.name_of(material.object_name))
                            .to_ascii_lowercase()
                            .starts_with("mic_body_")
                    {
                        materials
                            .entry(material.table_index)
                            .or_default()
                            .insert(parameter.clone());
                    }
                }
            }
        }
        for array in props
            .iter()
            .filter(|p| p.name == "PaintAdditionalParameters" && p.tag_type == "ArrayProperty")
        {
            for fields in struct_array(&package, array, e.serial_offset)? {
                if fields
                    .iter()
                    .any(|p| p.name == "bEnabled" && p.bool_value == Some(false))
                {
                    continue;
                }
                let name = named(&fields, "ParameterName", "NameProperty")?;
                parameter_names.insert(parameter_name(&package, name, 0)?);
            }
        }
        for group in props
            .iter()
            .filter(|p| p.name == "MaterialGroups" && p.struct_name == "PaintMaterialGroup")
        {
            let start = e.serial_offset + group.value_offset;
            let (fields, _) = package.nested_props(start, start + group.size)?;
            for array in fields
                .iter()
                .filter(|p| p.name == "Materials" && p.tag_type == "ArrayProperty")
            {
                let count = package.read_int(array.value_offset)?;
                if count < 0 || array.size != 4 + count as usize * 4 {
                    return Err("Invalid paint material group".into());
                }
                for i in 0..count as usize {
                    let mut reference = package.read_int(array.value_offset + 4 + i * 4)?;
                    if reference == 0 {
                        runtime_material_parameters.insert(parameter.clone());
                    }
                    let mut visited = HashSet::new();
                    while reference > 0 && visited.insert(reference) {
                        let index = reference as usize - 1;
                        let material = package
                            .exports
                            .get(index)
                            .ok_or("Invalid paint material reference")?;
                        let class = package.class_of(material);
                        if !matches!(strip(&class), "Material" | "MaterialInstanceConstant") {
                            return Err("Paint group references a non-material".into());
                        }
                        materials
                            .entry(index)
                            .or_default()
                            .insert(parameter.clone());
                        let (fields, _) = package.serialized_props(material)?;
                        reference = match fields
                            .iter()
                            .find(|p| p.name == "Parent" && p.tag_type == "ObjectProperty")
                        {
                            Some(p) => package.read_int(material.serial_offset + p.value_offset)?,
                            None => 0,
                        };
                    }
                }
            }
        }
    }
    for (name, parameter, _) in &material_overrides {
        if parameter_names.contains(parameter) {
            for material in &package.exports {
                if strip(&package.class_of(material)) == "MaterialInstanceConstant"
                    && strip(&package.name_of(material.object_name)) == name
                {
                    materials
                        .entry(material.table_index)
                        .or_default()
                        .insert(parameter.clone());
                }
            }
        }
    }
    // Null material slots are resolved by the game's attachment generator. Their
    // local materials still expose the declared paint parameter; baking must
    // populate the MIC override rather than ignoring the runtime slot entirely.
    let mut declared_material_parameters: HashMap<usize, HashSet<String>> = HashMap::new();
    for expression in &package.exports {
        if strip(&package.class_of(expression)) == "MaterialExpressionVectorParameter"
            && expression.outer_index > 0
        {
            let (props, _) = package.serialized_props(expression)?;
            if let Some(name) = props
                .iter()
                .find(|p| p.name == "ParameterName" && p.tag_type == "NameProperty")
            {
                declared_material_parameters
                    .entry(expression.outer_index as usize - 1)
                    .or_default()
                    .insert(parameter_name(&package, name, expression.serial_offset)?);
            }
        }
    }
    for material in &package.exports {
        let class = package.class_of(material);
        if !matches!(strip(&class), "Material" | "MaterialInstanceConstant") {
            continue;
        }
        for parameter in &runtime_material_parameters {
            let mut reference = material.table_index as i32 + 1;
            let mut visited = HashSet::new();
            let mut declared = false;
            while reference > 0 && visited.insert(reference) {
                let index = reference as usize - 1;
                let parent = package
                    .exports
                    .get(index)
                    .ok_or("Invalid material parent")?;
                let (fields, _) = package.serialized_props(parent)?;
                declared |= declared_material_parameters
                    .get(&index)
                    .is_some_and(|names| names.contains(parameter));
                if strip(&package.class_of(parent)) == "MaterialInstanceConstant" {
                    declared |= !vector_slot(&package, &fields, parent.serial_offset, parameter)?
                        .is_empty();
                }
                reference = match fields
                    .iter()
                    .find(|p| p.name == "Parent" && p.tag_type == "ObjectProperty")
                {
                    Some(p) => package.read_int(parent.serial_offset + p.value_offset)?,
                    None => 0,
                };
            }
            if declared {
                materials
                    .entry(material.table_index)
                    .or_default()
                    .insert(parameter.clone());
            }
        }
    }
    if parameter_names.is_empty() && materials.is_empty() {
        let product_roots: HashSet<String> = package
            .exports
            .iter()
            .filter(|e| strip(&package.class_of(e)).starts_with("ProductAsset_"))
            .map(|e| {
                package
                    .asset_path(e.table_index as i32 + 1)
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        for material in &package.exports {
            let class = package.class_of(material);
            if !matches!(strip(&class), "Material" | "MaterialInstanceConstant") {
                continue;
            }
            let path = package.asset_path(material.table_index as i32 + 1);
            if !product_roots.contains(path.split('.').next().unwrap_or("")) {
                continue;
            }
            let mut reference = material.table_index as i32 + 1;
            let mut visited = HashSet::new();
            let mut colours = HashSet::new();
            while reference > 0 && visited.insert(reference) {
                let parent = package
                    .exports
                    .get(reference as usize - 1)
                    .ok_or("Invalid material parent")?;
                if let Some(names) = declared_material_parameters.get(&parent.table_index) {
                    colours.extend(
                        names
                            .iter()
                            .filter(|name| local_colour_parameter(name))
                            .cloned(),
                    );
                }
                let (props, _) = package.serialized_props(parent)?;
                for array in props
                    .iter()
                    .filter(|p| p.name == "VectorParameterValues" && p.tag_type == "ArrayProperty")
                {
                    for fields in struct_array(&package, array, parent.serial_offset)? {
                        let name = named(&fields, "ParameterName", "NameProperty")?;
                        let name = parameter_name(&package, name, 0)?;
                        if local_colour_parameter(&name) {
                            colours.insert(name);
                        }
                    }
                }
                reference = match props
                    .iter()
                    .find(|p| p.name == "Parent" && p.tag_type == "ObjectProperty")
                {
                    Some(p) => package.read_int(parent.serial_offset + p.value_offset)?,
                    None => 0,
                };
            }
            if !colours.is_empty() {
                // Override on the product's MIC, leaving shared parent assets
                // and unrelated effects (such as a body's exhaust) intact.
                materials.insert(material.table_index, colours);
            }
        }
    }
    let colour = paint.material();
    let bytes: Vec<u8> = colour.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut changes = Vec::new();
    let mut insertions = Vec::new();
    let mut missing_vectors: HashMap<usize, Vec<String>> = HashMap::new();
    for (_, parameter, offset) in &material_overrides {
        if parameter_names.contains(parameter) {
            changes.push((*offset, bytes.clone()));
        }
    }
    // Baked paint has to use the painted effect even when the equipped inventory
    // item is unpainted. Otherwise colour changes target an unused particle tree.
    for e in &package.exports {
        if strip(&package.class_of(e)).starts_with("ProductAsset_") {
            let (props, _) = package.serialized_props(e)?;
            for painted in props
                .iter()
                .filter(|p| p.tag_type == "ObjectProperty" && p.name.starts_with("Painted"))
            {
                let Some(normal) = props.iter().find(|p| {
                    p.tag_type == "ObjectProperty"
                        && p.name == painted.name.trim_start_matches("Painted")
                }) else {
                    continue;
                };
                let reference = package.read_int(e.serial_offset + painted.value_offset)?;
                if reference > 0 && reference as usize <= package.exports.len() {
                    changes.push((
                        e.serial_offset + normal.value_offset,
                        reference.to_le_bytes().to_vec(),
                    ));
                }
            }
        }
    }
    let routing_changes = changes.len()
        - material_overrides
            .iter()
            .filter(|(_, parameter, _)| parameter_names.contains(parameter))
            .count();
    for e in &package.exports {
        let class = package.class_of(e);
        if strip(&class) == "MaterialInstanceConstant" {
            if let Some(parameters) = materials.get(&e.table_index) {
                let (props, _) = package.serialized_props(e)?;
                for parameter in parameters {
                    let slots = vector_slot(&package, &props, e.serial_offset, parameter)?;
                    if slots.is_empty() {
                        missing_vectors
                            .entry(e.table_index)
                            .or_default()
                            .push(parameter.clone());
                    } else {
                        for at in slots {
                            changes.push((at, bytes.clone()));
                        }
                    }
                }
            }
        } else if strip(&class) == "MaterialExpressionVectorParameter" && e.outer_index > 0 {
            if let Some(parameters) = materials.get(&(e.outer_index as usize - 1)) {
                let (props, _) = package.serialized_props(e)?;
                let Some(name) = props
                    .iter()
                    .find(|p| p.name == "ParameterName" && p.tag_type == "NameProperty")
                else {
                    continue;
                };
                if parameters.contains(&parameter_name(&package, name, e.serial_offset)?) {
                    if let Some(value) = props
                        .iter()
                        .find(|p| p.name == "DefaultValue" && p.tag_type == "StructProperty")
                    {
                        if value.struct_name != "LinearColor" || value.size != 16 {
                            return Err("Invalid paint default colour".into());
                        }
                        changes.push((e.serial_offset + value.value_offset, bytes.clone()));
                    } else {
                        let (_, native) = package.serialized_props(e)?;
                        insertions.push((
                            e.table_index,
                            native - 8,
                            property(
                                &package,
                                "DefaultValue",
                                "StructProperty",
                                Some("LinearColor"),
                                &bytes,
                            )?,
                        ));
                    }
                }
            }
        } else if strip(&class) == "DistributionFloatParticleParameter" {
            let start = e.serial_offset
                + if package.read_int(e.serial_offset + 4)? == -1 {
                    8
                } else {
                    4
                };
            let (props, native) = package.nested_props(start, e.serial_offset + e.serial_size)?;
            let Some(name) = props
                .iter()
                .find(|p| p.name == "ParameterName" && p.tag_type == "NameProperty")
            else {
                continue;
            };
            if painted_switches.contains(&parameter_name(&package, name, 0)?) {
                if let Some(value) = props
                    .iter()
                    .find(|p| p.name == "Constant" && p.tag_type == "FloatProperty" && p.size == 4)
                {
                    changes.push((value.value_offset, 1.0f32.to_le_bytes().to_vec()));
                } else {
                    insertions.push((
                        e.table_index,
                        native - e.serial_offset - 8,
                        property(
                            &package,
                            "Constant",
                            "FloatProperty",
                            None,
                            &1.0f32.to_le_bytes(),
                        )?,
                    ));
                }
                changes.push((name.value_offset, fname(&package, "None")?));
            }
        } else if strip(&class) == "DistributionVectorParticleParameter" {
            let start = e.serial_offset
                + if package.read_int(e.serial_offset + 4)? == -1 {
                    8
                } else {
                    4
                };
            let (props, native) = package.nested_props(start, e.serial_offset + e.serial_size)?;
            let Some(name) = props
                .iter()
                .find(|p| p.name == "ParameterName" && p.tag_type == "NameProperty")
            else {
                continue;
            };
            if parameter_names.contains(&parameter_name(&package, name, 0)?) {
                if let Some(value) = props
                    .iter()
                    .find(|p| p.name == "Constant" && p.tag_type == "StructProperty")
                {
                    if value.struct_name != "Vector" || value.size != 12 {
                        return Err("Invalid paint particle colour".into());
                    }
                    let old = &package.image[value.value_offset..value.value_offset + 12];
                    changes.push((value.value_offset, particle_colour(old, colour)?));
                } else {
                    // An omitted Constant inherits zero. These effects rely on
                    // a runtime parameter; freezing it requires an explicit value.
                    let mut tag = Vec::new();
                    for n in ["Constant", "StructProperty"] {
                        let index = package
                            .names
                            .iter()
                            .position(|name| name == n)
                            .ok_or_else(|| format!("Missing {n} FName"))?;
                        tag.extend_from_slice(&(index as i32).to_le_bytes());
                        tag.extend_from_slice(&0i32.to_le_bytes());
                    }
                    tag.extend_from_slice(&12i32.to_le_bytes());
                    tag.extend_from_slice(&0i32.to_le_bytes());
                    let vector = package
                        .names
                        .iter()
                        .position(|name| name == "Vector")
                        .ok_or("Missing Vector FName")?;
                    tag.extend_from_slice(&(vector as i32).to_le_bytes());
                    tag.extend_from_slice(&0i32.to_le_bytes());
                    tag.extend(colour[..3].iter().flat_map(|v| v.to_le_bytes()));
                    insertions.push((e.table_index, native - e.serial_offset - 8, tag));
                }
                // This is an explicit local colour override. Runtime paint/team
                // parameters must not replace it when the FX actor is spawned.
                let none = package
                    .names
                    .iter()
                    .position(|name| name == "None")
                    .ok_or("Missing None FName")?;
                changes.push((
                    name.value_offset,
                    [none as i32, 0]
                        .into_iter()
                        .flat_map(i32::to_le_bytes)
                        .collect(),
                ));
            }
        }
    }
    for e in &package.exports {
        if strip(&package.class_of(e)).starts_with("ProductAsset_") {
            let (props, _) = package.serialized_props(e)?;
            for p in props.iter().filter(|p| {
                p.name == "DefaultPaintColor" && p.struct_name == "LinearColor" && p.size == 16
            }) {
                changes.push((e.serial_offset + p.value_offset, bytes.clone()));
            }
        }
    }
    for (index, parameters) in &missing_vectors {
        let export = &package.exports[*index];
        let (props, native) = package.serialized_props(export)?;
        let mut entries = Vec::new();
        for parameter in parameters {
            entries.extend(vector_parameter(&package, parameter, &bytes)?);
        }
        if let Some(array) = props
            .iter()
            .find(|p| p.name == "VectorParameterValues" && p.tag_type == "ArrayProperty")
        {
            let count = package.read_int(export.serial_offset + array.value_offset)?;
            if !(0..=4096).contains(&count) {
                return Err("Invalid MIC vector count".into());
            }
            struct_array(&package, array, export.serial_offset)?;
            changes.push((
                export.serial_offset + array.tag_offset + 16,
                ((array.size + entries.len()) as i32).to_le_bytes().to_vec(),
            ));
            changes.push((
                export.serial_offset + array.value_offset,
                (count + parameters.len() as i32).to_le_bytes().to_vec(),
            ));
            insertions.push((*index, array.value_offset + array.size, entries));
        } else {
            let mut value = (parameters.len() as i32).to_le_bytes().to_vec();
            value.extend(entries);
            insertions.push((
                *index,
                native - 8,
                property(
                    &package,
                    "VectorParameterValues",
                    "ArrayProperty",
                    None,
                    &value,
                )?,
            ));
        }
    }
    changes.sort_by_key(|(at, _)| *at);
    changes.dedup_by_key(|(at, _)| *at);
    if changes.len() == routing_changes && insertions.is_empty() {
        // Common wheels such as OEM keep their real paint settings/materials
        // in Startup.upk. Their seek-free package only imports those objects;
        // absence of local parameters does not mean the wheel is unpaintable.
        if has_imported_paint_settings(&package)? {
            return Err("This item's paint settings are stored in a shared game package, such as Startup.upk. Recolouring shared paint assets is not supported yet. No change was installed.".into());
        }
        return Err("This item has no editable local colour parameters. Its colours may be fixed in textures or controlled by the game. No change was installed.".into());
    }
    if !write {
        return Ok(changes.len() + insertions.len());
    }
    for (at, bytes) in &changes {
        package.patch(*at, bytes)?;
    }
    for (index, at, tag) in &insertions {
        let export = &package.exports[*index];
        let mut bytes =
            package.image[export.serial_offset..export.serial_offset + export.serial_size].to_vec();
        bytes.splice(*at..*at, tag.iter().copied());
        package.replace_export_payload(*index, &bytes)?;
    }
    let temporary = path.with_extension("paint.tmp");
    let result = (|| {
        package.save(&temporary)?;
        let check = UpkPackage::load(&temporary)?;
        for e in &package.exports {
            if check
                .image
                .get(e.serial_offset..e.serial_offset + e.serial_size)
                != package
                    .image
                    .get(e.serial_offset..e.serial_offset + e.serial_size)
            {
                return Err("Paint export verification failed".into());
            }
        }
        std::fs::copy(&temporary, path).map_err(|e| e.to_string())?;
        Ok(changes.len() + insertions.len())
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod shared_paint_tests {
    use super::*;

    #[test]
    fn oem_reports_shared_paint_without_installing_a_change() {
        let Some(game) = std::env::var_os("HEBNIX_RECOLOUR_GAME_DIR") else {
            return;
        };
        let game = std::path::PathBuf::from(game);
        let name = "WHEEL_Star_SF.upk";
        let backup = game.join("Backups").join(format!("{name}.bak"));
        let source = if backup.is_file() {
            backup
        } else {
            game.join(name)
        };
        let original = std::fs::read(&source).unwrap();
        let package = UpkPackage::load(&source).unwrap();
        assert!(has_imported_paint_settings(&package).unwrap());
        let audit = inspect_paint(&source, name, SwapPaint::Crimson).unwrap_err();
        assert!(audit.contains("shared game package"), "{audit}");
        let root = std::env::temp_dir().join(format!("hebnix-oem-paint-{}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let destination = root.join(name);
        let sentinel = b"previous installed content";
        std::fs::write(&destination, sentinel).unwrap();
        let error =
            recolour_for_target(&source, &destination, name, SwapPaint::Crimson).unwrap_err();
        assert_eq!(error, audit);
        assert_eq!(std::fs::read(&destination).unwrap(), sentinel);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
