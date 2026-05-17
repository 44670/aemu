# Minecraft Runtime Completion Audit

Objective: make Minecraft PE APK run in the Rust Android HLE emulator.

## Success Criteria

- A Minecraft PE APK with a supported native ABI is available locally.
- The APK native libraries load into the 1:1 guest address map.
- Native dynamic imports resolve to APK-local exports or HLE system symbols.
- ARM relocations apply successfully.
- Native constructors run under the interpreter without unsupported
  instruction traps.
- The Android lifecycle/native entrypoint can be invoked.
- A bounded MCPE first-frame probe can stop successfully on the first present.
- EGL/GLES calls reach a host/WebGL-facing implementation instead of no-op
  placeholders.
- Input, assets, filesystem, and audio have enough HLE behavior for first
  frame execution.

## Prompt-To-Artifact Checklist

| Requirement | Current Evidence | Status |
| --- | --- | --- |
| Keep one Rust crate | `Cargo.toml` remains a single package; no workspace split. | Satisfied |
| Use Rust custom interpreter, not libdvm/QEMU/Dynarmic runtime | `src/armv7a.rs`; QEMU/Dynarmic remain references/oracles only. | Satisfied |
| Test APK path is `/mnt/hgfs/deb13/AndroidGames` | `AGENTS.md`, `docs/minecraft_pe_probe.md`, and CLI probes use that path. | Satisfied |
| 1:1 guest address map | `src/native_loader.rs`, `src/guest_memory.rs`, `AGENTS.md`. | Satisfied |
| APK native load/link | `cargo run -- link-apk ... --abi armeabi-v7a` reports loaded and relocated. | Satisfied for local ARMv7 research APK |
| System import HLE | `src/hle_imports.rs`; local MCPE probe resolves 906 imports with zero unresolved. GLES object-name generation writes texture/buffer/framebuffer/renderbuffer names back to guest memory, GLES shader/program HLE now reflects active uniforms and attributes from MCPE shader source, and the current target facades cover libstdc++ hash helpers, GLES precision/texture-parameter queries, profiler ticks, no-input/gamepad polling, transform interpolation, render-context texture unbind, and no-network social/auth ticks. | Initial coverage |
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address and linked runtime HLE entries such as `__dynamic_cast`; `run_function_with_args_until_hle` can now turn a selected HLE call into a bounded success condition. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native --abi armeabi-v7a --launch` completes all 1,604 constructors on the local APK. | Satisfied for local ARMv7 research APK |
| ARMv7/Thumb-2/NEON research probe | The release launch reaches `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, `android_main`, EGL setup, GL string queries, texture name generation, texture upload paths, `glViewport`, `glDepthRangef`, MCPE resource loading, `glDrawElements`, and `eglSwapBuffers` without an undefined NEON trap. | First-frame HLE coverage |
| Bounded first-frame probe | `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-uidiv-hle-20260517 --frames 1 --timeout 120` reaches `eglSwapBuffers` at step `311218123` and exits 0 at the frame limit after moving `__aeabi_uidiv/__aeabi_uidivmod` to Rust HLE traps. | Satisfied for local ARMv7 research APK |
| Host/WebGL drawing backend | `src/hle_imports.rs` records a bounded `GlesEvent` stream for shader/program, clear, viewport, draw, swap, buffer, texture, framebuffer/renderbuffer, stencil, uniform, vertex-attrib, client attribute payload, and common render-state calls; `src/sdl_shell.rs` and `src/wasm_webgl.rs` replay the captured stream for SDL2/WebGL. Earlier SDL2 evidence submitted 744 indexed draws with nonzero RGB readback, but the current `tools/mcpe_smoke.py` live baseline records zero indexed draws and black RGB readback. | Current no-draw regression/blocker |
| SDL2 live frames | `NativeRuntime::continue_until_hle` can resume guest execution after a stopped HLE call. With `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1`, current `run-apk-native --sdl2-live --steps 600000000` reaches first swap at step `311218123`, advances to frame limit, and reports zero host GL errors. Current 5-frame and 43-swap probes record no draw stream and black RGB readback; older heavy scheduling diagnostics reached real `DrawElements` later in the live loop but were too slow for a reliable smoke. | Live loop works; rendering progression remains blocked |
| SDL2 WebSocket harness and input | `src/ws_harness.rs` and `tools/ws_cli.py` provide `debug`, `screenshot`, `pointer`, and `tap`. `tools/mcpe_ui_smoke.py` now launches `run-apk-native --sdl2-live --ws`, waits for the WebSocket/debug frame milestone, runs a multi-step UI journal, captures PNG screenshots under `tmp/` by default, records `journal.jsonl`, and writes `summary.json` in one command. A traced UI smoke reaches `AInputQueue_getEvent` and `AMotionEvent_getX/Y` for tap down/up at guest coordinates `427,240`. `tmp/before.png` and `tmp/after.png` are byte-identical, so input delivery is no longer the only blocker. | Harness/input bridge satisfied; not playable |
| Browser-fed APK bytes | `load_apk_native_libraries_bytes` links APK native libraries from bytes, and `HleRuntime::set_apk_bytes` lets Android asset HLE serve `AAssetManager_open` from the same byte source. | Initial browser data path |
| Browser MCPE entrypoint | `src/wasm_api.rs` exports `runMcpeFirstFrame(apkBytes, abi, canvasId, maxSteps)` for wasm builds. It runs the byte-backed APK path through constructors, `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, and `android_main` until `eglSwapBuffers`, then replays captured GLES events into a WebGL 1 canvas and returns draw/readback/error stats. `web/mcpe_first_frame.html` wires that export to a file input and canvas. | Initial browser harness path |
| Browser/WebGL target remains viable | `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl` passes. | Build-gate satisfied |
| SDL2 desktop target remains viable | `cargo check --features sdl2` passes. | Build-gate satisfied |
| Local Minecraft PE can run on ARMv7-A interpreter | Current local APK has `armeabi-v7a` libraries; default `run-apk-native` now selects that ABI and the SDL2/live probes reach the current loading-frame blocker. | In progress |

## Current Blocking Evidence

Local profiler-first update on 2026-05-17:

- `tools/mcpe_smoke.py --profile-pc` now writes `pc_profile.jsonl` under each
  `tmp/mcpe-smoke-*` trace directory and summarizes samples in `summary.json`.
  `tools/trace_query.py <trace-dir> pc-profile` prints the latest snapshot,
  tolerating a truncated final JSONL row from timeout-killed runs.
- Baseline profiler run `tmp/mcpe-smoke-1778992046` timed out after 120s at
  `android_main`, recorded 54 GLES events, 0 swaps, 94,413 PC samples, and
  showed top samples in HLE-page `strlen`/`strcmp` helpers plus
  `libminecraftpe.so` `bn_mul_mont` and UI definition resolution.
- Moving `strlen`, `strcmp`, and `strncmp` from interpreted HLE-page ARM helper
  code to Rust HLE traps moved the top samples out of string helpers and raised
  same-window GLES event progress to 366 in `tmp/mcpe-smoke-1778992374`.
- Adding HLE coverage for defined 64-bit compiler helpers
  `__divdi3`, `__udivdi3`, `__moddi3`, and `__umoddi3` removed `__umoddi3`
  from the latest top profile, but did not move the 120s run beyond
  `android_main` or produce the first swap.
- Optimizing `MappedMemory` 16/32-bit loads and stores to avoid default
  byte-at-a-time trait access is correct and tested, but it did not produce a
  visible 120s MCPE progression change: `tmp/mcpe-smoke-1778993627` still
  records 366 GLES events, 0 swaps, and top samples in `bn_mul_mont`.
- The current evidence points at ARM interpreter throughput in native
  `bn_mul_mont` and MCPE UI/JSON parsing paths, not blind guest-thread slice
  tuning. The hot `bn_mul_mont` loop is ARM state around object offsets
  `0x012e0738..0x012e076c` and consists of `ldr`, `adds`, `umlal`, `adc`,
  `str`, `cmp`, and `bne`. A focused qemu-user oracle case now covers the
  observed `UMLAL + ADDS/ADC` pattern.

Follow-up profiler/scheduler evidence on 2026-05-17:

- `tools/trace_query.py <trace-dir> pc-profile` now records full Thumb-2
  instruction words for sampled Thumb PCs and classifies hot Thumb operations
  instead of reporting a single `thumb` bucket. `tmp/mcpe-profile-thumb-op2-20260517`
  still times out at `android_main` with 0 swaps under profiling overhead, but
  its top rows split UI/JSON work into `ldr`, `b`, `cmp`, `mov`, `add`, and
  `alu` buckets. HLE trap samples are now labeled `udf` instead of a misleading
  `ldrb`.
- `__aeabi_uidiv` and `__aeabi_uidivmod` now use Rust HLE traps instead of the
  old interpreted ARM division helper. In
  `tmp/mcpe-profile-uidiv-hle-20260517`, `__aeabi_uidivmod+...` no longer
  appears in the top PC profile, HLE-page samples fall to 510, and the top
  guest cost returns to `bn_mul_mont` plus native UI/JSON/resource code.
- Without PC profiling overhead,
  `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-uidiv-hle-20260517 --frames 1 --timeout 120`
  exits 0, reaches `eglSwapBuffers` at step `311218123`, records 375 GLES
  events and one swap, but records no draw calls and black RGB readback. The
  3-frame follow-up `tmp/mcpe-smoke-uidiv-hle-frames3-20260517` completes
  three swaps with zero draws, and the 200-frame timeout run
  `tmp/mcpe-smoke-uidiv-hle-frames200-20260517` reaches 43 swaps with zero
  `DrawArrays`/`DrawElements`; the GLES event stream contains texture
  uploads/binds and repeated swaps but no draw submissions.
- `tools/trace_query.py <trace-dir> thread-summary` now summarizes
  `AEMU_TRACE_THREADS` run logs. The default thread trace
  `tmp/mcpe-thread-trace-uidiv-hle-20260517` creates three runnable
  libgnustl wrapper threads, skips 40
  `libminecraftpe.so` `crossplat::threadpool::thread_start(void*)` threads,
  and records two wakes. A diagnostic `AEMU_RUN_ALL_PTHREADS=1` rerun
  (`tmp/mcpe-thread-all-uidiv-hle-20260517`) creates those 40 MCPE threadpool
  workers, but they all block on the same condvar and the smoke still records
  five blank swaps with zero draws. This rules out simply running all skipped
  pthreads as a sufficient fix and points to the need for more precise
  resource/lifecycle wait-wake evidence.

Local files rechecked on 2026-05-13:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1/lib/armeabi-v7a/libminecraftpe.so
```

The target `lib/armeabi-v7a/libminecraftpe.so` is present.

Default ARMv7-A runtime probe:

```sh
cargo run -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --steps 1000
```

Result:

```text
default ABI: armeabi-v7a
```

ARMv7/NEON runtime probe:

```sh
AEMU_TRACE_HLE=gl AEMU_TRACE_HLE_LIMIT=120 AEMU_TRACE_STEPS=200000000 timeout 600s cargo run --release -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 1200000000 --launch
```

Abbreviated result:

```text
constructors: 1604
native constructors completed
launch: libfmod.so JNI_OnLoad 0x700ccb68 java_vm 0x60047c24
launch: libminecraftpe.so JNI_OnLoad 0x7128d499 java_vm 0x60047c24
launch: nativeRegisterThis 0x7128d571 env 0x60047ba0
launch: ANativeActivity_onCreate 0x71294589 activity 0x60047c7c
launch: android_main 0x7128eef5 android_app 0x60048028
HLE ... name=eglGetDisplay ...
HLE ... name=eglInitialize ...
HLE ... name=eglCreateWindowSurface ...
HLE ... name=eglMakeCurrent ...
HLE ... name=glGetString ...
HLE ... name=glGenTextures ...
HLE ... name=glBindTexture ...
HLE ... name=glTexImage2D ...
HLE ... name=glTexSubImage2D ...
STEP ... step=200000000/1200000000 ...
STEP ... step=400000000/1200000000 ...
STEP ... step=600000000/1200000000 ...
STEP ... step=800000000/1200000000 ...
STEP ... step=1000000000/1200000000 ...
native run failed: android_main failed: step limit reached
```

The latest forced ARMv7 runs do not stop on undefined NEON opcodes. They now
pass the earlier `__dynamic_cast` stack crash through runtime C++ ABI HLE and
use a 32 MiB default guest stack below TLS. After adding GLES object-name
writes, `glGenTextures` feeds nonzero guest texture IDs into later
`glBindTexture` calls (`1`, `2`, `3`, ...), instead of leaving the guest on
texture `0`.

After adding libstdc++ hash helpers, GLES precision/texture-parameter query
facades, profiler/no-input/gamepad facades, no-network social/auth facades,
`mce::MathUtility::interpolateTransforms`, and
`mce::RenderContextOGL::unbindAllTextures`, a draw-focused launch probe reached
MCPE UI render setup and frame-loop bookkeeping:

```sh
AEMU_TRACE_HLE=Draw AEMU_TRACE_HLE_LIMIT=40 AEMU_TRACE_STEPS=200000000 timeout 900s cargo run --release -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 1600000000 --launch
```

It reached the 1.6B-step cap before any traced `glDraw*` or `eglSwapBuffers`
call. The final PC cluster mapped to `RunningAverage<double, 100>::append`,
`WorkerPool::processCoroutines(double)`, and
`std::chrono::_V2::system_clock::now()`.

The single-threaded HLE runtime now reserves
`WorkerPool::processCoroutines(double)` as a narrow target facade. Android
threads are already HLE no-ops, so bypassing this worker-pool drain removes the
previous background-callback bottleneck. With that facade, MCPE reaches repeated
clear/present work:

```sh
AEMU_TRACE_HLE=Swap AEMU_TRACE_HLE_LIMIT=20 AEMU_TRACE_STEPS=100000000 timeout 240s cargo run --release -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 800000000 --launch
```

This traces repeated `eglSwapInterval` and `eglSwapBuffers` calls beginning at
about 82.33M guest steps. A companion `AEMU_TRACE_HLE=glClear` probe reaches
repeated `glClearColor`, `glClearDepthf`, and `glClear` calls in the same
window.

The GLES shader facade now tracks `glCreateShader`, `glShaderSource`,
`glCreateProgram`, `glAttachShader`, and `glLinkProgram`, then reports active
uniforms/attributes through `glGetProgramiv`, `glGetActiveUniform`,
`glGetActiveAttrib`, `glGetUniformLocation`, and `glGetAttribLocation`.
Reflection parses MCPE-style GLSL aliases such as `MAT4`, `POS3`, and `POS4`,
takes the WebGL/GLES2 side of `#if __VERSION__ >= 300`, and filters
declared-but-unused uniforms so MCPE does not crash in
`mce::ShaderOGL::reflectShaderUniforms()` on optimized-out metadata.

After adding the MCPE resource bridge, `GameRenderer::render(float)` detects
the `MinecraftClient + 0x23e` resource-ready gate and calls the target
`MinecraftClient::onResourcesLoaded()` routine when needed. That native call now
executes many `glDrawElements` HLE calls, returns, and sets the ready byte to
`0x01`. The main Android loop then reaches repeated `eglSwapInterval` and
`eglSwapBuffers` calls. `run-apk-native --until-swap` turns this into a bounded
exit-0 first-frame probe instead of treating the endless game loop's step cap as
the only terminator.

The current blocker is no longer instruction decode, import resolution, EGL
startup, shader reflection, or basic swap-loop progress. Earlier resource/FBO
evidence reached a 744-indexed-draw first frame with nonblack readback, but the
current live baseline reaches `eglSwapBuffers` with zero indexed draws and
black RGB readback. The next graphics blocker is therefore earlier than shader
or texture visual correctness: determine why the current native path does not
produce the expected draw stream before replay.

The GLES HLE now records frame-relevant calls into a bounded `GlesEvent` queue:
shader/program creation and linking, active/bound textures, texture upload
parameters, buffer binding/upload, framebuffer/renderbuffer binding and
attachments, stencil/cull/polygon-offset state, shader program use, uniform
values, vertex attribute pointers/enables, blend/depth/color/scissor state,
clear/viewport, draws, flush, and swap. Buffer, texture, uniform, draw-index,
and client-side vertex attribute data now include copied guest payload bytes
when mapped and bounded. The SDL2 host replays the captured GLES2 stream into a
host GLES2 context, including shader compilation/linking, guest-to-host object
mapping, uniform location mapping, texture/buffer/framebuffer/renderbuffer
uploads, client-attribute staging VBOs, state calls, and indexed draw
submission.

The first-frame MCPE event capture no longer saturates the command queue after
raising the bound to 65,536 events. Historical resource/FBO traces captured
21,695 GLES events with 3,811,776 bytes of GLES payload data and replayed 744
indexed draws. The current `tools/mcpe_smoke.py` baseline reaches first swap at
step `365653113`, but frame 1 records only 2,804 live events and zero draw
submissions; framebuffer readback reports `854x480 rgb=0 alpha=409920` with
zero host GL errors.

## Latest Verification

- `cargo fmt --check`
- `cargo test dispatches_gles_shader_reflection_facade_outputs`
- `cargo test neon`
- `cargo test dispatches_gles_object_name_facade_outputs`
- `cargo test dispatches_gles_precision_and_texture_parameter_queries`
- `cargo test dispatches_libstdcxx_hash_bytes`
- `cargo test dispatches_profiler_facades`
- `cargo test dispatches_no_input_facades`
- `cargo test dispatches_no_gamepad_facades`
- `cargo test dispatches_worker_pool_coroutine_facade`
- `cargo test dispatches_minecraft_transform_interpolation`
- `cargo test dispatches_minecraft_ogl_unbind_all_textures`
- `cargo test dispatches_no_network_social_tick_facades`
- `cargo test` with 154 unit/integration-facing tests and 114 QEMU oracle tests
- `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl`
- `cargo check --features sdl2`
- `cargo build --release`
- `cargo build --release --features sdl2`
- `tools/mcpe_smoke.py --frames 2 --timeout 260 --expect-stage completed --expect-exit zero`
  exits 0, writes `tmp/mcpe-smoke-1778981853`, and reports stage
  `completed`, 161 GLES JSONL rows, zero native events, zero SDL draw PNGs,
  frame 1 `events=2804 payload=6290728 draws arrays=0 elements=0
  readback=854x480 rgb=0 alpha=409920 gl_errors=0`, and frame 2
  `events=33 payload=0 draws arrays=0 elements=0 readback=854x480 rgb=0
  alpha=409920 gl_errors=0`.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-swap-slices64-time0p1ms-f200
  --allow-existing-trace-dir --frames 200 --timeout 360
  --fake-time-step-nanos 100000 --native-trace-preset resource-done
  --expect-stage completed --expect-exit zero` exits 0 and records 200 swaps.
  The new frame-boundary worker service improves resource preload progress:
  845 work-load callbacks and 841 done callbacks are observed, and the resource
  counter drops from 858 to 18. It still records no final resource callback,
  no `MinecraftClient::onResourcesLoaded`, and zero SDL draw PNGs.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-swap-slices256-time0p1ms-f12
  --allow-existing-trace-dir --frames 12 --timeout 180
  --fake-time-step-nanos 100000 --guest-thread-swap-slices 256
  --expect-stage completed --expect-exit zero` exits 0 but is slow
  (`169.961s`) and still records no draw stream by frame 12.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-swap-slices256-time0p1ms-f260
  --allow-existing-trace-dir --frames 260 --timeout 480
  --fake-time-step-nanos 100000 --guest-thread-swap-slices 256` times out
  after 480 seconds at 59 swaps, but the partial run records 765
  `DrawElements` events and 10 SDL draw-change PNGs. This proves the native
  resource path can cross into real drawing under heavier guest-thread service,
  but the current fixed-slice strategy is not efficient enough for long UI
  smoke runs.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-swap-slices64-time0p1ms-f240
  --allow-existing-trace-dir --frames 240 --timeout 480
  --fake-time-step-nanos 100000 --expect-stage completed --expect-exit zero`
  times out before the requested frame limit, but the parsed artifacts record
  229 GLES swaps, 809 `DrawElements`, and 30 SDL draw-change PNGs. The first
  logged live frames are still black/no-draw through frame 180, and drawing
  begins later in the GLES stream. This confirms default 64-slice scheduling can
  eventually reach native rendering but is too slow for a reliable long smoke.
- `tools/mcpe_ui_smoke.py --out-dir tmp --trace-hle AInput,AMotion
  --trace-hle-limit 80 --min-gles-events 1 --expect-hle-call
  AInputQueue_getEvent --expect-hle-call AMotionEvent_getX --expect-hle-call
  AMotionEvent_getY --script 'debug; screenshot tmp/before.png; tap 427,240;
  wait 0.25; screenshot tmp/after.png; debug'` exits 0 and machine-gates that
  the tap enters Android input HLE. `tmp/before.png` and `tmp/after.png` are
  identical 854x480 PNGs with SHA256
  `c2376a3f584ac29966d02410bac23da4f888ea00096b42898bb8bcb3b02850b9`.
- `cargo run --release -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --all`
  reports 578 reserved HLE symbols, 906 resolved imports, and zero unresolved imports

One-command UI journal smoke:

```sh
tools/mcpe_ui_smoke.py --out-dir tmp
```

The default output root is `tmp/`; if `--out-dir` is omitted, the harness creates
`tmp/mcpe-ui-smoke-<timestamp>/` and places default screenshots directly in that
run directory. The command above keeps the two comparison screenshots directly
at `tmp/before.png` and `tmp/after.png`. The default journal waits for the first
debug frame, captures `before.png`, sends a center tap, waits briefly, captures
`after.png`, and records final debug stats. Use `--script` or `--script-file`
for longer menu/navigation flows. Use `--expect-screenshot-change` only for
future playable gates; the current known state still produces identical
before/after loading-frame screenshots after a tap. Use `--min-readback-rgb`
and `--min-draw-elements` when a scenario is expected to prove nonblack
rendering or indexed draw replay. Use `--trace-hle`, `--native-event`,
`--expect-native-event`, and `--dump-sdl-draws` to collect HLE/native/GLES/SDL
draw artifacts in the same scenario summary. Use `--expect-hle-call`,
`--expect-run-log-contains`, `--min-gles-events`, `--min-native-events`, and
`--min-sdl-draw-pngs` to turn those artifacts into explicit regression gates.

Latest result after rebuilding `target/release/aemu` with `--features sdl2`:
the command above reaches `eglSwapBuffers` at step `365653113`, opens the
WebSocket harness, runs six journal steps, writes `tmp/before.png` and
`tmp/after.png` as `854x480` PNG files, and exits cleanly with zero GL errors.
Both screenshots currently have SHA256
`c2376a3f584ac29966d02410bac23da4f888ea00096b42898bb8bcb3b02850b9`, so the
tap still produces no visible framebuffer change. A traced rerun with
`--trace-hle AInput,AMotion` records 988 GLES JSONL rows and HLE input calls
for both tap down/up in `tmp/run.log`;
`tmp/summary.json` reports zero native events and zero SDL draw PNGs for this
particular no-draw framebuffer state.

The input bridge is now machine-gated with:

```sh
tools/mcpe_ui_smoke.py --out-dir tmp --trace-hle AInput,AMotion \
  --trace-hle-limit 80 --min-gles-events 1 \
  --expect-hle-call AInputQueue_getEvent \
  --expect-hle-call AMotionEvent_getX \
  --expect-hle-call AMotionEvent_getY \
  --script 'debug; screenshot tmp/before.png; tap 427,240; wait 0.25; screenshot tmp/after.png; debug'
```

That run records 988 GLES JSONL rows and passes all HLE input expectations.

The default `tools/mcpe_smoke.py` harness now also passes an explicit
`--steps 600000000` to `run-apk-native`, because the current local first swap
is observed at step `365653113` and the old implicit 300M budget timed out in
`android_main`. Its stage parser treats `sdl2-live: reached frame limit` as
`completed`, so `--expect-stage completed --expect-exit zero` matches live
smoke success instead of only non-live `android_main` returns.
Verified command:

```sh
tools/mcpe_smoke.py --frames 2 --timeout 260 \
  --expect-stage completed --expect-exit zero
```

Latest trace directories:
`tmp/mcpe-smoke-swap-slices64-time0p1ms-f200`,
`tmp/mcpe-smoke-swap-slices256-time0p1ms-f12`, and
`tmp/mcpe-smoke-swap-slices256-time0p1ms-f260`; the latest 64-slice draw
evidence is `tmp/mcpe-smoke-swap-slices64-time0p1ms-f240`. The 200-frame
default/64-slice path still reports `draws arrays=0 elements=0` and
`readback=854x480 rgb=0 alpha=409920`; the 240-frame timeout reaches real
`DrawElements` in the GLES stream. The next blocker is a more efficient
guest-thread scheduling policy around resource preload and `eglSwapBuffers`,
not MCPE gameplay HLE.

`tools/mcpe_smoke.py` now parses live-frame stats and GLES event kind counts
into `summary.json` even when a run times out. New gates
`--min-gles-events`, `--min-gles-swaps`, `--min-gles-draw-elements`,
`--min-sdl-draw-pngs`, `--min-readback-rgb`, and `--max-gl-errors` let smoke
runs assert rendering progress directly.

## Required Next Input

No older APK is required for the current CPU target. The active blocker is still
restoring the expected MCPE draw stream and then progressing beyond the
unchanged input screenshots under the local `armeabi-v7a` APK.
