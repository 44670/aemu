#!/usr/bin/env python3
import argparse
import json
import pathlib
import re
import sys
import tempfile
from collections import Counter


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


def pc_profile_path(args) -> pathlib.Path:
    root = pathlib.Path(args.trace_dir)
    return pathlib.Path(args.pc_profile) if args.pc_profile else root / "pc_profile.jsonl"


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


def load_pc_profile(args) -> tuple[list[dict], pathlib.Path, int]:
    path = pc_profile_path(args)
    if not path.exists():
        return [], path, 0
    lines = path.read_text().splitlines()
    rows = []
    invalid_trailing_rows = 0
    for line_no, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as err:
            if line_no == len(lines):
                invalid_trailing_rows += 1
                continue
            raise TraceQueryError(f"{path}:{line_no}: invalid JSON: {err}") from err
        if not isinstance(row, dict):
            raise TraceQueryError(f"{path}:{line_no}: expected JSON object")
        rows.append(row)
    return rows, path, invalid_trailing_rows


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
    gles_kinds = Counter(row.get("kind") for row in gles_events)
    gles_swaps = [row.get("index") for row in gles_events if row.get("kind") == "SwapBuffers"]
    gles_draws = [
        row.get("index")
        for row in gles_events
        if row.get("kind") in ("DrawArrays", "DrawElements")
    ]
    print(f"hle_manifest={hle_manifest} rows={len(uploads)}")
    print(f"draw_manifest={draw_manifest} rows={len(draws)}")
    print(f"gles_events={gles_path} rows={len(gles_events)}")
    print(f"native_events={native_path} rows={len(native_events)}")
    print(
        "gles_kinds="
        + (", ".join(f"{kind}:{count}" for kind, count in gles_kinds.most_common()) or "none")
    )
    print(
        f"gles_swaps={len(gles_swaps)}"
        f" first={gles_swaps[0] if gles_swaps else 'none'}"
        f" last={gles_swaps[-1] if gles_swaps else 'none'}"
    )
    print(
        f"gles_draws={len(gles_draws)}"
        f" first={gles_draws[0] if gles_draws else 'none'}"
        f" last={gles_draws[-1] if gles_draws else 'none'}"
    )
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


def pc_profile_entry_matches(row: dict, args) -> bool:
    if args.thread is not None and row.get("thread_id") != args.thread:
        return False
    if args.library and row.get("library") != args.library:
        return False
    if args.contains:
        needle = args.contains.lower()
        fields = [
            row.get("symbol"),
            row.get("library"),
            row.get("op"),
            row.get("instr_hex"),
            row.get("pc_hex"),
            row.get("object_offset_hex"),
            row.get("symbol_offset_hex"),
        ]
        if not any(isinstance(field, str) and needle in field.lower() for field in fields):
            return False
    return True


def format_pc_profile_entry(row: dict) -> str:
    symbol = row.get("symbol") or row.get("pc_hex")
    offset = row.get("symbol_offset_hex")
    symbol_text = symbol if not offset else f"{symbol}+{offset}"
    return " ".join(
        [
            f"rank={row.get('rank')}",
            f"count={row.get('count')}",
            f"thread={row.get('thread_id')}",
            f"isa={row.get('isa')}",
            f"pc={row.get('pc_hex')}",
            f"instr={row.get('instr_hex', '<unknown>')}",
            f"op={row.get('op', '<unknown>')}",
            f"library={row.get('library', '<unknown>')}",
            f"object_offset={row.get('object_offset_hex', '<unknown>')}",
            f"symbol={symbol_text}",
        ]
    )


def pc_profile_symbol_name(row: dict) -> str:
    symbol = row.get("symbol")
    if isinstance(symbol, str) and symbol:
        return symbol
    pc = row.get("pc_hex")
    return pc if isinstance(pc, str) and pc else "<unknown>"


def print_pc_profile_symbols(rows: list[dict], limit: int) -> None:
    symbols = Counter()
    symbol_rows = Counter()
    symbol_threads = {}
    for row in rows:
        count = row.get("count") if isinstance(row.get("count"), int) else 0
        key = (row.get("library", "<unknown>"), pc_profile_symbol_name(row))
        symbols[key] += count
        symbol_rows[key] += 1
        thread = row.get("thread_id")
        symbol_threads.setdefault(key, Counter())[thread] += count
    print("pc-profile-symbols:")
    for (library, symbol), count in symbols.most_common(limit):
        threads = ", ".join(
            f"{thread}:{thread_count}"
            for thread, thread_count in symbol_threads[(library, symbol)].most_common(4)
        )
        print(
            f"  count={count} rows={symbol_rows[(library, symbol)]} "
            f"library={library} symbol={symbol} threads={threads or 'none'}"
        )


def print_pc_profile(args) -> int:
    rows, path, invalid_trailing_rows = load_pc_profile(args)
    if not rows:
        raise TraceQueryError(f"{path}: missing or empty PC profile trace")
    snapshot = rows[-1]
    top_rows = snapshot.get("top")
    if not isinstance(top_rows, list):
        raise TraceQueryError(f"{path}: latest PC profile snapshot has no top array")
    filtered = [
        row
        for row in top_rows
        if isinstance(row, dict) and pc_profile_entry_matches(row, args)
    ]
    libraries = Counter()
    threads = Counter()
    ops = Counter()
    for row in filtered:
        count = row.get("count") if isinstance(row.get("count"), int) else 0
        libraries[row.get("library", "<unknown>")] += count
        threads[row.get("thread_id")] += count
        ops[row.get("op", "<unknown>")] += count
    print(
        "pc-profile: "
        f"path={path} rows={len(rows)} invalid_trailing_rows={invalid_trailing_rows} "
        f"samples={snapshot.get('samples')} "
        f"guest_instructions={snapshot.get('guest_instructions')} "
        f"unique_buckets={snapshot.get('unique_buckets')} "
        f"interval={snapshot.get('interval')} "
        f"top_rows={len(top_rows)} filtered_rows={len(filtered)}"
    )
    print(
        "pc-profile-libraries: "
        + (
            ", ".join(f"{library}:{count}" for library, count in libraries.most_common(10))
            or "none"
        )
    )
    print(
        "pc-profile-threads: "
        + (", ".join(f"{thread}:{count}" for thread, count in threads.most_common(10)) or "none")
    )
    print(
        "pc-profile-ops: "
        + (", ".join(f"{op}:{count}" for op, count in ops.most_common(12)) or "none")
    )
    if args.symbols:
        print_pc_profile_symbols(filtered, args.limit)
    for row in filtered[: args.limit]:
        print(format_pc_profile_entry(row))
    return 0


THREAD_ACTION_RE = re.compile(
    r"^THREAD (?P<action>create|skip|wait|wake|abort|signal|condwait|stall)\b(?P<rest>.*)$"
)
THREAD_SLICE_RE = re.compile(
    r"^THREAD slice id=(?P<id>\d+) done=(?P<done>\w+) "
    r"pc=(?P<pc>0x[0-9a-fA-F]+) (?P<isa>\w+)"
)
THREAD_WAIT_RE = re.compile(r"id=(?P<id>\d+) (?P<wait>.+)$")
THREAD_WAKE_RE = re.compile(
    r"id=(?P<id>\d+) (?:(?:cond=(?P<cond>0x[0-9a-fA-F]+) "
    r"mutex=(?P<mutex>0x[0-9a-fA-F]+) wait=(?P<wait>\S+))|(?:mutex=(?P<wake_mutex>0x[0-9a-fA-F]+)))"
)


def parse_thread_log(args) -> tuple[dict, pathlib.Path]:
    path = native_log_path(args)
    if not path.exists():
        raise TraceQueryError(f"{path}: missing run log")
    actions = Counter()
    creates = []
    skips = []
    waits = []
    wakes = []
    signals = []
    condwaits = []
    aborts = []
    stalls = []
    slices = Counter()
    slice_pcs = Counter()
    line_count = 0
    for line in path.read_text(errors="replace").splitlines():
        if not line.startswith("THREAD "):
            continue
        line_count += 1
        if match := THREAD_SLICE_RE.match(line):
            thread = int(match.group("id"))
            actions["slice"] += 1
            slices[thread] += 1
            slice_pcs[(thread, match.group("pc"), match.group("isa"))] += 1
            continue
        match = THREAD_ACTION_RE.match(line)
        if not match:
            actions["other"] += 1
            continue
        action = match.group("action")
        rest = match.group("rest").strip()
        actions[action] += 1
        if action in ("create", "skip"):
            values = dict(re.findall(r"([a-z_]+)=(\S+)", rest))
            row = {
                "raw": rest,
                "id": int(values["id"]) if values.get("id", "").isdigit() else None,
                "start": values.get("start"),
                "arg": values.get("arg"),
                "entry": values.get("entry"),
                "entry_lib": values.get("entry_lib"),
                "library": values.get("library"),
                "sp": values.get("sp"),
            }
            (creates if action == "create" else skips).append(row)
        elif action == "wait":
            item = THREAD_WAIT_RE.search(rest)
            waits.append(
                {
                    "raw": rest,
                    "id": int(item.group("id")) if item else None,
                    "wait": item.group("wait") if item else rest,
                }
            )
        elif action == "wake":
            item = THREAD_WAKE_RE.search(rest)
            wakes.append(
                {
                    "raw": rest,
                    "id": int(item.group("id")) if item else None,
                    "cond": item.group("cond") if item else None,
                    "mutex": (item.group("mutex") or item.group("wake_mutex")) if item else None,
                    "wait": item.group("wait") if item else None,
                }
            )
        elif action in ("signal", "condwait"):
            values = dict(re.findall(r"([a-z_]+)=(\S+)", rest))
            row = {
                "raw": rest,
                "id": int(values["id"]) if values.get("id", "").isdigit() else None,
                "name": values.get("name"),
                "pc": values.get("pc"),
                "cond": values.get("cond"),
                "mutex": values.get("mutex"),
                "timeout": values.get("timeout"),
                "waiters_before": (
                    int(values["waiters_before"])
                    if values.get("waiters_before", "").isdigit()
                    else None
                ),
            }
            (signals if action == "signal" else condwaits).append(row)
        elif action == "abort":
            aborts.append({"raw": rest})
        elif action == "stall":
            stalls.append({"raw": rest})
    return (
        {
            "line_count": line_count,
            "actions": actions,
            "creates": creates,
            "skips": skips,
            "waits": waits,
            "wakes": wakes,
            "signals": signals,
            "condwaits": condwaits,
            "aborts": aborts,
            "stalls": stalls,
            "slices": slices,
            "slice_pcs": slice_pcs,
        },
        path,
    )


def start_key(row: dict) -> tuple[str | None, str | None, str | None]:
    return row.get("start"), row.get("library"), row.get("entry_lib")


def print_thread_summary(args) -> int:
    data, path = parse_thread_log(args)
    print(f"thread-summary: path={path} thread_lines={data['line_count']}")
    actions = data["actions"]
    print(
        "thread-actions: "
        + (", ".join(f"{name}:{count}" for name, count in actions.most_common()) or "none")
    )
    if data["slices"]:
        print(
            "thread-slices: "
            + ", ".join(
                f"{thread}:{count}" for thread, count in data["slices"].most_common(args.limit)
            )
        )
    created = Counter(start_key(row) for row in data["creates"])
    skipped = Counter(start_key(row) for row in data["skips"])
    waits = Counter(row.get("wait") for row in data["waits"])
    wakes = Counter(
        (row.get("cond"), row.get("mutex"), row.get("wait")) for row in data["wakes"]
    )
    signals = Counter(
        (row.get("name"), row.get("cond"), row.get("waiters_before"))
        for row in data["signals"]
    )
    condwaits = Counter(
        (row.get("name"), row.get("cond"), row.get("mutex"))
        for row in data["condwaits"]
    )
    print("thread-created-starts:")
    for (start, library, entry_lib), count in created.most_common(args.limit):
        print(f"  count={count} start={start} library={library} entry_lib={entry_lib}")
    print("thread-skipped-starts:")
    for (start, library, entry_lib), count in skipped.most_common(args.limit):
        print(f"  count={count} start={start} library={library} entry_lib={entry_lib}")
    print("thread-waits:")
    for wait, count in waits.most_common(args.limit):
        print(f"  count={count} wait={wait}")
    print("thread-wakes:")
    for (cond, mutex, wait), count in wakes.most_common(args.limit):
        print(f"  count={count} cond={cond} mutex={mutex} wait={wait}")
    print("thread-cond-signals:")
    for (name, cond, waiters_before), count in signals.most_common(args.limit):
        print(
            f"  count={count} name={name} cond={cond} waiters_before={waiters_before}"
        )
    print("thread-cond-waits:")
    for (name, cond, mutex), count in condwaits.most_common(args.limit):
        print(f"  count={count} name={name} cond={cond} mutex={mutex}")
    if data["stalls"]:
        print("thread-stalls:")
        for row in data["stalls"][-args.limit:]:
            print(f"  {row['raw']}")
    if args.slice_pcs:
        print("thread-slice-pcs:")
        for (thread, pc, isa), count in data["slice_pcs"].most_common(args.limit):
            print(f"  count={count} thread={thread} pc={pc} isa={isa}")
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


RESOURCE_WORK_EVENT = "ResourcePackManager::preloadTextures.work-load-texture.entry"
RESOURCE_DONE_ENTRY_EVENT = "ResourcePackManager::preloadTextures.done-callback.entry"
RESOURCE_DONE_LOAD_EVENT = "ResourcePackManager::preloadTextures.done-callback.load-count"
RESOURCE_DONE_CHECK_EVENT = "ResourcePackManager::preloadTextures.done-callback.check-count"
RESOURCE_DONE_FINAL_LOAD_EVENT = (
    "ResourcePackManager::preloadTextures.done-callback.final-callback-load"
)
RESOURCE_DONE_FINAL_CALL_EVENT = (
    "ResourcePackManager::preloadTextures.done-callback.final-callback-call"
)
MCPE_ON_RESOURCES_LOADED_EVENT = "MinecraftClient::onResourcesLoaded.entry"
MCPE_ON_RESOURCES_LOADED_STORE_EVENT = "MinecraftClient::onResourcesLoaded.store-23e"
MCPE_RENDER_RESOURCE_GATE_EVENT = "GameRenderer::render.resource-ready-gate"


def resource_progress_summary(native_events: list[dict], gles_events: list[dict]) -> dict:
    event_counts = Counter(row.get("event") for row in native_events)
    gles_counts = Counter(row.get("kind") for row in gles_events)
    rows_by_event = {}
    for row in native_events:
        event = row.get("event")
        if isinstance(event, str):
            rows_by_event.setdefault(event, []).append(row)
    gate_rows = [
        row for row in native_events if row.get("event") == MCPE_RENDER_RESOURCE_GATE_EVENT
    ]
    last_gate = gate_rows[-1] if gate_rows else None
    gate_bytes = byte_sample(last_gate, "r7+0x238") if last_gate else None
    ready_byte = gate_bytes[6] if gate_bytes is not None and len(gate_bytes) > 6 else None
    done_load_values = [
        value
        for row in native_events
        if row.get("event") == RESOURCE_DONE_LOAD_EVENT
        for value in [mem32_field_value(row, "r0", "r0+0x0")]
        if value is not None
    ]
    done_check_values = [
        value
        for row in native_events
        if row.get("event") == RESOURCE_DONE_CHECK_EVENT
        for value in [mem32_field_value(row, "r4", "r4+0x60")]
        if value is not None
    ]
    return {
        "event_counts": event_counts,
        "gles_counts": gles_counts,
        "gate_count": len(gate_rows),
        "last_gate": last_gate,
        "last_work": (rows_by_event.get(RESOURCE_WORK_EVENT) or [None])[-1],
        "last_done_entry": (rows_by_event.get(RESOURCE_DONE_ENTRY_EVENT) or [None])[-1],
        "last_done_check": (rows_by_event.get(RESOURCE_DONE_CHECK_EVENT) or [None])[-1],
        "last_done_final_call": (rows_by_event.get(RESOURCE_DONE_FINAL_CALL_EVENT) or [None])[-1],
        "last_on_resources_loaded": (rows_by_event.get(MCPE_ON_RESOURCES_LOADED_EVENT) or [None])[
            -1
        ],
        "gate_bytes": gate_bytes,
        "ready_byte": ready_byte,
        "work_load": event_counts.get(RESOURCE_WORK_EVENT, 0),
        "done_entry": event_counts.get(RESOURCE_DONE_ENTRY_EVENT, 0),
        "done_load": event_counts.get(RESOURCE_DONE_LOAD_EVENT, 0),
        "done_check": event_counts.get(RESOURCE_DONE_CHECK_EVENT, 0),
        "done_final_load": event_counts.get(RESOURCE_DONE_FINAL_LOAD_EVENT, 0),
        "done_final_call": event_counts.get(RESOURCE_DONE_FINAL_CALL_EVENT, 0),
        "on_resources_loaded": event_counts.get(MCPE_ON_RESOURCES_LOADED_EVENT, 0),
        "on_resources_loaded_store": event_counts.get(MCPE_ON_RESOURCES_LOADED_STORE_EVENT, 0),
        "done_load_first": done_load_values[0] if done_load_values else None,
        "done_load_last": done_load_values[-1] if done_load_values else None,
        "done_load_min": min(done_load_values) if done_load_values else None,
        "done_check_first": done_check_values[0] if done_check_values else None,
        "done_check_last": done_check_values[-1] if done_check_values else None,
        "done_check_min": min(done_check_values) if done_check_values else None,
        "gles_swaps": gles_counts.get("SwapBuffers", 0),
        "gles_draws": gles_counts.get("DrawArrays", 0) + gles_counts.get("DrawElements", 0),
    }


def format_resource_position(row: dict | None) -> str:
    if not row:
        return "none"
    return (
        f"step={row.get('step')}"
        f"/gles_next={row.get('gles_next_event_index')}"
        f"/thread={row.get('thread')}"
    )


def print_resource_progress(args) -> int:
    native_events, native_path = load_native_events(args)
    gles_events, gles_path = load_gles_events(args)
    summary = resource_progress_summary(native_events, gles_events)
    event_counts = summary["event_counts"]
    gles_counts = summary["gles_counts"]
    print(
        f"resource-progress: native_events={native_path} rows={len(native_events)} "
        f"gles_events={gles_path} rows={len(gles_events)}"
    )
    print(
        "resource-events: "
        f"work_load={summary['work_load']} "
        f"done_entry={summary['done_entry']} "
        f"done_load={summary['done_load']} "
        f"done_check={summary['done_check']} "
        f"done_final_load={summary['done_final_load']} "
        f"done_final_call={summary['done_final_call']} "
        f"onResourcesLoaded={summary['on_resources_loaded']} "
        f"onResourcesLoadedStore={summary['on_resources_loaded_store']}"
    )
    print(
        "resource-done-counter: "
        f"load_first={summary['done_load_first']} "
        f"load_last={summary['done_load_last']} "
        f"load_min={summary['done_load_min']} "
        f"check_first={summary['done_check_first']} "
        f"check_last={summary['done_check_last']} "
        f"check_min={summary['done_check_min']}"
    )
    last_gate = summary["last_gate"]
    if last_gate:
        gate_bytes = summary["gate_bytes"]
        gate_hex = gate_bytes.hex() if gate_bytes is not None else "none"
        print(
            "resource-gate: "
            f"count={summary['gate_count']} "
            f"last_step={last_gate.get('step')} "
            f"gles_next={last_gate.get('gles_next_event_index')} "
            f"client_23c={mem32_field_value(last_gate, 'r7', 'r7+0x23c')} "
            f"ready_byte_23e={summary['ready_byte']} "
            f"bytes_238={gate_hex}"
        )
    else:
        print("resource-gate: count=0")
    print(
        "resource-last: "
        f"work={format_resource_position(summary['last_work'])} "
        f"done_entry={format_resource_position(summary['last_done_entry'])} "
        f"done_check={format_resource_position(summary['last_done_check'])} "
        f"final_call={format_resource_position(summary['last_done_final_call'])} "
        f"onResourcesLoaded={format_resource_position(summary['last_on_resources_loaded'])}"
    )
    print(
        "resource-gles: "
        + (", ".join(f"{kind}:{count}" for kind, count in gles_counts.most_common()) or "none")
        + f" swaps={summary['gles_swaps']} draws={summary['gles_draws']}"
    )
    print("resource-native-events:")
    for event, count in event_counts.most_common(args.limit):
        print(f"  count={count} event={event}")
    return 0


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


def bn_mod_sqr_sequences(events: list[dict]) -> list[dict]:
    sequences = []
    pending_by_thread: dict[int | None, list[dict]] = {}
    for row in events:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "BN_mod_sqr.entry":
            pending_by_thread.setdefault(thread, []).append({"entry": row})
        elif event == "BN_mod_sqr.after-bn-sqr":
            pending = pending_by_thread.get(thread)
            if pending:
                pending[-1]["after_sqr"] = row
        elif event in ("BN_mod_sqr.before-bn-div", "BN_mod_sqr.before-bn-nnmod"):
            pending = pending_by_thread.get(thread)
            if pending:
                pending[-1]["before_div"] = row
        elif event == "BN_mod_sqr.return":
            pending = pending_by_thread.get(thread)
            if pending:
                seq = pending.pop()
                seq["return"] = row
                sequences.append(seq)
    return sequences


def bignum_value(
    row: dict, base: str, sample_prefix: str, name: str
) -> tuple[int | None, int | None, str | None, str | None]:
    top = mem32_field_value(row, base, f"{base}+0x4")
    raw = byte_sample(row, sample_prefix)
    if not isinstance(top, int):
        return None, None, None, f"missing {name} BIGNUM top"
    if top < 0:
        return None, top, None, f"negative {name} BIGNUM top {top}"
    if raw is None:
        return None, top, None, f"missing {name} BIGNUM byte sample"
    required_len = top * 4
    if len(raw) < required_len:
        return None, top, raw.hex(), f"short {name} BIGNUM sample for top={top}"
    return little_int(raw, top), top, raw[:required_len].hex(), None


def check_bn_mod_sqr_sequence(seq: dict) -> tuple[bool, dict]:
    entry = seq["entry"]
    ret = seq["return"]
    value, value_top, value_hex, value_err = bignum_value(entry, "r1", "*r1+0", "input")
    modulus, modulus_top, modulus_hex, modulus_err = bignum_value(
        entry, "r2", "*r2+0", "modulus"
    )
    output, output_top, output_hex, output_err = bignum_value(ret, "r6", "*r6+0", "output")
    detail = {
        "entry_step": entry.get("step"),
        "return_step": ret.get("step"),
        "input_top": value_top,
        "modulus_top": modulus_top,
        "output_top": output_top,
    }
    errors = [err for err in (value_err, modulus_err, output_err) if err]
    if errors:
        detail["error"] = "; ".join(errors)
        return False, detail
    if modulus is None or modulus <= 0:
        detail["error"] = f"invalid modulus 0x{modulus or 0:x}"
        return False, detail
    assert value is not None
    assert output is not None
    assert modulus_top is not None
    after_sqr = seq.get("after_sqr")
    if isinstance(after_sqr, dict):
        square, square_top, square_hex, square_err = bignum_value(
            after_sqr, "r6", "*r6+0", "square"
        )
        detail["square_step"] = after_sqr.get("step")
        detail["square_top"] = square_top
        if square_err:
            detail["square_error"] = square_err
        elif square is not None:
            expected_square = value * value
            square_words = max(square_top or 0, (expected_square.bit_length() + 31) // 32, 1)
            detail["square_actual"] = little_hex(square, square_words)
            detail["square_expected"] = little_hex(expected_square, square_words)
            detail["square_ok"] = square == expected_square
    before_div = seq.get("before_div")
    if isinstance(before_div, dict):
        before_value, before_top, before_hex, before_err = bignum_value(
            before_div, "r6", "*r6+0", "pre-reduce"
        )
        detail["before_div_step"] = before_div.get("step")
        detail["before_div_top"] = before_top
        if before_err:
            detail["before_div_error"] = before_err
        elif before_value is not None:
            detail["before_div"] = before_hex
            if "square_ok" in detail:
                detail["before_div_matches_square"] = before_value == value * value
    expected = (value * value) % modulus
    compare_words = max(output_top or 0, modulus_top, 1)
    detail["actual"] = little_hex(output, compare_words)
    detail["expected"] = little_hex(expected, compare_words)
    detail["input"] = value_hex
    detail["modulus"] = modulus_hex
    return output == expected and detail.get("square_ok", True), detail


def print_bn_mod_sqr_check(args) -> int:
    events, path = load_native_events(args)
    sequences = bn_mod_sqr_sequences(events)
    if not sequences:
        raise TraceQueryError(f"{path}: no BN_mod_sqr.entry -> BN_mod_sqr.return pairs found")
    checked = 0
    mismatches = []
    incomplete = []
    for seq in sequences:
        ok, detail = check_bn_mod_sqr_sequence(seq)
        if "error" in detail:
            incomplete.append(detail)
            continue
        checked += 1
        if not ok:
            mismatches.append(detail)

    print(
        f"bn-mod-sqr-check: path={path} pairs={len(sequences)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"input_top={detail.get('input_top')} modulus_top={detail.get('modulus_top')} "
            f"output_top={detail.get('output_top')}"
        )
        print(f"  actual  ={detail.get('actual')}")
        print(f"  expected={detail.get('expected')}")
        if "square_ok" in detail:
            print(
                f"  square_ok={detail.get('square_ok')} "
                f"square_step={detail.get('square_step')} square_top={detail.get('square_top')}"
            )
            print(f"  square_actual  ={detail.get('square_actual')}")
            print(f"  square_expected={detail.get('square_expected')}")
        if "before_div_matches_square" in detail:
            print(
                f"  before_div_matches_square={detail.get('before_div_matches_square')} "
                f"before_div_step={detail.get('before_div_step')} "
                f"before_div_top={detail.get('before_div_top')}"
            )
        print(f"  input   ={detail.get('input')}")
        print(f"  modulus ={detail.get('modulus')}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"input_top={detail.get('input_top')} modulus_top={detail.get('modulus_top')} "
            f"output_top={detail.get('output_top')} error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


def bn_nnmod_sequences(events: list[dict]) -> list[dict]:
    sequences = []
    pending_by_thread: dict[int | None, list[dict]] = {}
    for row in events:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "BN_nnmod.entry":
            pending_by_thread.setdefault(thread, []).append({"entry": row})
        elif event == "BN_nnmod.after-bn-div":
            pending = pending_by_thread.get(thread)
            if pending:
                pending[-1]["after_div"] = row
        elif event == "BN_nnmod.before-corrective-add":
            pending = pending_by_thread.get(thread)
            if pending:
                pending[-1]["before_corrective_add"] = row
        elif event == "BN_nnmod.return":
            pending = pending_by_thread.get(thread)
            if pending:
                seq = pending.pop()
                seq["return"] = row
                sequences.append(seq)
    return sequences


def check_bn_nnmod_sequence(seq: dict) -> tuple[bool, dict]:
    entry = seq["entry"]
    ret = seq["return"]
    value, value_top, value_hex, value_err = bignum_value(entry, "r1", "*r1+0", "input")
    modulus, modulus_top, modulus_hex, modulus_err = bignum_value(
        entry, "r2", "*r2+0", "modulus"
    )
    output, output_top, output_hex, output_err = bignum_value(ret, "r4", "*r4+0", "output")
    input_neg = mem32_field_value(entry, "r1", "r1+0xc") == 1
    output_neg = mem32_field_value(ret, "r4", "r4+0xc") == 1
    detail = {
        "entry_step": entry.get("step"),
        "return_step": ret.get("step"),
        "input_top": value_top,
        "modulus_top": modulus_top,
        "output_top": output_top,
        "input_neg": input_neg,
        "output_neg": output_neg,
    }
    errors = [err for err in (value_err, modulus_err, output_err) if err]
    if errors:
        detail["error"] = "; ".join(errors)
        return False, detail
    if modulus is None or modulus <= 0:
        detail["error"] = f"invalid modulus 0x{modulus or 0:x}"
        return False, detail
    assert value is not None
    assert output is not None
    assert modulus_top is not None
    signed_value = -value if input_neg else value
    expected = signed_value % modulus
    expected_div_remainder = value % modulus if not input_neg else -(value % modulus)
    signed_output = -output if output_neg else output
    compare_words = max(output_top or 0, modulus_top, 1)
    detail["actual"] = little_hex(output, compare_words)
    detail["expected"] = little_hex(expected, compare_words)
    detail["input"] = value_hex
    detail["modulus"] = modulus_hex
    after_div = seq.get("after_div")
    if isinstance(after_div, dict):
        remainder, remainder_top, _remainder_hex, remainder_err = bignum_value(
            after_div, "r4", "*r4+0", "BN_div remainder"
        )
        detail["after_div_step"] = after_div.get("step")
        detail["after_div_top"] = remainder_top
        detail["after_div_neg"] = mem32_field_value(after_div, "r4", "r4+0xc")
        if remainder_err:
            detail["after_div_error"] = remainder_err
        elif remainder is not None:
            detail["after_div_actual"] = little_hex(remainder, compare_words)
            remainder_neg = detail["after_div_neg"] == 1
            signed_remainder = -remainder if remainder_neg else remainder
            detail["after_div_ok"] = signed_remainder == expected_div_remainder
    return signed_output == expected and detail.get("after_div_ok", True), detail


def print_bn_nnmod_check(args) -> int:
    events, path = load_native_events(args)
    sequences = bn_nnmod_sequences(events)
    if not sequences:
        raise TraceQueryError(f"{path}: no BN_nnmod.entry -> BN_nnmod.return pairs found")
    checked = 0
    mismatches = []
    incomplete = []
    for seq in sequences:
        ok, detail = check_bn_nnmod_sequence(seq)
        if "error" in detail:
            incomplete.append(detail)
            continue
        checked += 1
        if not ok:
            mismatches.append(detail)

    print(
        f"bn-nnmod-check: path={path} pairs={len(sequences)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"input_top={detail.get('input_top')} modulus_top={detail.get('modulus_top')} "
            f"output_top={detail.get('output_top')} input_neg={detail.get('input_neg')} "
            f"output_neg={detail.get('output_neg')}"
        )
        print(f"  actual  ={detail.get('actual')}")
        print(f"  expected={detail.get('expected')}")
        if "after_div_ok" in detail:
            print(
                f"  after_div_ok={detail.get('after_div_ok')} "
                f"after_div_step={detail.get('after_div_step')} "
                f"after_div_top={detail.get('after_div_top')} "
                f"after_div_neg={detail.get('after_div_neg')}"
            )
            print(f"  after_div_actual={detail.get('after_div_actual')}")
        print(f"  input   ={detail.get('input')}")
        print(f"  modulus ={detail.get('modulus')}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"input_top={detail.get('input_top')} modulus_top={detail.get('modulus_top')} "
            f"output_top={detail.get('output_top')} error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


def bn_div_words_pairs(events: list[dict]) -> list[tuple[dict, dict]]:
    pairs = []
    pending_by_thread: dict[int | None, list[dict]] = {}
    for row in events:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "bn_div_words.entry":
            pending_by_thread.setdefault(thread, []).append(row)
        elif event == "bn_div_words.return":
            pending = pending_by_thread.get(thread)
            if pending:
                pairs.append((pending.pop(), row))
    return pairs


def check_bn_div_words_pair(entry: dict, ret: dict) -> tuple[bool, dict]:
    high = row_int(entry, "r0")
    low = row_int(entry, "r1")
    divisor = row_int(entry, "r2")
    actual = row_int(ret, "r0")
    detail = {
        "entry_step": entry.get("step"),
        "return_step": ret.get("step"),
        "high": high,
        "low": low,
        "divisor": divisor,
        "actual": actual,
    }
    if high is None or low is None or divisor is None or actual is None:
        detail["error"] = "missing entry or return register"
        return False, detail
    if divisor == 0:
        detail["error"] = "zero divisor"
        return False, detail
    numerator = ((high & 0xFFFF_FFFF) << 32) | (low & 0xFFFF_FFFF)
    expected = (numerator // divisor) & 0xFFFF_FFFF
    detail["expected"] = expected
    detail["numerator"] = numerator
    return actual == expected, detail


def print_bn_div_words_check(args) -> int:
    events, path = load_native_events(args)
    pairs = bn_div_words_pairs(events)
    if not pairs:
        raise TraceQueryError(f"{path}: no bn_div_words.entry -> bn_div_words.return pairs found")
    checked = 0
    mismatches = []
    incomplete = []
    for entry, ret in pairs:
        ok, detail = check_bn_div_words_pair(entry, ret)
        if "error" in detail:
            incomplete.append(detail)
            continue
        checked += 1
        if not ok:
            mismatches.append(detail)
    print(
        f"bn-div-words-check: path={path} pairs={len(pairs)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"high=0x{detail.get('high'):08x} low=0x{detail.get('low'):08x} "
            f"divisor=0x{detail.get('divisor'):08x}"
        )
        print(f"  numerator=0x{detail.get('numerator'):016x}")
        print(f"  actual=0x{detail.get('actual'):08x} expected=0x{detail.get('expected'):08x}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"entry_step={detail.get('entry_step')} return_step={detail.get('return_step')} "
            f"error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


def bn_div_sequences(events: list[dict]) -> list[dict]:
    sequences = []
    pending_by_thread: dict[int | None, list[dict]] = {}
    for row in events:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "BN_div.entry":
            pending_by_thread.setdefault(thread, []).append(
                {"entry": row, "track_remainder": row_int(row, "r1") not in (None, 0)}
            )
        elif event == "BN_div.before-rem-rshift":
            pending = pending_by_thread.get(thread)
            if pending and pending[-1].get("track_remainder"):
                pending[-1]["before_rshift"] = row
        elif event == "BN_div.after-rem-rshift":
            pending = pending_by_thread.get(thread)
            if pending:
                seq = pending.pop()
                if seq.get("track_remainder"):
                    seq["after_rshift"] = row
                    sequences.append(seq)
    return sequences


def check_bn_div_sequence(seq: dict) -> tuple[bool, dict]:
    entry = seq["entry"]
    after = seq["after_rshift"]
    value, value_top, value_hex, value_err = bignum_value(entry, "r2", "*r2+0", "dividend")
    divisor, divisor_top, divisor_hex, divisor_err = bignum_value(
        entry, "r3", "*r3+0", "divisor"
    )
    output, output_top, output_hex, output_err = bignum_value(after, "r6", "*r6+0", "remainder")
    value_neg = mem32_field_value(entry, "r2", "r2+0xc") == 1
    output_neg = mem32_field_value(after, "r6", "r6+0xc") == 1
    detail = {
        "entry_step": entry.get("step"),
        "after_step": after.get("step"),
        "dividend_top": value_top,
        "divisor_top": divisor_top,
        "output_top": output_top,
        "dividend_neg": value_neg,
        "output_neg": output_neg,
    }
    errors = [err for err in (value_err, divisor_err, output_err) if err]
    if errors:
        detail["error"] = "; ".join(errors)
        return False, detail
    if divisor is None or divisor <= 0:
        detail["error"] = f"invalid divisor 0x{divisor or 0:x}"
        return False, detail
    assert value is not None
    assert output is not None
    assert divisor_top is not None
    expected_signed = -(value % divisor) if value_neg else value % divisor
    signed_output = -output if output_neg else output
    compare_words = max(output_top or 0, divisor_top, 1)
    detail["actual"] = little_hex(output, compare_words)
    detail["expected"] = little_hex(abs(expected_signed), compare_words)
    detail["expected_neg"] = expected_signed < 0
    detail["dividend"] = value_hex
    detail["divisor"] = divisor_hex

    before = seq.get("before_rshift")
    if isinstance(before, dict):
        pre, pre_top, _pre_hex, pre_err = bignum_value(before, "r7", "*r7+0", "pre-rshift")
        shift = mem32_field_value(before, "sp", "sp+0x28")
        detail["before_step"] = before.get("step")
        detail["before_top"] = pre_top
        detail["rshift"] = shift
        if pre_err:
            detail["before_error"] = pre_err
        elif pre is not None and isinstance(shift, int):
            shifted = pre >> shift
            detail["before_shifted"] = little_hex(shifted, compare_words)
            detail["before_shifted_ok"] = shifted == abs(expected_signed)
    return signed_output == expected_signed and detail.get("before_shifted_ok", True), detail


def print_bn_div_check(args) -> int:
    events, path = load_native_events(args)
    sequences = bn_div_sequences(events)
    if not sequences:
        raise TraceQueryError(f"{path}: no BN_div.entry -> BN_div.after-rem-rshift pairs found")
    checked = 0
    mismatches = []
    incomplete = []
    for seq in sequences:
        ok, detail = check_bn_div_sequence(seq)
        if "error" in detail:
            incomplete.append(detail)
            continue
        checked += 1
        if not ok:
            mismatches.append(detail)
    print(
        f"bn-div-check: path={path} pairs={len(sequences)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"entry_step={detail.get('entry_step')} after_step={detail.get('after_step')} "
            f"dividend_top={detail.get('dividend_top')} divisor_top={detail.get('divisor_top')} "
            f"output_top={detail.get('output_top')} rshift={detail.get('rshift')} "
            f"expected_neg={detail.get('expected_neg')} output_neg={detail.get('output_neg')}"
        )
        print(f"  actual  ={detail.get('actual')}")
        print(f"  expected={detail.get('expected')}")
        if "before_shifted_ok" in detail:
            print(
                f"  before_shifted_ok={detail.get('before_shifted_ok')} "
                f"before_step={detail.get('before_step')} before_top={detail.get('before_top')}"
            )
            print(f"  before_shifted={detail.get('before_shifted')}")
        print(f"  dividend={detail.get('dividend')}")
        print(f"  divisor ={detail.get('divisor')}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"entry_step={detail.get('entry_step')} after_step={detail.get('after_step')} "
            f"error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


def row_reg(row: dict, reg: str) -> int | None:
    value = row.get(reg)
    if isinstance(value, int):
        return value
    regs = row.get("regs")
    if not isinstance(regs, dict):
        return None
    value = regs.get(reg)
    return value if isinstance(value, int) else None


def u32(value: int) -> int:
    return value & 0xffff_ffff


def u64_words(lo: int, hi: int) -> int:
    return u32(lo) | (u32(hi) << 32)


def host_mul_words(src: bytes, words: int, mul: int) -> tuple[int, bytes] | None:
    if words <= 0 or len(src) < words * 4:
        return None
    carry = 0
    out = bytearray(words * 4)
    for idx in range(words):
        word = int.from_bytes(src[idx * 4 : idx * 4 + 4], "little")
        product = word * u32(mul) + carry
        out[idx * 4 : idx * 4 + 4] = u32(product).to_bytes(4, "little")
        carry = product >> 32
    return u32(carry), bytes(out)


def host_sub_words(a: bytes, b: bytes, words: int) -> tuple[int, bytes] | None:
    if words <= 0 or min(len(a), len(b)) < words * 4:
        return None
    borrow = 0
    out = bytearray(words * 4)
    for idx in range(words):
        aw = int.from_bytes(a[idx * 4 : idx * 4 + 4], "little")
        bw = int.from_bytes(b[idx * 4 : idx * 4 + 4], "little")
        value = aw - bw - borrow
        borrow = 1 if value < 0 else 0
        out[idx * 4 : idx * 4 + 4] = u32(value).to_bytes(4, "little")
    return borrow, bytes(out)


def host_add_words(a: bytes, b: bytes, words: int) -> tuple[int, bytes] | None:
    if words <= 0 or min(len(a), len(b)) < words * 4:
        return None
    carry = 0
    out = bytearray(words * 4)
    for idx in range(words):
        aw = int.from_bytes(a[idx * 4 : idx * 4 + 4], "little")
        bw = int.from_bytes(b[idx * 4 : idx * 4 + 4], "little")
        value = aw + bw + carry
        carry = value >> 32
        out[idx * 4 : idx * 4 + 4] = u32(value).to_bytes(4, "little")
    return u32(carry), bytes(out)


def print_bn_div_loop_check(args) -> int:
    events, path = load_native_events(args)
    rows = [row for row in events if str(row.get("event", "")).startswith("BN_div.loop.")]
    if not rows:
        raise TraceQueryError(f"{path}: no BN_div.loop events found")

    pending: dict[tuple[int | None, str], dict] = {}
    counts = {
        "uldiv": 0,
        "mls": 0,
        "umull": 0,
        "mla": 0,
        "mul_words": 0,
        "sub_words": 0,
        "add_words": 0,
        "product_corrections": 0,
    }
    mismatches = []
    incomplete = []
    last_umull_by_thread: dict[int | None, dict] = {}

    def record_mismatch(kind: str, row: dict, expected: str, actual: str, extra: str = "") -> None:
        mismatches.append(
            {
                "kind": kind,
                "step": row.get("step"),
                "pc": row.get("pc"),
                "expected": expected,
                "actual": actual,
                "extra": extra,
            }
        )

    def record_incomplete(kind: str, row: dict, reason: str) -> None:
        incomplete.append(
            {
                "kind": kind,
                "step": row.get("step"),
                "pc": row.get("pc"),
                "reason": reason,
            }
        )

    for row in rows:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "BN_div.loop.uldiv-call":
            pending[(thread, "uldiv")] = row
        elif event == "BN_div.loop.uldiv-ret":
            call = pending.pop((thread, "uldiv"), None)
            if call is None:
                record_incomplete("uldiv", row, "missing call row")
                continue
            call_regs = [row_reg(call, f"r{idx}") for idx in range(4)]
            rets = [row_reg(row, f"r{idx}") for idx in range(4)]
            if any(value is None for value in call_regs + rets):
                record_incomplete("uldiv", row, "missing argument or return registers")
                continue
            numerator = u64_words(call_regs[0], call_regs[1])
            denominator = u64_words(call_regs[2], call_regs[3])
            if denominator == 0:
                record_incomplete("uldiv", row, "zero denominator")
                continue
            quotient = numerator // denominator
            remainder = numerator % denominator
            actual_quotient = u64_words(rets[0], rets[1])
            actual_remainder = u64_words(rets[2], rets[3])
            counts["uldiv"] += 1
            if quotient != actual_quotient or remainder != actual_remainder:
                record_mismatch(
                    "uldiv",
                    row,
                    f"q=0x{quotient:016x} r=0x{remainder:016x}",
                    f"q=0x{actual_quotient:016x} r=0x{actual_remainder:016x}",
                    f"num=0x{numerator:016x} den=0x{denominator:016x}",
                )
        elif event == "BN_div.loop.after-mls":
            r8 = row_reg(row, "r8")
            r2 = row_reg(row, "r2")
            r6 = row_reg(row, "r6")
            lr = row_reg(row, "lr")
            if None in (r8, r2, r6, lr):
                record_incomplete("mls", row, "missing registers")
                continue
            counts["mls"] += 1
            expected = u32(r6 - u32(r8 * r2))
            if expected != u32(lr):
                record_mismatch("mls", row, f"lr=0x{expected:08x}", f"lr=0x{u32(lr):08x}")
        elif event == "BN_div.loop.after-umull":
            r4 = row_reg(row, "r4")
            r10 = row_reg(row, "r10")
            r0 = row_reg(row, "r0")
            r1 = row_reg(row, "r1")
            if None in (r4, r10, r0, r1):
                record_incomplete("umull", row, "missing registers")
                continue
            counts["umull"] += 1
            product = u32(r4) * u32(r10)
            if u32(product) != u32(r0) or u32(product >> 32) != u32(r1):
                record_mismatch(
                    "umull",
                    row,
                    f"r1:r0=0x{product:016x}",
                    f"r1:r0=0x{u64_words(r0, r1):016x}",
                )
            last_umull_by_thread[thread] = row
        elif event == "BN_div.loop.after-mla":
            prev = last_umull_by_thread.get(thread)
            r11 = row_reg(row, "r11")
            r5 = row_reg(row, "r5")
            r1 = row_reg(row, "r1")
            prev_hi = row_reg(prev or {}, "r1")
            if None in (r11, r5, r1, prev_hi):
                record_incomplete("mla", row, "missing registers or prior UMULL")
                continue
            counts["mla"] += 1
            expected = u32(prev_hi + u32(r11 * r5))
            if expected != u32(r1):
                record_mismatch("mla", row, f"r1=0x{expected:08x}", f"r1=0x{u32(r1):08x}")
        elif event == "BN_div.loop.pre-product-cmp":
            r0 = row_reg(row, "r0")
            r1 = row_reg(row, "r1")
            r4 = row_reg(row, "r4")
            r5 = row_reg(row, "r5")
            if None not in (r0, r1, r4, r5) and u64_words(r4, r5) < u64_words(r0, r1):
                counts["product_corrections"] += 1
        elif event == "BN_div.loop.bn-mul-call":
            pending[(thread, "mul_words")] = row
        elif event == "BN_div.loop.bn-mul-ret":
            call = pending.pop((thread, "mul_words"), None)
            if call is None:
                record_incomplete("mul_words", row, "missing call row")
                continue
            words = row_reg(call, "r2")
            mul = row_reg(call, "r3")
            src = byte_sample(call, "r1+0x0")
            out = byte_sample(row, "*r5+0x0")
            ret = row_reg(row, "r0")
            if not isinstance(words, int) or not isinstance(mul, int) or src is None or out is None or ret is None:
                record_incomplete("mul_words", row, "missing words, multiplier, samples, or return")
                continue
            expected = host_mul_words(src, words, mul)
            if expected is None:
                record_incomplete("mul_words", row, f"short samples for {words} words")
                continue
            expected_ret, expected_out = expected
            counts["mul_words"] += 1
            if expected_ret != u32(ret) or out[: words * 4] != expected_out:
                record_mismatch(
                    "mul_words",
                    row,
                    f"ret=0x{expected_ret:08x} out={expected_out.hex()}",
                    f"ret=0x{u32(ret):08x} out={out[: words * 4].hex()}",
                    f"words={words} mul=0x{u32(mul):08x}",
                )
        elif event == "BN_div.loop.bn-sub-call":
            pending[(thread, "sub_words")] = row
        elif event == "BN_div.loop.bn-sub-ret":
            call = pending.pop((thread, "sub_words"), None)
            if call is None:
                record_incomplete("sub_words", row, "missing call row")
                continue
            words = row_reg(call, "r3")
            a = byte_sample(call, "r1+0x0")
            b = byte_sample(call, "r2+0x0")
            out = byte_sample(row, "*sp+0x7c")
            ret = row_reg(row, "r0")
            if not isinstance(words, int) or a is None or b is None or out is None or ret is None:
                record_incomplete("sub_words", row, "missing words, samples, or return")
                continue
            expected = host_sub_words(a, b, words)
            if expected is None:
                record_incomplete("sub_words", row, f"short samples for {words} words")
                continue
            expected_ret, expected_out = expected
            counts["sub_words"] += 1
            if expected_ret != u32(ret) or out[: words * 4] != expected_out:
                record_mismatch(
                    "sub_words",
                    row,
                    f"ret=0x{expected_ret:08x} out={expected_out.hex()}",
                    f"ret=0x{u32(ret):08x} out={out[: words * 4].hex()}",
                    f"words={words}",
                )
        elif event == "BN_div.loop.bn-add-call":
            pending[(thread, "add_words")] = row
        elif event == "BN_div.loop.bn-add-ret":
            call = pending.pop((thread, "add_words"), None)
            if call is None:
                record_incomplete("add_words", row, "missing call row")
                continue
            words = row_reg(call, "r3")
            a = byte_sample(call, "r1+0x0")
            b = byte_sample(call, "r2+0x0")
            out = byte_sample(row, "*sp+0x7c")
            ret = row_reg(row, "r0")
            if not isinstance(words, int) or a is None or b is None or out is None or ret is None:
                record_incomplete("add_words", row, "missing words, samples, or return")
                continue
            expected = host_add_words(a, b, words)
            if expected is None:
                record_incomplete("add_words", row, f"short samples for {words} words")
                continue
            expected_ret, expected_out = expected
            counts["add_words"] += 1
            if expected_ret != u32(ret) or out[: words * 4] != expected_out:
                record_mismatch(
                    "add_words",
                    row,
                    f"ret=0x{expected_ret:08x} out={expected_out.hex()}",
                    f"ret=0x{u32(ret):08x} out={out[: words * 4].hex()}",
                    f"words={words}",
                )

    print(
        f"bn-div-loop-check: path={path} rows={len(rows)} "
        + " ".join(f"{key}={value}" for key, value in counts.items())
        + f" mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"kind={detail['kind']} step={detail['step']} pc={fmt_u32(detail['pc'])} "
            f"expected={detail['expected']} actual={detail['actual']}"
        )
        if detail.get("extra"):
            print(f"  {detail['extra']}")
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"kind={detail['kind']} step={detail['step']} pc={fmt_u32(detail['pc'])} "
            f"reason={detail['reason']}"
        )
    return 1 if mismatches or incomplete else 0


P384_P = int(
    "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
    16,
)
P384_B = int(
    "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
    16,
)
P384_WORDS = 12
P384_BYTES = P384_WORDS * 4
P384_R = 1 << (32 * P384_WORDS)
P384_R_INV = pow(P384_R, -1, P384_P)


def mont_decode_p384(raw: bytes) -> int:
    return (little_int(raw, P384_WORDS) * P384_R_INV) % P384_P


def point_samples(row: dict, reg: str) -> tuple[dict | None, str | None]:
    x_raw = byte_sample(row, f"*{reg}+0x4")
    y_raw = byte_sample(row, f"*{reg}+0x18")
    z_raw = byte_sample(row, f"*{reg}+0x2c")
    if x_raw is None or y_raw is None or z_raw is None:
        return None, f"missing {reg} coordinate byte samples"
    if min(len(x_raw), len(y_raw), len(z_raw)) < P384_BYTES:
        return None, f"short {reg} coordinate byte sample"
    return {
        "reg": reg,
        "x_mont": little_int(x_raw, P384_WORDS),
        "y_mont": little_int(y_raw, P384_WORDS),
        "z_mont": little_int(z_raw, P384_WORDS),
        "x": mont_decode_p384(x_raw),
        "y": mont_decode_p384(y_raw),
        "z": mont_decode_p384(z_raw),
        "z_is_one": mem32_field_value(row, reg, f"{reg}+0x40"),
    }, None


def affine_from_jacobian(point: dict) -> tuple[int, int] | None:
    z = 1 if point.get("z_is_one") == 1 else point["z"]
    if z == 0:
        return None
    z_inv = pow(z, -1, P384_P)
    z2_inv = (z_inv * z_inv) % P384_P
    z3_inv = (z2_inv * z_inv) % P384_P
    return ((point["x"] * z2_inv) % P384_P, (point["y"] * z3_inv) % P384_P)


def p384_add_affine(
    left: tuple[int, int] | None, right: tuple[int, int] | None
) -> tuple[int, int] | None:
    if left is None:
        return right
    if right is None:
        return left
    x1, y1 = left
    x2, y2 = right
    if x1 == x2:
        if (y1 + y2) % P384_P == 0:
            return None
        return p384_double_affine(left)
    slope = ((y2 - y1) * pow((x2 - x1) % P384_P, -1, P384_P)) % P384_P
    x3 = (slope * slope - x1 - x2) % P384_P
    y3 = (slope * (x1 - x3) - y1) % P384_P
    return (x3, y3)


def p384_double_affine(point: tuple[int, int] | None) -> tuple[int, int] | None:
    if point is None:
        return None
    x, y = point
    if y == 0:
        return None
    slope = ((3 * x * x - 3) * pow((2 * y) % P384_P, -1, P384_P)) % P384_P
    x3 = (slope * slope - 2 * x) % P384_P
    y3 = (slope * (x - x3) - y) % P384_P
    return (x3, y3)


def point_on_p384(point: tuple[int, int] | None) -> bool:
    if point is None:
        return True
    x, y = point
    return (y * y - (x * x * x - 3 * x + P384_B)) % P384_P == 0


def fmt_p384_point(point: tuple[int, int] | None) -> str:
    if point is None:
        return "infinity"
    x, y = point
    return f"x={x:096x} y={y:096x}"


def ec_point_pairs(events: list[dict], entry_name: str, return_name: str) -> list[tuple[dict, dict]]:
    pairs = []
    pending = None
    for row in events:
        event = row.get("event")
        if event == entry_name:
            pending = row
        elif event == return_name and pending is not None:
            pairs.append((pending, row))
            pending = None
    return pairs


def check_ec_dbl_pair(entry: dict, ret: dict) -> tuple[bool, dict]:
    src, err = point_samples(entry, "r2")
    if err:
        return False, {"entry_step": entry.get("step"), "error": err}
    out, err = point_samples(ret, "r10")
    if err:
        return False, {"entry_step": entry.get("step"), "after_step": ret.get("step"), "error": err}
    src_affine = affine_from_jacobian(src)
    out_affine = affine_from_jacobian(out)
    expected = p384_double_affine(src_affine)
    return out_affine == expected, {
        "op": "dbl",
        "entry_step": entry.get("step"),
        "after_step": ret.get("step"),
        "input_on_curve": point_on_p384(src_affine),
        "output_on_curve": point_on_p384(out_affine),
        "input": fmt_p384_point(src_affine),
        "actual": fmt_p384_point(out_affine),
        "expected": fmt_p384_point(expected),
    }


def check_ec_add_pair(entry: dict, ret: dict) -> tuple[bool, dict]:
    left, err = point_samples(entry, "r2")
    if err:
        return False, {"entry_step": entry.get("step"), "error": err}
    right, err = point_samples(entry, "r3")
    if err:
        return False, {"entry_step": entry.get("step"), "error": err}
    out, err = point_samples(ret, "r8")
    if err:
        return False, {"entry_step": entry.get("step"), "after_step": ret.get("step"), "error": err}
    left_affine = affine_from_jacobian(left)
    right_affine = affine_from_jacobian(right)
    out_affine = affine_from_jacobian(out)
    expected = p384_add_affine(left_affine, right_affine)
    return out_affine == expected, {
        "op": "add",
        "entry_step": entry.get("step"),
        "after_step": ret.get("step"),
        "left_on_curve": point_on_p384(left_affine),
        "right_on_curve": point_on_p384(right_affine),
        "output_on_curve": point_on_p384(out_affine),
        "left": fmt_p384_point(left_affine),
        "right": fmt_p384_point(right_affine),
        "actual": fmt_p384_point(out_affine),
        "expected": fmt_p384_point(expected),
    }


def print_ec_point_check(args) -> int:
    events, path = load_native_events(args)
    checks = []
    for entry, ret in ec_point_pairs(
        events, "ec_GFp_simple_dbl.entry", "ec_GFp_simple_dbl.return"
    ):
        checks.append(check_ec_dbl_pair(entry, ret))
    for entry, ret in ec_point_pairs(
        events, "ec_GFp_simple_add.entry", "ec_GFp_simple_add.return"
    ):
        checks.append(check_ec_add_pair(entry, ret))
    if not checks:
        raise TraceQueryError(f"{path}: no ec_GFp_simple_dbl/add pairs found")
    failures = [detail for ok, detail in checks if not ok]
    print(
        f"ec-point-check: path={path} checked={len(checks)} "
        f"failures={len(failures)}"
    )
    for detail in failures[: args.limit]:
        print(
            "failure "
            f"op={detail.get('op')} entry_step={detail.get('entry_step')} "
            f"after_step={detail.get('after_step')} error={detail.get('error')}"
        )
        for key in (
            "input_on_curve",
            "left_on_curve",
            "right_on_curve",
            "output_on_curve",
            "input",
            "left",
            "right",
            "actual",
            "expected",
        ):
            if key in detail:
                print(f"  {key}={detail.get(key)}")
    return 1 if failures else 0


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
HLE_RE = re.compile(
    r"HLE function=(?P<function>0x[0-9a-fA-F]+) step=(?P<step>\d+) "
    r"pc=(?P<pc>0x[0-9a-fA-F]+) name=(?P<name>\S+) "
    r"r0=(?P<r0>0x[0-9a-fA-F]+) r1=(?P<r1>0x[0-9a-fA-F]+) "
    r"r2=(?P<r2>0x[0-9a-fA-F]+) r3=(?P<r3>0x[0-9a-fA-F]+)"
    r"(?: ret0=(?P<ret0>0x[0-9a-fA-F]+) ret1=(?P<ret1>0x[0-9a-fA-F]+) "
    r"ret2=(?P<ret2>0x[0-9a-fA-F]+) ret3=(?P<ret3>0x[0-9a-fA-F]+))?"
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


def parse_hle_log(args) -> tuple[list[dict], pathlib.Path]:
    path = native_log_path(args)
    if not path.exists():
        raise TraceQueryError(f"{path}: missing native log")
    rows = []
    for line_no, line in enumerate(path.read_text(errors="replace").splitlines(), start=1):
        match = HLE_RE.search(line)
        if not match:
            continue
        row = {
            "line": line_no,
            "step": int(match.group("step")),
            "pc": int(match.group("pc"), 0),
            "name": match.group("name"),
        }
        for key in ("r0", "r1", "r2", "r3", "ret0", "ret1", "ret2", "ret3"):
            value = match.group(key)
            if value is not None:
                row[key] = int(value, 0)
        rows.append(row)
    return rows, path


def print_hle_uldivmod_check(args) -> int:
    rows, path = parse_hle_log(args)
    calls = [row for row in rows if row.get("name") == "__aeabi_uldivmod"]
    if not calls:
        raise TraceQueryError(f"{path}: no __aeabi_uldivmod HLE trace rows found")
    checked = 0
    mismatches = []
    incomplete = []
    for row in calls:
        if any(key not in row for key in ("ret0", "ret1", "ret2", "ret3")):
            incomplete.append({"line": row.get("line"), "step": row.get("step"), "error": "missing ret registers"})
            continue
        lhs = row["r0"] | (row["r1"] << 32)
        rhs = row["r2"] | (row["r3"] << 32)
        if rhs == 0:
            expected_q = 0
            expected_r = 0
        else:
            expected_q = lhs // rhs
            expected_r = lhs % rhs
        actual_q = row["ret0"] | (row["ret1"] << 32)
        actual_r = row["ret2"] | (row["ret3"] << 32)
        checked += 1
        if actual_q != expected_q or actual_r != expected_r:
            mismatches.append(
                {
                    "line": row.get("line"),
                    "step": row.get("step"),
                    "lhs": lhs,
                    "rhs": rhs,
                    "actual_q": actual_q,
                    "expected_q": expected_q,
                    "actual_r": actual_r,
                    "expected_r": expected_r,
                }
            )
    print(
        f"hle-uldivmod-check: path={path} calls={len(calls)} "
        f"checked={checked} mismatches={len(mismatches)} incomplete={len(incomplete)}"
    )
    for detail in mismatches[: args.limit]:
        print(
            "mismatch "
            f"line={detail.get('line')} step={detail.get('step')} "
            f"lhs=0x{detail.get('lhs'):016x} rhs=0x{detail.get('rhs'):016x}"
        )
        print(
            f"  quotient actual=0x{detail.get('actual_q'):016x} "
            f"expected=0x{detail.get('expected_q'):016x}"
        )
        print(
            f"  remainder actual=0x{detail.get('actual_r'):016x} "
            f"expected=0x{detail.get('expected_r'):016x}"
        )
    for detail in incomplete[: args.limit]:
        print(
            "incomplete "
            f"line={detail.get('line')} step={detail.get('step')} error={detail.get('error')}"
        )
    return 1 if mismatches or incomplete else 0


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


def cxx_string_value(row: dict, base: str, offset: int) -> str | None:
    for item in row.get("cxx_strings", []):
        if not isinstance(item, dict):
            continue
        if item.get("base") == base and item.get("offset") == offset:
            value = item.get("bytes")
            return value if isinstance(value, str) else None
    return None


def expectation_matches(value: int | None, expectation: str) -> bool:
    if expectation == "any":
        return value is not None
    if expectation == "null":
        return value == 0
    if expectation == "nonnull":
        return value is not None and value != 0
    raise TraceQueryError(f"unknown expectation {expectation!r}")


def print_mcpe_font_pair_check(args) -> int:
    events, path = load_native_events(args)
    pending: dict[int | None, dict] = {}
    rows = []
    for row in events:
        event = row.get("event")
        thread = row.get("thread") if isinstance(row.get("thread"), int) else None
        if event == "TextureGroup::getTexturePair.entry":
            resource = cxx_string_value(row, "r1", 0)
            package = cxx_string_value(row, "r1", 4)
            pending[thread] = {
                "step": row.get("step"),
                "resource": resource,
                "package": package,
            }
        elif event == "Font::init.after-getTexturePair":
            call = pending.pop(thread, None)
            if call is None:
                continue
            rows.append(
                {
                    "step": call["step"],
                    "resource": call["resource"],
                    "package": call["package"],
                    "result": row_reg(row, "r0"),
                }
            )

    if not rows:
        raise TraceQueryError(f"{path}: no Font::init TextureGroup lookup rows found")

    print(f"mcpe-font-pair-check: path={path} rows={len(rows)}")
    for row in rows[: args.limit]:
        result = row["result"]
        result_text = "missing" if result is None else fmt_u32(result)
        print(
            f"step={row['step']} package={row['package']!r} "
            f"resource={row['resource']!r} result={result_text}"
        )

    expectations = {
        "font/default8.png": args.expect_default8,
        "font/ascii_sga.png": args.expect_ascii_sga,
    }
    failures = []
    for resource, expectation in expectations.items():
        matches = [row for row in rows if row.get("resource") == resource]
        if not matches:
            failures.append(f"missing lookup for {resource}")
            continue
        if not any(expectation_matches(row.get("result"), expectation) for row in matches):
            actual = ", ".join(
                "missing" if row.get("result") is None else fmt_u32(row.get("result"))
                for row in matches
            )
            failures.append(f"{resource} expected {expectation}, got {actual}")

    if failures:
        for failure in failures[: args.limit]:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    print(
        "mcpe-font-pair ok: "
        f"default8={args.expect_default8} ascii_sga={args.expect_ascii_sga}"
    )
    return 0


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
        with (root / "run.log").open("a") as log:
            log.write(
                "\n".join(
                    [
                        "THREAD create id=7 start=0x703911f9 arg=0x60048b40 entry=0x08580844 entry_lib=<unknown> sp=0x6c040000",
                        "THREAD condwait id=7 name=pthread_cond_wait pc=0x6f000000 cond=0x60049040 mutex=0x6004903c timeout=0x00000000",
                        "THREAD wait id=7 Condvar { cond: 1610911808, mutex: 1610911804 }",
                        "THREAD signal id=1 name=pthread_cond_signal pc=0x6f000004 cond=0x60049040 waiters_before=1",
                        "THREAD wake id=7 cond=0x60049040 mutex=0x6004903c wait=Runnable",
                        "THREAD skip id=13 start=0x715cde85 arg=0x71bfa0c8 library=libminecraftpe.so",
                        "THREAD slice id=7 done=false pc=0x70f7c722 Thumb r0=0x00000000",
                        "THREAD stall skipped id=13 start=0x715cde85 arg=0x71bfa0c8 start_at=libminecraftpe.so+0x010cde84",
                    ]
                )
                + "\n"
            )
        thread_args = argparse.Namespace(trace_dir=str(root), native_log=None)
        thread_data, _thread_path = parse_thread_log(thread_args)
        assert thread_data["actions"]["create"] == 1
        assert thread_data["actions"]["skip"] == 1
        assert thread_data["actions"]["wait"] == 1
        assert thread_data["actions"]["condwait"] == 1
        assert thread_data["actions"]["signal"] == 1
        assert thread_data["actions"]["wake"] == 1
        assert thread_data["actions"]["slice"] == 1
        assert thread_data["actions"]["stall"] == 1
        assert thread_data["skips"][0]["library"] == "libminecraftpe.so"
        assert "start_at=libminecraftpe.so" in thread_data["stalls"][0]["raw"]
        assert thread_data["signals"][0]["waiters_before"] == 1
        assert thread_data["condwaits"][0]["name"] == "pthread_cond_wait"
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
                            "step": 30,
                            "thread": 1,
                            "pc": 0x716F045C,
                            "event": "TextureGroup::getTexturePair.entry",
                            "regs": {"r1": 0x1000},
                            "cxx_strings": [
                                {
                                    "base": "r1",
                                    "offset": 0,
                                    "bytes": "font/default8.png",
                                },
                                {
                                    "base": "r1",
                                    "offset": 4,
                                    "bytes": "InAppPackageImages",
                                },
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 31,
                            "thread": 1,
                            "pc": 0x70C3CA88,
                            "event": "Font::init.after-getTexturePair",
                            "r0": 0x60DEF760,
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 32,
                            "thread": 1,
                            "pc": 0x716F045C,
                            "event": "TextureGroup::getTexturePair.entry",
                            "regs": {"r1": 0x1000},
                            "cxx_strings": [
                                {
                                    "base": "r1",
                                    "offset": 0,
                                    "bytes": "font/ascii_sga.png",
                                },
                                {
                                    "base": "r1",
                                    "offset": 4,
                                    "bytes": "InAppPackageImages",
                                },
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 33,
                            "thread": 1,
                            "pc": 0x70C3CA88,
                            "event": "Font::init.after-getTexturePair",
                            "r0": 0x60E68010,
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        font_args = argparse.Namespace(
            trace_dir=str(root),
            native_events=None,
            expect_default8="nonnull",
            expect_ascii_sga="nonnull",
            limit=4,
        )
        assert print_mcpe_font_pair_check(font_args) == 0
        (root / "run.log").write_text(
            "HLE function=0x7128eef5 step=99 pc=0x6f000198 name=__aeabi_uldivmod "
            "r0=0x00000000 r1=0x00000001 r2=0x00000003 r3=0x00000000 "
            "ret0=0x55555555 ret1=0x00000000 ret2=0x00000001 ret3=0x00000000\n"
        )
        hle_args = argparse.Namespace(trace_dir=str(root), native_log=None, limit=4)
        assert print_hle_uldivmod_check(hle_args) == 0
        (root / "pc_profile.jsonl").write_text(
            json.dumps(
                {
                    "kind": "pc_profile_snapshot",
                    "samples": 128,
                    "guest_instructions": 524288,
                    "interval": 4096,
                    "unique_buckets": 2,
                    "top": [
                        {
                            "rank": 1,
                            "thread_id": 1,
                            "pc_hex": "0x717e073c",
                            "isa": "Arm",
                            "count": 42,
                            "instr_hex": "0xe09ba007",
                            "op": "adds",
                            "library": "libminecraftpe.so",
                            "object_offset_hex": "0x012e073c",
                            "symbol": "bn_mul_mont",
                            "symbol_offset_hex": "0x11c",
                        },
                        {
                            "rank": 2,
                            "thread_id": 2,
                            "pc_hex": "0x6f001300",
                            "isa": "Arm",
                            "count": 7,
                            "instr_hex": "0xe7f000f0",
                            "op": "<unknown>",
                            "library": "hle-imports",
                            "object_offset_hex": "0x00001300",
                            "symbol": "malloc",
                            "symbol_offset_hex": "0x0",
                        },
                    ],
                },
                separators=(",", ":"),
            )
            + "\n"
            + '{"partial":'
        )
        pc_args = argparse.Namespace(
            trace_dir=str(root),
            pc_profile=None,
            contains="bn_mul",
            library=None,
            thread=None,
            limit=4,
        )
        rows, _profile_path, invalid = load_pc_profile(pc_args)
        assert len(rows) == 1
        assert invalid == 1
        assert pc_profile_entry_matches(rows[0]["top"][0], pc_args)
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
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 24,
                            "event": "BN_mod_sqr.entry",
                            "mem32": [
                                {
                                    "base": "r1",
                                    "fields": [
                                        {"label": "r1+0x4", "value": 1},
                                    ],
                                },
                                {
                                    "base": "r2",
                                    "fields": [
                                        {"label": "r2+0x4", "value": 1},
                                    ],
                                },
                            ],
                            "byte_samples": [
                                {"source": "*r1+0@0x1100", "hex": "05000000"},
                                {"source": "*r2+0@0x1200", "hex": "0d000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 25,
                            "event": "BN_mod_sqr.return",
                            "mem32": [
                                {
                                    "base": "r6",
                                    "fields": [
                                        {"label": "r6+0x4", "value": 1},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {"source": "*r6+0@0x1300", "hex": "0c000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        bn_sqr_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_mod_sqr_check(bn_sqr_args) == 0
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 26,
                            "event": "BN_nnmod.entry",
                            "mem32": [
                                {
                                    "base": "r1",
                                    "fields": [
                                        {"label": "r1+0x4", "value": 1},
                                    ],
                                },
                                {
                                    "base": "r2",
                                    "fields": [
                                        {"label": "r2+0x4", "value": 1},
                                    ],
                                },
                            ],
                            "byte_samples": [
                                {"source": "*r1+0@0x1400", "hex": "19000000"},
                                {"source": "*r2+0@0x1500", "hex": "0d000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 27,
                            "event": "BN_nnmod.after-bn-div",
                            "mem32": [
                                {
                                    "base": "r4",
                                    "fields": [
                                        {"label": "r4+0x4", "value": 1},
                                        {"label": "r4+0xc", "value": 0},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {"source": "*r4+0@0x1600", "hex": "0c000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 28,
                            "event": "BN_nnmod.return",
                            "mem32": [
                                {
                                    "base": "r4",
                                    "fields": [
                                        {"label": "r4+0x4", "value": 1},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {"source": "*r4+0@0x1600", "hex": "0c000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        bn_nnmod_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_nnmod_check(bn_nnmod_args) == 0
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 29,
                            "event": "bn_div_words.entry",
                            "r0": 1,
                            "r1": 0,
                            "r2": 3,
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 30,
                            "event": "bn_div_words.return",
                            "r0": 0x55555555,
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        div_words_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_div_words_check(div_words_args) == 0
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 31,
                            "event": "BN_div.entry",
                            "r1": 0x1A00,
                            "mem32": [
                                {
                                    "base": "r2",
                                    "fields": [
                                        {"label": "r2+0x4", "value": 1},
                                        {"label": "r2+0xc", "value": 0},
                                    ],
                                },
                                {
                                    "base": "r3",
                                    "fields": [
                                        {"label": "r3+0x4", "value": 1},
                                    ],
                                },
                            ],
                            "byte_samples": [
                                {"source": "*r2+0@0x1700", "hex": "19000000"},
                                {"source": "*r3+0@0x1800", "hex": "0d000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 32,
                            "event": "BN_div.before-rem-rshift",
                            "mem32": [
                                {
                                    "base": "sp",
                                    "fields": [
                                        {"label": "sp+0x28", "value": 1},
                                    ],
                                },
                                {
                                    "base": "r7",
                                    "fields": [
                                        {"label": "r7+0x4", "value": 1},
                                    ],
                                },
                            ],
                            "byte_samples": [
                                {"source": "*r7+0@0x1900", "hex": "18000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 33,
                            "event": "BN_div.after-rem-rshift",
                            "mem32": [
                                {
                                    "base": "r6",
                                    "fields": [
                                        {"label": "r6+0x4", "value": 1},
                                        {"label": "r6+0xc", "value": 0},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {"source": "*r6+0@0x1a00", "hex": "0c000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        bn_div_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_div_check(bn_div_args) == 0
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 41,
                            "event": "BN_div.loop.uldiv-call",
                            "thread": 1,
                            "pc": 0x129954C,
                            "regs": {"r0": 10, "r1": 0, "r2": 3, "r3": 0},
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 42,
                            "event": "BN_div.loop.uldiv-ret",
                            "thread": 1,
                            "pc": 0x1299550,
                            "regs": {"r0": 3, "r1": 0, "r2": 1, "r3": 0},
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 43,
                            "event": "BN_div.loop.after-mls",
                            "pc": 0x1299570,
                            "lr": 5,
                            "regs": {"r2": 3, "r6": 20, "r8": 5, "r14": 5},
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 44,
                            "event": "BN_div.loop.after-umull",
                            "thread": 1,
                            "pc": 0x1299584,
                            "r0": 42,
                            "r1": 0,
                            "regs": {"r0": 42, "r1": 0, "r4": 7, "r10": 6},
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 45,
                            "event": "BN_div.loop.after-mla",
                            "thread": 1,
                            "pc": 0x12995A0,
                            "r1": 10,
                            "regs": {"r1": 10, "r5": 2, "r11": 5},
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 46,
                            "event": "BN_div.loop.bn-mul-call",
                            "thread": 1,
                            "pc": 0x1299614,
                            "regs": {"r1": 0x2000, "r2": 2, "r3": 3},
                            "byte_samples": [
                                {"source": "r1+0x0", "hex": "0200000004000000"}
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 47,
                            "event": "BN_div.loop.bn-mul-ret",
                            "thread": 1,
                            "pc": 0x1299618,
                            "r0": 0,
                            "byte_samples": [
                                {"source": "*r5+0x0@0x3000", "hex": "060000000c000000"}
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 48,
                            "event": "BN_div.loop.bn-sub-call",
                            "thread": 1,
                            "pc": 0x1299640,
                            "regs": {"r1": 0x4000, "r2": 0x5000, "r3": 2},
                            "byte_samples": [
                                {"source": "r1+0x0", "hex": "0700000000000000"},
                                {"source": "r2+0x0", "hex": "0500000000000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 49,
                            "event": "BN_div.loop.bn-sub-ret",
                            "thread": 1,
                            "pc": 0x1299644,
                            "r0": 0,
                            "byte_samples": [
                                {"source": "*sp+0x7c@0x4000", "hex": "0200000000000000"}
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 50,
                            "event": "BN_div.loop.bn-add-call",
                            "thread": 1,
                            "pc": 0x1299758,
                            "regs": {"r1": 0x6000, "r2": 0x7000, "r3": 1},
                            "byte_samples": [
                                {"source": "r1+0x0", "hex": "ffffffff"},
                                {"source": "r2+0x0", "hex": "01000000"},
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 51,
                            "event": "BN_div.loop.bn-add-ret",
                            "thread": 1,
                            "pc": 0x1299760,
                            "r0": 1,
                            "byte_samples": [
                                {"source": "*sp+0x7c@0x6000", "hex": "00000000"}
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        bn_div_loop_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_bn_div_loop_check(bn_div_loop_args) == 0
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 30,
                            "event": "ec_GFp_simple_dbl.entry",
                            "mem32": [
                                {
                                    "base": "r2",
                                    "fields": [
                                        {"label": "r2+0x40", "value": 1},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {
                                    "source": "*r2+0x4@0x1000",
                                    "hex": "28b5c0496675d03d38ced6a0e278e3206e4d1b54fc3a9c87ff0ea359848654642bde4e6123f72f8113159e29c2ad3a4d",
                                },
                                {
                                    "source": "*r2+0x18@0x1100",
                                    "hex": "fea4034bad3d0423aca9b47bbfa8bfa150b0832e56e7ad8bd9fff4681952c3c640a86939260280dde9c5155ac2ab782b",
                                },
                                {
                                    "source": "*r2+0x2c@0x1200",
                                    "hex": "01000000ffffffffffffffff000000000100000000000000000000000000000000000000000000000000000000000000",
                                },
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 31,
                            "event": "ec_GFp_simple_dbl.return",
                            "mem32": [
                                {
                                    "base": "r10",
                                    "fields": [
                                        {"label": "r10+0x40", "value": 0},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {
                                    "source": "*r10+0x4@0x2000",
                                    "hex": "ce77131719f475173fd52df0ccca8d8f02167257d9af8e106ee14930c6b6fb2b3acb3026b1eeaac216a92fe862fde518",
                                },
                                {
                                    "source": "*r10+0x18@0x2100",
                                    "hex": "e019f8ef222e96d974f283eff8b8ac05d6a6a30a32497f6b2fd0f638d40a33f54203dee8115f0f1b2848eaf36284ef49",
                                },
                                {
                                    "source": "*r10+0x2c@0x2200",
                                    "hex": "fc4907965a7b0846585369f77e517f43a160075dacce5b17b3ffe9d132a4868d8150d3724c0400bbd38b2bb48457f156",
                                },
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        ec_args = argparse.Namespace(trace_dir=str(root), native_events=None, limit=4)
        assert print_ec_point_check(ec_args) == 0
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
        (root / "native_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "step": 10,
                            "event": RESOURCE_WORK_EVENT,
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 15,
                            "event": RESOURCE_DONE_LOAD_EVENT,
                            "mem32": [
                                {
                                    "base": "r0",
                                    "fields": [
                                        {"label": "r0+0x0", "value": 7},
                                    ],
                                }
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 16,
                            "event": RESOURCE_DONE_CHECK_EVENT,
                            "mem32": [
                                {
                                    "base": "r4",
                                    "fields": [
                                        {"label": "r4+0x60", "value": 7},
                                    ],
                                }
                            ],
                        },
                        separators=(",", ":"),
                    ),
                    json.dumps(
                        {
                            "step": 20,
                            "event": MCPE_RENDER_RESOURCE_GATE_EVENT,
                            "gles_next_event_index": 4,
                            "mem32": [
                                {
                                    "base": "r7",
                                    "fields": [
                                        {"label": "r7+0x23c", "value": 1},
                                    ],
                                }
                            ],
                            "byte_samples": [
                                {
                                    "source": "r7+0x238@0x60048600",
                                    "hex": "0000000001000000e886046000000000",
                                }
                            ],
                        },
                        separators=(",", ":"),
                    ),
                ]
            )
            + "\n"
        )
        (root / "gles_events.jsonl").write_text(
            "\n".join(
                [
                    json.dumps({"index": 4, "kind": "SwapBuffers"}, separators=(",", ":")),
                    json.dumps({"index": 5, "kind": "DrawElements"}, separators=(",", ":")),
                ]
            )
            + "\n"
        )
        resource_args = argparse.Namespace(
            trace_dir=str(root),
            native_events=None,
            gles_events=None,
            limit=5,
        )
        resource_events, _ = load_native_events(resource_args)
        resource_gles, _ = load_gles_events(resource_args)
        resource_summary = resource_progress_summary(resource_events, resource_gles)
        assert resource_summary["work_load"] == 1
        assert resource_summary["done_load"] == 1
        assert resource_summary["done_check"] == 1
        assert resource_summary["done_load_last"] == 7
        assert resource_summary["done_check_last"] == 7
        assert resource_summary["gate_count"] == 1
        assert resource_summary["ready_byte"] == 0
        assert resource_summary["gles_swaps"] == 1
        assert resource_summary["gles_draws"] == 1
        assert print_resource_progress(resource_args) == 0


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
    parser.add_argument("--pc-profile", help="override PC profile JSONL path")
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

    pc_profile = subparsers.add_parser("pc-profile", help="show guest PC profile hot spots")
    pc_profile.add_argument("--contains")
    pc_profile.add_argument("--library")
    pc_profile.add_argument("--thread", type=parse_u32)
    pc_profile.add_argument("--symbols", action="store_true")
    pc_profile.add_argument("--limit", type=int, default=40)

    thread_summary = subparsers.add_parser(
        "thread-summary", help="summarize AEMU_TRACE_THREADS run.log events"
    )
    thread_summary.add_argument("--limit", type=int, default=12)
    thread_summary.add_argument("--slice-pcs", action="store_true")

    resource_progress = subparsers.add_parser(
        "resource-progress",
        help="summarize MCPE resource preload, ready gate, and GLES draw progress",
    )
    resource_progress.add_argument("--limit", type=int, default=20)

    bn_mont = subparsers.add_parser(
        "bn-mont-check",
        help="check bn_mul_mont native trace rows against a host Montgomery oracle",
    )
    bn_mont.add_argument("--limit", type=int, default=10)

    bn_mod_sqr = subparsers.add_parser(
        "bn-mod-sqr-check",
        help="check BN_mod_sqr native trace rows against a host modular-square oracle",
    )
    bn_mod_sqr.add_argument("--limit", type=int, default=10)

    bn_nnmod = subparsers.add_parser(
        "bn-nnmod-check",
        help="check BN_nnmod native trace rows against a host modulus oracle",
    )
    bn_nnmod.add_argument("--limit", type=int, default=10)

    bn_div_words = subparsers.add_parser(
        "bn-div-words-check",
        help="check bn_div_words native trace rows against a host quotient oracle",
    )
    bn_div_words.add_argument("--limit", type=int, default=10)

    bn_div = subparsers.add_parser(
        "bn-div-check",
        help="check BN_div native trace rows against a host remainder oracle",
    )
    bn_div.add_argument("--limit", type=int, default=10)

    bn_div_loop = subparsers.add_parser(
        "bn-div-loop-check",
        help="check BN_div loop helper and arithmetic trace rows against host oracles",
    )
    bn_div_loop.add_argument("--limit", type=int, default=10)

    ec_point = subparsers.add_parser(
        "ec-point-check",
        help="check ec_GFp_simple add/double traces against P-384 affine arithmetic",
    )
    ec_point.add_argument("--limit", type=int, default=10)

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

    font_pair = subparsers.add_parser(
        "mcpe-font-pair-check",
        help="check native Font::init TextureGroup atlas lookups",
    )
    font_pair.add_argument("--expect-default8", choices=("null", "nonnull", "any"), default="nonnull")
    font_pair.add_argument("--expect-ascii-sga", choices=("null", "nonnull", "any"), default="nonnull")
    font_pair.add_argument("--limit", type=int, default=20)

    hle_uldivmod = subparsers.add_parser(
        "hle-uldivmod-check",
        help="check traced __aeabi_uldivmod HLE calls against a host oracle",
    )
    hle_uldivmod.add_argument("--limit", type=int, default=10)
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
    if args.command == "pc-profile":
        return print_pc_profile(args)
    if args.command == "thread-summary":
        return print_thread_summary(args)
    if args.command == "resource-progress":
        return print_resource_progress(args)
    if args.command == "bn-mont-check":
        return print_bn_mont_check(args)
    if args.command == "bn-mod-sqr-check":
        return print_bn_mod_sqr_check(args)
    if args.command == "bn-nnmod-check":
        return print_bn_nnmod_check(args)
    if args.command == "bn-div-words-check":
        return print_bn_div_words_check(args)
    if args.command == "bn-div-check":
        return print_bn_div_check(args)
    if args.command == "bn-div-loop-check":
        return print_bn_div_loop_check(args)
    if args.command == "ec-point-check":
        return print_ec_point_check(args)
    if args.command == "program":
        return print_program(args)
    if args.command == "mcpe-text":
        return print_mcpe_text(args)
    if args.command == "mcpe-texturedata":
        return print_mcpe_texturedata(args)
    if args.command == "mcpe-font-pair-check":
        return print_mcpe_font_pair_check(args)
    if args.command == "hle-uldivmod-check":
        return print_hle_uldivmod_check(args)
    raise SystemExit("command is required unless --self-test is used")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except TraceQueryError as err:
        print(f"trace_query failed: {err}", file=sys.stderr)
        raise SystemExit(1) from err
