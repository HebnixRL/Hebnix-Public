# Workshop multiplayer: what the server side needs

This is the checklist for whoever stands up the coordination server Hebnix's
Workshop multiplayer clients connect to (`TSNET_CONTROL_URL` in
`crates/hebnix-app/src/multiplayer-lan/mod.rs`, currently
`https://api.hebnix.com`). Update that constant if the server ends up
somewhere else.

None of this affects the client build -- everything below is server-side
infrastructure and a small backend endpoint.

## 1. Run headscale (self-hosted Tailscale coordination server)

Standard self-hosted [headscale](https://headscale.net/), no special
Hebnix-specific config beyond the usual `server_url`/`listen_addr`/DERP/DNS
settings in its `config.yaml`.

## 2. Reverse proxy: must support raw duplex HTTP, not just request/response

**This is the one gotcha worth calling out explicitly, confirmed by testing
against a real client tonight.** Tailscale's registration protocol
(`ts2021`/noise) needs a raw, unbuffered, bidirectional connection through
whatever sits in front of headscale -- it is not a normal
request-then-response HTTP call.

- **A free/quick tunnel (e.g. Cloudflare Quick Tunnels) does not work.**
  Client registration fails with a `400 Bad Request` that never even reaches
  headscale's own logs -- the tunnel's proxying breaks the protocol before
  the request gets there. Confirmed directly this session.
- **A normal reverse proxy works fine**, as long as it doesn't buffer the
  request/response or force it down to plain request/response semantics.
  For nginx specifically, headscale's own docs cover this
  (`proxy_http_version 1.1`, `proxy_buffering off`, forwarding the
  `Upgrade`/`Connection` headers) -- see
  [headscale's reverse-proxy guide](https://headscale.net/stable/ref/integration/reverse-proxy/).
  If `api.hebnix.com` already runs behind nginx/Caddy for other things,
  adding a location block for headscale with those settings is enough; a
  dedicated subdomain isn't required.
- Plain HTTP directly on the box, no proxy at all, also works (confirmed
  tonight) if TLS isn't wanted for the coordination endpoint specifically --
  but a reverse proxy with real TLS is the normal production setup.

## 3. Room API: mint a pre-auth key per player, per room

The client calls `?request=tsnet/authkey` (see `RoomClient::request_tsnet_authkey`
in `crates/hebnix-app/src/multiplayer-lan/room_api.rs`) expecting back:

```json
{ "auth_key": "...", "control_url": "https://api.hebnix.com", "expires_at": "..." }
```

Server-side, that's minting a headscale pre-auth key scoped to a single
node, short-lived, and ephemeral (so it disappears from the tailnet
automatically when the player disconnects rather than accumulating stale
nodes forever):

```
headscale preauthkeys create --user <pool-user> --ephemeral --reusable=false --expiration 1h
```

(via headscale's gRPC/HTTP API from the room-api backend, not shelling out to
the CLI in production, but that's the equivalent operation). A single
headscale "user" to pool all Workshop room nodes under is fine -- Hebnix's
own client-side ACLs/policy isn't a concern yet at this stage.

## 4. Room API: `tailnet_ip` field

Once a player's node registers and shows up in `headscale nodes list`, the
room API's room/player records need a `tailnet_ip` field so other players in
the room can read out the address to pass to Rocket League's `-multihome`.
No specific mechanism prescribed here -- whatever's natural for however the
room API already polls/pushes player state.

---

Everything past this point is just for context, not action items:

The client used to plan on using Tailscale's `tsnet` Go library embedded
directly in a custom Hebnix helper process. That's been replaced with the
real, upstream `tailscaled` daemon + `tailscale` CLI (see
`sidecar/README.md`) -- `tsnet` doesn't expose a real OS network adapter,
which turned out to be a hard requirement for Rocket League's `-multihome`
to work. This doesn't change anything about what the server needs to do;
it's the same headscale protocol either way.
