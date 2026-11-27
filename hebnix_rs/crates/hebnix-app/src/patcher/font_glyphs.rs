//! All-language glyphs. GFX_Fonts_SF.upk holds one Scaleform font movie per
//! language (Fonts_EFIGS, _RUS, _KOR, _JPN, _CHN) and the game loads only the
//! one for the UI language, so with English UI a Korean, Japanese, Chinese or
//! Cyrillic name shows as blocks. The patch copies every glyph the EFIGS text
//! faces lack from the other movies' faces into them (DefineFont3 tags), built
//! from the player's own package. Code points no game face has, like the symbols
//! and scripts in decorated names, are drawn from installed system fonts. The
//! original package is kept in the backups folder and put back on restore.
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
/// Installed fonts for code points no game face has, in order of preference.
/// shortcut: Windows font folder only, so Linux builds get only the game's glyphs.
const SYSTEM_FONTS: [&str; 15] = [
    "segoeui.ttf",
    "seguisym.ttf",
    "seguihis.ttf",
    "gadugi.ttf",
    "javatext.ttf",
    "ebrima.ttf",
    "msyi.ttf",
    "LeelawUI.ttf",
    "Nirmala.ttc",
    "mmrtext.ttf",
    "monbaiti.ttf",
    "ntailu.ttf",
    "taile.ttf",
    "phagspa.ttf",
    "himalaya.ttf",
];
/// DefineFont3 glyphs use a 1024 unit EM in twips.
const EM: f32 = 1024.0 * 20.0;
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

/// MSB-first bit writer for SWF shape records.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    used: u32,
}

impl Bits {
    fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            if self.used % 8 == 0 {
                self.out.push(0);
            }
            if value >> i & 1 != 0 {
                *self.out.last_mut().unwrap() |= 0x80 >> (self.used % 8);
            }
            self.used += 1;
        }
    }

    fn signed(&mut self, value: i32, n: u32) {
        self.put((value as u32) & (u32::MAX >> (32 - n)), n);
    }
}

/// Bits a signed field needs to hold every value.
fn signed_bits(values: &[i32]) -> u32 {
    values.iter().map(|&v| 33 - if v < 0 { v.leading_ones() } else { v.leading_zeros() }).max().unwrap_or(1)
}

fn rect(x_min: i32, x_max: i32, y_min: i32, y_max: i32) -> Vec<u8> {
    let n = signed_bits(&[x_min, x_max, y_min, y_max]);
    let mut bits = Bits::default();
    bits.put(n, 5);
    for v in [x_min, x_max, y_min, y_max] {
        bits.signed(v, n);
    }
    bits.out
}

/// A glyph outline (font units, Y up) as a DefineFont3 SHAPE (twips, Y down).
fn shape(curves: &[ab_glyph::OutlineCurve], scale: f32) -> Vec<u8> {
    use ab_glyph::{OutlineCurve, Point, point};
    let at = |p: Point| ((p.x * scale).round() as i32, (-p.y * scale).round() as i32);
    let edge = |bits: &mut Bits, pen: &mut (i32, i32), control: Option<(i32, i32)>, to: (i32, i32)| {
        if let Some(c) = control {
            let d = [c.0 - pen.0, c.1 - pen.1, to.0 - c.0, to.1 - c.1];
            let n = signed_bits(&d).max(2);
            bits.put(0b10, 2);
            bits.put(n - 2, 4);
            d.iter().for_each(|&v| bits.signed(v, n));
        } else if *pen != to {
            let d = [to.0 - pen.0, to.1 - pen.1];
            let n = signed_bits(&d).max(2);
            // straight, general line
            bits.put(0b11, 2);
            bits.put(n - 2, 4);
            bits.put(1, 1);
            d.iter().for_each(|&v| bits.signed(v, n));
        }
        *pen = to;
    };
    let mut bits = Bits::default();
    // one fill bit, no line styles
    bits.put(1, 4);
    bits.put(0, 4);
    let mut pen: Option<(i32, i32)> = None;
    for curve in curves {
        let (OutlineCurve::Line(p0, _) | OutlineCurve::Quad(p0, ..) | OutlineCurve::Cubic(p0, ..)) = *curve;
        let start = at(p0);
        if pen != Some(start) {
            // style change record: move to, with fill style 0 on the first as the game's glyphs have
            let first = pen.is_none();
            bits.put(if first { 0b000011 } else { 0b000001 }, 6);
            let n = signed_bits(&[start.0, start.1]);
            bits.put(n, 5);
            bits.signed(start.0, n);
            bits.signed(start.1, n);
            if first {
                bits.put(1, 1);
            }
            pen = Some(start);
        }
        let pen = pen.as_mut().unwrap();
        match *curve {
            OutlineCurve::Line(_, p1) => edge(&mut bits, pen, None, at(p1)),
            OutlineCurve::Quad(_, p1, p2) => edge(&mut bits, pen, Some(at(p1)), at(p2)),
            OutlineCurve::Cubic(p0, p1, p2, p3) => {
                // split at t = 0.5 and fit one quad to each half
                let mid = |a: Point, b: Point| point((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                let (a, b, c) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
                let (d, e) = (mid(a, b), mid(b, c));
                let m = mid(d, e);
                let quad = |q0: Point, q1: Point, q2: Point, q3: Point| {
                    point((3.0 * (q1.x + q2.x) - q0.x - q3.x) / 4.0, (3.0 * (q1.y + q2.y) - q0.y - q3.y) / 4.0)
                };
                edge(&mut bits, pen, Some(at(quad(p0, a, d, m))), at(m));
                edge(&mut bits, pen, Some(at(quad(m, e, c, p3))), at(p3));
            }
        }
    }
    bits.put(0, 6);
    bits.out
}

/// A face holding `data`'s glyph for every BMP code point outside private use.
fn system_face(data: &[u8]) -> Option<Font> {
    use ab_glyph::Font as _;
    let font = ab_glyph::FontRef::try_from_slice_and_index(data, 0).ok()?;
    let scale = EM / font.units_per_em()?;
    let (mut codes, mut shapes, mut advances, mut bounds) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for code in (0x20..0xe000).chain(0xf900..0xfffe) {
        let Some(ch) = char::from_u32(code) else { continue };
        let id = font.glyph_id(ch);
        if id.0 == 0 {
            continue;
        }
        let (glyph, rect) = match font.outline(id) {
            // ab_glyph puts the top edge (font Y up) in min.y
            Some(o) => (
                shape(&o.curves, scale),
                rect(
                    (o.bounds.min.x * scale).round() as i32,
                    (o.bounds.max.x * scale).round() as i32,
                    (-o.bounds.min.y * scale).round() as i32,
                    (-o.bounds.max.y * scale).round() as i32,
                ),
            ),
            None => (shape(&[], scale), rect(0, 0, 0, 0)),
        };
        codes.push(code as u16);
        shapes.push(glyph);
        advances.push(((font.h_advance_unscaled(id) * scale).round().clamp(0.0, 32767.0) as i16).to_le_bytes());
        bounds.push(rect);
    }
    Some(Font {
        id: 0,
        flags: 0x80,
        language: 0,
        name: b"System\0".to_vec(),
        shapes,
        codes,
        layout: Some(Layout { ascent: [0; 2], descent: [0; 2], leading: [0; 2], advances, bounds, kerning: vec![0, 0] }),
    })
}

/// Faces from the installed fonts in `SYSTEM_FONTS`; missing or unreadable ones are skipped.
fn system_faces() -> Vec<Font> {
    let Some(dir) = std::env::var_os("WINDIR").map(|w| PathBuf::from(w).join("Fonts")) else { return Vec::new() };
    SYSTEM_FONTS.iter().filter_map(|name| system_face(&fs::read(dir.join(name)).ok()?)).collect()
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

/// The target movie with every text face given the donors' missing glyphs,
/// then the system faces' glyphs for code points no donor has.
fn merged(target: &[u8], donors: &[Vec<u8>], system: &[Font]) -> Result<Vec<u8>, String> {
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
            for face in system {
                added += font.merge(face);
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
    let swf = merged(&target, &donors, &system_faces())?;
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
        // calibration: the game's Arial A next to the same glyph read from arial.ttf
        let game = fonts(&target).into_iter().find(|f| f.name() == "Arial").unwrap();
        let i = game.codes.iter().position(|&c| c == 'A' as u16).unwrap();
        let ours = system_face(&fs::read(r"C:\Windows\Fonts\arial.ttf").unwrap()).unwrap();
        let j = ours.codes.iter().position(|&c| c == 'A' as u16).unwrap();
        for (f, k) in [(&game, i), (&ours, j)] {
            let layout = f.layout.as_ref().unwrap();
            println!("{} A advance={} bounds={:?}", f.name(), i16::from_le_bytes(layout.advances[k]), read_rect(&layout.bounds[k]));
            println!("  {:?}", decode(&f.shapes[k]));
        }
        let system = system_faces();
        for f in &system {
            println!("system glyphs={}", f.codes.len());
        }
        let out = merged(&target, &donors, &system).unwrap();
        assert!(is_merged(&out));
        for f in fonts(&out) {
            println!("merged {} glyphs={}", f.name(), f.codes.len());
        }
        println!("movie {} -> {} bytes", target.len(), out.len());
    }

    /// MSB-first bit reader, the inverse of `Bits`.
    struct Reader<'a> {
        b: &'a [u8],
        at: usize,
    }

    impl Reader<'_> {
        fn get(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, _| {
                let bit = self.b[self.at / 8] >> (7 - self.at % 8) & 1;
                self.at += 1;
                v << 1 | u32::from(bit)
            })
        }

        fn signed(&mut self, n: u32) -> i32 {
            let v = self.get(n);
            if n > 0 && v >> (n - 1) & 1 != 0 { (v | !(u32::MAX >> (32 - n))) as i32 } else { v as i32 }
        }
    }

    fn read_rect(b: &[u8]) -> [i32; 4] {
        let mut r = Reader { b, at: 0 };
        let n = r.get(5);
        [r.signed(n), r.signed(n), r.signed(n), r.signed(n)]
    }

    /// A SHAPE as absolute points: ('M', x, y), ('L', x, y), ('Q', x, y) per anchor, and
    /// ('F', fill style 0, fill style 1) where a style is set.
    fn decode(shape: &[u8]) -> Vec<(char, i32, i32)> {
        let mut r = Reader { b: shape, at: 0 };
        let (fill_bits, line_bits) = (r.get(4), r.get(4));
        let (mut x, mut y, mut out) = (0, 0, Vec::new());
        loop {
            if r.get(1) == 0 {
                let flags = r.get(5);
                if flags == 0 {
                    return out;
                }
                if flags & 1 != 0 {
                    let n = r.get(5);
                    (x, y) = (r.signed(n), r.signed(n));
                    out.push(('M', x, y));
                }
                let f0 = if flags & 2 != 0 { r.get(fill_bits) as i32 } else { -1 };
                let f1 = if flags & 4 != 0 { r.get(fill_bits) as i32 } else { -1 };
                if flags & 6 != 0 {
                    out.push(('F', f0, f1));
                }
                if flags & 8 != 0 {
                    r.get(line_bits);
                }
            } else if r.get(1) == 1 {
                let n = r.get(4) + 2;
                if r.get(1) == 1 {
                    x += r.signed(n);
                    y += r.signed(n);
                } else if r.get(1) == 1 {
                    y += r.signed(n);
                } else {
                    x += r.signed(n);
                }
                out.push(('L', x, y));
            } else {
                let n = r.get(4) + 2;
                x += r.signed(n);
                y += r.signed(n);
                x += r.signed(n);
                y += r.signed(n);
                out.push(('Q', x, y));
            }
        }
    }

    #[test]
    fn an_outline_converts_to_a_shape_that_decodes_to_the_same_points() {
        use ab_glyph::{OutlineCurve::*, point};
        // a square with a curved top, then a second contour; Y up, scale 2
        let curves = [
            Line(point(0.0, 0.0), point(100.0, 0.0)),
            Quad(point(100.0, 0.0), point(50.0, 150.0), point(0.0, 0.0)),
            Line(point(-3000.0, 10.0), point(-3000.0, -10.5)),
            Cubic(point(-3000.0, -10.5), point(0.0, 0.0), point(0.0, 0.0), point(-3000.0, 10.0)),
        ];
        assert_eq!(
            decode(&shape(&curves, 2.0)),
            vec![
                ('M', 0, 0),
                ('F', 1, -1),
                ('L', 200, 0),
                ('Q', 0, 0),
                ('M', -6000, -20),
                ('L', -6000, 21),
                ('Q', -1500, 0),
                ('Q', -6000, -20),
            ]
        );
        assert_eq!(read_rect(&rect(-5, 70000, 0, -1)), [-5, 70000, 0, -1]);
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
        let out = merged(&target, &[donor], &[]).unwrap();
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
