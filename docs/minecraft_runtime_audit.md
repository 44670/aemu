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
| System import HLE | `src/hle_imports.rs`; local MCPE probe resolves 906 imports with zero unresolved. | Initial coverage |
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native` enumerates 1,604 constructors on local APK. | Initial coverage |
| Browser/WebGL target remains viable | `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl` passes. | Build-gate satisfied |
| SDL2 desktop target remains viable | `cargo check --features sdl2` passes. | Build-gate satisfied |
| Local Minecraft PE can run on ARMv6 interpreter | Current local APK has only `armeabi-v7a`; default `run-apk-native` fails with missing `armeabi`. | Blocked |

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

Forced ARMv7 research probe:

```sh
cargo run -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 1000
```

Result:

```text
constructors: 1604
libfmod.so constructor 0x70015e30 failed: undefined ARM instruction 0xe30260ab at 0x70015e50
```

This is expected: the local APK is ARMv7/Thumb-2/VFPv3/NEON, while the runtime
target is ARMv6/Thumb-1/VFPv2.

## Latest Verification

- `cargo fmt --check`
- `git diff --check`
- `cargo check --features sdl2`
- `cargo check --target wasm32-unknown-unknown --no-default-features --features webgl`
- `cargo test` with 77 unit/integration-facing tests and 95 QEMU oracle tests

## Required Next Input

Provide an older Minecraft PE APK or extracted native library containing:

```text
lib/armeabi/libminecraftpe.so
```

Alternatively, explicitly change the target from ARMv6 to ARMv7/Thumb-2/NEON.
