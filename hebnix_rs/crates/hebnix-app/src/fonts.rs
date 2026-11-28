//! Theme fonts: finding one on this PC, and fetching a missing one once the user agrees.
//! Downloads only ever end up as a single validated .ttf/.otf in Hebnix's fonts folder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

/// Download cap, and the cap on each font unpacked from a zip.
const MAX_DOWNLOAD: u64 = 20 * 1024 * 1024;
/// Cap on everything unpacked from one zip.
const MAX_UNPACKED: u64 = 60 * 1024 * 1024;

/// Where a theme's font can be downloaded from when it isn't installed.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct FontSource {
    /// Google Fonts family name, e.g. "Lexend".
    pub google: Option<String>,
    /// Fontshare slug, e.g. "satoshi".
    pub fontshare: Option<String>,
    /// Font Squirrel slug, e.g. "open-sans".
    pub fontsquirrel: Option<String>,
    /// Any https link to a .ttf, .otf or a .zip holding them.
    pub url: Option<String>,
    /// Optional SHA-256 of the file at `url`; a mismatch saves nothing.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    Google(String),
    Fontshare(String),
    FontSquirrel(String),
    Url { url: String, sha256: Option<String> },
}

impl FontSource {
    /// The places to try, named services first.
    pub fn fetches(&self) -> Vec<Fetch> {
        let mut fetches = Vec::new();
        fetches.extend(self.google.clone().map(Fetch::Google));
        fetches.extend(self.fontshare.clone().map(Fetch::Fontshare));
        fetches.extend(self.fontsquirrel.clone().map(Fetch::FontSquirrel));
        fetches.extend(self.url.clone().map(|url| Fetch::Url { url, sha256: self.sha256.clone() }));
        fetches
    }
}

impl Fetch {
    /// Where the download comes from, for the approval prompt.
    pub fn describe(&self) -> String {
        match self {
            Fetch::Google(_) => "Google Fonts".into(),
            Fetch::Fontshare(_) => "Fontshare".into(),
            Fetch::FontSquirrel(_) => "Font Squirrel".into(),
            Fetch::Url { url, .. } => url::Url::parse(url)
                .ok()
                .and_then(|url| {
                    let file = url.path_segments().and_then(|mut parts| parts.next_back().map(str::to_owned)).unwrap_or_default();
                    url.host_str().map(|host| if file.is_empty() { host.to_owned() } else { format!("{host} ({file})") })
                })
                .unwrap_or_else(|| url.clone()),
        }
    }

    /// Key for remembering that the user said no to this download.
    pub fn key(&self) -> String {
        match self {
            Fetch::Google(name) => format!("google:{name}"),
            Fetch::Fontshare(slug) => format!("fontshare:{slug}"),
            Fetch::FontSquirrel(slug) => format!("fontsquirrel:{slug}"),
            Fetch::Url { url, sha256 } => format!("url:{url}#{}", sha256.as_deref().unwrap_or_default()),
        }
    }
}

fn system() -> &'static fontdb::Database {
    static SYSTEM: OnceLock<fontdb::Database> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Family names of the fonts installed on this PC, sorted. Scanned once per run.
pub fn system_families() -> &'static [String] {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut names: Vec<String> = system().faces().filter_map(|face| face.families.first().map(|(name, _)| name.clone())).collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        names
    })
}

/// Picks the face closest to regular: upright, weight 400, from `family` when it has one.
fn regular_face<'a>(faces: impl Iterator<Item = &'a fontdb::FaceInfo>, family: &str) -> Option<&'a fontdb::FaceInfo> {
    faces.min_by_key(|face| {
        let named = face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family));
        let italic = face.style != fontdb::Style::Normal;
        (u32::from(!named) * 100_000) + (u32::from(italic) * 10_000) + u32::from(face.weight.0.abs_diff(400))
    })
}

fn system_face(name: &str) -> Option<(Vec<u8>, u32)> {
    let db = system();
    let face = regular_face(db.faces().filter(|face| face.families.iter().any(|(family, _)| family.eq_ignore_ascii_case(name))), name)?;
    db.with_face_data(face.id, |data, index| (data.to_vec(), index))
}

/// A theme font's bytes and face index: Hebnix's fonts folder first, then fonts installed on this PC.
pub fn resolve(fonts_dir: &Path, name: &str) -> Option<(Vec<u8>, u32)> {
    if let Some(path) = crate::theme::find_font_file(fonts_dir, name) {
        return std::fs::read(path).ok().map(|bytes| (bytes, 0));
    }
    system_face(name)
}

pub fn installed(fonts_dir: &Path, name: &str) -> bool {
    crate::theme::find_font_file(fonts_dir, name).is_some()
        || system_families().iter().any(|family| family.eq_ignore_ascii_case(name))
}

/// The family name as a file name; anything that could leave the fonts folder is refused.
fn file_stem(family: &str) -> Result<&str, String> {
    let family = family.trim();
    let ok = !family.is_empty()
        && family.len() <= 64
        && family.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'));
    if ok { Ok(family) } else { Err(format!("\"{family}\" is not a usable font name.")) }
}

fn title_case(name: &str) -> String {
    name.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slug(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "-")
}

/// The first font file link in a Google Fonts stylesheet.
fn google_font_url(css: &str) -> Option<&str> {
    let start = css.find("url(")? + 4;
    let end = start + css[start..].find(')')?;
    Some(css[start..end].trim_matches(|c| c == '\'' || c == '"'))
}

fn read_capped(response: ureq::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err("The download is larger than 20 MB.".into());
    }
    Ok(bytes)
}

/// Font files in a zip, flattened to their file names. Mac resource forks and anything
/// that isn't .ttf/.otf are skipped.
fn zip_fonts(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("The zip could not be opened: {e}"))?;
    let mut fonts = Vec::new();
    let mut unpacked = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(path) = entry.enclosed_name() else { continue };
        let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_owned) else { continue };
        let font = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"));
        if entry.is_dir() || !font || name.starts_with("._") || path.components().any(|part| part.as_os_str() == std::ffi::OsStr::new("__MACOSX")) || entry.size() > MAX_DOWNLOAD {
            continue;
        }
        let mut contents = Vec::new();
        entry.by_ref().take(MAX_DOWNLOAD + 1).read_to_end(&mut contents).map_err(|e| e.to_string())?;
        unpacked += contents.len() as u64;
        if contents.len() as u64 > MAX_DOWNLOAD || unpacked > MAX_UNPACKED {
            return Err("The zip unpacks to more than Hebnix allows for fonts.".into());
        }
        fonts.push((name, contents));
    }
    Ok(fonts)
}

/// Keeps the regular face out of the downloaded files, checked by parsing it, as `{family}.ttf|otf`.
fn install(bytes: Vec<u8>, family: &str, fonts_dir: &Path) -> Result<PathBuf, String> {
    let stem = file_stem(family)?;
    let files = if bytes.starts_with(b"PK\x03\x04") { zip_fonts(&bytes)? } else { vec![(String::new(), bytes)] };
    let mut db = fontdb::Database::new();
    for (_, contents) in files {
        // faces that don't parse are left out, which is the validation
        db.load_font_data(contents);
    }
    let face = regular_face(db.faces(), family).ok_or("The download has no usable .ttf or .otf font.")?;
    let data = db.with_face_data(face.id, |data, _| data.to_vec()).ok_or("The font could not be read.")?;
    let ext = if data.starts_with(b"OTTO") { "otf" } else { "ttf" };
    std::fs::create_dir_all(fonts_dir).map_err(|e| format!("Could not create the fonts folder: {e}"))?;
    let path = fonts_dir.join(format!("{stem}.{ext}"));
    let partial = fonts_dir.join(format!("{stem}.{ext}.part"));
    std::fs::write(&partial, data).map_err(|e| format!("Could not save the font: {e}"))?;
    std::fs::rename(&partial, &path).map_err(|e| format!("Could not save the font: {e}"))?;
    Ok(path)
}

/// Downloads `family` and installs it into the fonts folder. Blocking; run it off the UI thread.
/// Returns the host the file finally came from.
pub fn download(fetch: &Fetch, family: &str, fonts_dir: &Path) -> Result<String, String> {
    file_stem(family)?;
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .https_only(true)
        .redirects(5)
        .timeout(Duration::from_secs(30))
        .build();
    let get = |url: &str| agent.get(url).call().map_err(|error| match error {
        ureq::Error::Status(code, _) => format!("The server answered {code}."),
        other => other.to_string(),
    });
    let response = match fetch {
        Fetch::Google(name) => {
            // Google family names are case-sensitive, so "inter" retries as "Inter"
            let css = [name.trim().to_owned(), title_case(name)]
                .iter()
                .find_map(|family| get(&format!("https://fonts.googleapis.com/css2?family={}", family.replace(' ', "+"))).ok())
                .ok_or_else(|| format!("{name} isn't on Google Fonts."))?
                .into_string()
                .map_err(|e| e.to_string())?;
            let url = google_font_url(&css).ok_or("Google Fonts sent no font link.")?;
            get(url)?
        }
        Fetch::Fontshare(slug_name) => get(&format!("https://api.fontshare.com/v2/fonts/download/{}", slug(slug_name)))?,
        Fetch::FontSquirrel(slug_name) => get(&format!("https://www.fontsquirrel.com/fonts/download/{}", slug(slug_name)))?,
        Fetch::Url { url, .. } => get(url)?,
    };
    let host = url::Url::parse(response.get_url()).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
    let bytes = read_capped(response)?;
    if let Fetch::Url { sha256: Some(expected), .. } = fetch {
        use sha2::Digest;
        let actual: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err("The downloaded file doesn't match the theme's sha256, so nothing was saved.".into());
        }
    }
    install(bytes, family, fonts_dir)?;
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_css_link_and_names() {
        let css = "@font-face {\n  src: url(https://fonts.gstatic.com/s/lexend/v26/abc.ttf) format('truetype');\n}";
        assert_eq!(google_font_url(css), Some("https://fonts.gstatic.com/s/lexend/v26/abc.ttf"));
        assert_eq!(slug("Open Sans"), "open-sans");
        assert_eq!(title_case("open  sans"), "Open Sans");
        assert!(file_stem("Open Sans").is_ok());
        assert!(file_stem("../evil").is_err() && file_stem("a\\b").is_err() && file_stem(" ").is_err());
    }

    #[test]
    fn zips_keep_only_font_files_flattened() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [("Fam/OTF/Fam-Regular.otf", "a"), ("__MACOSX/Fam/._Fam-Regular.otf", "b"), ("Fam/readme.txt", "c"), ("Fam/WEB/Fam.woff2", "d")] {
            zip.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut zip, body.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let fonts = zip_fonts(&bytes).unwrap();
        assert_eq!(fonts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Fam-Regular.otf"]);
    }

    #[test]
    fn install_keeps_the_regular_face_and_refuses_junk() {
        let dir = std::env::temp_dir().join(format!("hebnix-fonts-{}", std::process::id()));
        let regular = include_bytes!("../../../examples/fonts/Barlow-Medium.ttf").to_vec();
        let path = install(regular, "Barlow", &dir).unwrap();
        assert_eq!(path, dir.join("Barlow.ttf"));
        assert!(install(b"not a font".to_vec(), "Junk", &dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn source_order_and_prompt_text() {
        let source = FontSource { url: Some("https://github.com/x/y/releases/download/v1/Fam.zip".into()), google: Some("Fam".into()), ..Default::default() };
        let fetches = source.fetches();
        assert_eq!(fetches[0], Fetch::Google("Fam".into()));
        assert_eq!(fetches[1].describe(), "github.com (Fam.zip)");
    }
}
