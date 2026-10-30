//! Uses the same endpoint, platform aliases and avatar_url schema as
//! hebnix-sdk/src/tracker/client.rs, without credentials or a User-Agent.
use image::{ImageReader, codecs::jpeg::JpegEncoder};
use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const API: &str = "https://req.hebnix.com/player";
const MAX_PROFILE: usize = 2 * 1024 * 1024;
const MAX_IMAGE: usize = 6 * 1024 * 1024;
const CACHE_LIMIT: usize = 512;
type Cached = (Instant, Duration, Result<Arc<Vec<u8>>, String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub platform: String,
    pub account: String,
}
impl Identity {
    fn parse(primary: &str) -> Result<Self, String> {
        let parts: Vec<_> = primary.split('|').collect();
        if parts.len() != 3 || parts[2].parse::<u8>().is_err() {
            return Err("Invalid PrimaryId".into());
        }
        let platform = match parts[0].to_ascii_lowercase().as_str() {
            "epic" | "epicgames" => "epic",
            "steam" => "steam",
            "psn" | "ps4" | "ps5" | "playstation" => "psn",
            "xbl" | "xbox" | "xboxone" => "xboxone",
            "switch" | "nintendo" => "switch",
            _ => return Err("Unsupported platform or bot".into()),
        };
        if parts[1].is_empty()
            || parts[1].len() > 128
            || !parts[1]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.~".contains(&b))
        {
            return Err("Invalid account identifier".into());
        }
        Ok(Self {
            platform: platform.into(),
            account: parts[1].into(),
        })
    }
    fn key(&self) -> String {
        format!("{}:{}", self.platform, self.account)
    }
}

fn limited(mut response: Response, max: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err("Response exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Response read failed")?;
    if bytes.len() > max {
        return Err("Response exceeds size limit".into());
    }
    Ok(bytes)
}
fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unknown image format")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "Unsupported or oversized image")?;
    let pixels = decoded
        .resize_to_fill(128, 128, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let rgb = image::RgbImage::from_fn(128, 128, |x, y| {
        let p = pixels.get_pixel(x, y);
        let a = p[3] as u32;
        image::Rgb([0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8))
    });
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 88)
        .encode_image(&rgb)
        .map_err(|_| "JPEG encoding failed")?;
    Ok(jpeg)
}

struct Resolver {
    profile: Client,
    image: Client,
    cache: Mutex<HashMap<String, Cached>>,
}
impl Resolver {
    fn new() -> Result<Self, String> {
        // reqwest sends no User-Agent unless explicitly configured. No token
        // headers, cookies, account secrets or SDK build-key files are used.
        let profile = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let image = Client::builder()
            .redirect(Policy::limited(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            profile,
            image,
            cache: Mutex::new(HashMap::new()),
        })
    }
    fn fetch(&self, id: &Identity) -> Result<Arc<Vec<u8>>, String> {
        let key = id.key();
        if let Some((at, ttl, value)) = self
            .cache
            .lock()
            .map_err(|_| "Cache unavailable")?
            .get(&key)
        {
            if at.elapsed() < *ttl {
                return value.clone();
            }
        }
        let result = self.fetch_uncached(id).map(Arc::new);
        let ttl = if result.is_ok() {
            300
        } else if result
            .as_ref()
            .err()
            .is_some_and(|e| e == "No avatar available")
        {
            60
        } else {
            20
        };
        let mut cache = self.cache.lock().map_err(|_| "Cache unavailable")?;
        cache.retain(|_, (at, ttl, _)| at.elapsed() < *ttl);
        if cache.len() >= CACHE_LIMIT {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, v)| v.0)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            key,
            (Instant::now(), Duration::from_secs(ttl), result.clone()),
        );
        result
    }
    fn fetch_uncached(&self, id: &Identity) -> Result<Vec<u8>, String> {
        let url = format!("{API}/{}/{}", id.platform, id.account);
        let response = self
            .profile
            .get(url)
            .header("Accept", "application/json")
            .send()
            .map_err(|_| "Profile service unavailable")?;
        let profile: Value = serde_json::from_slice(&limited(response, MAX_PROFILE)?)
            .map_err(|_| "Invalid profile response")?;
        let avatar = profile
            .get("avatar_url")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or("No avatar available")?;
        let parsed = reqwest::Url::parse(avatar).map_err(|_| "Invalid avatar URL")?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("Avatar URL must use HTTPS".into());
        }
        let image = self
            .image
            .get(parsed)
            .send()
            .map_err(|_| "Avatar download failed")?;
        normalize_image(&limited(image, MAX_IMAGE)?)
    }
}

fn decode_path(path: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut at = 0;
    let raw = path.as_bytes();
    while at < raw.len() {
        if raw[at] == b'%' {
            let hex = raw.get(at + 1..at + 3).ok_or("Invalid URL encoding")?;
            let text = std::str::from_utf8(hex).map_err(|_| "Invalid URL encoding")?;
            bytes.push(u8::from_str_radix(text, 16).map_err(|_| "Invalid URL encoding")?);
            at += 3;
        } else {
            bytes.push(raw[at]);
            at += 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| "Invalid URL encoding".into())
}
fn reply(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body));
}
fn handle(mut stream: TcpStream, resolver: &Resolver) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    while bytes.len() < 8192 && !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
        }
    }
    let header = String::from_utf8_lossy(&bytes);
    let mut request = header.lines().next().unwrap_or("").split_whitespace();
    let method = request.next().unwrap_or("");
    let path = request.next().unwrap_or("");
    if method != "GET" {
        reply(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"GET required",
        );
        return;
    }
    if path == "/health" {
        reply(
            &mut stream,
            "200 OK",
            "application/json",
            b"{\"status\":\"ok\",\"provider\":\"req.hebnix.com\"}",
        );
        return;
    }
    let id = path
        .strip_prefix("/profile/")
        .ok_or_else(|| "Unknown route".to_string())
        .and_then(decode_path)
        .and_then(|v| Identity::parse(&v));
    let Ok(id) = id else {
        println!("Rejected profile route: {}",path.chars().take(180).collect::<String>());
        reply(
            &mut stream,
            "404 Not Found",
            "text/plain",
            b"Unknown player",
        );
        return;
    };
    match resolver.fetch(&id) {
        Ok(jpeg) => {
            println!("avatar {}: ready ({} bytes)", id.key(), jpeg.len());
            reply(&mut stream, "200 OK", "image/jpeg", &jpeg);
        }
        Err(error) => {
            println!("avatar {}: {error}", id.key());
            reply(
                &mut stream,
                "404 Not Found",
                "text/plain",
                b"No avatar available",
            );
        }
    }
}
pub fn serve() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:17895").map_err(|e| e.to_string())?;
    let resolver = Arc::new(Resolver::new()?);
    let active = Arc::new(AtomicUsize::new(0));
    println!(
        "Profile avatar bridge listening on 127.0.0.1:17895; provider={API}; no authentication or custom User-Agent"
    );
    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else {
            continue;
        };
        if active.fetch_add(1, Ordering::SeqCst) >= 8 {
            active.fetch_sub(1, Ordering::SeqCst);
            reply(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                b"Busy",
            );
            continue;
        }
        let resolver = resolver.clone();
        let active = active.clone();
        thread::spawn(move || {
            handle(stream, &resolver);
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}
pub fn probe(primary: &str) -> Result<(), String> {
    let id = Identity::parse(primary)?;
    match Resolver::new()?.fetch(&id) {
        Ok(jpeg) => println!(
            "Profile resolved: {} (normalized JPEG: {} bytes)",
            id.key(),
            jpeg.len()
        ),
        Err(e) => println!("Profile resolved: {} ({e})", id.key()),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_match_sdk_platform_aliases() {
        assert_eq!(Identity::parse("Xbox|1234|0").unwrap().platform, "xboxone");
        assert_eq!(Identity::parse("PS5|example|0").unwrap().platform, "psn");
        assert_eq!(Identity::parse("Epic|abcd|1").unwrap().key(), "epic:abcd");
    }
    #[test]
    fn rejects_bots_and_path_injection() {
        for id in [
            "Unknown|0|0",
            "Epic|../secret|0",
            "Steam|123|0/other",
            "Epic||0",
            "Epic|ab|0|extra",
        ] {
            assert!(Identity::parse(id).is_err(), "{id}");
        }
    }
    #[test]
    fn accepts_url_encoded_primary_ids() {
        assert_eq!(decode_path("Epic%7Cabc%7C0").unwrap(), "Epic|abc|0");
        assert!(decode_path("Epic%XXabc").is_err());
    }
    #[test]
    fn normalizes_alpha_and_dimensions() {
        let source = image::RgbaImage::from_pixel(17, 11, image::Rgba([0, 0, 0, 0]));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let jpeg = normalize_image(png.get_ref()).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (128, 128));
        assert!(decoded.to_rgb8().get_pixel(64, 64)[0] > 245);
    }
    #[test]
    fn requests_have_no_token_or_user_agent() {
        let resolver = Resolver::new().unwrap();
        let request = resolver
            .profile
            .get("https://req.hebnix.com/player/epic/test")
            .header("Accept", "application/json")
            .build()
            .unwrap();
        assert!(!request.headers().contains_key("X-App-Token"));
        assert!(!request.headers().contains_key("User-Agent"));
    }
}
