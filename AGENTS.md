# AGENTS.md

## Project Target

This project is a Rust-based Android HLE emulator for old Android 4.x-era
OpenGL ES games, with an initial focus on ARMv6/`armeabi` titles such as early
Minecraft PE.

The primary target is running these games inside a browser through
WebAssembly/WebGL. Desktop SDL2 is the native development and debugging target.

This is not a full Android emulator and not a modern APK compatibility layer.
Prefer high-level emulation of the Android native app surface, system services,
EGL/GLES, audio, input, files, and assets needed by specific old games.

## Technology Direction

- Keep this as one Rust crate unless the user explicitly asks to split it.
- Use Rust for the emulator/runtime core.
- Use SDL2 for desktop windowing, input, audio, and GL context management.
- Use Emscripten/wasm for the browser target when that best preserves SDL2 and
  WebGL integration.
- Treat GLES as the guest-facing graphics API.
- Treat WebGL 1 as the baseline browser backend.
- Add WebGL 2 paths only where they remove a real limitation or improve a known
  target game.
- ANGLE may be used on desktop if it helps provide a consistent GLES backend,
  but it is not a substitute for the browser WebGL backend.

## Graphics Priorities

Old Android games may use OpenGL ES 1.1 fixed-function rendering or OpenGL ES
2.0 shaders.

- GLES 1.1 fixed function should be emulated over shader-based backends.
- GLES 2.0 should map as directly as possible to WebGL 1.
- GLES 3.x should be optional and should not be part of the initial baseline.
- EGL should be implemented as a facade that creates and manages the host SDL2
  canvas/context and presents through `eglSwapBuffers`.

Important GLES 1.1 functions to expect in old games include:

- `glMatrixMode`
- `glLoadIdentity`
- `glOrthof`
- `glVertexPointer`
- `glTexCoordPointer`
- `glColorPointer`
- `glEnableClientState`
- `glDrawArrays`
- `glDrawElements`

## Android HLE Priorities

Model only the Android behavior needed to run target games.

Likely early surfaces:

- APK/asset loading
- `lib/armeabi/*.so` loading and imported-symbol resolution
- Bionic/libc/pthread/math/time/file/socket shims as needed
- Android native app entrypoints and lifecycle stubs
- `ANativeActivity`
- `ANativeWindow`
- `AInputQueue`
- EGL facade
- GLES facade
- OpenSL ES or AudioTrack-style audio facade mapped to SDL2 audio
- Touch, keyboard, and controller input mapped from SDL2/browser events
- Save-data and external-storage path mapping

Avoid building broad framework behavior until a target game actually needs it.

## ARM CPU Direction

Use a custom Rust ARM interpreter for guest Android native code.

Do not embed QEMU, Unicorn, or another large CPU emulator as the runtime core.
Those projects are useful as references and test oracles, but they do not fit
the single-crate Rust/wasm/browser direction cleanly.

Interpreter baseline:

- ARMv5TE plus ARMv6 user-mode integer instructions
- ARM state and Thumb-1 state with interworking
- Target-driven ARMv7-A, Thumb-2, VFPv3, and NEON coverage for local
  `armeabi-v7a` Minecraft PE probes, including Thumb-2 NEON decode transforms
  into the shared A32-style NEON handlers
- little-endian ARM EABI
- user-mode condition flags and exceptions needed by native app code
- helper paths for unaligned memory behavior as target games require
- VFP/NEON support should continue to be added from target APK disassembly,
  export reports, and runtime traces, not by trying to claim full architecture
  completeness upfront

References:

- Use the official ARM architecture manuals as the semantic source of truth.
- Use QEMU `qemu-arm`/TCG behavior as an oracle for instruction tests:
  `https://github.com/qemu/qemu`.
- Read Dynarmic for decoder organization, A32/Thumb instruction semantics,
  callback boundaries, block caching ideas, and test-case inspiration.
- Use target APK `.so` export reports and traces to decide which instructions
  and ABI edges are needed first.

Local shallow reference clones live outside the repo at:

```text
../aemu-refs
```

Current reference checkouts:

- `../aemu-refs/dynarmic`
- `../aemu-refs/qemu` (`https://github.com/qemu/qemu`)
- `../aemu-refs/unicorn`
- `../aemu-refs/aosp-dalvik-4.4.4_r2`

Dynarmic notes:

- Dynarmic is a dynamic recompiler, not the runtime architecture for this
  project.
- Do not port its JIT backend into the wasm/browser runtime.
- Its supported guest list includes `v5TE`, `v6K`, `v6T2`, and `v7A`, which
  makes it relevant for ARMv6-era Android behavior.
- Its "bring your own memory system" callback shape is a useful reference for
  keeping guest memory explicit.
- Its documented non-goals and approximations are also useful warnings:
  user-mode only, approximate FPSR behavior, imperfect misaligned access
  trapping, and approximate exclusive-monitor behavior.

Implementation shape:

- Decode guest instructions into small internal operations or cached decoded
  basic blocks.
- Keep execution interpreter-only for browser compatibility.
- Keep guest memory access behind explicit checked read/write helpers.
- Keep Linux/Android syscalls and imported shared-library symbols in HLE layers,
  not inside the CPU core.
- Track CPU coverage and known gaps in `docs/armv6_status.md`; do not treat
  green unit tests as proof of full ARMv6 completion without updating that
  checklist.
- Use `docs/armv6_completion_audit.md` for the current prompt-to-artifact
  completion audit before deciding whether the ARMv6 interpreter goal is done.

## Native Library Inspection

Use `~/export_rust.py` to inspect Android `.so` files.

Example:

```sh
~/export_rust.py path/to/libminecraftpe.so
```

The script writes a sibling export report named:

```text
libminecraftpe.so.export.txt
```

Use `rg`/`grep` on the generated `*.so.export.txt` files to inspect imports,
exports, JNI symbols, GL/EGL usage, Android native APIs, and libc dependencies.

Useful examples:

```sh
rg 'gl(MatrixMode|CreateShader|VertexPointer|Draw)' *.so.export.txt
rg 'egl[A-Z]' *.so.export.txt
rg 'ANative|AInput|AAsset|slCreateEngine' *.so.export.txt
rg 'JNI_OnLoad|Java_' *.so.export.txt
```

These export reports should guide the HLE surface. Implement symbols demanded
by the target library before broad generic runtime work.

The project CLI can probe a single `.so` or all native libraries in an APK:

```sh
cargo run -- probe-so path/to/libminecraftpe.so
cargo run -- probe-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

The native desktop debug shell is feature-gated behind SDL2:

```sh
cargo run --features sdl2 -- sdl2-shell
cargo run --features sdl2 -- sdl2-shell --frames 120
cargo run --features sdl2 -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 300000000 --sdl2
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 cargo run --release --features sdl2 -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live
```

The shell currently creates a GLES2-style SDL2 context and normalizes
keyboard, mouse, touch, resize, and quit events through `src/host.rs`.
`run-apk-native --sdl2` implies `--until-swap` for now and replays the recorded
first-swap GLES event stream into the SDL2 context after the first guest
`eglSwapBuffers`. For the local MCPE ARMv7 probe this includes shader/program
replay, payload-backed textures/buffers/uniforms, client-side vertex attribute
staging, and all captured indexed draw submissions.
`run-apk-native --sdl2-live` keeps the guest in `android_main` after the first
swap, drains and replays each frame's GLES event batch, and resumes to the next
`eglSwapBuffers`; use `--sdl2-frames N` for bounded verification runs. On the
local X11 display, use `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1` so SDL creates a
GLES context through EGL. The SDL2 live loop can also expose a small local
WebSocket control harness:

```sh
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --ws 127.0.0.1:8766
tools/ws_cli.py --url ws://127.0.0.1:8766 debug
tools/ws_cli.py --url ws://127.0.0.1:8766 screenshot --out target/aemu-ws-screenshot.png
tools/ws_cli.py --url ws://127.0.0.1:8766 tap 427 240
```

Current live rendering reaches MCPE's first `eglSwapBuffers`, replays frames in
SDL2, and the harness captures framebuffer screenshots directly as PNG; no PPM
or conversion step is expected. A run on
`DISPLAY=:0` has been verified past frame 2000 without the previous HLE
`std::string` heap exhaustion. WebSocket/SDL2 pointer events now enter the
guest through a minimal `AInputQueue`/`AMotionEvent` facade and MCPE's
`Multitouch::feed` target hook; a traced tap at `427,240` reached both down and
up feed calls. Do not call this playable yet: the framebuffer still stays on
the gradient/loading frame after input, the fake MCPE texture/geometry/UI
resource HLE remains a likely menu blocker, and audio remains stubbed.
Browser/WebGL replay scaffolding lives in `src/wasm_webgl.rs`; WebGL 1 remains
the default target for GLES2 guest rendering. The wasm-only host mirrors the
SDL2 replay state model with guest-to-host GL object maps, payload upload,
client attribute/index staging, framebuffer readback, and GL error accounting.
`HleRuntime::set_apk_bytes` lets browser-fed APK bytes satisfy Android asset
reads without relying on a host filesystem path.
`src/wasm_api.rs` exports the initial browser MCPE path as
`runMcpeFirstFrame(apkBytes, abi, canvasId, maxSteps)`: it links native
libraries from APK bytes, runs constructors and the native activity to the
first `eglSwapBuffers`, then replays the captured GLES stream into a WebGL 1
canvas. `web/mcpe_first_frame.html` is the static browser harness for that
export after generating `web/pkg` with `wasm-bindgen`; build the wasm library
with:

```sh
cargo build --lib --target wasm32-unknown-unknown --no-default-features --features webgl
```

Texture upload tracing:

```sh
AEMU_DUMP_GLES_TEXTURE_UPLOADS_DIR=target/mcpe-gles-uploads \
AEMU_DUMP_GLES_TEXTURE_UPLOADS_MATCH=64x32 \
AEMU_DUMP_GLES_TEXTURE_UPLOADS_LIMIT=20 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
```

`AEMU_DUMP_GLES_TEXTURE_UPLOADS_DIR` dumps guest-captured texture uploads as
`.png` plus `.raw` and appends a `manifest.jsonl` with guest texture ID, event
index, dimensions, format/type, source pointer, and nonzero-pixel counts. The
SDL replay side also writes `.png`/`.raw` for
`AEMU_DUMP_SDL_TEXTURE_UPLOADS_DIR`; use matching filters such as `all`,
`teximage2d`, `texsubimage2d`, `64x32`, `tex325`, `fmt1908`, or `ty1401`.

## Guest Addressing

Use a 1:1 guest virtual address map in the runtime path.

- Map ELF `PT_LOAD` segments at their final guest virtual addresses.
- Treat `load_bias + st_value`, relocation places, init arrays, stacks, TLS,
  and HLE trampoline addresses as guest addresses directly.
- Do not add a separate runtime address-translation layer between ELF/object
  addresses and guest memory.
- Host storage may use internal vector offsets, but those offsets must stay
  hidden behind the `Memory` trait.

The native linker probe is:

```sh
cargo run -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

The default ABI is `armeabi`. Use `--abi armeabi-v7a` only as a research probe
for the current local Minecraft PE APK; it is not valid for the ARMv6 runtime
target.

System-library import metadata and the first HLE dispatcher live in
`src/hle_imports.rs`. Keep adding symbols there from real APK unresolved import
reports, not from speculative Android surface area.

`src/native_runtime.rs` wires the interpreter to HLE imports: ARM UDF trap
stubs in the HLE page are resolved back to imported symbol names and dispatched
through the HLE runtime, returning through guest LR.

The constructor execution probe is:

```sh
cargo run -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

## Test APKs

Local test APKs live under:

```text
/mnt/hgfs/deb13/AndroidGames
```

Use this directory when inspecting target APKs, extracting `lib/armeabi/*.so`,
checking assets, or running early boot tests. Do not copy large APKs into the
repo unless the user explicitly asks.

Local Minecraft PE probe notes live in:

```text
docs/minecraft_pe_probe.md
```

Current Minecraft runtime completion audit and blocker evidence live in:

```text
docs/minecraft_runtime_audit.md
```

The current research and milestone plan lives in:

```text
docs/research_plan.md
```

## Engineering Approach

- Keep the first milestones game-driven.
- Keep the Rust project in a single crate.
- Start with one known APK/library and add only the HLE needed to boot it.
- Prefer explicit symbol tables and small shims over large speculative APIs.
- Keep desktop and browser backends sharing the same emulator core.
- Put platform-specific code behind narrow backend traits or modules.
- Keep guest memory access explicit and bounds-checked.
- Make graphics state tracking testable without a real GL context where
  practical.
- Use `rg` for code and symbol searches.

## Non-Goals For Now

- Full Android framework compatibility.
- Modern ART-only APK support.
- JIT compilation in the browser.
- GLES 3.x as a required baseline.
- General-purpose Play Store APK compatibility.
