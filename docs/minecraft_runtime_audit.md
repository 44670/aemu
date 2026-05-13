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
| HLE trap dispatch from interpreter | `src/native_runtime.rs` dispatches ARM UDF HLE traps by guest address and linked runtime HLE entries such as `__dynamic_cast`. | Initial coverage |
| Constructor runner | `src/native_runtime.rs`; `run-apk-native --abi armeabi-v7a --launch` completes all 1,604 constructors on the local APK. | Satisfied for local ARMv7 research APK |
| ARMv7/Thumb-2/NEON research probe | The release launch reaches `JNI_OnLoad`, `nativeRegisterThis`, `ANativeActivity_onCreate`, `android_main`, and the configured step cap without an undefined NEON trap. | Initial coverage |
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
AEMU_TRACE_STEPS=50000000 timeout 240s cargo run --release -- run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --steps 200000000 --launch
```

Abbreviated result:

```text
constructors: 1604
native constructors completed
launch: libfmod.so JNI_OnLoad 0x700ccb68 java_vm 0x600466ec
launch: libminecraftpe.so JNI_OnLoad 0x7128d499 java_vm 0x600466ec
launch: nativeRegisterThis 0x7128d571 env 0x60046668
launch: ANativeActivity_onCreate 0x71294589 activity 0x60046744
launch: android_main 0x7128eef5 android_app 0x60046af0
STEP ... step=50000000/200000000 ...
STEP ... step=100000000/200000000 ...
STEP ... step=150000000/200000000 ...
native run failed: android_main failed: step limit reached
```

The latest forced ARMv7 run does not stop on an undefined NEON opcode. It now
passes the earlier `__dynamic_cast` stack crash through runtime C++ ABI HLE,
uses a 32 MiB default guest stack below TLS, and reaches the 200M-step cap in
`libgnustl_shared.so` near `std::string` copy construction from the MCPE
resource-pack load path. The current MCPE blocker is runtime/libgnustl startup
progress, not vector instruction decode.

## Latest Verification

- `cargo fmt --check`
- `cargo test neon` with 14 focused unit-level tests and 10 QEMU-oracle NEON
  tests
- `cargo test` with 117 unit/integration-facing tests and 108 QEMU oracle tests
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
