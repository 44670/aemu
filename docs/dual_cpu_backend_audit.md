# Dual CPU Backend Audit

## Objective

Add dual CPU backends for native Android execution:

- keep the current Rust AEMU interpreter backend as the default runtime path;
- add a QEMU ARMv7-A TCG interpreter-mode backend as a second backend;
- use the QEMU backend to bring the target game up first;
- compare QEMU-backed execution against the AEMU interpreter to verify AEMU CPU
  behavior.

## Current Artifacts

| Requirement | Evidence | Status |
| --- | --- | --- |
| Preserve current AEMU interpreter | `NativeRuntime::new(...)` still constructs `NativeCpuBackendKind::AemuInterpreter`; `run-apk-native` defaults to `--cpu-backend aemu`. | Present |
| Backend selection point | `NativeCpuBackendKind` and `NativeRuntime::new_with_cpu_backend(...)` in `src/native_runtime.rs`; CLI parsing for `run-apk-native --cpu-backend aemu|qemu-armv7a-tcg` in `src/main.rs`. | Present |
| QEMU backend does not silently fall back | `rejects_unwired_qemu_cpu_backend_without_fallback` verifies `qemu-armv7a-tcg` returns `UnsupportedCpuBackend`. | Present |
| QEMU ARMv7-A TCG executable runner | `src/qemu_tcg.rs` can compile an ARMv7-A Linux smoke program with clang and execute it under `qemu-arm`; `aemu qemu-tcg-smoke` verifies exit code 42. | Present as native-only diagnostic runner |
| QEMU ARMv7-A TCG native-runtime backend | No runtime backend executes linked Android guest code through QEMU TCG yet. Existing QEMU execution is a native-only smoke runner plus instruction-level subprocess oracles in `tests/qemu_oracle.rs`. | Missing |
| Bring MCPE up with QEMU backend | No `run-apk-native --cpu-backend qemu-armv7a-tcg` success path exists. | Missing |
| Compare QEMU and AEMU behavior | `aemu cpu-compare-smoke` runs the same ARMv7-A smoke program under QEMU TCG and the AEMU interpreter and compares the exit value. Existing QEMU oracle tests also compare selected instructions. | Present for smoke snippets; missing for native-runtime traces |

## Verification Commands

```sh
cargo test cpu_backend -- --nocapture
cargo check
target/debug/aemu qemu-tcg-smoke
target/debug/aemu cpu-compare-smoke
target/debug/aemu run-apk-native /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --cpu-backend qemu-armv7a-tcg --steps 1
```

The last command is currently expected to fail with:

```text
CPU backend 'qemu-armv7a-tcg' is not wired into NativeRuntime yet
```

## Next Implementation Slice

The next concrete step is to define the native runtime CPU-backend boundary that
can service HLE traps, guest memory, registers, and thread slices without
exposing the rest of `NativeRuntime` to backend-specific details. After that,
the QEMU-backed implementation can be added as a native-only diagnostic backend
without making QEMU part of the wasm/browser runtime core.
