use super::{TitleOverride, abc};
use std::io::{Read, Write};

type Result<T> = std::result::Result<T, String>;
// the all-language glyph merge grows GFX_Fonts_SF's font movie well past 32 MiB
const MAX_MOVIE: usize = 256 * 1024 * 1024;
fn u32_at(data: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4)
            .ok_or("Truncated UI movie")?
            .try_into()
            .unwrap(),
    ) as usize)
}

pub(super) fn patch(raw: &[u8], rules: &[TitleOverride]) -> Result<Option<(Vec<u8>, usize)>> {
    rewrite(raw, &|abc| abc::patch(abc, rules))
}

/// Apply a template's changed ABC blocks to the live movie, retaining its
/// other tags. Block names identify candidates and ABC validates the edit.
pub(in crate::patcher) fn apply_template(
    live: &[u8],
    original: &[u8],
    patched: &[u8],
) -> Result<Option<Vec<u8>>> {
    fn blocks(raw: &[u8]) -> Result<std::collections::HashMap<Vec<u8>, Vec<u8>>> {
        let mut blocks = std::collections::HashMap::new();
        rewrite_blocks(raw, &mut |header, data| {
            if blocks.insert(header.to_vec(), data.to_vec()).is_some() {
                return Err("Ambiguous ABC block name in movie template".into());
            }
            Ok(None)
        })?;
        Ok(blocks)
    }
    let original = blocks(original)?;
    let mut patched = blocks(patched)?;
    if original.len() != patched.len() || !original.keys().all(|key| patched.contains_key(key)) {
        return Err("Movie template changed the ABC block layout".into());
    }
    patched.retain(|name, code| original[name] != *code);
    let mut found = std::collections::HashSet::new();
    let result = rewrite_blocks(live, &mut |name, data| {
        let Some(patch) = patched.get(name) else {
            return Ok(None);
        };
        if !found.insert(name.to_vec()) {
            return Err("Ambiguous target ABC block in live movie".into());
        }
        abc::apply_template(data, &original[name], patch)
            .map(|bytes| bytes.map(|bytes| (bytes, 1)))
    })?;
    if found.len() != patched.len() {
        return Err("A target ABC block is missing from the live movie".into());
    }
    Ok(result.map(|(bytes, _)| bytes))
}

/// Run `edit` over every DoABC block of a GFx/SWF movie and rebuild the movie
/// when any block changed. `edit` returns the new ABC and a change count.
pub(in crate::patcher) fn rewrite(
    raw: &[u8],
    edit: &dyn Fn(&[u8]) -> Result<Option<(Vec<u8>, usize)>>,
) -> Result<Option<(Vec<u8>, usize)>> {
    rewrite_blocks(raw, &mut |_, data| edit(data))
}

fn rewrite_blocks(
    raw: &[u8],
    edit: &mut impl FnMut(&[u8], &[u8]) -> Result<Option<(Vec<u8>, usize)>>,
) -> Result<Option<(Vec<u8>, usize)>> {
    if raw.len() < 9 {
        return Err("Truncated title UI movie".into());
    }
    let signature = &raw[..3];
    let compressed = matches!(signature, b"CFX" | b"CWS");
    if !compressed && !matches!(signature, b"GFX" | b"FWS") {
        return Err("Unsupported title movie encoding".into());
    }
    let size = u32_at(raw, 4)?;
    if !(9..=MAX_MOVIE).contains(&size) {
        return Err("Unsupported title movie size".into());
    }
    let mut plain = raw[..8].to_vec();
    if compressed {
        flate2::read::ZlibDecoder::new(&raw[8..])
            .take(MAX_MOVIE as u64)
            .read_to_end(&mut plain)
            .map_err(|e| e.to_string())?;
    } else {
        plain.extend_from_slice(&raw[8..]);
    }
    if plain.len() != size {
        return Err("Title movie length mismatch".into());
    }
    let bits = plain[8] >> 3;
    let start = 8 + (5 + 4 * bits as usize).div_ceil(8) + 4;
    let mut at = start;
    let mut output = plain
        .get(..start)
        .ok_or("Truncated title movie header")?
        .to_vec();
    let mut changed = 0;
    while at < plain.len() {
        let tag_start = at;
        let tag = u16::from_le_bytes(
            plain
                .get(at..at + 2)
                .ok_or("Truncated movie tag")?
                .try_into()
                .unwrap(),
        );
        at += 2;
        let len = if tag & 63 == 63 {
            let len = u32_at(&plain, at)?;
            at += 4;
            len
        } else {
            (tag & 63) as usize
        };
        let end = at
            .checked_add(len)
            .filter(|end| *end <= plain.len())
            .ok_or("Invalid movie tag length")?;
        if tag >> 6 == 82 {
            let data = &plain[at..end];
            let nul = data
                .get(4..)
                .and_then(|name| name.iter().position(|b| *b == 0))
                .ok_or("Invalid DoABC name")?
                + 4;
            let abc_start = nul + 1;
            if let Some((abc, count)) = edit(&data[..abc_start], &data[abc_start..])? {
                let len = abc_start + abc.len();
                output.extend_from_slice(&((82u16 << 6) | 63).to_le_bytes());
                output.extend_from_slice(&(len as u32).to_le_bytes());
                output.extend_from_slice(&data[..abc_start]);
                output.extend(abc);
                changed += count;
            } else {
                output.extend_from_slice(&plain[tag_start..end]);
            }
        } else {
            output.extend_from_slice(&plain[tag_start..end]);
        }
        at = end;
        if tag >> 6 == 0 {
            output.extend_from_slice(&plain[at..]);
            break;
        }
    }
    if changed == 0 {
        return Ok(None);
    }
    let size = u32::try_from(output.len()).map_err(|_| "Title movie too large")?;
    output[4..8].copy_from_slice(&size.to_le_bytes());
    if compressed {
        let mut compressed =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        compressed
            .write_all(&output[8..])
            .map_err(|e| e.to_string())?;
        let mut encoded = output[..8].to_vec();
        encoded.extend(compressed.finish().map_err(|e| e.to_string())?);
        output = encoded;
    }
    Ok(Some((output, changed)))
}

/// Undo one patch's ABC edits while keeping the other methods and movie tags.
pub(super) fn revert(live: &[u8], before: &[u8], after: &[u8]) -> Result<Option<(Vec<u8>, usize)>> {
    fn blocks(raw: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut blocks = Vec::new();
        rewrite_blocks(raw, &mut |header, data| {
            blocks.push((header.to_vec(), data.to_vec()));
            Ok(None)
        })?;
        Ok(blocks)
    }

    let before = blocks(before)?;
    let after = blocks(after)?;
    let mut edits = before.iter().zip(&after);
    let result = rewrite_blocks(live, &mut |header, data| {
        let Some(((old_header, old), (new_header, new))) = edits.next() else {
            return Err("Unexpected ABC block in shared UI movie".into());
        };
        if header != old_header || header != new_header {
            return Err("ABC block order changed in shared UI movie".into());
        }
        let result = abc::revert(data, old, new)?;
        Ok(result.map(|data| (data, 1)))
    })?;
    if before.len() != after.len() || edits.next().is_some() {
        return Err("ABC blocks are missing from shared UI movie".into());
    }
    Ok(result)
}


#[cfg(test)]
mod tests {
    #[test]
    fn a_movie_larger_than_32_mib_without_title_code_is_left_alone() {
        // FWS header, empty RECT, frame rate and count, one 40 MiB tag, End
        let mut swf = vec![b'F', b'W', b'S', 10, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        swf.extend((2u16 << 6 | 63).to_le_bytes());
        swf.extend((40u32 << 20).to_le_bytes());
        swf.resize(swf.len() + (40 << 20), 0);
        swf.extend([0, 0]);
        let len = swf.len() as u32;
        swf[4..8].copy_from_slice(&len.to_le_bytes());
        assert!(matches!(super::patch(&swf, &[]), Ok(None)));
    }
}
