#!/usr/bin/env python3
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time


DEFAULT_APK = pathlib.Path("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk")
DEFAULT_BINARY = pathlib.Path("target/release/aemu")
DEFAULT_ABI = "armeabi-v7a"
DEFAULT_OUT_DIR = pathlib.Path("target/mcpe-smoke")

OBJECT_RE = re.compile(
    r"^\s+(?P<name>[^:]+): load_bias (?P<load_bias>0x[0-9a-fA-F]+), "
    r"mapped (?P<memory_base>0x[0-9a-fA-F]+)\+(?P<memory_size>0x[0-9a-fA-F]+),"
)
CRASH_RE = re.compile(
    r"address (?P<fault>0x[0-9a-fA-F]+) is not mapped .* "
    r"while executing (?P<isa>Arm|Thumb) at (?P<pc>0x[0-9a-fA-F]+)"
)
RECENT_PC_RE = re.compile(
    r"^\s+(?P<isa>Arm|Thumb) pc=(?P<pc>0x[0-9a-fA-F]+).* "
    r"sp=(?P<sp>0x[0-9a-fA-F]+) lr=(?P<lr>0x[0-9a-fA-F]+)$"
)
LABEL_RE = re.compile(r"^\s*([0-9a-fA-F]+) <([^>]+)>:")


STAGE_MARKERS = [
    ("constructors", "native constructors completed"),
    ("fmod_jni", "launch: libfmod.so JNI_OnLoad"),
    ("mcpe_jni", "launch: libminecraftpe.so JNI_OnLoad"),
    ("native_register_this", "launch: nativeRegisterThis"),
    ("activity_on_create", "launch: ANativeActivity_onCreate"),
    ("android_main", "launch: android_main"),
    ("completed", "native activity launch returned"),
]


def parse_u32(raw: str) -> int:
    return int(raw, 16 if raw.lower().startswith("0x") else 10)


def run_capture(cmd, *, env=None, timeout=60, log_path=None):
    started = time.time()
    timed_out = False
    try:
        completed = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
            env=env,
        )
        output = completed.stdout or ""
        returncode = completed.returncode
    except subprocess.TimeoutExpired as err:
        timed_out = True
        output = err.stdout or ""
        if isinstance(output, bytes):
            output = output.decode("utf-8", errors="replace")
        returncode = None
    elapsed = time.time() - started
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text(output, encoding="utf-8")
    return {
        "cmd": cmd,
        "returncode": returncode,
        "timed_out": timed_out,
        "elapsed_seconds": round(elapsed, 3),
        "output": output,
    }


def unique_trace_dir(base: pathlib.Path) -> pathlib.Path:
    stamp = int(time.time())
    for idx in range(100):
        candidate = base.parent / f"{base.name}-{stamp}" if idx == 0 else base.parent / f"{base.name}-{stamp}-{idx}"
        try:
            candidate.mkdir(parents=True)
            return candidate
        except FileExistsError:
            continue
    raise RuntimeError(f"could not create a unique trace directory under {base.parent}")


def prepare_trace_dir(args) -> pathlib.Path:
    if args.trace_dir:
        trace_dir = pathlib.Path(args.trace_dir)
        trace_dir.mkdir(parents=True, exist_ok=True)
        if any(trace_dir.iterdir()) and not args.allow_existing_trace_dir:
            raise RuntimeError(
                f"{trace_dir} is not empty; use --allow-existing-trace-dir or choose a new path"
            )
        return trace_dir
    return unique_trace_dir(pathlib.Path(args.out_dir))


def parse_link_objects(link_log: str):
    objects = []
    for line in link_log.splitlines():
        match = OBJECT_RE.match(line)
        if not match:
            continue
        objects.append(
            {
                "name": match.group("name"),
                "load_bias": parse_u32(match.group("load_bias")),
                "memory_base": parse_u32(match.group("memory_base")),
                "memory_size": parse_u32(match.group("memory_size")),
            }
        )
    return objects


def parse_run_log(run_log: str):
    reached_stage = None
    for stage, marker in STAGE_MARKERS:
        if marker in run_log:
            reached_stage = stage

    crash = None
    match = CRASH_RE.search(run_log)
    if match:
        crash = {
            "fault_address": parse_u32(match.group("fault")),
            "isa": match.group("isa"),
            "pc": parse_u32(match.group("pc")),
        }

    recent = []
    for line in run_log.splitlines():
        match = RECENT_PC_RE.match(line)
        if not match:
            continue
        recent.append(
            {
                "isa": match.group("isa"),
                "pc": parse_u32(match.group("pc")),
                "sp": parse_u32(match.group("sp")),
                "lr": parse_u32(match.group("lr")),
            }
        )
    return {
        "reached_stage": reached_stage,
        "native_run_failed": "native run failed:" in run_log,
        "crash": crash,
        "recent_guest_pcs": recent,
    }


def extracted_so_path(apk: pathlib.Path, abi: str, library_name: str) -> pathlib.Path | None:
    if apk.suffix != ".apk":
        return None
    extracted = apk.with_suffix("")
    path = extracted / "lib" / abi / library_name
    return path if path.exists() else None


def symbolicate_pc(pc: int, isa: str | None, objects, apk: pathlib.Path, abi: str):
    selected = None
    for obj in objects:
        base = obj["memory_base"]
        end = base + obj["memory_size"]
        if base <= pc < end:
            selected = obj
            break
    if selected is None:
        return None

    offset = pc - selected["load_bias"]
    result = {
        "object": selected["name"],
        "load_bias": selected["load_bias"],
        "offset": offset,
    }
    so_path = extracted_so_path(apk, abi, selected["name"])
    if so_path is None:
        return result
    result["so_path"] = str(so_path)

    objdump = shutil.which("llvm-objdump")
    if objdump is None:
        return result

    start = max(0, offset - 0x20) & ~1
    stop = offset + 0x60
    cmd = [objdump, "-d", f"--start-address=0x{start:x}", f"--stop-address=0x{stop:x}", str(so_path)]
    if isa == "Thumb":
        cmd.insert(2, "--triple=thumbv7-none-linux-gnueabi")
    try:
        completed = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=10,
        )
    except subprocess.TimeoutExpired:
        result["disassembly_error"] = "llvm-objdump timed out"
        return result

    result["disassembly_returncode"] = completed.returncode
    result["disassembly"] = completed.stdout.splitlines()
    nearest = None
    for line in completed.stdout.splitlines():
        match = LABEL_RE.match(line)
        if not match:
            continue
        label_addr = int(match.group(1), 16)
        if label_addr <= offset:
            nearest = {
                "address": label_addr,
                "name": match.group(2),
                "delta": offset - label_addr,
            }
    if nearest is not None:
        result["nearest_symbol"] = nearest
    return result


def count_jsonl(path: pathlib.Path) -> int:
    if not path.exists():
        return 0
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for line in handle if line.strip())


def collect_artifacts(trace_dir: pathlib.Path):
    draw_dir = trace_dir / "sdl-draw"
    gles_path = trace_dir / "gles_events.jsonl"
    return {
        "gles_events_jsonl": str(gles_path),
        "gles_event_count": count_jsonl(gles_path),
        "sdl_draw_dir": str(draw_dir),
        "sdl_draw_png_count": len(list(draw_dir.glob("*.png"))) if draw_dir.exists() else 0,
        "sdl_draw_manifest_count": count_jsonl(draw_dir / "draw_manifest.jsonl"),
    }


def validate_expectations(args, summary):
    errors = []
    if args.expect_crash_pc is not None:
        crash = summary["run"].get("crash")
        actual = None if crash is None else crash["pc"]
        expected = parse_u32(args.expect_crash_pc)
        if actual != expected:
            errors.append(f"expected crash pc 0x{expected:08x}, got {format_hex(actual)}")
    if args.expect_fault_address is not None:
        crash = summary["run"].get("crash")
        actual = None if crash is None else crash["fault_address"]
        expected = parse_u32(args.expect_fault_address)
        if actual != expected:
            errors.append(f"expected fault address 0x{expected:08x}, got {format_hex(actual)}")
    if args.expect_stage is not None and summary["run"].get("reached_stage") != args.expect_stage:
        errors.append(
            f"expected stage {args.expect_stage}, got {summary['run'].get('reached_stage')}"
        )
    if args.expect_exit == "zero" and summary["process"]["returncode"] != 0:
        errors.append(f"expected zero exit, got {summary['process']['returncode']}")
    if args.expect_exit == "nonzero" and summary["process"]["returncode"] == 0:
        errors.append("expected nonzero exit, got 0")
    return errors


def format_hex(value):
    return "None" if value is None else f"0x{value:08x}"


def print_summary(summary, expectation_errors):
    crash = summary["run"].get("crash")
    symbolication = summary["run"].get("symbolication")
    print(f"trace_dir: {summary['trace_dir']}")
    print(
        "process: "
        f"returncode={summary['process']['returncode']} "
        f"timed_out={summary['process']['timed_out']} "
        f"elapsed={summary['process']['elapsed_seconds']}s"
    )
    print(f"stage: {summary['run'].get('reached_stage')}")
    if crash:
        print(
            "crash: "
            f"isa={crash['isa']} pc=0x{crash['pc']:08x} "
            f"fault=0x{crash['fault_address']:08x}"
        )
    if symbolication:
        nearest = symbolication.get("nearest_symbol") or {}
        symbol = nearest.get("name", "?")
        delta = nearest.get("delta")
        delta_text = "" if delta is None else f"+0x{delta:x}"
        print(
            "symbolication: "
            f"{symbolication.get('object')}+0x{symbolication.get('offset', 0):08x} "
            f"{symbol}{delta_text}"
        )
    artifacts = summary["artifacts"]
    print(
        "artifacts: "
        f"gles_events={artifacts['gles_event_count']} "
        f"sdl_draw_pngs={artifacts['sdl_draw_png_count']} "
        f"sdl_draw_manifest_rows={artifacts['sdl_draw_manifest_count']}"
    )
    print(f"run_log: {summary['run_log']}")
    print(f"summary_json: {summary['summary_json']}")
    if expectation_errors:
        print("expectations: failed")
        for error in expectation_errors:
            print(f"  {error}")
    else:
        print("expectations: ok")


def build_arg_parser():
    parser = argparse.ArgumentParser(
        description="Run the default MCPE SDL2 smoke path and write deterministic trace artifacts."
    )
    parser.add_argument("--apk", default=str(DEFAULT_APK))
    parser.add_argument("--abi", default=DEFAULT_ABI)
    parser.add_argument("--binary", default=str(DEFAULT_BINARY))
    parser.add_argument("--out-dir", default=str(DEFAULT_OUT_DIR))
    parser.add_argument("--trace-dir")
    parser.add_argument("--allow-existing-trace-dir", action="store_true")
    parser.add_argument("--frames", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--display", default=":0")
    parser.add_argument("--gles-event-limit", type=int, default=2000)
    parser.add_argument("--draw-dump-limit", type=int, default=10)
    parser.add_argument("--expect-crash-pc")
    parser.add_argument("--expect-fault-address")
    parser.add_argument("--expect-stage", choices=[stage for stage, _marker in STAGE_MARKERS])
    parser.add_argument("--expect-exit", choices=["any", "zero", "nonzero"], default="any")
    parser.add_argument("--echo-log", action="store_true")
    return parser


def main(argv=None):
    args = build_arg_parser().parse_args(argv)
    apk = pathlib.Path(args.apk)
    binary = pathlib.Path(args.binary)
    if not apk.exists():
        raise SystemExit(f"APK not found: {apk}")
    if not binary.exists():
        raise SystemExit(f"aemu binary not found: {binary}; run cargo build --release --features sdl2")

    try:
        trace_dir = prepare_trace_dir(args)
    except RuntimeError as err:
        raise SystemExit(str(err)) from err

    link_log_path = trace_dir / "link.log"
    run_log_path = trace_dir / "run.log"
    summary_path = trace_dir / "summary.json"

    link = run_capture(
        [str(binary), "link-apk", str(apk), "--abi", args.abi, "--limit", "0"],
        timeout=30,
        log_path=link_log_path,
    )
    objects = parse_link_objects(link["output"])

    env = os.environ.copy()
    env.setdefault("DISPLAY", args.display)
    env.setdefault("SDL_VIDEO_X11_FORCE_EGL", "1")
    env["AEMU_TRACE_GLES_EVENTS_JSONL"] = str(trace_dir / "gles_events.jsonl")
    env["AEMU_TRACE_GLES_EVENTS_MATCH"] = (
        "SwapBuffers,UseProgram,BindTexture,DrawElements,TexImage2D,TexSubImage2D"
    )
    env["AEMU_TRACE_GLES_EVENTS_LIMIT"] = str(args.gles_event_limit)
    env["AEMU_TRACE_SDL_DRAW_CHANGES"] = "50"
    env["AEMU_DUMP_SDL_DRAW_CHANGES_DIR"] = str(trace_dir / "sdl-draw")
    env["AEMU_DUMP_SDL_DRAW_CHANGES_MATCH"] = "all"
    env["AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT"] = str(args.draw_dump_limit)

    cmd = [
        str(binary),
        "run-apk-native",
        str(apk),
        "--abi",
        args.abi,
        "--sdl2-live",
        "--sdl2-frames",
        str(args.frames),
    ]
    run = run_capture(cmd, env=env, timeout=args.timeout, log_path=run_log_path)
    if args.echo_log and run["output"]:
        print(run["output"], end="")

    parsed_run = parse_run_log(run["output"])
    crash = parsed_run.get("crash")
    if crash is not None:
        parsed_run["symbolication"] = symbolicate_pc(
            crash["pc"], crash.get("isa"), objects, apk, args.abi
        )

    summary = {
        "trace_dir": str(trace_dir),
        "apk": str(apk),
        "abi": args.abi,
        "binary": str(binary),
        "link_log": str(link_log_path),
        "run_log": str(run_log_path),
        "summary_json": str(summary_path),
        "link": {
            "returncode": link["returncode"],
            "timed_out": link["timed_out"],
            "elapsed_seconds": link["elapsed_seconds"],
            "objects": objects,
        },
        "process": {
            "cmd": cmd,
            "returncode": run["returncode"],
            "timed_out": run["timed_out"],
            "elapsed_seconds": run["elapsed_seconds"],
        },
        "run": parsed_run,
        "artifacts": collect_artifacts(trace_dir),
    }
    summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    expectation_errors = validate_expectations(args, summary)
    print_summary(summary, expectation_errors)

    if expectation_errors:
        return 1
    if args.expect_exit == "any" and not (
        args.expect_crash_pc or args.expect_fault_address or args.expect_stage
    ):
        return 0 if run["returncode"] == 0 else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
