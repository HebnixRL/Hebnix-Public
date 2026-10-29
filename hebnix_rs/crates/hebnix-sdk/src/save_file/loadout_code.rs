//! Loadout preset share codes, as written by LoadoutManager. Pure codec, no save access.
//! Current: `LDS5` (varint) and `LDS6` (LDS5 run-length packed). Legacy: `LDS1`, `LDS2`.
//! `LDS3`/`LDS4` are not decoded.
use base64::{Engine, engine::general_purpose::STANDARD};

pub const SLOTS: usize = 28;
const MAX_ITEMS: usize = 0x38;
const MAX_ATTRS: usize = 256;
const MAX_TOTAL: usize = 16 * 1024 * 1024;
const TEAM_LEN: usize = 0x23C;
const LEGACY_EXTRA: usize = 0x480;
// Index order is part of the format.
pub const ATTR_KEYS: [&str; 8] = [
    "TitleId",
    "Painted",
    "Certified",
    "Quality",
    "Blueprint",
    "BlueprintCost",
    "TeamEdition",
    "NoNotify",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub product_id: i32,
    pub instance_upper: u64,
    pub instance_lower: u64,
    pub series_id: i32,
    pub added_timestamp: u64,
    pub trade_hold: Option<i32>,
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Team {
    pub products: [i32; SLOTS],
    /// index into `Preset::items`
    pub items: [Option<usize>; SLOTS],
    pub team: u8,
    pub team_color: u8,
    pub custom_color: u8,
    pub set: bool,
    pub team_finish: i32,
    pub custom_finish: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub items: Vec<Item>,
    /// blue, orange
    pub teams: [Team; 2],
}

fn invalid() -> String {
    "invalid or unsupported preset code".into()
}

fn zz(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn unzz(u: u64) -> i64 {
    ((u >> 1) as i64) ^ -((u & 1) as i64)
}

fn put_varint(out: &mut Vec<u8>, mut u: u64) {
    while u > 0x7F {
        out.push((u as u8 & 0x7F) | 0x80);
        u >>= 7;
    }
    out.push(u as u8);
}

fn put_zz(out: &mut Vec<u8>, n: i32) {
    put_varint(out, zz(n as i64));
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    if s.len() > 0x10000 {
        return Err("preset name too long".into());
    }
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .p
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or_else(invalid)?;
        let v = &self.b[self.p..end];
        self.p = end;
        Ok(v)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn varint(&mut self) -> Result<u64, String> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let c = self.u8()?;
            if shift > 63 {
                return Err(invalid());
            }
            v |= ((c & 0x7F) as u64) << shift;
            shift += 7;
            if c < 0x80 {
                return Ok(v);
            }
        }
    }
    fn zz32(&mut self) -> Result<i32, String> {
        i32::try_from(unzz(self.varint()?)).map_err(|_| invalid())
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.varint()?;
        let n = usize::try_from(n)
            .ok()
            .filter(|n| *n <= 0x10000)
            .ok_or_else(invalid)?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| invalid())
    }
}

fn rle_pack(raw: &[u8]) -> Vec<u8> {
    let mut out = raw[..8].to_vec();
    let mut i = 8;
    while i < raw.len() {
        let zero = raw[i] == 0;
        let mut j = i;
        while j < raw.len() && j - i < 0x80 && (raw[j] == 0) == zero {
            j += 1;
        }
        out.push(if zero { 0x80 } else { 0 } | (j - i - 1) as u8);
        if !zero {
            out.extend_from_slice(&raw[i..j]);
        }
        i = j;
    }
    out
}

fn rle_unpack(packed: &[u8], total: usize) -> Result<Vec<u8>, String> {
    if total > MAX_TOTAL || packed.len() < 8 {
        return Err(invalid());
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&packed[..8]);
    let mut i = 8;
    while i < packed.len() {
        let c = packed[i];
        let n = (c & 0x7F) as usize + 1;
        if c & 0x80 != 0 {
            out.resize(out.len() + n, 0);
            i += 1;
        } else {
            let lit = packed.get(i + 1..i + 1 + n).ok_or_else(invalid)?;
            out.extend_from_slice(lit);
            i += 1 + n;
        }
        if out.len() > total {
            return Err(invalid());
        }
    }
    if out.len() != total {
        return Err(invalid());
    }
    Ok(out)
}

fn team_bytes(team: &Team, item_count: usize) -> Result<Vec<u8>, String> {
    let mut mask = 0u32;
    for s in 0..SLOTS {
        if team.products[s] != 0 || team.items[s].is_some() {
            mask |= 1 << s;
        }
    }
    let mut out = mask.to_le_bytes().to_vec();
    for s in 0..SLOTS {
        if mask >> s & 1 == 0 {
            continue;
        }
        put_zz(&mut out, team.products[s]);
        out.push(match team.items[s] {
            None => 0,
            Some(i) if i < item_count => (i + 1) as u8,
            Some(_) => return Err("item reference out of range".into()),
        });
    }
    out.extend_from_slice(&[
        team.team,
        team.team_color,
        team.custom_color,
        team.set as u8,
    ]);
    put_zz(&mut out, team.team_finish);
    put_zz(&mut out, team.custom_finish);
    Ok(out)
}

fn read_team(r: &mut Reader, item_count: usize) -> Result<Team, String> {
    let mask = r.u32()?;
    let mut t = Team::default();
    for s in 0..SLOTS {
        if mask >> s & 1 == 0 {
            continue;
        }
        t.products[s] = r.zz32()?;
        t.items[s] = match r.u8()? {
            0 => None,
            i if (i as usize) <= item_count => Some(i as usize - 1),
            _ => return Err(invalid()),
        };
    }
    t.team = r.u8()?;
    t.team_color = r.u8()?;
    t.custom_color = r.u8()?;
    t.set = r.u8()? != 0;
    t.team_finish = r.zz32()?;
    t.custom_finish = r.zz32()?;
    Ok(t)
}

/// Encode as LDS5, RLE-pack to LDS6, keep the shorter, like LoadoutManager.
pub fn encode(preset: &Preset) -> Result<String, String> {
    if preset.items.len() > MAX_ITEMS {
        return Err("too many items in preset".into());
    }
    let mut out = b"LDS5\0\0\0\0".to_vec();
    put_str(&mut out, &preset.name)?;
    out.push(preset.items.len() as u8);
    for it in &preset.items {
        put_zz(&mut out, it.product_id);
        out.extend_from_slice(&it.instance_upper.to_le_bytes());
        out.extend_from_slice(&it.instance_lower.to_le_bytes());
        put_zz(&mut out, it.series_id);
        put_varint(&mut out, it.added_timestamp);
        out.push(it.trade_hold.is_some() as u8);
        put_zz(&mut out, it.trade_hold.unwrap_or(0));
        out.push(!it.attributes.is_empty() as u8);
        if it.attributes.len() > MAX_ATTRS {
            return Err("too many attributes on item".into());
        }
        put_varint(&mut out, it.attributes.len() as u64);
        for (key, value) in &it.attributes {
            match ATTR_KEYS.iter().position(|k| k == key) {
                Some(i) => out.push(i as u8),
                None => {
                    out.push(0xFF);
                    put_str(&mut out, key)?;
                }
            }
            match value.parse::<i64>() {
                Ok(n) if !value.is_empty() && n.to_string() == *value => {
                    out.push(0);
                    put_varint(&mut out, zz(n));
                }
                _ => {
                    out.push(1);
                    put_str(&mut out, value)?;
                }
            }
        }
    }
    let blue = team_bytes(&preset.teams[0], preset.items.len())?;
    let orange = team_bytes(&preset.teams[1], preset.items.len())?;
    let same = blue == orange;
    out.extend_from_slice(&blue);
    out.push(same as u8);
    if !same {
        out.extend_from_slice(&orange);
    }
    let total = out.len() as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    let mut packed = rle_pack(&out);
    packed[3] = b'6';
    let best = if out.len() <= packed.len() {
        out
    } else {
        packed
    };
    Ok(STANDARD.encode(best))
}

/// Decode any supported `LDS1`/`LDS2`/`LDS5`/`LDS6` code.
pub fn decode(code: &str) -> Result<Preset, String> {
    let raw = STANDARD.decode(code.trim()).map_err(|_| invalid())?;
    if raw.len() < 8 {
        return Err(invalid());
    }
    let total = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    match &raw[..4] {
        b"LDS5" => decode_lds5(&raw),
        b"LDS6" => decode_lds5(&rle_unpack(&raw, total)?),
        b"LDS1" => decode_lds1(&raw),
        b"LDS2" => {
            let name_len = total;
            let mut b = rle_unpack(&raw, name_len + LEGACY_EXTRA)?;
            let base = name_len + 8;
            for i in 0..TEAM_LEN {
                b[base + TEAM_LEN + i] ^= b[base + i];
            }
            b[3] = b'1';
            decode_lds1(&b)
        }
        _ => Err(invalid()),
    }
}

fn decode_lds5(raw: &[u8]) -> Result<Preset, String> {
    let mut r = Reader { b: raw, p: 8 };
    let name = r.str()?;
    let count = r.u8()? as usize;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let product_id = r.zz32()?;
        let instance_upper = r.u64()?;
        let instance_lower = r.u64()?;
        let series_id = r.zz32()?;
        let added_timestamp = r.varint()?;
        let has_trade = r.u8()? != 0;
        let trade = r.zz32()?;
        r.u8()?; // has-attributes flag, derived from the list on encode
        let attr_count = r.varint()? as usize;
        if attr_count > MAX_ATTRS {
            return Err(invalid());
        }
        let mut attributes = Vec::with_capacity(attr_count);
        for _ in 0..attr_count {
            let key = match r.u8()? {
                0xFF => r.str()?,
                k => ATTR_KEYS.get(k as usize).ok_or_else(invalid)?.to_string(),
            };
            let value = match r.u8()? {
                0 => unzz(r.varint()?).to_string(),
                1 => r.str()?,
                _ => return Err(invalid()),
            };
            attributes.push((key, value));
        }
        items.push(Item {
            product_id,
            instance_upper,
            instance_lower,
            series_id,
            added_timestamp,
            trade_hold: has_trade.then_some(trade),
            attributes,
        });
    }
    let blue = read_team(&mut r, items.len())?;
    let orange = if r.u8()? != 0 {
        blue.clone()
    } else {
        read_team(&mut r, items.len())?
    };
    if r.p != raw.len() {
        return Err(invalid());
    }
    Ok(Preset {
        name,
        items,
        teams: [blue, orange],
    })
}

/// Legacy fixed-width layout. No item records, so each occupied slot becomes an item.
fn decode_lds1(raw: &[u8]) -> Result<Preset, String> {
    let name_len = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    if raw.len() != name_len + 8 + 2 * TEAM_LEN {
        return Err(invalid());
    }
    let name = String::from_utf8(raw[8..8 + name_len].to_vec()).map_err(|_| invalid())?;
    let mut r = Reader {
        b: raw,
        p: 8 + name_len,
    };
    let mut items: Vec<Item> = Vec::new();
    let mut teams: Vec<Team> = Vec::with_capacity(2);
    for _ in 0..2 {
        let mut t = Team::default();
        let mut products = [0u32; SLOTS];
        for p in products.iter_mut() {
            *p = r.u32()?;
        }
        let mut instances = [(0u64, 0u64); SLOTS];
        for i in instances.iter_mut() {
            *i = (r.u64()?, r.u64()?);
        }
        for s in 0..SLOTS {
            t.products[s] = i32::try_from(products[s]).map_err(|_| invalid())?;
            let (upper, lower) = instances[s];
            if upper == 0 && lower == 0 {
                continue;
            }
            let found = items
                .iter()
                .position(|it| it.instance_upper == upper && it.instance_lower == lower);
            t.items[s] = Some(found.unwrap_or_else(|| {
                items.push(Item {
                    product_id: t.products[s],
                    instance_upper: upper,
                    instance_lower: lower,
                    ..Item::default()
                });
                items.len() - 1
            }));
        }
        t.team = r.u8()?;
        t.team_color = r.u8()?;
        t.custom_color = r.u8()?;
        t.set = r.u8()? != 0;
        t.team_finish = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
        t.custom_finish = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
        teams.push(t);
    }
    let orange = teams.pop().unwrap();
    let blue = teams.pop().unwrap();
    Ok(Preset {
        name,
        items,
        teams: [blue, orange],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Preset {
        let item = Item {
            product_id: 4284,
            instance_upper: 0x1122_3344_5566_7788,
            instance_lower: 0x99AA_BBCC_DDEE_FF00,
            series_id: 0,
            added_timestamp: 1_700_000_000,
            trade_hold: None,
            attributes: vec![
                ("Painted".into(), "5".into()),
                ("Certified".into(), "-3".into()),
                ("Custom".into(), "abc".into()),
            ],
        };
        let mut blue = Team {
            team_color: 3,
            custom_color: 90,
            set: true,
            team_finish: 270,
            custom_finish: -1,
            ..Team::default()
        };
        blue.products[0] = 23;
        blue.items[1] = Some(0);
        let orange = Team {
            team: 1,
            team_color: 5,
            ..blue.clone()
        };
        Preset {
            name: "Fennec ★".into(),
            items: vec![item],
            teams: [blue, orange],
        }
    }

    // Byte-exact against lds_codec.py (encode of the same preset).
    const PY_FENNEC: &str = "TERTNWIAAAAKRmVubmVjIOKYhQH4Qoh3ZlVEMyIRAP/u3cy7qpkAgOLPqgYAAAEDAQAKAgAF/wZDdXN0b20BA2FiYwMAAAAuAAABAANaAZwEAQADAAAALgAAAQEFWgGcBAE=";

    #[test]
    fn matches_reference_encoder() {
        assert_eq!(encode(&sample()).unwrap(), PY_FENNEC);
    }

    #[test]
    fn decodes_reference_code() {
        assert_eq!(decode(PY_FENNEC).unwrap(), sample());
    }

    #[test]
    fn round_trips_same_team() {
        let mut p = sample();
        p.teams[1] = p.teams[0].clone();
        assert_eq!(decode(&encode(&p).unwrap()).unwrap(), p);
    }

    // Reference LDS1: name "old", two teams with products 100..127 and no instances.
    const PY_LDS1: &str = "TERTMQMAAABvbGRkAAAAZQAAAGYAAABnAAAAaAAAAGkAAABqAAAAawAAAGwAAABtAAAAbgAAAG8AAABwAAAAcQAAAHIAAABzAAAAdAAAAHUAAAB2AAAAdwAAAHgAAAB5AAAAegAAAHsAAAB8AAAAfQAAAH4AAAB/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAgEHAAAA+P///2QAAABlAAAAZgAAAGcAAABoAAAAaQAAAGoAAABrAAAAbAAAAG0AAABuAAAAbwAAAHAAAABxAAAAcgAAAHMAAAB0AAAAdQAAAHYAAAB3AAAAeAAAAHkAAAB6AAAAewAAAHwAAAB9AAAAfgAAAH8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAECAQcAAAD4////";

    #[test]
    fn decodes_legacy_lds1() {
        let p = decode(PY_LDS1).unwrap();
        assert_eq!(p.name, "old");
        assert_eq!(p.teams[1].products[27], 127);
        assert_eq!(p.teams[0].team_finish, 7);
        assert_eq!(p.teams[1].custom_finish, -8);
        assert!(p.items.is_empty());
    }

    #[test]
    fn rejects_bad_reference() {
        let mut p = sample();
        p.teams[0].items[3] = Some(9);
        assert!(encode(&p).is_err());
        assert!(decode("AAAA").is_err());
    }
}
