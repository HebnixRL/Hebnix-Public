//! downloads a rocket league workshop item straight from steam's CDN.
//!
//! the hebnix workshop api (see docs/workshop-api.md) hands out the item's
//! depot manifest and the depot key -- it gets them with a steam account that
//! owns rocket league. everything after that needs no login: the CDN server
//! list comes from steam's public web api, and the chunks are fetched,
//! decrypted and put back together here by steamroom.

use std::io::Read;
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use steamroom::cdn::{CdnClient, CdnServer, CdnServerPool};
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::{DepotId, DepotKey};
use steamroom_client::download::{CdnChunkFetcher, DepotJob};
use steamroom_client::event::DownloadEvent;
use steamroom_client::manifest::parse_cdn_manifest;

const WORKSHOP_API_URL: &str = "https://req.hebnix.com/workshop/v1";
const CDN_SERVERS_URL: &str = "https://api.steampowered.com/IContentServerDirectoryService/GetServersForSteamPipe/v1/?cell_id=0&max_servers=20";
const RL_APPID: u32 = 252950;
const MAX_REPLY_BYTES: u64 = 16 * 1024 * 1024;
/// the biggest workshop item hebnix will download; rocket league maps are
/// well under this
pub const MAX_ITEM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_CONCURRENT_CHUNKS: usize = 8;

/// the workshop api's base url (`HEBNIX_WORKSHOP_API_URL` overrides it, for
/// testing against a local server)
pub fn api_url() -> String {
    std::env::var("HEBNIX_WORKSHOP_API_URL")
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| WORKSHOP_API_URL.to_string())
}

/// a GET to the workshop api on req.hebnix.com, signed with the app token like
/// every other req.hebnix.com call
pub(super) fn api_get(path: &str) -> Result<Value, String> {
    let token = hebnix_sdk::req_auth::app_token()?;
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let result = agent
        .get(&format!("{}{path}", api_url()))
        .set("X-App-Token", &token)
        .set("Accept", "application/json")
        .set("User-Agent", concat!("Hebnix/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .call();
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body = read_json(response).unwrap_or_default();
            hebnix_sdk::req_auth::observe_response(status, &body);
            return Err(status_message(status, &body));
        }
        Err(other) => return Err(format!("Couldn't reach the Hebnix workshop service: {other}")),
    };
    read_json(response).map_err(|e| format!("The workshop service sent back bad data: {e}"))
}

fn read_json(response: ureq::Response) -> Result<Value, String> {
    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_REPLY_BYTES)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// what to tell the player when the workshop api says no. the server's own
/// `error` text wins when it sends one.
fn status_message(status: u16, body: &Value) -> String {
    if let Some(error) = body["error"].as_str().filter(|e| !e.trim().is_empty()) {
        return format!("The Hebnix workshop service said: {}", error.trim());
    }
    match status {
        404 => "The Hebnix workshop service doesn't know that item.".to_string(),
        429 => "The Hebnix workshop service is busy. Wait a minute and try again.".to_string(),
        498 => "Hebnix's app token was refused. Check that your PC's clock is right.".to_string(),
        code => format!("The Hebnix workshop service failed (HTTP {code})."),
    }
}

#[derive(Deserialize)]
struct ManifestReply {
    app_id: u32,
    depot_id: u32,
    manifest_id: String,
    depot_key: String,
    /// the raw manifest exactly as steam's CDN serves it, base64
    manifest: String,
}

/// a workshop item's manifest, decrypted and checked, plus the key its
/// chunks are encrypted with
pub struct ItemManifest {
    pub manifest: DepotManifest,
    pub depot_key: DepotKey,
    pub total_bytes: u64,
}

/// reads the workshop api's manifest reply. nothing in it is trusted: the ids
/// must be rocket league's, and every file must stay inside the download
/// folder.
pub fn parse_manifest_reply(reply: &Value) -> Result<ItemManifest, String> {
    let reply: ManifestReply = serde_json::from_value(reply.clone())
        .map_err(|e| format!("The manifest reply was malformed: {e}"))?;
    if reply.app_id != RL_APPID || reply.depot_id != RL_APPID {
        return Err("The manifest isn't for Rocket League.".to_string());
    }
    let manifest_id: u64 = reply
        .manifest_id
        .trim()
        .parse()
        .map_err(|_| "The manifest reply had no valid manifest id.")?;
    let key: [u8; 32] = hex::decode(reply.depot_key.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("The manifest reply had no valid depot key.")?;
    let depot_key = DepotKey(key);
    let raw = base64::engine::general_purpose::STANDARD
        .decode(reply.manifest.trim())
        .map_err(|_| "The manifest reply had no valid manifest.")?;
    let mut manifest =
        parse_cdn_manifest(&raw).map_err(|e| format!("The manifest couldn't be read: {e}"))?;
    if manifest.filenames_encrypted {
        manifest
            .decrypt_filenames(&depot_key)
            .map_err(|e| format!("The manifest couldn't be decrypted: {e}"))?;
    }
    if manifest.manifest_id.is_some_and(|id| id.0 != manifest_id) {
        return Err("The manifest doesn't match the id it was sent with.".to_string());
    }
    let total_bytes = check_files(&manifest)?;
    Ok(ItemManifest {
        manifest,
        depot_key,
        total_bytes,
    })
}

/// a manifest file name is only written if it's a plain relative path
pub fn is_safe_path(name: &str) -> bool {
    let path = name.replace('\\', "/");
    !path.is_empty()
        && !path.contains(':')
        && Path::new(&path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// refuses links, paths that would land outside the download folder, and
/// items too big to be a map. returns the item's total size.
fn check_files(manifest: &DepotManifest) -> Result<u64, String> {
    let mut total: u64 = 0;
    for file in &manifest.files {
        if file.link_target.is_some() {
            return Err(format!("The item has a link in it ({}).", file.filename));
        }
        if !is_safe_path(&file.filename) {
            return Err(format!("The item has an unsafe file path ({}).", file.filename));
        }
        total = total.saturating_add(file.size);
    }
    if total > MAX_ITEM_BYTES {
        return Err("The item is too big to be a Rocket League map.".to_string());
    }
    if manifest.files.is_empty() {
        return Err("The item has no files in it.".to_string());
    }
    Ok(total)
}

pub fn fetch_manifest(wid: &str) -> Result<ItemManifest, String> {
    parse_manifest_reply(&api_get(&format!("/manifest/{wid}"))?)
}

/// steam's CDN servers, from its public web api (no login)
pub fn parse_cdn_servers(reply: &Value) -> Vec<CdnServer> {
    reply
        .pointer("/response/servers")
        .and_then(Value::as_array)
        .map(|servers| {
            servers
                .iter()
                .filter(|s| matches!(s["type"].as_str(), Some("CDN" | "SteamCache")))
                .filter_map(|s| {
                    let host = s["host"].as_str()?.to_string();
                    let https =
                        matches!(s["https_support"].as_str(), Some("mandatory" | "optional"));
                    let vhost = s["vhost"].as_str().unwrap_or(&host).to_string();
                    // CdnServer is non_exhaustive; serde is its public constructor
                    serde_json::from_value(serde_json::json!({
                        "host": host,
                        "port": if https { 443 } else { 80 },
                        "https": https,
                        "vhost": vhost,
                    }))
                    .ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fetch_cdn_servers() -> Result<Vec<CdnServer>, String> {
    let response = ureq::get(CDN_SERVERS_URL)
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("Couldn't get Steam's download servers: {e}"))?;
    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_REPLY_BYTES)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let servers = parse_cdn_servers(&reply);
    if servers.is_empty() {
        return Err("Steam sent back no download servers.".to_string());
    }
    Ok(servers)
}

/// downloads every file in `item` into `out_dir`. `progress` gets the bytes
/// written so far and the total.
pub fn download(
    item: &ItemManifest,
    out_dir: &Path,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    let servers = fetch_cdn_servers()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let cdn = CdnClient::new().map_err(|e| e.to_string())?;
        let fetcher = CdnChunkFetcher::new(cdn, CdnServerPool::new(servers), None);
        let (events, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let job = DepotJob::builder()
            .depot_id(DepotId(RL_APPID))
            .depot_key(item.depot_key.clone())
            .install_dir(out_dir.to_path_buf())
            .max_downloads(MAX_CONCURRENT_CHUNKS)
            .event_sender(events)
            .build()
            .map_err(|e| e.to_string())?;
        let download = job.download(&item.manifest, Arc::new(fetcher));
        tokio::pin!(download);
        let total = item.total_bytes;
        let mut done: u64 = 0;
        loop {
            tokio::select! {
                result = &mut download => {
                    return result
                        .map(|_| ())
                        .map_err(|e| format!("The download failed: {e}"));
                }
                Some(event) = event_rx.recv() => {
                    // per-chunk uncompressed sizes, so they add up to the
                    // manifest's file sizes
                    if let DownloadEvent::ChunkCompleted { bytes } = event {
                        done = done.saturating_add(bytes);
                        progress(done.min(total), total);
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_plain_relative_paths_are_written() {
        assert!(is_safe_path("map.udk"));
        assert!(is_safe_path("sub\\folder\\map.udk"));
        assert!(!is_safe_path(""));
        assert!(!is_safe_path("..\\..\\evil.dll"));
        assert!(!is_safe_path("a/../../b"));
        assert!(!is_safe_path("/etc/passwd"));
        assert!(!is_safe_path("C:\\Windows\\x.dll"));
        assert!(!is_safe_path("./map.udk"));
    }

    #[test]
    fn manifest_replies_for_other_games_are_refused() {
        let reply = json!({
            "app_id": 440, "depot_id": 440, "manifest_id": "1",
            "depot_key": "00".repeat(32), "manifest": ""
        });
        assert!(parse_manifest_reply(&reply).is_err());
    }

    #[test]
    fn manifest_replies_need_a_32_byte_key() {
        let reply = json!({
            "app_id": 252950, "depot_id": 252950, "manifest_id": "1",
            "depot_key": "abcd", "manifest": ""
        });
        assert_eq!(
            parse_manifest_reply(&reply).err().as_deref(),
            Some("The manifest reply had no valid depot key.")
        );
    }

    #[test]
    fn the_servers_error_text_is_shown_when_it_sends_one() {
        assert_eq!(
            status_message(403, &json!({"error": "Item isn't a Rocket League map"})),
            "The Hebnix workshop service said: Item isn't a Rocket League map"
        );
        assert!(status_message(429, &Value::Null).contains("busy"));
        assert!(status_message(498, &json!({})).contains("clock"));
    }

    #[test]
    fn cdn_servers_come_from_the_web_api_reply() {
        let reply = json!({"response": {"servers": [
            {"type": "SteamCache", "host": "cache1.steamcontent.com", "vhost": "cache1.steamcontent.com", "https_support": "mandatory"},
            {"type": "CDN", "host": "cdn.example.net", "https_support": "unavailable"},
            {"type": "OpenCache", "host": "skip.me"},
        ]}});
        let servers = parse_cdn_servers(&reply);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].host, "cache1.steamcontent.com");
        assert!(servers[0].https);
        assert_eq!(servers[0].port, 443);
        assert!(!servers[1].https);
        assert_eq!(servers[1].port, 80);
        assert_eq!(servers[1].vhost, "cdn.example.net");
    }
}
