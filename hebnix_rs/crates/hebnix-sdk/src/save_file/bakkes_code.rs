//! BakkesMod item mod codes (`cl_itemmod_code`), read only. Layout notes:
//! `Documents/bakkesmod-code-format.md`. Versions 3 and 4, checked against sample codes
//! and not against the game. Codes carry product ids, not item instances.
//! Version 4 widens product ids from 13 to 16 bits (BakkesModLoadoutLib, RL 2.21).
use super::loadout_code;
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// BakkesMod equip slot, not the save's loadout slot index
    pub slot: u8,
    pub product_id: u16,
    /// 0 = none
    pub paint: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Team {
    pub items: Vec<Item>,
    /// primary and secondary RGB, when the code overrides the paint
    pub colors: Option<([u8; 3], [u8; 3])>,
    pub wheel_team: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub version: u8,
    pub blue: Team,
    /// `None` when the code uses the blue loadout for both teams
    pub orange: Option<Team>,
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    /// LSB first within each byte, multi-bit fields low bit first
    fn read(&mut self, n: usize) -> Result<u32, String> {
        let mut v = 0;
        for i in 0..n {
            let byte = self.data.get(self.pos / 8).ok_or("code ends early")?;
            v |= u32::from(byte >> (self.pos % 8) & 1) << i;
            self.pos += 1;
        }
        Ok(v)
    }

    fn rgb(&mut self) -> Result<[u8; 3], String> {
        Ok([self.read(8)? as u8, self.read(8)? as u8, self.read(8)? as u8])
    }

    fn team(&mut self, version: u8) -> Result<Team, String> {
        let mut team = Team::default();
        let id_bits = if version >= 4 { 16 } else { 13 };
        for _ in 0..self.read(4)? {
            let slot = self.read(5)? as u8;
            let product_id = self.read(id_bits)? as u16;
            let paint = if self.read(1)? == 1 { self.read(6)? as u8 } else { 0 };
            if product_id != 0 {
                team.items.push(Item { slot, product_id, paint });
            }
        }
        if self.read(1)? == 1 {
            team.colors = Some((self.rgb()?, self.rgb()?));
        }
        team.wheel_team = self.read(6)? as u8;
        Ok(team)
    }
}

/// Equip slots BakkesMod reads. Save loadout slot indices match these numbers.
const EQUIP_SLOTS: [u8; 12] = [0, 1, 2, 3, 4, 5, 7, 12, 13, 14, 15, 18];
const CODE_VERSION: u8 = 4;

/// BakkesMod view of a preset. Team colors and wheel teams are not stored in presets.
pub fn from_preset(preset: &loadout_code::Preset) -> Result<Code, String> {
    let blue = team_from_preset(preset, 0)?;
    let orange = team_from_preset(preset, 1)?;
    let same = blue.items == orange.items;
    Ok(Code { version: CODE_VERSION, blue, orange: (!same).then_some(orange) })
}

fn team_from_preset(preset: &loadout_code::Preset, index: usize) -> Result<Team, String> {
    let team = &preset.teams[index];
    let mut out = Team::default();
    if !team.set {
        return Ok(out);
    }
    for slot in EQUIP_SLOTS {
        let product = team.products[usize::from(slot)];
        if product <= 0 {
            continue;
        }
        let product_id = u16::try_from(product).map_err(|_| format!("product {product} does not fit a BakkesMod code"))?;
        let paint = match team.items[usize::from(slot)] {
            Some(i) => painted(preset.items.get(i).ok_or("item reference out of range")?)?,
            None => 0,
        };
        out.items.push(Item { slot, product_id, paint });
    }
    Ok(out)
}

fn painted(item: &loadout_code::Item) -> Result<u8, String> {
    let Some((_, value)) = item.attributes.iter().find(|(key, _)| key == "Painted") else {
        return Ok(0);
    };
    value
        .parse::<u8>()
        .ok()
        .filter(|paint| *paint < 64)
        .ok_or_else(|| format!("paint {value} does not fit a BakkesMod code"))
}

#[derive(Default)]
struct BitsOut {
    bytes: Vec<u8>,
    pos: usize,
}

impl BitsOut {
    /// LSB first within each byte, multi-bit fields low bit first
    fn write(&mut self, v: u32, n: usize) {
        for i in 0..n {
            if self.pos / 8 == self.bytes.len() {
                self.bytes.push(0);
            }
            if v >> i & 1 == 1 {
                self.bytes[self.pos / 8] |= 1 << (self.pos % 8);
            }
            self.pos += 1;
        }
    }
}

fn write_team(out: &mut BitsOut, team: &Team) -> Result<(), String> {
    if team.items.len() > 15 {
        return Err("too many items for a BakkesMod code".into());
    }
    out.write(team.items.len() as u32, 4);
    for item in &team.items {
        out.write(u32::from(item.slot), 5);
        out.write(u32::from(item.product_id), 16);
        out.write(u32::from(item.paint > 0), 1);
        if item.paint > 0 {
            out.write(u32::from(item.paint), 6);
        }
    }
    match team.colors {
        Some((primary, secondary)) => {
            out.write(1, 1);
            for channel in primary.iter().chain(&secondary) {
                out.write(u32::from(*channel), 8);
            }
        }
        None => out.write(0, 1),
    }
    out.write(u32::from(team.wheel_team), 6);
    Ok(())
}

/// Writes the code the way BakkesModLoadoutLib does: header first, size and CRC last.
pub fn encode(code: &Code) -> Result<String, String> {
    let mut out = BitsOut::default();
    out.write(0, 24);
    out.write(u32::from(code.orange.is_none()), 1);
    write_team(&mut out, &code.blue)?;
    if let Some(orange) = &code.orange {
        write_team(&mut out, orange)?;
    }
    let bits = out.pos;
    if bits % 8 == 0 {
        out.bytes.push(0);
    }
    let size = bits.div_ceil(8);
    if size > 1023 {
        return Err("code is too long for a BakkesMod code".into());
    }
    let crc = out.bytes[3..size].iter().fold(0xFFu8, |c, b| c ^ b);
    out.pos = 0;
    out.write(u32::from(CODE_VERSION), 6);
    out.write(size as u32, 10);
    out.write(u32::from(crc), 8);
    Ok(STANDARD.encode(&out.bytes))
}

pub fn decode(code: &str) -> Result<Code, String> {
    let data = STANDARD.decode(code.trim()).map_err(|_| "not base64")?;
    let mut bits = Bits { data: &data, pos: 0 };
    let version = bits.read(6)? as u8;
    let size = bits.read(10)? as usize;
    let crc = bits.read(8)? as u8;
    if !(3..=4).contains(&version) {
        return Err(format!("code version {version} is not supported, only 3 and 4"));
    }
    // BakkesMod pads codes with one extra zero byte when the bits end on a byte boundary
    if size > data.len() || data.len() > size + 1 {
        return Err("code length does not match its header".into());
    }
    if data[3..size].iter().fold(0xFF, |c, b| c ^ b) != crc {
        return Err("code checksum is wrong".into());
    }
    let blue_is_orange = bits.read(1)? == 1;
    let blue = bits.team(version)?;
    let orange = if blue_is_orange { None } else { Some(bits.team(version)?) };
    Ok(Code { version, blue, orange })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "gwf/D/DCQxCGhI06DBCA8R5O7Rze+hEAIAABAAAD";

    #[test]
    fn sample_code_matches_the_format_notes() {
        let code = decode(SAMPLE).unwrap();
        assert!(code.orange.is_none());
        let items: Vec<_> = code.blue.items.iter().map(|i| (i.slot, i.product_id, i.paint)).collect();
        assert_eq!(
            items,
            [(0, 4284, 3), (1, 4290, 0), (2, 2614, 3), (3, 32, 0), (12, 1980, 0), (14, 1898, 3), (15, 2027, 0)]
        );
        assert_eq!(code.blue.colors, Some(([0, 0, 1], [8, 0, 0])));
        assert_eq!(code.blue.wheel_team, 24);
    }

    #[test]
    fn bad_codes_are_rejected() {
        assert!(decode("not a code!").is_err());
        let mut raw = STANDARD.decode(SAMPLE).unwrap();
        raw[10] ^= 1;
        assert!(decode(&STANDARD.encode(raw)).unwrap_err().contains("checksum"));
    }

    #[test]
    fn version_4_codes_use_16_bit_product_ids() {
        // loadouts checked in game: body, wheels (Hypnotik), Black wheel paint, Gold Rush boost
        let code = decode("xANrB/BCBMTCwmGAAAAA").unwrap();
        assert_eq!(code.version, 4);
        let items: Vec<_> = code.blue.items.iter().map(|i| (i.slot, i.product_id, i.paint)).collect();
        assert_eq!(items, [(0, 4284, 0), (2, 2827, 3), (3, 32, 0)]);
        assert!(code.orange.is_none());
    }

    #[test]
    fn encode_round_trips_in_game_codes() {
        // In-game codes. Compared as models: in-game codes flag some unpainted items
        // as paintable with paint 0, and encode writes no flag for paint 0.
        for code in [
            "xAckEfBCBELfREQsLEQGCICCDhBxiiNeS8hBQrEAAA==",
            "BAMIBfBCBMTCQgAA",
            "xAPqB/BCBMTCQmCAAAAA",
            "xANrB/BCBMTCwmGAAAAA",
            "RAStCfBCBMTCwmGAACjgAAAA",
            "RASFCfBCLMTCwmGAACjgAAAA",
        ] {
            let model = decode(code).unwrap();
            assert_eq!(decode(&encode(&model).unwrap()).unwrap(), model);
        }
    }
}
