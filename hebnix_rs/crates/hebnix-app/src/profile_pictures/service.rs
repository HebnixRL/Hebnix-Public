//! Uses the same endpoint, platform aliases and avatar_url schema as
//! hebnix-sdk/src/tracker/client.rs, including its short-lived request token.
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
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const API: &str = "https://req.hebnix.com/player";
// Match the plugin HTTP client: the public image host rejects requests with
// no User-Agent before they reach the upload/file endpoints.
const AVATAR_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
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
pub(super) fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>, String> {
    normalize_size(bytes, 250)
}
fn normalize_size(bytes: &[u8], size: u32) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_IMAGE {
        return Err("Image exceeds 6 MB.".into());
    }
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
        .resize(size, size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut rgb = image::RgbImage::new(size, size);
    let left = (size - pixels.width()) / 2;
    let top = (size - pixels.height()) / 2;
    for (x, y, p) in pixels.enumerate_pixels() {
        let a = p[3] as u32;
        rgb.put_pixel(
            left + x,
            top + y,
            image::Rgb([0, 1, 2].map(|c| ((p[c] as u32 * a + 127) / 255) as u8)),
        );
    }
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
    pictures: Option<PathBuf>,
}
impl Resolver {
    fn new() -> Result<Self, String> {
        // Only the profile request receives the SDK token. CDN image requests
        // and public avatar uploads use separate requests without that header.
        let profile = Client::builder()
            .redirect(Policy::none())
            // the profile service rejects requests without a registered app User-Agent (HTTP 427)
            .user_agent(concat!("Hebnix/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let image = Client::builder()
            .redirect(Policy::limited(3))
            .user_agent(AVATAR_USER_AGENT)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            profile,
            image,
            cache: Mutex::new(HashMap::new()),
            pictures: None,
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
        if valid_upload_id(&id.account) {
            if let Some(root) = &self.pictures {
                if let Ok(bytes) =
                    std::fs::read(root.join(format!("{}-{}.jpg", id.platform, id.account)))
                {
                    return normalize_size(&bytes, 128);
                }
            }
        }
        if id.platform == "epic" {
            if valid_epic(&id.account) {
                if let Some(root) = &self.pictures {
                    if let Ok(bytes) = std::fs::read(root.join(format!("{}.jpg", id.account))) {
                        return normalize_size(&bytes, 128);
                    }
                }
            }
        }
        self.fetch_remote(id, API)
    }
    fn fetch_remote(&self, id: &Identity, api: &str) -> Result<Vec<u8>, String> {
        let token = hebnix_sdk::req_auth::app_token()?;
        self.fetch_remote_with_token(id, api, &token)
    }
    fn profile_request(
        &self,
        id: &Identity,
        api: &str,
        token: &str,
    ) -> reqwest::blocking::RequestBuilder {
        let url = format!("{api}/{}/{}", id.platform, id.account);
        self.profile
            .get(url)
            .header("Accept", "application/json")
            .header("X-App-Token", token)
    }
    fn fetch_remote_with_token(
        &self,
        id: &Identity,
        api: &str,
        token: &str,
    ) -> Result<Vec<u8>, String> {
        self.fetch_remote_size(id, api, token, 128)
    }
    fn fetch_remote_size(
        &self,
        id: &Identity,
        api: &str,
        token: &str,
        size: u32,
    ) -> Result<Vec<u8>, String> {
        let response = self
            .profile_request(id, api, token)
            .send()
            .map_err(|_| "Profile service unavailable")?;
        let profile: Value = serde_json::from_slice(&limited(response, MAX_PROFILE)?)
            .map_err(|_| "Invalid profile response")?;
        let avatar = avatar_url(&profile).ok_or("No avatar available")?;
        self.download(avatar, size)
    }
    fn download(&self, avatar: &str, size: u32) -> Result<Vec<u8>, String> {
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
        normalize_size(&limited(image, MAX_IMAGE)?, size)
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
        println!(
            "Rejected profile route: {}",
            path.chars().take(180).collect::<String>()
        );
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
pub(super) fn valid_epic(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn valid_upload_id(id: &str) -> bool {
    valid_epic(id) || ((10..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()))
}
pub(super) fn account_picture(platform: &str, id: &str) -> Result<Vec<u8>, String> {
    let identity = Identity::parse(&format!("{platform}|{id}|0"))?;
    Resolver::new()?.fetch_remote_size(&identity, API, &hebnix_sdk::req_auth::app_token()?, 250)
}
fn avatar_url(profile: &Value) -> Option<&str> {
    profile
        .get("avatar_url")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
}
pub(super) fn upload(account: &str, jpeg: Vec<u8>) -> Result<String, String> {
    upload_to(
        "https://pubapi.hebnix.com/rocket-profiles/upload",
        account,
        jpeg,
    )
}
fn upload_to(endpoint: &str, account: &str, jpeg: Vec<u8>) -> Result<String, String> {
    if !valid_upload_id(account) {
        return Err("Invalid platform account.".into());
    }
    let part = reqwest::blocking::multipart::Part::bytes(jpeg)
        .file_name("avatar.jpg")
        .mime_str("image/jpeg")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("platform_id", account.to_owned())
        .part("image", part);
    let client = Client::builder()
        .redirect(Policy::none())
        .user_agent(AVATAR_USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(endpoint)
        .multipart(form)
        .send()
        .map_err(|e| e.to_string())?;
    let body: Value =
        serde_json::from_slice(&limited(response, MAX_PROFILE)?).map_err(|e| e.to_string())?;
    body.get("image")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.replace("/rocket-profiles/files/", "/files/rocket-profiles/"))
        .ok_or_else(|| "Upload response did not contain an image URL.".into())
}

pub(super) struct Runtime {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    resolver: Arc<Resolver>,
}
impl Runtime {
    pub(super) fn start(pictures: &Path) -> Result<Self, String> {
        let mut resolver = Resolver::new()?;
        resolver.pictures = Some(pictures.to_owned());
        let resolver = Arc::new(resolver);
        let listener = TcpListener::bind("127.0.0.1:17895")
            .map_err(|e| format!("Cannot start profile picture service: {e}. Stop the old avatar test server if it is still running."))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_resolver = resolver.clone();
        let thread = thread::spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            while !thread_stop.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(40));
                        continue;
                    }
                    Err(_) => break,
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
                let resolver = thread_resolver.clone();
                let active = active.clone();
                thread::spawn(move || {
                    handle(stream, &resolver);
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
            resolver,
        })
    }
    pub(super) fn invalidate(&self) {
        if let Ok(mut cache) = self.resolver.cache.lock() {
            cache.clear();
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_platform_uses_the_profile_service_and_null_keeps_default() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}/player", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut paths = Vec::new();
            for _ in 0..5 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(!request.contains("user-agent:"));
                assert!(request.contains("x-app-token: test-token\r\n"));
                paths.push(request.lines().next().unwrap().to_string());
                reply(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    br#"{"avatar_url":null}"#,
                );
            }
            paths
        });
        let resolver = Resolver::new().unwrap();
        for primary in [
            "Epic|account|0",
            "Steam|account|0",
            "PS4|account|0",
            "XboxOne|account|0",
            "Switch|account|0",
        ] {
            assert_eq!(
                resolver
                    .fetch_remote_with_token(&Identity::parse(primary).unwrap(), &api, "test-token")
                    .unwrap_err(),
                "No avatar available"
            );
        }
        assert_eq!(
            server.join().unwrap(),
            ["epic", "steam", "psn", "xboxone", "switch"]
                .map(|p| format!("get /player/{p}/account http/1.1"))
        );
    }
    #[test]
    fn partial_alpha_is_blended_against_black() {
        let source = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 128]));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let jpeg = normalize_image(png.get_ref()).unwrap();
        let rgb = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        let pixel = rgb.get_pixel(64, 64);
        assert!((120..=136).contains(&pixel[0]));
        assert!(pixel[1] < 10 && pixel[2] < 10);
    }
    #[test]
    fn null_or_missing_avatar_keeps_the_default() {
        for profile in [
            serde_json::json!({}),
            serde_json::json!({"avatar_url": null}),
            serde_json::json!({"avatar_url": ""}),
            serde_json::json!({"avatar_url": " null "}),
        ] {
            assert_eq!(avatar_url(&profile), None);
        }
        let profile = serde_json::json!({"avatar_url": "https://example.com/avatar.png"});
        assert_eq!(avatar_url(&profile), Some("https://example.com/avatar.png"));
    }
    #[test]
    fn imported_picture_resolves_without_network() {
        let root = std::env::temp_dir().join(format!(
            "hebnix-avatar-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let id = Identity::parse("Epic|0123456789abcdef0123456789abcdef|0").unwrap();
        let source = image::RgbImage::from_pixel(3, 7, image::Rgb([0, 200, 255]));
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        std::fs::write(root.join(format!("{}.jpg", id.account)), png.get_ref()).unwrap();
        let mut resolver = Resolver::new().unwrap();
        resolver.profile = Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .build()
            .unwrap();
        resolver.pictures = Some(root.clone());
        let result = resolver.fetch(&id).unwrap();
        assert!(result.starts_with(&[0xff, 0xd8]));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn upload_matches_plugin_multipart_contract() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/upload", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .and_then(|n| n.parse().ok())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = br#"{"image":"https://example.com/me.jpg"}"#;
            reply(&mut stream, "201 Created", "application/json", body);
            String::from_utf8(request).unwrap()
        });
        let owner = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            upload_to(&endpoint, owner, b"test-image-bytes".to_vec()).unwrap(),
            "https://example.com/me.jpg"
        );
        let request = server.join().unwrap();
        assert!(request.contains("name=\"platform_id\"\r\n\r\n0123456789abcdef0123456789abcdef"));
        assert!(request.contains("name=\"image\"; filename=\"avatar.jpg\""));
        assert!(request.contains("test-image-bytes"));
        let headers = request
            .split("\r\n\r\n")
            .next()
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.contains(&format!(
            "user-agent: {}",
            AVATAR_USER_AGENT.to_ascii_lowercase()
        )));
        assert!(!headers.contains("x-app-token:"));
        assert!(!headers.contains("authorization:"));
    }
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
        assert_eq!((decoded.width(), decoded.height()), (250, 250));
        assert!(
            decoded
                .to_rgb8()
                .get_pixel(64, 64)
                .0
                .iter()
                .all(|v| *v < 10)
        );
    }
    #[test]
    fn request_key_is_sent_only_to_the_profile_service() {
        let resolver = Resolver::new().unwrap();
        let id = Identity::parse("Epic|example|0").unwrap();
        let request = resolver
            .profile_request(&id, API, "test-token")
            .build()
            .unwrap();
        assert_eq!(request.headers()["X-App-Token"], "test-token");
        assert!(!request.headers().contains_key("User-Agent"));
        let image = resolver
            .image
            .get("https://example.com/avatar.jpg")
            .build()
            .unwrap();
        assert!(!image.headers().contains_key("X-App-Token"));
    }
    #[test]
    fn image_downloads_send_plugin_agent_without_the_request_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/picture.jpg", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            reply(&mut stream, "200 OK", "image/jpeg", b"test");
            String::from_utf8(request).unwrap().to_ascii_lowercase()
        });
        Resolver::new().unwrap().image.get(url).send().unwrap();
        let request = server.join().unwrap();
        assert!(request.contains(&format!(
            "user-agent: {}",
            AVATAR_USER_AGENT.to_ascii_lowercase()
        )));
        assert!(!request.contains("x-app-token:"));
    }
}
