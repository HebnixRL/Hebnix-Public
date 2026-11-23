//! All-language glyphs. GFX_Fonts_SF.upk holds one Scaleform font movie per
//! language (Fonts_EFIGS, _RUS, _KOR, _JPN, _CHN) and the game loads only the
//! one for the UI language, so with English UI a Korean, Japanese, Chinese or
//! Cyrillic name shows as blocks. The patch copies every glyph the EFIGS text
//! faces lack from the other movies' faces into them (DefineFont3 tags), built
//! from the player's own package. The original package is kept in the backups
//! folder and put back on restore.
use super::upk_package::UpkPackage;
use super::workshop_menu_ui::Movie;
use super::{backup_guard, game_path};
use std::fs;
use std::path::{Path, PathBuf};

const PACKAGE: &str = "GFX_Fonts_SF.upk";
const BACKUP: &str = "GFX_Fonts_SF.upk.bak";
const MARKER: &str = "GFX_Fonts_SF.upk.build.sha256";

const fn movie(export: &'static str) -> Movie {
    Movie { package: PACKAGE, export, patched: &[], vanilla: "" }
}

const TARGET: Movie = movie("GFX_Fonts.Fonts_EFIGS");
/// Donors in order of preference: a code point comes from the first that has it.
const DONORS: [Movie; 4] = [
    movie("GFX_Fonts.Fonts_RUS"),
    movie("GFX_Fonts.Fonts_KOR"),
    movie("GFX_Fonts.Fonts_JPN"),
    movie("GFX_Fonts.Fonts_CHN"),
];
const DEFINE_FONT3: u16 = 75;
/// Faces with fewer glyphs are icon or digit fonts, not text.
const TEXT_FACE_GLYPHS: usize = 100;

struct Layout {
    ascent: [u8; 2],
    descent: [u8; 2],
    leading: [u8; 2],
    advances: Vec<[u8; 2]>,
    /// raw byte-aligned RECT per glyph
    bounds: Vec<Vec<u8>>,
    /// raw KerningCount and records
    kerning: Vec<u8>,
}

struct Font {
    id: u16,
    flags: u8,
    language: u8,
    name: Vec<u8>,
    shapes: Vec<Vec<u8>>,
    codes: Vec<u16>,
    layout: Option<Layout>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Byte length of the RECT at `at`.
fn rect_len(b: &[u8], at: usize) -> Option<usize> {
    let bits = usize::from(*b.get(at)? >> 3);
    Some((5 + bits * 4).div_ceil(8))
}

impl Font {
    fn parse(d: &[u8]) -> Option<Font> {
        let id = u16_at(d, 0)?;
        let flags = *d.get(2)?;
        let language = *d.get(3)?;
        let name_len = usize::from(*d.get(4)?);
        let name = d.get(5..5 + name_len)?.to_vec();
        let n = usize::from(u16_at(d, 5 + name_len)?);
        let table = 7 + name_len;
        let wide = flags & 0x08 != 0;
        let offset = |i: usize| -> Option<usize> {
            if wide { u32_at(d, table + i * 4).map(|v| v as usize) } else { u16_at(d, table + i * 2).map(usize::from) }
        };
        let code_offset = offset(n)?;
        let mut shapes = Vec::with_capacity(n);
        for i in 0..n {
            let end = if i + 1 < n { offset(i + 1)? } else { code_offset };
            shapes.push(d.get(table + offset(i)?..table + end)?.to_vec());
        }
        let codes_at = table + code_offset;
        let codes = (0..n).map(|i| u16_at(d, codes_at + i * 2)).collect::<Option<Vec<_>>>()?;
        let layout = if flags & 0x80 != 0 {
            let mut at = codes_at + n * 2;
            let pair = |o: usize| -> Option<[u8; 2]> { d.get(o..o + 2)?.try_into().ok() };
            let (ascent, descent, leading) = (pair(at)?, pair(at + 2)?, pair(at + 4)?);
            at += 6;
            let advances = (0..n).map(|i| pair(at + i * 2)).collect::<Option<Vec<_>>>()?;
            at += n * 2;
            let mut bounds = Vec::with_capacity(n);
            for _ in 0..n {
                let len = rect_len(d, at)?;
                bounds.push(d.get(at..at + len)?.to_vec());
                at += len;
            }
            Some(Layout { ascent, descent, leading, advances, bounds, kerning: d.get(at..)?.to_vec() })
        } else {
            None
        };
        Some(Font { id, flags, language, name, shapes, codes, layout })
    }

    /// The tag body, always with wide offsets and wide codes.
    fn write(&self) -> Vec<u8> {
        let n = self.codes.len();
        let mut out = self.id.to_le_bytes().to_vec();
        out.extend([self.flags | 0x0c, self.language, self.name.len() as u8]);
        out.extend(&self.name);
        out.extend((n as u16).to_le_bytes());
        let mut at = 4 * n + 4;
        for shape in &self.shapes {
            out.extend((at as u32).to_le_bytes());
            at += shape.len();
        }
        out.extend((at as u32).to_le_bytes());
        for shape in &self.shapes {
            out.extend(shape);
        }
        for code in &self.codes {
            out.extend(code.to_le_bytes());
        }
        if let Some(layout) = &self.layout {
            out.extend(layout.ascent.iter().chain(&layout.descent).chain(&layout.leading));
            out.extend(layout.advances.iter().flatten());
            out.extend(layout.bounds.iter().flatten());
            out.extend(&layout.kerning);
        }
        out
    }

    fn name(&self) -> String {
        String::from_utf8_lossy(&self.name).trim_end_matches('\0').to_string()
    }

    fn heavy(&self) -> bool {
        let name = self.name();
        self.flags & 0x01 != 0 || ["Bold", "Medium", "DB", "Heavy"].iter().any(|w| name.contains(w))
    }

    fn is_text(&self) -> bool {
        self.codes.len() >= TEXT_FACE_GLYPHS && self.layout.is_some()
    }

    /// Add `donor`'s glyphs for code points this face lacks; the result stays sorted by code.
    fn merge(&mut self, donor: &Font) -> usize {
        let (Some(layout), Some(theirs)) = (self.layout.as_mut(), donor.layout.as_ref()) else { return 0 };
        let mut glyphs: Vec<_> = (0..self.codes.len())
            .map(|i| (self.codes[i], self.shapes[i].clone(), layout.advances[i], layout.bounds[i].clone()))
            .collect();
        let before = glyphs.len();
        for i in 0..donor.codes.len() {
            if glyphs.binary_search_by_key(&donor.codes[i], |g| g.0).is_err() {
                glyphs.push((donor.codes[i], donor.shapes[i].clone(), theirs.advances[i], theirs.bounds[i].clone()));
            }
        }
        glyphs.sort_by_key(|g| g.0);
        let added = glyphs.len() - before;
        self.codes = glyphs.iter().map(|g| g.0).collect();
        self.shapes = glyphs.iter().map(|g| g.1.clone()).collect();
        layout.advances = glyphs.iter().map(|g| g.2).collect();
        layout.bounds = glyphs.into_iter().map(|g| g.3).collect();
        added
    }
}

/// Offset of the first tag after the SWF header.
fn first_tag(swf: &[u8]) -> Option<usize> {
    Some(8 + rect_len(swf, 8)? + 4)
}

/// (tag code, body start, body end) for every tag.
fn tags(swf: &[u8]) -> Option<Vec<(u16, usize, usize)>> {
    let mut out = Vec::new();
    let mut p = first_tag(swf)?;
    while p + 2 <= swf.len() {
        let head = u16_at(swf, p)?;
        let (code, mut len) = (head >> 6, usize::from(head & 0x3f));
        p += 2;
        if len == 0x3f {
            len = u32_at(swf, p)? as usize;
            p += 4;
        }
        if p + len > swf.len() {
            return None;
        }
        out.push((code, p, p + len));
        p += len;
        if code == 0 {
            break;
        }
    }
    Some(out)
}

fn fonts(swf: &[u8]) -> Vec<Font> {
    tags(swf)
        .unwrap_or_default()
        .into_iter()
        .filter(|(code, ..)| *code == DEFINE_FONT3)
        .filter_map(|(_, start, end)| Font::parse(&swf[start..end]))
        .collect()
}

/// The target movie with every text face given the donors' missing glyphs.
fn merged(target: &[u8], donors: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let donors: Vec<Font> = donors.iter().flat_map(|swf| fonts(swf)).filter(Font::is_text).collect();
    let tags = tags(target).ok_or("The font movie could not be read")?;
    let mut out = target[..first_tag(target).ok_or("The font movie is empty")?].to_vec();
    let mut faces = 0;
    for (code, start, end) in tags {
        let mut body = target[start..end].to_vec();
        if code == DEFINE_FONT3
            && let Some(mut font) = Font::parse(&body).filter(Font::is_text)
        {
            let heavy = font.heavy();
            // per donor movie order, prefer the face of matching weight
            let mut added = 0;
            for pass in [true, false] {
                for donor in donors.iter().filter(|d| (d.heavy() == heavy) == pass) {
                    added += font.merge(donor);
                }
            }
            if added > 0 {
                faces += 1;
            }
            if font.codes.len() > usize::from(u16::MAX) {
                return Err(format!("{} would have too many glyphs", font.name()));
            }
            body = font.write();
        }
        out.extend((code << 6 | 0x3f).to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
    }
    if faces == 0 {
        return Err("No font face needed extra glyphs".into());
    }
    let len = out.len() as u32;
    out[4..8].copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

/// Whether the EFIGS text faces already hold Hangul.
fn is_merged(swf: &[u8]) -> bool {
    fonts(swf).iter().filter(|f| f.is_text()).any(|f| f.codes.iter().any(|&c| (0xac00..0xd7a4).contains(&c)))
}

fn backup_path(backups: &Path) -> PathBuf {
    backups.join(BACKUP)
}

pub fn enabled(backups: &Path) -> bool {
    backup_path(backups).is_file()
}

fn ensure_closed() -> Result<(), String> {
    if hebnix_sdk::process::is_rocket_league_running() {
        Err("Close Rocket League before changing the all-language font.".into())
    } else {
        Ok(())
    }
}

pub fn enable(cooked: &Path, backups: &Path) -> Result<(), String> {
    ensure_closed()?;
    let live = game_path::resolve(cooked, PACKAGE);
    let mut upk = UpkPackage::load(&live)?;
    let target = TARGET.movie(&upk)?;
    if is_merged(&target) {
        return if enabled(backups) { Ok(()) } else { Err(format!("{PACKAGE} is already patched but its original is missing. Verify the game files.")) };
    }
    let donors = DONORS.iter().map(|m| m.movie(&upk)).collect::<Result<Vec<_>, _>>()?;
    let swf = merged(&target, &donors)?;
    backup_guard::prepare(cooked, backups, MARKER, |name| name == BACKUP)?;
    let backup = backup_path(backups);
    fs::copy(&live, &backup).map_err(|e| format!("Could not back up {PACKAGE}: {e}"))?;
    let temp = cooked.join(format!("{PACKAGE}.font.tmp"));
    let result = TARGET.with_movie(&mut upk, &swf, &temp).and_then(|()| {
        let saved = UpkPackage::load(&temp)?;
        if !is_merged(&TARGET.movie(&saved)?) {
            return Err(format!("Patched {PACKAGE} failed validation"));
        }
        fs::rename(&temp, &live).map_err(|e| format!("Could not write {PACKAGE}: {e}"))
    });
    let _ = fs::remove_file(&temp);
    if result.is_err() {
        let _ = fs::remove_file(&backup);
    }
    result
}

pub fn restore(cooked: &Path, backups: &Path) -> Result<(), String> {
    let backup = backup_path(backups);
    if !backup.is_file() {
        return Ok(());
    }
    ensure_closed()?;
    backup_guard::check(cooked, backups, MARKER, |name| name == BACKUP)?;
    let live = game_path::resolve(cooked, PACKAGE);
    fs::copy(&backup, &live).map_err(|e| format!("Could not restore {PACKAGE}: {e}"))?;
    let _ = fs::remove_file(&backup);
    let _ = fs::remove_file(backups.join(MARKER));
    Ok(())
}

/// Page state: the job writing the package.
#[derive(Default)]
pub struct State {
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    status: String,
}

impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn render(&mut self, ui: &mut eframe::egui::Ui, cooked: &Path, backups: &Path) {
        use crate::i18n::t;
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    self.status = result.unwrap_or_else(|e| e);
                    self.job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            }
        }
        let on = enabled(backups);
        let mut want = on;
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.checkbox(&mut want, t("game-mod-font-glyphs")).on_hover_text(t("game-mod-font-glyphs-description"));
            ui.separator();
            ui.label(t("game-mod-font-glyphs-description"));
        });
        if want != on {
            let (cooked, backups) = (cooked.to_path_buf(), backups.to_path_buf());
            let (tx, rx) = std::sync::mpsc::channel();
            self.job = Some(rx);
            self.status.clear();
            std::thread::spawn(move || {
                let result = if want {
                    enable(&cooked, &backups).map(|_| t("game-mod-font-glyphs-on"))
                } else {
                    restore(&cooked, &backups).map(|_| t("game-mod-font-glyphs-off"))
                };
                let _ = tx.send(result);
            });
        }
        if self.busy() {
            ui.spinner();
        } else if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(id: u16, name: &str, glyphs: &[(u16, &[u8])]) -> Font {
        Font {
            id,
            flags: 0x80,
            language: 1,
            name: format!("{name}\0").into_bytes(),
            shapes: glyphs.iter().map(|g| g.1.to_vec()).collect(),
            codes: glyphs.iter().map(|g| g.0).collect(),
            layout: Some(Layout {
                ascent: [1, 0],
                descent: [2, 0],
                leading: [3, 0],
                advances: glyphs.iter().map(|g| [g.0 as u8, 0]).collect(),
                // nbits 2: 5 + 8 bits, two bytes
                bounds: glyphs.iter().map(|g| vec![0x10, g.0 as u8]).collect(),
                kerning: vec![0, 0],
            }),
        }
    }

    fn movie(fonts: &[Font]) -> Vec<u8> {
        // GFX, version 10, length, RECT with nbits 0 (one byte), frame rate and count
        let mut out = b"GFX\x0a\0\0\0\0\x00\x00\x18\x01\x00".to_vec();
        for f in fonts {
            let body = f.write();
            out.extend((DEFINE_FONT3 << 6 | 0x3f).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend(body);
        }
        out.extend([0, 0]);
        let len = out.len() as u32;
        out[4..8].copy_from_slice(&len.to_le_bytes());
        out
    }

    /// HEBNIX_FONTS_UPK=<GFX_Fonts_SF.upk> cargo test real_package -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_package() {
        let upk = UpkPackage::load(Path::new(&std::env::var("HEBNIX_FONTS_UPK").unwrap())).unwrap();
        let target = TARGET.movie(&upk).unwrap();
        let donors: Vec<_> = DONORS.iter().map(|m| m.movie(&upk).unwrap()).collect();
        for swf in std::iter::once(&target).chain(&donors) {
            for f in fonts(swf) {
                println!("{} glyphs={} heavy={} text={}", f.name(), f.codes.len(), f.heavy(), f.is_text());
            }
        }
        let out = merged(&target, &donors).unwrap();
        assert!(is_merged(&out));
        for f in fonts(&out) {
            println!("merged {} glyphs={}", f.name(), f.codes.len());
        }
        println!("movie {} -> {} bytes", target.len(), out.len());
    }

    #[test]
    fn a_face_round_trips_through_write_and_parse() {
        let f = font(7, "Arial Narrow", &[(65, &[1, 2, 3]), (66, &[4])]);
        let back = Font::parse(&f.write()).unwrap();
        assert_eq!(back.codes, f.codes);
        assert_eq!(back.shapes, f.shapes);
        assert_eq!(back.layout.unwrap().bounds, f.layout.unwrap().bounds);
    }

    #[test]
    fn missing_glyphs_are_added_in_code_order_and_existing_ones_kept() {
        let latin: Vec<(u16, &[u8])> = (32..32 + TEXT_FACE_GLYPHS as u16).map(|c| (c, &[9u8][..])).collect();
        let target = movie(&[font(1, "Arial Narrow", &latin)]);
        let mut korean = latin.clone();
        korean[0].1 = &[7];
        korean.push((0xac00, &[5, 5]));
        korean.push((0x0410, &[6]));
        let donor = movie(&[font(1, "Korean Face", &korean)]);
        let out = merged(&target, &[donor]).unwrap();
        assert!(is_merged(&out));
        assert!(!is_merged(&target));
        let f = &fonts(&out)[0];
        assert_eq!(f.codes.len(), TEXT_FACE_GLYPHS + 2);
        assert!(f.codes.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(f.shapes[0], vec![9], "the face's own glyph wins");
        let at = f.codes.iter().position(|&c| c == 0xac00).unwrap();
        assert_eq!(f.shapes[at], vec![5, 5]);
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize, out.len());
    }
}
