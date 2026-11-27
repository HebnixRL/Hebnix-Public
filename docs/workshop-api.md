# Hebnix workshop API

Server spec for in-app Workshop search and downloads. The client side is
`hebnix_rs/crates/hebnix-app/src/ui/workshop/steam_cdn.rs` and
`steam_download.rs`.

## What the server does, and what it doesn't

Hebnix searches the Rocket League Steam Workshop and downloads maps straight
from Steam's CDN. Players need two things they can't get themselves:

1. **Search results**, which need a Steam Web API key.
2. **The item's depot manifest and the depot key**, which need a Steam account
   that owns Rocket League.

The server provides only those. **It never sends map files.** Each player
downloads the map chunks from Steam's CDN and decrypts them locally. The
server sends a few KB per map, so no R2 or bandwidth cost scales with map size.

```
Hebnix ──search / manifest──▶ req.hebnix.com/workshop/v1 ──▶ Steam Web API
   │                                                     └──▶ Steam (owner account)
   └──────────── map chunks ────────────▶ Steam CDN (public, no login)
```

## Base URL

```
https://req.hebnix.com/workshop/v1
```

For local testing, the client's base URL can be overridden with
`HEBNIX_WORKSHOP_API_URL=http://127.0.0.1:8798`. The client appends
`/search` or `/manifest/{wid}` to whatever base it uses.

## Auth: the req key

Every request carries `X-App-Token`, the same app token every other
`req.hebnix.com` endpoint uses (`hebnix_sdk::req_auth::app_token()`):

- The secret is `HEBNIX_REQ_KEY` (Base32), the same secret the existing
  req.hebnix.com services already hold.
- The token is the lowercase hex **MD5 of the 8-digit TOTP code**: HMAC-SHA1,
  30-second step, RFC 6238 dynamic truncation, `code % 100_000_000`,
  zero-padded to 8 digits.
- Accept the current step and **one step either side** (±30 s) for clock drift.
- On a missing or wrong token, reply **`498`** with
  `{"error": "Incorrect application token"}`. Use exactly that text: the
  client checks for it and tells the player to fix their PC clock.

The token proves a request came from a Hebnix build, nothing more. The secret
is inside every released binary, so anyone determined can extract it. Treat it
as a filter for casual scripts. Rate limits and caching below are the real
protection.

## Error format

Every non-200 reply has a JSON body:

```json
{ "error": "Short sentence the player can read" }
```

The client shows the `error` text in the app, so write it for players, not
developers. If the body has no `error`, the client falls back to its own text
for `404`, `429` and `498`, and shows `HTTP <code>` for anything else.

| Status | When |
|---|---|
| `400` | Bad parameter (see the validation rules for each endpoint) |
| `403` | Item exists but isn't a usable Rocket League item (wrong app, not public, too big) |
| `404` | No such Workshop item |
| `429` | Rate-limited. Always send `Retry-After` (seconds). |
| `498` | Bad or missing app token |
| `502` | Steam failed or timed out |
| `503` | Endpoint switched off or backing off from Steam. Send `Retry-After`. |

## `GET /search`

Proxies Steam's `IPublishedFileService/QueryFiles` using the server's Web API
key. The key never appears in any reply.

### Query parameters

| Param | Rules |
|---|---|
| `q` | Search text. Optional; empty means browse. Trim it, then reject (`400`) anything over 100 characters or containing control characters. |
| `sort` | Must be exactly `popular`, `trending` or `recent`. Anything else is a `400`. |
| `cursor` | Optional. The `next_cursor` from the previous page. Reject (`400`) anything over 256 characters or outside `[A-Za-z0-9+/=_-]`. |

Reject unknown parameters with `400` too, so nobody can use the server to send
arbitrary QueryFiles options to Steam.

### Steam call

| Field | Value |
|---|---|
| `appid` | `252950` |
| `numperpage` | `30`, fixed. The client can't change it. |
| `cursor` | the client's cursor, or `*` |
| `search_text` | `q` (omit when empty) |
| `return_metadata` | `true` |
| `query_type` | `popular` → `12` (most unique subscribers), `trending` → `3` with `days=7`, `recent` → `1` (newest) |

Verify these `query_type` numbers against Steam's `EPublishedFileQueryType`
docs when building it.

### Reply

```json
{
  "total": 1234,
  "next_cursor": "AoJ4...",
  "items": [
    {
      "wid": "1759763738",
      "title": "Speed Jump: Trials 3",
      "file_size": 59224775,
      "subscriptions": 120000
    }
  ]
}
```

- `wid` is the decimal Workshop ID **as a string**. The client drops items
  without one.
- `next_cursor` is `null` or `""` on the last page.
- Drop items Steam marks as banned or not public.
- Extra fields are ignored. You can add `preview_url` or `author` now; the UI
  can use them later.

### Caching

Cache each reply for **10 minutes**, keyed on `(lowercased trimmed q, sort,
cursor)`. Most traffic is the empty-query browse pages, so nearly every
request should be a cache hit and never reach Steam.

## `GET /manifest/{wid}`

Returns the depot manifest for the item's current version, plus the depot key.

### Validation

- `wid` must match `^[0-9]{1,20}$` and fit in a u64. Otherwise `400`.

### Steps

Log the server in with a **dedicated** Steam account that owns Rocket League
(see Protecting the Steam account). steamroom (Rust) and SteamKit2 (.NET) can
both do this.

1. Call `ISteamRemoteStorage/GetPublishedFileDetails` for `wid`. This endpoint
   needs no key and is POST form data.
   - `result != 1`: return `404`.
   - `consumer_app_id != 252950`: return `403` ("That item isn't for Rocket
     League.").
   - Banned or not public (`visibility != 0`): return `403`.
   - `file_size` over 4 GB: return `403`.
   - The manifest ID is `hcontent_file`, and `time_updated` tells you when the
     item changed.
2. Get the depot key for depot `252950` (app `252950`). It never changes, so
   fetch it once at startup and keep it in memory. Never write it to logs.
3. Call `ContentServerDirectory.GetManifestRequestCode` for app `252950`,
   depot `252950` and the manifest ID. Then download
   `/depot/252950/manifest/{manifest_id}/5/{request_code}` from a CDN host.
   Request codes expire after a few minutes, so fetch one right before
   downloading and don't store it.
4. Cache the raw manifest bytes keyed on `(wid, time_updated)`. A new
   `time_updated` means a new manifest. Reject any manifest over 8 MB, since a
   real map manifest is a few KB.

### Reply

```json
{
  "app_id": 252950,
  "depot_id": 252950,
  "manifest_id": "7914978119799822363",
  "depot_key": "<64 hex chars>",
  "manifest": "<base64 of the manifest exactly as the CDN served it>"
}
```

`manifest_id` is a string because it doesn't fit in a JSON number safely.

The client refuses the reply unless:

- both IDs are `252950`;
- the key is 32 bytes;
- the manifest's own ID matches `manifest_id`;
- every file path stays inside the download folder (no `..`, no absolute
  paths, no drive letters);
- there are no symlinks;
- the item is 4 GB or less and has at least one file.

### Caching

- **Manifests**: cache by `(wid, time_updated)` with no expiry; they never
  change. Evict least-recently-used if storage matters. Each one is a few KB,
  so 10,000 maps is tens of MB.
- **Item details** (step 1): cache **10 minutes**. This is what decides whether
  a cached manifest is still current, so the owner account is only used when a
  map really changed or was never requested before.
- **Failures**: cache `404`s and `403`s for **10 minutes**, so repeatedly
  requesting a bad ID costs nothing.
- **Duplicate requests**: when several requests for the same uncached `wid`
  arrive together, make one Steam fetch and give every waiting request its
  result.

## Abuse protection

The expensive or fragile things are:

1. the Steam owner account;
2. the Web API key's daily quota (100,000 calls per day);
3. the server itself.

Everything below protects those, roughly in order of importance.

### Rate limits

Apply limits per client IP. Group IPv6 addresses by `/64`, since one user
usually holds a whole block. If the server sits behind Cloudflare, read the IP
from `CF-Connecting-IP` and ignore `X-Forwarded-For`, which the client can
forge.

| Endpoint | Per IP | Burst |
|---|---|---|
| `/search` | 30 per minute | 10 |
| `/manifest/{wid}` | 20 per minute, 300 per day | 5 |

These are well above what a person clicking in the app can reach. Answer with
`429` and `Retry-After`.

Cache hits still count toward these limits, so a scraper can't pull the whole
manifest cache quickly.

### Global caps on Steam

These limits are server-wide, independent of the client:

- **Owner account**: at most **600 manifest request codes per hour** and
  **2 at a time**. Requests past that wait in a queue for up to 10 seconds,
  then get `503` with `Retry-After`. Only cache misses use this budget.
- **Web API**: at most **50,000 calls per day** (half of Steam's limit, to
  leave room for other features). Past that, search returns `503` until the
  next UTC day. Cached pages keep working.
- **When Steam pushes back**: a `429`, `AccessDenied`, `RateLimitExceeded` or
  logon failure triggers exponential back-off, starting at 1 minute and
  capped at 1 hour. During back-off, serve from cache only and return `503`
  for anything else. Never retry in a tight loop.

### Protecting the Steam account

- Use a **separate Steam account** that owns Rocket League. Don't use anyone's
  personal account: if Valve restricts it, nothing else is lost. The account
  should own nothing else and hold no Steam Wallet balance.
- Store the **refresh token**, not the password. Keep it in the server's
  secret store, never in the repo or in logs.
- Use Steam Guard. With a mobile authenticator, keep the shared secret in the
  secret store so the server can log back in by itself after a restart.
- Keep **one logged-in session** for the whole server and reuse it. Don't log
  in per request.
- If the account gets restricted, turn off manifests with the kill switch
  (below), swap the account, and turn it back on. Clients need no update.

### Kill switches

Two config flags switch each endpoint off without a redeploy:

- `WORKSHOP_SEARCH_ENABLED`
- `WORKSHOP_MANIFEST_ENABLED`

While off, the endpoint returns `503` with an `error` the player sees, for
example "Workshop downloads are paused for maintenance." Cached search pages
may still be served.

### Input and output hygiene

- Validate every parameter as described above, before any Steam call or cache
  lookup.
- Never pass client input to Steam other than `q`, `cursor` and the numeric
  `wid`.
- Never forward Steam's error text to clients. Map it to the statuses above.
- Never return the Web API key, the refresh token or the request codes.
  (Players do need the depot key, which is the same for everyone.)
- Cap request bodies at 0 bytes (both endpoints are GET only) and URLs at
  2 KB.
- Send `Cache-Control: no-store` on replies. Hebnix doesn't cache them, and
  shared proxies shouldn't either.

### Logging and monitoring

- Log one line per request: time, a hash of the IP, endpoint, `wid` or a hash
  of `q`, status, cache hit or miss, and Steam time in ms. Keep logs 14 days.
  Don't log the raw `q` or full IPs.
- Alert when:
  - `429`s go over 5% of requests;
  - owner-account use reaches 80% of its hourly cap;
  - Web API use reaches 80% of its daily cap;
  - back-off starts;
  - `498`s spike, which can mean the req key leaked or a client clock bug
    shipped.
- If the req key is clearly leaked, rotate `HEBNIX_REQ_KEY` along with the
  other req.hebnix.com services and ship a release. Until then, rate limits
  carry the load.

## Before going live

- [ ] `498` with the exact `Incorrect application token` text on a bad token,
      and acceptance of ±1 TOTP step.
- [ ] `400` for an invalid `sort`, a `q` over 100 characters, a bad `cursor`
      and a non-numeric `wid`.
- [ ] `403` for a Workshop item from another game (any non-RL `wid`).
- [ ] `404` for a `wid` that doesn't exist, cached so a repeat doesn't call
      Steam.
- [ ] A second `/manifest/1759763738` is a cache hit, with no owner-account
      call in the logs.
- [ ] Hitting the rate limit gives `429` with `Retry-After`.
- [ ] The kill switch gives `503` and the app shows its text.
- [ ] End-to-end from the client:

```sh
HEBNIX_WORKSHOP_API_URL=https://req.hebnix.com/workshop/v1 HEBNIX_TEST_WID=1759763738 \
  cargo test -p hebnix-app --bin hebnix -- --ignored downloads_a_map_end_to_end
```

That test needs a build with `HEBNIX_REQ_KEY` set, otherwise it can't make a
token.
