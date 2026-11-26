//! Local display getters in the game’s Scaleform movies. Menu rows key off
//! the playlist ID, including scoreboard and promotion display rows.
use super::{
    Abc, RankPick, Reader, Result, TitleIdSource, Trait, bind, branch, native_player_value, op,
    prologue_len, put, skip_debug,
};

const MENU_FIELDS: &[&str] = &[
    "SkillTier",
    "PrevSkillTier",
    "SkillDivision",
    "PrevSkillDivision",
    "SkillMMR",
    "bCompletedPlacementMatches",
    "bPlayerSkillsUpdated",
];
const PLAYER_FIELDS: &[&str] = &[
    "SkillTier",
    "SkillMMR",
    "PrevSkillMMR",
    "bMMRWasReplicated",
    "bCompletedPlacementMatches",
];

fn mmr(out: &mut Vec<u8>, value: usize) {
    op(out, 0x25, value - 100);
    out.extend([0x24, 20, 0xa3]); // (MMR - 100) / 20
}

fn menu_value(out: &mut Vec<u8>, field: &str, pick: &RankPick) {
    match field {
        "SkillTier" | "PrevSkillTier" => out.extend([0x24, pick.tier]),
        "SkillDivision" | "PrevSkillDivision" => {
            let division = if (1..22).contains(&pick.tier) {
                pick.division
            } else {
                0
            };
            out.extend([0x24, division]);
        }
        "SkillMMR" => mmr(out, pick.mmr),
        _ => out.push(0x26), // true
    }
}

fn read_this(out: &mut Vec<u8>, property: usize) {
    out.push(0xd0);
    op(out, 0x66, property);
}

fn player_block(
    out: &mut Vec<u8>,
    field: &str,
    pick: &RankPick,
    local: usize,
    row: usize,
    native_tier: &[u8],
    change: &[u8],
) -> Result<Option<usize>> {
    if matches!(field, "bMMRWasReplicated" | "bCompletedPlacementMatches") {
        out.extend([0x26, 0x48]);
        return Ok(None);
    }

    read_this(out, local);
    let remote = branch(out, 0x12);
    match field {
        "SkillTier" => out.extend([0x24, pick.tier]),
        "PrevSkillMMR" => {
            op(out, 0x25, pick.mmr - 100);
            out.extend(change);
            out.extend([0xa1, 0x24, 20, 0xa3]);
        }
        _ => mmr(out, pick.mmr),
    }
    out.push(0x48);
    let end = out.len();
    bind(out, remote, end)?;
    out.extend(native_tier);
    out.extend([0x24, pick.tier, 0xaf]); // native tier > chosen tier
    let fallback = branch(out, 0x11);
    if field == "SkillTier" {
        out.extend([0x24, pick.tier]);
    } else {
        // Stable offset of 10–20 MMR on either side of the local value.
        op(out, 0x25, pick.mmr - 100);
        read_this(out, row);
        out.extend([0x24, 7, 0xa2, 0x24, 11, 0xa4, 0x24, 10, 0xa0]);
        read_this(out, row);
        out.extend([0x24, 2, 0xa4, 0x12, 1, 0, 0, 0x90]);
        out.extend([0xa0, 0x24, 20, 0xa3]);
        if field == "SkillMMR" {
            out.extend(change);
            out.extend([0x24, 20, 0xa3, 0xa0]);
        }
    }
    out.push(0x48);
    Ok(Some(fallback))
}

fn getter(abc: &Abc, traits: &[Trait], field: &str) -> Option<usize> {
    traits
        .iter()
        .find(|entry| entry.kind == 2 && abc.name(entry.name).is_ok_and(|name| name.local == field))
        .map(|entry| entry.name)
}

pub(in crate::patcher::title_patcher) fn patch(
    data: &[u8],
    picks: &[RankPick],
) -> Result<Option<(Vec<u8>, usize)>> {
    let abc = Abc::parse(data)?;
    let event = abc.classes.iter().find_map(|class| {
        (abc.name(class.name).ok()?.full == "tagame.api:GameEvent")
            .then(|| getter(&abc, &class.statics, "PlaylistId"))
            .flatten()
            .map(|playlist| (class.name, playlist))
    });
    if picks.is_empty() { return Err("No rank overrides selected".into()); }
    let mut replacements = Vec::new();
    for class in &abc.classes {
        let owner = abc.name(class.name)?.full.as_str();
        let fields = match owner {
            "tagame.api:PlaylistsRow" => MENU_FIELDS,
            "tagame.api:PlayerInfoRow" => PLAYER_FIELDS,
            _ => continue,
        };
        let menu = owner == "tagame.api:PlaylistsRow";
        let Some(selector) = getter(
            &abc,
            &class.fields,
            if menu { "Id" } else { "bLocalPlayer" },
        ) else {
            continue;
        };
        // RowIndex is inherited in some UI builds. The stock getter validates
        // the actual lookup expression before we reuse its multiname.
        let mut native_tier = &[][..];
        let mut change = vec![0x24, 0];
        let mut row_name = None;
        if !menu {
            let source = TitleIdSource::Property(selector);
            let Ok(tier) = native_player_value(&abc, data, class, "SkillTier", &source) else {
                continue;
            };
            native_tier = tier;
            let mut reader = Reader { data: tier, at: 1 };
            reader.index()?;
            reader.byte()?;
            reader.index()?;
            reader.take(2)?;
            row_name = Some(reader.index()?);
            if let (Ok(now), Ok(before)) = (
                native_player_value(&abc, data, class, "SkillMMR", &source),
                native_player_value(&abc, data, class, "PrevSkillMMR", &source),
            ) {
                change.clear();
                change.extend(now);
                change.extend(before);
                change.extend([0xa1, 0x24, 20, 0xa2]);
                // Guard a NaN delta and round the signed difference.
                change.extend([0x2a, 0x2a, 0x13, 3, 0, 0, 0x29, 0x24, 0]);
                change.extend([0x24, 2, 0xa2]);
                op(&mut change, 0x25, 20001);
                change.extend([0xa0, 0x24, 2, 0xa3, 0x73]);
                op(&mut change, 0x25, 10000);
                change.push(0xa1);
            }
        }
        for entry in &class.fields {
            let field = abc.name(entry.name)?.local.as_str();
            if entry.kind != 2 || !fields.contains(&field) {
                continue;
            }
            let Some(body) = abc.bodies.iter().find(|body| Some(body.id) == entry.method) else {
                continue;
            };
            let code = &data[body.code.clone()];
            if body.exceptions != 0 || code.last() != Some(&0x48) {
                continue;
            }
            let Ok(prologue) = prologue_len(code) else {
                continue;
            };
            let mut output = code[..prologue].to_vec();
            if menu {
                // A stock getter starts with a DataAPI lookup after optional
                // debug opcodes. Refuse a previously injected getter.
                let mut native = Reader {
                    data: code,
                    at: prologue,
                };
                skip_debug(&mut native)?;
                if native.byte()? != 0x60
                    || abc.name(native.index()?)?.full != "projectx.api:DataAPI"
                {
                    return Err(format!("Unexpected bytecode in {owner}.{field}"));
                }
                for pick in picks {
                    read_this(&mut output, selector);
                    output.extend([0x24, pick.playlist]);
                    let mismatch = branch(&mut output, 0x14);
                    menu_value(&mut output, field, pick);
                    output.push(0x48);
                    let end = output.len();
                    bind(&mut output, mismatch, end)?;
                }
            } else {
                let Some(row) = row_name else { continue };
                if native_player_value(&abc, data, class, field, &TitleIdSource::Property(selector))
                    .is_err()
                {
                    continue;
                }
                let mut fallbacks = Vec::new();
                if let Some((class, playlist_id)) = event {
                    for pick in picks {
                        op(&mut output, 0x60, class);
                        op(&mut output, 0x66, playlist_id);
                        output.extend([0x24, pick.playlist]);
                        let mismatch = branch(&mut output, 0x14);
                        fallbacks.extend(player_block(
                            &mut output,
                            field,
                            pick,
                            selector,
                            row,
                            native_tier,
                            &change,
                        )?);
                        let end = output.len();
                        bind(&mut output, mismatch, end)?;
                    }
                }
                // Unknown/unselected playlists must keep their native rank.
                // Match size cannot distinguish Doubles, Hoops and Heatseeker;
                // a strongest-rank fallback also leaked into unrelated modes.
                let end = output.len();
                for fallback in fallbacks {
                    bind(&mut output, fallback, end)?;
                }
            }
            output.extend(&code[prologue..]);
            let mut replacement = Vec::new();
            for value in [
                body.id,
                body.stack.max(8),
                body.locals,
                body.init_scope,
                body.max_scope,
                output.len(),
            ] {
                put(&mut replacement, value);
            }
            replacement.extend(output);
            replacement.extend(&data[body.code.end..body.span.end]);
            replacements.push((body.span.clone(), replacement));
        }
    }
    if replacements.is_empty() {
        return Ok(None);
    }
    replacements.sort_by_key(|(span, _)| span.start);
    let count = replacements.len();
    let mut output = Vec::new();
    let mut at = 0;
    for (span, replacement) in replacements {
        output.extend(
            data.get(at..span.start)
                .ok_or("Duplicate rank getter body")?,
        );
        output.extend(replacement);
        at = span.end;
    }
    output.extend(&data[at..]);
    Abc::parse(&output)?;
    Ok(Some((output, count)))
}

#[cfg(test)]
#[test]
fn equal_size_playlists_use_distinct_ids_and_retain_native_fallback() {
    // An extracted, unmodified SWF/GFx movie; read only, never a game write.
    let Some(path) = std::env::var_os("HEBNIX_RANK_MOVIE_FIXTURE") else { return };
    let raw = std::fs::read(path).unwrap();
    let picks = [
        RankPick { playlist: 11, tier: 10, division: 1, mmr: 900, players: 4 },
        RankPick { playlist: 63, tier: 22, division: 0, mmr: 1800, players: 4 },
    ];
    let checked = std::cell::Cell::new(0);
    super::super::movie::rewrite(&raw, &|data| {
        let original = Abc::parse(data)?;
        let Some((patched, _)) = patch(data, &picks)? else { return Ok(None) };
        let after = Abc::parse(&patched)?;
        let Some(event) = after.classes.iter().find(|class| after.name(class.name).is_ok_and(|name| name.full == "tagame.api:GameEvent")) else { return Ok(None) };
        let playlist = getter(&after, &event.statics, "PlaylistId").unwrap();
        let Some(player) = after.classes.iter().find(|class| after.name(class.name).is_ok_and(|name| name.full == "tagame.api:PlayerInfoRow")) else { return Ok(None) };
        for entry in &player.fields {
            let field = after.name(entry.name)?.local.as_str();
            if entry.kind != 2 || !PLAYER_FIELDS.contains(&field) { continue }
            let body = after.bodies.iter().find(|body| Some(body.id) == entry.method).unwrap();
            let before = original.bodies.iter().find(|before| before.id == body.id).unwrap();
            let code = &patched[body.code.clone()];
            let stock = &data[before.code.clone()];
            for pick in picks {
                let mut selector = Vec::new();
                op(&mut selector, 0x60, event.name);
                op(&mut selector, 0x66, playlist);
                selector.extend([0x24, pick.playlist, 0x14]);
                assert!(code.windows(selector.len()).any(|window| window == selector));
            }
            assert!(code.ends_with(&stock[prologue_len(stock)?..]), "missing native fallback for {field}");
            checked.set(checked.get() + 1);
        }
        Ok(None)
    }).unwrap();
    assert!(checked.get() >= PLAYER_FIELDS.len());
}
