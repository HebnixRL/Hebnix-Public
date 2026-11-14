//! Append-only ABC constants and validated title getter overrides.
use super::TitleOverride;
use std::{collections::HashMap, ops::Range};

type Result<T> = std::result::Result<T, String>;
const LIMIT: usize = 1_000_000;
const MARKER: &str = "HEBNIX_TITLE_UPK_V1";

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or("ABC offset overflow")?;
        let value = self.data.get(self.at..end).ok_or("Truncated ABC")?;
        self.at = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn uint(&mut self) -> Result<u32> {
        let mut value = 0;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            if shift == 28 && byte > 15 {
                return Err("Invalid ABC integer".into());
            }
            value |= ((byte & 127) as u32) << shift;
            if byte < 128 {
                return Ok(value);
            }
        }
        Err("Invalid ABC integer".into())
    }
    fn index(&mut self) -> Result<usize> {
        let value = self.uint()? as usize;
        if value > 0x3fff_ffff {
            return Err("Invalid ABC u30".into());
        }
        Ok(value)
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.index()?;
        if n > LIMIT {
            return Err("ABC count exceeds supported limit".into());
        }
        Ok(n)
    }
}
fn put(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n != 0 { 128 } else { 0 });
        if n == 0 {
            break;
        }
    }
}
fn op(out: &mut Vec<u8>, opcode: u8, value: usize) {
    out.push(opcode);
    put(out, value);
}
#[derive(Clone)]
struct Name {
    full: String,
    local: String,
}
#[derive(Clone)]
struct Trait {
    name: usize,
    method: Option<usize>,
    kind: u8,
}
#[derive(Clone)]
struct Method {
    params: Vec<usize>,
    returns: usize,
}
#[derive(Clone)]
struct Class {
    name: usize,
    init: usize,
    fields: Vec<Trait>,
    statics: Vec<Trait>,
}
struct Body {
    id: usize,
    span: Range<usize>,
    code: Range<usize>,
    stack: usize,
    locals: usize,
    init_scope: usize,
    max_scope: usize,
    exceptions: usize,
}
struct Pool<T> {
    span: Range<usize>,
    count: usize,
    entries: usize,
    values: Vec<T>,
}
struct Abc {
    uints: Pool<u32>,
    strings: Pool<String>,
    names: Vec<Name>,
    methods: Vec<Method>,
    classes: Vec<Class>,
    bodies: Vec<Body>,
}

fn traits(r: &mut Reader<'_>) -> Result<Vec<Trait>> {
    let mut out = Vec::new();
    for _ in 0..r.count()? {
        let name = r.index()?;
        let flags = r.byte()?;
        let kind = flags & 15;
        let method = match kind {
            0 | 6 => {
                r.index()?;
                r.index()?;
                if r.index()? != 0 {
                    r.byte()?;
                }
                None
            }
            1..=3 => {
                r.index()?;
                Some(r.index()?)
            }
            4 | 5 => {
                r.index()?;
                r.index()?;
                None
            }
            _ => return Err("Unsupported ABC trait".into()),
        };
        if flags & 64 != 0 {
            for _ in 0..r.count()? {
                r.index()?;
            }
        }
        out.push(Trait { name, method, kind });
    }
    Ok(out)
}
impl Abc {
    fn name(&self, index: usize) -> Result<&Name> {
        self.names
            .get(index)
            .ok_or("Invalid ABC name reference".into())
    }
    fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader { data, at: 0 };
        if r.take(4)? != [16, 0, 46, 0] {
            return Err("Unsupported title ABC version".into());
        }
        for _ in 1..r.count()? {
            r.uint()?;
        }
        let start = r.at;
        let count = r.count()?;
        let entries = r.at;
        let mut uints = vec![0];
        for _ in 1..count {
            uints.push(r.uint()?);
        }
        let uints = Pool {
            span: start..r.at,
            count,
            entries,
            values: uints,
        };
        let doubles = r.count()?.saturating_sub(1);
        r.take(doubles.checked_mul(8).ok_or("ABC double overflow")?)?;
        let start = r.at;
        let count = r.count()?;
        let entries = r.at;
        let mut strings = vec![String::new()];
        for _ in 1..count {
            let size = r.count()?;
            strings.push(
                std::str::from_utf8(r.take(size)?)
                    .map_err(|_| "Invalid ABC UTF-8")?
                    .into(),
            );
        }
        let strings = Pool {
            span: start..r.at,
            count,
            entries,
            values: strings,
        };
        let mut ns = vec![String::new()];
        for _ in 1..r.count()? {
            r.byte()?;
            let index = r.index()?;
            ns.push(
                strings
                    .values
                    .get(index)
                    .ok_or("Invalid namespace string")?
                    .clone(),
            );
        }
        for _ in 1..r.count()? {
            for _ in 0..r.count()? {
                r.index()?;
            }
        }
        let mut names = vec![Name {
            full: String::new(),
            local: String::new(),
        }];
        for _ in 1..r.count()? {
            let (namespace, name) = match r.byte()? {
                0x07 | 0x0d => {
                    let namespace = r.index()?;
                    (Some(namespace), Some(r.index()?))
                }
                0x0f | 0x10 => (None, Some(r.index()?)),
                0x11 | 0x12 => (None, None),
                0x09 | 0x0e => {
                    let name = r.index()?;
                    r.index()?;
                    (None, Some(name))
                }
                0x1b | 0x1c => {
                    r.index()?;
                    (None, None)
                }
                0x1d => {
                    let base = r.index()?;
                    for _ in 0..r.count()? {
                        r.index()?;
                    }
                    names.push(names.get(base).ok_or("Invalid ABC generic name")?.clone());
                    continue;
                }
                _ => return Err("Unsupported ABC multiname".into()),
            };
            let local = name
                .map(|index| {
                    strings
                        .values
                        .get(index)
                        .cloned()
                        .ok_or("Invalid ABC name string")
                })
                .transpose()?
                .unwrap_or_default();
            let namespace = namespace
                .map(|index| ns.get(index).cloned().ok_or("Invalid ABC namespace"))
                .transpose()?
                .unwrap_or_default();
            names.push(Name {
                full: format!("{namespace}:{local}"),
                local,
            });
        }
        let mut methods = Vec::new();
        for _ in 0..r.count()? {
            let params = r.count()?;
            let returns = r.index()?;
            let params = (0..params).map(|_| r.index()).collect::<Result<Vec<_>>>()?;
            r.index()?;
            let flags = r.byte()?;
            if flags & 8 != 0 {
                for _ in 0..r.count()? {
                    r.index()?;
                    r.byte()?;
                }
            }
            if flags & 128 != 0 {
                for _ in &params {
                    r.index()?;
                }
            }
            methods.push(Method { params, returns });
        }
        for _ in 0..r.count()? {
            r.index()?;
            for _ in 0..r.count()?.checked_mul(2).ok_or("Metadata overflow")? {
                r.index()?;
            }
        }
        let mut classes = Vec::new();
        for _ in 0..r.count()? {
            let name = r.index()?;
            r.index()?;
            if r.byte()? & 8 != 0 {
                r.index()?;
            }
            for _ in 0..r.count()? {
                r.index()?;
            }
            let init = r.index()?;
            classes.push(Class {
                name,
                init,
                fields: traits(&mut r)?,
                statics: Vec::new(),
            });
        }
        for class in &mut classes {
            r.index()?;
            class.statics = traits(&mut r)?;
        }
        for _ in 0..r.count()? {
            r.index()?;
            traits(&mut r)?;
        }
        let mut bodies = Vec::new();
        for _ in 0..r.count()? {
            let start = r.at;
            let id = r.index()?;
            let stack = r.index()?;
            let locals = r.index()?;
            let init_scope = r.index()?;
            let max_scope = r.index()?;
            let len = r.index()?;
            let code = r.at..r.at.checked_add(len).ok_or("ABC code overflow")?;
            r.take(len)?;
            let exceptions = r.count()?;
            for _ in 0..exceptions {
                for _ in 0..5 {
                    r.index()?;
                }
            }
            traits(&mut r)?;
            bodies.push(Body {
                id,
                span: start..r.at,
                code,
                stack,
                locals,
                init_scope,
                max_scope,
                exceptions,
            });
        }
        if r.at != data.len() {
            return Err("Unexpected ABC trailer".into());
        }
        Ok(Self {
            uints,
            strings,
            names,
            methods,
            classes,
            bodies,
        })
    }
}
/// Title edits append constants and change their own getters. Rank edits
/// change different getters and use existing names and inline constants.
pub(super) fn revert(live: &[u8], before: &[u8], after: &[u8]) -> Result<Option<Vec<u8>>> {
    if before == after || live == before {
        return Ok(None);
    }
    if live == after {
        return Ok(Some(before.to_vec()));
    }
    let (current, old, new) = (Abc::parse(live)?, Abc::parse(before)?, Abc::parse(after)?);
    let header_end = |abc: &Abc, len| abc.bodies.first().map_or(len, |body| body.span.start);
    let live_header = &live[..header_end(&current, live.len())];
    let old_header = &before[..header_end(&old, before.len())];
    let new_header = &after[..header_end(&new, after.len())];
    let mut output = match select_original(
        live_header,
        old_header,
        new_header,
        "ABC header (including constant pools)",
    ) {
        Ok(header) => header.to_vec(),
        Err(error) => {
            let outside_pools = |header: &[u8], abc: &Abc| {
                [
                    &header[..abc.uints.span.start],
                    &header[abc.uints.span.end..abc.strings.span.start],
                    &header[abc.strings.span.end..],
                ]
                .concat()
            };
            let unchanged = outside_pools(old_header, &old);
            if !new.strings.values.starts_with(&old.strings.values)
                || !current.strings.values.starts_with(&new.strings.values)
                || !new.uints.values.starts_with(&old.uints.values)
                || !current.uints.values.starts_with(&new.uints.values)
                || outside_pools(new_header, &new) != unchanged
                || outside_pools(live_header, &current) != unchanged
                || old.strings.values.iter().any(|value| value == MARKER)
                || current.strings.values.iter().filter(|value| *value == MARKER).count() != 1
            {
                return Err(error);
            }
            let marker = new.strings.values.iter().position(|value| value == MARKER).ok_or(error)?;
            // Keep all indexes stable. Retire the patch's bookkeeping string
            // so the restored getters can be patched again.
            let mut reader = Reader { data: live_header, at: current.strings.entries };
            for _ in 1..marker {
                let len = reader.index()?;
                reader.take(len)?;
            }
            let len = reader.index()?;
            let end = reader.at;
            reader.take(len)?;
            let mut header = live_header.to_vec();
            header[end..reader.at].fill(b'_');
            header
        }
    };

    if current.bodies.len() != old.bodies.len() || current.bodies.len() != new.bodies.len() {
        return Err("Method count changed in shared UI bytecode".into());
    }
    for ((current, old), new) in current.bodies.iter().zip(&old.bodies).zip(&new.bodies) {
        if current.id != old.id || current.id != new.id {
            return Err("Method order changed in shared UI bytecode".into());
        }
        output.extend_from_slice(select_original(
            &live[current.span.clone()],
            &before[old.span.clone()],
            &after[new.span.clone()],
            &format!("ABC method {}", current.id),
        )?);
    }
    if output == live {
        return Ok(None);
    }
    Abc::parse(&output)?;
    Ok(Some(output))
}

fn select_original<'a>(
    live: &'a [u8],
    before: &'a [u8],
    after: &[u8],
    section: &str,
) -> Result<&'a [u8]> {
    if before == after || live == before {
        Ok(live)
    } else if live == after {
        Ok(before)
    } else {
        Err(format!("Another edit changed the same UI bytecode in the {section}"))
    }
}

struct Constants<'a> {
    strings: &'a mut Vec<String>,
    uints: &'a mut Vec<u32>,
}
#[derive(Clone, Copy)]
enum TitleIdSource {
    Property(usize),
    Indexed { class: usize, getter: usize },
}
impl Constants<'_> {
    fn string(&mut self, value: &str) -> usize {
        if let Some(index) = self.strings.iter().position(|s| s == value) {
            return index;
        }
        self.strings.push(value.into());
        self.strings.len() - 1
    }
    fn uint(&mut self, value: u32) -> usize {
        if let Some(index) = self.uints.iter().position(|v| *v == value) {
            return index;
        }
        self.uints.push(value);
        self.uints.len() - 1
    }
}
fn branch(out: &mut Vec<u8>, opcode: u8) -> usize {
    out.push(opcode);
    let at = out.len();
    out.extend([0; 3]);
    at
}
fn bind(out: &mut [u8], at: usize, end: usize) -> Result<()> {
    let relative = i32::try_from(end).map_err(|_| "Branch overflow")?
        - i32::try_from(at + 3).map_err(|_| "Branch overflow")?;
    if !(-0x800000..=0x7fffff).contains(&relative) {
        return Err("Title branch too large".into());
    }
    out[at..at + 3].copy_from_slice(&relative.to_le_bytes()[..3]);
    Ok(())
}
fn literal(out: &mut Vec<u8>, field: &str, rule: &TitleOverride, constants: &mut Constants<'_>) {
    if matches!(field, "Text" | "TitleText" | "XPTitle") {
        op(out, 0x2c, constants.string(&rule.text));
    } else {
        op(
            out,
            0x2e,
            constants.uint(if matches!(field, "GlowColor" | "TitleGlowColor") {
                if rule.glow {
                    rule.glow_color.unwrap_or(rule.color)
                } else {
                    0
                }
            } else {
                rule.color
            }),
        );
    }
}
fn skip_debug(r: &mut Reader<'_>) -> Result<()> {
    loop {
        match r.data.get(r.at) {
            Some(0xf0 | 0xf1) => {
                r.byte()?;
                r.index()?;
            }
            Some(0xef) => {
                r.byte()?;
                r.byte()?;
                r.index()?;
                r.byte()?;
                r.index()?;
            }
            _ => break,
        }
    }
    Ok(())
}

fn prologue_len(code: &[u8]) -> Result<usize> {
    let mut r = Reader { data: code, at: 0 };
    skip_debug(&mut r)?;
    match r.byte()? {
        0xd0 => {}                  // getlocal_0
        0x62 if r.index()? == 0 => {} // getlocal 0
        _ => return Err("Unsupported title method prologue".into()),
    }
    skip_debug(&mut r)?;
    if r.byte()? != 0x30 {
        return Err("Unsupported title method prologue".into());
    }
    Ok(r.at)
}

fn modified_xp_title(code: &[u8]) -> bool {
    let mut r = Reader { data: code, at: 0 };
    r.byte().ok() == Some(0xd0)
        && r.byte().ok() == Some(0x66) // getproperty
        && r.index().is_ok()
        && r.byte().ok() == Some(0x2e) // pushuint
        && r.index().is_ok()
        && r.byte().ok() == Some(0x15) // iflt
        && r.take(3).is_ok()
}

fn has_modified_row_xp_title(abc: &Abc, data: &[u8], class: &Class) -> bool {
    class.fields.iter().any(|entry| {
        entry.kind == 2
            && abc.name(entry.name).is_ok_and(|name| name.local == "XPTitle")
            && abc.bodies.iter().any(|body| {
                Some(body.id) == entry.method && modified_xp_title(&data[body.code.clone()])
            })
    })
}

/// Extract only the known straight-line native DataAPI getter expression.
/// Never inline a branch, return, exception handler, or an already patched getter.
fn native_player_value<'a>(
    abc: &Abc,
    data: &'a [u8],
    class: &Class,
    field: &str,
    source: &TitleIdSource,
) -> Result<&'a [u8]> {
    let indexed = matches!(source, TitleIdSource::Indexed { .. });
    let name = if indexed {
        format!("Get{field}")
    } else {
        field.into()
    };
    let entries = if indexed {
        &class.statics
    } else {
        &class.fields
    };
    let method = entries
        .iter()
        .find(|entry| abc.name(entry.name).is_ok_and(|value| value.local == name))
        .and_then(|entry| entry.method)
        .ok_or("Missing native match title getter")?;
    let body = abc
        .bodies
        .iter()
        .find(|body| body.id == method)
        .ok_or("Missing match title getter body")?;
    let code = &data[body.code.clone()];
    let owner = &abc.name(class.name)?.full;
    let start = prologue_len(code).map_err(|error| {
        format!("{error} in {owner}.{name} (starts with {:02x?})", &code[..code.len().min(8)])
    })?;
    let mut r = Reader {
        data: code,
        at: start,
    };
    // Debug builds also put debugline/debug register metadata immediately
    // after pushscope. It is not part of the native getter expression.
    skip_debug(&mut r)?;
    let start = r.at;
    let valid = body.exceptions == 0
        && r.byte()? == 0x60
        && abc.name(r.index()?)?.full == "projectx.api:DataAPI"
        && r.byte()? == 0x2c
        && abc
            .strings
            .values
            .get(r.index()?)
            .is_some_and(|value| value == "PlayerInfo")
        && if indexed {
            r.byte()? == 0xd1
        } else {
            r.byte()? == 0xd0 && r.byte()? == 0x66 && abc.name(r.index()?)?.local == "RowIndex"
        }
        && r.byte()? == 0x2c
        && abc
            .strings
            .values
            .get(r.index()?)
            .is_some_and(|value| value == field)
        && r.byte()? == 0x46
        && abc.name(r.index()?)?.local == "getValue"
        && r.index()? == 3
        && r.byte()? == 0x48
        && r.at == code.len();
    if !valid {
        return Err(format!(
            "Unsupported native match title getter layout: {name}"
        ));
    }
    Ok(&code[start..code.len() - 1])
}

/// Resolve the local player's equipped catalog ID through the same table used
/// by LocalPlayerView. PlayerInfo.PrimaryTitle is not a reliable equipped ID.
fn local_selected_title(
    abc: &Abc,
    api: &Class,
    code: &mut Vec<u8>,
    local_player: &[u8],
    field: &str,
    rules: &[TitleOverride],
    lower: usize,
    constants: &mut Constants<'_>,
) -> Result<()> {
    let Some(table) = abc.classes.iter().find(|class| {
        abc.name(class.name)
            .is_ok_and(|name| name.full == "tagame.api:PlayerTitlesPlayerTitles")
    }) else {
        return Err("Missing equipped title table".into());
    };
    let checked =
        |class: &Class, name: &str, kind: u8, params: &[&str], returns: &str| -> Result<usize> {
            let entry = class
                .statics
                .iter()
                .find(|entry| {
                    entry.kind == kind
                        && abc.name(entry.name).is_ok_and(|value| value.local == name)
                })
                .ok_or_else(|| format!("Missing equipped title method {name}"))?;
            let method = entry
                .method
                .and_then(|id| abc.methods.get(id))
                .ok_or("Missing equipped title signature")?;
            if method.params.len() != params.len()
                || abc.name(method.returns)?.full != returns
                || method.params.iter().zip(params).any(|(index, expected)| {
                    !abc.name(*index).is_ok_and(|name| name.full == *expected)
                })
            {
                return Err(format!("Unsupported equipped title signature: {name}"));
            }
            Ok(entry.name)
        };
    let selected = checked(api, "SelectedTitle", 2, &[], ":int")?;
    let count = checked(table, "getRowCount", 1, &[], ":int")?;
    let id = checked(table, "GetId", 1, &[":int"], ":String")?;
    let index = |code: &mut Vec<u8>| {
        op(code, 0x60, api.name);
        op(code, 0x66, selected);
    };
    let source = |code: &mut Vec<u8>| {
        op(code, 0x60, table.name);
        index(code);
        op(code, 0x46, id);
        put(code, 1);
    };
    code.extend_from_slice(local_player);
    let remote = branch(code, 0x12); // iffalse
    index(code);
    code.extend([0x24, 0]); // pushbyte 0
    let negative = branch(code, 0x15); // iflt
    index(code);
    op(code, 0x60, table.name);
    op(code, 0x46, count);
    put(code, 0);
    let outside = branch(code, 0x18); // ifge
    source(code);
    code.push(0x20);
    let null = branch(code, 0x13);
    for rule in rules.iter().filter(|rule| rule.target_id.is_some()) {
        source(code);
        op(code, 0x46, lower);
        put(code, 0);
        op(
            code,
            0x2c,
            constants.string(&rule.target_id.as_ref().unwrap().to_ascii_lowercase()),
        );
        let mismatch = branch(code, 0x14);
        literal(code, field, rule, constants);
        code.push(0x48);
        let end = code.len();
        bind(code, mismatch, end)?;
    }
    let end = code.len();
    for at in [remote, negative, outside, null] {
        bind(code, at, end)?;
    }
    Ok(())
}

/// A call with no arguments and no result, run at the end of a method.
#[derive(Clone, Copy)]
pub(in crate::patcher) enum Call<'a> {
    /// `this.name()`
    This(&'a str),
    /// `Class.name()` on a static method of another class, given as `package:Class`.
    Static { class: &'a str, name: &'a str },
}

const RETURN_VOID: u8 = 0x47;
const GET_LOCAL_0: u8 = 0xd0;
const GET_LEX: u8 = 0x60;
const CALL_PROP_VOID: u8 = 0x4f;

/// Insert `calls` just before the final `returnvoid` of `class.method` (an
/// instance method; `class` is `package:Class`). Every name used is already in
/// the ABC, so no constant pool changes: a method's own trait name is a
/// multiname that calls it. Branches stay valid because they are relative and
/// the insert point is the last instruction. Returns None when the movie has
/// no such class.
pub(in crate::patcher) fn append_calls(
    data: &[u8],
    class: &str,
    method: &str,
    calls: &[Call<'_>],
) -> Result<Option<(Vec<u8>, usize)>> {
    let abc = Abc::parse(data)?;
    let Some(owner) = abc
        .classes
        .iter()
        .find(|c| abc.name(c.name).is_ok_and(|n| n.full == class))
    else {
        return Ok(None);
    };
    let trait_method = |traits: &[Trait], name: &str| {
        traits
            .iter()
            .find(|t| (1..=3).contains(&t.kind) && abc.name(t.name).is_ok_and(|n| n.local == name))
            .copied_name_and_method()
    };
    let (_, id) = trait_method(&owner.fields, method)
        .ok_or_else(|| format!("{class} has no method {method}"))?;
    let body = abc
        .bodies
        .iter()
        .find(|b| b.id == id)
        .ok_or("Method has no body")?;
    if body.exceptions != 0 {
        return Err(format!("{class}.{method} has exception handlers"));
    }
    let code = &data[body.code.clone()];
    if code.last() != Some(&RETURN_VOID) {
        return Err(format!("{class}.{method} does not end in returnvoid"));
    }
    let mut inserted = Vec::new();
    for call in calls {
        match *call {
            Call::This(name) => {
                let (multiname, _) = trait_method(&owner.fields, name)
                    .ok_or_else(|| format!("{class} has no method {name}"))?;
                inserted.push(GET_LOCAL_0);
                op(&mut inserted, CALL_PROP_VOID, multiname);
                put(&mut inserted, 0);
            }
            Call::Static {
                class: target,
                name,
            } => {
                let target = abc
                    .classes
                    .iter()
                    .find(|c| abc.name(c.name).is_ok_and(|n| n.full == target))
                    .ok_or_else(|| format!("Missing class {target}"))?;
                let (multiname, _) = trait_method(&target.statics, name)
                    .ok_or_else(|| format!("Missing static method {name}"))?;
                op(&mut inserted, GET_LEX, target.name);
                op(&mut inserted, CALL_PROP_VOID, multiname);
                put(&mut inserted, 0);
            }
        }
    }
    if code.ends_with(&[&inserted[..], &[RETURN_VOID]].concat()) {
        return Err(format!("{class}.{method} already has these calls"));
    }
    let mut new_code = code[..code.len() - 1].to_vec();
    new_code.extend(&inserted);
    new_code.push(RETURN_VOID);
    Ok(Some((replace_body_code(data, body, &new_code), 1)))
}

/// The embedded Workshop movie supplies this method. Leaving it as a
/// no-op keeps the Workshop and LAN controls available without a downloader.
pub(in crate::patcher) fn disable_workshop_downloader(data: &[u8]) -> Result<Option<(Vec<u8>, usize)>> {
    let abc = Abc::parse(data)?;
    let mut matches = abc.classes.iter().flat_map(|class| &class.fields).filter_map(|field| {
        (abc.name(field.name).ok()?.local == "hbxToggleDl").then_some(field.method).flatten()
    });
    let Some(method) = matches.next() else { return Ok(None) };
    if matches.next().is_some() {
        return Err("Ambiguous Workshop downloader method".into());
    }
    let body = abc.bodies.iter().find(|body| body.id == method && body.exceptions == 0)
        .ok_or("Missing Workshop downloader method body")?;
    Ok(Some((replace_body_code(data, body, &[RETURN_VOID]), 1)))
}

/// Return the offset of the Instant Suite method's first instruction after
/// its scope setup. Leaving the method in place preserves all ABC indexes.
pub(in crate::patcher) fn instant_suite_hook(data: &[u8]) -> Result<Option<(usize, bool)>> {
    if !data.windows(6).any(|bytes| bytes == b"hbxAsk") {
        return Ok(None);
    }
    let abc = Abc::parse(data)?;
    let Some(class) = abc.classes.iter().find(|class| {
        abc.name(class.name).is_ok_and(|name| name.local == "WinnerPrompt")
    }) else {
        return Ok(None);
    };
    let Some(method) = class.fields.iter().find_map(|field| {
        (abc.name(field.name).ok()?.local == "hbxAsk").then_some(field.method).flatten()
    }) else {
        return Ok(None);
    };
    let body = abc.bodies.iter().find(|body| body.id == method)
        .ok_or("Instant Suite method has no body")?;
    match data[body.code.clone()].get(..4) {
        Some([0xd0, 0x30, 0xd0, 0x66]) => Ok(Some((body.code.start + 2, true))),
        Some([0xd0, 0x30, RETURN_VOID, 0x66]) => Ok(Some((body.code.start + 2, false))),
        _ => Err("Instant Suite method layout changed".into()),
    }
}

pub(in crate::patcher) const INSTANT_SUITE_ACTION: &str = "GetSCT";

pub(in crate::patcher) fn has_instant_suite_bridge(data: &[u8]) -> bool {
    data.windows(b"HEBNIX_INSTANT_SUITE_BRIDGE".len()).any(|bytes| bytes == b"HEBNIX_INSTANT_SUITE_BRIDGE")
}

/// Replace the retired image request with a bounded poll of the native Shell bridge.
pub(in crate::patcher) fn instant_suite_bridge(data: &[u8], enabled: bool) -> Result<Option<(Vec<u8>, usize)>> {
    if instant_suite_hook(data)?.is_none() {
        return Ok(None);
    }
    let abc = Abc::parse(data)?;
    let class = abc.classes.iter().find(|class| {
        abc.name(class.name).is_ok_and(|name| name.full == "prompts:WinnerPrompt")
    }).ok_or("Winner prompt class is missing")?;
    let method = |name: &str| {
        let id = class.fields.iter().find_map(|field| {
            (abc.name(field.name).ok()?.local == name).then_some(field.method).flatten()
        }).ok_or_else(|| format!("Missing winner method {name}"))?;
        abc.bodies.iter().find(|body| body.id == id && body.exceptions == 0)
            .ok_or_else(|| format!("Missing winner method body {name}"))
    };
    let name = |full: &str| {
        abc.names.iter().position(|name| name.full == full)
            .ok_or_else(|| format!("Missing winner property {full}"))
    };
    let hooks = name("projectx.proxy:UIShellHooks")?;
    let native = name(":SHELL_CallUnrealFunction")?;
    let listener = name(":hbxNone")?;
    let deadline = name(":hbxL")?;
    let done = name(":hbxDone")?;
    let date = name(":Date")?;
    let time = name(":time")?;
    let add = name(":addEventListener")?;
    let remove = name(":removeEventListener")?;
    let got = name(":hbxGot")?;
    let mut strings = abc.strings.values.clone();
    let mut uints = Vec::new();
    let mut constants = Constants { strings: &mut strings, uints: &mut uints };
    let frame = constants.string("enterFrame");
    let shell_class = constants.string("GFxShell_TA");
    let shell = constants.string("Shell");
    let get_key = constants.string("GetKeyForAction");
    let action = constants.string(INSTANT_SUITE_ACTION);
    constants.string("HEBNIX_INSTANT_SUITE_BRIDGE");
    let clock = |code: &mut Vec<u8>| {
        op(code, 0x5d, date); // findpropstrict Date
        op(code, 0x4a, date); // constructprop Date()
        put(code, 0);
        op(code, 0x66, time);
    };
    let event_listener = |code: &mut Vec<u8>, operation| {
        code.push(0xd0);
        op(code, 0x2c, frame);
        code.push(0xd0);
        op(code, 0x66, listener);
        op(code, 0x4f, operation);
        put(code, 2);
    };
    let mut ask = vec![0xd0, 0x30, if enabled { 0xd0 } else { RETURN_VOID }];
    op(&mut ask, 0x66, done);
    let start = branch(&mut ask, 0x12); // iffalse
    ask.push(RETURN_VOID);
    let at = ask.len();
    bind(&mut ask, start, at)?;
    ask.extend([0xd0, 0x26]); // this, true
    op(&mut ask, 0x61, done);
    ask.push(0xd0);
    clock(&mut ask);
    op(&mut ask, 0x25, 5_000); // pushshort
    ask.push(0xa0); // add
    op(&mut ask, 0x61, deadline);
    event_listener(&mut ask, add);
    ask.extend([0xd0, 0x20]); // this, null
    op(&mut ask, 0x4f, listener);
    ask.extend([1, RETURN_VOID]);

    // The native function returns decimal epoch milliseconds followed by one
    // action digit. Expired or empty replies keep polling until the deadline.
    let mut poll = vec![0xd0, 0x30];
    clock(&mut poll);
    poll.extend([0x75, 0xd5, 0xd1, 0xd0]); // convert_d; now = local1
    op(&mut poll, 0x66, deadline);
    let expired = branch(&mut poll, 0x18); // ifge
    op(&mut poll, 0x60, hooks);
    op(&mut poll, 0x66, native);
    let unavailable = branch(&mut poll, 0x12);
    op(&mut poll, 0x60, hooks);
    op(&mut poll, 0x2c, shell_class);
    op(&mut poll, 0x2c, shell);
    poll.extend([0x24, 0]);
    op(&mut poll, 0x2c, get_key);
    op(&mut poll, 0x2c, action);
    poll.push(0x27); // false
    op(&mut poll, 0x46, native);
    poll.extend([6, 0x75, 0xd6, 0xd2, 0xd1, 0x24, 10, 0xa2]); // reply, now * 10
    let fresh = branch(&mut poll, 0x17); // ifgt
    let at = poll.len();
    bind(&mut poll, unavailable, at)?;
    poll.push(RETURN_VOID);
    let at = poll.len();
    bind(&mut poll, fresh, at)?;
    event_listener(&mut poll, remove);
    poll.extend([0xd0, 0xd2, 0x24, 10, 0xa4, 0x73]); // this, int(reply % 10)
    op(&mut poll, 0x4f, got);
    poll.extend([1, RETURN_VOID]);
    let at = poll.len();
    bind(&mut poll, expired, at)?;
    event_listener(&mut poll, remove);
    poll.push(RETURN_VOID);

    let got_body = method("hbxGot")?;
    let old = &data[got_body.code.clone()];
    // Retain the original action dispatch, replacing event.target.width
    // with the command digit passed by the poller.
    let mut dispatch = vec![0xd0, 0x30, 0xd1, 0x73, 0xd6];
    let mut reader = Reader { data: old, at: 3 };
    if reader.byte()? != 0x66 || abc.name(reader.index()?)?.local != "target"
        || reader.byte()? != 0x66 || abc.name(reader.index()?)?.local != "width"
        || reader.take(2)? != [0x73, 0xd6]
    {
        return Err("Unexpected Instant Suite action dispatch".into());
    }
    dispatch.extend_from_slice(&old[reader.at..]);

    let removed = method("handleRemovedFromStage")?;
    let old = &data[removed.code.clone()];
    if old.last() != Some(&RETURN_VOID) {
        return Err("Unexpected winner prompt removal handler".into());
    }
    let mut cleanup = old[..old.len() - 1].to_vec();
    event_listener(&mut cleanup, remove);
    cleanup.extend([0xd0, 0x27]); // this, false
    op(&mut cleanup, 0x61, done);
    cleanup.push(RETURN_VOID);
    let mut edits = Vec::new();
    for (body, code, locals) in [
        (method("hbxAsk")?, ask, 2),
        (method("hbxNone")?, poll, 3),
        (got_body, dispatch, 3),
        (removed, cleanup, removed.locals),
    ] {
        let mut replacement = Vec::new();
        for value in [body.id, body.stack.max(8), locals, body.init_scope, body.max_scope, code.len()] {
            put(&mut replacement, value);
        }
        replacement.extend(code);
        replacement.extend_from_slice(&data[body.code.end..body.span.end]);
        edits.push((body.span.clone(), replacement));
    }
    let mut pool = Vec::new();
    put(&mut pool, strings.len());
    pool.extend_from_slice(&data[abc.strings.entries..abc.strings.span.end]);
    for value in &strings[abc.strings.count..] {
        put(&mut pool, value.len());
        pool.extend(value.as_bytes());
    }
    edits.push((abc.strings.span, pool));
    edits.sort_by_key(|(range, _)| range.start);
    let mut output = data.to_vec();
    for (range, bytes) in edits.into_iter().rev() {
        output.splice(range, bytes);
    }
    if instant_suite_hook(&output)?.map(|(_, on)| on) != Some(enabled) {
        return Err("Instant Suite bridge validation failed".into());
    }
    Ok(Some((output, 1)))
}

fn replace_body_code(data: &[u8], body: &Body, new_code: &[u8]) -> Vec<u8> {
    let mut rebuilt = Vec::new();
    put(&mut rebuilt, body.id);
    // Each call leaves at most a class and a receiver on the operand stack.
    put(&mut rebuilt, body.stack.max(2));
    put(&mut rebuilt, body.locals);
    put(&mut rebuilt, body.init_scope);
    put(&mut rebuilt, body.max_scope);
    put(&mut rebuilt, new_code.len());
    rebuilt.extend(new_code);
    rebuilt.extend(&data[body.code.end..body.span.end]);
    let mut out = data[..body.span.start].to_vec();
    out.extend(rebuilt);
    out.extend(&data[body.span.end..]);
    out
}

/// Extend the team's existing colour transform to the visible boost fill.
/// The fingerprint guards the two branch offsets we relocate and the
/// multiname operands reused from the inspected HUD method.
pub(in crate::patcher) fn patch_boost_ring(
    data: &[u8],
    enable: bool,
) -> Result<Option<(Vec<u8>, usize)>> {
    use sha2::{Digest, Sha256};

    let abc = Abc::parse(data)?;
    let Some(owner) = abc.classes.iter().find(|class| {
        abc.name(class.name)
            .is_ok_and(|name| name.full == "tagame.hud:BoostMeterView")
    }) else {
        return Ok(None);
    };
    let field = |name: &str| {
        owner
            .fields
            .iter()
            .find(|field| abc.name(field.name).is_ok_and(|entry| entry.local == name))
            .ok_or_else(|| format!("Boost HUD is missing {name}"))
    };
    let method = field("onTeamColorChanged")?
        .method
        .ok_or("Missing boost colour handler")?;
    let body = abc
        .bodies
        .iter()
        .find(|body| body.id == method)
        .ok_or("Missing boost colour handler body")?;
    let code = &data[body.code.clone()];
    const ORIGINAL: &str = "d646e74d8c2eb0ea3ffb40ddd7db79a99d7d133df2b65fe2ce7e7057df98c93f";
    const TEAM_DISABLED_BRANCH: usize = 21;
    const END_OF_TEAM_BLOCK: usize = 231;
    const START_OF_NORMAL_BLOCK: usize = 235;
    const FINAL_RETURN: usize = 378;

    let mut receiver = vec![GET_LOCAL_0];
    op(&mut receiver, 0x66, field("fillProgressBar")?.name);
    // Copy the existing transform getter and setter on backgroundMovieClip.
    const TRANSFORM: &[u8] = &[0x66, 0x96, 0x53];
    const SET_COLOR_TRANSFORM: &[u8] = &[0x61, 0x99, 0x03];
    receiver.extend(TRANSFORM);
    let mut team = receiver.clone();
    // this.m_teamStyle.colorTransform
    team.extend([GET_LOCAL_0, 0x66, 0xac, 0x1d, 0x66, 0x99, 0x03]);
    team.extend(SET_COLOR_TRANSFORM);
    let mut normal = receiver;
    // new ColorTransform() restores the original texture colours.
    normal.extend([0x5d, 0x08, 0x4a, 0x08, 0x00]);
    normal.extend(SET_COLOR_TRANSFORM);
    if body.exceptions != 0 {
        return Err("This Rocket League build has an unsupported boost HUD colour handler".into());
    }
    let patched = code.len() == FINAL_RETURN + 1 + team.len() + normal.len()
        && code[END_OF_TEAM_BLOCK..].starts_with(&team)
        && code.ends_with(&[normal.as_slice(), &[RETURN_VOID]].concat());
    if patched {
        let mut original = code[..END_OF_TEAM_BLOCK].to_vec();
        let jump = branch(&mut original, 0x10);
        original.extend(&code[START_OF_NORMAL_BLOCK + team.len()..FINAL_RETURN + team.len()]);
        original.push(RETURN_VOID);
        bind(&mut original, TEAM_DISABLED_BRANCH, START_OF_NORMAL_BLOCK)?;
        bind(&mut original, jump, FINAL_RETURN)?;
        if hex::encode(Sha256::digest(&original)) != ORIGINAL {
            return Err("The team-coloured boost ring was modified by another patch".into());
        }
        return Ok((!enable).then(|| (replace_body_code(data, body, &original), 1)));
    }
    if !enable {
        return Ok(None);
    }
    if hex::encode(Sha256::digest(code)) != ORIGINAL {
        return Err("This Rocket League build has an unsupported boost HUD colour handler".into());
    }

    let mut replacement = code[..END_OF_TEAM_BLOCK].to_vec();
    replacement.extend(team);
    let jump = branch(&mut replacement, 0x10);
    let normal_start = replacement.len();
    replacement.extend(&code[START_OF_NORMAL_BLOCK..FINAL_RETURN]);
    replacement.extend(normal);
    let return_at = replacement.len();
    replacement.push(RETURN_VOID);
    bind(&mut replacement, TEAM_DISABLED_BRANCH, normal_start)?;
    bind(&mut replacement, jump, return_at)?;
    Ok(Some((replace_body_code(data, body, &replacement), 1)))
}

trait NameAndMethod {
    fn copied_name_and_method(self) -> Option<(usize, usize)>;
}
impl NameAndMethod for Option<&Trait> {
    fn copied_name_and_method(self) -> Option<(usize, usize)> {
        self.and_then(|t| t.method.map(|m| (t.name, m)))
    }
}

/// Returns None for ABCs without the game's title API. Existing indices remain
/// valid: only new pool entries and complete, validated method bodies are added.
pub(super) fn patch(data: &[u8], rules: &[TitleOverride]) -> Result<Option<(Vec<u8>, usize)>> {
    let abc = Abc::parse(data)?;
    let Some(api) = abc.classes.iter().find(|class| {
        abc.name(class.name)
            .is_ok_and(|name| name.full == "tagame.api:PlayerTitles")
    }) else {
        return Ok(None);
    };
    if abc.strings.values.iter().any(|value| value == MARKER) {
        return Err("Title movie already patched without a matching restore baseline".into());
    }
    let data_class = abc
        .classes
        .iter()
        .find(|class| {
            abc.name(class.name)
                .is_ok_and(|name| name.full == "tagame.api:PlayerTitleData")
        })
        .ok_or("Missing PlayerTitleData class")?;
    if !abc
        .methods
        .get(data_class.init)
        .ok_or("Invalid title constructor")?
        .params
        .is_empty()
    {
        return Err("Unsupported title constructor".into());
    }
    let mut fields = HashMap::new();
    for field in &data_class.fields {
        if field.kind == 0 {
            fields.insert(abc.name(field.name)?.local.as_str(), field.name);
        }
    }
    for field in ["Id", "Text", "Color", "GlowColor"] {
        if !fields.contains_key(field) {
            return Err(format!("Missing title field {field}"));
        }
    }
    let lower = abc
        .names
        .iter()
        .position(|name| name.local == "toLowerCase")
        .ok_or("Missing String.toLowerCase name")?;
    let central = api
        .statics
        .iter()
        .find(|entry| {
            abc.name(entry.name)
                .is_ok_and(|name| name.local == "GetTitleData")
        })
        .and_then(|entry| entry.method)
        .ok_or("Missing static title getter")?;
    let method = abc.methods.get(central).ok_or("Invalid title method")?;
    if method.params.len() != 1
        || abc.name(method.params[0])?.full != ":String"
        || abc.name(method.returns)?.full != "tagame.api:PlayerTitleData"
    {
        return Err("Unsupported title getter signature".into());
    }
    let mut strings = abc.strings.values.clone();
    let mut uints = abc.uints.values.clone();
    let mut constants = Constants {
        strings: &mut strings,
        uints: &mut uints,
    };
    constants.string(MARKER);
    let mut jobs = vec![(central, None)];
    for class in &abc.classes {
        let class_name = &abc.name(class.name)?.full;
        let source = match class_name.as_str() {
            "tagame.api:PlayerTitlesPlayerTitlesRow" => "Id",
            "tagame.api:PartyMemberProfileRow" => "Title",
            "tagame.api:PlayerInfoRow" => "PrimaryTitle",
            _ => continue,
        };
        let Some(source_name) = class
            .fields
            .iter()
            .find(|field| {
                field.kind == 2 && abc.name(field.name).is_ok_and(|name| name.local == source)
            })
            .map(|field| field.name)
        else {
            continue;
        };
        let source_method = class
            .fields
            .iter()
            .find(|field| field.name == source_name)
            .and_then(|field| field.method)
            .and_then(|index| abc.methods.get(index))
            .ok_or("Missing title ID getter signature")?;
        if !source_method.params.is_empty() || abc.name(source_method.returns)?.full != ":String" {
            return Err("Unsupported title ID getter type".into());
        }
        for field in &class.fields {
            let name = &abc.name(field.name)?.local;
            let wanted = match class_name.as_str() {
                "tagame.api:PlayerTitlesPlayerTitlesRow" => {
                    matches!(name.as_str(), "Text" | "Color" | "GlowColor")
                }
                "tagame.api:PartyMemberProfileRow" => {
                    matches!(name.as_str(), "TitleText" | "TitleColor" | "TitleGlowColor")
                }
                // Match banners and the scoreboard use the already resolved
                // XPTitle string rather than calling PlayerTitles.GetTitleData.
                // Keep PrimaryTitle as the original ID used to select a rule.
                _ => matches!(name.as_str(), "XPTitle" | "TitleColor" | "TitleGlowColor"),
            };
            if wanted && field.kind == 2 {
                jobs.push((
                    field.method.ok_or("Missing row getter")?,
                    Some((TitleIdSource::Property(source_name), name.clone())),
                ));
            }
        }
    }
    // Some HUD views use PlayerInfo.GetXPTitle(rowIndex) directly rather
    // than constructing a PlayerInfoRow. Cover that public API as well.
    if let Some(class) = abc.classes.iter().find(|class| {
        abc.name(class.name)
            .is_ok_and(|name| name.full == "tagame.api:PlayerInfo")
    }) {
        let source = class
            .statics
            .iter()
            .find(|entry| {
                abc.name(entry.name)
                    .is_ok_and(|name| name.local == "GetPrimaryTitle")
            })
            .ok_or("Missing indexed title ID getter")?;
        let method = source
            .method
            .and_then(|id| abc.methods.get(id))
            .ok_or("Missing indexed title ID signature")?;
        if method.params.len() != 1
            || abc.name(method.params[0])?.full != ":int"
            || abc.name(method.returns)?.full != ":String"
        {
            return Err("Unsupported indexed title ID getter".into());
        }
        for entry in &class.statics {
            let name = &abc.name(entry.name)?.local;
            if matches!(
                name.as_str(),
                "GetXPTitle" | "GetTitleColor" | "GetTitleGlowColor"
            ) {
                jobs.push((
                    entry.method.ok_or("Missing indexed title getter")?,
                    Some((
                        TitleIdSource::Indexed {
                            class: class.name,
                            getter: source.name,
                        },
                        name[3..].to_string(),
                    )),
                ));
            }
        }
    }
    let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::new();
    for (id, row) in &jobs {
        let body = abc
            .bodies
            .iter()
            .find(|body| body.id == *id)
            .ok_or("Title getter has no body")?;
        let original = &data[body.code.clone()];
        let added_scope = row.as_ref().is_some_and(|(source, field)| {
            matches!(source, TitleIdSource::Property(name) if abc.name(*name).is_ok_and(|name| name.local == "PrimaryTitle"))
                && field == "XPTitle"
                && modified_xp_title(original)
        });
        let prologue = if added_scope { Ok(0) } else { prologue_len(original) }.map_err(|error| {
            let field = row.as_ref().map_or("GetTitleData", |(_, field)| field);
            format!(
                "{error} in {field} (method {id}, starts with {:02x?})",
                &original[..original.len().min(8)]
            )
        })?;
        if body.exceptions != 0 || (!added_scope && body.max_scope <= body.init_scope) {
            return Err(format!(
                "Unsupported title method framing: method {id}, exceptions {}, scope {}->{}, prefix {:02x?}",
                body.exceptions,
                body.init_scope,
                body.max_scope,
                &original[..original.len().min(8)]
            ));
        }
        if let Some((source, field)) = row {
            let method = abc
                .methods
                .get(*id)
                .ok_or("Missing title getter signature")?;
            let valid_params = match source {
                TitleIdSource::Property(_) => method.params.is_empty(),
                TitleIdSource::Indexed { .. } => {
                    method.params.len() == 1 && abc.name(method.params[0])?.full == ":int"
                }
            };
            let expected = if matches!(field.as_str(), "Text" | "TitleText" | "XPTitle") {
                ":String"
            } else {
                ":uint"
            };
            if !valid_params
                || abc.name(method.returns)?.full != expected
                || original.last() != Some(&0x48)
            {
                return Err("Unsupported title display getter".into());
            }
        }
        let mut code = original[..prologue].to_vec();
        if added_scope {
            code.extend([0xd0, 0x30]); // getlocal_0, pushscope
        }
        let source = |code: &mut Vec<u8>| {
            if let Some((source, _)) = row {
                match source {
                    TitleIdSource::Property(name) => {
                        code.push(0xd0);
                        op(code, 0x66, *name);
                    }
                    TitleIdSource::Indexed { class, getter } => {
                        op(code, 0x60, *class);
                        code.push(0xd1);
                        op(code, 0x46, *getter);
                        put(code, 1);
                    }
                }
            } else {
                code.push(0xd1);
            }
        };
        source(&mut code);
        code.push(0x20);
        let null = branch(&mut code, 0x13);
        source(&mut code);
        op(&mut code, 0x2c, constants.string(""));
        let empty = branch(&mut code, 0x13);
        // Specific overrides win over the optional legacy wildcard.
        for rule in rules
            .iter()
            .filter(|rule| rule.target_id.is_some())
            .chain(rules.iter().filter(|rule| rule.target_id.is_none()))
        {
            let mismatch = if let Some(target) = &rule.target_id {
                source(&mut code);
                op(&mut code, 0x46, lower);
                put(&mut code, 0);
                op(
                    &mut code,
                    0x2c,
                    constants.string(&target.to_ascii_lowercase()),
                );
                Some(branch(&mut code, 0x14))
            } else {
                None
            };
            if let Some((_, field)) = row {
                literal(&mut code, field, rule, &mut constants);
            } else {
                op(&mut code, 0x5d, data_class.name);
                op(&mut code, 0x4a, data_class.name);
                put(&mut code, 0);
                code.push(0x2a);
                source(&mut code);
                op(&mut code, 0x61, fields["Id"]);
                for field in ["Text", "Color", "GlowColor"] {
                    code.push(0x2a);
                    literal(&mut code, field, rule, &mut constants);
                    op(&mut code, 0x61, fields[field]);
                }
            }
            code.push(0x48);
            if let Some(at) = mismatch {
                let end = code.len();
                bind(&mut code, at, end)?;
            }
        }
        let fallback = code.len();
        bind(&mut code, null, fallback)?;
        bind(&mut code, empty, fallback)?;
        // Match data may leave PrimaryTitle empty (or use it for a different
        // title system) while XPTitle contains the equipped title's resolved text.
        // Read the original native values directly: calling patched XPTitle here
        // would recurse, and calling patched colours would contaminate matching.
        if let Some((source, field)) = row {
            if let Some(class) = abc.classes.iter().find(|class| {
                abc.name(class.name).is_ok_and(|name| match source {
                    TitleIdSource::Property(name_id) => {
                        name.full == "tagame.api:PlayerInfoRow"
                            && abc
                                .name(*name_id)
                                .is_ok_and(|name| name.local == "PrimaryTitle")
                    }
                    TitleIdSource::Indexed { class: name_id, .. } => {
                        class.name == *name_id && name.full == "tagame.api:PlayerInfo"
                    }
                })
            }) {
                let local_player = native_player_value(&abc, data, class, "bLocalPlayer", source)?;
                local_selected_title(
                    &abc,
                    api,
                    &mut code,
                    local_player,
                    field,
                    rules,
                    lower,
                    &mut constants,
                )?;
                // A modified getter can have branches and cannot be inlined as
                // a native expression. ID and local-player matching still apply.
                if !matches!(source, TitleIdSource::Property(_))
                    || !has_modified_row_xp_title(&abc, data, class)
                {
                    let text = native_player_value(&abc, data, class, "XPTitle", source)?;
                    let color = native_player_value(&abc, data, class, "TitleColor", source)?;
                    let glow = native_player_value(&abc, data, class, "TitleGlowColor", source)?;
                    for rule in rules.iter().filter(|rule| rule.target_id.is_some()) {
                        let Some(appearance) = &rule.original else {
                            continue;
                        };
                        if appearance.text.is_empty() {
                            continue;
                        }
                        code.extend_from_slice(text);
                        code.push(0x20);
                        let null = branch(&mut code, 0x13);
                        code.extend_from_slice(text);
                        op(&mut code, 0x46, lower);
                        put(&mut code, 0);
                        op(
                            &mut code,
                            0x2c,
                            constants.string(&appearance.text.to_ascii_lowercase()),
                        );
                        let mismatch = branch(&mut code, 0x14);
                        let color_mismatch = appearance.color.map(|value| {
                            code.extend_from_slice(color);
                            op(&mut code, 0x2e, constants.uint(0xffffff));
                            code.push(0xa8); // bitand: ignore native ARGB alpha
                            op(&mut code, 0x2e, constants.uint(value));
                            branch(&mut code, 0x14)
                        });
                        code.extend_from_slice(glow);
                        op(&mut code, 0x2e, constants.uint(0xffffff));
                        code.push(0xa8);
                        op(&mut code, 0x2e, constants.uint(appearance.glow_color));
                        let glow_mismatch = branch(&mut code, 0x14);
                        literal(&mut code, field, rule, &mut constants);
                        code.push(0x48);
                        let end = code.len();
                        for at in [
                            Some(null),
                            Some(mismatch),
                            color_mismatch,
                            Some(glow_mismatch),
                        ]
                        .into_iter()
                        .flatten()
                        {
                            bind(&mut code, at, end)?;
                        }
                    }
                }
            }
        }
        if added_scope {
            // Restore the original scope depth before entering the intact body.
            code.push(0x1d); // popscope
        }
        code.extend_from_slice(&original[prologue..]);
        let mut replacement = Vec::new();
        for n in [
            body.id,
            body.stack.max(6),
            body.locals,
            body.init_scope,
            body.max_scope.max(body.init_scope + usize::from(added_scope)),
            code.len(),
        ] {
            put(&mut replacement, n);
        }
        replacement.extend(code);
        replacement.extend_from_slice(&data[body.code.end..body.span.end]);
        edits.push((body.span.clone(), replacement));
    }
    let mut pool = Vec::new();
    put(&mut pool, uints.len());
    pool.extend_from_slice(&data[abc.uints.entries..abc.uints.span.end]);
    for value in &uints[abc.uints.count.max(1)..] {
        put(&mut pool, *value as usize);
    }
    edits.push((abc.uints.span.clone(), pool));
    let mut pool = Vec::new();
    put(&mut pool, strings.len());
    pool.extend_from_slice(&data[abc.strings.entries..abc.strings.span.end]);
    for value in &strings[abc.strings.count.max(1)..] {
        put(&mut pool, value.len());
        pool.extend(value.as_bytes());
    }
    edits.push((abc.strings.span.clone(), pool));
    edits.sort_by_key(|(range, _)| range.start);
    if edits.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
        return Err("Overlapping ABC edits".into());
    }
    let mut output = data.to_vec();
    for (range, bytes) in edits.into_iter().rev() {
        output.splice(range, bytes);
    }
    let check = Abc::parse(&output)?;
    if check.methods.len() != abc.methods.len()
        || check.classes.len() != abc.classes.len()
        || check.bodies.len() != abc.bodies.len()
    {
        return Err("Title ABC verification failed".into());
    }
    Ok(Some((output, jobs.len())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn title_restore_preserves_appended_hud_constants() {
        fn movie(strings: &[&str]) -> Vec<u8> {
            let mut abc = vec![16, 0, 46, 0, 0, 0, 0];
            put(&mut abc, strings.len() + 1);
            for value in strings {
                put(&mut abc, value.len());
                abc.extend(value.as_bytes());
            }
            abc.extend([0; 8]);
            abc
        }

        let before = movie(&["original"]);
        let after = movie(&["original", MARKER, "title"]);
        let live = movie(&["original", MARKER, "title", "HUD edit"]);
        let restored = revert(&live, &before, &after).unwrap().unwrap();
        let abc = Abc::parse(&restored).unwrap();
        assert_eq!(&abc.strings.values[3..], &["title", "HUD edit"]);
        assert!(!abc.strings.values.iter().any(|value| value == MARKER));
        let conflict = movie(&["changed", MARKER, "title", "HUD edit"]);
        assert!(revert(&conflict, &before, &after).is_err());
    }

    #[test]
    fn recognizes_modified_xp_title_branch_prefix() {
        let code = [0xd0, 0x66, 0xec, 1, 0x2e, 1, 0x15, 0x78, 0, 0];
        assert!(modified_xp_title(&code));
        assert!(!modified_xp_title(&code[..8]));
        assert!(!modified_xp_title(&[0xd0, 0x30, 0x48]));
    }

    #[test]
    fn title_prologue_accepts_equivalent_scope_setup() {
        assert_eq!(prologue_len(&[0x62, 0, 0x30, 0x60]).unwrap(), 3);
        assert_eq!(prologue_len(&[0xd0, 0xf0, 1, 0x30, 0x60]).unwrap(), 4);
        assert_eq!(prologue_len(&[0x62, 0, 0xf1, 1, 0x30, 0x60]).unwrap(), 5);
        assert!(prologue_len(&[0x62, 1, 0x30, 0x60]).is_err());
        assert!(prologue_len(&[0xd0, 0x1c, 0x60]).is_err()); // pushwith
        assert!(prologue_len(&[0x62, 0x80]).is_err());
    }

    #[test]
    fn native_getter_debug_metadata_after_prologue() {
        // debugfile, debugline, getlocal0/pushscope, debug register, debugline, getlex.
        let code = [
            0xf1, 1, 0xf0, 2, 0xd0, 0x30, 0xef, 1, 3, 0, 4, 0xf0, 5, 0x60,
        ];
        let start = prologue_len(&code).unwrap();
        assert_eq!(start, 6);
        let mut reader = Reader {
            data: &code,
            at: start,
        };
        skip_debug(&mut reader).unwrap();
        assert_eq!(reader.at, 13);
        assert_eq!(reader.byte().unwrap(), 0x60);
        // Unknown executable instructions are never skipped as metadata.
        let mut branch = Reader {
            data: &[0x10, 0, 0, 0],
            at: 0,
        };
        skip_debug(&mut branch).unwrap();
        assert_eq!(branch.at, 0);
        let mut truncated = Reader {
            data: &[0xef, 1, 3],
            at: 0,
        };
        assert!(skip_debug(&mut truncated).is_err());
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Value {
        Null,
        Text(String),
        Number(u32),
        Boolean(bool),
        Class(usize),
        Object(Rc<RefCell<HashMap<String, Value>>>),
    }
    // Execute the emitted AVM2 prefix. Reaching the untouched native-call
    // suffix means the original implementation handles the requested title.
    fn evaluate_player_value_at(
        abc: &Abc,
        code: &[u8],
        fallback: usize,
        id: Option<&str>,
        row_index: Option<u32>,
        appearance: Option<&super::super::TitleAppearance>,
        local_selection: Option<&(i32, Vec<Option<String>>)>,
    ) -> Option<Value> {
        let mut r = Reader { data: code, at: 0 };
        let mut stack = Vec::new();
        for _ in 0..10000 {
            if r.at == fallback {
                return None;
            }
            let instruction = r.byte().unwrap();
            match instruction {
                0xd0 => stack.push(Value::Class(0)),
                0xd1 => {
                    stack.push(row_index.map(Value::Number).unwrap_or_else(|| {
                        id.map(|s| Value::Text(s.into())).unwrap_or(Value::Null)
                    }))
                }
                0x30 => {
                    stack.pop().unwrap();
                }
                0xf0 | 0xf1 => {
                    r.index().unwrap();
                }
                0xef => {
                    r.byte().unwrap();
                    r.index().unwrap();
                    r.byte().unwrap();
                    r.index().unwrap();
                }
                0x20 => stack.push(Value::Null),
                0x2c => stack.push(Value::Text(abc.strings.values[r.index().unwrap()].clone())),
                0x2e => stack.push(Value::Number(abc.uints.values[r.index().unwrap()])),
                0x24 => stack.push(Value::Number(r.byte().unwrap() as i8 as i32 as u32)),
                0xa8 => {
                    let Value::Number(b) = stack.pop().unwrap() else {
                        panic!("Expected mask")
                    };
                    let Value::Number(a) = stack.pop().unwrap() else {
                        panic!("Expected colour")
                    };
                    stack.push(Value::Number(a & b));
                }
                0x12 | 0x13 | 0x14 | 0x15 | 0x18 => {
                    let bytes = r.take(3).unwrap();
                    let mut delta = [0u8; 4];
                    delta[..3].copy_from_slice(bytes);
                    if bytes[2] & 128 != 0 {
                        delta[3] = 255;
                    }
                    let b = stack.pop().unwrap();
                    let take = if instruction == 0x12 {
                        b == Value::Boolean(false)
                    } else {
                        let a = stack.pop().unwrap();
                        match instruction {
                            0x13 => a == b,
                            0x14 => a != b,
                            _ => {
                                let (Value::Number(a), Value::Number(b)) = (a, b) else {
                                    panic!("Expected indices")
                                };
                                if instruction == 0x15 {
                                    (a as i32) < (b as i32)
                                } else {
                                    (a as i32) >= (b as i32)
                                }
                            }
                        }
                    };
                    if take {
                        r.at = (r.at as i64 + i32::from_le_bytes(delta) as i64) as usize;
                    }
                }
                0x46 => {
                    let name = r.index().unwrap();
                    let count = r.index().unwrap();
                    if abc.names[name].local == "toLowerCase" {
                        assert_eq!(count, 0);
                        let Value::Text(text) = stack.pop().unwrap() else {
                            panic!("Invalid lower-case receiver")
                        };
                        stack.push(Value::Text(text.to_lowercase()));
                    } else if abc.names[name].local == "getValue" {
                        assert_eq!(count, 3);
                        let Value::Text(field) = stack.pop().unwrap() else {
                            panic!("Missing native field")
                        };
                        assert_eq!(stack.pop().unwrap(), Value::Number(row_index.unwrap_or(0)));
                        assert_eq!(stack.pop().unwrap(), Value::Text("PlayerInfo".into()));
                        stack.pop().unwrap(); // DataAPI class
                        if field == "bLocalPlayer" {
                            stack.push(Value::Boolean(local_selection.is_some()));
                            continue;
                        }
                        let appearance = appearance.expect("Native match data required");
                        stack.push(match field.as_str() {
                            "XPTitle" => Value::Text(appearance.text.clone()),
                            "TitleColor" => Value::Number(appearance.color.unwrap_or(0xffffff)),
                            "TitleGlowColor" => Value::Number(appearance.glow_color),
                            _ => panic!("Unexpected native field"),
                        });
                    } else if abc.names[name].local == "getRowCount" {
                        assert_eq!(count, 0);
                        stack.pop().unwrap();
                        stack.push(Value::Number(local_selection.unwrap().1.len() as u32));
                    } else if abc.names[name].local == "GetId" {
                        assert_eq!(count, 1);
                        let Value::Number(index) = stack.pop().unwrap() else {
                            panic!("Expected selected index")
                        };
                        stack.pop().unwrap();
                        stack.push(
                            local_selection.unwrap().1[index as usize]
                                .as_ref()
                                .map(|id| Value::Text(id.clone()))
                                .unwrap_or(Value::Null),
                        );
                    } else {
                        assert_eq!(abc.names[name].local, "GetPrimaryTitle");
                        assert_eq!(count, 1);
                        assert_eq!(stack.pop().unwrap(), Value::Number(row_index.unwrap()));
                        let Value::Class(class) = stack.pop().unwrap() else {
                            panic!("Invalid PlayerInfo receiver")
                        };
                        assert_eq!(abc.names[class].full, "tagame.api:PlayerInfo");
                        stack.push(id.map(|s| Value::Text(s.into())).unwrap_or(Value::Null));
                    }
                }
                0x66 => {
                    let name = r.index().unwrap();
                    let receiver = stack.pop().unwrap();
                    if abc.names[name].local == "SelectedTitle" {
                        let Value::Class(class) = receiver else {
                            panic!("Expected title API")
                        };
                        assert_eq!(abc.names[class].full, "tagame.api:PlayerTitles");
                        stack.push(Value::Number(local_selection.unwrap().0 as u32));
                        continue;
                    }
                    assert_eq!(receiver, Value::Class(0));
                    if abc.names[name].local == "RowIndex" {
                        stack.push(Value::Number(row_index.unwrap_or(0)));
                    } else {
                        assert_eq!(abc.names[name].local, "PrimaryTitle");
                        stack.push(id.map(|s| Value::Text(s.into())).unwrap_or(Value::Null));
                    }
                }
                0x5d | 0x60 => stack.push(Value::Class(r.index().unwrap())),
                0x4a => {
                    let name = r.index().unwrap();
                    assert_eq!(r.index().unwrap(), 0);
                    assert_eq!(stack.pop().unwrap(), Value::Class(name));
                    stack.push(Value::Object(Rc::new(RefCell::new(HashMap::new()))));
                }
                0x2a => stack.push(stack.last().unwrap().clone()),
                0x61 => {
                    let name = r.index().unwrap();
                    let value = stack.pop().unwrap();
                    let Value::Object(object) = stack.pop().unwrap() else {
                        panic!("Invalid field receiver")
                    };
                    object
                        .borrow_mut()
                        .insert(abc.names[name].local.clone(), value);
                }
                0x48 => {
                    let value = stack.pop().unwrap();
                    assert!(stack.is_empty());
                    return Some(value);
                }
                _ => panic!("Unsupported emitted opcode {instruction:x}"),
            }
        }
        panic!("Title dispatcher did not terminate")
    }
    fn evaluate_match_value_at(
        abc: &Abc,
        code: &[u8],
        fallback: usize,
        id: Option<&str>,
        row_index: Option<u32>,
        appearance: Option<&super::super::TitleAppearance>,
    ) -> Option<Value> {
        evaluate_player_value_at(abc, code, fallback, id, row_index, appearance, None)
    }

    fn evaluate_value_at(
        abc: &Abc,
        code: &[u8],
        fallback: usize,
        id: Option<&str>,
        row_index: Option<u32>,
    ) -> Option<Value> {
        evaluate_match_value_at(abc, code, fallback, id, row_index, None)
    }

    #[test]
    #[ignore = "requires an installed game's extracted title movie"]
    fn resolved_match_title_without_primary_id() {
        let original = movie_abc();
        let before = Abc::parse(&original).unwrap();
        let appearance = super::super::TitleAppearance {
            text: "S20 {Silver} Tournament Winner".into(),
            color: Some(0xffffff),
            glow_color: 0,
        };
        let rule = TitleOverride {
            target_id: Some("tournament_original_id".into()),
            original: Some(appearance.clone()),
            text: "Custom match title".into(),
            color: 0x123456,
            glow: true,
            glow_color: Some(0xabcdef),
        };
        let (patched, _) = patch(&original, &[rule.clone()]).unwrap().unwrap();
        let after = Abc::parse(&patched).unwrap();
        for indexed in [false, true] {
            let class_name = if indexed {
                "tagame.api:PlayerInfo"
            } else {
                "tagame.api:PlayerInfoRow"
            };
            let class = before
                .classes
                .iter()
                .find(|class| before.name(class.name).unwrap().full == class_name)
                .unwrap();
            let entries = if indexed {
                &class.statics
            } else {
                &class.fields
            };
            for (field, expected) in [
                ("XPTitle", Value::Text(rule.text.clone())),
                ("TitleColor", Value::Number(rule.color)),
                ("TitleGlowColor", Value::Number(rule.glow_color.unwrap())),
            ] {
                let name = if indexed {
                    format!("Get{field}")
                } else {
                    field.into()
                };
                let id = entries
                    .iter()
                    .find(|entry| before.name(entry.name).unwrap().local == name)
                    .unwrap()
                    .method
                    .unwrap();
                let old = before.bodies.iter().find(|body| body.id == id).unwrap();
                let new = after.bodies.iter().find(|body| body.id == id).unwrap();
                let suffix = old.code.len() - prologue_len(&original[old.code.clone()]).unwrap();
                let code = &patched[new.code.clone()];
                let fallback = code.len() - suffix;
                // Local matches use the equipped catalog ID even when match
                // text is localized and colours differ from the catalog.
                let native = super::super::TitleAppearance {
                    text: "Localized runtime title".into(),
                    color: Some(0xff000000),
                    glow_color: 0,
                };
                let equipped = (0, vec![rule.target_id.clone()]);
                assert_eq!(
                    evaluate_player_value_at(
                        &after,
                        code,
                        fallback,
                        None,
                        indexed.then_some(7),
                        Some(&native),
                        Some(&equipped)
                    ),
                    Some(expected.clone())
                );
                for selection in [
                    (-1, vec![rule.target_id.clone()]),
                    (1, vec![rule.target_id.clone()]),
                    (0, vec![]),
                    (0, vec![None]),
                    (0, vec![Some("unmatched_equipped_title".into())]),
                ] {
                    assert_eq!(
                        evaluate_player_value_at(
                            &after,
                            code,
                            fallback,
                            None,
                            indexed.then_some(7),
                            Some(&native),
                            Some(&selection)
                        ),
                        None
                    );
                }
                // Non-local rows must never inherit the local player's title.
                assert_eq!(
                    evaluate_player_value_at(
                        &after,
                        code,
                        fallback,
                        None,
                        indexed.then_some(7),
                        Some(&native),
                        None
                    ),
                    None
                );
                let argb = super::super::TitleAppearance {
                    color: appearance.color.map(|color| color | 0xff000000),
                    glow_color: appearance.glow_color | 0xff000000,
                    ..appearance.clone()
                };
                assert_eq!(
                    evaluate_match_value_at(
                        &after,
                        code,
                        fallback,
                        None,
                        indexed.then_some(7),
                        Some(&argb)
                    ),
                    Some(expected.clone())
                );
                for primary in [None, Some(""), Some("unrelated_primary_title")] {
                    assert_eq!(
                        evaluate_match_value_at(
                            &after,
                            code,
                            fallback,
                            primary,
                            indexed.then_some(7),
                            Some(&appearance)
                        ),
                        Some(expected.clone())
                    );
                }
                for native in [
                    super::super::TitleAppearance {
                        text: "Another title".into(),
                        ..appearance.clone()
                    },
                    super::super::TitleAppearance {
                        color: Some(0x999999),
                        ..appearance.clone()
                    },
                    super::super::TitleAppearance {
                        glow_color: 0x999999,
                        ..appearance.clone()
                    },
                ] {
                    assert_eq!(
                        evaluate_match_value_at(
                            &after,
                            code,
                            fallback,
                            None,
                            indexed.then_some(7),
                            Some(&native)
                        ),
                        None
                    );
                }
            }
        }
    }

    fn evaluate_value(abc: &Abc, code: &[u8], fallback: usize, id: Option<&str>) -> Option<Value> {
        evaluate_value_at(abc, code, fallback, id, None)
    }
    fn evaluate(
        abc: &Abc,
        code: &[u8],
        fallback: usize,
        id: Option<&str>,
    ) -> Option<HashMap<String, Value>> {
        evaluate_value(abc, code, fallback, id).map(|value| {
            let Value::Object(object) = value else {
                panic!("Invalid return object")
            };
            object.borrow().clone()
        })
    }
    fn movie_abc() -> Vec<u8> {
        let raw = std::fs::read(
            std::env::var("HEBNIX_TITLE_MOVIE")
                .expect("Set HEBNIX_TITLE_MOVIE to an extracted uncompressed GFX file"),
        )
        .unwrap();
        let bits = raw[8] >> 3;
        let mut at = 8 + (5 + 4 * bits as usize).div_ceil(8) + 4;
        while at < raw.len() {
            let tag = u16::from_le_bytes(raw[at..at + 2].try_into().unwrap());
            at += 2;
            let len = if tag & 63 == 63 {
                let len = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
                at += 4;
                len
            } else {
                (tag & 63) as usize
            };
            if tag >> 6 == 82 {
                let start =
                    at + 4 + raw[at + 4..at + len].iter().position(|b| *b == 0).unwrap() + 1;
                return raw[start..at + len].to_vec();
            }
            at += len;
        }
        panic!("Movie has no ABC")
    }
    #[test]
    #[ignore = "requires an installed game's extracted title movie"]
    fn real_title_dispatch_and_original_fallback() {
        let original = movie_abc();
        let before = Abc::parse(&original).unwrap();
        let specific = TitleOverride {
            target_id: Some("XP_Veteran".into()),
            original: None,
            text: "Custom title ✓".into(),
            color: 0x12abef,
            glow: true,
            glow_color: Some(0xee459f),
        };
        for wildcard in [false, true] {
            let mut rules = vec![specific.clone()];
            if wildcard {
                rules.insert(
                    0,
                    TitleOverride {
                        target_id: None,
                        original: None,
                        text: "Fallback".into(),
                        color: 0xaabbcc,
                        glow: false,
                        glow_color: Some(0x00ff00),
                    },
                );
            }
            let (patched, changed) = patch(&original, &rules).unwrap().unwrap();
            assert!(changed >= 4);
            let after = Abc::parse(&patched).unwrap();
            assert_eq!(
                &after.strings.values[..before.strings.values.len()],
                before.strings.values
            );
            assert_eq!(
                &after.uints.values[..before.uints.values.len()],
                before.uints.values
            );
            let api = before
                .classes
                .iter()
                .find(|c| before.name(c.name).unwrap().full == "tagame.api:PlayerTitles")
                .unwrap();
            let id = api
                .statics
                .iter()
                .find(|t| before.name(t.name).unwrap().local == "GetTitleData")
                .unwrap()
                .method
                .unwrap();
            let old = before.bodies.iter().find(|b| b.id == id).unwrap();
            let new = after.bodies.iter().find(|b| b.id == id).unwrap();
            let code = &patched[new.code.clone()];
            let prologue = prologue_len(&original[old.code.clone()]).unwrap();
            let suffix = &original[old.code.start + prologue..old.code.end];
            assert!(code.ends_with(suffix));
            let fallback = code.len() - suffix.len();
            let title = evaluate(&after, code, fallback, Some("XP_VETERAN")).unwrap();
            assert_eq!(title["Id"], Value::Text("XP_VETERAN".into()));
            assert_eq!(title["Text"], Value::Text(specific.text.clone()));
            assert_eq!(title["Color"], Value::Number(specific.color));
            assert_eq!(
                title["GlowColor"],
                Value::Number(specific.glow_color.unwrap())
            );
            assert!(evaluate(&after, code, fallback, None).is_none());
            assert!(evaluate(&after, code, fallback, Some("")).is_none());
            let other = evaluate(&after, code, fallback, Some("unmatched"));
            if wildcard {
                let other = other.unwrap();
                assert_eq!(other["Text"], Value::Text("Fallback".into()));
                assert_eq!(other["GlowColor"], Value::Number(0));
            } else {
                assert!(other.is_none());
            }
            // Exercise the actual property used by PlayerBannerInfoComponent
            // in matches, plus the colours it passes to applyTitleStyle.
            for (class_name, indexed) in [
                ("tagame.api:PlayerInfoRow", false),
                ("tagame.api:PlayerInfo", true),
            ] {
                let player_info = before
                    .classes
                    .iter()
                    .find(|class| before.name(class.name).unwrap().full == class_name)
                    .unwrap();
                for (field, expected) in [
                    ("XPTitle", Value::Text(specific.text.clone())),
                    ("TitleColor", Value::Number(specific.color)),
                    (
                        "TitleGlowColor",
                        Value::Number(specific.glow_color.unwrap()),
                    ),
                ] {
                    let fields = if indexed {
                        &player_info.statics
                    } else {
                        &player_info.fields
                    };
                    let name = if indexed {
                        format!("Get{field}")
                    } else {
                        field.into()
                    };
                    let method = fields
                        .iter()
                        .find(|entry| before.name(entry.name).unwrap().local == name)
                        .unwrap()
                        .method
                        .unwrap();
                    let old = before.bodies.iter().find(|body| body.id == method).unwrap();
                    let new = after.bodies.iter().find(|body| body.id == method).unwrap();
                    let code = &patched[new.code.clone()];
                    let prologue = prologue_len(&original[old.code.clone()]).unwrap();
                    let suffix = &original[old.code.start + prologue..old.code.end];
                    assert!(code.ends_with(suffix));
                    let fallback = code.len() - suffix.len();
                    let evaluate_getter =
                        |id| evaluate_value_at(&after, code, fallback, id, indexed.then_some(7));
                    assert_eq!(evaluate_getter(Some("XP_VETERAN")), Some(expected));
                    assert!(evaluate_getter(None).is_none());
                    assert!(evaluate_getter(Some("")).is_none());
                    let unmatched = evaluate_getter(Some("unmatched"));
                    if wildcard {
                        let expected = match field {
                            "XPTitle" => Value::Text("Fallback".into()),
                            "TitleColor" => Value::Number(0xaabbcc),
                            _ => Value::Number(0),
                        };
                        assert_eq!(unmatched, Some(expected));
                    } else {
                        assert!(unmatched.is_none());
                    }
                }
            }
            let unchanged = before
                .bodies
                .iter()
                .filter(|body| {
                    let new = after.bodies.iter().find(|b| b.id == body.id).unwrap();
                    original[body.code.clone()] == patched[new.code.clone()]
                })
                .count();
            assert_eq!(before.bodies.len() - unchanged, changed);
            assert!(
                patch(&patched, &rules).is_err(),
                "Never stack patches over an unknown baseline"
            );
        }
    }
}

#[path = "rank_abc.rs"]
mod rank_abc;
use super::RankPick;
pub(super) use rank_abc::patch as patch_ranks;
