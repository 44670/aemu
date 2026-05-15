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
- `../aemu-refs/aosp-bionic-4.4.4_r2`

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

Prefer the one-command MCPE smoke harness when checking default SDL2 progress:

```sh
cargo build --release --features sdl2
tools/mcpe_smoke.py
```

It creates a unique `target/mcpe-smoke-*` trace directory, writes `link.log`,
`run.log`, `summary.json`, GLES event JSONL, and SDL draw PNG artifacts when a
frame is reached. It also parses crash PC/fault address and symbolicates the PC
against the linked native object. For known blocker tracking, pass explicit
expectations such as:

```sh
tools/mcpe_smoke.py --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
```

For the current MCPE certificate/WebToken blocker, use the built-in native trace
preset instead of hand-writing `AEMU_TRACE_NATIVE_*` environment variables:

```sh
tools/mcpe_smoke.py --native-trace-preset webtoken \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10 \
  --expect-native-event WebToken::createFromData.return-null
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 20
```

To inspect the upstream OpenSSL key-generation path that feeds the WebToken
certificate flow, use the `keygen` preset together with HLE import tracing:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen \
  --trace-hle =open,=read,=fopen,=fread,=gettimeofday,=clock_gettime,=time \
  --trace-hle-limit 300 --trace-hle-file \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10 \
  --expect-native-event OpenSSLInterface::generateKeyPair.fail-keygen2
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 40
```

If generated EC private keys later fail point validation, use `keygen-ec` to
follow bundled OpenSSL's `EC_KEY_generate_key` and `EC_POINT_mul` path:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen-ec \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 120
```

Use `keygen-serialize` to inspect whether `i2d_ECPrivateKey` / `point2oct`
serializes the generated public point correctly:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen-serialize \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 160
```

After key generation succeeds, use the `signdata` preset to inspect the bundled
OpenSSL signing path without HLE-ing MCPE game or engine functions:

```sh
tools/mcpe_smoke.py --native-trace-preset signdata \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 80
```

If `signdata` shows `d2i_AutoPrivateKey` returning null, add the `d2i-private`
preset to follow bundled OpenSSL's ASN.1 private-key decoder:

```sh
tools/mcpe_smoke.py --native-trace-preset signdata \
  --native-trace-preset d2i-private \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py target/mcpe-smoke-<stamp> native-event --limit 120
```

`--trace-hle`, `--trace-hle-limit`, and `--trace-hle-file` write the existing
HLE dispatcher/file diagnostics into `run.log` and summarize their counts in
`summary.json`, so system-boundary failures can be checked without long ad hoc
environment-variable command lines. Prefix an HLE filter token with `=` for an
exact imported-symbol match; for example `=read` avoids matching `pthread_*`.
Native event byte samples use `--native-event-bytes PC:*reg+offset,len` for a
32-bit pointer dereference or `PC:reg+offset,len` for a direct guest address.

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
guest through a minimal Android `AInputQueue`/`AMotionEvent` facade. The old
MCPE `Multitouch::feed` target hook is diagnostic-only and not linked by
default. Do not call this playable yet: the framebuffer still stays on the
gradient/loading frame after input, default execution now keeps MCPE
texture/geometry/UI/resource logic native, and audio remains stubbed.
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
trace_dir=target/mcpe-trace-check
AEMU_DUMP_GLES_TEXTURE_UPLOADS_DIR=$trace_dir/hle \
AEMU_DUMP_GLES_TEXTURE_UPLOADS_MATCH=64x32 \
AEMU_DUMP_GLES_TEXTURE_UPLOADS_LIMIT=2 \
AEMU_DUMP_SDL_TEXTURE_UPLOADS_DIR=$trace_dir/sdl \
AEMU_DUMP_SDL_TEXTURE_UPLOADS_MATCH=64x32 \
AEMU_DUMP_SDL_TEXTURE_UPLOADS_LIMIT=2 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
tools/trace_check.py "$trace_dir" --expect-hle 2 --expect-sdl 2
```

`AEMU_DUMP_GLES_TEXTURE_UPLOADS_DIR` dumps guest-captured texture uploads as
`.png` plus `.raw` and appends a `manifest.jsonl` with guest texture ID, event
index, dimensions, format/type, source pointer, and nonzero-pixel counts. The
SDL replay side also writes `.png`/`.raw` for
`AEMU_DUMP_SDL_TEXTURE_UPLOADS_DIR`; use matching filters such as `all`,
`teximage2d`, `texsubimage2d`, `64x32`, `tex325`, `fmt1908`, or `ty1401`.
`tools/trace_check.py` validates PNG structure, nonblank RGB payloads, raw
payload lengths, HLE manifest consistency, and HLE-vs-SDL upload matches.

Per-draw framebuffer tracing:

```sh
trace_dir=target/mcpe-draw-trace
AEMU_TRACE_SDL_DRAW_CHANGES=200 \
AEMU_DUMP_SDL_DRAW_CHANGES_DIR=$trace_dir/sdl-draw \
AEMU_DUMP_SDL_DRAW_CHANGES_MATCH=program86,tex325 \
AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT=20 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
tools/trace_check.py "$trace_dir" --expect-hle 0 --expect-sdl 0 --expect-draws 1 --no-require-pairs
tools/trace_query.py "$trace_dir" mcpe-text
```

`AEMU_DUMP_SDL_DRAW_CHANGES_DIR` writes changed default-framebuffer draws
directly as `.png` files plus `draw_manifest.jsonl`. The manifest records the
draw's program/texture, default framebuffer delta, and the bound texture's
tracked dimensions, format/type, last upload size, payload length, and nonzero
pixel counts. Use
`AEMU_DUMP_SDL_DRAW_CHANGES_MATCH` tokens such as `all`, `DrawElements`,
`event1671`, `draw42`, `program86`, `prog86`, or `tex325`.
Use `tools/trace_query.py "$trace_dir" summary`, `texture 325`,
`program 86`, `gles-event 21624`, or `mcpe-text` to inspect texture provenance
without manually opening PNGs. For MCPE text regression checks, gate captured
draws with either:

```sh
tools/trace_check.py "$trace_dir" --expect-hle 0 --expect-sdl 0 --expect-draws 1 \
  --no-require-pairs --expect-draw-program 86 \
  --require-draw-texture-size 86:256x256 --reject-draw-texture-size 86:64x32
tools/trace_query.py "$trace_dir" mcpe-text
```

For the default native TextureGroup path, use:

```sh
tools/trace_query.py "$trace_dir" mcpe-text --profile native
```

The native path currently uses a 128x128 text atlas; the old HLE
font-expansion path uses 256x256. Both profiles reject the known bad 64x32
binding.

MCPE game/engine target facades are diagnostic only and must stay native by
default. This includes `Font::init()`, `TextureGroup::*`, `AppPlatform::*`
image loaders, `ImageUtils::*`, `GeometryGroup::*`, MCPE input/gamepad/menu
methods, render helpers, profiler/telemetry, social, networking, and Realms
methods. Keep AEMU HLE at Android/system/libc/libm/libstdc++/EGL/GLES import
boundaries, not at MCPE gameplay or rendering-engine methods.

For old experiments only, set `AEMU_MCPE_HLE_GAME_LOGIC=1` to restore all MCPE
target HLE facades. To restore just the old TextureGroup diagnostic HLE
facades, set `AEMU_MCPE_HLE_TEXTURE_GROUP=1` or the narrower
`AEMU_MCPE_HLE_TEXTURE_DATA=1`, `AEMU_MCPE_HLE_TEXTURE_PAIR=1`, or
`AEMU_MCPE_HLE_TEXTURE_IS_LOADED=1`. Mixing native `getTexturePair` with the
old diagnostic `isLoaded(...) == true` HLE facade can make
`TextureAtlas::redrawAtlas()` call `TexturePair::clear()` on a null native
pair.

The old MCPE resource bridge that calls `MinecraftClient::onResourcesLoaded`
from inside `GameRenderer::render` is also disabled by default. It may be
restored only for old diagnostics with `AEMU_ENABLE_MCPE_RESOURCE_BRIDGE=1`
or `AEMU_MCPE_HLE_GAME_LOGIC=1`.

For native TextureData fallback traces captured in `run.log`, use:

```sh
tools/trace_query.py "$trace_dir" mcpe-texturedata
```

This summarizes the observed
`TextureData -> ResourceLocation -> TextureOGL/gl_name` chain. In the known bad
MCPE path, native `getTexture(TextureData const&)` falls back through an empty
`ResourceLocation` and resolves `TextureOGL` GL name `325`; that is diagnostic
evidence, not a long-term license to HLE MCPE engine methods.

GLES event timeline tracing:

```sh
AEMU_TRACE_GLES_EVENTS_JSONL=$trace_dir/gles_events.jsonl \
AEMU_TRACE_GLES_EVENTS_MATCH=UseProgram,BindTexture,DrawElements,tex325,program86 \
AEMU_TRACE_GLES_EVENTS_LIMIT=25000 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
tools/trace_query.py "$trace_dir" gles-event 21624 --context 4
```

`gles_events.jsonl` uses the same event index space as
`draw_manifest.jsonl:event_index`, so a dumped SDL draw can be traced back to
the captured GLES import-boundary event. Each row includes the event kind,
current program, active texture, bound 2D texture, payload length, and
event-specific fields such as bind target/texture or draw count/type.

Structured native PC event tracing:

```sh
AEMU_TRACE_NATIVE_EVENTS_JSONL=$trace_dir/native_events.jsonl \
AEMU_TRACE_NATIVE_EVENTS='0x716f0534:TextureGroup::uploadTexture;0x716f2070:TexturePtr::ctor.afterGet;0x716f1fec:TexturePair::clear;0x716eb570:TextureOGL::deleteTexture;0x716eb818:TextureOGL::bindTexture' \
AEMU_TRACE_NATIVE_EVENT_MEM32='0x716eb818:r0+0x24,+0x28;0x716f0534:r2,+0x38,+0x3c' \
AEMU_TRACE_NATIVE_EVENT_DEREF32='0x716f2070:r6+0x4,+0x24' \
AEMU_TRACE_NATIVE_EVENT_CXX_STRING='0x716f2038:r2+0,96;0x716f2038:r2+0x4,96' \
AEMU_TRACE_NATIVE_EVENTS_LIMIT=200 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
tools/trace_query.py "$trace_dir" native-event --contains Texture
```

`native_events.jsonl` captures exact-PC guest events with thread id, r0-r3,
sp/lr, full registers, and the current GLES boundary context
(`gles_next_event_index`, current program, active texture, and bound 2D
texture). `AEMU_TRACE_NATIVE_EVENT_MEM32` and
`AEMU_TRACE_NATIVE_EVENT_DEREF32` use the same syntax as `AEMU_TRACE_MEM32` and
`AEMU_TRACE_MEM32_DEREF`, but attach object fields directly to each native JSONL
event. `AEMU_TRACE_NATIVE_EVENT_CXX_STRING` uses the same syntax as
`AEMU_TRACE_CXX_STRING` for string fields. Use these to line up native
object/lifecycle state with GLES import events without scraping large stderr
traces.

Do not treat HLE or patching of MCPE engine methods such as `TextureGroup`,
`Font`, or render-object methods as the long-term fix. Those hooks are only
allowed as temporary diagnostics or compatibility scaffolding while tracing the
real boundary problem. The durable target is correct CPU/object ABI behavior and
correct Android/OpenGL ES HLE at the system/import boundary.

Targeted guest object tracing:

```sh
AEMU_TRACE_MEM32_DEREF='0x716eb818:r0+0x4,+0x24' \
AEMU_TRACE_MEM32_DEREF_LIMIT=80 \
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --sdl2-frames 1
```

`AEMU_TRACE_MEM32_DEREF` uses the same `pc:base+offset,+offset` parser as
`AEMU_TRACE_MEM32`, but each offset dereferences the previous loaded word. Use
it for pointer chains such as `TexturePtr -> TextureOGL -> gl_name` when
checking whether native object state diverges before `glBindTexture`.

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
