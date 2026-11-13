//! Lossless edits to loadout presets (`LoadoutSets` in `ProfileLoadoutSave_TA`).
//! Built on `item_cleanup::Document`. Deleted presets are unlinked, not removed,
//! like cleanup does for inventory, so object numbers stay valid.
use super::binary_serializer::write_ue3;
use super::item_cleanup::{
    Document, Field, Object, field, fields, instance, int_array, replace_field, text,
};
use super::loadout_code::{Item, Preset, SLOTS, Team};
use std::collections::HashMap;
use std::ops::Range;

pub enum Edit {
    Rename { position: usize, name: String },
    Delete { position: usize },
    Move { from: usize, to: usize, expected: Vec<u32> },
    Equip { position: usize, changes: Vec<(usize, usize, u64, u64)> },
    Update { position: usize, name: String, changes: Vec<(usize, usize, u64, u64)>, colours: [[u8; 2]; 2], expected: Preset },
    Import(Vec<Preset>),
}

/// Attribute key, object type suffix, value field. Only these kinds exist in the save.
const ATTRS: [(&str, &str, &str); 7] = [
    ("TitleId", "TitleID", "TitleId"),
    ("Painted", "Painted", "PaintID"),
    ("Certified", "Certified", "StatId"),
    ("Quality", "Quality", "Quality"),
    ("Blueprint", "Blueprint", "ProductID"),
    ("BlueprintCost", "BlueprintCost", "Cost"),
    ("NoNotify", "NoNotify", ""),
];

/// Returns the edited save, or `None` when nothing changed.
pub fn plan(raw: &[u8], edit: &Edit) -> Result<Option<Vec<u8>>, String> {
    let mut doc = Document::read(raw)?;
    if doc.encode()? != raw {
        return Err("save does not round-trip losslessly; original left unchanged".into());
    }
    let changed = match edit {
        Edit::Rename { position, name } => rename(&mut doc, *position, name)?,
        Edit::Delete { position } => delete(&mut doc, *position)?,
        Edit::Move { from, to, expected } => {
            if set_refs(&doc)? != *expected { return Err("The preset order changed; reopen the move dialog.".into()); }
            move_preset(&mut doc, *from, *to)?
        },
        Edit::Equip { position, changes } => equip(&mut doc, *position, changes)?,
        Edit::Update { position, name, changes, colours, expected } => {
            if export_preset(raw, *position)? != *expected { return Err("The preset changed; reopen the editor.".into()); }
            let renamed = rename(&mut doc, *position, name)?;
            let equipped = equip(&mut doc, *position, changes)?;
            let recoloured = update_colours(&mut doc, *position, *colours, expected)?;
            renamed || equipped || recoloured
        },
        Edit::Import(presets) => import(&mut doc, presets)?,
    };
    if !changed {
        return Ok(None);
    }
    let bytes = doc.encode()?;
    if Document::read(&bytes)?.encode()? != bytes {
        return Err("loadout edit verification failed".into());
    }
    Ok(Some(bytes))
}

fn update_colours(doc: &mut Document, position: usize, colours: [[u8; 2]; 2], expected: &Preset) -> Result<bool, String> {
    let previous = expected.teams.each_ref().map(|team| [team.team_color, team.custom_color]);
    if colours == previous {
        return Ok(false);
    }
    let set = *set_refs(doc)?.get(position).ok_or("no such preset")? as usize;
    let set_obj = doc.objects.get(set).ok_or("invalid preset reference")?;
    if set_obj.kind != "TAGame.LoadoutSet_TA" {
        return Err("invalid preset type".into());
    }
    let mut loadouts = array_u32(&set_obj.bytes, "Loadouts")?;
    if loadouts.len() != 2 {
        return Err("unsupported loadout set shape".into());
    }
    if loadouts[0] == loadouts[1] && colours[0] != colours[1] {
        let shared = doc.objects.get(loadouts[1] as usize).ok_or("invalid loadout reference")?;
        if shared.kind != "TAGame.Loadout_TA" {
            return Err("invalid loadout type".into());
        }
        loadouts[1] = push_object(doc, "TAGame.Loadout_TA", shared.bytes.clone());
        set_array(&mut doc.objects[set].bytes, "Loadouts", &loadouts)?;
    }
    for (team, loadout) in loadouts.into_iter().enumerate() {
        if colours[team] == previous[team] {
            continue;
        }
        let obj = doc.objects.get_mut(loadout as usize).ok_or("invalid loadout reference")?;
        if obj.kind != "TAGame.Loadout_TA" {
            return Err("invalid loadout type".into());
        }
        write_team_paint(&mut obj.bytes, &[
            ("TeamColorID", i64::from(colours[team][0])),
            ("CustomColorID", i64::from(colours[team][1])),
        ])?;
    }
    Ok(true)
}

pub fn preset_order(raw: &[u8]) -> Result<Vec<u32>, String> {
    set_refs(&Document::read(raw)?)
}

/// Owned inventory instances, with their product IDs, from the same save as the presets.
pub fn owned_instances(raw: &[u8]) -> Result<Vec<(i32, u64, u64, i64)>, String> {
    let doc = Document::read(raw)?;
    let mut items = Vec::new();
    for (key, index) in inventory_index(&doc)? {
        let product_obj = &doc.objects[index];
        let attributes = if span(&product_obj.bytes, "Attributes").is_ok() { array_u32(&product_obj.bytes, "Attributes")? } else { Vec::new() };
        if attributes.iter().any(|&id| {
                doc.objects.get(id as usize).is_some_and(|o| o.kind == "TAGame.ProductAttribute_Blueprint_TA")
            }) { continue; }
        let paint = attributes.iter().filter_map(|&id| doc.objects.get(id as usize))
            .find(|o| o.kind == "TAGame.ProductAttribute_Painted_TA")
            .map(|o| read_scalar(&o.bytes, "PaintID")).transpose()?.unwrap_or(0);
        let id = scalar_or_zero(&doc.objects[index].bytes, "ProductID")?;
        let upper = u64::from_str_radix(&key[..16], 16).map_err(|_| "invalid instance ID")?;
        let lower = u64::from_str_radix(&key[16..], 16).map_err(|_| "invalid instance ID")?;
        if id > 0 && id <= i32::MAX as i64 && (upper != 0 || lower != 0) {
            items.push((id as i32, upper, lower, paint));
        }
    }
    Ok(items)
}

fn equip(doc: &mut Document, position: usize, changes: &[(usize, usize, u64, u64)]) -> Result<bool, String> {
    if changes.is_empty() { return Ok(false); }
    let set = *set_refs(doc)?.get(position).ok_or("no such preset")? as usize;
    let set_obj = doc.objects.get(set).ok_or("invalid preset reference")?;
    if set_obj.kind != "TAGame.LoadoutSet_TA" { return Err("invalid preset type".into()); }
    let loadouts = array_u32(&set_obj.bytes, "Loadouts")?;
    if loadouts.len() != 2 { return Err("unsupported loadout set shape".into()); }
    let inventory = inventory_index(doc)?;
    for &(team, slot, upper, lower) in changes {
        if team >= 2 || slot >= SLOTS { return Err("invalid team or slot".into()); }
        let key = format!("{upper:016x}{lower:016x}");
        let index = *inventory.get(&key).ok_or("item is no longer owned by this account")?;
        let product = scalar_or_zero(&doc.objects[index].bytes, "ProductID")?;
        if product <= 0 || product > i32::MAX as i64 { return Err("invalid owned product".into()); }
        let obj = doc.objects.get_mut(loadouts[team] as usize).ok_or("invalid loadout reference")?;
        if obj.kind != "TAGame.Loadout_TA" { return Err("invalid loadout type".into()); }
        let mut products = array_u32(&obj.bytes, "Products")?;
        let mut instances = instance_array(&obj.bytes)?;
        if products.len() != SLOTS || instances.len() != SLOTS { return Err("unsupported loadout slot count".into()); }
        products[slot] = product as u32;
        instances[slot] = (upper, lower);
        set_array(&mut obj.bytes, "Products", &products)?;
        let mut payload = (SLOTS as u32).to_le_bytes().to_vec();
        for (upper, lower) in instances { payload.extend(instance_blob(upper, lower)); }
        let (props, _) = fields(&obj.bytes)?;
        let f = field(&props, "OnlineProducts128", "ArrayProperty")?;
        replace_field(&mut obj.bytes, f, &payload);
    }
    Ok(true)
}

fn move_preset(doc: &mut Document, from: usize, to: usize) -> Result<bool, String> {
    let mut refs = set_refs(doc)?;
    if from >= refs.len() || to >= refs.len() {
        return Err("no such preset position".into());
    }
    if from == to { return Ok(false); }
    let value = refs.remove(from);
    refs.insert(to, value);
    let profile = profile_index(doc)?;
    set_array(&mut doc.objects[profile].bytes, "LoadoutSets", &refs)?;
    Ok(true)
}

/// Read-only: the preset at `position` as a share code.
pub fn export(raw: &[u8], position: usize) -> Result<String, String> {
    super::loadout_code::encode(&export_preset(raw, position)?)
}

/// Read-only: the preset at `position`, parsed from the save.
pub fn export_preset(raw: &[u8], position: usize) -> Result<Preset, String> {
    let doc = Document::read(raw)?;
    let set_index = set_refs(&doc)?
        .get(position)
        .copied()
        .ok_or("no such preset")?;
    let set = &doc.objects[set_index as usize];
    if set.kind != "TAGame.LoadoutSet_TA" {
        return Err("preset reference is not a loadout set".into());
    }
    let name = read_string(&set.bytes, "LoadoutSetName")?;
    let loadouts = array_u32(&set.bytes, "Loadouts")?;
    if loadouts.len() != 2 {
        return Err("unsupported loadout set shape".into());
    }
    let inventory = inventory_index(&doc)?;
    let mut preset = Preset {
        name,
        ..Preset::default()
    };
    for (t, loadout) in loadouts.iter().enumerate() {
        let obj = &doc.objects[*loadout as usize].bytes;
        let products = array_u32(obj, "Products")?;
        let instances = instance_array(obj)?;
        if products.len() != SLOTS || instances.len() != SLOTS {
            return Err("unsupported loadout slot count".into());
        }
        let paint = &obj[span(obj, "TeamPaint")?.1];
        let mut team = Team {
            team: t as u8,
            set: true,
            team_color: scalar_or_zero(paint, "TeamColorID")? as u8,
            custom_color: scalar_or_zero(paint, "CustomColorID")? as u8,
            team_finish: scalar_or_zero(paint, "TeamFinishID")? as i32,
            custom_finish: scalar_or_zero(paint, "CustomFinishID")? as i32,
            ..Team::default()
        };
        for s in 0..SLOTS {
            team.products[s] = products[s] as i32;
            let (upper, lower) = instances[s];
            if upper == 0 && lower == 0 {
                continue;
            }
            let key = format!("{upper:016x}{lower:016x}");
            let idx = match preset
                .items
                .iter()
                .position(|it| it.instance_upper == upper && it.instance_lower == lower)
            {
                Some(i) => i,
                None => {
                    let item = match inventory.get(&key) {
                        Some(obj) => item_from_product(&doc, *obj)?,
                        None => Item {
                            product_id: team.products[s],
                            instance_upper: upper,
                            instance_lower: lower,
                            trade_hold: Some(0),
                            ..Item::default()
                        },
                    };
                    preset.items.push(item);
                    preset.items.len() - 1
                }
            };
            team.items[s] = Some(idx);
        }
        preset.teams[t] = team;
    }
    Ok(preset)
}

fn rename(doc: &mut Document, position: usize, name: &str) -> Result<bool, String> {
    if name.trim().is_empty() {
        return Err("preset name is empty".into());
    }
    let index = set_refs(doc)?
        .get(position)
        .copied()
        .ok_or("no such preset")? as usize;
    let obj = &mut doc.objects[index];
    if obj.kind != "TAGame.LoadoutSet_TA" {
        return Err("preset reference is not a loadout set".into());
    }
    let (props, _) = fields(&obj.bytes)?;
    let f = field(&props, "LoadoutSetName", "StrProperty")?;
    replace_field(&mut obj.bytes, f, &write_ue3(name));
    Ok(true)
}

fn delete(doc: &mut Document, position: usize) -> Result<bool, String> {
    let mut refs = set_refs(doc)?;
    if position >= refs.len() {
        return Err("no such preset".into());
    }
    if refs.len() <= 1 {
        return Err("keep at least one preset".into());
    }
    let profile = profile_index(doc)?;
    let equipped = read_scalar(&doc.objects[profile].bytes, "EquippedLoadoutSet")?;
    if equipped == refs[position] as i64 {
        return Err("equip another preset before deleting this one".into());
    }
    refs.remove(position);
    set_array(&mut doc.objects[profile].bytes, "LoadoutSets", &refs)?;
    Ok(true)
}

/// Appends one preset per code: missing items become inventory products,
/// then two `Loadout_TA` objects (blue, orange) and one `LoadoutSet_TA`.
fn import(doc: &mut Document, presets: &[Preset]) -> Result<bool, String> {
    if presets.is_empty() {
        return Ok(false);
    }
    let product_template = template(doc, "TAGame.OnlineProduct_TA")?;
    let loadout_template = template(doc, "TAGame.Loadout_TA")?;
    let set_template = template(doc, "TAGame.LoadoutSet_TA")?;
    let mut inventory = inventory_index(doc)?;
    let mut new_products = Vec::new();
    let mut new_sets = Vec::new();
    for preset in presets {
        if preset.name.trim().is_empty() {
            return Err("imported preset has no name".into());
        }
        let mut item_objects = Vec::with_capacity(preset.items.len());
        for item in &preset.items {
            if item.instance_upper == 0 && item.instance_lower == 0 {
                return Err("imported item has no instance ID".into());
            }
            let key = format!("{:016x}{:016x}", item.instance_upper, item.instance_lower);
            let index = match inventory.get(&key) {
                Some(&existing) => {
                    if scalar_or_zero(&doc.objects[existing].bytes, "ProductID")?
                        != item.product_id as i64
                    {
                        return Err(format!(
                            "An imported item conflicts with the save's inventory: {key}"
                        ));
                    }
                    existing
                }
                None => {
                    let index = push_product(doc, &product_template, item)?;
                    new_products.push(index as u32);
                    inventory.insert(key, index);
                    index
                }
            };
            item_objects.push(index);
        }
        let mut loadouts = Vec::with_capacity(2);
        for team in &preset.teams {
            let bytes = build_loadout(&loadout_template, team, preset)?;
            loadouts.push(push_object(doc, "TAGame.Loadout_TA", bytes));
        }
        let mut set = set_template.clone();
        let (props, _) = fields(&set)?;
        let f = field(&props, "LoadoutSetName", "StrProperty")?;
        replace_field(&mut set, f, &write_ue3(&preset.name));
        set_array(&mut set, "Loadouts", &loadouts)?;
        new_sets.push(push_object(doc, "TAGame.LoadoutSet_TA", set));
    }
    if !new_products.is_empty() {
        let mut refs = array_u32(&doc.root, "OnlineProducts")?;
        refs.extend(new_products);
        set_array(&mut doc.root, "OnlineProducts", &refs)?;
    }
    let profile = profile_index(doc)?;
    let mut refs = set_refs(doc)?;
    refs.extend(new_sets);
    set_array(&mut doc.objects[profile].bytes, "LoadoutSets", &refs)?;
    Ok(true)
}

fn template(doc: &Document, kind: &str) -> Result<Vec<u8>, String> {
    doc.objects
        .iter()
        .find(|o| o.kind == kind)
        .map(|o| o.bytes.clone())
        .ok_or_else(|| format!("save has no {kind} to copy from"))
}

fn push_object(doc: &mut Document, kind: &str, bytes: Vec<u8>) -> u32 {
    let index = doc.objects.len() as u32;
    doc.objects.push(Object {
        kind: kind.into(),
        index,
        bytes,
    });
    index
}

fn push_product(doc: &mut Document, template: &[u8], item: &Item) -> Result<usize, String> {
    let mut b = template.to_vec();
    put_scalar(&mut b, "ProductID", item.product_id as i64)?;
    put_scalar(&mut b, "SeriesID", item.series_id as i64)?;
    put_scalar(
        &mut b,
        "AddedTimestamp",
        i64::try_from(item.added_timestamp).map_err(|_| "timestamp too large")?,
    )?;
    // A missing TradeHold reads back as None, which is the zero default.
    put_scalar(&mut b, "TradeHold", item.trade_hold.unwrap_or(0) as i64)?;
    let (props, _) = fields(&b)?;
    let f = field(&props, "InstanceID", "StructProperty")?;
    let mut inst = b[f.payload.clone()].to_vec();
    put_qword(&mut inst, "UpperBits", item.instance_upper)?;
    put_qword(&mut inst, "LowerBits", item.instance_lower)?;
    replace_field(&mut b, f, &inst);
    let mut refs = Vec::with_capacity(item.attributes.len());
    for (key, value) in &item.attributes {
        refs.push(push_attribute(doc, key, value)?);
    }
    set_array(&mut b, "Attributes", &refs)?;
    Ok(push_object(doc, "TAGame.OnlineProduct_TA", b) as usize)
}

fn push_attribute(doc: &mut Document, key: &str, value: &str) -> Result<u32, String> {
    let (_, suffix, field_name) = ATTRS
        .iter()
        .find(|a| a.0 == key)
        .ok_or_else(|| format!("unsupported product attribute {key}"))?;
    let kind = format!("TAGame.ProductAttribute_{suffix}_TA");
    let mut b = template(doc, &kind)?;
    if !field_name.is_empty() {
        if span(&b, field_name)?.0 == "IntProperty" {
            let n = value
                .parse::<i64>()
                .map_err(|_| format!("invalid {key} value"))?;
            write_scalar(&mut b, field_name, n)?;
        } else {
            let tag = span(&b, field_name)?.0;
            let (props, _) = fields(&b)?;
            let f = field(&props, field_name, &tag)?;
            replace_field(&mut b, f, &write_ue3(value));
        }
    }
    Ok(push_object(doc, &kind, b))
}

/// Clone of `template` with products, instances and paint replaced.
fn build_loadout(template: &[u8], team: &Team, preset: &Preset) -> Result<Vec<u8>, String> {
    let mut b = template.to_vec();
    let products: Vec<u32> = team.products.iter().map(|p| *p as u32).collect();
    set_array(&mut b, "Products", &products)?;
    let mut payload = (SLOTS as u32).to_le_bytes().to_vec();
    for slot in &team.items {
        let (upper, lower) = match slot {
            Some(i) => {
                let it = preset.items.get(*i).ok_or("item reference out of range")?;
                (it.instance_upper, it.instance_lower)
            }
            None => (0, 0),
        };
        payload.extend(instance_blob(upper, lower));
    }
    let (props, _) = fields(&b)?;
    let f = field(&props, "OnlineProducts128", "ArrayProperty")?;
    replace_field(&mut b, f, &payload);
    write_team_paint(&mut b, &[
        ("TeamColorID", team.team_color as i64),
        ("CustomColorID", team.custom_color as i64),
        ("TeamFinishID", team.team_finish as i64),
        ("CustomFinishID", team.custom_finish as i64),
    ])?;
    Ok(b)
}

fn write_team_paint(data: &mut Vec<u8>, updates: &[(&str, i64)]) -> Result<(), String> {
    let (props, _) = fields(data)?;
    let f = field(&props, "TeamPaint", "StructProperty")?;
    let mut paint = data[f.payload.clone()].to_vec();
    for &(name, value) in updates {
        put_scalar(&mut paint, name, value)?;
    }
    replace_field(data, f, &paint);
    Ok(())
}

fn scalar_or_zero(data: &[u8], name: &str) -> Result<i64, String> {
    match span(data, name) {
        Ok(_) => read_scalar(data, name),
        Err(_) => Ok(0),
    }
}

fn instance_blob(upper: u64, lower: u64) -> Vec<u8> {
    let qword = |name: &str, v: u64| {
        let mut out = write_ue3(name);
        out.extend(write_ue3("QWordProperty"));
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
        out
    };
    let mut out = qword("UpperBits", upper);
    out.extend(qword("LowerBits", lower));
    out.extend(write_ue3("None"));
    out
}

fn write_scalar(data: &mut [u8], name: &str, value: i64) -> Result<(), String> {
    let (tag, r) = span(data, name)?;
    match (tag.as_str(), r.len()) {
        ("ByteProperty", 1) => {
            data[r.start] = u8::try_from(value).map_err(|_| format!("{name} out of range"))?;
        }
        ("IntProperty" | "ObjectProperty", 4) => {
            let v = i32::try_from(value).map_err(|_| format!("{name} out of range"))?;
            data[r].copy_from_slice(&v.to_le_bytes());
        }
        ("QWordProperty", 8) => data[r].copy_from_slice(&value.to_le_bytes()),
        _ => return Err(format!("unsupported scalar {name}")),
    }
    Ok(())
}

/// Writes a scalar, inserting it when absent. UE3 omits default (zero) values.
fn put_scalar(data: &mut Vec<u8>, name: &str, value: i64) -> Result<(), String> {
    if span(data, name).is_ok() {
        return write_scalar(data, name, value);
    }
    if value == 0 {
        return Ok(());
    }
    match i32::try_from(value) {
        Ok(v) => insert_property(data, name, "IntProperty", &v.to_le_bytes()),
        Err(_) => insert_property(data, name, "QWordProperty", &value.to_le_bytes()),
    }
}

fn put_qword(data: &mut Vec<u8>, name: &str, value: u64) -> Result<(), String> {
    if span(data, name).is_ok() {
        return write_qword(data, name, value);
    }
    if value == 0 {
        return Ok(());
    }
    insert_property(data, name, "QWordProperty", &value.to_le_bytes())
}

fn write_qword(data: &mut [u8], name: &str, value: u64) -> Result<(), String> {
    let (tag, r) = span(data, name)?;
    if tag != "QWordProperty" || r.len() != 8 {
        return Err(format!("unsupported instance field {name}"));
    }
    data[r].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Inserts a property before the trailing `None` of a property stream.
fn insert_property(
    data: &mut Vec<u8>,
    name: &str,
    tag: &str,
    payload: &[u8],
) -> Result<(), String> {
    let none = write_ue3("None");
    if data.len() < none.len() || data[data.len() - none.len()..] != none[..] {
        return Err("unsupported property stream ending".into());
    }
    let mut header = write_ue3(name);
    header.extend(write_ue3(tag));
    header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(payload);
    let at = data.len() - none.len();
    data.splice(at..at, header);
    Ok(())
}

/// Adds an array property for objects that lack it.
fn append_array(data: &mut Vec<u8>, name: &str, values: &[u32]) -> Result<(), String> {
    let mut payload = (values.len() as u32).to_le_bytes().to_vec();
    for v in values {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    insert_property(data, name, "ArrayProperty", &payload)
}

// Shared by export and import.

fn profile_index(doc: &Document) -> Result<usize, String> {
    doc.objects
        .iter()
        .position(|o| o.kind == "TAGame.ProfileLoadoutSave_TA")
        .ok_or_else(|| "save has no loadout profile".into())
}

fn set_refs(doc: &Document) -> Result<Vec<u32>, String> {
    array_u32(&doc.objects[profile_index(doc)?].bytes, "LoadoutSets")
}

/// Span (tag, payload) of the named property. Errors when missing.
fn span(data: &[u8], name: &str) -> Result<(String, Range<usize>), String> {
    let (props, _) = fields(data).map_err(|e| format!("{name}: {e}"))?;
    let f = props
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("missing {name}"))?;
    Ok((f.tag.clone(), f.payload.clone()))
}

/// An object array is either one `ArrayProperty` or repeated indexed `ObjectProperty` entries.
fn array_u32(data: &[u8], name: &str) -> Result<Vec<u32>, String> {
    let (props, _) = fields(data)?;
    let entries: Vec<&Field> = props.iter().filter(|f| f.name == name).collect();
    match entries.as_slice() {
        [] => Err(format!("missing {name}")),
        [f] if f.tag == "ArrayProperty" => int_array(&data[f.payload.clone()]),
        _ => entries
            .iter()
            .map(|f| match f.tag.as_str() {
                "ObjectProperty" => Ok(u32::from_le_bytes(
                    data[f.payload.clone()].try_into().unwrap(),
                )),
                _ => Err(format!("unsupported array {name}")),
            })
            .collect(),
    }
}

/// Replaces an object array's contents. Keeps its shape; appends an `ArrayProperty` if absent.
fn set_array(data: &mut Vec<u8>, name: &str, values: &[u32]) -> Result<(), String> {
    let (props, _) = fields(data)?;
    let entries: Vec<&Field> = props.iter().filter(|f| f.name == name).collect();
    match entries.as_slice() {
        [f] if f.tag == "ArrayProperty" => {
            let mut payload = (values.len() as u32).to_le_bytes().to_vec();
            for v in values {
                payload.extend_from_slice(&v.to_le_bytes());
            }
            replace_field(data, f, &payload);
            Ok(())
        }
        [] if values.is_empty() => Ok(()),
        [] => append_array(data, name, values),
        _ if entries.iter().all(|f| f.tag == "ObjectProperty") => {
            let mut out = Vec::with_capacity(data.len());
            let mut cur = 0;
            for f in &entries {
                out.extend_from_slice(&data[cur..f.head]);
                cur = f.payload.end;
            }
            out.extend_from_slice(&data[cur..]);
            let none = write_ue3("None");
            let at = out.len() - none.len();
            let mut entries_bytes = Vec::new();
            for (i, v) in values.iter().enumerate() {
                entries_bytes.extend(write_ue3(name));
                entries_bytes.extend(write_ue3("ObjectProperty"));
                entries_bytes.extend_from_slice(&4u32.to_le_bytes());
                entries_bytes.extend_from_slice(&(i as u32).to_le_bytes());
                entries_bytes.extend_from_slice(&v.to_le_bytes());
            }
            out.splice(at..at, entries_bytes);
            *data = out;
            Ok(())
        }
        _ => Err(format!("unsupported array {name}")),
    }
}

fn read_scalar(data: &[u8], name: &str) -> Result<i64, String> {
    let (tag, r) = span(data, name)?;
    match (tag.as_str(), r.len()) {
        ("ByteProperty", 1) => Ok(data[r.start] as i64),
        ("IntProperty" | "ObjectProperty", 4) => {
            Ok(i32::from_le_bytes(data[r].try_into().unwrap()) as i64)
        }
        ("QWordProperty", 8) => Ok(i64::from_le_bytes(data[r].try_into().unwrap())),
        _ => Err(format!("unsupported scalar {name}")),
    }
}

fn read_string(data: &[u8], name: &str) -> Result<String, String> {
    let (tag, r) = span(data, name)?;
    // Enum-valued attributes (Quality) are ByteProperty with a name payload.
    if !matches!(
        tag.as_str(),
        "StrProperty" | "NameProperty" | "ByteProperty"
    ) {
        return Err(format!("unsupported string {name}"));
    }
    Ok(text(&data[r], 0)?.0)
}

fn instance_array(data: &[u8]) -> Result<Vec<(u64, u64)>, String> {
    let (_, r) = span(data, "OnlineProducts128")?;
    let arr = &data[r];
    let count = u32::from_le_bytes(
        arr.get(..4)
            .ok_or("truncated instance array")?
            .try_into()
            .unwrap(),
    );
    let mut at = 4;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (props, end) = fields(&arr[at..])?;
        let mut pair = (0u64, 0u64);
        for p in props {
            let v = u64::from_le_bytes(
                arr[at + p.payload.start..at + p.payload.end]
                    .try_into()
                    .unwrap(),
            );
            match p.name.as_str() {
                "UpperBits" => pair.0 = v,
                "LowerBits" => pair.1 = v,
                _ => return Err("unknown instance field".into()),
            }
        }
        out.push(pair);
        at += end;
    }
    Ok(out)
}

/// Root `OnlineProducts` membership, keyed by instance ID (lowercase hex).
fn inventory_index(doc: &Document) -> Result<HashMap<String, usize>, String> {
    let (props, _) = fields(&doc.root)?;
    let f = field(&props, "OnlineProducts", "ArrayProperty")?;
    let mut map = HashMap::new();
    for idx in int_array(&doc.root[f.payload.clone()])? {
        let obj = doc
            .objects
            .get(idx as usize)
            .ok_or("invalid inventory reference")?;
        let (_, r) = span(&obj.bytes, "InstanceID")?;
        let (key, _, _) = instance(&obj.bytes[r])?;
        map.insert(key, idx as usize);
    }
    Ok(map)
}

fn item_from_product(doc: &Document, index: usize) -> Result<Item, String> {
    let obj: &Object = &doc.objects[index];
    let (_, r) = span(&obj.bytes, "InstanceID")?;
    let (key, _, _) = instance(&obj.bytes[r])?;
    let (upper, lower) = (
        u64::from_str_radix(&key[..16], 16).map_err(|_| "invalid instance")?,
        u64::from_str_radix(&key[16..], 16).map_err(|_| "invalid instance")?,
    );
    let mut attributes = Vec::new();
    if span(&obj.bytes, "Attributes").is_ok() {
        for a in array_u32(&obj.bytes, "Attributes")? {
            attributes.push(attribute_pair(doc, a as usize)?);
        }
    }
    Ok(Item {
        product_id: scalar_or_zero(&obj.bytes, "ProductID")? as i32,
        instance_upper: upper,
        instance_lower: lower,
        series_id: scalar_or_zero(&obj.bytes, "SeriesID")? as i32,
        added_timestamp: u64::try_from(scalar_or_zero(&obj.bytes, "AddedTimestamp")?)
            .map_err(|_| "negative timestamp")?,
        trade_hold: Some(scalar_or_zero(&obj.bytes, "TradeHold")? as i32),
        attributes,
    })
}

fn attribute_pair(doc: &Document, index: usize) -> Result<(String, String), String> {
    let obj = doc
        .objects
        .get(index)
        .ok_or("invalid attribute reference")?;
    let (key, field_name) = ATTRS
        .iter()
        .find(|(_, suffix, _)| obj.kind == format!("TAGame.ProductAttribute_{suffix}_TA"))
        .map(|(k, _, f)| (*k, *f))
        .ok_or_else(|| format!("unsupported product attribute {}", obj.kind))?;
    let value = match field_name {
        "" => String::new(),
        name => match span(&obj.bytes, name)?.0.as_str() {
            "IntProperty" => read_scalar(&obj.bytes, name)?.to_string(),
            _ => read_string(&obj.bytes, name)?,
        },
    };
    Ok((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Vec<u8> { write_ue3("None") }
    fn obj(kind: &str, index: u32, bytes: Vec<u8>) -> Object {
        Object { kind: kind.into(), index, bytes }
    }

    #[test]
    fn changing_team_colours_preserves_the_other_team_and_paint_finishes() {
        let mut profile = empty();
        append_array(&mut profile, "LoadoutSets", &[1]).unwrap();
        let mut set = empty();
        append_array(&mut set, "Loadouts", &[2, 2]).unwrap();
        let mut paint = empty();
        put_scalar(&mut paint, "TeamFinishID", 43).unwrap();
        let mut loadout = write_ue3("TeamPaint");
        loadout.extend(write_ue3("StructProperty"));
        loadout.extend((paint.len() as u32).to_le_bytes());
        loadout.extend(0u32.to_le_bytes());
        loadout.extend(write_ue3("TeamPaint"));
        loadout.extend(paint);
        loadout.extend(empty());
        let mut doc = Document { header: vec![], root: empty(), tail: vec![], objects: vec![
            obj("TAGame.ProfileLoadoutSave_TA", 0, profile),
            obj("TAGame.LoadoutSet_TA", 1, set),
            obj("TAGame.Loadout_TA", 2, loadout.clone()),
        ]};
        assert!(update_colours(&mut doc, 0, [[0, 0], [19, 12]], &Preset::default()).unwrap());
        assert_eq!(array_u32(&doc.objects[1].bytes, "Loadouts").unwrap(), [2, 3]);
        assert_eq!(doc.objects[2].bytes, loadout);
        let orange = &doc.objects[3].bytes;
        let paint = &orange[span(orange, "TeamPaint").unwrap().1];
        assert_eq!(read_scalar(paint, "TeamColorID").unwrap(), 19);
        assert_eq!(read_scalar(paint, "CustomColorID").unwrap(), 12);
        assert_eq!(read_scalar(paint, "TeamFinishID").unwrap(), 43);
    }

    #[test]
    fn moving_presets_preserves_equipped_reference_and_objects() {
        let mut profile = empty();
        append_array(&mut profile, "LoadoutSets", &[1, 2, 3, 4]).unwrap();
        insert_property(&mut profile, "EquippedLoadoutSet", "ObjectProperty", &2u32.to_le_bytes()).unwrap();
        let mut doc = Document { header: vec![], root: empty(), tail: vec![], objects: vec![obj("TAGame.ProfileLoadoutSave_TA", 0, profile)] };
        assert!(move_preset(&mut doc, 0, 3).unwrap());
        assert_eq!(set_refs(&doc).unwrap(), vec![2, 3, 4, 1]);
        assert_eq!(read_scalar(&doc.objects[0].bytes, "EquippedLoadoutSet").unwrap(), 2);
        assert_eq!(doc.objects.len(), 1);
        assert!(!move_preset(&mut doc, 1, 1).unwrap());
        assert!(move_preset(&mut doc, 0, 4).is_err());
    }

    #[test]
    fn equipping_checks_ownership_and_updates_both_slot_arrays() {
        let mut root = empty();
        append_array(&mut root, "OnlineProducts", &[4]).unwrap();
        let mut profile = empty();
        append_array(&mut profile, "LoadoutSets", &[1]).unwrap();
        let mut set = empty();
        append_array(&mut set, "Loadouts", &[2, 3]).unwrap();
        let mut loadout = empty();
        append_array(&mut loadout, "Products", &[0; SLOTS]).unwrap();
        let mut instances = (SLOTS as u32).to_le_bytes().to_vec();
        for _ in 0..SLOTS { instances.extend(instance_blob(0, 0)); }
        insert_property(&mut loadout, "OnlineProducts128", "ArrayProperty", &instances).unwrap();
        let mut product = empty();
        insert_property(&mut product, "ProductID", "IntProperty", &42u32.to_le_bytes()).unwrap();
        let instance = instance_blob(7, 9);
        let mut instance_field = write_ue3("InstanceID");
        instance_field.extend(write_ue3("StructProperty"));
        instance_field.extend((instance.len() as u32).to_le_bytes());
        instance_field.extend(0u32.to_le_bytes());
        instance_field.extend(write_ue3("ProductInstanceID"));
        instance_field.extend(instance);
        product.splice(0..0, instance_field);
        let mut doc = Document { header: vec![], root, tail: vec![], objects: vec![
            obj("TAGame.ProfileLoadoutSave_TA", 0, profile),
            obj("TAGame.LoadoutSet_TA", 1, set),
            obj("TAGame.Loadout_TA", 2, loadout.clone()),
            obj("TAGame.Loadout_TA", 3, loadout),
            obj("TAGame.OnlineProduct_TA", 4, product),
        ]};
        assert!(equip(&mut doc, 0, &[(0, 2, 7, 9)]).unwrap());
        assert_eq!(array_u32(&doc.objects[2].bytes, "Products").unwrap()[2], 42);
        assert_eq!(instance_array(&doc.objects[2].bytes).unwrap()[2], (7, 9));
        assert_eq!(array_u32(&doc.objects[3].bytes, "Products").unwrap()[2], 0);
        assert!(equip(&mut doc, 0, &[(0, 2, 7, 10)]).is_err());
    }
}
