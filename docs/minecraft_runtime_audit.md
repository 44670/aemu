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
| System import HLE | `src/hle_imports.rs`; local MCPE probe resolves 906 imports with zero unresolved. GLES object-name generation now writes texture/buffer/framebuffer/renderbuffer names back to guest memory. | Initial coverage |
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address and linked runtime HLE entries such as `__dynamic_cast`. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native --abi armeabi-v7a --launch` completes all 1,604 constructors on the local APK. | Satisfied for local ARMv7 research APK |
| ARMv7/Thumb-2/NEON research probe | The release launch reaches `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, `android_main`, EGL setup, GL string queries, texture name generation, and texture upload paths without an undefined NEON trap. | Initial coverage |
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

The latest forced ARMv7 run does not stop on an undefined NEON opcode. It now
passes the earlier `__dynamic_cast` stack crash through runtime C++ ABI HLE and
uses a 32 MiB default guest stack below TLS. A 1.2B-step probe reaches EGL
initialization and GL texture upload, then reaches the step cap in
`GameRenderer::_prepHoloTransforms(float, bool)` around guest
`0x70eccefa`/ELF offset `0x9ccefa`. The current MCPE blocker is later render
progress to `glDraw*`/presentation, not vector instruction decode or initial GL
startup.

After adding GLES object-name writes, a shorter 40M-step probe confirms
`glGenTextures` feeds nonzero guest texture IDs into later `glBindTexture`
calls (`1`, `2`, `3`, ...), instead of leaving the guest on texture `0`.

## Latest Verification

- `cargo fmt --check`
- `cargo test dispatches_gles_object_name_facade_outputs`
- `cargo test` with 129 unit/integration-facing tests and 108 QEMU oracle tests
- `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl`
- `cargo check --features sdl2`
- `cargo run --release -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --limit 5`
  reports 906 resolved imports and zero unresolved imports

## Required Next Input

Provide an older Minecraft PE APK or extracted native library containing:

```text
lib/armeabi/libminecraftpe.so
```

The local `armeabi-v7a` APK remains useful as the active ARMv7/Thumb-2/NEON
research target.
