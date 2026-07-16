#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import pathlib
import queue
import re
import subprocess
import sys
import threading
import time

import trace_check
import ws_cli


DEFAULT_APK = pathlib.Path("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk")
DEFAULT_BINARY = pathlib.Path("target/release/aemu")
DEFAULT_ABI = "armeabi-v7a"
DEFAULT_OUTPUT_ROOT = pathlib.Path("tmp")
DEFAULT_WS_ADDR = "127.0.0.1:0"
DEFAULT_STEPS = 600_000_000
DEFAULT_SCRIPT = (
    "debug; "
    "screenshot {trace_dir}/before.png; "
    "tap 427,240; "
    "wait 0.25; "
    "screenshot {trace_dir}/after.png; "
    "debug"
)
PLAY_PRESET_SCRIPT = (
    "screenshot {screenshots}/01_visible.png; "
    "tap 280,386; "
    "wait 1.0; "
    "screenshot {screenshots}/02_after_not_now.png; "
    "tap 427,240; "
    "wait 8.0; "
    "screenshot {screenshots}/03_after_play.png; "
    "debug"
)
FLAT_WORLD_ENTRY_SCRIPT = (
    "screenshot {screenshots}/01_xbox.png; "
    "tap 280,386; "
    "wait 1.0; "
    "screenshot {screenshots}/02_menu.png; "
    "tap 427,240; "
    "wait 1.0; "
    "screenshot {screenshots}/03_play.png; "
    "tap 427,106; "
    "wait 1.0; "
    "screenshot {screenshots}/04_create.png; "
    "tap 786,26; "
    "wait 0.5; "
    "screenshot {screenshots}/05_advanced.png; "
    "tap 565,280; "
    "wait 0.5; "
    "screenshot {screenshots}/06_flat_selected.png; "
    "tap 634,357; "
    "wait 1.0; "
    "screenshot {screenshots}/07_loading.png; "
    "wait 5.0; "
    "screenshot {screenshots}/08_world_6s.png; "
    "wait 15.0; "
    "screenshot {screenshots}/09_world_21s.png; "
    "wait 15.0; "
    "screenshot {screenshots}/10_world_36s.png; "
)
FLAT_WORLD_PRESET_SCRIPT = FLAT_WORLD_ENTRY_SCRIPT + "debug"
PLAYABLE_FLAT_WORLD_PRESET_SCRIPT = (
    FLAT_WORLD_ENTRY_SCRIPT
    + "pointer down 112,320; "
    "wait 2.0; "
    "pointer up 112,320; "
    "wait 2.0; "
    "screenshot {screenshots}/11_moved.png; "
    "pointer down 500,200; "
    "wait 0.1; "
    "pointer move 550,200; "
    "wait 0.1; "
    "pointer move 600,200; "
    "wait 0.1; "
    "pointer move 650,200; "
    "wait 0.1; "
    "pointer up 650,200; "
    "wait 2.0; "
    "screenshot {screenshots}/12_looked.png; "
    "pointer down 500,200; "
    "wait 0.1; "
    "pointer move 500,250; "
    "wait 0.1; "
    "pointer move 500,300; "
    "wait 0.1; "
    "pointer move 500,350; "
    "wait 0.1; "
    "pointer up 500,350; "
    "wait 0.5; "
    "pointer down 500,300; "
    "wait 0.1; "
    "pointer move 500,275; "
    "wait 0.1; "
    "pointer move 500,250; "
    "wait 0.1; "
    "pointer up 500,250; "
    "wait 2.0; "
    "screenshot {screenshots}/13_aimed.png; "
    "tap 427,220; "
    "wait 2.0; "
    "screenshot {screenshots}/14_placed.png; "
    "pointer down 427,220; "
    "wait 0.8; "
    "pointer up 427,220; "
    "wait 2.0; "
    "screenshot {screenshots}/15_broken.png; "
    "debug"
)

FLAT_WORLD_MIN_SKY_RATIO = 0.75
FLAT_WORLD_MIN_GROUND_RATIO = 0.75
PLAYABLE_MIN_GROUND_CHANGE_RATIO = 0.20
PLAYABLE_MIN_AIM_CHANGE_RATIO = 0.40
PLAYABLE_MIN_PLACED_GRAY_RATIO = 0.30
PLAYABLE_MAX_BROKEN_GRAY_RATIO = 0.15
PLAYABLE_MIN_BROKEN_HOLE_RATIO = 0.20

WS_URL_RE = re.compile(r"sdl2-live: websocket (?P<url>ws://\S+)")


def parse_step_limit(value):
    if value.lower() == "inf":
        return "inf"
    try:
        parsed = int(value, 10)
    except ValueError as err:
        raise argparse.ArgumentTypeError("steps must be a non-negative integer or inf") from err
    if parsed < 0:
        raise argparse.ArgumentTypeError("steps must be a non-negative integer or inf")
    return parsed


def timestamp():
    return time.strftime("%Y%m%d-%H%M%S")


def prepare_output_dir(path):
    if path is None:
        out_dir = DEFAULT_OUTPUT_ROOT / f"mcpe-ui-smoke-{timestamp()}"
    else:
        out_dir = pathlib.Path(path)
    (out_dir / "screenshots").mkdir(parents=True, exist_ok=True)
    return out_dir


def load_script(args, out_dir):
    chunks = []
    if args.script_file:
        chunks.append(pathlib.Path(args.script_file).read_text(encoding="utf-8"))
    if args.script:
        chunks.append(args.script)
    if not chunks:
        chunks.append(DEFAULT_SCRIPT)
    text = "\n".join(chunks)
    return text.format(trace_dir=out_dir, screenshots=out_dir / "screenshots")


def start_reader(process, log_path):
    lines = queue.Queue()

    def read_stdout():
        with log_path.open("w", encoding="utf-8") as log:
            for line in process.stdout:
                log.write(line)
                log.flush()
                lines.put(line)

    thread = threading.Thread(target=read_stdout, daemon=True)
    thread.start()
    return lines, thread


def wait_for_ws_url(process, lines, timeout):
    deadline = time.monotonic() + timeout
    buffered = []
    while time.monotonic() < deadline:
        if process.poll() is not None:
            while True:
                try:
                    buffered.append(lines.get_nowait())
                except queue.Empty:
                    break
            raise RuntimeError(
                "emulator exited before WebSocket became available; "
                f"returncode={process.returncode}"
            )
        try:
            line = lines.get(timeout=0.1)
        except queue.Empty:
            continue
        buffered.append(line)
        match = WS_URL_RE.search(line)
        if match:
            return match.group("url"), buffered
    raise RuntimeError(f"timed out waiting {timeout:.1f}s for SDL2 WebSocket URL")


def wait_for_debug_milestone(client, args, timeout, journal, out_dir):
    targets = {
        "frames": max(0, args.wait_frames),
        "draw_elements": max(0, args.wait_draw_elements),
        "readback_nonzero_rgb_pixels": max(0, args.wait_readback_rgb),
    }
    if all(value <= 0 for value in targets.values()):
        return None
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            response = client.request({"cmd": "debug"})
        except Exception as err:
            response = {"ok": False, "error": str(err)}
        last = response
        milestone_met = response.get("ok") and debug_meets_targets(response, targets)
        row = {
            "kind": "wait_debug",
            "targets": targets,
            "response": response,
        }
        if milestone_met:
            checkpoint = pc_profile_checkpoint(args, out_dir)
            if checkpoint is not None:
                row["pc_profile"] = checkpoint
        journal.write(json.dumps(row, sort_keys=True) + "\n")
        if milestone_met:
            return response
        time.sleep(args.wait_debug_interval)
    raise RuntimeError(
        "timed out waiting for debug milestone "
        f"{json.dumps(targets, sort_keys=True)}; last={json.dumps(last, sort_keys=True)}"
    )


def debug_meets_targets(response, targets):
    for key, target in targets.items():
        if target > 0 and int(response.get(key) or 0) < target:
            return False
    return True


def action_json(step):
    value = {"action": step.action}
    value.update(step.kwargs)
    return value


def ensure_parent(path):
    pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)


def run_step(client, step, args):
    if step.action == "tap":
        down, up = ws_cli.send_tap(
            client,
            step.kwargs["x"],
            step.kwargs["y"],
            args.pointer_id,
            args.pressure,
            args.tap_duration_ms,
        )
        return {"ok": bool(down.get("ok") and up.get("ok")), "down": down, "up": up}
    if step.action == "pointer":
        pressure = 0.0 if step.kwargs["phase"] == "up" else args.pressure
        return client.request(
            ws_cli.make_pointer_payload(
                step.kwargs["phase"],
                step.kwargs["x"],
                step.kwargs["y"],
                args.pointer_id,
                pressure,
            )
        )
    if step.action == "wait":
        time.sleep(step.kwargs["seconds"])
        return {"ok": True, "seconds": step.kwargs["seconds"]}
    if step.action == "screenshot":
        ensure_parent(step.kwargs["out"])
        result, _size = ws_cli.save_screenshot(client, step.kwargs["out"])
        return result
    if step.action == "debug":
        return client.request({"cmd": "debug"})
    return {"ok": False, "error": f"unhandled action {step.action!r}"}


def file_sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def summarize_screenshot(result):
    if not result.get("ok") or not result.get("path"):
        return None
    path = pathlib.Path(result["path"])
    if not path.exists():
        return None
    return {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": file_sha256(path),
        "format": result.get("format"),
        "width": result.get("width"),
        "height": result.get("height"),
    }


def region_ratio(image, x_start, y_start, x_end, y_end, predicate):
    left = int(image.width * x_start)
    top = int(image.height * y_start)
    right = int(image.width * x_end)
    bottom = int(image.height * y_end)
    matched = 0
    pixels = 0
    for y in range(top, bottom):
        for x in range(left, right):
            offset = (y * image.width + x) * 3
            red, green, blue = image.rgb[offset : offset + 3]
            matched += bool(predicate(red, green, blue))
            pixels += 1
    return matched / pixels if pixels else 0.0


def flat_world_metrics(path):
    image = trace_check.parse_png(pathlib.Path(path))
    sky_ratio = region_ratio(
        image,
        0.15,
        0.15,
        0.85,
        0.45,
        lambda red, green, blue: blue >= 150 and blue >= red + 20 and blue >= green + 5,
    )
    ground_ratio = region_ratio(
        image,
        0.20,
        0.54,
        0.80,
        0.83,
        lambda red, green, blue: green >= red + 10 and green >= blue + 5,
    )
    return {
        "sky_ratio": round(sky_ratio, 6),
        "ground_ratio": round(ground_ratio, 6),
        "min_sky_ratio": FLAT_WORLD_MIN_SKY_RATIO,
        "min_ground_ratio": FLAT_WORLD_MIN_GROUND_RATIO,
        "passed": sky_ratio >= FLAT_WORLD_MIN_SKY_RATIO
        and ground_ratio >= FLAT_WORLD_MIN_GROUND_RATIO,
    }


def image_region_change(first, second, x_start, y_start, x_end, y_end):
    if first.width != second.width or first.height != second.height:
        raise trace_check.TraceError(
            f"image dimensions differ: {first.width}x{first.height} vs "
            f"{second.width}x{second.height}"
        )
    left = int(first.width * x_start)
    top = int(first.height * y_start)
    right = int(first.width * x_end)
    bottom = int(first.height * y_end)
    changed = 0
    channels = 0
    absolute_difference = 0
    for y in range(top, bottom):
        for x in range(left, right):
            offset = (y * first.width + x) * 3
            for channel in range(3):
                difference = abs(first.rgb[offset + channel] - second.rgb[offset + channel])
                changed += difference > 8
                absolute_difference += difference
                channels += 1
    return {
        "changed_channel_ratio": round(changed / channels if channels else 0.0, 6),
        "mean_absolute_difference": round(
            absolute_difference / channels if channels else 0.0, 6
        ),
    }


def playability_metrics(paths):
    world, moved, looked, aimed, placed, broken = [
        trace_check.parse_png(pathlib.Path(path)) for path in paths
    ]
    move = image_region_change(world, moved, 0.20, 0.54, 0.80, 0.83)
    look = image_region_change(moved, looked, 0.20, 0.54, 0.80, 0.83)
    aim = image_region_change(looked, aimed, 0.20, 0.12, 0.80, 0.84)
    placed_gray_ratio = region_ratio(
        placed,
        0.35,
        0.10,
        0.65,
        0.62,
        lambda red, green, blue: 45 <= red <= 180
        and abs(red - green) <= 18
        and abs(green - blue) <= 18,
    )
    broken_gray_ratio = region_ratio(
        broken,
        0.35,
        0.10,
        0.65,
        0.62,
        lambda red, green, blue: 45 <= red <= 180
        and abs(red - green) <= 18
        and abs(green - blue) <= 18,
    )
    broken_hole_ratio = region_ratio(
        broken,
        0.40,
        0.35,
        0.60,
        0.62,
        lambda red, green, blue: red >= 25
        and red >= green + 3
        and green >= blue - 5
        and blue < 80,
    )
    checks = {
        "movement": move["changed_channel_ratio"] >= PLAYABLE_MIN_GROUND_CHANGE_RATIO,
        "camera": look["changed_channel_ratio"] >= PLAYABLE_MIN_GROUND_CHANGE_RATIO,
        "aim": aim["changed_channel_ratio"] >= PLAYABLE_MIN_AIM_CHANGE_RATIO,
        "placement": placed_gray_ratio >= PLAYABLE_MIN_PLACED_GRAY_RATIO,
        "breaking": broken_gray_ratio <= PLAYABLE_MAX_BROKEN_GRAY_RATIO
        and broken_hole_ratio >= PLAYABLE_MIN_BROKEN_HOLE_RATIO,
    }
    return {
        "move": move,
        "look": look,
        "aim": aim,
        "placed_gray_ratio": round(placed_gray_ratio, 6),
        "broken_gray_ratio": round(broken_gray_ratio, 6),
        "broken_hole_ratio": round(broken_hole_ratio, 6),
        "checks": checks,
        "passed": all(checks.values()),
    }


def count_jsonl(path):
    path = pathlib.Path(path)
    if not path.exists():
        return 0
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def read_jsonl(path):
    path = pathlib.Path(path)
    if not path.exists():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows


def summarize_pc_profile(path):
    summary = {
        "jsonl": str(path),
        "rows": 0,
        "samples": 0,
        "guest_instructions": 0,
        "unique_buckets": 0,
        "top": [],
    }
    if not path.exists():
        return summary
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            summary["rows"] += 1
            summary["samples"] = row.get("samples", summary["samples"])
            summary["guest_instructions"] = row.get(
                "guest_instructions", summary["guest_instructions"]
            )
            summary["unique_buckets"] = row.get("unique_buckets", summary["unique_buckets"])
            top = row.get("top")
            if isinstance(top, list):
                summary["top"] = top[:10]
    return summary


def latest_pc_profile_checkpoint(path):
    path = pathlib.Path(path)
    if not path.exists():
        return None
    latest = None
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            latest = row
    if latest is None:
        return None
    top = latest.get("top")
    return {
        "samples": latest.get("samples", 0),
        "guest_instructions": latest.get("guest_instructions", 0),
        "unique_buckets": latest.get("unique_buckets", 0),
        "top": top[:5] if isinstance(top, list) else [],
    }


def pc_profile_checkpoint(args, out_dir):
    if not args.profile_pc:
        return None
    return latest_pc_profile_checkpoint(pathlib.Path(out_dir) / "pc_profile.jsonl")


def native_event_matches(row, needle):
    needle = needle.lower()
    event = row.get("event")
    if isinstance(event, str) and needle in event.lower():
        return True
    pc = row.get("pc")
    return isinstance(pc, int) and needle in f"0x{pc:08x}".lower()


def collect_artifacts(out_dir):
    out_dir = pathlib.Path(out_dir)
    gles_path = out_dir / "gles_events.jsonl"
    native_path = out_dir / "native_events.jsonl"
    pc_profile_path = out_dir / "pc_profile.jsonl"
    draw_dir = out_dir / "sdl-draw"
    pc_profile = summarize_pc_profile(pc_profile_path)
    return {
        "gles_events_jsonl": str(gles_path),
        "gles_event_count": count_jsonl(gles_path),
        "native_events_jsonl": str(native_path),
        "native_event_count": count_jsonl(native_path),
        "pc_profile_jsonl": str(pc_profile_path),
        "pc_profile_rows": pc_profile["rows"],
        "pc_profile_samples": pc_profile["samples"],
        "pc_profile_guest_instructions": pc_profile["guest_instructions"],
        "pc_profile_unique_buckets": pc_profile["unique_buckets"],
        "pc_profile_top": pc_profile["top"],
        "sdl_draw_dir": str(draw_dir),
        "sdl_draw_png_count": len(list(draw_dir.glob("*.png"))) if draw_dir.exists() else 0,
        "sdl_draw_manifest_jsonl": str(draw_dir / "draw_manifest.jsonl"),
        "sdl_draw_manifest_count": count_jsonl(draw_dir / "draw_manifest.jsonl"),
    }


def append_env_list(env, name, values):
    if values:
        env[name] = ";".join(values)


def apply_trace_env(args, out_dir, env):
    env["AEMU_TRACE_GLES_EVENTS_JSONL"] = str(out_dir / "gles_events.jsonl")
    env["AEMU_TRACE_GLES_EVENTS_MATCH"] = args.gles_event_match
    env["AEMU_TRACE_GLES_EVENTS_LIMIT"] = str(args.gles_event_limit)
    if args.gles_event_skip is not None:
        env["AEMU_TRACE_GLES_EVENTS_SKIP"] = str(args.gles_event_skip)
    if args.fake_time_step_nanos is not None:
        env["AEMU_FAKE_TIME_STEP_NANOS"] = str(args.fake_time_step_nanos)
    if args.fake_time_step_after_draw_nanos is not None:
        env["AEMU_FAKE_TIME_STEP_AFTER_DRAW_NANOS"] = str(args.fake_time_step_after_draw_nanos)
    if args.dynarmic_run_ticks is not None:
        env["AEMU_DYNARMIC_RUN_TICKS"] = str(args.dynarmic_run_ticks)
    if args.guest_thread_swap_slices is not None:
        env["AEMU_GUEST_THREAD_SWAP_SLICES"] = str(args.guest_thread_swap_slices)
    if args.main_thread_wait_idle_spins is not None:
        env["AEMU_MAIN_THREAD_WAIT_IDLE_SPINS"] = str(args.main_thread_wait_idle_spins)
    if args.main_thread_wait_slice_steps is not None:
        env["AEMU_MAIN_THREAD_WAIT_SLICE_STEPS"] = str(args.main_thread_wait_slice_steps)
    if args.profile_pc:
        env["AEMU_PROFILE_PC_JSONL"] = str(out_dir / "pc_profile.jsonl")
        env["AEMU_PROFILE_PC_INTERVAL"] = str(args.profile_pc_interval)
        env["AEMU_PROFILE_PC_FLUSH_INTERVAL"] = str(args.profile_pc_flush_interval)
        env["AEMU_PROFILE_PC_TOP"] = str(args.profile_pc_top)
        if args.profile_pc_limit is not None:
            env["AEMU_PROFILE_PC_LIMIT"] = str(args.profile_pc_limit)
    if args.trace_hle:
        env["AEMU_TRACE_HLE"] = args.trace_hle
    if args.trace_hle_limit is not None:
        env["AEMU_TRACE_HLE_LIMIT"] = str(args.trace_hle_limit)
    if args.trace_hle_file:
        env["AEMU_TRACE_HLE_FILE"] = "1"
    if args.native_event:
        env["AEMU_TRACE_NATIVE_EVENTS_JSONL"] = str(out_dir / "native_events.jsonl")
    append_env_list(env, "AEMU_TRACE_NATIVE_EVENTS", args.native_event)
    append_env_list(env, "AEMU_TRACE_NATIVE_EVENT_MEM32", args.native_event_mem32)
    append_env_list(env, "AEMU_TRACE_NATIVE_EVENT_DEREF32", args.native_event_deref32)
    append_env_list(env, "AEMU_TRACE_NATIVE_EVENT_CXX_STRING", args.native_event_cxx_string)
    append_env_list(env, "AEMU_TRACE_NATIVE_EVENT_BYTES", args.native_event_bytes)
    if args.native_event_limit is not None:
        env["AEMU_TRACE_NATIVE_EVENTS_LIMIT"] = str(args.native_event_limit)
    if args.dump_sdl_draws:
        draw_dir = out_dir / "sdl-draw"
        draw_dir.mkdir(parents=True, exist_ok=True)
        env["AEMU_DUMP_SDL_DRAW_CHANGES_DIR"] = str(draw_dir)
        env["AEMU_DUMP_SDL_DRAW_CHANGES_MATCH"] = args.sdl_draw_match
        env["AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT"] = str(args.sdl_draw_limit)
        env["AEMU_TRACE_SDL_DRAW_CHANGES"] = str(args.sdl_draw_limit)
        if args.sdl_draw_skip is not None:
            env["AEMU_TRACE_SDL_DRAW_CHANGES_SKIP"] = str(args.sdl_draw_skip)
        if args.sdl_draw_all:
            env["AEMU_TRACE_SDL_DRAW_CHANGES_ALL"] = "1"


def summarize_env(env):
    keys = [
        "DISPLAY",
        "SDL_VIDEO_X11_FORCE_EGL",
        "AEMU_TRACE_GLES_EVENTS_JSONL",
        "AEMU_TRACE_GLES_EVENTS_MATCH",
        "AEMU_TRACE_GLES_EVENTS_LIMIT",
        "AEMU_TRACE_GLES_EVENTS_SKIP",
        "AEMU_FAKE_TIME_STEP_NANOS",
        "AEMU_FAKE_TIME_STEP_AFTER_DRAW_NANOS",
        "AEMU_DYNARMIC_RUN_TICKS",
        "AEMU_GUEST_THREAD_SWAP_SLICES",
        "AEMU_MAIN_THREAD_WAIT_IDLE_SPINS",
        "AEMU_MAIN_THREAD_WAIT_SLICE_STEPS",
        "AEMU_PROFILE_PC_JSONL",
        "AEMU_PROFILE_PC_INTERVAL",
        "AEMU_PROFILE_PC_FLUSH_INTERVAL",
        "AEMU_PROFILE_PC_TOP",
        "AEMU_PROFILE_PC_LIMIT",
        "AEMU_TRACE_HLE",
        "AEMU_TRACE_HLE_LIMIT",
        "AEMU_TRACE_HLE_FILE",
        "AEMU_TRACE_NATIVE_EVENTS_JSONL",
        "AEMU_TRACE_NATIVE_EVENTS",
        "AEMU_TRACE_NATIVE_EVENT_MEM32",
        "AEMU_TRACE_NATIVE_EVENT_DEREF32",
        "AEMU_TRACE_NATIVE_EVENT_CXX_STRING",
        "AEMU_TRACE_NATIVE_EVENT_BYTES",
        "AEMU_TRACE_NATIVE_EVENTS_LIMIT",
        "AEMU_DUMP_SDL_DRAW_CHANGES_DIR",
        "AEMU_DUMP_SDL_DRAW_CHANGES_MATCH",
        "AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT",
        "AEMU_TRACE_SDL_DRAW_CHANGES",
        "AEMU_TRACE_SDL_DRAW_CHANGES_SKIP",
        "AEMU_TRACE_SDL_DRAW_CHANGES_ALL",
    ]
    return {key: env.get(key) for key in keys if env.get(key) is not None}


def run_journal(client, steps, args, journal_path):
    entries = []
    screenshots = []
    with journal_path.open("a", encoding="utf-8") as journal:
        for index, step in enumerate(steps, start=1):
            started = time.monotonic()
            try:
                result = run_step(client, step, args)
                ok = bool(result.get("ok"))
                error = None
            except Exception as err:
                result = None
                ok = False
                error = str(err)
            entry = {
                "kind": "step",
                "step": index,
                "action": action_json(step),
                "ok": ok,
                "elapsed_seconds": round(time.monotonic() - started, 6),
                "result": result,
            }
            if error is not None:
                entry["error"] = error
            if result is not None and step.action == "screenshot":
                screenshot = summarize_screenshot(result)
                if screenshot is not None:
                    screenshots.append(screenshot)
                    entry["screenshot"] = screenshot
            checkpoint = pc_profile_checkpoint(args, journal_path.parent)
            if checkpoint is not None:
                entry["pc_profile"] = checkpoint
            journal.write(json.dumps(entry, sort_keys=True) + "\n")
            entries.append(entry)
            if not ok:
                break
    return entries, screenshots


def terminate_process(process, timeout=5):
    if process.poll() is not None:
        return False
    process.terminate()
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=timeout)
    return True


def build_emulator_cmd(args, ws_addr):
    cmd = [
        str(args.binary),
        "run-apk-native",
        str(args.apk),
        "--abi",
        args.abi,
        "--cpu-backend",
        args.cpu_backend,
        "--steps",
        str(args.steps),
        "--sdl2-live",
        "--sdl2-frames",
        str(args.frames),
        "--ws",
        ws_addr,
    ]
    if not args.respect_host_quit:
        cmd.append("--sdl2-ignore-host-quit")
    return cmd


def collect_debug(entries):
    values = []
    for entry in entries:
        if entry.get("action", {}).get("action") != "debug":
            continue
        result = entry.get("result")
        if isinstance(result, dict):
            values.append(result)
    return values


def collect_validation_debug(summary):
    values = []
    wait_debug = summary["journal"].get("wait_debug")
    if isinstance(wait_debug, dict):
        values.append(wait_debug)
    values.extend(collect_debug(summary["journal"]["entries"]))
    return values


def validate_summary(args, summary):
    errors = []
    failed_steps = [entry for entry in summary["journal"]["entries"] if not entry.get("ok")]
    if failed_steps:
        first = failed_steps[0]
        errors.append(f"journal step {first['step']} failed: {first.get('error') or first.get('result')}")

    screenshots = summary["journal"]["screenshots"]
    if len(screenshots) < args.min_screenshots:
        errors.append(
            f"expected at least {args.min_screenshots} screenshots, "
            f"got {len(screenshots)}"
        )
    if args.expect_screenshot_change:
        if len(screenshots) < 2:
            errors.append("expected screenshot change, but fewer than two screenshots were captured")
        elif screenshots[0]["sha256"] == screenshots[-1]["sha256"]:
            errors.append(
                "expected first and last screenshots to differ, "
                f"but both have sha256={screenshots[0]['sha256']}"
            )
    for spec in args.expect_screenshot_pair_change or []:
        try:
            first_index, second_index = parse_screenshot_pair(spec)
        except ValueError as err:
            errors.append(str(err))
            continue
        if first_index > len(screenshots) or second_index > len(screenshots):
            errors.append(
                f"expected screenshot pair {spec} to exist, "
                f"but only {len(screenshots)} screenshots were captured"
            )
            continue
        first = screenshots[first_index - 1]
        second = screenshots[second_index - 1]
        if first["sha256"] == second["sha256"]:
            errors.append(
                f"expected screenshots {first_index}:{second_index} to differ, "
                f"but both have sha256={first['sha256']}"
            )

    for shot in screenshots:
        if shot["bytes"] < args.min_screenshot_bytes:
            errors.append(
                f"{shot['path']}: screenshot is only {shot['bytes']} bytes, "
                f"expected at least {args.min_screenshot_bytes}"
            )

    for index in args.expect_flat_world_screenshot or []:
        if index <= 0 or index > len(screenshots):
            errors.append(
                f"expected flat-world screenshot {index} to exist, "
                f"but only {len(screenshots)} screenshots were captured"
            )
            continue
        shot = screenshots[index - 1]
        try:
            metrics = flat_world_metrics(shot["path"])
        except (OSError, trace_check.TraceError) as err:
            errors.append(f"{shot['path']}: could not inspect flat-world frame: {err}")
            continue
        shot["flat_world"] = metrics
        if not metrics["passed"]:
            errors.append(
                f"{shot['path']}: expected an entered flat-world frame; "
                f"sky_ratio={metrics['sky_ratio']:.3f} "
                f"ground_ratio={metrics['ground_ratio']:.3f}"
            )

    if args.expect_playability_sequence:
        try:
            indices = parse_playability_sequence(args.expect_playability_sequence)
        except ValueError as err:
            errors.append(str(err))
        else:
            if max(indices) > len(screenshots):
                errors.append(
                    "expected playability screenshot sequence "
                    f"{args.expect_playability_sequence} to exist, but only "
                    f"{len(screenshots)} screenshots were captured"
                )
            else:
                selected = [screenshots[index - 1] for index in indices]
                try:
                    metrics = playability_metrics([shot["path"] for shot in selected])
                except (OSError, trace_check.TraceError) as err:
                    errors.append(f"could not inspect playability sequence: {err}")
                else:
                    metrics["screenshots"] = [shot["path"] for shot in selected]
                    summary["playability"] = metrics
                    for name, passed in metrics["checks"].items():
                        if not passed:
                            errors.append(
                                f"playability {name} check failed: "
                                f"{json.dumps(metrics, sort_keys=True)}"
                            )

    debug_values = collect_validation_debug(summary)
    if debug_values:
        last_debug = debug_values[-1]
        gl_errors = int(last_debug.get("gl_error_count") or 0)
        if gl_errors > args.max_gl_errors:
            errors.append(f"expected gl_error_count <= {args.max_gl_errors}, got {gl_errors}")
        skipped_client_attrib = int(last_debug.get("skipped_client_attrib_draws") or 0)
        if (
            args.max_skipped_client_attrib_draws is not None
            and skipped_client_attrib > args.max_skipped_client_attrib_draws
        ):
            errors.append(
                "expected skipped_client_attrib_draws <= "
                f"{args.max_skipped_client_attrib_draws}, got {skipped_client_attrib}"
            )
        frames = int(last_debug.get("frames") or 0)
        if frames < args.expect_frames:
            errors.append(f"expected debug frames >= {args.expect_frames}, got {frames}")
        readback_rgb = int(last_debug.get("readback_nonzero_rgb_pixels") or 0)
        if readback_rgb < args.min_readback_rgb:
            errors.append(
                f"expected readback_nonzero_rgb_pixels >= {args.min_readback_rgb}, "
                f"got {readback_rgb}"
            )
        draw_elements = int(last_debug.get("draw_elements") or 0)
        if draw_elements < args.min_draw_elements:
            errors.append(f"expected draw_elements >= {args.min_draw_elements}, got {draw_elements}")
    elif args.expect_frames > 0:
        errors.append("no debug step was captured")

    if args.expect_native_event:
        native_rows = read_jsonl(summary["artifacts"]["native_events_jsonl"])
        for expected in args.expect_native_event:
            if not any(native_event_matches(row, expected) for row in native_rows):
                errors.append(f"expected native event matching {expected!r}")

    artifacts = summary["artifacts"]
    gles_event_count = artifacts.get("gles_event_count", 0)
    if gles_event_count < args.min_gles_events:
        errors.append(
            f"expected at least {args.min_gles_events} GLES trace events, "
            f"got {gles_event_count}"
        )
    native_event_count = artifacts.get("native_event_count", 0)
    if native_event_count < args.min_native_events:
        errors.append(
            f"expected at least {args.min_native_events} native trace events, "
            f"got {native_event_count}"
        )
    pc_profile_samples = artifacts.get("pc_profile_samples", 0)
    if pc_profile_samples < args.min_pc_profile_samples:
        errors.append(
            f"expected at least {args.min_pc_profile_samples} PC profile samples, "
            f"got {pc_profile_samples}"
        )
    sdl_draw_png_count = artifacts.get("sdl_draw_png_count", 0)
    if sdl_draw_png_count < args.min_sdl_draw_pngs:
        errors.append(
            f"expected at least {args.min_sdl_draw_pngs} SDL draw PNGs, "
            f"got {sdl_draw_png_count}"
        )

    run_log = None
    if args.expect_run_log_contains or args.reject_run_log_contains or args.expect_hle_call:
        run_log_path = pathlib.Path(summary["process"]["run_log"])
        run_log = run_log_path.read_text(encoding="utf-8", errors="replace")
    if args.expect_run_log_contains:
        for expected in args.expect_run_log_contains:
            if expected not in run_log:
                errors.append(f"expected run log to contain {expected!r}")
    if args.reject_run_log_contains:
        for rejected in args.reject_run_log_contains:
            if rejected in run_log:
                errors.append(f"expected run log to not contain {rejected!r}")
    if args.expect_hle_call:
        for expected in args.expect_hle_call:
            if f"name={expected}" not in run_log:
                errors.append(f"expected HLE call {expected!r} in run log")

    process = summary["process"]
    if process["returncode"] not in (0, None) and not process["terminated_by_harness"]:
        errors.append(f"emulator exited with returncode {process['returncode']}")
    return errors


def parse_screenshot_pair(spec):
    parts = spec.split(":", 1)
    if len(parts) != 2:
        raise ValueError(f"expected screenshot pair A:B, got {spec!r}")
    try:
        first = int(parts[0])
        second = int(parts[1])
    except ValueError as err:
        raise ValueError(f"expected numeric screenshot pair A:B, got {spec!r}") from err
    if first <= 0 or second <= 0:
        raise ValueError(f"screenshot pair indices are 1-based, got {spec!r}")
    return first, second


def parse_playability_sequence(spec):
    parts = spec.split(":")
    if len(parts) != 6:
        raise ValueError(
            "expected playability sequence WORLD:MOVED:LOOKED:AIMED:PLACED:BROKEN, "
            f"got {spec!r}"
        )
    try:
        indices = tuple(int(part) for part in parts)
    except ValueError as err:
        raise ValueError(f"expected numeric playability sequence, got {spec!r}") from err
    if any(index <= 0 for index in indices):
        raise ValueError(f"playability indices are 1-based, got {spec!r}")
    return indices


def print_summary(summary):
    errors = summary["expectation_errors"]
    print(f"mcpe-ui-smoke: trace_dir={summary['trace_dir']}")
    print(f"mcpe-ui-smoke: ws_url={summary['ws_url']}")
    print(
        "mcpe-ui-smoke: "
        f"steps={len(summary['journal']['entries'])} "
        f"screenshots={len(summary['journal']['screenshots'])} "
        f"returncode={summary['process']['returncode']} "
        f"terminated_by_harness={summary['process']['terminated_by_harness']}"
    )
    wait_debug = summary["journal"].get("wait_debug")
    if wait_debug:
        print(
            "mcpe-ui-smoke: wait_debug "
            f"frames={wait_debug.get('frames')} "
            f"draw_elements={wait_debug.get('draw_elements')} "
            f"rgb={wait_debug.get('readback_nonzero_rgb_pixels')}"
        )
    for shot in summary["journal"]["screenshots"]:
        print(
            "mcpe-ui-smoke: screenshot "
            f"{shot['path']} {shot.get('width')}x{shot.get('height')} "
            f"bytes={shot['bytes']} sha256={shot['sha256']}"
        )
        if shot.get("flat_world"):
            metrics = shot["flat_world"]
            print(
                "mcpe-ui-smoke: flat_world "
                f"screenshot={shot['path']} "
                f"sky_ratio={metrics['sky_ratio']:.3f} "
                f"ground_ratio={metrics['ground_ratio']:.3f} "
                f"passed={metrics['passed']}"
            )
    artifacts = summary.get("artifacts", {})
    print(
        "mcpe-ui-smoke: artifacts "
        f"gles_events={artifacts.get('gles_event_count', 0)} "
        f"native_events={artifacts.get('native_event_count', 0)} "
        f"pc_profile_samples={artifacts.get('pc_profile_samples', 0)} "
        f"sdl_draw_pngs={artifacts.get('sdl_draw_png_count', 0)}"
    )
    playability = summary.get("playability")
    if playability:
        print(
            "mcpe-ui-smoke: playability "
            f"move={playability['move']['changed_channel_ratio']:.3f} "
            f"look={playability['look']['changed_channel_ratio']:.3f} "
            f"aim={playability['aim']['changed_channel_ratio']:.3f} "
            f"placed_gray={playability['placed_gray_ratio']:.3f} "
            f"broken_gray={playability['broken_gray_ratio']:.3f} "
            f"broken_hole={playability['broken_hole_ratio']:.3f} "
            f"passed={playability['passed']}"
        )
    if artifacts.get("pc_profile_top"):
        top = artifacts["pc_profile_top"][0]
        where = top.get("symbol") or top.get("library") or top.get("pc_hex")
        print(
            "mcpe-ui-smoke: pc_profile "
            f"rows={artifacts.get('pc_profile_rows', 0)} "
            f"unique={artifacts.get('pc_profile_unique_buckets', 0)} "
            f"top={where}+{top.get('symbol_offset_hex', '0x0')} "
            f"count={top.get('count')} thread={top.get('thread_id')}"
        )
    if errors:
        for error in errors:
            print(f"mcpe-ui-smoke: ERROR: {error}", file=sys.stderr)
    else:
        print("mcpe-ui-smoke: ok")


def build_arg_parser():
    parser = argparse.ArgumentParser(
        description="Launch MCPE in SDL2 live mode and run a WebSocket UI journal"
    )
    parser.add_argument("--apk", type=pathlib.Path, default=DEFAULT_APK)
    parser.add_argument("--binary", type=pathlib.Path, default=DEFAULT_BINARY)
    parser.add_argument("--abi", default=DEFAULT_ABI)
    parser.add_argument("--cpu-backend", choices=["aemu", "dynarmic"], default="aemu")
    parser.add_argument("--out-dir", type=pathlib.Path)
    parser.add_argument("--display", default=":0")
    parser.add_argument("--ws", default=DEFAULT_WS_ADDR)
    parser.add_argument(
        "--steps",
        type=parse_step_limit,
        default=DEFAULT_STEPS,
        help="guest step limit per run/frame, or inf",
    )
    parser.add_argument(
        "--preset",
        choices=["play", "flat-world", "playable-flat-world"],
        help="apply a reusable MCPE UI smoke profile",
    )
    parser.add_argument(
        "--fake-time-step-nanos",
        type=int,
        help="set AEMU_FAKE_TIME_STEP_NANOS for Android time HLE diagnostics",
    )
    parser.add_argument(
        "--fake-time-step-after-draw-nanos",
        type=int,
        help="set AEMU_FAKE_TIME_STEP_AFTER_DRAW_NANOS after GLES draw submissions start",
    )
    parser.add_argument(
        "--dynarmic-run-ticks",
        type=int,
        help="set AEMU_DYNARMIC_RUN_TICKS for chunked native Dynarmic runs",
    )
    parser.add_argument(
        "--guest-thread-swap-slices",
        type=int,
        help="set AEMU_GUEST_THREAD_SWAP_SLICES for frame-boundary guest worker scheduling",
    )
    parser.add_argument(
        "--main-thread-wait-idle-spins",
        type=int,
        help="set AEMU_MAIN_THREAD_WAIT_IDLE_SPINS for deadlock detection while the main guest thread waits",
    )
    parser.add_argument(
        "--main-thread-wait-slice-steps",
        type=int,
        help="set AEMU_MAIN_THREAD_WAIT_SLICE_STEPS for guest worker scheduling while the main guest thread waits",
    )
    parser.add_argument(
        "--profile-pc",
        action="store_true",
        help="write low-overhead guest PC/function hot spot samples to pc_profile.jsonl",
    )
    parser.add_argument(
        "--profile-pc-interval",
        type=int,
        default=4096,
        help="sample one guest PC every N interpreted guest instructions",
    )
    parser.add_argument(
        "--profile-pc-limit",
        type=int,
        help="stop collecting PC samples after this many samples",
    )
    parser.add_argument(
        "--profile-pc-flush-interval",
        type=int,
        default=512,
        help="append a PC profile snapshot every N new samples",
    )
    parser.add_argument(
        "--profile-pc-top",
        type=int,
        default=80,
        help="include this many hottest buckets in each PC profile snapshot",
    )
    parser.add_argument("--frames", type=int, default=240)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--wait-ws-timeout", type=float, default=90.0)
    parser.add_argument(
        "--ws-request-timeout",
        type=float,
        default=15.0,
        help="WebSocket socket timeout for individual debug/screenshot/input requests",
    )
    parser.add_argument("--wait-frames", type=int, default=1)
    parser.add_argument("--wait-draw-elements", type=int, default=0)
    parser.add_argument("--wait-readback-rgb", type=int, default=0)
    parser.add_argument("--wait-debug-interval", type=float, default=0.25)
    parser.add_argument(
        "--first-visible-draw",
        action="store_true",
        help="wait for the validated first visible DrawElements/readback milestone before running the UI journal",
    )
    parser.add_argument("--script", help="semicolon/newline separated ws_cli journal actions")
    parser.add_argument("--script-file", help="read journal actions from a file")
    parser.add_argument("--pointer-id", type=int, default=0)
    parser.add_argument("--pressure", type=float, default=1.0)
    parser.add_argument("--tap-duration-ms", type=float, default=180.0)
    parser.add_argument("--post-journal-seconds", type=float, default=0.25)
    parser.add_argument(
        "--keep-running-until-exit",
        action="store_true",
        help="wait for --sdl2-frames process exit instead of terminating after the journal",
    )
    parser.add_argument(
        "--respect-host-quit",
        action="store_true",
        help="allow SDL2 Quit/Escape events to stop the emulator during the scripted journal",
    )
    parser.add_argument("--expect-frames", type=int, default=1)
    parser.add_argument("--max-gl-errors", type=int, default=0)
    parser.add_argument("--max-skipped-client-attrib-draws", type=int)
    parser.add_argument("--min-readback-rgb", type=int, default=0)
    parser.add_argument("--min-draw-elements", type=int, default=0)
    parser.add_argument("--min-screenshots", type=int, default=0)
    parser.add_argument("--min-screenshot-bytes", type=int, default=100)
    parser.add_argument("--min-gles-events", type=int, default=0)
    parser.add_argument("--min-native-events", type=int, default=0)
    parser.add_argument("--min-pc-profile-samples", type=int, default=0)
    parser.add_argument("--min-sdl-draw-pngs", type=int, default=0)
    parser.add_argument(
        "--expect-run-log-contains",
        action="append",
        help="require run.log to contain this exact substring",
    )
    parser.add_argument(
        "--reject-run-log-contains",
        action="append",
        help="fail if run.log contains this exact substring",
    )
    parser.add_argument(
        "--expect-hle-call",
        action="append",
        help="require a traced HLE line with name=<value> in run.log",
    )
    parser.add_argument("--gles-event-match", default="SwapBuffers,UseProgram,BindTexture,DrawElements")
    parser.add_argument("--gles-event-limit", type=int, default=50000)
    parser.add_argument(
        "--gles-event-skip",
        type=int,
        help="skip GLES events before this global event index before applying match/limit",
    )
    parser.add_argument("--trace-hle", help="set AEMU_TRACE_HLE filter")
    parser.add_argument("--trace-hle-limit", type=int)
    parser.add_argument("--trace-hle-file", action="store_true")
    parser.add_argument(
        "--native-event",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENTS spec, e.g. 0x716eb818:TextureOGL::bind",
    )
    parser.add_argument("--native-event-mem32", action="append", default=[])
    parser.add_argument("--native-event-deref32", action="append", default=[])
    parser.add_argument("--native-event-cxx-string", action="append", default=[])
    parser.add_argument("--native-event-bytes", action="append", default=[])
    parser.add_argument("--native-event-limit", type=int)
    parser.add_argument(
        "--expect-native-event",
        action="append",
        help="require a native trace event whose name or PC contains this text",
    )
    parser.add_argument("--dump-sdl-draws", action="store_true")
    parser.add_argument("--sdl-draw-match", default="all")
    parser.add_argument("--sdl-draw-limit", type=int, default=50)
    parser.add_argument(
        "--sdl-draw-skip",
        type=int,
        help="skip this many cumulative SDL draw submissions before tracing draw changes",
    )
    parser.add_argument(
        "--sdl-draw-all",
        action="store_true",
        help="trace unchanged default-framebuffer draws as well as changed draws",
    )
    parser.add_argument(
        "--expect-screenshot-change",
        action="store_true",
        help="fail unless the first and last captured screenshots have different hashes",
    )
    parser.add_argument(
        "--expect-screenshot-pair-change",
        action="append",
        help="fail unless the 1-based screenshot pair A:B has different hashes",
    )
    parser.add_argument(
        "--expect-flat-world-screenshot",
        action="append",
        type=int,
        help="require the 1-based screenshot to contain the entered flat-world sky and ground",
    )
    parser.add_argument(
        "--expect-playability-sequence",
        help=(
            "require WORLD:MOVED:LOOKED:AIMED:PLACED:BROKEN screenshots to prove "
            "in-world movement, camera, placement, and breaking"
        ),
    )
    parser.add_argument("--echo-log", action="store_true")
    return parser


def append_arg_once(args, name, value):
    values = getattr(args, name)
    if values is None:
        values = []
        setattr(args, name, values)
    if value not in values:
        values.append(value)


def apply_preset_defaults(args):
    if args.preset is None:
        return
    if args.preset == "play":
        args.first_visible_draw = True
        if not args.script and not args.script_file:
            args.script = PLAY_PRESET_SCRIPT
        args.wait_ws_timeout = max(args.wait_ws_timeout, 220.0)
        args.post_journal_seconds = max(args.post_journal_seconds, 60.0)
        args.min_screenshots = max(args.min_screenshots, 3)
        if args.max_skipped_client_attrib_draws is None:
            args.max_skipped_client_attrib_draws = 0
        append_arg_once(args, "expect_screenshot_pair_change", "1:2")
        append_arg_once(args, "reject_run_log_contains", "THREAD stall")
        append_arg_once(args, "reject_run_log_contains", "native run failed")
    elif args.preset in ("flat-world", "playable-flat-world"):
        args.first_visible_draw = True
        if not args.script and not args.script_file:
            args.script = (
                PLAYABLE_FLAT_WORLD_PRESET_SCRIPT
                if args.preset == "playable-flat-world"
                else FLAT_WORLD_PRESET_SCRIPT
            )
        if args.steps == DEFAULT_STEPS:
            args.steps = "inf"
        args.frames = max(args.frames, 20_000)
        args.wait_ws_timeout = max(args.wait_ws_timeout, 220.0)
        args.min_screenshots = max(
            args.min_screenshots, 15 if args.preset == "playable-flat-world" else 10
        )
        if args.fake_time_step_nanos is None:
            args.fake_time_step_nanos = 100_000
        if args.max_skipped_client_attrib_draws is None:
            args.max_skipped_client_attrib_draws = 0
        if args.preset == "playable-flat-world":
            args.tap_duration_ms = 50.0
            if args.expect_playability_sequence is None:
                args.expect_playability_sequence = "10:11:12:13:14:15"
        for pair in ("1:2", "2:3", "3:4", "4:5", "5:6", "6:7"):
            append_arg_once(args, "expect_screenshot_pair_change", pair)
        for index in (8, 9, 10):
            append_arg_once(args, "expect_flat_world_screenshot", index)
        append_arg_once(args, "reject_run_log_contains", "THREAD stall")
        append_arg_once(args, "reject_run_log_contains", "native run failed")
        append_arg_once(args, "reject_run_log_contains", "HLE heap exhausted")


def apply_milestone_defaults(args):
    if not args.first_visible_draw:
        return
    args.frames = max(args.frames, 260)
    args.timeout = max(args.timeout, 640.0)
    args.ws_request_timeout = max(args.ws_request_timeout, 90.0)
    if args.guest_thread_swap_slices is None:
        args.guest_thread_swap_slices = 256
    if args.main_thread_wait_slice_steps is None:
        args.main_thread_wait_slice_steps = 65536
    args.wait_draw_elements = max(args.wait_draw_elements, 1)
    args.wait_readback_rgb = max(args.wait_readback_rgb, 1)
    args.min_draw_elements = max(args.min_draw_elements, 1)
    args.min_readback_rgb = max(args.min_readback_rgb, 1)
    args.min_screenshot_bytes = max(args.min_screenshot_bytes, 1000)


def main(argv=None):
    args = build_arg_parser().parse_args(argv)
    apply_preset_defaults(args)
    apply_milestone_defaults(args)
    if not args.apk.exists():
        raise SystemExit(f"APK not found: {args.apk}")
    if not args.binary.exists():
        raise SystemExit(f"aemu binary not found: {args.binary}; run cargo build --release --features sdl2")
    try:
        out_dir = prepare_output_dir(args.out_dir)
        script_text = load_script(args, out_dir)
        steps = ws_cli.parse_journal_text(script_text)
    except Exception as err:
        raise SystemExit(str(err)) from err

    run_log_path = out_dir / "run.log"
    journal_path = out_dir / "journal.jsonl"
    summary_path = out_dir / "summary.json"
    script_path = out_dir / "journal.txt"
    script_path.write_text(script_text + "\n", encoding="utf-8")

    env = os.environ.copy()
    env.setdefault("DISPLAY", args.display)
    env.setdefault("SDL_VIDEO_X11_FORCE_EGL", "1")
    apply_trace_env(args, out_dir, env)

    cmd = build_emulator_cmd(args, args.ws)
    started = time.monotonic()
    process = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env=env,
    )
    lines, reader = start_reader(process, run_log_path)
    terminated_by_harness = False
    entries = []
    screenshots = []
    wait_debug = None
    ws_url = None
    startup_lines = []
    startup_error = None
    try:
        ws_url, startup_lines = wait_for_ws_url(process, lines, args.wait_ws_timeout)
        client = ws_cli.WsClient(ws_url, timeout=args.ws_request_timeout)
        try:
            with journal_path.open("w", encoding="utf-8") as journal:
                wait_debug = wait_for_debug_milestone(client, args, args.timeout, journal, out_dir)
            entries, screenshots = run_journal(client, steps, args, journal_path)
        finally:
            client.close()
        if args.keep_running_until_exit:
            try:
                process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                terminated_by_harness = terminate_process(process)
        else:
            time.sleep(args.post_journal_seconds)
            terminated_by_harness = terminate_process(process)
    except Exception as err:
        startup_error = str(err)
        terminated_by_harness = terminate_process(process)
    finally:
        reader.join(timeout=2)

    if args.echo_log and run_log_path.exists():
        print(run_log_path.read_text(encoding="utf-8", errors="replace"), end="")

    summary = {
        "trace_dir": str(out_dir),
        "script": str(script_path),
        "ws_url": ws_url,
        "startup_lines": startup_lines,
        "startup_error": startup_error,
        "command": cmd,
        "environment": summarize_env(env),
        "process": {
            "returncode": process.returncode,
            "elapsed_seconds": round(time.monotonic() - started, 6),
            "terminated_by_harness": terminated_by_harness,
            "run_log": str(run_log_path),
        },
        "journal": {
            "path": str(journal_path),
            "wait_debug": wait_debug,
            "entries": entries,
            "screenshots": screenshots,
        },
        "artifacts": collect_artifacts(out_dir),
        "pc_profile": {
            "enabled": args.profile_pc,
            "interval": args.profile_pc_interval if args.profile_pc else None,
            "limit": args.profile_pc_limit if args.profile_pc else None,
            "flush_interval": args.profile_pc_flush_interval if args.profile_pc else None,
            "top": args.profile_pc_top if args.profile_pc else None,
        },
    }
    errors = []
    if startup_error is not None:
        errors.append(startup_error)
    errors.extend(validate_summary(args, summary))
    summary["expectation_errors"] = errors
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print_summary(summary)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
