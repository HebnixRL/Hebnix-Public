# Hebnix workshop API

Hebnix searches the Rocket League Steam Workshop and downloads maps straight
from Steam's CDN. Two things need a server: the Steam Web API key (for search)
and a Steam account that owns Rocket League (for depot manifests and the depot
key). This document is the contract between that server and the client in
`hebnix_rs/crates/hebnix-app/src/ui/workshop/steam_cdn.rs` and `steam_download.rs`.

The server never sends map files. Players download the map data from Steam's
CDN themselves; the server only hands out a few KB per map.

## Base URL and auth

- Default base URL: `https://api.hebnix.com/workshop`
- Override for testing: `HEBNIX_WORKSHOP_API_URL=http://127.0.0.1:8798`
- Every request carries `X-Hebnix-Key: <key>`, the same shared key the
  multiplayer directory uses (`directory_key.txt` / `HEBNIX_DIRECTORY_KEY`,
  baked in at build time). The server should reject requests without it and
  rate-limit per IP.

Errors: any non-200 status. The client shows a specific message for `404`
(unknown item) and `429` (rate-limited); anything else is shown with its code.

## `GET /v1/search`

Proxies Steam's `IPublishedFileService/QueryFiles` with the server's Web API
key.

| Query param | Meaning |
|---|---|
| `q` | search text, may be empty (empty = browse) |
| `sort` | `popular`, `trending` or `recent` |
| `cursor` | optional, the `next_cursor` from the previous page |

Suggested Steam mapping: `appid=252950`, `return_metadata=true`,
`numperpage=30`, `cursor=<cursor or *>`, `search_text=<q>`;
`popular` → `query_type=12` (total unique subscriptions), `trending` →
`query_type=3` (trend) with `days=7`, `recent` → `query_type=1` (publication
date). Check these against Steam's `EPublishedFileQueryType` docs when
building it.
Cache replies for a few minutes; they barely change.

Reply:

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

- `wid` must be the decimal workshop id as a string; items without one are dropped.
- `next_cursor` is `null` or `""` on the last page.
- Extra fields are ignored, so the server can add more (preview URL, author)
  for later UI work.

## `GET /v1/manifest/{wid}`

Returns the depot manifest for the item's current version and the depot key.

How the server gets it, logged in with a Steam account that owns Rocket League
(steamroom or SteamKit both work):

1. `ISteamRemoteStorage/GetPublishedFileDetails` for `wid`. Refuse unless
   `consumer_app_id == 252950`. The manifest id is `hcontent_file`.
2. Depot key for depot `252950` (app `252950`). It doesn't change, so fetch it
   once and keep it.
3. `ContentServerDirectory.GetManifestRequestCode` for app/depot `252950` and
   the manifest id, then download the manifest from the CDN
   (`/depot/252950/manifest/{manifest_id}/5/{request_code}`).
4. Cache the raw manifest bytes by `wid` + `time_updated`. A new
   `time_updated` means a new manifest.

Reply:

```json
{
  "app_id": 252950,
  "depot_id": 252950,
  "manifest_id": "7914978119799822363",
  "depot_key": "<64 hex chars>",
  "manifest": "<base64 of the manifest exactly as the CDN served it>"
}
```

The client refuses the reply unless both ids are `252950`, the key is 32
bytes, the manifest's own id matches `manifest_id`, every file path stays
inside the download folder, there are no symlinks, and the item is under
4 GB.

## Testing the client

```sh
HEBNIX_WORKSHOP_API_URL=http://127.0.0.1:8798 HEBNIX_TEST_WID=1759763738 \
  cargo test -p hebnix-app --bin hebnix -- --ignored downloads_a_map_end_to_end
```
