//! Launch.log parser.
//!
//! pulls session info (username, steam id, rich presence..) and match info
//! (playlist, mode, map, server ip/port) once the stats api confirms it.
//! playlist names come from the psynet config api.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::log::models::{LogGameInfo, LogInfo, LogSessionInfo};
use crate::process::is_rocket_league_running;
use crate::utils::psynet::{fetch_psynet_config, get_online_playlists};

const STATS_HOST: &str = "127.0.0.1";
const STATS_PORT: u16 = 49123;
const STATS_VERIFY_TIMEOUT: f64 = 3.0;

macro_rules! re {
    ($name:ident, $pattern:expr) => {
        fn $name() -> &'static Regex {
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

re!(re_username, r"DevOnline: Logged in as '([^']+)'");
re!(re_steam_id, r"DevOnline: Steam ID: (\d+)");
re!(re_epic_username, r#"(?i)-epicusername=(?:"([^"]+)"|(\S+))"#);
re!(re_epic_id, r"(?i)-epicuserid=([0-9a-f]+)");
re!(
    re_eos_id,
    r"Connect login result 'EOS_Success' for player '([0-9a-fA-F]{32})'"
);
re!(
    re_primary_id,
    r"Online_X\.UniqueNetIDToString\([^)]*PlayerID\)=\(([^|()]+)\|([^|()]+)\|(\d+)\)"
);
re!(re_platform, r"ScriptLog: Detected platform: (\w+)");
re!(
    re_rich_presence,
    r"(?m)DevOnline: Set rich presence to: (.+?) data: (.+?)\s*$"
);
re!(
    re_welcomed,
    r"DevNet: Welcomed by server \(Level: ([^,]+), Game: ([^,]+), GameTags: ([^)]+)\)"
);
// JoinSettings writes PlaylistId=6, the anti cheat line writes PlaylistId=(6)
re!(re_playlist_id, r"PlaylistId=\(?(\d+)");
re!(re_playlist, r"Playlist=\(?(\d+)");
re!(re_server_name, r#"ServerName="([^"]+)""#);
re!(re_region, r#"Region="([^"]+)""#);
re!(re_browse_remote, r"DevNet: Browse: ([\d.]+):(\d+)/(\S+)");
re!(re_browse_local, r"DevNet: Browse: (\S+)");
re!(re_build_id, r"Log: BuildID: (\d+) from GPsyonixBuildID");
re!(re_browse_game, r"[?&]Game=([^?&]+)");
re!(re_browse_tags, r"[?&]GameTags=([^?&]+)");
re!(re_loadmap_tags, r"LoadMap: \S*[?&]GameTags=([^?&\s]+)");
re!(re_loadmap_offline, r"LoadMap: \S*[?&]Offline(?:[?&]|\s|$)");

fn find_last<'t>(re: &Regex, text: &'t str) -> Option<regex::Captures<'t>> {
    re.captures_iter(text).last()
}

// Stats API verification

/// quick check if we're in a live match via the stats api socket. returns
/// (in_game, api_available). in_game: Some(true)=in match, Some(false)=not,
/// None=couldn't tell.
fn verify_via_stats_api(timeout: f64) -> (Option<bool>, bool) {
    let addr = format!("{STATS_HOST}:{STATS_PORT}");
    let Ok(parsed) = addr.parse() else {
        return (None, false);
    };
    let Ok(mut sock) = TcpStream::connect_timeout(&parsed, Duration::from_secs(2)) else {
        return (None, false);
    };
    if sock
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n")
        .is_err()
    {
        return (None, false);
    }
    let api_available = true;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));

    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 65536];

    while Instant::now() < deadline {
        match sock.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if text.contains("\"UpdateState\"") || text.contains("\"event\":\"UpdateState\"") {
                    return (Some(true), api_available);
                }
                if text.contains("\"MatchEnded\"")
                    || text.contains("\"event\":\"MatchEnded\"")
                    || text.contains("\"MatchDestroyed\"")
                    || text.contains("\"event\":\"MatchDestroyed\"")
                {
                    return (Some(false), api_available);
                }
                if text.contains("\"RoundStarted\"") || text.contains("\"event\":\"RoundStarted\"")
                {
                    return (Some(true), api_available);
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return (None, api_available),
        }
    }
    (None, api_available)
}

fn check_stats_port() -> bool {
    let addr = format!("{STATS_HOST}:{STATS_PORT}");
    addr.parse()
        .ok()
        .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(1)).ok())
        .is_some()
}

// Main entry point

/// On Linux, Launch.log lives under the emulated "My Documents" of the
/// Wine/Proton prefix Rocket League runs in, never the host's ~/Documents.
#[cfg(target_os = "linux")]
fn find_launch_log() -> PathBuf {
    let rel = Path::new("My Games/Rocket League/TAGame/Logs/Launch.log");
    let docs = crate::process::find_rocket_league()
        .and_then(|game| crate::process::documents_dir_for_exe(&game.exe_path))
        .or_else(crate::process::rocket_league_documents_dir)
        .or_else(dirs::document_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    docs.join(rel)
}

#[cfg(target_os = "windows")]
fn find_launch_log() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("My Games")
        .join("Rocket League")
        .join("TAGame")
        .join("Logs")
        .join("Launch.log")
}

/// Read the local identity and display name without fetching playlists or contacting an API.
pub fn local_player_session() -> Option<LogSessionInfo> {
    let bytes = std::fs::read(find_launch_log()).ok()?;
    Some(parse_session(&String::from_utf8_lossy(&bytes)))
}

/// Read match context once StatsAPI has reported a live match.
pub fn local_match_context() -> Option<(Option<String>, LogGameInfo)> {
    let bytes = std::fs::read(find_launch_log()).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some((
        parse_primary_id(&text),
        parse_game_info(since_last_menu_load(&text), true, &Default::default()),
    ))
}

/// Match identity from the join that led to the current map, without a
/// network request. Unknown sessions remain unknown rather than inheriting
/// the playlist of an earlier match.
pub fn current_match_playlist() -> Option<i64> {
    let bytes = std::fs::read(find_launch_log()).ok()?;
    match_playlist_at_last_load(&String::from_utf8_lossy(&bytes))
}

fn recent_launch_log() -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(find_launch_log()).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(256 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The current map URL, provided no new travel is pending.
pub fn current_map_url() -> Option<String> {
    let recent = recent_launch_log()?;
    if recent.contains("Log: LoadMap: ") {
        current_map_from_log(&recent)
    } else {
        let bytes = std::fs::read(find_launch_log()).ok()?;
        current_map_from_log(&String::from_utf8_lossy(&bytes))
    }
}

fn current_map_from_log(log: &str) -> Option<String> {
    let (_, load) = log.rsplit_once("Log: LoadMap: ")?;
    let (url, events) = load.split_once('\n')?;
    (!events.contains("DevNet: Browse:")).then(|| url.trim().to_owned())
}

/// True once the requested map has finished loading, unless another travel
/// has already started. Engine-added URL options do not affect the map name.
pub fn map_load_finished(expected_map: &str) -> bool {
    recent_launch_log().is_some_and(|log| {
        log.rsplit_once("Log: LoadMap: ")
            .and_then(|(_, load)| load.split_once('\n'))
            .is_some_and(|(url, events)| {
                url.split('?').next().is_some_and(|map| map.trim().eq_ignore_ascii_case(expected_map))
                    && events.contains("Finished loading level:")
                    && !events.contains("DevNet: Browse:")
            })
    })
}

fn match_playlist_at_last_load(text: &str) -> Option<i64> {
    let (before, load) = text.rsplit_once("LoadMap: ")?;
    let url = load.lines().next()?.split_whitespace().next()?;
    let map = url.split('?').next()?;
    if map.eq_ignore_ascii_case("MENU_Main_p") || map.starts_with("JoinGameTransition") {
        return None;
    }
    let training = url.split('?').skip(1).any(|option| {
        let Some((key, value)) = option.split_once('=') else { return false };
        (key.eq_ignore_ascii_case("GameTags") && value.split(',').any(|tag| {
            tag.eq_ignore_ascii_case("Freeplay") || tag.eq_ignore_ascii_case("Training")
        })) || (key.eq_ignore_ascii_case("Game") && value.to_ascii_lowercase().contains("gameinfo_training"))
    });
    if training || last_map_load_is_offline(text) {
        return Some(0);
    }
    let boundary = before.rmatch_indices("LoadMap: ").find_map(|(at, marker)| {
        let map = &before[at + marker.len()..];
        (!map.starts_with("JoinGameTransition")).then_some(at)
    }).unwrap_or(0);
    let join = &before[boundary..];
    let join = join.rfind("JoinGame: StartJoin").map_or(join, |at| &join[at..]);
    let private = join.lines().next().is_some_and(|line| {
        line.starts_with("JoinGame: StartJoin") && line.contains("MatchType=JMT_Private")
    });
    let playlist = join.lines()
        .rev()
        .filter(|line| line.contains("TryToPlayOnlineWithAntiCheat") || line.contains("JoinGame: StartJoin"))
        .find_map(|line| re_playlist_id().captures(line)?.get(1)?.as_str().parse().ok());
    if private && !matches!(playlist, Some(22 | 34)) {
        return Some(6);
    }
    playlist
}

#[cfg(test)]
#[test]
fn playlist_is_scoped_to_the_join_for_the_loaded_map() {
    let private = "Log: LoadMap: MENU_Main_p\n\
        JoinGame: StartJoin JoinSettings=((MatchType=JMT_Private,PlaylistId=6))\n\
        Log: LoadMap: JoinGameTransition?game=TAGame.GameInfo_Transition_TA\n\
        Log: LoadMap: 127.0.0.1:9005/music_p?GameTags=Ball_God";
    assert_eq!(match_playlist_at_last_load(private), Some(6));
    assert_eq!(match_playlist_at_last_load(&private.replace(",PlaylistId=6", "")), Some(6));
    assert_eq!(match_playlist_at_last_load(&format!("{private}\nLog: LoadMap: music_p")), None);
    assert_eq!(match_playlist_at_last_load(&format!("{private}\nLog: LoadMap: music_p?Offline")), Some(0));
    assert_eq!(match_playlist_at_last_load(&format!("{private}\nLog: LoadMap: stadium_day_p?Game=TAGame.GameInfo_Soccar_TA?GameTags=Freeplay")), Some(0));
    assert_eq!(match_playlist_at_last_load("Log: LoadMap: stadium_p?Game=TAGame.GameInfo_Training_TA"), Some(0));
    let tournament = format!("{private}\nOnline: TryToPlayOnlineWithAntiCheat PlaylistId=(34)\nLog: LoadMap: 127.0.0.1:9005/music_p");
    assert_eq!(match_playlist_at_last_load(&tournament), Some(34));
}

fn parse_session(text: &str) -> LogSessionInfo {
    // Session info (always parsed)
    let mut session = LogSessionInfo::default();
    if let Some(c) = find_last(re_username(), &text) {
        session.username = Some(c[1].to_string());
    }
    if let Some(c) = re_steam_id().captures(&text) {
        session.steam_id = Some(c[1].to_string());
    }
    if let Some(c) = find_last(re_epic_id(), &text) {
        session.epic_id = Some(c[1].to_string());
    }
    if session.epic_id.is_none() {
        if let Some(c) = find_last(re_eos_id(), &text) {
            session.epic_id = Some(c[1].to_string());
        }
    }
    session.primary_id = parse_primary_id(&text);
    // Epic's launcher command line is local and remains unaffected by Hebnix's
    // account-spoofer proxy. Prefer it over an externally supplied display name.
    if session.epic_id.is_some() {
        if let Some(c) = find_last(re_epic_username(), &text) {
            session.username = Some(
                c.get(1)
                    .or_else(|| c.get(2))
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
            );
        }
    }
    if let Some(c) = re_platform().captures(&text) {
        session.platform = Some(c[1].to_string());
    }
    if let Some(c) = find_last(re_rich_presence(), &text) {
        session.rich_presence = Some(c[1].trim().to_string());
        session.rich_presence_data = Some(c[2].trim().to_string());
    }

    session
}

/// Read the local account ID without network or Stats API verification.
pub fn local_player_id() -> Option<String> {
    let bytes = std::fs::read(find_launch_log()).ok()?;
    parse_primary_id(&String::from_utf8_lossy(&bytes))
}

fn parse_primary_id(text: &str) -> Option<String> {
    if let Some(c) = find_last(re_primary_id(), text) {
        return Some(format!("{}|{}|{}", &c[1], &c[2], &c[3]));
    }
    if let Some(c) = find_last(re_epic_id(), text).or_else(|| find_last(re_eos_id(), text)) {
        return Some(format!("Epic|{}|0", &c[1]));
    }
    re_steam_id()
        .captures(text)
        .map(|c| format!("Steam|{}|0", &c[1]))
}

/// A join that failed because the host's map is not installed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinMismatch {
    /// address the game tried to connect to
    pub ip: String,
    /// the package the game could not match
    pub package: String,
    /// where the line sits in the log, to tell a new attempt from an old one
    pub at: usize,
}

/// The last failed join in the log, `None` if RL is not running.
pub fn join_mismatch(log_path: Option<&Path>) -> Option<JoinMismatch> {
    let log_path = log_path
        .map(Path::to_path_buf)
        .unwrap_or_else(find_launch_log);
    let text = String::from_utf8_lossy(&std::fs::read(log_path).ok()?).to_string();
    if !is_rocket_league_running() {
        return None;
    }
    join_mismatch_in(&text)
}

fn join_mismatch_in(text: &str) -> Option<JoinMismatch> {
    let mut at = 0;
    let mut found = None;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.split_once("Pending connect to '").map(|(_, r)| r) {
            if let (Some((ip, _)), Some((_, package))) = (
                rest.split_once('/'),
                rest.split_once("Content mismatch (Package "),
            ) {
                if let Some((package, _)) = package.split_once(')') {
                    found = Some(JoinMismatch {
                        ip: ip.into(),
                        package: package.into(),
                        at,
                    });
                }
            }
        }
        at += line.len();
    }
    found
}

/// The map file this player is hosting a LAN match on right now: the path on
/// the last `LoadMap:` line, when that load listens for players. A later menu
/// load, or any non-hosting load, means nothing is being hosted.
pub fn hosted_map(log_path: Option<&Path>) -> Option<String> {
    let log_path = log_path
        .map(Path::to_path_buf)
        .unwrap_or_else(find_launch_log);
    let text = String::from_utf8_lossy(&std::fs::read(log_path).ok()?).to_string();
    if !is_rocket_league_running() {
        return None;
    }
    hosted_map_in(&text)
}

fn hosted_map_in(text: &str) -> Option<String> {
    let line = text[text.rfind("LoadMap:")?..].lines().next()?;
    let target = line.strip_prefix("LoadMap:")?.trim();
    let (path, options) = target.split_once('?')?;
    let has = |flag: &str| options.split('?').any(|option| option.eq_ignore_ascii_case(flag));
    (has("Listen") && !has("HbxSoloPractice") && !path.is_empty()).then(|| path.to_string())
}

/// Parse Launch.log into session + game info. verify=true checks the Stats API
/// before returning game data. Session information is always returned.
pub fn parse_launch_log(log_path: Option<&Path>, verify: bool, lang: &str) -> LogInfo {
    let log_path = log_path
        .map(Path::to_path_buf)
        .unwrap_or_else(find_launch_log);

    if !log_path.is_file() {
        return LogInfo {
            log_path: Some(log_path.to_string_lossy().to_string()),
            ..Default::default()
        };
    }

    let bytes = std::fs::read(&log_path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).to_string();

    // PsyNet config for playlist names
    let build_id = re_build_id().captures(&text).map(|c| c[1].to_string());
    let online_playlists = match &build_id {
        Some(id) => get_online_playlists(&fetch_psynet_config(id, lang)),
        None => Default::default(),
    };

    let session = parse_session(&text);

    // Game info (only after verification)
    let mut game: Option<LogGameInfo> = None;
    let rl_running = is_rocket_league_running();
    let stats_available;
    let verified;

    if verify {
        let (result, avail) = verify_via_stats_api(STATS_VERIFY_TIMEOUT);
        verified = result == Some(true);
        stats_available = avail;
    } else {
        verified = false;
        stats_available = check_stats_port();
    }

    // rl down means the log describes a session that already ended
    if rl_running && (verified || !verify) {
        let mut g = parse_game_info(&text, verified, &online_playlists);

        // in-game but map/ip missing? wait + re-read, the log fills in during join
        if verified && g.map_name.is_none() && g.server_ip.is_none() {
            std::thread::sleep(Duration::from_secs(1));
            if let Ok(bytes2) = std::fs::read(&log_path) {
                let text2 = String::from_utf8_lossy(&bytes2).to_string();
                if text2 != text {
                    g = parse_game_info(&text2, verified, &online_playlists);
                }
            }
        }
        game = Some(g);
    }

    LogInfo {
        session,
        game,
        build_id: build_id.filter(|_| rl_running),
        log_path: Some(log_path.to_string_lossy().to_string()),
        parse_time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0),
        stats_api_available: stats_available,
    }
}

// Game info sub-parser

fn parse_game_info(
    text: &str,
    verified: bool,
    online_playlists: &std::collections::HashMap<i64, String>,
) -> LogGameInfo {
    let mut game = LogGameInfo {
        verified,
        ..Default::default()
    };

    // playlist id. an offline match queues nothing, so a stale id from the last
    // online one is still sitting in the log
    game.offline = last_map_load_is_offline(text);
    if !game.offline {
        let queued = since_last_menu_load(text);
        if let Some(c) = find_last(re_playlist_id(), queued) {
            game.playlist_id = c[1].parse().ok();
        } else if let Some(c) = find_last(re_playlist(), queued) {
            game.playlist_id = c[1].parse().ok();
        }
    }
    if let Some(pid) = game.playlist_id {
        game.playlist_name = online_playlists
            .get(&pid)
            .cloned()
            .or_else(|| crate::utils::playlists::known_name(pid).map(str::to_owned));
    }

    // welcomed by server (map + game class + tags)
    let mut game_class = String::new();
    if let Some(c) = find_last(re_welcomed(), text) {
        game.map_name = Some(c[1].to_string());
        game_class = c[2].to_string();
        let tags = c[3].to_string();
        if !tags.is_empty() {
            game.game_tags = Some(tags);
        }
    }

    // browse urls (server ip/port for online, map for training/freeplay)
    let m_remote = find_last(re_browse_remote(), text);
    if let Some(c) = &m_remote {
        game.server_ip = Some(c[1].to_string());
        game.server_port = c[2].parse().ok();
    }

    // no welcomed-by-server map? try the browse url path
    if game.map_name.is_none() {
        if let Some(c) = &m_remote {
            let path = c[3].to_string();
            game.map_name = extract_map_from_browse_path(&path);
            if game_class.is_empty() {
                if let Some(g) = extract_from_browse_query(&path, re_browse_game()) {
                    game_class = g;
                }
            }
            if game.game_tags.is_none() {
                game.game_tags = extract_from_browse_query(&path, re_browse_tags());
            }
        }

        let needs_local = match &game.map_name {
            None => true,
            Some(m) => m.starts_with("JoinGameTransition"),
        };
        if needs_local {
            // last local browse line (no ip:port prefix)
            let local = re_browse_local()
                .captures_iter(text)
                .filter(|c| {
                    let path = &c[1];
                    // skip remote-style ip:port paths handled above
                    !re_browse_remote().is_match(&format!("DevNet: Browse: {path}"))
                })
                .last();
            if let Some(c) = local {
                let path = c[1].to_string();
                if let Some(map_name) = extract_map_from_browse_path(&path) {
                    if map_name != path {
                        game.map_name = Some(map_name);
                    }
                }
                if game_class.is_empty() {
                    if let Some(g) = extract_from_browse_query(&path, re_browse_game()) {
                        game_class = g;
                    }
                }
                if game.game_tags.is_none() {
                    game.game_tags = extract_from_browse_query(&path, re_browse_tags());
                }
            }
        }
    }

    if !game_class.is_empty() {
        game.game_class = Some(game_class);
    }

    // the last LoadMap is the current map. matching the last LoadMap that happens to carry GameTags instead inherits them from an older load, a menu clears them
    if text.contains("LoadMap:") {
        game.game_tags = tags_of_last_map_load(text);
    }
    if let Some(tags) = game.game_tags.as_deref() {
        (game.mutators, game.bot_skill, game.player_count) = split_game_tags(tags);
    }
    game.mutator_count = count_mutator_packages(text).max(game.mutators.len() as i64);

    if let Some(c) = find_last(re_server_name(), text) {
        game.server_name = Some(c[1].to_string());
    }
    if let Some(c) = find_last(re_region(), text) {
        game.region = Some(c[1].to_string());
    }

    game
}

/// the menu load between two matches, so a queue cannot outlive its own match
fn since_last_menu_load(text: &str) -> &str {
    match text.rfind("LoadMap: MENU_Main_p") {
        Some(pos) => &text[pos..],
        None => text,
    }
}

/// exhibition and season write ?Offline? on the load, private matches do not
fn last_map_load_is_offline(text: &str) -> bool {
    let Some(pos) = text.rfind("LoadMap:") else {
        return false;
    };
    text[pos..]
        .lines()
        .next()
        .is_some_and(|line| re_loadmap_offline().is_match(line))
}

/// GameTags off the last LoadMap, None when that load carries none
fn tags_of_last_map_load(text: &str) -> Option<String> {
    let pos = text.rfind("LoadMap:")?;
    let line = text[pos..].lines().next()?;
    re_loadmap_tags().captures(line).map(|c| c[1].to_string())
}

/// the game loads Mutators_SF.upk once per mutator
fn count_mutator_packages(text: &str) -> i64 {
    let from = text.rfind("LoadMap:").map(|i| i + 1).unwrap_or(0);
    text[from..].matches("Mutators_SF").count() as i64
}

fn split_game_tags(tags: &str) -> (Vec<String>, Option<String>, Option<i64>) {
    let mut mutators = Vec::new();
    let mut bot_skill = None;
    let mut player_count = None;
    for tag in tags.split(',') {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        if let Some(skill) = tag.strip_prefix("Bots") {
            bot_skill = Some(skill.to_string());
        } else if let Some(count) = tag.strip_prefix("PlayerCount") {
            player_count = count.parse().ok();
        } else {
            mutators.push(tag.to_string());
        }
    }
    (mutators, bot_skill, player_count)
}

/// map name from a browse url path
fn extract_map_from_browse_path(path: &str) -> Option<String> {
    let path_only = path.split('?').next().unwrap_or("");
    let lower = path_only.to_lowercase();
    if lower.starts_with("joingametransition") || lower.starts_with("menu_") {
        return None;
    }
    if path_only.is_empty() || (path_only.contains(':') && !path_only.contains('/')) {
        return None;
    }
    Some(path_only.to_string())
}

fn extract_from_browse_query(path: &str, re: &Regex) -> Option<String> {
    re.captures(path).map(|c| c[1].to_string())
}

#[cfg(test)]
mod hosted_map_tests {
    use super::hosted_map_in;

    const HOSTING: &str = "[0064.25] Log: LoadMap: E:\\maps\\1843822064\\n.udk?Game=TAGame.GameInfo_Soccar_TA?Workshop?GameTags=BotsNone?Lan?Listen\n";

    #[test]
    fn hosting_load_gives_the_map_path() {
        assert_eq!(
            hosted_map_in(HOSTING).as_deref(),
            Some("E:\\maps\\1843822064\\n.udk")
        );
    }

    #[test]
    fn freeplay_or_a_later_menu_load_is_not_hosting() {
        let freeplay = "[1.0] Log: LoadMap: E:\\maps\\n.udk?Game=TAGame.GameInfo_Soccar_TA?Workshop?GameTags=Freeplay\n";
        assert_eq!(hosted_map_in(freeplay), None);
        assert_eq!(hosted_map_in(&HOSTING.replace("?Lan?Listen", "?HbxSoloPractice?Lan?Listen")), None);
        let after = format!("{HOSTING}[2.0] Log: LoadMap: MENU_Main_p?closed\n");
        assert_eq!(hosted_map_in(&after), None);
        assert_eq!(hosted_map_in("no loads"), None);
    }
}

#[cfg(test)]
mod join_mismatch_tests {
    use super::join_mismatch_in;

    #[test]
    fn finds_the_last_failed_join() {
        let log = "[1.0] Log: Pending connect to '127.10.0.2/MENU_Main_p?Lan??NumPlayers=1' failed; Content mismatch (Package speedjumptrials1) (Error: 40) (Error: 43)
[2.0] Log: other
";
        let found = join_mismatch_in(log).unwrap();
        assert_eq!(
            (found.ip.as_str(), found.package.as_str(), found.at),
            ("127.10.0.2", "speedjumptrials1", 0)
        );
        assert_eq!(
            join_mismatch_in(
                "[1.0] Log: nothing here
"
            ),
            None
        );
        let again = format!("{log}{log}");
        assert_eq!(join_mismatch_in(&again).unwrap().at, log.len());
    }
}

#[cfg(test)]
mod local_session_tests {
    use super::*;
    #[test]
    fn local_identity_and_name_are_paired_without_online_verification() {
        let session = parse_session(
            "-epicuserid=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -epicusername=\"Saved Player\"\nDevOnline: Logged in as 'Spoofed Name'",
        );
        assert_eq!(session.username.as_deref(), Some("Saved Player"));
        assert_eq!(
            session.primary_id.as_deref(),
            Some("Epic|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|0")
        );
    }
}
