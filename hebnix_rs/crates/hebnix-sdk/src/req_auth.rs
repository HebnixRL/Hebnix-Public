use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha1::Sha1;

include!(concat!(env!("OUT_DIR"), "/req_key.rs"));

static CLOCK_WARNING_PENDING: AtomicBool = AtomicBool::new(false);

/// Record a token rejection from a req.hebnix.com response for the app UI.
pub fn observe_response(status: u16, body: &serde_json::Value) {
    if status == 498
        && body.get("error").and_then(serde_json::Value::as_str)
            == Some("Incorrect application token")
    {
        CLOCK_WARNING_PENDING.store(true, Ordering::Relaxed);
    }
}

pub fn take_clock_warning() -> bool {
    CLOCK_WARNING_PENDING.swap(false, Ordering::Relaxed)
}

fn decode_base32(value: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut bits = 0u32;
    let mut count = 0u8;
    for ch in value.bytes().filter(|ch| !ch.is_ascii_whitespace() && *ch != b'-') {
        if ch == b'=' {
            break;
        }
        let digit = match ch.to_ascii_uppercase() {
            b'A'..=b'Z' => ch.to_ascii_uppercase() - b'A',
            b'2'..=b'7' => ch - b'2' + 26,
            _ => return Err("request key must be Base32 encoded".into()),
        };
        bits = (bits << 5) | u32::from(digit);
        count += 5;
        if count >= 8 {
            count -= 8;
            output.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    if output.is_empty() {
        return Err("request key is missing".into());
    }
    Ok(output)
}

fn token_at(key: &str, unix_seconds: u64) -> Result<String, String> {
    let secret = decode_base32(key)?;
    let counter = (unix_seconds / 30).to_be_bytes();
    let mut mac = Hmac::<Sha1>::new_from_slice(&secret).map_err(|_| "invalid request key")?;
    mac.update(&counter);
    let result = mac.finalize().into_bytes();
    let offset = usize::from(result[19] & 0x0f);
    let number = u32::from_be_bytes(result[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    let code = format!("{:08}", number % 100_000_000);
    let digest = Md5::digest(code.as_bytes());
    Ok(format!("{digest:x}"))
}

/// `Ok(None)` when this build carries no key (dev builds without a
/// `req_key.txt`), so callers can still send an untokened request.
pub fn app_token_if_configured() -> Result<Option<String>, String> {
    if KEY.trim().is_empty() {
        return Ok(None);
    }
    app_token().map(Some)
}

pub fn app_token() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch")?
        .as_secs();
    token_at(KEY, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B SHA-1 secret ("12345678901234567890") in Base32;
    // at T=59 the 8-digit code is 94287082.
    const RFC_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn token_is_md5_of_the_8_digit_totp() {
        let token = token_at(RFC_SECRET, 59).unwrap();
        assert_eq!(token, format!("{:x}", Md5::digest(b"94287082")));
    }

    #[test]
    fn missing_key_is_an_error() {
        assert!(token_at("", 59).is_err());
    }

    #[test]
    fn clock_warning_requires_both_status_and_token_error() {
        let token_error = serde_json::json!({"error": "Incorrect application token"});
        observe_response(401, &token_error);
        observe_response(498, &serde_json::json!({"error": "Other error"}));
        assert!(!take_clock_warning());

        observe_response(498, &token_error);
        observe_response(498, &token_error);
        assert!(take_clock_warning());
        assert!(!take_clock_warning());
    }
}
