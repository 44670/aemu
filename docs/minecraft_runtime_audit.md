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
| Use Rust custom interpreter, not libdvm/QEMU/Dynarmic runtime | `src/armv6.rs`; QEMU/Dynarmic remain references/oracles only. | Satisfied |
| Test APK path is `/mnt/hgfs/deb13/AndroidGames` | `AGENTS.md`, `docs/minecraft_pe_probe.md`, and CLI probes use that path. | Satisfied |
| 1:1 guest address map | `src/native_loader.rs`, `src/guest_memory.rs`, `AGENTS.md`. | Satisfied |
| APK native load/link | `cargo run -- link-apk ... --abi armeabi-v7a` reports loaded and relocated. | Satisfied for local ARMv7 research APK |
| System import HLE | `src/hle_imports.rs`; local MCPE probe resolves 906 imports with zero unresolved. GLES object-name generation writes texture/buffer/framebuffer/renderbuffer names back to guest memory, GLES shader/program HLE now reflects active uniforms and attributes from MCPE shader source, and the current target facades cover libstdc++ hash helpers, GLES precision/texture-parameter queries, profiler ticks, no-input/gamepad polling, transform interpolation, render-context texture unbind, and no-network social/auth ticks. | Initial coverage |
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address and linked runtime HLE entries such as `__dynamic_cast`; `run_function_with_args_until_hle` can now turn a selected HLE call into a bounded success condition. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native --abi armeabi-v7a --launch` completes all 1,604 constructors on the local APK. | Satisfied for local ARMv7 research APK |
| ARMv7/Thumb-2/NEON research probe | The release launch reaches `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, `android_main`, EGL setup, GL string queries, texture name generation, texture upload paths, `glViewport`, `glDepthRangef`, MCPE resource loading, `glDrawElements`, and `eglSwapBuffers` without an undefined NEON trap. | First-frame HLE coverage |
| Bounded first-frame probe | `target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 300000000 --until-swap` exits successfully after `native activity reached eglSwapBuffers at step 254925219`. | Satisfied for local ARMv7 research APK |
| Host/WebGL drawing backend | `src/hle_imports.rs` records a bounded `GlesEvent` stream for shader/program, clear, viewport, draw, swap, buffer, texture, uniform, vertex-attrib, client attribute payload, and common render-state calls; `src/sdl_shell.rs` replays the first MCPE frame into an SDL2 GLES2 context, submits all 744 captured indexed draws, reads back nonzero RGB/alpha pixels across the 854x480 drawable, and reports zero host GL errors. `src/wasm_webgl.rs` now has a wasm-only WebGL host that mirrors the SDL2 replay state model and compiles for the browser target. | SDL2 first-frame visual replay satisfied; browser harness pending |
| Browser-fed APK bytes | `load_apk_native_libraries_bytes` links APK native libraries from bytes, and `HleRuntime::set_apk_bytes` lets Android asset HLE serve `AAssetManager_open` from the same byte source. | Initial browser data path |
| Browser MCPE entrypoint | `src/wasm_api.rs` exports `runMcpeFirstFrame(apkBytes, abi, canvasId, maxSteps)` for wasm builds. It runs the byte-backed APK path through constructors, `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, and `android_main` until `eglSwapBuffers`, then replays captured GLES events into a WebGL 1 canvas and returns draw/readback/error stats. `web/mcpe_first_frame.html` wires that export to a file input and canvas. | Initial browser harness path |
| Browser/WebGL target remains viable | `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl` passes. | Build-gate satisfied |
| SDL2 desktop target remains viable | `cargo check --features sdl2` passes. | Build-gate satisfied |
| Local Minecraft PE can run on ARMv6 interpreter | Current local APK has only `armeabi-v7a`; default `run-apk-native` fails with missing `armeabi`. | Blocked for ARMv6 |

## Current Blocking Evidence

Local files rechecked on 2026-05-13:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1/lib/armeabi-v7a/libminecraftpe.so
```

No `lib/armeabi/libminecraftpe.so` is present.

Default ARMv6 runtime probe:

```sh
cargo run -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --steps 1000
```

Result:

```text
native run failed: link failed: no native libraries found for ABI armeabi; available ABIs: armeabi-v7a
```

Forced ARMv7/NEON research probe:

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
submitting the first captured MCPE draw stream through SDL2, or producing
nonblack first-swap host pixels. The remaining graphics gap is wiring the
wasm-only WebGL replay host into a browser harness and validating browser
readback against the same first-swap stream.

The GLES HLE now records frame-relevant calls into a bounded `GlesEvent` queue:
shader/program creation and linking, active/bound textures, texture upload
parameters, buffer binding/upload, shader program use, uniform values, vertex
attribute pointers/enables, blend/depth/color/scissor state, clear/viewport,
draws, flush, and swap. Buffer, texture, uniform, draw-index, and client-side
vertex attribute data now include copied guest payload bytes when mapped and
bounded. The SDL2 host replays the captured GLES2 stream into a host GLES2
context, including shader compilation/linking, guest-to-host object mapping,
uniform location mapping, texture/buffer uploads, client-attribute staging VBOs,
state calls, and indexed draw submission.

The first-frame MCPE event capture no longer saturates the command queue after
raising the bound to 65,536 events. With `--gles-summary`, the local 0.15.0.1
APK reaches `eglSwapBuffers` at step `254925219` and reports 21,674 captured
GLES events: 157 `CreateProgram`, 144 `ShaderSource`, 157 `LinkProgram`, 744
`DrawElements`, 841 `TexImage2D`, 839 `TexSubImage2D`, 1,496
`VertexAttribPointer`, 719 `Uniform1i`, 752 uniform-vector updates, and one
swap. The same probe records 3,811,776 bytes of GLES payload data. The SDL2
replay path submits all 744 captured indexed draws with zero skipped client
attribute or index draws. Framebuffer readback after replay reports
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
- `target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 300000000 --until-swap --gles-summary --sdl2 --sdl2-hold-ms 250` exits 0 after reaching `eglSwapBuffers` at step `254925219`, summarizing 21,674 captured GLES events with 3,811,776 copied payload bytes, reporting `sdl2: submitted draws arrays=0 elements=744 skipped_client_attrib=0 skipped_missing_indices=0`, `sdl2: readback 854x480 nonzero_rgb_pixels=409920 nonzero_alpha_pixels=409920`, and `sdl2: gl_errors count=0`
- `cargo run --release -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --all`
  reports 578 reserved HLE symbols, 906 resolved imports, and zero unresolved imports

## Required Next Input

Provide an older Minecraft PE APK or extracted native library containing:

```text
lib/armeabi/libminecraftpe.so
```

The local `armeabi-v7a` APK remains useful as the active ARMv7/Thumb-2/NEON
research target.
