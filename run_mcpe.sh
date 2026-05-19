#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-live}"
APK="${APK:-/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk}"
ABI="${ABI:-armeabi-v7a}"
CPU_BACKEND="${CPU_BACKEND:-dynarmic}"
BIN="${BIN:-target/release/aemu}"
FEATURES="${FEATURES:-sdl2,dynarmic}"
STEPS="${STEPS:-600000000}"
WS_ADDR="${WS_ADDR:-127.0.0.1:8766}"
export DISPLAY="${DISPLAY:-:0}"
export SDL_VIDEO_X11_FORCE_EGL="${SDL_VIDEO_X11_FORCE_EGL:-1}"

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
    cargo build --release --features "$FEATURES"
fi

case "$MODE" in
    live)
        cmd=(
            "$BIN" run-apk-native "$APK"
            --abi "$ABI"
            --cpu-backend "$CPU_BACKEND"
            --steps "$STEPS"
            --sdl2-live
            --ws "$WS_ADDR"
        )
        if [[ -n "${AEMU_FRAMES:-}" ]]; then
            cmd+=(--sdl2-frames "$AEMU_FRAMES")
        fi
        printf 'DISPLAY=%s SDL_VIDEO_X11_FORCE_EGL=%s\n' "$DISPLAY" "$SDL_VIDEO_X11_FORCE_EGL"
        printf 'WebSocket: ws://%s\n' "$WS_ADDR"
        printf 'Command:'
        printf ' %q' "${cmd[@]}"
        printf '\n'
        exec "${cmd[@]}"
        ;;
    first-swap)
        exec tools/mcpe_smoke.py \
            --display "$DISPLAY" \
            --cpu-backend "$CPU_BACKEND" \
            --out-dir "${OUT_DIR:-tmp/mcpe-first-swap}" \
            --expect-stage completed \
            --expect-exit zero \
            --min-gles-swaps 1 \
            --max-gl-errors 0 \
            --timeout "${TIMEOUT:-80}"
        ;;
    visible-smoke | smoke)
        exec tools/mcpe_smoke.py \
            --display "$DISPLAY" \
            --cpu-backend "$CPU_BACKEND" \
            --first-visible-draw \
            --out-dir "${OUT_DIR:-tmp/mcpe-visible}" \
            --expect-stage completed \
            --expect-exit zero \
            --min-gles-swaps 1 \
            --min-gles-draw-elements 1 \
            --min-readback-rgb 1 \
            --max-gl-errors 0 \
            --timeout "${TIMEOUT:-160}"
        ;;
    *)
        cat >&2 <<EOF
usage: $0 [live|first-swap|visible-smoke]

Environment overrides:
  APK=... ABI=... CPU_BACKEND=aemu|dynarmic DISPLAY=:0
  AEMU_FRAMES=N       Stop live mode after N SDL2 frames.
  WS_ADDR=host:port   Live WebSocket control endpoint.
  SKIP_BUILD=1        Reuse existing target/release/aemu.
EOF
        exit 2
        ;;
esac
