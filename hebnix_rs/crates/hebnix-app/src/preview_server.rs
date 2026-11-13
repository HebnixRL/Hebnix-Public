//! Serves small PNG thumbnails of catalogue banners on localhost for the game's Workshop
//! menu. The game downloads every row's PreviewURL itself when the menu opens, and the raw
//! banners (1080p, progressive JPEGs) were too much for it. Thumbnails are made
//! on the first request (or ahead of time, see `warm`) and kept in the cache folder.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::Duration;

pub const PORT: u16 = 47853;
// the grid shows 16:9 previews about this size, and the engine keeps every row's texture in memory
const WIDTH: u32 = 224;
const HEIGHT: u32 = 126;
const MAX_BANNER_BYTES: u64 = 12 * 1024 * 1024;

/// the address the game downloads a map's preview from
pub fn url(id: &str) -> String {
    format!("http://127.0.0.1:{PORT}/p/{id}.png")
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// one banner is fetched and shrunk at a time, so a menu opening on a cold cache is not a stampede
static MAKING: Mutex<()> = Mutex::new(());

/// Start serving, once per run. A busy port only costs the previews.
pub fn start(cache_dir: &Path) {
    static STARTED: Once = Once::new();
    let dir = cache_dir.join("thumbs");
    STARTED.call_once(move || {
        let _ = std::fs::create_dir_all(&dir);
        let _ = DIR.set(dir.clone());
        warm();
        let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) else {
            tracing::warn!("preview server: port {PORT} is busy");
            return;
        };
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &dir);
                });
            }
        });
    });
}

/// Make every catalogue thumbnail in the background so the menu finds them cached.
pub fn warm() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let Some(dir) = DIR.get().cloned() else { return };
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        for id in crate::patcher::workshop_menu::downloadable_ids() {
            let _ = thumbnail(&dir, &id);
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

fn serve(mut stream: TcpStream, dir: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut head = [0u8; 2048];
    let n = stream.read(&mut head)?;
    let request = String::from_utf8_lossy(&head[..n]);
    let bytes = {
        request
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET /p/"))
            .and_then(|rest| rest.split(".png").next())
            .filter(|id| !id.is_empty() && id.len() < 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .and_then(|id| thumbnail(dir, id))
    };
    match bytes {
        Some(bytes) => {
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len())?;
            stream.write_all(&bytes)
        }
        None => stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}

fn thumbnail(dir: &Path, id: &str) -> Option<Vec<u8>> {
    let path: PathBuf = dir.join(format!("{id}.{WIDTH}x{HEIGHT}.png"));
    if let Ok(bytes) = std::fs::read(&path) {
        return Some(bytes);
    }
    let _making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(bytes) = std::fs::read(&path) {
        return Some(bytes);
    }
    let banner = crate::patcher::workshop_menu::downloadable_preview(id)?;
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(15))
        .build()
        .get(&banner)
        .call()
        .ok()?;
    let mut raw = Vec::new();
    response.into_reader().take(MAX_BANNER_BYTES).read_to_end(&mut raw).ok()?;
    let bytes = letterboxed(&raw)?;
    let _ = std::fs::write(&path, &bytes);
    Some(bytes)
}

/// the image fitted into a 16:9 frame whose spare edges are transparent, so the card's own
/// colour shows there, as a PNG
fn letterboxed(raw: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(raw).ok()?;
    let fitted = image.thumbnail(WIDTH, HEIGHT).to_rgba8();
    let mut canvas = image::RgbaImage::new(WIDTH, HEIGHT);
    let (x, y) = ((WIDTH - fitted.width()) / 2, (HEIGHT - fitted.height()) / 2);
    image::imageops::overlay(&mut canvas, &fitted, i64::from(x), i64::from(y));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_is_16_9_with_transparent_bars() {
        let mut source = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(400, 100))
            .write_to(&mut std::io::Cursor::new(&mut source), image::ImageFormat::Png)
            .unwrap();
        let out = letterboxed(&source).unwrap();
        let decoded = image::load_from_memory(&out).unwrap().to_rgba8();
        assert_eq!((decoded.width(), decoded.height()), (WIDTH, HEIGHT));
        // a 4:1 image leaves bars above and below, the middle row is the picture
        assert_eq!(decoded.get_pixel(WIDTH / 2, 0)[3], 0);
        assert_eq!(decoded.get_pixel(WIDTH / 2, HEIGHT / 2)[3], 255);
    }
}
