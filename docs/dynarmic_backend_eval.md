# Dynarmic Backend Evaluation

Date: 2026-05-19

## Goal

Evaluate Dynarmic as an optional native-only ARMv7-A CPU backend for SDL2
performance experiments, without replacing the default Rust `armv7a`
interpreter and without changing the wasm/WebGL runtime direction.

## Constraints

- `--cpu-backend aemu` remains the default portable backend.
- `--cpu-backend dynarmic` is available only with the native `dynarmic` Cargo
  feature.
- Dynarmic is not built for wasm targets. `build.rs` fails clearly if the
  `dynarmic` feature is enabled for `wasm32`.
- Dynarmic executes guest ARM/Thumb code only. AEMU still owns guest memory,
  HLE imports, JNI/native-activity glue, cooperative guest-thread scheduling,
  EGL/GLES capture, and SDL2/WebGL replay.
- MCPE game/render-engine HLE hooks are not part of this performance
  experiment.

## Implementation Evidence

- `Cargo.toml` adds the optional `dynarmic` feature.
- `build.rs` builds the local Dynarmic reference checkout from
  `../aemu-refs/dynarmic` for native targets only.
- `src/dynarmic_shim.cpp` isolates the C++ Dynarmic API and exposes a small C
  ABI.
- `src/dynarmic_backend.rs` wraps the C ABI, synchronizes ARM/VFP/CP15 CPU
  state, and contains focused backend tests. Debug builds and unmapped pages
  still fall back through AEMU `Memory` callbacks; Linux release builds provide
  Dynarmic a host page table for directly mapped non-executable guest pages.
  Guest writes that overlap executable pages invalidate the matching Dynarmic
  cache range after the current step/run returns.
- `src/native_loader.rs` records executable ELF `PT_LOAD` ranges so the
  Dynarmic backend can avoid scanning every object range on each guest store.
- `src/native_runtime.rs` adds `NativeCpuBackendKind::Dynarmic`, runtime
  dispatch, HLE trap handling, kuser/helper handling, and scheduler step
  accounting.
- `tools/mcpe_smoke.py` accepts `--cpu-backend dynarmic` and
  `--dynarmic-run-ticks`.
- `tools/mcpe_ui_smoke.py` accepts the same backend/tick options so WebSocket
  UI journals can exercise Dynarmic without hand-launching SDL2.

## Correctness Fixes Found During Evaluation

1. Thumb ITSTATE was lost across AEMU/Dynarmic CPU synchronization because
   `Cpsr` only preserved NZCVQ/GE/T.
   - Fix: `src/armv7a.rs` now encodes/decodes CPSR IT bits.
   - Regression: `dynarmic_preserves_thumb_it_state_across_cpu_sync`.

2. HLE calls reached inside a Dynarmic chunk reported stale r0-r3 values to
   the runtime scheduler because the `HleCall` step used registers from the
   chunk start.
   - Fix: HLE dispatch now uses synchronized current CPU registers.
   - Regression: `dynarmic_defers_chunk_hle_trap_with_current_args`.

3. HLE calls reached inside a Dynarmic chunk were counted as a single runtime
   step, undercounting guest instructions before the trap and delaying
   cooperative guest-thread scheduling.
   - Fix: the runtime now defers HLE dispatch when a chunk reaches an HLE trap
     after prior guest work. It first returns the pre-HLE guest instruction
     count, then dispatches the HLE trap on the next runtime step.
   - Result: `AEMU_DYNARMIC_RUN_TICKS=256` is stable for current first-swap
     and 260-frame loading-smoke probes.

4. Guest branches to ARM kuser helper addresses can surface from Dynarmic as
   either memory aborts or `NoExecuteFault` exceptions instead of normal
   decoded instructions, because AEMU does not map the high kuser page.
   - Fix: Dynarmic abort/exception handling now dispatches recognized kuser
     helper addresses through the existing runtime kuser path.
   - Regressions:
     `dynarmic_dispatches_kuser_helper_reached_inside_chunk` and
     `dynarmic_dispatches_kuser_no_execute_exception_reached_from_thumb`.

5. Guest writes to executable code must not leave stale Dynarmic cached blocks.
   - Fix: executable `PT_LOAD` pages and HLE trap pages are registered with the
     Dynarmic wrapper; writes to those pages invalidate the corresponding
     cached range after the current Dynarmic step/run exits.
   - Regression:
     `dynarmic_invalidates_cached_blocks_when_guest_writes_executable_range`.
   - Note: immediate invalidation from inside Dynarmic's memory callback was
     rejected because it can re-enter the JIT unsafely. The current target APK
     does not need same-block self-modifying branch support; if that appears,
     run with smaller Dynarmic tick chunks while adding a safe execution
     barrier.

6. Hidden fake-time defaults distorted SDL2/Dynarmic progress during
   performance runs.
   - Fix: Android time HLE now defaults to host `SystemTime`/`Instant`.
     `AEMU_FAKE_TIME_STEP_NANOS` and
     `AEMU_FAKE_TIME_STEP_AFTER_DRAW_NANOS` remain explicit diagnostics only,
     and the smoke harnesses no longer inject fake time for visible-draw
     milestones.

7. Dynarmic release runs spent too much time crossing from C++ JIT memory
   accesses into Rust callbacks.
   - Fix: Linux release builds create a Dynarmic page table from AEMU's 1:1
     guest mappings. Non-executable mapped pages are read/written directly by
     generated code through host addresses; executable pages stay callback-only
     so code writes still trigger cache invalidation.

## Verification

Commands run:

```sh
cargo test --features dynarmic dynarmic
cargo test --features dynarmic
cargo test --lib
cargo build --release --features sdl2,dynarmic
cargo check --target wasm32-unknown-unknown --no-default-features --features webgl
cargo check --target wasm32-unknown-unknown --no-default-features --features webgl,dynarmic
python3 -m py_compile tools/mcpe_smoke.py tools/mcpe_ui_smoke.py tools/ws_cli.py
```

Results:

- Dynarmic focused tests: 10 passed.
- Full Dynarmic feature test suite: 227 unit tests passed, plus the
  `armv7a_oracle` integration oracle test passed.
- Library tests: 217 passed.
- Release SDL2/Dynarmic build: passed.
- WebGL wasm check without Dynarmic: passed, with one existing
  `guest_memory.rs` unused-variable warning.
- WebGL wasm check with Dynarmic: failed intentionally in `build.rs` with
  `the dynarmic feature is native-only and is not supported for wasm targets`.

MCPE SDL2 smoke results:

| Backend | Trace Dir | Elapsed | Stage | First Swap Step | Notes |
| --- | --- | ---: | --- | ---: | --- |
| AEMU interpreter | `tmp/dynarmic-eval-aemu-baseline-current-1779119332` | 15.920s | completed | 311060354 | baseline first-swap run |
| Dynarmic default ticks=256 | `tmp/dynarmic-eval-dynarmic-default256-1779119318` | 6.422s | completed | 311539332 | first swap, 1 swap, 0 GL errors |
| Dynarmic default ticks=256, 260 frames | `tmp/dynarmic-eval-dynarmic-default256-frames260-final-1779119555` | 8.918s | completed | 311539332 | 260 swaps, 0 GL errors |
| Dynarmic after executable-page invalidation bitmap | `tmp/dynarmic-eval-dynarmic-post-bitmap-invalidation-1779122826` | 6.409s | completed | 311539332 | first swap, 1 swap, 0 GL errors |

Earlier chunk probes after the HLE fixes also reached first swap with
`ticks=9`, `16`, `32`, `64`, `128`, and `256`.

MCPE UI/WebSocket smoke results:

| Milestone | Trace Dir | Elapsed | Evidence |
| --- | --- | ---: | --- |
| First visible draw | `tmp/dynarmic-eval-dynarmic-first-visible-draw-1779119600` | 8.845s | frame 59, 721 `DrawElements`, screenshot under `stop.png` |
| Not Now -> main menu -> Play | `tmp/dynarmic-eval-dynarmic-ui-play-frames2000` | 23.369s | 3 screenshots; reaches Worlds tab with `Create New World` |
| Create New World form | `tmp/dynarmic-eval-dynarmic-ui-create-world-menu` | 27.603s | screenshot shows `Create a World` form |
| Create World, 30s wait | `tmp/dynarmic-eval-dynarmic-ui-create-world-run-kuserfix2` | 54.056s | screenshot shows `Generating world / Building terrain`, no runtime failure |
| Create World, 120s wait | `tmp/dynarmic-eval-dynarmic-ui-create-world-wait120` | 143.759s | screenshot shows an in-world first-person view with controls/hotbar, 346570 `DrawElements`, 0 GL errors |
| In-world look drag | `tmp/dynarmic-eval-dynarmic-inworld-look-drag` | 146.247s | 4 screenshots; right-side pointer drag changes the in-world view, 366810 `DrawElements`, 0 GL errors |

PC profiling:

- Trace dir: `tmp/dynarmic-eval-dynarmic-inworld-profile`.
- Settings: interval 65536 guest instructions, flush interval 256, top 40,
  limit 5000 samples.
- Result: 5000 samples across 327680118 guest instructions.
- Top symbols:
  `_Znwj`, `Localization::_appendTranslations`,
  `Json` red-black tree erase, `bn_mul_mont`,
  `Json::Reader::decodeString`, and
  `UIDefRepository::_resolveReferences`.
- The profiler now counts Dynarmic chunk guest-instruction deltas instead of
  sampling every runtime step as one instruction.

## Current Decision

Dynarmic is promising as a native-only acceleration experiment: the current
first-swap smoke is about 2.5x faster than the interpreter on the same SDL2
harness. More importantly, the UI harness now reaches the main menu, the world
creation form, an in-world first-person view, and a visibly changed look-drag
interaction in minutes instead of the previous interpreter-scale
hundreds-of-seconds path.

It is not yet a replacement for the default backend. The current Dynarmic
evidence proves start-screen interaction and world creation into an in-world
rendered state, but it does not yet prove sustained gameplay, audio, save/load
robustness, or touch movement/mining/building interaction. The browser target
still uses the Rust interpreter.

Continue developing the Dynarmic backend as an optional native profiling and
performance path while keeping the AEMU interpreter as the default and wasm
runtime CPU.
