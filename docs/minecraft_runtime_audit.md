# Minecraft Runtime Completion Audit

> Current-state note (2026-07-16): sections below preserve investigation
> chronology and include obsolete target-HLE experiments. Production no longer
> registers or dispatches MCPE game/engine methods, no resource bridge calls
> `onResourcesLoaded`, and no thread allowlist skips guest pthread routines.
> Desktop and WebAssembly launchers now continue the actual pthread created by
> native-app-glue; they neither call `android_main` directly nor allocate a
> fallback `android_app`. That does not close native lifecycle integrity: both
> launchers directly call target JNI entrypoints, overwrite the guest-created
> `android_app`, and inject lifecycle poll sources instead of traversing the
> framework callbacks and app-glue command pipe. See `docs/no_fake_hle.md` for
> the three-way status: native symbol dispatch is clean, native lifecycle is
> not, and whole-APK execution remains open because APK-defined Java code is
> lifted in Rust instead of executed as DEX. The current interpreter run
> completes 300 frames with 5,979 indexed draws and zero replay GL errors, and
> the automated interaction gate creates and enters a flat world, remains in
> gameplay for 36 seconds, then verifies movement, camera control, block
> placement, and block breaking with zero replay GL errors. The
> investigation chronology below is retained as history; its former blockers
> are not the current runtime state.

Objective: make Minecraft PE APK run in the Rust Android HLE emulator.

## Success Criteria

- A Minecraft PE APK with a supported native ABI is available locally.
- The APK native libraries load into the 1:1 guest address map.
- Native dynamic imports resolve to APK-local exports or HLE system symbols.
- ARM relocations apply successfully.
- Native constructors run under the interpreter without unsupported
  instruction traps.
- The Android lifecycle reaches the native entrypoint through APK Java and the
  framework callback chain without launcher-injected app state or commands.
- A bounded MCPE first-frame probe can stop successfully on the first present.
- EGL/GLES calls reach a host/WebGL-facing implementation instead of no-op
  placeholders.
- Input, assets, filesystem, and audio have enough HLE behavior for first
  frame execution.

## Prompt-To-Artifact Checklist

| Requirement | Current Evidence | Status |
| --- | --- | --- |
| Keep one Rust crate | `Cargo.toml` remains a single package; no workspace split. | Satisfied |
| Use Rust custom interpreter as the default runtime | `src/armv7a.rs` remains the default CPU backend; QEMU remains an oracle; Dynarmic is gated as an optional native-only performance experiment documented in `docs/dynarmic_backend_eval.md`. | Satisfied |
| Test APK path is `/mnt/hgfs/deb13/AndroidGames` | `AGENTS.md`, `docs/minecraft_pe_probe.md`, and CLI probes use that path. | Satisfied |
| 1:1 guest address map | `src/native_loader.rs`, `src/guest_memory.rs`, `AGENTS.md`. | Satisfied |
| APK native load/link | `cargo run -- link-apk ... --abi armeabi-v7a` reports loaded and relocated. | Satisfied for local ARMv7 research APK |
| System import HLE | `src/hle_imports.rs`; the local MCPE link resolves 906 imports with zero unresolved. Platform HLE is stateful or fails explicitly; local UDP supports only bounded loopback/self-test traffic, external networking remains unavailable, JNI declarations come from the APK DEX index, and APK-defined native game symbols remain native. APK-defined Java bridge lifts are listed separately in `docs/no_fake_hle.md`. | Native symbol-dispatch gate satisfied; lifecycle and whole-APK gates open |
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address and linked runtime HLE entries such as `__dynamic_cast`; `run_function_with_args_until_hle` can now turn a selected HLE call into a bounded success condition. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native --abi armeabi-v7a --launch` completes all 1,604 constructors on the local APK. | Satisfied for local ARMv7 research APK |
| ARMv7/Thumb-2/NEON research probe | The release launch reaches `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, `android_main`, EGL setup, GL string queries, texture name generation, texture upload paths, `glViewport`, `glDepthRangef`, MCPE resource loading, `glDrawElements`, and `eglSwapBuffers` without an undefined NEON trap. | First-frame HLE coverage |
| Bounded first-frame probe | `tools/mcpe_smoke.py --trace-dir tmp/mcpe-smoke-uidiv-hle-20260517 --frames 1 --timeout 120` reaches `eglSwapBuffers` at step `311218123` and exits 0 at the frame limit after moving `__aeabi_uidiv/__aeabi_uidivmod` to Rust HLE traps. | Satisfied for local ARMv7 research APK |
| Host/WebGL drawing backend | `src/hle_imports.rs` records checked guest GLES state and bounded replay events; `src/sdl_shell.rs` and `src/wasm_webgl.rs` replay them for SDL2/WebGL. `tmp/mcpe-strict-jni-20260716-final5` reaches 300 swaps, 5,979 indexed draws, a 406,452-pixel RGB readback, zero skipped client-attribute draws, and zero replay GL errors. | SDL2 gate satisfied; WebGL build gate satisfied |
| SDL2 live frames | The release AEMU interpreter reaches first swap at guest step `310629057` and completes 300 frames in 31.401 seconds on the real native-app-glue pthread after host lifecycle injection. | Rendering/performance evidence only |
| SDL2 WebSocket harness and input | `tools/mcpe_ui_smoke.py --preset playable-flat-world` drives the real input queue through Xbox dismissal, Play, Create New World, Advanced, Flat, loading, movement, camera rotation/pitch, stone placement, and long-press breaking. `tmp/mcpe-playable-flat-world-gate-20260716` machine-checks world frames at 6, 21, and 36 seconds plus all four interactions; the final sample has 1,924 frames, 105,475 indexed draws, and zero replay GL errors. | New-world entry and bounded playability gate satisfied |
| Browser-fed APK bytes | `load_apk_native_libraries_bytes` links APK native libraries from bytes, and `HleRuntime::set_apk_bytes` lets Android asset HLE serve `AAssetManager_open` from the same byte source. | Initial browser data path |
| Browser MCPE entrypoint | `src/wasm_api.rs` exports `runMcpeFirstFrame(apkBytes, abi, canvasId, maxSteps)` for wasm builds. It uses the same direct `JNI_OnLoad`/`nativeRegisterThis` calls and synthetic lifecycle setup as desktop before continuing the real pthread to `eglSwapBuffers`, then replays GLES events into WebGL 1. | Initial browser harness; lifecycle nonconforming |
| Browser/WebGL target remains viable | `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl` passes. | Build-gate satisfied |
| SDL2 desktop target remains viable | `cargo check --features sdl2` passes. | Build-gate satisfied |
| Local Minecraft PE can run on ARMv7-A interpreter | The custom interpreter creates and enters a rendered flat world, remains in gameplay through the 36-second stability frame, moves, changes view, places a block, and breaks it without a runtime or rendering failure. Native game/engine symbols execute in the guest, but activity launch/lifecycle is synthesized and selected APK Java methods remain Rust lifts. | Bounded playability smoke satisfied; lifecycle and DEX incomplete |

## Historical Blocking Evidence

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
- `AEMU_TRACE_THREADS` now records condition-variable wait calls and
  signal/broadcast calls, including the condvar address and the number of
  waiters present before each signal. The default
  `tmp/mcpe-thread-sync-20260517` run still completes five blank swaps with
  411 GLES events, 5 swaps, and 0 draws. It skips the 40 MCPE threadpool
  workers, observes three `pthread_cond_wait` calls, and records two real
  wakes; most condition signals have `waiters_before=0`.
- The `AEMU_RUN_ALL_PTHREADS=1` comparison
  `tmp/mcpe-thread-sync-all-20260517` creates all 40 MCPE threadpool workers,
  but each worker runs once and blocks on
  `pthread_cond_wait(cond=0x606cba8c, mutex=0x606cba88)`. No signal or
  broadcast to `0x606cba8c` occurs during the five-frame run. The GLES stream
  is only `BindTexture:162`, `TexImage2D:122`, `TexSubImage2D:122`, and
  `SwapBuffers:5`; there are no `UseProgram`, `Clear`, `DrawArrays`, or
  `DrawElements` events. This makes blind cooperative thread-slice tuning the
  wrong next step: the immediate blocker is that native render/resource logic
  reaches texture uploads and swaps but not draw submission or threadpool work
  dispatch.
- A lower-overhead PC profile,
  `tools/mcpe_smoke.py --trace-dir tmp/mcpe-profile-low-20260517 --frames 5 --timeout 150 --profile-pc --profile-pc-interval 65536`,
  still reaches the same five blank swaps and captures 5,734 samples. The
  sampled hot paths are `bn_mul_mont`, `UIDefRepository::_resolveReferences`,
  `Json::Reader`, `ImageUtils::savePNG`, and, on worker thread 7,
  `Localization::_appendTranslations`. This supports investigating native
  resource/UI preload progress and draw-gating state before changing scheduler
  policy.
- `tmp/mcpe-resource-done-20260517`, captured with
  `--native-trace-preset resource-done`, records 197
  `ResourcePackManager::preloadTextures.work-load-texture.entry` events and
  five `GameRenderer::render.resource-ready-gate` events, but no preload
  done-callback events and no `MinecraftClient::onResourcesLoaded`. The gate's
  client byte window stays `0000000001000000e886046000000000`, so the
  resource-ready byte at `client+0x23e` is still zero when the blank swaps are
  emitted. The immediate no-draw state is therefore an unfinished native
  resource preload path, not a proven missed MCPE threadpool wake.
- `tools/trace_query.py <trace-dir> resource-progress` now summarizes this
  preload gate directly from `native_events.jsonl` and `gles_events.jsonl`.
  On the historical
  `tmp/mcpe-smoke-swap-slices64-time0p1ms-f200` trace it reports 845 preload
  work events, 841 done-callback entries, 200 resource gate hits, and still no
  final callback or draw; the done counter falls from 858 to 18 but never
  reaches zero. On the draw-reaching historical traces that did not enable
  native resource events, it still reports GLES draw progress
  (`DrawElements:765` at 59 swaps for
  `tmp/mcpe-smoke-swap-slices256-time0p1ms-f260` and `DrawElements:809` at
  229 swaps for `tmp/mcpe-smoke-swap-slices64-time0p1ms-f240`).
- SDL2 live runs can now stop on a draw milestone with
  `--sdl2-stop-after-draw-elements N`, exposed in `tools/mcpe_smoke.py` as
  `--stop-after-gles-draw-elements N`. This is intended for efficient UI
  smoke scripts that set a high frame cap but exit as soon as the first draw
  milestone appears, instead of running until a fixed frame limit or timeout.
  `tmp/mcpe-stop-after-draw-540-20260517` validates the path with
  `--fake-time-step-nanos 100000 --guest-thread-swap-slices 256 --frames 260
  --stop-after-gles-draw-elements 1`: it exits 0 after 533.409s at frame 61,
  records `DrawElements:721`, and stops via
  `sdl2-live: reached draw-elements limit` rather than timing out.
  Future `mcpe_smoke.py` runs with this stop condition also pass
  `--sdl2-stop-screenshot <trace-dir>/stop.png`, so the draw milestone leaves
  a PNG framebuffer artifact under `./tmp/`.
- `tmp/mcpe-stop-after-draw-shot-20260517` validates that screenshot path and
  the visible-readback gate. The run exits 0 after 501.771s with
  `--fake-time-step-nanos 100000 --guest-thread-swap-slices 256 --frames 260
  --stop-after-gles-draw-elements 1`. It reaches the draw stop at frame 61
  with 721 `DrawElements`, first draw event index `14276`, stop readback
  `854x480 rgb=35348 alpha=409920 gl_errors=0`, and writes
  `tmp/mcpe-stop-after-draw-shot-20260517/stop.png` as an 854x480 RGB PNG
  (63,017 bytes). `tools/mcpe_smoke.py` now lets future milestone scripts gate
  this directly with `--min-readback-rgb`, `--require-stop-screenshot`, and
  `--min-stop-screenshot-bytes`.
- `tmp/mcpe-stop-after-draw-resource-20260517` repeats the same draw-stop gate
  with `--native-trace-preset resource-done` and the stricter visible artifact
  expectations. It exits 0 after 506.497s, writes the same 854x480 RGB
  `stop.png`, and records 3,501 native resource events. At the visible draw
  stop the preload path has completed all 859 work callbacks and all 859 done
  callbacks, the done counter reaches zero, and the trace records
  `ResourcePackManager::preloadTextures.done-callback.final-callback-call`,
  `MinecraftClient::onResourcesLoaded.entry`, and
  `MinecraftClient::onResourcesLoaded.store-23e` before the GLES stream's
  first draw at event index `14276`. This narrows the next scheduler work to
  making native resource callback draining deterministic and efficient; it is
  not evidence for broad MCPE game-method HLE.
- The same validated command shape is now available as
  `tools/mcpe_smoke.py --first-visible-draw` or
  `tools/mcpe_smoke.py --first-visible-draw-resource`. These presets apply the
  known-good fake time step, guest-thread swap slices, draw-stop condition,
  visible readback gate, stop screenshot gate, zero-GL-error gate, and, for
  the resource variant, the `resource-done` native trace preset. They are
  command-line harness presets only; they do not change runtime semantics.
- `tmp/mcpe-first-visible-draw-profile-20260517` runs the validated
  `--first-visible-draw` milestone with low-overhead PC profiling at interval
  65,536. It exits 0 after 503.785s, reaches the same frame-61 visible draw,
  and records 8,281 PC samples over 542,751,356 guest instructions. The
  function-level profile is dominated by thread 7
  `Localization::_appendTranslations` (1,009 top-row samples), followed by
  main-thread `bn_mul_mont` (340), `UIDefRepository::_resolveReferences`
  (182), `operator new(unsigned int)` (88), thread 8
  `stbi_zlib_decode_malloc_guesssize` (53), and `Json::Reader::readToken`
  (45). This supports checking guest worker/resource text and string
  allocation behavior before changing scheduler policy.
- `tools/trace_query.py <trace-dir> pc-profile --symbols` now aggregates the
  latest PC profile snapshot by `(library, symbol)` so hot functions can be
  compared directly instead of reading individual PC rows.
- `tools/mcpe_smoke.py` also has narrow `localization` and `localization-hot`
  native trace presets for the new top profile function. The entry-only
  `tmp/mcpe-localization-entry-20260517` run exits 0 after 112.837s for one
  swap and captures five `Localization::_appendTranslations.entry` calls with
  C++ string arguments: `loc/en_US-pocket.lang`, `loc/en_GB-pocket.lang`,
  `loc/de_DE-pocket.lang`, `loc/es_MX-pocket.lang`, and
  `loc/ja_JP-pocket.lang`. The earlier hot-loop run
  `tmp/mcpe-localization-trace-20260517` confirms the profile PCs but fills an
  800-event limit quickly, so the entry-only preset is the better default for
  low-noise resource-text diagnosis.
- The QEMU-user oracle suite now includes
  `thumb2_localization_it_highreg_loop`, a focused Thumb-2 regression extracted
  from the localization profile shape. It validates high-register `cmp.w`,
  `ITT NE`, predicated `ldr.w`/`add.w`, and a backward branch against
  qemu-user before those instructions are treated as safe under MCPE's
  resource-text hot path.
- `HleRuntime` now caches parsed APK ZIP entries when APK bytes are already
  resident, so repeated `AAssetManager_open` and texture/image asset reads do
  not reparse the central directory. The control run
  `tmp/mcpe-first-visible-draw-apk-cache-20260517` still takes 501.090s to
  reach the same 61-swap, 721-`DrawElements`, visible screenshot milestone.
  This confirms central-directory reparsing was worth cleaning up but is not
  the primary first-visible-draw bottleneck.

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
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-stop-after-draw-shot-20260517
  --frames 260 --timeout 560 --fake-time-step-nanos 100000
  --guest-thread-swap-slices 256 --stop-after-gles-draw-elements 1
  --min-gles-draw-elements 1 --expect-stage completed --expect-exit zero`
  exits 0, records 61 swaps, 721 `DrawElements`, stop readback
  `rgb=35348`, and writes `stop.png` as an 854x480 RGB PNG.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-stop-after-draw-resource-20260517
  --frames 260 --timeout 640 --fake-time-step-nanos 100000
  --guest-thread-swap-slices 256 --native-trace-preset resource-done
  --stop-after-gles-draw-elements 1 --min-gles-draw-elements 1
  --min-readback-rgb 1 --require-stop-screenshot
  --min-stop-screenshot-bytes 1000 --max-gl-errors 0
  --expect-stage completed --expect-exit zero` exits 0, records 61 swaps,
  721 `DrawElements`, 3,501 native resource events, all 859 preload
  callbacks, final resource callback, `onResourcesLoaded.store-23e`, and a
  visible stop screenshot.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-first-visible-draw-profile-20260517
  --first-visible-draw --profile-pc --profile-pc-interval 65536
  --profile-pc-flush-interval 256 --profile-pc-top 80` exits 0, records 61
  swaps, 721 `DrawElements`, a visible stop screenshot, and 8,281 PC samples.
- `tools/trace_query.py tmp/mcpe-first-visible-draw-profile-20260517 pc-profile
  --symbols --limit 12` reports `Localization::_appendTranslations` as the
  top aggregated symbol, then `bn_mul_mont` and
  `UIDefRepository::_resolveReferences`.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-localization-entry-20260517
  --frames 1 --timeout 180 --fake-time-step-nanos 100000
  --guest-thread-swap-slices 256 --native-trace-preset localization
  --expect-stage completed --expect-exit zero --expect-native-event
  Localization::_appendTranslations.entry` exits 0 and captures the first five
  localization language files without filling the native event limit.
- `tools/mcpe_smoke.py --trace-dir tmp/mcpe-first-visible-draw-apk-cache-20260517
  --first-visible-draw` exits 0 after 501.090s with 61 swaps, 721
  `DrawElements`, and the same visible stop screenshot, so APK ZIP entry
  caching does not materially move the first-visible-draw milestone.
- `cargo test armv7a_qemu_user_oracle_cases_match_aemu` passes with the new
  `thumb2_localization_it_highreg_loop` case.
- `tools/mcpe_ui_smoke.py --out-dir tmp --trace-hle AInput,AMotion
  --trace-hle-limit 80 --min-gles-events 1 --expect-hle-call
  AInputQueue_getEvent --expect-hle-call AMotionEvent_getX --expect-hle-call
  AMotionEvent_getY --script 'debug; screenshot tmp/before.png; tap 427,240;
  wait 0.25; screenshot tmp/after.png; debug'` exits 0 and machine-gates that
  the tap enters Android input HLE. `tmp/before.png` and `tmp/after.png` are
  identical 854x480 PNGs with SHA256
  `c2376a3f584ac29966d02410bac23da4f888ea00096b42898bb8bcb3b02850b9`.
- `tools/mcpe_ui_smoke.py --out-dir tmp/mcpe-ui-not-now-timeout-20260517
  --first-visible-draw --trace-hle AInput,AMotion --trace-hle-limit 80
  --expect-hle-call AInputQueue_getEvent --expect-hle-call AMotionEvent_getX
  --expect-hle-call AMotionEvent_getY --expect-screenshot-change --script
  'debug; screenshot {trace_dir}/before.png; tap 280,386; wait 1.0;
  screenshot {trace_dir}/after.png; debug'` exits 0 after 560.045s. The
  wait gate reaches frame 61 with 721 `DrawElements` and readback RGB 35348,
  then the tap reaches Android input HLE as down/up events at guest
  coordinates `280,386`. The before screenshot shows the Xbox Live sign-in
  dialog; the after screenshot shows the main menu with `Play`, `Achievements`,
  `Options`, and `Store`. This proves start-screen button interaction after a
  visible draw gate, not full playability.
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
for longer menu/navigation flows.

For UI actions that must not run during the blank/loading frame, use
`--first-visible-draw` or explicit `--wait-draw-elements` and
`--wait-readback-rgb` gates. The first-visible preset also raises the
WebSocket request timeout because MCPE can spend more than 10 seconds inside a
single guest execution segment near resource completion; `ws_cli.py` now sends
that timeout as `timeout_seconds`, and `src/ws_harness.rs` honors it per
request. Use `--expect-screenshot-change` only when the script should cause a
real visible state transition. Use `--trace-hle`, `--native-event`,
`--expect-native-event`, and `--dump-sdl-draws` to collect HLE/native/GLES/SDL
draw artifacts in the same scenario summary. Use `--expect-hle-call`,
`--expect-run-log-contains`, `--min-gles-events`, `--min-native-events`, and
`--min-sdl-draw-pngs` to turn those artifacts into explicit regression gates.

Latest visible-UI result after rebuilding `target/release/aemu` with
`--features sdl2`: `tmp/mcpe-ui-not-now-timeout-20260517` reaches
`eglSwapBuffers` at step `311151082`, waits through 60 black/readback-zero
frames, reaches the first visible gate at frame 61, captures a before PNG of
the Xbox Live dialog, taps `Not Now`, captures an after PNG of the main menu,
and exits with zero expectation errors. The final debug step reports
`frames=67`, `draw_elements=878`, `readback_nonzero_rgb_pixels=409920`, and
zero GL errors. This is a start-screen interaction milestone; it does not yet
prove menu navigation into gameplay/world interaction.

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
