#!/usr/bin/env python3
"""Loopback-only HTTP fixture for the manual collision/recovery lab.

This deliberately serves a wrong ETag on range responses in bad mode. Never
expose it to a network or use it to serve real files.
"""
import argparse
import http.server
import re
import socketserver
import threading
import time
import urllib.parse

TOTAL = 32 * 1024 * 1024
CHUNK = 64 * 1024
PATTERN = bytes(range(256)) * 257
ETAG = '"rget-lab-v1"'


class State:
    def __init__(self):
        self.bad = False
        self.lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    state = State()

    def do_GET(self):
        route = urllib.parse.urlsplit(self.path).path
        if route in ("/__mode/good", "/__mode/bad"):
            with self.state.lock:
                self.state.bad = route.endswith("/bad")
            body = ("Range validator mode: " + ("bad" if self.state.bad else "good") + "\n").encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
            return
        names = {"/resume": "resume.bin", "/corrupt": "corrupt.bin", "/collision": "collision.bin"}
        if route not in names:
            self.send_error(404)
            return
        start, end = 0, TOTAL - 1
        range_header = self.headers.get("Range")
        if range_header:
            match = re.fullmatch(r"bytes=(\d+)-(\d*)", range_header)
            if not match:
                self.send_error(416)
                return
            start = int(match.group(1))
            end = min(int(match.group(2)) if match.group(2) else TOTAL - 1, TOTAL - 1)
            if start > end or start >= TOTAL:
                self.send_error(416)
                return
        with self.state.lock:
            bad = self.state.bad
        etag = '"rget-lab-invalid"' if range_header and bad else ETAG
        print(f"{self.path} range={range_header!r} if-range={self.headers.get('If-Range')!r} etag={etag}", flush=True)
        self.send_response(206 if range_header else 200)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", etag)
        self.send_header("Content-Disposition", f'attachment; filename="{names[route]}"')
        self.send_header("Connection", "close")
        if range_header:
            self.send_header("Content-Range", f"bytes {start}-{end}/{TOTAL}")
        self.end_headers()
        try:
            pos = start
            while pos <= end:
                size = min(CHUNK, end - pos + 1)
                offset = pos % 256
                self.wfile.write(PATTERN[offset:offset + size])
                pos += size
                if end > start:
                    time.sleep(0.04)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

    def log_message(self, _format, *_args):
        pass


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=18765)
    args = parser.parse_args()
    with Server(("127.0.0.1", args.port), Handler) as server:
        print(f"Collision lab listening on http://127.0.0.1:{server.server_port}", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
