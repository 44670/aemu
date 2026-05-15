#!/usr/bin/env python3
import argparse
import json
import pathlib
import re
import sys
import tempfile


class TraceQueryError(RuntimeError):
    pass


def parse_u32(raw: str) -> int:
    try:
        value = int(raw, 0)
    except ValueError as err:
        raise argparse.ArgumentTypeError(f"invalid integer {raw!r}") from err
    if value < 0:
        raise argparse.ArgumentTypeError(f"integer must be non-negative, got {raw!r}")
    return value


def parse_size(raw: str) -> tuple[int, int]:
    match = re.fullmatch(r"(?P<width>\d+)x(?P<height>\d+)", raw.strip())
    if not match:
        raise argparse.ArgumentTypeError(f"expected WxH, got {raw!r}")
    width = int(match.group("width"))
    height = int(match.group("height"))
    if width <= 0 or height <= 0:
        raise argparse.ArgumentTypeError(f"size must be positive, got {raw!r}")
    return width, height


def read_jsonl(path: pathlib.Path) -> list[dict]:
    if not path.exists():
        return []
    rows = []
    for line_no, line in enumerate(path.read_text().splitlines(), start=1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as err:
            raise TraceQueryError(f"{path}:{line_no}: invalid JSON: {err}") from err
        if not isinstance(row, dict):
            raise TraceQueryError(f"{path}:{line_no}: expected JSON object")
        rows.append(row)
    return rows


def trace_paths(args) -> tuple[pathlib.Path, pathlib.Path, pathlib.Path]:
    root = pathlib.Path(args.trace_dir)
    hle_dir = pathlib.Path(args.hle_dir) if args.hle_dir else root / "hle"
    draw_dir = pathlib.Path(args.draw_dir) if args.draw_dir else root / "sdl-draw"
    return root, hle_dir, draw_dir


def gles_events_path(args) -> pathlib.Path:
    root = pathlib.Path(args.trace_dir)
    return pathlib.Path(args.gles_events) if args.gles_events else root / "gles_events.jsonl"


def native_log_path(args) -> pathlib.Path:
    root = pathlib.Path(args.trace_dir)
    return pathlib.Path(args.native_log) if args.native_log else root / "run.log"


def native_events_path(args) -> pathlib.Path:
    root = pathlib.Path(args.trace_dir)
    return pathlib.Path(args.native_events) if args.native_events else root / "native_events.jsonl"


def load_trace(args) -> tuple[list[dict], list[dict], pathlib.Path, pathlib.Path]:
    _root, hle_dir, draw_dir = trace_paths(args)
    uploads = read_jsonl(hle_dir / "manifest.jsonl")
    draws = read_jsonl(draw_dir / "draw_manifest.jsonl")
    return uploads, draws, hle_dir / "manifest.jsonl", draw_dir / "draw_manifest.jsonl"


def load_gles_events(args) -> tuple[list[dict], pathlib.Path]:
    path = gles_events_path(args)
    return read_jsonl(path), path


def load_native_events(args) -> tuple[list[dict], pathlib.Path]:
    path = native_events_path(args)
    return read_jsonl(path), path


def row_int(row: dict, key: str) -> int | None:
    value = row.get(key)
    return value if isinstance(value, int) else None


def draw_texture_info(row: dict, uploads: list[dict]) -> dict:
    texture = row_int(row, "texture")
    width = row_int(row, "texture_width")
    height = row_int(row, "texture_height")
    if texture is None:
        return {"texture": None, "source": "draw_manifest", "size": None}
    if width is not None and height is not None and width > 0 and height > 0:
        return {
            "texture": texture,
            "source": "draw_manifest",
            "size": (width, height),
            "format": row_int(row, "texture_format"),
            "type": row_int(row, "texture_type"),
            "payload_len": row_int(row, "texture_last_payload_len"),
            "upload_event_index": None,
        }

    draw_event = row_int(row, "event_index")
    candidates = [
        upload
        for upload in uploads
        if row_int(upload, "texture") == texture
        and row_int(upload, "width") is not None
        and row_int(upload, "height") is not None
        and (
            draw_event is None
            or row_int(upload, "event_index") is None
            or row_int(upload, "event_index") <= draw_event
        )
    ]
    candidates.sort(key=lambda item: row_int(item, "event_index") or -1)
    if not candidates:
        return {"texture": texture, "source": "missing", "size": None}
    upload = candidates[-1]
    return {
        "texture": texture,
        "source": "hle_manifest",
        "size": (row_int(upload, "width"), row_int(upload, "height")),
        "format": row_int(upload, "format"),
        "type": row_int(upload, "type"),
        "payload_len": row_int(upload, "payload_len"),
        "upload_event_index": row_int(upload, "event_index"),
    }


def format_size(size: tuple[int, int] | None) -> str:
    if size is None:
        return "unknown"
    return f"{size[0]}x{size[1]}"


def format_draw(row: dict, uploads: list[dict]) -> str:
    info = draw_texture_info(row, uploads)
    parts = [
        f"row={row.get('index')}",
        f"event={row.get('event_index')}",
        f"draw={row.get('draw')}",
        f"kind={row.get('kind')}",
        f"program={row.get('program')}",
        f"tex{row.get('texture')}",
        f"tex_size={format_size(info['size'])}",
        f"source={info['source']}",
    ]
    if info.get("upload_event_index") is not None:
        parts.append(f"upload_event={info['upload_event_index']}")
    return " ".join(parts)


def texture_rows(texture: int, uploads: list[dict], draws: list[dict]) -> dict:
    return {
        "uploads": [row for row in uploads if row_int(row, "texture") == texture],
        "draws": [row for row in draws if row_int(row, "texture") == texture],
    }


def print_summary(args) -> int:
    uploads, draws, hle_manifest, draw_manifest = load_trace(args)
    gles_events, gles_path = load_gles_events(args)
    native_events, native_path = load_native_events(args)
    programs = sorted({row.get("program") for row in draws if isinstance(row.get("program"), int)})
    textures = sorted({row.get("texture") for row in draws if isinstance(row.get("texture"), int)})
    print(f"hle_manifest={hle_manifest} rows={len(uploads)}")
    print(f"draw_manifest={draw_manifest} rows={len(draws)}")
    print(f"gles_events={gles_path} rows={len(gles_events)}")
    print(f"native_events={native_path} rows={len(native_events)}")
    print(f"draw_programs={','.join(map(str, programs[:24])) or 'none'}")
    if len(programs) > 24:
        print(f"draw_programs_more={len(programs) - 24}")
    print(f"draw_textures={','.join('tex' + str(texture) for texture in textures[:24]) or 'none'}")
    if len(textures) > 24:
        print(f"draw_textures_more={len(textures) - 24}")
    return 0


def format_gles_event(row: dict) -> str:
    parts = [
        f"event={row.get('index')}",
        f"kind={row.get('kind')}",
        f"program={row.get('current_program')}",
        f"bound_tex2d={row.get('bound_texture_2d')}",
    ]
    for key in (
        "target",
        "texture",
        "mode",
        "count",
        "type",
        "indices",
        "width",
        "height",
        "format",
        "payload_len",
    ):
        if key in row:
            parts.append(f"{key}={row.get(key)}")
    return " ".join(parts)


def print_gles_event(args) -> int:
    events, path = load_gles_events(args)
    if not events:
        raise TraceQueryError(f"{path}: missing or empty GLES event trace")
    low = max(0, args.event - args.context)
    high = args.event + args.context
    rows = [
        row
        for row in events
        if isinstance(row.get("index"), int) and low <= row["index"] <= high
    ]
    print(f"gles-event {args.event}: path={path} rows={len(rows)}")
    for row in rows[: args.limit]:
        print(format_gles_event(row))
    return 0


def format_native_event(row: dict) -> str:
    parts = [
        f"step={row.get('step')}",
        f"thread={row.get('thread')}",
        f"pc=0x{row.get('pc'):08x}" if isinstance(row.get("pc"), int) else f"pc={row.get('pc')}",
        f"event={row.get('event')}",
        f"isa={row.get('isa')}",
        f"r0=0x{row.get('r0'):08x}" if isinstance(row.get("r0"), int) else f"r0={row.get('r0')}",
        f"r1=0x{row.get('r1'):08x}" if isinstance(row.get("r1"), int) else f"r1={row.get('r1')}",
        f"r2=0x{row.get('r2'):08x}" if isinstance(row.get("r2"), int) else f"r2={row.get('r2')}",
        f"r3=0x{row.get('r3'):08x}" if isinstance(row.get("r3"), int) else f"r3={row.get('r3')}",
        f"sp=0x{row.get('sp'):08x}" if isinstance(row.get("sp"), int) else f"sp={row.get('sp')}",
        f"lr=0x{row.get('lr'):08x}" if isinstance(row.get("lr"), int) else f"lr={row.get('lr')}",
        f"gles_next={row.get('gles_next_event_index')}",
        f"program={row.get('gl_current_program')}",
        f"bound_tex2d={row.get('gl_bound_texture_2d')}",
    ]
    parts.extend(format_native_mem32(row.get("mem32"), "mem32"))
    parts.extend(format_native_deref32(row.get("deref32")))
    parts.extend(format_native_cxx_strings(row.get("cxx_strings")))
    parts.extend(format_native_byte_samples(row.get("byte_samples")))
    return " ".join(parts)


def fmt_u32(value) -> str:
    return f"0x{value:08x}" if isinstance(value, int) else str(value)


def format_native_mem32(items, prefix: str) -> list[str]:
    if not isinstance(items, list):
        return []
    parts = []
    for item in items:
        if not isinstance(item, dict):
            continue
        fields = item.get("fields")
        if not isinstance(fields, list):
            continue
        values = []
        for field in fields:
            if not isinstance(field, dict):
                continue
            label = field.get("label")
            if "value" in field:
                values.append(f"{label}={fmt_u32(field.get('value'))}")
            elif "error" in field:
                values.append(f"{label}=<{field.get('error')}>")
        if values:
            parts.append(f"{prefix}=" + ",".join(values))
    return parts


def format_native_deref32(items) -> list[str]:
    if not isinstance(items, list):
        return []
    parts = []
    for item in items:
        if not isinstance(item, dict):
            continue
        chain = item.get("chain")
        if not isinstance(chain, list):
            continue
        values = []
        for field in chain:
            if not isinstance(field, dict):
                continue
            label = field.get("label")
            if "value" in field:
                values.append(f"{label}->{fmt_u32(field.get('value'))}")
            elif "error" in field:
                values.append(f"{label}=<{field.get('error')}>")
        if values:
            parts.append("deref32=" + ",".join(values))
    return parts


def format_native_cxx_strings(items) -> list[str]:
    if not isinstance(items, list):
        return []
    parts = []
    for item in items:
        if not isinstance(item, dict):
            continue
        offset = item.get("offset", 0)
        offset_text = f"0x{offset:x}" if isinstance(offset, int) else str(offset)
        label = f"{item.get('base')}+{offset_text}"
        if "bytes" in item:
            parts.append(f"cxx[{label}]={item.get('bytes')!r}")
        elif "error" in item:
            parts.append(f"cxx[{label}]=<{item.get('error')}>")
    return parts


def format_native_byte_samples(items) -> list[str]:
    if not isinstance(items, list):
        return []
    parts = []
    for item in items:
        if not isinstance(item, dict):
            continue
        source = item.get("source")
        if "hex" in item:
            parts.append(
                f"bytes[{source}@{fmt_u32(item.get('addr'))}]"
                f"=len={item.get('len')} sha1={item.get('sha1')} hex={item.get('hex')}"
            )
        elif "error" in item:
            parts.append(f"bytes[{source}]=<{item.get('error')}>")
    return parts


def native_event_matches_contains(row: dict, needle: str) -> bool:
    if needle in str(row.get("event", "")).lower():
        return True
    pc = row.get("pc")
    return isinstance(pc, int) and needle in f"0x{pc:08x}".lower()


def print_native_event(args) -> int:
    events, path = load_native_events(args)
    rows = events
    if args.pc is not None:
        rows = [row for row in rows if row_int(row, "pc") == args.pc]
    if args.contains:
        needle = args.contains.lower()
        rows = [row for row in rows if native_event_matches_contains(row, needle)]
    print(f"native-event: path={path} rows={len(rows)}")
    for row in rows[: args.limit]:
        print(format_native_event(row))
    return 0


def byte_sample(row: dict, source_prefix: str) -> bytes | None:
    samples = row.get("byte_samples")
    if not isinstance(samples, list):
        return None
    for sample in samples:
        if not isinstance(sample, dict):
            continue
        source = sample.get("source")
        hex_value = sample.get("hex")
        if not isinstance(source, str) or not source.startswith(source_prefix):
            continue
        if not isinstance(hex_value, str):
            continue
        try:
            return bytes.fromhex(hex_value)
        except ValueError:
            return None
    return None


def mem32_field_value(row: dict, base: str, label: str) -> int | None:
    items = row.get("mem32")
    if not isinstance(items, list):
        return None
    for item in items:
        if not isinstance(item, dict) or item.get("base") != base:
            continue
        fields = item.get("fields")
        if not isinstance(fields, list):
            continue
        for field in fields:
            if not isinstance(field, dict) or field.get("label") != label:
                continue
            value = field.get("value")
            return value if isinstance(value, int) else None
    return None


def little_int(raw: bytes, words: int) -> int:
    return int.from_bytes(raw[: words * 4], "little")


def little_hex(value: int, words: int) -> str:
    return value.to_bytes(words * 4, "little").hex()


def bn_mont_pairs(events: list[dict]) -> list[tuple[dict, dict]]:
    pairs = []
    pending = None
    for row in events:
        event = row.get("event")
        if event == "bn_mul_mont.entry":
            pending = row
        elif event == "BN_mod_mul_montgomery.after-bn_mul_mont" and pending is not None:
            pairs.append((pending, row))
            pending = None
    return pairs


def check_bn_mont_pair(entry: dict, after: dict) -> tuple[bool, dict]:
    words = mem32_field_value(entry, "sp", "sp+0x4")
    a = byte_sample(entry, "r1+0x0")
    b = byte_sample(entry, "r2+0x0")
    modulus = byte_sample(entry, "r3+0x0")
    output = byte_sample(after, "*r9+0x0")
    detail = {
        "entry_step": entry.get("step"),
        "after_step": after.get("step"),
        "words": words,
    }
    if not isinstance(words, int) or words <= 0:
        detail["error"] = "missing or invalid Montgomery word count"
        return False, detail
    if a is None or b is None or modulus is None or output is None:
        detail["error"] = "missing input/output byte sample"
        return False, detail
    required_len = words * 4
    if min(len(a), len(b), len(modulus), len(output)) < required_len:
        detail["error"] = f"short byte sample for {words} words"
        return False, detail

    n = little_int(modulus, words)
    if n <= 0 or n % 2 == 0:
        detail["error"] = f"invalid odd Montgomery modulus 0x{n:x}"
        return False, detail
    radix = 1 << (32 * words)
    expected = (little_int(a, words) * little_int(b, words) * pow(radix, -1, n)) % n
    actual = little_int(output, words)
    detail["actual"] = little_hex(actual, words)
    detail["expected"] = little_hex(expected, words)
    detail["a"] = a[:required_len].hex()
    detail["b"] = b[:required_len].hex()
    detail["modulus"] = modulus[:required_len].hex()
    return actual == expected, detail


def print_bn_mont_check(args) -> int:
    events, path = load_native_events(args)
    pairs = bn_mont_pairs(events)
    if not pairs:
        raise TraceQueryError(f"{path}: no bn_mul_mont.entry -> after-bn_mul_mont pairs found")
    checked = 0
    mismatches = []
    incomplete = []
    for entry, after in pairs:
        ok, detail = check_bn_mont_pair(entry, after)
        if "error" in detail:
            incomplete.append(detail)
            continue
        checked += 1
        if not ok:
            mismatches.append(detail)

    print(
        f"bn-mont-check: path={path} pairs={len(pairs)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"entry_step={detail.get('entry_step')} after_step={detail.get('after_step')} "
            f"words={detail.get('words')}"
        )
        print(f"  actual  ={detail.get('actual')}")
        print(f"  expected={detail.get('expected')}")
        print(f"  a       ={detail.get('a')}")
        print(f"  b       ={detail.get('b')}")
        print(f"  modulus ={detail.get('modulus')}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"entry_step={detail.get('entry_step')} after_step={detail.get('after_step')} "
            f"words={detail.get('words')} error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


MEM32_RE = re.compile(
    r"(?P<prefix>THREAD_TRACE id=(?P<thread>\d+) )?"
    r"MEM32 step=(?P<step>\d+) pc=(?P<pc>0x[0-9a-fA-F]+) "
    r"(?P<base>[a-z0-9]+)=(?P<base_value>0x[0-9a-fA-F]+)(?P<fields>.*)"
)
MEM32_FIELD_RE = re.compile(
    r"\[(?P<label>[^\]]+)@(?P<addr>0x[0-9a-fA-F]+)\]=(?P<value>0x[0-9a-fA-F]+)"
)
CXX_STRING_RE = re.compile(
    r"(?P<prefix>THREAD_TRACE id=(?P<thread>\d+) )?"
    r"CXX_STRING step=(?P<step>\d+) pc=(?P<pc>0x[0-9a-fA-F]+) "
    r"(?P<base>[a-z0-9]+)\+(?P<offset>0x[0-9a-fA-F]+|0)=0x[0-9a-fA-F]+ "
    r"data=(?P<data>0x[0-9a-fA-F]+) len=(?P<len>\d+) bytes=(?P<bytes>.*)"
)


def parse_trace_string(raw: str) -> str:
    raw = raw.strip()
    if len(raw) >= 2 and raw[0] == '"' and raw[-1] == '"':
        try:
            return bytes(raw[1:-1], "utf-8").decode("unicode_escape")
        except UnicodeDecodeError:
            return raw[1:-1]
    return raw


def parse_native_log(args) -> tuple[list[dict], pathlib.Path]:
    path = native_log_path(args)
    if not path.exists():
        raise TraceQueryError(f"{path}: missing native log")

    pending_ctor: dict[str, str | int | None] | None = None
    texture_fields: dict[int, dict[str, int]] = {}
    ctor_by_out: dict[int, dict] = {}
    rows = []

    for line_no, line in enumerate(path.read_text(errors="replace").splitlines(), start=1):
        cxx = CXX_STRING_RE.search(line)
        if cxx and int(cxx.group("pc"), 0) == 0x716F2038:
            step = int(cxx.group("step"))
            offset = int(cxx.group("offset"), 0)
            value = parse_trace_string(cxx.group("bytes"))
            if pending_ctor is None or pending_ctor.get("step") != step:
                pending_ctor = {
                    "line": line_no,
                    "step": step,
                    "resource": None,
                    "package": None,
                }
            if offset == 0:
                pending_ctor["resource"] = value
            elif offset == 4:
                pending_ctor["package"] = value
            continue

        mem = MEM32_RE.search(line)
        if not mem:
            continue
        pc = int(mem.group("pc"), 0)
        step = int(mem.group("step"))
        base_value = int(mem.group("base_value"), 0)
        fields = {
            item.group("label"): int(item.group("value"), 0)
            for item in MEM32_FIELD_RE.finditer(mem.group("fields"))
        }

        if pc == 0x716F206E:
            gl_name = next(
                (value for label, value in fields.items() if label.startswith("r0+0x24")),
                None,
            )
            target = next(
                (value for label, value in fields.items() if label.startswith("r0+0x28")),
                None,
            )
            if gl_name is not None:
                texture_fields[base_value] = {
                    "gl": gl_name,
                    "target": target or 0,
                    "line": line_no,
                    "step": step,
                }
            continue

        if pc == 0x716F2070:
            texture = next(
                (value for label, value in fields.items() if label.startswith("r6+0x4")),
                None,
            )
            if texture is not None:
                ctor = {
                    "line": line_no,
                    "step": step,
                    "out": base_value,
                    "texture": texture,
                    "resource": None,
                    "package": None,
                    "gl": None,
                    "target": None,
                }
                if pending_ctor is not None and step >= int(pending_ctor["step"]):
                    ctor["resource"] = pending_ctor.get("resource")
                    ctor["package"] = pending_ctor.get("package")
                if texture in texture_fields:
                    ctor.update(
                        {
                            "gl": texture_fields[texture].get("gl"),
                            "target": texture_fields[texture].get("target"),
                        }
                    )
                ctor_by_out[base_value] = ctor
            continue

        if pc in (0x716F03C8, 0x716F0402):
            texture = next(
                (
                    value
                    for label, value in fields.items()
                    if label.startswith("r5+0x4")
                ),
                None,
            )
            row = {
                "line": line_no,
                "step": step,
                "pc": pc,
                "branch": "found" if pc == 0x716F03C8 else "fallback",
                "out": base_value,
                "texture": texture,
                "resource": None,
                "package": None,
                "gl": None,
                "target": None,
            }
            if base_value in ctor_by_out:
                ctor = ctor_by_out[base_value]
                row.update(
                    {
                        "texture": ctor.get("texture", texture),
                        "resource": ctor.get("resource"),
                        "package": ctor.get("package"),
                        "gl": ctor.get("gl"),
                        "target": ctor.get("target"),
                    }
                )
            if row["texture"] in texture_fields:
                row["gl"] = texture_fields[row["texture"]].get("gl")
                row["target"] = texture_fields[row["texture"]].get("target")
            rows.append(row)

    return rows, path


def print_mcpe_texturedata(args) -> int:
    rows, path = parse_native_log(args)
    if args.empty_only:
        rows = [row for row in rows if row.get("resource") in (None, "")]
    print(f"mcpe-texturedata: path={path} rows={len(rows)}")
    for row in rows[: args.limit]:
        resource = row.get("resource")
        package = row.get("package")
        texture = row.get("texture")
        gl_name = row.get("gl")
        target = row.get("target")
        texture_text = "unknown" if texture is None else f"0x{texture:08x}"
        gl_text = "unknown" if gl_name is None else str(gl_name)
        target_text = "unknown" if target is None else f"0x{target:04x}"
        print(
            f"line={row['line']} step={row['step']} branch={row['branch']} "
            f"out=0x{row['out']:08x} resource={resource!r} package={package!r} "
            f"texture={texture_text} gl={gl_text} target={target_text}"
        )
    fallback_empty = [
        row for row in rows if row.get("branch") == "fallback" and row.get("resource") in (None, "")
    ]
    if fallback_empty:
        print(f"empty_resource_fallbacks={len(fallback_empty)}")
    return 0


def print_texture(args) -> int:
    uploads, draws, _hle_manifest, _draw_manifest = load_trace(args)
    rows = texture_rows(args.texture, uploads, draws)
    print(f"texture tex{args.texture}: uploads={len(rows['uploads'])} draws={len(rows['draws'])}")
    for row in rows["uploads"][: args.limit]:
        print(
            "upload "
            f"row={row.get('index')} event={row.get('event_index')} "
            f"{row.get('kind')} {row.get('width')}x{row.get('height')} "
            f"fmt=0x{row.get('format', 0):04x} type=0x{row.get('type', 0):04x} "
            f"payload={row.get('payload_len')}"
        )
    for row in rows["draws"][: args.limit]:
        print("draw " + format_draw(row, uploads))
    return 0


def validate_program_texture_sizes(
    draws: list[dict],
    uploads: list[dict],
    program: int,
    require_size: tuple[int, int] | None,
    reject_size: tuple[int, int] | None,
) -> list[str]:
    failures = []
    program_draws = [row for row in draws if row_int(row, "program") == program]
    if not program_draws:
        return [f"no captured draw rows for program{program}"]
    for row in program_draws:
        info = draw_texture_info(row, uploads)
        size = info["size"]
        if require_size is not None:
            if size is None:
                failures.append(
                    f"draw {row.get('draw')} tex{row.get('texture')} has unknown texture size"
                )
            elif size != require_size:
                failures.append(
                    f"draw {row.get('draw')} tex{row.get('texture')} "
                    f"has {format_size(size)}, expected {format_size(require_size)}"
                )
        if reject_size is not None and size == reject_size:
            failures.append(
                f"draw {row.get('draw')} tex{row.get('texture')} "
                f"has rejected {format_size(reject_size)}"
            )
    return failures


def print_program(args) -> int:
    uploads, draws, _hle_manifest, _draw_manifest = load_trace(args)
    program_draws = [row for row in draws if row_int(row, "program") == args.program]
    if args.json:
        out = []
        for row in program_draws:
            item = dict(row)
            info = draw_texture_info(row, uploads)
            item["resolved_texture_source"] = info["source"]
            item["resolved_texture_size"] = info["size"]
            item["resolved_upload_event_index"] = info.get("upload_event_index")
            out.append(item)
        print(json.dumps(out, indent=2, sort_keys=True))
    else:
        print(f"program{args.program}: draws={len(program_draws)}")
        for row in program_draws[: args.limit]:
            print(format_draw(row, uploads))
    failures = validate_program_texture_sizes(
        draws,
        uploads,
        args.program,
        args.expect_texture_size,
        args.reject_texture_size,
    )
    if failures:
        for failure in failures[:10]:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    return 0


def print_mcpe_text(args) -> int:
    uploads, draws, _hle_manifest, _draw_manifest = load_trace(args)
    program_draws = [row for row in draws if row_int(row, "program") == args.program]
    expect_texture_size, reject_texture_size = mcpe_text_texture_size_gates(args)
    print(f"mcpe-text program{args.program}: draws={len(program_draws)}")
    for row in program_draws[: args.limit]:
        print(format_draw(row, uploads))
    failures = validate_program_texture_sizes(
        draws,
        uploads,
        args.program,
        expect_texture_size,
        reject_texture_size,
    )
    if failures:
        for failure in failures[:10]:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    print(
        "mcpe-text ok: "
        f"program{args.program} uses {format_size(expect_texture_size)} "
        f"and avoids {format_size(reject_texture_size)}"
    )
    return 0


def mcpe_text_texture_size_gates(args) -> tuple[tuple[int, int], tuple[int, int]]:
    expect_texture_size = args.expect_texture_size
    if expect_texture_size is None:
        expect_texture_size = (128, 128) if args.profile == "native" else (256, 256)
    reject_texture_size = args.reject_texture_size or (64, 32)
    return expect_texture_size, reject_texture_size


def run_self_test() -> None:
    with tempfile.TemporaryDirectory() as temp:
        root = pathlib.Path(temp)
        (root / "hle").mkdir()
        (root / "sdl-draw").mkdir()
        (root / "hle" / "manifest.jsonl").write_text(
            json.dumps(
                {
                    "index": 0,
                    "event_index": 7,
                    "kind": "teximage2d",
                    "texture": 325,
                    "width": 64,
                    "height": 32,
                    "format": 0x1908,
                    "type": 0x1401,
                    "payload_len": 8192,
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        (root / "sdl-draw" / "draw_manifest.jsonl").write_text(
            json.dumps(
                {
                    "index": 0,
                    "event_index": 11,
                    "draw": 4,
                    "kind": "DrawElements",
                    "program": 86,
                    "texture": 325,
                    "width": 854,
                    "height": 480,
                    "png": "draw.png",
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        args = argparse.Namespace(
            trace_dir=str(root),
            hle_dir=None,
            draw_dir=None,
            program=86,
            profile="hle",
            expect_texture_size=(256, 256),
            reject_texture_size=(64, 32),
        )
        uploads, draws, _hle_manifest, _draw_manifest = load_trace(args)
        assert draw_texture_info(draws[0], uploads)["size"] == (64, 32)
        failures = validate_program_texture_sizes(
            draws,
            uploads,
            args.program,
            args.expect_texture_size,
            args.reject_texture_size,
        )
        assert failures
        (root / "run.log").write_text(
            "\n".join(
                [
                    'CXX_STRING step=10 pc=0x716f2038 r2+0x0=0x1000 data=0x2000 len=0 bytes=""',
                    'CXX_STRING step=10 pc=0x716f2038 r2+0x4=0x1004 data=0x2010 len=18 bytes="InAppPackageImages"',
                    "MEM32 step=12 pc=0x716f206e r0=0x60ff33f0 [r0+0x24@0x60ff3414]=0x00000145 [r0+0x28@0x60ff3418]=0x00000de1",
                    "MEM32 step=13 pc=0x716f2070 r6=0x6dffde8c [r6+0x0@0x6dffde8c]=0x606c7150 [r6+0x4@0x6dffde90]=0x60ff33f0",
                    "MEM32 step=14 pc=0x716f0402 r5=0x6dffde8c [r5+0x0@0x6dffde8c]=0x606c7150 [r5+0x4@0x6dffde90]=0x60ff33f0",
                ]
            )
            + "\n"
        )
        native_args = argparse.Namespace(trace_dir=str(root), native_log=None, empty_only=True)
        native_rows, _native_path = parse_native_log(native_args)
        assert len(native_rows) == 1
        assert native_rows[0]["branch"] == "fallback"
        assert native_rows[0]["resource"] == ""
        assert native_rows[0]["gl"] == 325
        (root / "native_events.jsonl").write_text(
            json.dumps(
                {
                    "step": 17,
                    "thread": 1,
                    "pc": 0x716EB818,
                    "event": "TextureOGL::bindTexture",
                    "isa": "Thumb",
                    "r0": 0x60FF33F0,
                    "r1": 0x145,
                    "r2": 0,
                    "r3": 0,
                    "sp": 0x6DFF0000,
                    "lr": 0x716F0000,
                    "gles_next_event_index": 21600,
                    "gl_current_program": 79,
                    "gl_bound_texture_2d": 325,
                    "mem32": [
                        {
                            "base": "r0",
                            "base_value": 0x60FF33F0,
                            "fields": [
                                {
                                    "label": "r0+0x24",
                                    "offset": 0x24,
                                    "addr": 0x60FF3414,
                                    "value": 325,
                                }
                            ],
                        }
                    ],
                    "deref32": [
                        {
                            "base": "r0",
                            "base_value": 0x60FF33F0,
                            "chain": [
                                {
                                    "depth": 0,
                                    "parent": "r0",
                                    "label": "r0+0x4",
                                    "offset": 4,
                                    "addr": 0x60FF33F4,
                                    "value": 0x60FF3414,
                                }
                            ],
                        }
                    ],
                    "cxx_strings": [
                        {
                            "base": "r2",
                            "offset": 4,
                            "addr": 0x1004,
                            "data": 0x2010,
                            "len": 18,
                            "bytes": "InAppPackageImages",
                            "escaped": '"InAppPackageImages"',
                            "truncated": False,
                        }
                    ],
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        event_args = argparse.Namespace(
            trace_dir=str(root), native_events=None, pc=None, contains="bindtexture", limit=4
        )
        native_events, _events_path = load_native_events(event_args)
        assert len(native_events) == 1
        assert native_event_matches_contains(native_events[0], "bindtexture")
        formatted = format_native_event(native_events[0])
        assert "gles_next=21600" in formatted
        assert "mem32=r0+0x24=0x00000145" in formatted
        assert "deref32=r0+0x4->0x60ff3414" in formatted
        assert "cxx[r2+0x4]='InAppPackageImages'" in formatted
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 20,
                            "event": "bn_mul_mont.entry",
                            "mem32": [
                                {
                                    "base": "sp",
                                    "fields": [
                                        {"label": "sp+0x4", "value": 1},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {"source": "r1+0x0", "hex": "03000000"},
                                {"source": "r2+0x0", "hex": "04000000"},
                                {"source": "r3+0x0", "hex": "0d000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 21,
                            "event": "BN_mod_mul_montgomery.after-bn_mul_mont",
                            "byte_samples": [
                                {"source": "*r9+0x0@0x1000", "hex": "0a000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        bn_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_mont_check(bn_args) == 0
        native_args = argparse.Namespace(
            trace_dir=str(root),
            hle_dir=None,
            draw_dir=None,
            program=86,
            profile="native",
            expect_texture_size=None,
            reject_texture_size=None,
            limit=4,
        )
        assert mcpe_text_texture_size_gates(native_args) == ((128, 128), (64, 32))


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Query AEMU texture upload and SDL draw trace manifests"
    )
    parser.add_argument("trace_dir", nargs="?", help="trace root containing hle/ and sdl-draw/")
    parser.add_argument("--hle-dir", help="override HLE upload manifest directory")
    parser.add_argument("--draw-dir", help="override SDL draw manifest directory")
    parser.add_argument("--gles-events", help="override GLES event JSONL path")
    parser.add_argument("--native-log", help="override native stderr trace log path")
    parser.add_argument("--native-events", help="override native event JSONL path")
    parser.add_argument("--self-test", action="store_true", help="run built-in self-test")
    subparsers = parser.add_subparsers(dest="command")

    subparsers.add_parser("summary", help="print manifest counts and captured programs/textures")

    texture = subparsers.add_parser("texture", help="show uploads and draw uses for one texture")
    texture.add_argument("texture", type=parse_u32)
    texture.add_argument("--limit", type=int, default=40)

    event = subparsers.add_parser("gles-event", help="show GLES event timeline rows by index")
    event.add_argument("event", type=parse_u32)
    event.add_argument("--context", type=int, default=3)
    event.add_argument("--limit", type=int, default=40)

    native_event = subparsers.add_parser(
        "native-event", help="show structured native PC event trace rows"
    )
    native_event.add_argument("--pc", type=parse_u32)
    native_event.add_argument("--contains")
    native_event.add_argument("--limit", type=int, default=40)

    bn_mont = subparsers.add_parser(
        "bn-mont-check",
        help="check bn_mul_mont native trace rows against a host Montgomery oracle",
    )
    bn_mont.add_argument("--limit", type=int, default=10)

    program = subparsers.add_parser("program", help="show captured draws for one GL program")
    program.add_argument("program", type=parse_u32)
    program.add_argument("--limit", type=int, default=40)
    program.add_argument("--expect-texture-size", type=parse_size)
    program.add_argument("--reject-texture-size", type=parse_size)
    program.add_argument("--json", action="store_true")

    mcpe = subparsers.add_parser("mcpe-text", help="gate MCPE text draw texture binding")
    mcpe.add_argument("--program", type=parse_u32, default=86)
    mcpe.add_argument(
        "--profile",
        choices=("hle", "native"),
        default="hle",
        help="default expected atlas size profile",
    )
    mcpe.add_argument("--expect-texture-size", type=parse_size)
    mcpe.add_argument("--reject-texture-size", type=parse_size)
    mcpe.add_argument("--limit", type=int, default=40)

    texture_data = subparsers.add_parser(
        "mcpe-texturedata",
        help="summarize MCPE TextureData native trace fallbacks from run.log",
    )
    texture_data.add_argument("--empty-only", action="store_true", default=True)
    texture_data.add_argument("--all", action="store_false", dest="empty_only")
    texture_data.add_argument("--limit", type=int, default=40)
    return parser


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.self_test:
        run_self_test()
        print("trace_query self-test ok")
        return 0
    if not args.trace_dir:
        raise SystemExit("trace_dir is required unless --self-test is used")
    if args.command == "summary":
        return print_summary(args)
    if args.command == "texture":
        return print_texture(args)
    if args.command == "gles-event":
        return print_gles_event(args)
    if args.command == "native-event":
        return print_native_event(args)
    if args.command == "bn-mont-check":
        return print_bn_mont_check(args)
    if args.command == "program":
        return print_program(args)
    if args.command == "mcpe-text":
        return print_mcpe_text(args)
    if args.command == "mcpe-texturedata":
        return print_mcpe_texturedata(args)
    raise SystemExit("command is required unless --self-test is used")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except TraceQueryError as err:
        print(f"trace_query failed: {err}", file=sys.stderr)
        raise SystemExit(1) from err
