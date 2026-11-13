//! Read the game's history-only export. Never reads session credentials.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

pub fn read(path: &Path, player: &str) -> Result<Value, String> {
    let mut file = File::open(path).map_err(|_| "Could not read Rocket League's current log")?;
    let length = file
        .metadata()
        .map_err(|_| "Could not inspect the game log")?
        .len();
    const MAX_LOG: u64 = 64 * 1024 * 1024;
    let skipped = length > MAX_LOG;
    if skipped {
        file.seek(SeekFrom::Start(length - MAX_LOG))
            .map_err(|_| "Could not seek in game log")?;
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    if skipped {
        reader
            .read_until(b'\n', &mut line)
            .map_err(|_| "Could not read game log")?;
    }
    let mut matches = BTreeMap::new();
    loop {
        line.clear();
        let count = reader
            .read_until(b'\n', &mut line)
            .map_err(|_| "Could not read game log")?;
        if count == 0 {
            break;
        }
        // A running log can end halfway through a buffered write. Retry on the
        // next poll; never accept or report that partial record as corrupt.
        if line.last() != Some(&b'\n') {
            break;
        }
        if line.len() > 1024 * 1024 {
            continue;
        }
        if let Ok(line) = std::str::from_utf8(&line) {
            if let Some((guid, item)) = parse_line(line, player) {
                matches.insert(guid, item);
            }
        }
    }
    if matches.is_empty() {
        return Err("Waiting for Rocket League's live MatchHistory export.".into());
    }
    Ok(json!({"LocalPlayerID":player,"Matches":matches.into_values().collect::<Vec<_>>()}))
}

fn parse_line(line: &str, player: &str) -> Option<(String, Value)> {
    let (_, body) = line.strip_prefix('[')?.split_once("] ")?;
    let raw = body.strip_prefix("MatchHistory: ")?;
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let guid = value["Match"]["MatchGUID"].as_str()?;
    if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let guid = guid.to_ascii_uppercase();
    // Refuse a cached row for another account. Spectator rows may still be
    // present here; the delivery layer separately requires actual play time.
    if !value["Match"]["Players"]
        .as_array()?
        .iter()
        .any(|p| p["PlayerID"].as_str() == Some(player))
    {
        return None;
    }
    let url = value
        .get("ReplayURL")
        .or_else(|| value.get("ReplayUrl"))?
        .as_str()?
        .to_owned();
    let parsed = url::Url::parse(&url).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("api.rlpp.psynet.gg")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port_or_known_default() != Some(443)
    {
        return None;
    }
    value.as_object_mut()?.remove("ReplayURL");
    value["ReplayUrl"] = url.into();
    Some((guid, value))
}
