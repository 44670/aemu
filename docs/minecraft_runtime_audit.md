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
| Bounded first-frame probe | `target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 300000000 --until-swap` exits successfully after `native activity reached eglSwapBuffers at step 254627090` in the current FBO/resource bridge build. | Satisfied for local ARMv7 research APK |
| Host/WebGL drawing backend | `src/hle_imports.rs` records a bounded `GlesEvent` stream for shader/program, clear, viewport, draw, swap, buffer, texture, framebuffer/renderbuffer, stencil, uniform, vertex-attrib, client attribute payload, and common render-state calls; `src/sdl_shell.rs` replays the first MCPE frame into an SDL2 GLES2 context, submits all 744 captured indexed draws, reads back nonzero RGB/alpha pixels across the 854x480 drawable, and reports zero host GL errors. `src/wasm_webgl.rs` mirrors the SDL2 framebuffer/renderbuffer/state replay model for the browser target. | SDL2 first-frame visual replay satisfied; browser harness pending |
| SDL2 live frames | `NativeRuntime::continue_until_hle` can resume guest execution after a stopped HLE call. With `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1`, `run-apk-native --sdl2-live` reaches the first MCPE swap at step `254627090`, replays live frame batches to an SDL2 GLES window, and reports nonzero `854x480` readback with zero host GL errors. WebSocket screenshots such as `target/aemu-display0-fbo.ppm` verify `854x480`, `372` colors, `409920` nonblack pixels, and SHA256 `fb264d18933619212f2f9a7ea8502258655cfe29389a64372e4da000e2bb583d`. The visible output remains the static gradient/loading frame. | Live rendering satisfied; menu/resource progression still pending |
| SDL2 WebSocket harness and input | `src/ws_harness.rs` and `tools/ws_cli.py` provide `debug`, `screenshot`, `pointer`, and `tap`. A traced `tools/ws_cli.py --url ws://127.0.0.1:8768 tap 427 240 --duration-ms 180` reached `AInputQueue_getEvent`, `AMotionEvent_getX/Y`, and `_ZN10Multitouch4feedEccssi` for both down and up, with guest coordinates `427,240`. Before/after screenshots `target/aemu-inputqueue-before.ppm` and `target/aemu-inputqueue-after.ppm` were byte-identical, so input delivery is no longer the only blocker. | Harness/input bridge satisfied; not playable |
| Browser-fed APK bytes | `load_apk_native_libraries_bytes` links APK native libraries from bytes, and `HleRuntime::set_apk_bytes` lets Android asset HLE serve `AAssetManager_open` from the same byte source. | Initial browser data path |
| Browser MCPE entrypoint | `src/wasm_api.rs` exports `runMcpeFirstFrame(apkBytes, abi, canvasId, maxSteps)` for wasm builds. It runs the byte-backed APK path through constructors, `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, and `android_main` until `eglSwapBuffers`, then replays captured GLES events into a WebGL 1 canvas and returns draw/readback/error stats. `web/mcpe_first_frame.html` wires that export to a file input and canvas. | Initial browser harness path |
| Browser/WebGL target remains viable | `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl` passes. | Build-gate satisfied |
| SDL2 desktop target remains viable | `cargo check --features sdl2` passes. | Build-gate satisfied |
| Local Minecraft PE can run on ARMv7-A interpreter | Current local APK has `armeabi-v7a` libraries; default `run-apk-native` now selects that ABI and the SDL2/live probes reach the current loading-frame blocker. | In progress |

## Current Blocking Evidence

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
startup, shader reflection, resource readiness, reaching draw/swap calls,
submitting the first captured MCPE draw stream through SDL2, replaying
framebuffer/renderbuffer state, or producing nonblack first-swap host pixels.
The visible result is still the static gradient/loading frame, so the next
graphics blocker is either missing UI batch content/state before replay or a
shader/texture/uniform mismatch that leaves the drawn UI visually absent.

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
raising the bound to 65,536 events. In the current resource/FBO build, the local
0.15.0.1 APK reaches `eglSwapBuffers` at step `254627090` and reports 21,695
captured GLES events with 3,811,776 bytes of GLES payload data. The SDL2 replay
path submits all 744 captured indexed draws with zero skipped client attribute
or index draws. Framebuffer readback after replay reports
`854x480 nonzero_rgb_pixels=409920 nonzero_alpha_pixels=409920` and
`gl_errors count=0`.

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
- `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 2` exits 0 after reaching `eglSwapBuffers` at step `254627090`; frame 1 reports `events=21695 payload=3811776 draws arrays=0 elements=744 readback=854x480 rgb=409920 alpha=409920 gl_errors=0`, and frame 2 reports `events=276 payload=6776 draws arrays=0 elements=767 readback=854x480 rgb=409920 alpha=409920 gl_errors=0`.
- `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --ws 127.0.0.1:8773` reaches live rendering on the real X display. `tools/ws_cli.py --url ws://127.0.0.1:8773 screenshot --out target/aemu-display0-fbo.ppm` captures a `1229775` byte `854x480` PPM; `sha256sum target/aemu-display0-fbo.ppm` reports `fb264d18933619212f2f9a7ea8502258655cfe29389a64372e4da000e2bb583d`, and `magick identify` reports `372` colors. The screenshot is still byte-identical to the previous static gradient/loading frame.
- `target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 300000000 --until-swap --gles-summary --sdl2 --sdl2-hold-ms 250` exits 0 after reaching `eglSwapBuffers` at step `254925219`, summarizing 21,674 captured GLES events with 3,811,776 copied payload bytes, reporting `sdl2: submitted draws arrays=0 elements=744 skipped_client_attrib=0 skipped_missing_indices=0`, `sdl2: readback 854x480 nonzero_rgb_pixels=409920 nonzero_alpha_pixels=409920`, and `sdl2: gl_errors count=0`
- `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --ws 127.0.0.1:8766` reaches live rendering on the real X display. A run passed frame `2000` without the previous HLE heap exhaustion; every reported frame had `readback=854x480 rgb=409920 alpha=409920 gl_errors=0`. WebSocket screenshot capture verifies `854x480`, `372` colors, and `409920` nonblack pixels.
- `AEMU_TRACE_HLE=AInput,AMotion,Multitouch,MenuPointer DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --ws 127.0.0.1:8768` plus `tools/ws_cli.py --url ws://127.0.0.1:8768 tap 427 240 --duration-ms 180` shows the tap entering `AInputQueue_getEvent`, `AMotionEvent_getX/Y`, and `_ZN10Multitouch4feedEccssi` for both down and up. `target/aemu-inputqueue-before.ppm` and `target/aemu-inputqueue-after.ppm` are byte-identical, so MCPE remains on the gradient/loading frame.
- `cargo run --release -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --all`
  reports 578 reserved HLE symbols, 906 resolved imports, and zero unresolved imports

## Required Next Input

No older APK is required for the current CPU target. The active blocker is still
MCPE progression beyond the static gradient/loading frame under the local
`armeabi-v7a` APK.
