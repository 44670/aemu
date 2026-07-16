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

## Current Status

- The authoritative runtime remains the custom Rust interpreter. Dynarmic is
  an optional native-only experiment and never substitutes for interpreter or
  WebAssembly acceptance.
- The local ARMv7 Minecraft PE artifact reaches a bounded playable flat world
  under the release interpreter. The canonical UI gate creates the world,
  remains in gameplay, moves, changes view, places a block, and breaks it while
  rejecting replay GL errors and skipped client-attribute draws.
- This playability result does not close the whole-APK correctness gate.
  Native symbol dispatch is clean, but the launcher still injects part of the
  Android lifecycle and selected APK-defined Java methods are semantic Rust
  lifts rather than executed DEX. Keep these three gates separate.
- Audio remains unsupported. An unavailable audio service must fail explicitly
  rather than report fabricated completion.
- `docs/no_fake_hle.md` is the current correctness audit,
  `docs/minecraft_runtime_audit.md` separates current evidence from historical
  investigation, and `docs/interpreter_release_pipeline.md` records the
  retained no-cache performance batch.

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
- EGL must maintain guest-visible display/config/context/surface ownership and
  error state, while the host SDL2/WebGL backend presents `eglSwapBuffers`.

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
- Android native app entrypoints and stateful lifecycle delivery
- `ANativeActivity`
- `ANativeWindow`
- `AInputQueue`
- EGL state model and host presentation bridge
- GLES state model and checked host replay
- OpenSL ES or AudioTrack-style audio model mapped to SDL2 audio
- Touch, keyboard, and controller input mapped from SDL2/browser events
- Save-data and external-storage path mapping

Avoid building broad framework behavior until a target game actually needs it.

## No-Fake-HLE Contract

HLE is permitted only at external Android/system ABI boundaries for services
that are absent from the loaded guest process. It must not replace APK-defined
game or engine logic, manufacture game state, skip a guest callback or thread,
or turn an unsupported operation into generic success.

- Every APK-defined game/engine function executes as guest code. The only
  defined-symbol overrides are the explicit compiler integer helper list in
  `src/native_loader.rs`, whose arithmetic semantics are tested.
- A system entry marked implemented must validate its inputs and model its
  return value, output memory, persistent state, side effects, and failure
  behavior that the guest can observe. A bounded unavailable service returns a
  documented failure or configured absence; it does not pretend success.
- Unknown or unmodeled imports and JNI entries fail at the exact call. Do not
  add generic return-zero, return-one, or void-success dispatch behavior.
- Every successful `pthread_create` schedules the supplied guest start routine.
  Native-app-glue itself creates the app thread; launchers promote that real
  pthread and never call `android_main` directly or fabricate a fallback
  `android_app`.
- Target-specific symbol registries, environment-controlled game hooks,
  resource-completion bridges, and selective thread allowlists are forbidden,
  including diagnostic and test-only variants.
- APK-defined Java methods are APK logic and ultimately execute through the
  DEX/activity path. Existing declaration-checked Rust semantic lifts are
  explicitly recorded incomplete state, not precedent for adding more lifts
  or claiming whole-APK correctness.
- Any exception to these rules needs an explicit current platform requirement,
  a semantic test, and a link audit proving it cannot capture game code.

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

- Keep one authoritative direct ARM/Thumb/VFP/NEON decoder and one set of
  semantic leaves. Do not add a PC-indexed decoded-instruction cache,
  decoded-op cache, basic-block cache, executable-page tracking, threaded-code
  engine, parallel decoder, or JIT.
- Ordinary release execution stays in the fused `Cpu::run_release_batch` loop
  with CPU and budget state resident. Runtime, HLE, scheduler, diagnostics,
  formatting, and errors run only on exact side exits. Diagnostic and
  one-instruction callers continue to use the same decoder through outlined
  wrappers.
- Hierarchical direct routing may bypass unrelated decoder probes only when it
  reaches the existing authoritative semantic leaf and differential tests
  prove that every excluded special encoding still takes the canonical route.
- Keep guest memory access behind explicit checked read/write helpers.
- Keep Linux/Android syscalls and imported shared-library symbols in HLE layers,
  not inside the CPU core.
- Inline only after matched native, WebAssembly, and end-to-end measurements
  plus linked objdump inspection show a retained gain. Do not perform broad
  `#[inline(always)]` passes or spend code size on unmeasured machinery.
- Track validated CPU behavior in `docs/armv7a_cpu_validation.md`; green unit
  tests alone do not prove complete ARMv6 or ARMv7 architecture coverage.
- Use `docs/interpreter_release_pipeline.md` for generated-code and benchmark
  evidence and `docs/dynarmic_backend_eval.md` for the optional backend's
  limits. Historical measurements are tied to their recorded binaries and are
  not claims about a later source tree.

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

It creates a unique `tmp/mcpe-smoke-*` trace directory, writes `link.log`,
`run.log`, `summary.json`, GLES event JSONL, and SDL draw PNG artifacts when a
frame is reached. It also parses crash PC/fault address and symbolicates the PC
against the linked native object. It records parsed live-frame stats plus GLES
swap/draw counts in `summary.json`; use `--min-gles-events`,
`--min-gles-swaps`, `--min-gles-draw-elements`, `--min-sdl-draw-pngs`,
`--min-readback-rgb`, and `--max-gl-errors` to turn those artifacts into
regression gates.

The current end-to-end interaction gate is:

```sh
tools/mcpe_ui_smoke.py --cpu-backend aemu --preset playable-flat-world
```

Any loader, launch, scheduler, HLE, EGL/GLES, input, or JNI change that can
affect MCPE must rerun this gate in release-interpreter mode. Counters alone are
not acceptance: inspect the machine-checked world and interaction screenshots.

### Historical MCPE Diagnostics

The following presets reproduce earlier failures and remain useful for
regression localization. They are not descriptions of the current blocker and
must not replace the canonical playability and no-fake gates. For historical
blocker tracking, pass explicit expectations such as:

```sh
tools/mcpe_smoke.py --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
```

For the historical MCPE certificate/WebToken failure, use the built-in native
trace preset instead of hand-writing `AEMU_TRACE_NATIVE_*` environment
variables:

```sh
tools/mcpe_smoke.py --native-trace-preset webtoken \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10 \
  --expect-native-event WebToken::createFromData.return-null
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 20
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
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 40
```

If generated EC private keys later fail point validation, use `keygen-ec` to
follow bundled OpenSSL's `EC_KEY_generate_key` and `EC_POINT_mul` path:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen-ec \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 120
```

Use `keygen-mul` to inspect `EC_POINT_mul` inputs, the group generator point,
the private scalar BIGNUM limbs, and the generated public point. Combine it
with `--cpu-feature-preset no-neon` to force OpenSSL away from its NEON
Montgomery multiply path and compare scalar-vs-NEON behavior:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen-mul \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/mcpe_smoke.py --cpu-feature-preset no-neon --native-trace-preset keygen-mul \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 80
```

Use `bn-mont` when `keygen-mul` points at low-level OpenSSL field arithmetic;
it dumps `BN_mod_mul_montgomery` and `bn_mul_mont` limb buffers so individual
Montgomery products can be checked against a host oracle:

```sh
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-mont-check
```

Use `ec-point-ops` after Montgomery multiplication checks out; it traces
bundled OpenSSL `ec_GFp_simple_dbl`, `ec_GFp_simple_add`, and
`ec_GFp_simple_make_affine` point-coordinate inputs/outputs so the bad public
point can be narrowed to point arithmetic or affine conversion:

```sh
tools/mcpe_smoke.py --native-trace-preset ec-point-ops \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 80
```

Use `point-affine` when `EC_POINT_mul` produces the correct Jacobian point but
`ec_GFp_simple_point2oct` serializes the wrong affine coordinates; it traces
`ec_GFp_simple_point_get_affine_coordinates` through decoded Z, Z inverse,
Z-inverse squared, and final output `BIGNUM` limbs:

```sh
tools/mcpe_smoke.py --native-trace-preset point-affine \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --contains point_get_affine --limit 120
```

If affine tracing shows `Z^-2` diverging after
`ec_GFp_simple_field_sqr -> BN_mod_sqr`, use `bn-mod-sqr` to check bundled
OpenSSL modular squaring directly against a host oracle. This preset verifies
the `BN_sqr` output before the `BN_div` reduction step:

```sh
tools/mcpe_smoke.py --native-trace-preset bn-mod-sqr \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-mod-sqr-check
```

If `bn-mod-sqr-check` reports `square_ok=True` but the final reduced result is
wrong, use `bn-nnmod` to isolate the bundled OpenSSL division/remainder path:

```sh
tools/mcpe_smoke.py --native-trace-preset bn-nnmod \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-nnmod-check
```

If `bn-nnmod-check` points at `BN_div`, first verify the 64-bit division import
used by the quotient-estimation path:

```sh
tools/mcpe_smoke.py --trace-hle =__aeabi_uldivmod --trace-hle-limit 80 \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> hle-uldivmod-check
```

Then use `bn-div` to check `BN_div` directly, including the final normalized
remainder before and after the right-shift denormalization step:

```sh
tools/mcpe_smoke.py --native-trace-preset bn-div \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-div-check
```

If `bn-div-check` shows a bad remainder before denormalization, narrow the
fault inside the OpenSSL long-division loop with `bn-div-loop`. This verifies
the local quotient-estimate call, `MLS`/`UMULL`/`MLA` arithmetic, and the
`bn_mul_words`/`bn_sub_words`/`bn_add_words` helper inputs and outputs without
HLE-ing OpenSSL or MCPE game methods:

```sh
tools/mcpe_smoke.py --native-trace-preset bn-div-loop \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-div-loop-check
```

The historical `font-texture-pair` preset reproduces the former native
`Font::init` crash. It traces the `TextureGroup::getTexturePair` lookups for
`font/default8.png` and `font/ascii_sga.png`; the old failure returned null for
`ascii_sga.png` and dereferenced it at `0x70c3cad4`:

```sh
tools/mcpe_smoke.py --native-trace-preset font-texture-pair \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x70c3cad4 --expect-fault-address 0x40
tools/trace_query.py tmp/mcpe-smoke-<stamp> mcpe-font-pair-check
```

The static OpenSSL `bn_div_words` wrapper is available as an extra check, but
the current MCPE `BN_div` hot path may call `__aeabi_uldivmod` through PLT
directly and record no `bn_div_words` events:

```sh
tools/mcpe_smoke.py --native-trace-preset bn-div-words \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> bn-div-words-check
```

Use `keygen-serialize` to inspect whether `i2d_ECPrivateKey` / `point2oct`
serializes the generated public point correctly:

```sh
tools/mcpe_smoke.py --native-trace-preset keygen-serialize \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 160
```

After key generation succeeds, use the `signdata` preset to inspect the bundled
OpenSSL signing path without HLE-ing MCPE game or engine functions:

```sh
tools/mcpe_smoke.py --native-trace-preset signdata \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 80
```

If `signdata` shows `d2i_AutoPrivateKey` returning null, add the `d2i-private`
preset to follow bundled OpenSSL's ASN.1 private-key decoder:

```sh
tools/mcpe_smoke.py --native-trace-preset signdata \
  --native-trace-preset d2i-private \
  --expect-stage android_main --expect-exit nonzero \
  --expect-crash-pc 0x71673170 --expect-fault-address 0x10
tools/trace_query.py tmp/mcpe-smoke-<stamp> native-event --limit 120
```

`--trace-hle`, `--trace-hle-limit`, and `--trace-hle-file` write the existing
HLE dispatcher/file diagnostics into `run.log` and summarize their counts in
`summary.json`, so system-boundary failures can be checked without long ad hoc
environment-variable command lines. Prefix an HLE filter token with `=` for an
exact imported-symbol match; for example `=read` avoids matching `pthread_*`.
Native event byte samples use `--native-event-bytes PC:*reg+offset,len` for a
32-bit pointer dereference or `PC:reg+offset,len` for a direct guest address.

### Current SDL2 and Browser Frontends

The shell currently creates a GLES2-style SDL2 context and normalizes
keyboard, mouse, touch, resize, and quit events through `src/host.rs`.
`run-apk-native --sdl2` implies `--until-swap` for now and replays the recorded
first-swap GLES event stream into the SDL2 context after the first guest
`eglSwapBuffers`. For the local MCPE ARMv7 probe this includes shader/program
replay, payload-backed textures/buffers/uniforms, client-side vertex attribute
staging, and all captured indexed draw submissions.
`run-apk-native --sdl2-live` keeps the real native-app-glue pthread running
after the first swap, drains and replays each frame's GLES event batch, and
resumes it to the next `eglSwapBuffers`; use `--sdl2-frames N` for bounded
verification runs. On the
local X11 display, use `DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1` so SDL creates a
GLES context through EGL. The SDL2 live loop can also expose a small local
WebSocket control harness:

```sh
DISPLAY=:0 SDL_VIDEO_X11_FORCE_EGL=1 target/release/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --sdl2-live --ws 127.0.0.1:8766
tools/ws_cli.py --url ws://127.0.0.1:8766 debug
tools/ws_cli.py --url ws://127.0.0.1:8766 screenshot --out tmp/aemu-ws-screenshot.png
tools/ws_cli.py --url ws://127.0.0.1:8766 tap 427 240
```

Current live rendering reaches and interacts with a native flat world under
the release interpreter. The harness captures framebuffer screenshots directly
as PNG; no PPM conversion is expected. WebSocket/SDL2 pointer events enter the
guest through the Android `AInputQueue`/`AMotionEvent` model. The old MCPE
`Multitouch::feed` target hook has been removed. MCPE texture, geometry, UI,
resource, input, and world logic stays native; audio remains unsupported and
must fail explicitly when reached rather than report fake completion. This is
bounded playability evidence only; lifecycle and DEX execution remain open as
described in `docs/no_fake_hle.md`.
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
trace_dir=tmp/mcpe-trace-check
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
trace_dir=tmp/mcpe-draw-trace
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

The native path currently uses a 128x128 text atlas. Historical HLE
font-expansion traces used 256x256, but that target path has been removed. The
native profile rejects the known bad 64x32 binding.

MCPE game/engine target facades have been removed and must not return. This
includes `Font::init()`, `TextureGroup::*`, `AppPlatform::*`
image loaders, `ImageUtils::*`, `GeometryGroup::*`, MCPE input/gamepad/menu
methods, render helpers, profiler/telemetry, social, networking, and Realms
methods. Keep AEMU HLE at Android/system/libc/libm/libstdc++/EGL/GLES import
boundaries, not at MCPE gameplay or rendering-engine methods.

MCPE target facades must not exist in the HLE registry or dispatcher, including
as test fixtures or environment-variable overrides. The linker resolves game
and engine symbols to their native definitions. The old resource bridge
that called `MinecraftClient::onResourcesLoaded` from
`GameRenderer::render` has been removed.

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

Do not HLE or patch MCPE engine methods such as `TextureGroup`, `Font`, or
render-object methods, including for diagnostic or test-only builds. Trace
their native execution at exact PCs without replacing it. The durable target
is correct CPU/object ABI behavior and correct Android/OpenGL ES HLE at the
system/import boundary.

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

## Required Gates

Run these before every commit:

```sh
cargo fmt --check
git diff --check
cargo test
cargo check --features sdl2
cargo check --target wasm32-unknown-unknown --no-default-features --features webgl
```

For interpreter hot-path changes, also build and exercise the release
WebAssembly benchmark and compare the same native MCPE endpoint. Record guest
steps, output/GLES identity, wall and process CPU time, artifact size, and
linked objdump evidence; a microbenchmark alone cannot retain a change.

```sh
cargo build --release --lib --target wasm32-unknown-unknown \
  --no-default-features --features wasm-bench
node tools/wasm_cpu_bench.mjs \
  target/wasm32-unknown-unknown/release/aemu.wasm
```

For changes to APK loading, lifecycle, threading, JNI, HLE, EGL/GLES, input, or
presentation, build release SDL2 and run the relevant bounded smoke. A change
that can affect the interactive MCPE path must pass
`tools/mcpe_ui_smoke.py --cpu-backend aemu --preset playable-flat-world`;
preserve separate symbol-dispatch, native-lifecycle, and DEX/whole-APK
results.

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
