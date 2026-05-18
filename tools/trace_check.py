#!/usr/bin/env python3
import argparse
import dataclasses
import hashlib
import json
import pathlib
import re
import sys
import tempfile
import zlib


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"

GL_ALPHA = 0x1906
GL_RGB = 0x1907
GL_RGBA = 0x1908
GL_LUMINANCE = 0x1909
GL_LUMINANCE_ALPHA = 0x190A
GL_UNSIGNED_BYTE = 0x1401
GL_UNSIGNED_SHORT_4_4_4_4 = 0x8033
GL_UNSIGNED_SHORT_5_5_5_1 = 0x8034
GL_UNSIGNED_SHORT_5_6_5 = 0x8363
GL_BGRA_EXT = 0x80E1


UPLOAD_RE = re.compile(
    r"^(?P<index>\d+)-"
    r"(?:(?:event(?P<event>\d+)-))?"
    r"(?P<kind>teximage2d|texsubimage2d)-"
    r"tex(?P<texture>\d+)-"
    r"(?P<width>\d+)x(?P<height>\d+)-"
    r"fmt(?P<format>[0-9a-fA-F]+)-"
    r"ty(?P<type>[0-9a-fA-F]+)$"
)

DRAW_RE = re.compile(
    r"^(?P<index>\d+)-"
    r"event(?P<event>\d+)-"
    r"draw(?P<draw>\d+)-"
    r"(?P<kind>[A-Za-z]+)-"
    r"program(?P<program>\d+)-"
    r"tex(?P<texture>\d+)-"
    r"(?P<width>\d+)x(?P<height>\d+)$"
)


class TraceError(RuntimeError):
    pass


@dataclasses.dataclass
class PngInfo:
    path: pathlib.Path
    width: int
    height: int
    color_type: int
    rgb: bytes
    alpha: bytes | None

    @property
    def nonzero_rgb_pixels(self) -> int:
        return sum(
            1
            for offset in range(0, len(self.rgb), 3)
            if self.rgb[offset] or self.rgb[offset + 1] or self.rgb[offset + 2]
        )

    @property
    def nonzero_alpha_pixels(self) -> int:
        if self.alpha is None:
            return self.width * self.height
        return sum(1 for value in self.alpha if value)

    @property
    def rgb_sha1(self) -> str:
        return hashlib.sha1(self.rgb).hexdigest()


@dataclasses.dataclass
class UploadArtifact:
    side: str
    index: int
    event_index: int | None
    kind: str
    texture: int
    width: int
    height: int
    format: int
    ty: int
    png: pathlib.Path
    raw: pathlib.Path
    png_info: PngInfo
    raw_sha1: str
    raw_len: int
    raw_nonzero_rgb_pixels: int | None
    raw_nonzero_alpha_pixels: int | None

    @property
    def pair_key(self) -> tuple[str, int, int, int, int, int]:
        return (self.kind, self.texture, self.width, self.height, self.format, self.ty)


@dataclasses.dataclass
class DrawArtifact:
    index: int
    event_index: int
    draw: int
    kind: str
    program: int
    texture: int
    width: int
    height: int
    png: pathlib.Path
    png_info: PngInfo


def parse_png(path: pathlib.Path) -> PngInfo:
    data = path.read_bytes()
    if not data.startswith(PNG_SIGNATURE):
        raise TraceError(f"{path}: not a PNG file")
    offset = len(PNG_SIGNATURE)
    width = height = color_type = None
    idat = bytearray()
    saw_iend = False
    while offset + 12 <= len(data):
        length = int.from_bytes(data[offset : offset + 4], "big")
        kind = data[offset + 4 : offset + 8]
        payload_start = offset + 8
        payload_end = payload_start + length
        crc_end = payload_end + 4
        if crc_end > len(data):
            raise TraceError(f"{path}: truncated PNG chunk {kind!r}")
        payload = data[payload_start:payload_end]
        stored_crc = int.from_bytes(data[payload_end:crc_end], "big")
        actual_crc = zlib.crc32(payload, zlib.crc32(kind)) & 0xFFFFFFFF
        if actual_crc != stored_crc:
            raise TraceError(
                f"{path}: CRC mismatch in PNG chunk {kind.decode('ascii', errors='replace')}"
            )
        offset = crc_end
        if kind == b"IHDR":
            if length != 13:
                raise TraceError(f"{path}: invalid IHDR length {length}")
            width = int.from_bytes(payload[0:4], "big")
            height = int.from_bytes(payload[4:8], "big")
            bit_depth = payload[8]
            color_type = payload[9]
            compression = payload[10]
            filter_method = payload[11]
            interlace = payload[12]
            if bit_depth != 8:
                raise TraceError(f"{path}: unsupported PNG bit depth {bit_depth}")
            if color_type not in (2, 6):
                raise TraceError(f"{path}: unsupported PNG color type {color_type}")
            if compression != 0 or filter_method != 0 or interlace != 0:
                raise TraceError(f"{path}: unsupported PNG header options")
        elif kind == b"IDAT":
            idat.extend(payload)
        elif kind == b"IEND":
            saw_iend = True
            break
    if width is None or height is None or color_type is None:
        raise TraceError(f"{path}: missing IHDR")
    if not saw_iend:
        raise TraceError(f"{path}: missing IEND")
    if width <= 0 or height <= 0:
        raise TraceError(f"{path}: invalid dimensions {width}x{height}")
    channels = 3 if color_type == 2 else 4
    bpp = channels
    row_bytes = width * channels
    try:
        filtered = zlib.decompress(bytes(idat))
    except zlib.error as err:
        raise TraceError(f"{path}: IDAT zlib decode failed: {err}") from err
    expected = height * (1 + row_bytes)
    if len(filtered) != expected:
        raise TraceError(
            f"{path}: decoded length {len(filtered)} does not match expected {expected}"
        )
    rows = []
    prev = bytes(row_bytes)
    pos = 0
    for _row in range(height):
        filter_type = filtered[pos]
        pos += 1
        row = bytearray(filtered[pos : pos + row_bytes])
        pos += row_bytes
        unfilter_row(filter_type, row, prev, bpp)
        rows.append(bytes(row))
        prev = bytes(row)
    rgb = bytearray()
    alpha = bytearray() if color_type == 6 else None
    for row in rows:
        if color_type == 2:
            rgb.extend(row)
            continue
        for offset in range(0, len(row), 4):
            rgb.extend(row[offset : offset + 3])
            alpha.append(row[offset + 3])
    return PngInfo(path, width, height, color_type, bytes(rgb), bytes(alpha) if alpha else None)


def unfilter_row(filter_type: int, row: bytearray, prev: bytes, bpp: int) -> None:
    if filter_type == 0:
        return
    if filter_type == 1:
        for idx in range(len(row)):
            left = row[idx - bpp] if idx >= bpp else 0
            row[idx] = (row[idx] + left) & 0xFF
        return
    if filter_type == 2:
        for idx in range(len(row)):
            row[idx] = (row[idx] + prev[idx]) & 0xFF
        return
    if filter_type == 3:
        for idx in range(len(row)):
            left = row[idx - bpp] if idx >= bpp else 0
            up = prev[idx]
            row[idx] = (row[idx] + ((left + up) >> 1)) & 0xFF
        return
    if filter_type == 4:
        for idx in range(len(row)):
            left = row[idx - bpp] if idx >= bpp else 0
            up = prev[idx]
            up_left = prev[idx - bpp] if idx >= bpp else 0
            row[idx] = (row[idx] + paeth(left, up, up_left)) & 0xFF
        return
    raise TraceError(f"unsupported PNG filter type {filter_type}")


def paeth(left: int, up: int, up_left: int) -> int:
    p = left + up - up_left
    pa = abs(p - left)
    pb = abs(p - up)
    pc = abs(p - up_left)
    if pa <= pb and pa <= pc:
        return left
    if pb <= pc:
        return up
    return up_left


def parse_upload_png(side: str, path: pathlib.Path, min_nonzero_rgb: int) -> UploadArtifact:
    match = UPLOAD_RE.match(path.stem)
    if not match:
        raise TraceError(f"{path}: upload filename does not match trace pattern")
    raw_path = path.with_suffix(".raw")
    if not raw_path.exists():
        raise TraceError(f"{path}: missing matching raw payload {raw_path.name}")
    info = parse_png(path)
    width = int(match.group("width"))
    height = int(match.group("height"))
    if info.width != width or info.height != height:
        raise TraceError(
            f"{path}: PNG dimensions {info.width}x{info.height} do not match filename {width}x{height}"
        )
    if info.nonzero_rgb_pixels < min_nonzero_rgb:
        raise TraceError(
            f"{path}: only {info.nonzero_rgb_pixels} nonzero RGB pixels, expected at least {min_nonzero_rgb}"
        )
    fmt = int(match.group("format"), 16)
    ty = int(match.group("type"), 16)
    raw = raw_path.read_bytes()
    expected_len = upload_payload_len(width, height, fmt, ty)
    if expected_len is not None and len(raw) != expected_len:
        raise TraceError(
            f"{raw_path}: raw length {len(raw)} does not match expected {expected_len}"
        )
    raw_stats = raw_payload_stats(width, height, fmt, ty, raw)
    return UploadArtifact(
        side=side,
        index=int(match.group("index")),
        event_index=int(match.group("event")) if match.group("event") is not None else None,
        kind=match.group("kind"),
        texture=int(match.group("texture")),
        width=width,
        height=height,
        format=fmt,
        ty=ty,
        png=path,
        raw=raw_path,
        png_info=info,
        raw_sha1=hashlib.sha1(raw).hexdigest(),
        raw_len=len(raw),
        raw_nonzero_rgb_pixels=raw_stats[0] if raw_stats is not None else None,
        raw_nonzero_alpha_pixels=raw_stats[1] if raw_stats is not None else None,
    )


def parse_draw_png(path: pathlib.Path, min_nonzero_rgb: int) -> DrawArtifact:
    match = DRAW_RE.match(path.stem)
    if not match:
        raise TraceError(f"{path}: draw filename does not match trace pattern")
    info = parse_png(path)
    width = int(match.group("width"))
    height = int(match.group("height"))
    if info.width != width or info.height != height:
        raise TraceError(
            f"{path}: PNG dimensions {info.width}x{info.height} do not match filename {width}x{height}"
        )
    if info.nonzero_rgb_pixels < min_nonzero_rgb:
        raise TraceError(
            f"{path}: only {info.nonzero_rgb_pixels} nonzero RGB pixels, expected at least {min_nonzero_rgb}"
        )
    return DrawArtifact(
        index=int(match.group("index")),
        event_index=int(match.group("event")),
        draw=int(match.group("draw")),
        kind=match.group("kind"),
        program=int(match.group("program")),
        texture=int(match.group("texture")),
        width=width,
        height=height,
        png=path,
        png_info=info,
    )


def upload_payload_len(width: int, height: int, fmt: int, ty: int) -> int | None:
    pixels = width * height
    if ty in (
        GL_UNSIGNED_SHORT_4_4_4_4,
        GL_UNSIGNED_SHORT_5_5_5_1,
        GL_UNSIGNED_SHORT_5_6_5,
    ):
        return pixels * 2
    components = {
        GL_ALPHA: 1,
        GL_LUMINANCE: 1,
        GL_LUMINANCE_ALPHA: 2,
        GL_RGB: 3,
        GL_RGBA: 4,
        GL_BGRA_EXT: 4,
    }.get(fmt)
    if components is None:
        return None
    elem_size = {GL_UNSIGNED_BYTE: 1}.get(ty)
    if elem_size is None:
        return None
    return pixels * components * elem_size


def raw_payload_stats(
    width: int, height: int, fmt: int, ty: int, payload: bytes
) -> tuple[int, int] | None:
    pixels = width * height
    nonzero_rgb = 0
    nonzero_alpha = 0
    if (fmt, ty) in ((GL_RGBA, GL_UNSIGNED_BYTE), (GL_BGRA_EXT, GL_UNSIGNED_BYTE)):
        for offset in range(0, min(len(payload), pixels * 4), 4):
            if payload[offset] or payload[offset + 1] or payload[offset + 2]:
                nonzero_rgb += 1
            if payload[offset + 3]:
                nonzero_alpha += 1
        return nonzero_rgb, nonzero_alpha
    if (fmt, ty) == (GL_RGB, GL_UNSIGNED_BYTE):
        for offset in range(0, min(len(payload), pixels * 3), 3):
            if payload[offset] or payload[offset + 1] or payload[offset + 2]:
                nonzero_rgb += 1
        return nonzero_rgb, pixels
    if (fmt, ty) == (GL_ALPHA, GL_UNSIGNED_BYTE):
        return 0, sum(1 for value in payload[:pixels] if value)
    if (fmt, ty) == (GL_LUMINANCE, GL_UNSIGNED_BYTE):
        nonzero_rgb = sum(1 for value in payload[:pixels] if value)
        return nonzero_rgb, pixels
    if (fmt, ty) == (GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE):
        for offset in range(0, min(len(payload), pixels * 2), 2):
            if payload[offset]:
                nonzero_rgb += 1
            if payload[offset + 1]:
                nonzero_alpha += 1
        return nonzero_rgb, nonzero_alpha
    if (fmt, ty) == (GL_RGB, GL_UNSIGNED_SHORT_5_6_5):
        for offset in range(0, min(len(payload), pixels * 2), 2):
            if int.from_bytes(payload[offset : offset + 2], "little"):
                nonzero_rgb += 1
        return nonzero_rgb, pixels
    if (fmt, ty) == (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4):
        for offset in range(0, min(len(payload), pixels * 2), 2):
            value = int.from_bytes(payload[offset : offset + 2], "little")
            if value & 0x0FFF:
                nonzero_rgb += 1
            if value & 0xF000:
                nonzero_alpha += 1
        return nonzero_rgb, nonzero_alpha
    if (fmt, ty) == (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1):
        for offset in range(0, min(len(payload), pixels * 2), 2):
            value = int.from_bytes(payload[offset : offset + 2], "little")
            if value & 0xFFFE:
                nonzero_rgb += 1
            if value & 0x0001:
                nonzero_alpha += 1
        return nonzero_rgb, nonzero_alpha
    return None


def read_manifest(path: pathlib.Path) -> list[dict]:
    if not path.exists():
        return []
    out = []
    for line_no, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError as err:
            raise TraceError(f"{path}:{line_no}: invalid JSON: {err}") from err
    return out


def validate_manifest(hle: list[UploadArtifact], manifest: list[dict], manifest_path: pathlib.Path):
    by_index = {item.index: item for item in hle}
    for row in manifest:
        index = row.get("index")
        item = by_index.get(index)
        if item is None:
            raise TraceError(f"{manifest_path}: row index {index!r} has no matching HLE PNG")
        checks = {
            "kind": item.kind,
            "texture": item.texture,
            "event_index": item.event_index,
            "width": item.width,
            "height": item.height,
            "format": item.format,
            "type": item.ty,
            "payload_len": item.raw_len,
        }
        if item.raw_nonzero_rgb_pixels is not None:
            checks["nonzero_rgb_pixels"] = item.raw_nonzero_rgb_pixels
        if item.raw_nonzero_alpha_pixels is not None:
            checks["nonzero_alpha_pixels"] = item.raw_nonzero_alpha_pixels
        for key, expected in checks.items():
            if row.get(key) != expected:
                raise TraceError(
                    f"{manifest_path}: row {index} {key}={row.get(key)!r}, expected {expected!r}"
                )


def validate_draw_manifest(
    draws: list[DrawArtifact], manifest: list[dict], manifest_path: pathlib.Path
) -> None:
    by_index = {item.index: item for item in draws}
    for row in manifest:
        index = row.get("index")
        item = by_index.get(index)
        if item is None:
            raise TraceError(f"{manifest_path}: row index {index!r} has no matching draw PNG")
        checks = {
            "event_index": item.event_index,
            "draw": item.draw,
            "kind": item.kind,
            "program": item.program,
            "texture": item.texture,
            "width": item.width,
            "height": item.height,
            "png": item.png.name,
        }
        for key, expected in checks.items():
            if row.get(key) != expected:
                raise TraceError(
                    f"{manifest_path}: row {index} {key}={row.get(key)!r}, expected {expected!r}"
                )


def parse_size(raw: str) -> tuple[int, int]:
    match = re.fullmatch(r"(?P<width>\d+)x(?P<height>\d+)", raw.strip())
    if not match:
        raise argparse.ArgumentTypeError(f"expected WxH size, got {raw!r}")
    width = int(match.group("width"))
    height = int(match.group("height"))
    if width <= 0 or height <= 0:
        raise argparse.ArgumentTypeError(f"size must be positive, got {raw!r}")
    return width, height


def parse_program_size(raw: str) -> tuple[int, int, int]:
    program_raw, sep, size_raw = raw.strip().partition(":")
    if not sep:
        raise argparse.ArgumentTypeError(f"expected PROGRAM:WxH, got {raw!r}")
    try:
        program = int(program_raw, 0)
    except ValueError as err:
        raise argparse.ArgumentTypeError(f"invalid program id {program_raw!r}") from err
    width, height = parse_size(size_raw)
    return program, width, height


def draw_texture_size(row: dict) -> tuple[int, int] | None:
    width = row.get("texture_width")
    height = row.get("texture_height")
    if isinstance(width, int) and isinstance(height, int) and width > 0 and height > 0:
        return width, height
    return None


def validate_draw_program_gates(
    manifest: list[dict],
    manifest_path: pathlib.Path,
    expect_programs: list[int],
    require_changed_programs: list[int],
    require_sizes: list[tuple[int, int, int]],
    reject_sizes: list[tuple[int, int, int]],
) -> None:
    for program in expect_programs:
        if not any(row.get("program") == program for row in manifest):
            raise TraceError(f"{manifest_path}: no draw manifest rows for program{program}")

    for program in require_changed_programs:
        rows = [row for row in manifest if row.get("program") == program]
        if not rows:
            raise TraceError(f"{manifest_path}: no draw manifest rows for program{program}")
        if not any((row.get("changed_pixels") or 0) > 0 for row in rows):
            raise TraceError(
                f"{manifest_path}: program{program} has no rows with changed_pixels > 0"
            )

    for program, width, height in require_sizes:
        rows = [row for row in manifest if row.get("program") == program]
        if not rows:
            raise TraceError(f"{manifest_path}: no draw manifest rows for program{program}")
        bad_rows = []
        for row in rows:
            size = draw_texture_size(row)
            if size is None:
                raise TraceError(
                    f"{manifest_path}: row {row.get('index')!r} for program{program} "
                    "has no texture_width/texture_height; rerun the draw trace with current aemu"
                )
            if size != (width, height):
                bad_rows.append((row, size))
        if bad_rows:
            sample = ", ".join(
                f"row {row.get('index')} tex{row.get('texture')} "
                f"{size[0]}x{size[1]}"
                for row, size in bad_rows[:5]
            )
            raise TraceError(
                f"{manifest_path}: program{program} expected texture size "
                f"{width}x{height}, mismatches: {sample}"
            )

    for program, width, height in reject_sizes:
        bad_rows = [
            row
            for row in manifest
            if row.get("program") == program and draw_texture_size(row) == (width, height)
        ]
        if bad_rows:
            sample = ", ".join(
                f"row {row.get('index')} draw={row.get('draw')} tex{row.get('texture')}"
                for row in bad_rows[:5]
            )
            raise TraceError(
                f"{manifest_path}: program{program} used rejected texture size "
                f"{width}x{height}: {sample}"
            )


def load_uploads(side: str, directory: pathlib.Path, min_nonzero_rgb: int) -> list[UploadArtifact]:
    if not directory.exists():
        return []
    uploads = [
        parse_upload_png(side, path, min_nonzero_rgb)
        for path in sorted(directory.glob("*.png"))
    ]
    raw_without_png = sorted(
        path for path in directory.glob("*.raw") if not path.with_suffix(".png").exists()
    )
    if raw_without_png:
        names = ", ".join(path.name for path in raw_without_png[:5])
        raise TraceError(f"{directory}: raw payloads without PNG: {names}")
    return uploads


def load_draws(directory: pathlib.Path, min_nonzero_rgb: int) -> list[DrawArtifact]:
    if not directory.exists():
        return []
    return [
        parse_draw_png(path, min_nonzero_rgb)
        for path in sorted(directory.glob("*.png"))
    ]


def pair_uploads(hle: list[UploadArtifact], sdl: list[UploadArtifact]) -> list[tuple[UploadArtifact, UploadArtifact]]:
    remaining = {}
    for item in sdl:
        remaining.setdefault(item.pair_key, []).append(item)
    pairs = []
    missing = []
    for item in hle:
        candidates = remaining.get(item.pair_key) or []
        if not candidates:
            missing.append(item)
            continue
        pairs.append((item, candidates.pop(0)))
    if missing:
        sample = ", ".join(f"tex{item.texture}/{item.kind}/{item.width}x{item.height}" for item in missing[:5])
        raise TraceError(f"missing SDL upload pairs for HLE captures: {sample}")
    return pairs


def validate_pairs(pairs: list[tuple[UploadArtifact, UploadArtifact]], compare_raw: bool) -> None:
    for hle, sdl in pairs:
        if hle.png_info.rgb_sha1 != sdl.png_info.rgb_sha1:
            raise TraceError(
                f"PNG RGB mismatch for tex{hle.texture} {hle.kind} {hle.width}x{hle.height}: "
                f"{hle.png.name} vs {sdl.png.name}"
            )
        if compare_raw and hle.raw_sha1 != sdl.raw_sha1:
            raise TraceError(
                f"raw payload mismatch for tex{hle.texture} {hle.kind} {hle.width}x{hle.height}: "
                f"{hle.raw.name} vs {sdl.raw.name}"
            )


def validate_screenshot(path: pathlib.Path, min_nonzero_rgb: int) -> PngInfo:
    info = parse_png(path)
    if info.nonzero_rgb_pixels < min_nonzero_rgb:
        raise TraceError(
            f"{path}: only {info.nonzero_rgb_pixels} nonzero RGB pixels, expected at least {min_nonzero_rgb}"
        )
    return info


def summarize_uploads(upload: UploadArtifact) -> dict:
    return {
        "side": upload.side,
        "index": upload.index,
        "event_index": upload.event_index,
        "kind": upload.kind,
        "texture": upload.texture,
        "width": upload.width,
        "height": upload.height,
        "format": upload.format,
        "type": upload.ty,
        "raw_len": upload.raw_len,
        "raw_sha1": upload.raw_sha1,
        "png": str(upload.png),
        "png_nonzero_rgb_pixels": upload.png_info.nonzero_rgb_pixels,
        "png_rgb_sha1": upload.png_info.rgb_sha1,
        "raw_nonzero_rgb_pixels": upload.raw_nonzero_rgb_pixels,
        "raw_nonzero_alpha_pixels": upload.raw_nonzero_alpha_pixels,
    }


def summarize_draw(draw: DrawArtifact) -> dict:
    return {
        "index": draw.index,
        "event_index": draw.event_index,
        "draw": draw.draw,
        "kind": draw.kind,
        "program": draw.program,
        "texture": draw.texture,
        "width": draw.width,
        "height": draw.height,
        "png": str(draw.png),
        "png_nonzero_rgb_pixels": draw.png_info.nonzero_rgb_pixels,
        "png_rgb_sha1": draw.png_info.rgb_sha1,
    }


def run_check(args) -> dict:
    root = pathlib.Path(args.trace_dir)
    hle_dir = pathlib.Path(args.hle_dir) if args.hle_dir else root / "hle"
    sdl_dir = pathlib.Path(args.sdl_dir) if args.sdl_dir else root / "sdl"
    hle = load_uploads("hle", hle_dir, args.min_nonzero_rgb)
    sdl = load_uploads("sdl", sdl_dir, args.min_nonzero_rgb)
    if len(hle) < args.expect_hle:
        raise TraceError(f"{hle_dir}: found {len(hle)} HLE PNG uploads, expected at least {args.expect_hle}")
    if len(sdl) < args.expect_sdl:
        raise TraceError(f"{sdl_dir}: found {len(sdl)} SDL PNG uploads, expected at least {args.expect_sdl}")
    manifest_path = hle_dir / "manifest.jsonl"
    manifest = read_manifest(manifest_path)
    if hle:
        if not manifest:
            raise TraceError(f"{manifest_path}: missing HLE manifest")
        validate_manifest(hle, manifest, manifest_path)
    pairs = []
    if args.require_pairs:
        pairs = pair_uploads(hle, sdl)
        validate_pairs(pairs, args.compare_raw)
    screenshot = None
    if args.screenshot:
        screenshot = validate_screenshot(pathlib.Path(args.screenshot), args.min_screenshot_nonzero_rgb)
    draw_dir = pathlib.Path(args.draw_dir) if args.draw_dir else root / "sdl-draw"
    draw_manifest_path = draw_dir / "draw_manifest.jsonl"
    draw_manifest = read_manifest(draw_manifest_path)
    if args.skip_draw_pngs:
        draws = []
        if len(draw_manifest) < args.expect_draws:
            raise TraceError(
                f"{draw_manifest_path}: found {len(draw_manifest)} draw manifest rows, "
                f"expected at least {args.expect_draws}"
            )
    else:
        draws = load_draws(draw_dir, args.min_draw_nonzero_rgb)
        if len(draws) < args.expect_draws:
            raise TraceError(
                f"{draw_dir}: found {len(draws)} draw PNGs, expected at least {args.expect_draws}"
            )
    if draws:
        if not draw_manifest:
            raise TraceError(f"{draw_manifest_path}: missing draw manifest")
        validate_draw_manifest(draws, draw_manifest, draw_manifest_path)
    draw_gate_enabled = (
        args.expect_draw_program
        or args.require_draw_program_change
        or args.require_draw_texture_size
        or args.reject_draw_texture_size
    )
    if draw_gate_enabled:
        if not draw_manifest:
            raise TraceError(f"{draw_manifest_path}: missing draw manifest")
        validate_draw_program_gates(
            draw_manifest,
            draw_manifest_path,
            args.expect_draw_program,
            args.require_draw_program_change,
            args.require_draw_texture_size,
            args.reject_draw_texture_size,
        )
    return {
        "ok": True,
        "trace_dir": str(root),
        "hle_uploads": len(hle),
        "sdl_uploads": len(sdl),
        "manifest_rows": len(manifest),
        "paired_uploads": len(pairs),
        "draws": len(draws),
        "draw_manifest_rows": len(draw_manifest),
        "hle": [summarize_uploads(item) for item in hle[: args.detail_limit]],
        "sdl": [summarize_uploads(item) for item in sdl[: args.detail_limit]],
        "sdl_draw": [summarize_draw(item) for item in draws[: args.detail_limit]],
        "screenshot": None
        if screenshot is None
        else {
            "path": str(screenshot.path),
            "width": screenshot.width,
            "height": screenshot.height,
            "nonzero_rgb_pixels": screenshot.nonzero_rgb_pixels,
            "rgb_sha1": screenshot.rgb_sha1,
        },
    }


def make_test_png(width: int, height: int, rgb: bytes) -> bytes:
    rows = bytearray()
    for y in range(height):
        start = y * width * 3
        rows.append(0)
        rows.extend(rgb[start : start + width * 3])
    compressed = zlib.compress(bytes(rows))
    out = bytearray(PNG_SIGNATURE)
    append_chunk(out, b"IHDR", width.to_bytes(4, "big") + height.to_bytes(4, "big") + bytes([8, 2, 0, 0, 0]))
    append_chunk(out, b"IDAT", compressed)
    append_chunk(out, b"IEND", b"")
    return bytes(out)


def append_chunk(out: bytearray, kind: bytes, payload: bytes) -> None:
    out.extend(len(payload).to_bytes(4, "big"))
    out.extend(kind)
    out.extend(payload)
    crc = zlib.crc32(kind)
    crc = zlib.crc32(payload, crc)
    out.extend((crc & 0xFFFFFFFF).to_bytes(4, "big"))


def run_self_test() -> None:
    with tempfile.TemporaryDirectory() as temp:
        root = pathlib.Path(temp)
        hle_dir = root / "hle"
        sdl_dir = root / "sdl"
        hle_dir.mkdir()
        sdl_dir.mkdir()
        stem_hle = "0000-event00007-texsubimage2d-tex3-2x1-fmt1908-ty1401"
        stem_sdl = "0000-texsubimage2d-tex3-2x1-fmt1908-ty1401"
        raw = bytes([0x10, 0, 0, 0xFF, 0, 0x20, 0, 0xFF])
        rgb = bytes([0x10, 0, 0, 0, 0x20, 0])
        draw_stem = "0000-event00011-draw00004-DrawElements-program86-tex325-2x1"
        (hle_dir / f"{stem_hle}.png").write_bytes(make_test_png(2, 1, rgb))
        (hle_dir / f"{stem_hle}.raw").write_bytes(raw)
        (sdl_dir / f"{stem_sdl}.png").write_bytes(make_test_png(2, 1, rgb))
        (sdl_dir / f"{stem_sdl}.raw").write_bytes(raw)
        (hle_dir / "manifest.jsonl").write_text(
            json.dumps(
                {
                    "index": 0,
                    "event_index": 7,
                    "kind": "texsubimage2d",
                    "texture": 3,
                    "active_texture": 0x84C0,
                    "target": 0x0DE1,
                    "level": 0,
                    "xoffset": 0,
                    "yoffset": 0,
                    "width": 2,
                    "height": 1,
                    "format": GL_RGBA,
                    "type": GL_UNSIGNED_BYTE,
                    "pixels": 0x1234,
                    "payload_len": len(raw),
                    "nonzero_rgb_pixels": 2,
                    "nonzero_alpha_pixels": 2,
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        draw_dir = root / "sdl-draw"
        draw_dir.mkdir()
        (draw_dir / f"{draw_stem}.png").write_bytes(make_test_png(2, 1, rgb))
        (draw_dir / "draw_manifest.jsonl").write_text(
            json.dumps(
                {
                    "index": 0,
                    "event_index": 11,
                    "draw": 4,
                    "kind": "DrawElements",
                    "count": 6,
                    "program": 86,
                    "active_texture": 0x84C0,
                    "texture": 325,
                    "viewport": [0, 0, 2, 1],
                    "changed_pixels": 2,
                    "changed_bytes": 8,
                    "texture_width": 64,
                    "texture_height": 32,
                    "texture_format": GL_RGBA,
                    "texture_type": GL_UNSIGNED_BYTE,
                    "texture_last_upload_width": 64,
                    "texture_last_upload_height": 32,
                    "texture_last_payload_len": len(raw),
                    "texture_last_nonzero_rgb_pixels": 2,
                    "texture_last_nonzero_alpha_pixels": 2,
                    "width": 2,
                    "height": 1,
                    "png": f"{draw_stem}.png",
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        args = argparse.Namespace(
            trace_dir=str(root),
            hle_dir=None,
            sdl_dir=None,
            draw_dir=None,
            expect_hle=1,
            expect_sdl=1,
            expect_draws=1,
            require_pairs=True,
            compare_raw=True,
            min_nonzero_rgb=1,
            min_draw_nonzero_rgb=1,
            screenshot=None,
            min_screenshot_nonzero_rgb=1,
            detail_limit=3,
            skip_draw_pngs=False,
            expect_draw_program=[86],
            require_draw_program_change=[86],
            require_draw_texture_size=[],
            reject_draw_texture_size=[],
        )
        summary = run_check(args)
        assert summary["paired_uploads"] == 1
        assert summary["draws"] == 1
        assert summary["hle"][0]["raw_nonzero_rgb_pixels"] == 2
        assert summary["hle"][0]["raw_nonzero_alpha_pixels"] == 2


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Validate aemu SDL2/HLE trace PNG, raw, and manifest artifacts"
    )
    parser.add_argument("trace_dir", nargs="?", help="trace root containing hle/ and sdl/")
    parser.add_argument("--hle-dir", help="override HLE trace directory")
    parser.add_argument("--sdl-dir", help="override SDL trace directory")
    parser.add_argument("--draw-dir", help="override SDL draw PNG directory")
    parser.add_argument("--screenshot", help="also validate a WebSocket screenshot PNG")
    parser.add_argument("--expect-hle", type=int, default=1)
    parser.add_argument("--expect-sdl", type=int, default=1)
    parser.add_argument("--expect-draws", type=int, default=0)
    parser.add_argument(
        "--expect-draw-program",
        type=lambda raw: int(raw, 0),
        action="append",
        default=[],
        help="require at least one draw manifest row with this GL program id",
    )
    parser.add_argument(
        "--require-draw-program-change",
        type=lambda raw: int(raw, 0),
        action="append",
        default=[],
        help="require at least one draw manifest row for this GL program id with changed_pixels > 0",
    )
    parser.add_argument(
        "--require-draw-texture-size",
        type=parse_program_size,
        action="append",
        default=[],
        metavar="PROGRAM:WxH",
        help="require every captured draw for PROGRAM to use a texture with this size",
    )
    parser.add_argument(
        "--reject-draw-texture-size",
        type=parse_program_size,
        action="append",
        default=[],
        metavar="PROGRAM:WxH",
        help="fail if any captured draw for PROGRAM uses a texture with this size",
    )
    parser.add_argument("--min-nonzero-rgb", type=int, default=1)
    parser.add_argument("--min-draw-nonzero-rgb", type=int, default=1)
    parser.add_argument("--min-screenshot-nonzero-rgb", type=int, default=1)
    parser.add_argument("--detail-limit", type=int, default=5)
    parser.add_argument(
        "--skip-draw-pngs",
        action="store_true",
        help="validate draw gates from draw_manifest.jsonl without decoding every draw PNG",
    )
    parser.add_argument("--no-require-pairs", action="store_true")
    parser.add_argument("--no-compare-raw", action="store_true")
    parser.add_argument("--json", action="store_true", help="print full JSON summary")
    parser.add_argument("--self-test", action="store_true", help="run built-in validator self-test")
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.self_test:
        run_self_test()
        print("trace_check self-test ok")
        return 0
    if not args.trace_dir:
        raise SystemExit("trace_dir is required unless --self-test is used")
    args.require_pairs = not args.no_require_pairs
    args.compare_raw = not args.no_compare_raw
    try:
        summary = run_check(args)
    except TraceError as err:
        print(f"trace_check failed: {err}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(
            "trace_check ok: "
            f"hle={summary['hle_uploads']} sdl={summary['sdl_uploads']} "
            f"manifest={summary['manifest_rows']} pairs={summary['paired_uploads']} "
            f"draws={summary['draws']} draw_manifest={summary['draw_manifest_rows']}"
        )
        if summary["screenshot"]:
            shot = summary["screenshot"]
            print(
                f"screenshot: {shot['width']}x{shot['height']} "
                f"nonzero_rgb={shot['nonzero_rgb_pixels']} {shot['path']}"
            )
        for side in ("hle", "sdl"):
            for item in summary[side]:
                print(
                    f"{side}: #{item['index']} tex{item['texture']} {item['kind']} "
                    f"{item['width']}x{item['height']} raw={item['raw_len']} "
                    f"raw_nonzero_rgb={item['raw_nonzero_rgb_pixels']} "
                    f"raw_nonzero_alpha={item['raw_nonzero_alpha_pixels']}"
                )
        for item in summary["sdl_draw"]:
            print(
                f"sdl-draw: #{item['index']} event={item['event_index']} "
                f"draw={item['draw']} program={item['program']} tex{item['texture']} "
                f"{item['width']}x{item['height']} "
                f"nonzero_rgb={item['png_nonzero_rgb_pixels']}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
