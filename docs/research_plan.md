# Research Plan

This is the working plan for the Android 4.x HLE emulator target: old Android
games, ARMv7-A/`armeabi-v7a` native code, SDL2 for native debugging, and
wasm/WebGL for browser execution.

## Plan First

1. Keep the project as one Rust crate with narrow modules instead of a
   multi-crate workspace.
2. Treat the APK as an input container: inspect ZIP metadata, parse the
   manifest/resources/assets as needed, extract or stream files, and load
   `lib/armeabi-v7a/*.so` through our own ELF/linker path.
3. Run native game code with the custom Rust ARM interpreter by default. Use
   QEMU user-mode as an offline instruction oracle and Dynarmic as a
   decoder/semantics reference. A native-only Dynarmic backend may be kept as
   an explicit performance experiment, but it must not replace the default
   interpreter or the wasm/browser CPU path.
4. HLE Android and Bionic APIs on demand: libc, pthreads, time, file paths,
   APK assets, JNI/native-activity entrypoints, EGL, GLES, input, audio, and
   save data.
5. Use GLES as the guest-facing graphics API. Translate GLES 2.0 to WebGL 1
   where possible, emulate GLES 1.1 fixed-function over shaders, and add WebGL
   2 only for a concrete target-game requirement.
6. Use SDL2 as the native debug shell. For wasm, prefer the path that keeps
   SDL2 input/audio/windowing and WebGL context ownership straightforward.
7. Make the local Minecraft PE `armeabi-v7a` APK the first target-driven
   validation case.

## Research Decisions

### Dalvik and `libdvm`

Running the Dalvik interpreter directly is not enough to run an APK. Dalvik
executes DEX bytecode; Android app execution also depends on package install
state, resources, class loading, JNI, native libraries, lifecycle objects,
window/input/audio services, Binder-facing framework behavior, and Bionic
process state.

`libdvm` from Android 4.4 is still valuable as a source reference. Reusing it
as this project's runtime is the wrong default because it is C/C++, tied to
AOSP build assumptions and Android system libraries, includes JIT/interpreter
configuration paths, and does not solve ARM native `.so` execution in the
browser. The runtime should HLE the specific Java/JNI surface demanded by the
game instead.

### APK Install and Extraction

An APK is a ZIP-format package. Android install does not simply run the APK in
place; it records package metadata, validates/signs, exposes resources/assets,
and makes native libraries available to the app. On old Android releases,
compressed native libraries are normally extracted to an app library directory.
Modern Android can avoid extraction for page-aligned uncompressed libraries
when `android:extractNativeLibs` is false, but that optimization should not be
assumed for Android 4.x-era targets.

ZIP metadata stores the compression method and compressed/uncompressed sizes.
It does not reliably preserve the original deflate compression level. The local
Minecraft PE 0.15.0.1 APK stores its native libraries as deflated entries, with
`libminecraftpe.so` compressed from 23,554,092 bytes to 8,171,610 bytes.

### ARMv7-A Interpreter

Use the ARM architecture manuals as semantic authority and use Dynarmic/QEMU to
cross-check decoder organization and behavior. The interpreter should implement
ARMv7-A user-mode ARM, Thumb-1, Thumb-2, interworking, VFPv3, and target-driven
NEON coverage for the current `armeabi-v7a` Minecraft PE probe, while keeping
older user-mode instructions that remain valid on ARMv7-A.
Privileged instructions should trap or become explicit user-mode stubs instead
of silently pretending to emulate kernel behavior.

### Optional Dynarmic Native Backend

Dynarmic is being evaluated as an optional native-only SDL2 CPU backend selected
with `--cpu-backend dynarmic` and the `dynarmic` Cargo feature. It is not part
of the wasm/WebGL runtime and does not bypass AEMU guest memory, Android HLE,
EGL/GLES capture, SDL2/WebGL replay, or cooperative guest-thread scheduling.
Current evaluation evidence lives in `docs/dynarmic_backend_eval.md`.

### Graphics and WebGL

WebGL 1 is the right first browser backend for Minecraft PE-style GLES 2.0
imports because WebGL 1 follows the OpenGL ES 2.0 shader pipeline model. WebGL
2 corresponds to OpenGL ES 3.0-style functionality and should be optional until
a target APK or trace needs it.

The local Minecraft PE 0.15.0.1 library imports GLES 2.0-style APIs such as
shader/program, attribute/uniform, VBO/FBO, texture, draw, viewport, scissor,
blend, and depth functions. It does not directly import common GLES 1.1
fixed-function names. The local APK is ARMv7/Thumb-2/VFPv3/NEON and is the
current CPU/HLE validation target.

ANGLE can help native desktop testing when a consistent GLES implementation is
useful, but it does not replace the browser WebGL backend.

## Milestones

1. Probe APKs and native libraries.
   Current status: implemented for native library ELF attributes, ZIP
   compression metadata, and pure-Rust stored/deflated ZIP entry extraction.
2. Finish the ARMv7-A interpreter audit with focused QEMU oracle tests.
   Current status: broad ARMv7-A/VFPv2 coverage exists, but the audit is not
   complete.
3. Add an ELF loader/linker for `armeabi-v7a` `.so` files and imported-symbol
   dispatch into HLE shims.
   Current status: APK run planning selects `lib/armeabi-v7a` as the ARMv7-A
   interpreter ABI. Initial ELF `PT_LOAD` segment planning and
   `VecMemory` materialization exists. Dynamic metadata parsing now reports
   `DT_NEEDED`, dynamic symbol imports, relocation table ranges, and init
   arrays. ARM `REL` relocation entries are decoded and associated with dynamic
   symbol names. Initial ARM `REL` relocation application exists for
   `RELATIVE`, `ABS32`, `REL32`, `GLOB_DAT`, `JUMP_SLOT`, and `TARGET1`;
   checked multi-region guest memory exists for mapping multiple libraries. A
   native APK linker probe now loads APK-local `.so` dependencies in dependency
   order, maps every segment at its final 1:1 guest virtual address, builds a
   global dynamic-symbol table, reserves guest HLE trampoline addresses, and
   reports unresolved imports before relocation. Initial system-library HLE
   metadata and dispatch now covers the local Minecraft PE import set well
   enough for the `armeabi-v7a` research probe to load and relocate with zero
   unresolved imports. A native runtime shell now maps stack/TLS/heap and wires
   ARM HLE trap stubs back to symbol-name dispatch. It can enumerate relocated
   `DT_INIT`/`DT_INIT_ARRAY` constructor targets and run each target until it
   returns through a sentinel LR. A `run-apk-native` CLI now reaches actual
   constructor execution and the local ARMv7 Minecraft PE APK now runs through
   the native activity path into EGL/GLES first-frame probes. Further
   EGL/GLES/audio/Android lifecycle behavior is still pending.
4. Build Bionic/libc/pthread/time/file/memory shims only as demanded by target
   imports.
5. Implement EGL facade and GLES 2.0 command translation to SDL2 GL/WebGL.
   Current status: initial host abstraction exists, with a feature-gated SDL2
   GLES2 debug shell and WebGL 1/WebGL 2 context-selection scaffolding. GLES
   HLE now records clear, viewport, draw, swap, buffer, texture, uniform,
   vertex-attrib, and common render-state events into a bounded command queue.
   Historical MCPE first-frame traces captured 21,674 GLES events before the
   first swap without queue saturation, including 3,811,776 copied payload
   bytes for buffer, texture, uniform, and client-side draw-index data, and the
   SDL2 host replayed 744 indexed draws with nonzero RGB/alpha pixels. The
   current 64-slice `tools/mcpe_smoke.py` baseline reaches the live frame limit
   but records zero draw submissions and black RGB readback. A heavier
   256-slice guest-thread diagnostic reaches real `DrawElements`, proving the
   native resource path can progress into drawing, but it times out before long
   smoke completion. The active blocker is now efficient guest-thread scheduling
   around resource preload and `eglSwapBuffers`, then UI progression. A
   wasm-only WebGL replay host now compiles and mirrors the SDL2 guest
   object/state mapping.
   APK linking and Android asset reads both have byte-backed paths, so browser
   code does not need a host filesystem path for native libraries or
   `AAssetManager_open`. The initial wasm export and static harness can run
   MCPE bytes to first swap and replay the resulting GLES stream into WebGL 1;
   browser-side performance and worker/asynchronous execution are still pending.
   SDL2 live mode now resumes guest execution after each `eglSwapBuffers`,
   replays each frame batch into the SDL2 GLES context on `DISPLAY=:0`, and
   still reports zero host GL errors. `tools/mcpe_ui_smoke.py` now wraps SDL2
   live mode and the WebSocket harness into a one-command UI journal for
   multi-step tap/screenshot/debug checks, with default screenshots under
   `tmp/`. It can gate the journal on visible `DrawElements`/readback progress
   and has a verified start-screen script that taps `Not Now` after the first
   visible draw and reaches the main menu. Full playability still needs much
   faster cooperative guest-thread/resource scheduling, deeper menu/world
   interaction checks, and audio HLE.
6. Add GLES 1.1 fixed-function emulation over shaders for older games that
   require it.
7. Add input/audio/storage HLE and enough Android lifecycle/JNI glue to reach
   the first frame.
8. Build the wasm/WebGL target and keep desktop SDL2 as the debugger target.
   Current status: the crate checks for `wasm32-unknown-unknown` with the
   `webgl` feature and builds a `cdylib` wasm target for `wasm-bindgen` via
   `cargo build --lib --target wasm32-unknown-unknown --no-default-features --features webgl`.

## Sources Checked

- Android app fundamentals and APK packaging:
  https://developer.android.com/guide/components/fundamentals
- Android `android:extractNativeLibs` manifest behavior:
  https://developer.android.com/guide/topics/manifest/application-element#extractNativeLibs
- Android NDK ABI documentation:
  https://developer.android.com/ndk/guides/abis
- Android runtime documentation:
  https://source.android.com/docs/core/runtime
- WebGL 1.0 specification:
  https://registry.khronos.org/webgl/specs/latest/1.0/
- WebGL 2.0 specification:
  https://registry.khronos.org/webgl/specs/latest/2.0/
- Emscripten OpenGL/WebGL support notes:
  https://emscripten.org/docs/porting/multimedia_and_graphics/OpenGL-support.html
- Local AOSP Dalvik 4.4.4 reference:
  `../aemu-refs/aosp-dalvik-4.4.4_r2`
- Local Dynarmic reference:
  `../aemu-refs/dynarmic`
