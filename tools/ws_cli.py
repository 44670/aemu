#!/usr/bin/env python3
import argparse
import base64
import hashlib
import json
import os
import re
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


class JournalStep:
    def __init__(self, action, **kwargs):
        self.action = action
        self.kwargs = kwargs

    def as_json(self):
        value = {"action": self.action}
        value.update(self.kwargs)
        return value


def parse_xy(text):
    parts = [part for part in re.split(r"[,\s]+", text.strip()) if part]
    if len(parts) != 2:
        raise ValueError(f"expected x,y coordinate, got {text!r}")
    return float(parts[0]), float(parts[1])


def parse_journal_step(text):
    words = text.split(maxsplit=1)
    if not words:
        return None
    op = words[0].lower()
    rest = words[1].strip() if len(words) > 1 else ""
    if op in ("tap", "touch"):
        x, y = parse_xy(rest)
        return JournalStep("tap", x=x, y=y)
    if op == "pointer":
        parts = rest.split(maxsplit=1)
        if len(parts) != 2:
            raise ValueError(f"pointer expects: pointer PHASE x,y, got {text!r}")
        phase = parts[0].lower()
        if phase not in ("down", "move", "up"):
            raise ValueError(f"pointer phase must be down, move, or up, got {phase!r}")
        x, y = parse_xy(parts[1])
        return JournalStep("pointer", phase=phase, x=x, y=y)
    if op == "wait":
        if not rest:
            raise ValueError("wait expects seconds")
        return JournalStep("wait", seconds=float(rest))
    if op == "screenshot":
        if not rest:
            raise ValueError("screenshot expects output path")
        return JournalStep("screenshot", out=rest)
    if op == "debug":
        if rest:
            raise ValueError(f"debug takes no arguments, got {rest!r}")
        return JournalStep("debug")
    raise ValueError(f"unknown journal action {op!r}")


def parse_journal_text(text):
    steps = []
    for line in text.splitlines():
        line = line.split("#", 1)[0]
        for part in line.split(";"):
            part = part.strip()
            if not part:
                continue
            steps.append(parse_journal_step(part))
    return steps


def cmd_debug(client, _args):
    print_json(client.request({"cmd": "debug"}))


def save_screenshot(client, out):
    response = client.request({"cmd": "screenshot"})
    if not response.get("ok"):
        return response, 0
    data = base64.b64decode(response["data_base64"])
    with open(out, "wb") as file:
        file.write(data)
    return (
        {
            "ok": True,
            "path": out,
            "format": response.get("format", "png"),
            "width": response.get("width"),
            "height": response.get("height"),
            "bytes": len(data),
        },
        len(data),
    )


def cmd_screenshot(client, args):
    response, _size = save_screenshot(client, args.out)
    print_json(response)
    if not response.get("ok"):
        return 1
    return 0


def make_pointer_payload(phase, x, y, pointer_id=0, pressure=1.0):
    return {
        "cmd": "pointer",
        "phase": phase,
        "id": pointer_id,
        "x": x,
        "y": y,
        "pressure": pressure,
    }


def pointer_payload(args, phase):
    return make_pointer_payload(phase, args.x, args.y, args.id, args.pressure)


def cmd_pointer(client, args):
    print_json(client.request(pointer_payload(args, args.phase)))
    return 0


def send_tap(client, x, y, pointer_id=0, pressure=1.0, duration_ms=50.0):
    down = make_pointer_payload("down", x, y, pointer_id, pressure)
    down_response = client.request(down)
    time.sleep(duration_ms / 1000.0)
    up = make_pointer_payload("up", x, y, pointer_id, pressure)
    up["pressure"] = 0.0
    up_response = client.request(up)
    return down_response, up_response


def cmd_tap(client, args):
    down_response, up_response = send_tap(
        client, args.x, args.y, args.id, args.pressure, args.duration_ms
    )
    print_json(down_response)
    print_json(up_response)
    return 0


def load_journal(args):
    chunks = []
    if args.file:
        with open(args.file, "r", encoding="utf-8") as file:
            chunks.append(file.read())
    if args.script:
        chunks.append(args.script)
    if not chunks:
        raise ValueError("journal expects a script argument or --file")
    return parse_journal_text("\n".join(chunks))


def cmd_journal(client, args):
    try:
        steps = load_journal(args)
    except ValueError as err:
        print_json({"ok": False, "error": str(err)})
        return 2
    print_json({"ok": True, "steps": [step.as_json() for step in steps]})
    for index, step in enumerate(steps, start=1):
        try:
            if step.action == "tap":
                down, up = send_tap(
                    client,
                    step.kwargs["x"],
                    step.kwargs["y"],
                    args.id,
                    args.pressure,
                    args.duration_ms,
                )
                result = {"ok": bool(down.get("ok") and up.get("ok")), "down": down, "up": up}
            elif step.action == "pointer":
                pressure = 0.0 if step.kwargs["phase"] == "up" else args.pressure
                result = client.request(
                    make_pointer_payload(
                        step.kwargs["phase"],
                        step.kwargs["x"],
                        step.kwargs["y"],
                        args.id,
                        pressure,
                    )
                )
            elif step.action == "wait":
                time.sleep(step.kwargs["seconds"])
                result = {"ok": True}
            elif step.action == "screenshot":
                result, _size = save_screenshot(client, step.kwargs["out"])
            elif step.action == "debug":
                result = client.request({"cmd": "debug"})
            else:
                result = {"ok": False, "error": f"unhandled action {step.action!r}"}
        except Exception as err:
            print_json(
                {
                    "ok": False,
                    "step": index,
                    "action": step.as_json(),
                    "error": str(err),
                }
            )
            return 1
        print_json(
            {
                "ok": bool(result.get("ok")),
                "step": index,
                "action": step.as_json(),
                "result": result,
            }
        )
        if not result.get("ok"):
            return 1
    return 0


def build_parser():
    parser = argparse.ArgumentParser(description="aemu SDL2 WebSocket harness client")
    parser.add_argument("--url", default="ws://127.0.0.1:8766", help="WebSocket URL")
    sub = parser.add_subparsers(dest="cmd", required=True)

    debug = sub.add_parser("debug", help="print emulator/debug state")
    debug.set_defaults(func=cmd_debug)

    screenshot = sub.add_parser("screenshot", help="save framebuffer screenshot as PNG")
    screenshot.add_argument("--out", default="tmp/aemu-ws-screenshot.png")
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

    journal = sub.add_parser("journal", help="run semicolon/newline separated actions")
    journal.add_argument(
        "script",
        nargs="?",
        help="actions, for example: touch 280,386; wait 1; screenshot tmp/out.png",
    )
    journal.add_argument("--file", help="read actions from a script file")
    journal.add_argument("--id", type=int, default=0)
    journal.add_argument("--pressure", type=float, default=1.0)
    journal.add_argument("--duration-ms", type=float, default=50.0)
    journal.set_defaults(func=cmd_journal)

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
