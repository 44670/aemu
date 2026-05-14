#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import time
import urllib.parse


GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class WsClient:
    def __init__(self, url):
        parsed = urllib.parse.urlparse(url)
        if parsed.scheme != "ws":
            raise ValueError("only ws:// URLs are supported")
        self.host = parsed.hostname or "127.0.0.1"
        self.port = parsed.port or 80
        self.path = parsed.path or "/"
        if parsed.query:
            self.path += "?" + parsed.query
        self.sock = socket.create_connection((self.host, self.port), timeout=10)
        self._handshake()

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass

    def request(self, payload):
        self._write_text(json.dumps(payload, separators=(",", ":")))
        return json.loads(self._read_text())

    def _handshake(self):
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        request = (
            f"GET {self.path} HTTP/1.1\r\n"
            f"Host: {self.host}:{self.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(request.encode("ascii"))
        response = b""
        while b"\r\n\r\n" not in response:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("websocket handshake closed")
            response += chunk
            if len(response) > 8192:
                raise RuntimeError("websocket handshake response too large")
        head = response.decode("iso-8859-1", errors="replace")
        if " 101 " not in head.split("\r\n", 1)[0]:
            raise RuntimeError(f"websocket upgrade failed: {head.splitlines()[0]}")
        expected = base64.b64encode(hashlib.sha1((key + GUID).encode("ascii")).digest()).decode(
            "ascii"
        )
        if expected.lower() not in head.lower():
            raise RuntimeError("websocket accept key mismatch")

    def _write_text(self, text):
        payload = text.encode("utf-8")
        header = bytearray([0x81])
        if len(payload) < 126:
            header.append(0x80 | len(payload))
        elif len(payload) <= 0xFFFF:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", len(payload)))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", len(payload)))
        mask = os.urandom(4)
        header.extend(mask)
        masked = bytes(byte ^ mask[idx % 4] for idx, byte in enumerate(payload))
        self.sock.sendall(header + masked)

    def _read_text(self):
        while True:
            first, second = self._read_exact(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read_exact(8))[0]
            payload = self._read_exact(length)
            if opcode == 0x1:
                return payload.decode("utf-8")
            if opcode == 0x8:
                raise RuntimeError("websocket closed")

    def _read_exact(self, size):
        out = bytearray()
        while len(out) < size:
            chunk = self.sock.recv(size - len(out))
            if not chunk:
                raise RuntimeError("websocket closed")
            out.extend(chunk)
        return bytes(out)


def print_json(value):
    print(json.dumps(value, indent=2, sort_keys=True))


def cmd_debug(client, _args):
    print_json(client.request({"cmd": "debug"}))


def cmd_screenshot(client, args):
    response = client.request({"cmd": "screenshot"})
    if not response.get("ok"):
        print_json(response)
        return 1
    data = base64.b64decode(response["data_base64"])
    with open(args.out, "wb") as file:
        file.write(data)
    print_json(
        {
            "ok": True,
            "path": args.out,
            "format": response.get("format", "ppm"),
            "width": response.get("width"),
            "height": response.get("height"),
            "bytes": len(data),
        }
    )
    return 0


def pointer_payload(args, phase):
    return {
        "cmd": "pointer",
        "phase": phase,
        "id": args.id,
        "x": args.x,
        "y": args.y,
        "pressure": args.pressure,
    }


def cmd_pointer(client, args):
    print_json(client.request(pointer_payload(args, args.phase)))
    return 0


def cmd_tap(client, args):
    print_json(client.request(pointer_payload(args, "down")))
    time.sleep(args.duration_ms / 1000.0)
    up = pointer_payload(args, "up")
    up["pressure"] = 0.0
    print_json(client.request(up))
    return 0


def build_parser():
    parser = argparse.ArgumentParser(description="aemu SDL2 WebSocket harness client")
    parser.add_argument("--url", default="ws://127.0.0.1:8766", help="WebSocket URL")
    sub = parser.add_subparsers(dest="cmd", required=True)

    debug = sub.add_parser("debug", help="print emulator/debug state")
    debug.set_defaults(func=cmd_debug)

    screenshot = sub.add_parser("screenshot", help="save framebuffer screenshot as PPM")
    screenshot.add_argument("--out", default="target/aemu-ws-screenshot.ppm")
    screenshot.set_defaults(func=cmd_screenshot)

    pointer = sub.add_parser("pointer", help="send one pointer event")
    pointer.add_argument("phase", choices=["down", "move", "up"])
    pointer.add_argument("x", type=float)
    pointer.add_argument("y", type=float)
    pointer.add_argument("--id", type=int, default=0)
    pointer.add_argument("--pressure", type=float, default=1.0)
    pointer.set_defaults(func=cmd_pointer)

    tap = sub.add_parser("tap", help="send pointer down/up at a coordinate")
    tap.add_argument("x", type=float)
    tap.add_argument("y", type=float)
    tap.add_argument("--id", type=int, default=0)
    tap.add_argument("--pressure", type=float, default=1.0)
    tap.add_argument("--duration-ms", type=float, default=50.0)
    tap.set_defaults(func=cmd_tap)

    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    client = WsClient(args.url)
    try:
        return args.func(client, args) or 0
    finally:
        client.close()


if __name__ == "__main__":
    raise SystemExit(main())
