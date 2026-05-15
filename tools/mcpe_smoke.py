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
MCPE_LIBRARY = "libminecraftpe.so"

MCPE_NATIVE_TRACE_PRESETS = {
    "webtoken": {
        "description": "trace MCPE certificate WebToken creation without HLE-ing game logic",
        "events": [
            (0x006AFD50, "Certificate::createBasicCertificate.copy-token-call"),
            (0x006AE900, "WebToken::copy.entry"),
            (0x006B2A40, "WebToken::createFromData.entry"),
            (0x006B2A7E, "WebToken::createFromData.after-token-builder"),
            (0x006B2A8C, "WebToken::createFromData.check-token-builder"),
            (0x006B2BDE, "WebToken::createFromData.check-signature-compare"),
            (0x006B2C24, "WebToken::createFromData.return-null-token-builder"),
            (0x006B2C2C, "WebToken::createFromData.return-null-signature"),
            (0x006B2C7C, "WebToken::createFromData.return-success"),
        ],
        "mem32": [
            (0x006AFD50, "sp+0x5c,+0x60,+0xe0,+0x12c"),
            (0x006AE900, "sp+0,+0x4,+0x8,+0xc,+0x10,+0x14,+0x18,+0x1c"),
            (0x006B2A40, "r0+0"),
            (0x006B2A40, "r1+0,+0x4,+0x8,+0xc,+0x10,+0x14,+0x18,+0x1c"),
            (0x006B2A40, "r2+0,+0x4,+0x8,+0xc"),
            (0x006B2A7E, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2A8C, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2BDE, "sp+0x6c,+0x70,+0x74,+0x78"),
            (0x006B2C24, "r8+0"),
            (0x006B2C2C, "r8+0"),
            (0x006B2C7C, "r8+0"),
        ],
        "deref32": [
            (0x006B2A40, "r2+0x8,+0x4"),
        ],
        "event_limit": 200,
    },
    "keygen": {
        "description": "trace the MCPE PrivateKeyManager/OpenSSL key generation path",
        "events": [
            (0x006B0F26, "PrivateKeyManager::ctor.generate-call"),
            (0x011CD458, "Asymmetric::generateKeyPair.wrapper-entry"),
            (0x011CD45E, "Asymmetric::generateKeyPair.wrapper-jump"),
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CD9AC, "OpenSSLInterface::generateKeyPair.after-new-ctx"),
            (0x011CD9B8, "OpenSSLInterface::generateKeyPair.after-keygen-init"),
            (0x011CD9EA, "OpenSSLInterface::generateKeyPair.after-ec-curve-ctrl"),
            (0x011CD9F8, "OpenSSLInterface::generateKeyPair.after-paramgen"),
            (0x011CDA0C, "OpenSSLInterface::generateKeyPair.keygen-ctx-created"),
            (0x011CDA1A, "OpenSSLInterface::generateKeyPair.after-keygen-init2"),
            (0x011CDA28, "OpenSSLInterface::generateKeyPair.after-keygen2"),
            (0x011CDB34, "OpenSSLInterface::generateKeyPair.fail-keygen2"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x006B0F28, "PrivateKeyManager::ctor.generate-return"),
        ],
        "mem32": [
            (0x011CD458, "r0+0,+0x4,+0x8,+0xc,+0x10"),
            (0x011CD45E, "r0+0,+0x4,+0x8,+0xc,+0x10"),
        ],
        "cxx_string": [
            (0x006B0F28, "r4+0x4,128"),
            (0x006B0F28, "r4+0xc,128"),
        ],
        "event_limit": 200,
    },
    "keygen-ec": {
        "description": "trace bundled OpenSSL EC key generation and public-point multiply",
        "events": [
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CDA28, "OpenSSLInterface::generateKeyPair.after-keygen2"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x012399B8, "EC_KEY_generate_key.entry"),
            (0x01239A08, "EC_KEY_generate_key.after-private-rand"),
            (0x01239A4C, "EC_KEY_generate_key.private-ready"),
            (0x01239A98, "EC_KEY_generate_key.after-point-mul"),
            (0x01239AA4, "EC_KEY_generate_key.success-store"),
            (0x01239A40, "EC_KEY_generate_key.cleanup-return"),
            (0x01237240, "EC_POINT_mul.entry"),
            (0x01237280, "EC_POINT_mul.call-method"),
            (0x01237284, "EC_POINT_mul.after-method"),
            (0x012378EC, "ec_wNAF_mul.entry"),
            (0x01236CFC, "EC_POINT_is_on_curve.entry"),
            (0x01236D40, "EC_POINT_is_on_curve.after-method"),
            (0x012EC640, "ec_GFp_simple_is_on_curve.entry"),
            (0x012EC71C, "ec_GFp_simple_is_on_curve.return"),
        ],
        "mem32": [
            (0x012399B8, "r0+0x4,+0x8,+0xc"),
            (0x01239A08, "r0"),
            (0x01239A4C, "r4+0,+0x4"),
            (0x01239A98, "r0"),
            (0x01239AA4, "r6+0x8,+0xc"),
            (0x01237240, "r0+0"),
            (0x01237240, "r1+0"),
            (0x01237240, "r2+0"),
            (0x01237284, "r0"),
            (0x01236D40, "r0"),
            (0x012EC71C, "r0"),
        ],
        "event_limit": 300,
    },
    "keygen-serialize": {
        "description": "trace bundled OpenSSL EC private-key DER serialization and point2oct output",
        "events": [
            (0x011CD988, "OpenSSLInterface::generateKeyPair.entry"),
            (0x011CDB48, "OpenSSLInterface::generateKeyPair.return"),
            (0x01255684, "i2d_PrivateKey.entry"),
            (0x012556C0, "i2d_PrivateKey.call-ec"),
            (0x012556D4, "i2d_PrivateKey.return"),
            (0x012A254C, "i2d_ECPrivateKey.entry"),
            (0x012A2594, "i2d_ECPrivateKey.after-struct-new"),
            (0x012A2688, "i2d_ECPrivateKey.after-private-octet"),
            (0x012A26D4, "i2d_ECPrivateKey.after-point2oct-size"),
            (0x012A2728, "i2d_ECPrivateKey.after-point2oct-write"),
            (0x012A2914, "i2d_ECPrivateKey.have-public-bit-string"),
            (0x012A2938, "i2d_ECPrivateKey.after-public-bit-string"),
            (0x012A27E4, "i2d_ECPrivateKey.after-asn1-write"),
            (0x0123A3F0, "EC_POINT_point2oct.entry"),
            (0x0123A4B8, "EC_POINT_point2oct.call-method"),
            (0x012A81B0, "ec_GFp_simple_point2oct.entry"),
            (0x012A828C, "ec_GFp_simple_point2oct.have-output-len"),
            (0x012A82F0, "ec_GFp_simple_point2oct.after-affine"),
            (0x012A8330, "ec_GFp_simple_point2oct.after-form-byte"),
            (0x012A8378, "ec_GFp_simple_point2oct.after-x-bytes"),
            (0x012A8420, "ec_GFp_simple_point2oct.after-y-bytes"),
            (0x012A8470, "ec_GFp_simple_point2oct.return-uncompressed"),
            (0x012A8490, "ec_GFp_simple_point2oct.return-cleanup"),
        ],
        "mem32": [
            (0x01255684, "r0+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012556D4, "r0"),
            (0x012A254C, "r0+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A2688, "r0"),
            (0x012A26D4, "r0"),
            (0x012A2728, "r0"),
            (0x012A2938, "r0"),
            (0x0123A3F0, "r0+0"),
            (0x0123A3F0, "r1+0"),
            (0x012A81B0, "r0+0"),
            (0x012A81B0, "r1+0"),
            (0x012A828C, "r0"),
            (0x012A82F0, "r0"),
            (0x012A8470, "r6"),
            (0x012A8490, "r6"),
        ],
        "bytes": [
            (0x0123A3F0, "r3+0,100"),
            (0x012A81B0, "r3+0,100"),
            (0x012A8470, "r5+0,100"),
            (0x012A8490, "r5+0,100"),
        ],
        "event_limit": 300,
    },
    "signdata": {
        "description": "trace MCPE PrivateKeyManager/OpenSSL signing after key generation succeeds",
        "events": [
            (0x006B11A0, "PrivateKeyManager::sign.entry"),
            (0x006B11BA, "PrivateKeyManager::sign.virtual-call"),
            (0x011CDD28, "OpenSSLInterface::signData.entry"),
            (0x011CDD54, "OpenSSLInterface::signData.after-d2i-private"),
            (0x011CDD62, "OpenSSLInterface::signData.after-ctx-new"),
            (0x011CDD6E, "OpenSSLInterface::signData.after-sign-init"),
            (0x011CDD94, "OpenSSLInterface::signData.after-ec-curve-ctrl"),
            (0x011CDE28, "OpenSSLInterface::signData.after-ctx-ctrl"),
            (0x011CDE48, "OpenSSLInterface::signData.after-sign-size"),
            (0x011CDE68, "OpenSSLInterface::signData.after-sign-data"),
            (0x011CDE82, "OpenSSLInterface::signData.success"),
            (0x011CDDB2, "OpenSSLInterface::signData.fail-private"),
            (0x011CDDBE, "OpenSSLInterface::signData.fail-ctx"),
            (0x011CDDD0, "OpenSSLInterface::signData.fail-sign-init"),
            (0x011CDDE8, "OpenSSLInterface::signData.fail-ec-curve"),
            (0x011CDF00, "OpenSSLInterface::signData.fail-ctx-ctrl"),
            (0x011CDE96, "OpenSSLInterface::signData.fail-sign-size"),
            (0x011CDEAE, "OpenSSLInterface::signData.fail-sign-data"),
            (0x006B11BC, "PrivateKeyManager::sign.returned"),
        ],
        "mem32": [
            (0x011CDE48, "sp+0x14"),
            (0x011CDE68, "sp+0x14"),
            (0x011CDE82, "r9+0"),
            (0x006B11BC, "r0+0"),
        ],
        "deref32": [
            (0x006B11A0, "r1+0x8,+0,+0x14"),
        ],
        "cxx_string": [
            (0x006B11A0, "r2+0,768"),
            (0x011CDD28, "r2+0,192"),
            (0x011CDD28, "r3+0,768"),
            (0x006B11BC, "r0+0,256"),
        ],
        "bytes": [
            (0x011CDD28, "*r2+0,167"),
        ],
        "event_limit": 250,
    },
    "d2i-private": {
        "description": "trace bundled OpenSSL d2i_AutoPrivateKey and EC private-key decode",
        "events": [
            (0x0125555C, "d2i_AutoPrivateKey.entry"),
            (0x01255584, "d2i_AutoPrivateKey.after-sequence-any"),
            (0x0125558C, "d2i_AutoPrivateKey.after-type-num-1"),
            (0x0125559C, "d2i_AutoPrivateKey.after-type-num-2"),
            (0x012555AC, "d2i_AutoPrivateKey.after-type-num-3"),
            (0x012555B8, "d2i_AutoPrivateKey.selected-keytype"),
            (0x012555D8, "d2i_AutoPrivateKey.after-privatekey-decode"),
            (0x01255604, "d2i_AutoPrivateKey.after-pkcs8-decode"),
            (0x01255618, "d2i_AutoPrivateKey.after-pkcs8-convert"),
            (0x01255648, "d2i_AutoPrivateKey.fail-pkcs8-convert"),
            (0x01255670, "d2i_AutoPrivateKey.success-no-output-arg"),
            (0x012A2170, "d2i_ECPrivateKey.entry"),
            (0x012A2184, "d2i_ECPrivateKey.after-asn1-item-d2i"),
            (0x012A21A0, "d2i_ECPrivateKey.have-output-key"),
            (0x012A21CC, "d2i_ECPrivateKey.check-parameters-type"),
            (0x012A2214, "d2i_ECPrivateKey.after-parameters"),
            (0x012A2220, "d2i_ECPrivateKey.have-group"),
            (0x012A223C, "d2i_ECPrivateKey.after-private-key-bn"),
            (0x012A2260, "d2i_ECPrivateKey.after-public-point-new"),
            (0x012A22A8, "d2i_ECPrivateKey.after-public-point-decode"),
            (0x012A22B0, "d2i_ECPrivateKey.success"),
            (0x012A22C4, "d2i_ECPrivateKey.return"),
            (0x012A231C, "d2i_ECPrivateKey.fail-missing-public-key"),
            (0x012A2364, "d2i_ECPrivateKey.fail-missing-private-key"),
            (0x012A2388, "d2i_ECPrivateKey.fail-public-point-decode"),
            (0x012A23C8, "d2i_ECPrivateKey.fail-missing-group"),
            (0x012A23EC, "d2i_ECPrivateKey.fail-private-key-bn"),
            (0x012A2410, "d2i_ECPrivateKey.fail-public-point-new"),
            (0x012A2434, "d2i_ECPrivateKey.derive-public-from-private"),
            (0x012A244C, "d2i_ECPrivateKey.after-public-derive"),
            (0x012A2460, "d2i_ECPrivateKey.fail-public-derive"),
            (0x012A2484, "d2i_ECPrivateKey.fail-asn1-item-d2i"),
            (0x0123A4D4, "EC_POINT_oct2point.entry"),
            (0x0123A594, "EC_POINT_oct2point.call-method"),
            (0x0123A5A0, "EC_POINT_oct2point.after-method"),
            (0x012A8520, "ec_GFp_simple_oct2point.entry"),
            (0x012A8550, "ec_GFp_simple_oct2point.after-form-parse"),
            (0x012A8624, "ec_GFp_simple_oct2point.after-length-check"),
            (0x012A86BC, "ec_GFp_simple_oct2point.fail-length"),
            (0x012A866C, "ec_GFp_simple_oct2point.after-x-bin2bn"),
            (0x012A8684, "ec_GFp_simple_oct2point.after-x-range"),
            (0x012A87AC, "ec_GFp_simple_oct2point.after-y-bin2bn"),
            (0x012A87C4, "ec_GFp_simple_oct2point.after-y-range"),
            (0x012A8870, "ec_GFp_simple_oct2point.after-set-affine"),
            (0x012A873C, "ec_GFp_simple_oct2point.after-is-on-curve"),
            (0x012A8748, "ec_GFp_simple_oct2point.fail-not-on-curve"),
            (0x012A8768, "ec_GFp_simple_oct2point.fail"),
            (0x012A876C, "ec_GFp_simple_oct2point.cleanup"),
        ],
        "mem32": [
            (0x0125555C, "r1+0"),
            (0x01255584, "sp+0xc"),
            (0x012555B8, "r8"),
            (0x012555D8, "r0"),
            (0x01255604, "r0"),
            (0x01255618, "r5"),
            (0x012A2170, "r1+0"),
            (0x012A2184, "r0"),
            (0x012A21A0, "r4+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A21CC, "r7+0,+0x4"),
            (0x012A2214, "r7+0"),
            (0x012A2220, "r5+0,+0x4,+0x8,+0xc"),
            (0x012A223C, "r0"),
            (0x012A2260, "r0"),
            (0x012A22A8, "r0"),
            (0x012A22B0, "r4+0,+0x4,+0x8,+0xc,+0x10,+0x14"),
            (0x012A22C4, "r4"),
            (0x012A244C, "r0"),
            (0x0123A4D4, "r0+0"),
            (0x0123A4D4, "r1+0"),
            (0x0123A594, "r2+0"),
            (0x0123A5A0, "r0"),
            (0x012A8520, "r0+0"),
            (0x012A8520, "r1+0"),
            (0x012A8520, "r2+0"),
            (0x012A8550, "r2"),
            (0x012A8624, "r2"),
            (0x012A866C, "r0"),
            (0x012A8684, "r0"),
            (0x012A87AC, "r0"),
            (0x012A87C4, "r0"),
            (0x012A8870, "r0"),
            (0x012A873C, "r0"),
        ],
        "bytes": [
            (0x0125555C, "*r1+0,167"),
            (0x012A2170, "*r1+0,167"),
            (0x0123A4D4, "r2+0,100"),
            (0x012A8520, "r2+0,100"),
        ],
        "event_limit": 300,
    },
}

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


def object_by_name(objects, name: str):
    for obj in objects:
        if obj["name"] == name:
            return obj
    return None


def runtime_pc(objects, library_name: str, offset: int) -> int:
    obj = object_by_name(objects, library_name)
    if obj is None:
        raise RuntimeError(f"{library_name} was not linked; cannot resolve native trace preset")
    return obj["load_bias"] + offset


def trace_spec_for_offset(objects, offset: int, suffix: str, *, library_name: str = MCPE_LIBRARY) -> str:
    return f"0x{runtime_pc(objects, library_name, offset):08x}:{suffix}"


def append_native_trace_preset(config, preset_name: str, objects):
    preset = MCPE_NATIVE_TRACE_PRESETS[preset_name]
    for offset, name in preset["events"]:
        config["events"].append(trace_spec_for_offset(objects, offset, name))
    for offset, fields in preset.get("mem32", []):
        config["mem32"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("deref32", []):
        config["deref32"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("cxx_string", []):
        config["cxx_string"].append(trace_spec_for_offset(objects, offset, fields))
    for offset, fields in preset.get("bytes", []):
        config["bytes"].append(trace_spec_for_offset(objects, offset, fields))
    config["presets"].append(
        {
            "name": preset_name,
            "description": preset["description"],
            "library": MCPE_LIBRARY,
            "event_count": len(preset["events"]),
        }
    )
    config["event_limit"] = max(config["event_limit"] or 0, preset.get("event_limit", 0)) or None


def build_native_trace_config(args, objects):
    config = {
        "presets": [],
        "events": list(args.native_event or []),
        "mem32": list(args.native_event_mem32 or []),
        "deref32": list(args.native_event_deref32 or []),
        "cxx_string": list(args.native_event_cxx_string or []),
        "bytes": list(args.native_event_bytes or []),
        "event_limit": args.native_event_limit,
    }
    for preset_name in args.native_trace_preset or []:
        append_native_trace_preset(config, preset_name, objects)
    return config


def apply_native_trace_env(env, trace_dir: pathlib.Path, config):
    if not config["events"]:
        return
    env["AEMU_TRACE_NATIVE_EVENTS_JSONL"] = str(trace_dir / "native_events.jsonl")
    env["AEMU_TRACE_NATIVE_EVENTS"] = ";".join(config["events"])
    if config["mem32"]:
        env["AEMU_TRACE_NATIVE_EVENT_MEM32"] = ";".join(config["mem32"])
    if config["deref32"]:
        env["AEMU_TRACE_NATIVE_EVENT_DEREF32"] = ";".join(config["deref32"])
    if config["cxx_string"]:
        env["AEMU_TRACE_NATIVE_EVENT_CXX_STRING"] = ";".join(config["cxx_string"])
    if config["bytes"]:
        env["AEMU_TRACE_NATIVE_EVENT_BYTES"] = ";".join(config["bytes"])
    if config["event_limit"] is not None:
        env["AEMU_TRACE_NATIVE_EVENTS_LIMIT"] = str(config["event_limit"])


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
        "hle_trace_count": sum(
            1 for line in run_log.splitlines() if line.startswith("HLE function=")
        ),
        "hle_file_trace_count": sum(
            1 for line in run_log.splitlines() if line.startswith("HLE file ")
        ),
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
    native_events_path = trace_dir / "native_events.jsonl"
    return {
        "gles_events_jsonl": str(gles_path),
        "gles_event_count": count_jsonl(gles_path),
        "native_events_jsonl": str(native_events_path),
        "native_event_count": count_jsonl(native_events_path),
        "sdl_draw_dir": str(draw_dir),
        "sdl_draw_png_count": len(list(draw_dir.glob("*.png"))) if draw_dir.exists() else 0,
        "sdl_draw_manifest_count": count_jsonl(draw_dir / "draw_manifest.jsonl"),
    }


def native_event_matches(row: dict, needle: str) -> bool:
    needle = needle.lower()
    event = row.get("event")
    if isinstance(event, str) and needle in event.lower():
        return True
    pc = row.get("pc")
    return isinstance(pc, int) and needle in f"0x{pc:08x}".lower()


def read_jsonl(path: pathlib.Path):
    if not path.exists():
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows


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
    if args.expect_native_event:
        native_events = read_jsonl(pathlib.Path(summary["artifacts"]["native_events_jsonl"]))
        for expected in args.expect_native_event:
            if not any(native_event_matches(row, expected) for row in native_events):
                errors.append(f"expected native event matching {expected!r}")
    if args.expect_run_log_contains:
        run_log = pathlib.Path(summary["run_log"]).read_text(encoding="utf-8", errors="replace")
        for expected in args.expect_run_log_contains:
            if expected not in run_log:
                errors.append(f"expected run log to contain {expected!r}")
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
    native_trace = summary.get("native_trace") or {}
    if native_trace.get("presets") or native_trace.get("events"):
        presets = ",".join(preset["name"] for preset in native_trace.get("presets", [])) or "manual"
        print(
            "native_trace: "
            f"presets={presets} events={len(native_trace.get('events', []))} "
            f"limit={native_trace.get('event_limit')}"
        )
    hle_trace = summary.get("hle_trace") or {}
    if hle_trace.get("filter") or hle_trace.get("file_trace"):
        print(
            "hle_trace: "
            f"filter={hle_trace.get('filter')} "
            f"limit={hle_trace.get('limit')} "
            f"calls={summary['run'].get('hle_trace_count', 0)} "
            f"file_lines={summary['run'].get('hle_file_trace_count', 0)}"
        )
    artifacts = summary["artifacts"]
    print(
        "artifacts: "
        f"gles_events={artifacts['gles_event_count']} "
        f"native_events={artifacts['native_event_count']} "
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
    parser.add_argument(
        "--native-trace-preset",
        action="append",
        choices=sorted(MCPE_NATIVE_TRACE_PRESETS),
        help="enable a built-in native PC trace preset using the linked object load bias",
    )
    parser.add_argument(
        "--native-event",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENTS spec, e.g. 0x70bb2a40:name",
    )
    parser.add_argument(
        "--native-event-mem32",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_MEM32 spec",
    )
    parser.add_argument(
        "--native-event-deref32",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_DEREF32 spec",
    )
    parser.add_argument(
        "--native-event-cxx-string",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_CXX_STRING spec",
    )
    parser.add_argument(
        "--native-event-bytes",
        action="append",
        help="append raw AEMU_TRACE_NATIVE_EVENT_BYTES spec, e.g. 0x716cdd28:*r2+0,192",
    )
    parser.add_argument("--native-event-limit", type=int)
    parser.add_argument(
        "--trace-hle",
        help="set AEMU_TRACE_HLE filter, e.g. '*' or '=open,=read,=fopen,=fread'",
    )
    parser.add_argument("--trace-hle-limit", type=int)
    parser.add_argument(
        "--trace-hle-file",
        action="store_true",
        help="enable AEMU_TRACE_HLE_FILE file/random/stdio diagnostics in run.log",
    )
    parser.add_argument("--expect-crash-pc")
    parser.add_argument("--expect-fault-address")
    parser.add_argument("--expect-stage", choices=[stage for stage, _marker in STAGE_MARKERS])
    parser.add_argument("--expect-exit", choices=["any", "zero", "nonzero"], default="any")
    parser.add_argument(
        "--expect-native-event",
        action="append",
        help="require at least one structured native event whose name or PC contains this text",
    )
    parser.add_argument(
        "--expect-run-log-contains",
        action="append",
        help="require run.log to contain this exact substring",
    )
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
    try:
        native_trace_config = build_native_trace_config(args, objects)
    except RuntimeError as err:
        raise SystemExit(str(err)) from err

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
    if args.trace_hle:
        env["AEMU_TRACE_HLE"] = args.trace_hle
    if args.trace_hle_limit is not None:
        env["AEMU_TRACE_HLE_LIMIT"] = str(args.trace_hle_limit)
    if args.trace_hle_file:
        env["AEMU_TRACE_HLE_FILE"] = "1"
    apply_native_trace_env(env, trace_dir, native_trace_config)

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
        "native_trace": native_trace_config,
        "hle_trace": {
            "filter": args.trace_hle,
            "limit": args.trace_hle_limit,
            "file_trace": args.trace_hle_file,
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
