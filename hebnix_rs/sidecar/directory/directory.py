#!/usr/bin/env python3
"""Hebnix Workshop multiplayer directory.

Every Hebnix with Workshop multiplayer on runs a tailcat node (the hebnix-tc
sidecar) and announces it here every 20 seconds:

    POST /  (header X-Hebnix-Key: <shared key>)
    {"id": "<16 hex>", "token": "tc...", "name": "<player name>"}

and gets back everyone currently announced, itself included:

    {"peers": [{"id": "...", "token": "...", "name": "..."}, ...]}

A clean departure sends DELETE with the same id and token. An entry that
isn't refreshed for TTL seconds is dropped. Nothing is written
to disk. The key is the same for every Hebnix build (baked in at build time),
so it only keeps casual scrapers out; matches are protected by their in-game
LAN password, as with the old headscale setup.

Configuration (environment):
    HEBNIX_DIRECTORY_KEY   shared key (required)
    HEBNIX_DIRECTORY_PORT  listen port on 127.0.0.1 (default 8087)
"""

import json
import os
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

KEY = os.environ.get("HEBNIX_DIRECTORY_KEY", "")
LISTEN = ("127.0.0.1", int(os.environ.get("HEBNIX_DIRECTORY_PORT", "8087")))
TTL = 60
MAX_ENTRIES = 1000
# per client IP, announces per minute. A client sends 3, and players behind
# one NAT (a LAN party) share an IP
RATE_LIMIT = 60
MAX_BODY = 2048

ID_RE = re.compile(r"^[0-9a-f]{16}$")
TOKEN_RE = re.compile(r"^tc[A-Za-z0-9+/=_.:-]{20,1000}$")

_entries = {}  # id -> (expires_at, entry)
_recent = {}  # client ip -> [monotonic stamps]
_lock = threading.Lock()


def allowed(client, now):
    stamps = [t for t in _recent.get(client, []) if now - t < 60]
    if len(stamps) >= RATE_LIMIT:
        _recent[client] = stamps
        return False
    stamps.append(now)
    _recent[client] = stamps
    return True


def clean_name(value):
    if not isinstance(value, str):
        return ""
    value = "".join(c for c in value if c.isprintable()).strip()
    return value[:64]


def announce(body, client, *, departing=False):
    """returns (status, reply)"""
    try:
        data = json.loads(body)
    except ValueError:
        return 400, {"error": "bad json"}
    if not isinstance(data, dict):
        return 400, {"error": "bad request"}
    entry_id, token = data.get("id"), data.get("token")
    if not isinstance(entry_id, str) or not ID_RE.match(entry_id):
        return 400, {"error": "bad id"}
    if not isinstance(token, str) or not TOKEN_RE.match(token):
        return 400, {"error": "bad token"}
    entry = {"id": entry_id, "token": token, "name": clean_name(data.get("name"))}
    now = time.monotonic()
    with _lock:
        if not allowed(client, now):
            return 429, {"error": "too many requests, try again in a minute"}
        for key in [k for k, (expires, _) in _entries.items() if expires < now]:
            del _entries[key]
        for ip in [ip for ip, stamps in _recent.items() if not stamps or now - stamps[-1] > 60]:
            del _recent[ip]
        if departing:
            current = _entries.get(entry_id)
            if current and current[1]["token"] == token:
                del _entries[entry_id]
            return 200, {}
        if entry_id not in _entries and len(_entries) >= MAX_ENTRIES:
            return 503, {"error": "the directory is full"}
        _entries[entry_id] = (now + TTL, entry)
        peers = [e for _, e in _entries.values()]
    return 200, {"peers": peers}


class Handler(BaseHTTPRequestHandler):
    server_version = "HebnixDirectory/1"

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        self.update_directory(departing=False)

    def do_DELETE(self):
        self.update_directory(departing=True)

    def update_directory(self, *, departing):
        if self.headers.get("X-Hebnix-Key", "") != KEY:
            return self.reply(403, {"error": "forbidden"})
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0 or length > MAX_BODY:
            return self.reply(400, {"error": "bad request"})
        body = self.rfile.read(length)
        client = (self.headers.get("X-Forwarded-For") or self.client_address[0]).split(",")[0].strip()
        status, reply = announce(body, client, departing=departing)
        self.reply(status, reply)

    def do_GET(self):
        self.reply(404, {"error": "unknown request"})

    def log_message(self, fmt, *args):
        # one line per request is noise at 3 requests a minute per player;
        # only errors are logged (log_error still goes through here)
        if args and isinstance(args[0], str) and args[0].startswith('"POST'):
            return
        super().log_message(fmt, *args)


if __name__ == "__main__":
    if not KEY:
        sys.exit("set HEBNIX_DIRECTORY_KEY")
    ThreadingHTTPServer(LISTEN, Handler).serve_forever()
