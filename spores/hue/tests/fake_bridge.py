#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""A deterministic Philips Hue v1 bridge, for the socket tests.

It answers the handful of routes `lib/hue.soil` uses, keeps a little state so
`toggle` has something to invert, and refuses any path whose credential segment
is not the key the shell leased to the host. That refusal is the point: the
tests pass a fake key to `soil-lang --http-credential`, and the bridge only
sees it if the host substituted it for `__CREDENTIAL__`.

Every request is appended to `$HUE_LOG` as `METHOD PATH BODY`, so a test can
assert what crossed the wire.

    python3 fake_bridge.py            # prints "port <n>", then serves
"""

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

KEY = "test-key"
LOG = os.environ.get("HUE_LOG")
PORT = 0

STATE = {"on": False, "bri": 0}
CONFIG = {
    "name": "Fake Bridge",
    "lights": {"1": {"name": "Desk"}, "3": {"name": "Window Lamp"}},
    "groups": {"0": {"name": "All"}, "1": {"name": "Living Room"}},
}
ALL = {"name": "All", "lights": ["1", "3"], "action": STATE}
LIGHTS = {
    "1": {"name": "Desk", "state": {"on": False, "bri": 0}},
    "3": {"name": "Window Lamp", "state": STATE},
}
GROUP = {"name": "Living Room", "lights": ["3"], "action": STATE}
SCENES = {"7": {"name": "Relax", "group": "1"}, "8": {"name": "Focus", "group": "1"}}


def record(method, path, body):
    if not LOG:
        return
    with open(LOG, "a", encoding="utf-8") as handle:
        handle.write(f"{method} {path} {json.dumps(body) if body is not None else '-'}\n")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send_json(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def route(self):
        """Return (path after /api, credential segment), or refuse."""
        path = self.path
        if not path.startswith("/api"):
            self.send_json(404, [{"error": {"description": "not found"}}])
            return None
        rest = path[len("/api"):]
        if rest == "" or rest == "/":
            return ("", None)
        parts = rest.lstrip("/").split("/", 1)
        credential = parts[0]
        if credential != KEY:
            self.send_json(
                403,
                [{"error": {"description": f"unknown application key {credential}"}}],
            )
            return None
        return (("/" + parts[1]) if len(parts) > 1 else "", credential)

    def do_GET(self):
        record("GET", self.path, None)
        route = self.route()
        if route is None:
            return
        path, credential = route
        if credential is None:
            self.send_json(404, [{"error": {"description": "not found"}}])
        elif path == "":
            self.send_json(200, dict(CONFIG, config={"name": CONFIG["name"]}))
        elif path == "/lights":
            self.send_json(200, LIGHTS)
        elif path == "/lights/3":
            self.send_json(200, LIGHTS["3"])
        elif path == "/groups/0":
            self.send_json(200, ALL)
        elif path == "/groups/1":
            self.send_json(200, GROUP)
        elif path == "/scenes":
            self.send_json(200, SCENES)
        else:
            self.send_json(200, [{"error": {"description": f"no route {path}"}}])

    def do_PUT(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        record("PUT", self.path, body)
        route = self.route()
        if route is None:
            return
        path, _ = route
        if path == "/lights/999/state":
            self.send_json(
                200,
                [{"error": {"description": "invalid value, 999, for parameter, light"}}],
            )
            return
        if "on" in body:
            STATE["on"] = body["on"]
        if "bri" in body:
            if not isinstance(body["bri"], int):
                self.send_json(
                    200,
                    [{"error": {"description": "invalid value, for parameter, bri"}}],
                )
                return
            STATE["bri"] = body["bri"]
        if "xy" in body:
            STATE["colormode"] = "xy"
            STATE["xy"] = body["xy"]
        self.send_json(200, [{"success": {path: body}}])

    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        record("POST", self.path, body)
        if self.path.rstrip("/") == "/api":
            self.send_json(200, [{"success": {"username": KEY}}])
            return
        self.send_json(404, [{"error": {"description": "not found"}}])


def main():
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    print(f"port {server.server_address[1]}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    sys.exit(main())
