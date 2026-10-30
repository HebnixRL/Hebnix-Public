//! BakkesMod item mod codes (`cl_itemmod_code`), read only. Layout notes:
//! `Documents/bakkesmod-code-format.md`. Versions 3 and 4, checked against sample codes
//! and not against the game. Codes carry product ids, not item instances.
//! Version 4 widens product ids from 13 to 16 bits (BakkesModLoadoutLib, RL 2.21).
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

pub fn decode(code: &str) -> Result<Code, String> {
    let data = STANDARD.decode(code.trim()).map_err(|_| "not base64")?;
    let mut bits = Bits { data: &data, pos: 0 };
    let version = bits.read(6)? as u8;
    let size = bits.read(10)? as usize;
    let crc = bits.read(8)? as u8;
    if !(3..=4).contains(&version) {
        return Err(format!("code version {version} is not supported, only 3 and 4"));
    }
    if size != data.len() {
        return Err("code length does not match its header".into());
    }
    if data[3..].iter().fold(0xFF, |c, b| c ^ b) != crc {
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
}
