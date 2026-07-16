# ARM Interpreter Release Pipeline

## Scope

This report records the first no-cache release-pipeline batch. The baseline was
captured immediately before the batch from the same dirty development tree. The
candidate retains one ARM/Thumb/VFP/NEON decoder and does not add a decoded
instruction cache, decoded-op cache, basic-block cache, exact-opcode table,
executable-page tracking, or JIT.

The batch makes three structural changes:

- `Cpu::run_release_batch` executes ordinary instructions in one fused loop and
  returns a four-word scalar outcome. Traps are written only on the trap side
  exit.
- Native main-thread, guest-thread, and `pthread_once` release paths select the
  fused AEMU loop once. Diagnostics retain the authoritative one-instruction
  path. Runtime, HLE, scheduler, trace, profile, environment, and formatting
  work is outside the ordinary instruction loop; only exact return, intercepted
  HLE, resource-bridge, kuser, trap, and budget guards cause side exits.
- A32 immediate and unshifted-register data-processing encodings route directly
  to the existing `exec_arm_data_processing` semantic leaf after condition and
  exact special-instruction handling. Encodings which can denote MRS, MSR,
  MOVW, MOVT, hints, multiply, extra transfer, or miscellaneous operations are
  excluded and continue through the canonical narrower decoders.

Fetch and memory semantics are unchanged: Thumb fetch reads the second
halfword only after recognizing a 32-bit prefix, and all instruction/data
accesses still use the checked `Memory` interface.

## Correctness Gates

The final source passed:

```text
cargo fmt --check
cargo test
  222 unit tests passed
  armv7a_qemu_user_oracle_cases_match_aemu passed with qemu-arm 10.0.8
  doc tests passed
cargo build --release
cargo build --release --lib --target wasm32-unknown-unknown \
  --no-default-features --features wasm-bench
cargo build --release --lib --target wasm32-unknown-unknown \
  --no-default-features --features webgl
```

New differential tests compare routed A32 families with the authoritative
semantic leaf and compare release/diagnostic HLE exits, committed step indices,
arguments, and complete CPU state. Existing tests cover ARM/Thumb flags,
shifter carry, IT behavior, PC/interworking, HLE resume, kuser helpers, guest
thread scheduling, waits, and `pthread_once`.

The matched MCPE run reached the first `eglSwapBuffers` at guest step
`310949259` for every baseline and candidate run. Baseline and candidate GLES
JSONL are byte-identical: 375 lines, 75,723 bytes, SHA-256
`83d64590aa3037e688af6348baa23cfe1d5543e67724f3dba40346aa7532c625`.
The complete first-swap run reports 2,780 GLES events and 6,290,728 payload
bytes.

## Benchmark Protocol

Host: Linux 7.1.0-rc2+ x86-64 VMware VM, virtual AMD Ryzen 9 7900X3D, Rust
1.95.0/LLVM 22.1.2. Native and Node runs were pinned to CPU 4. Guest time used
`AEMU_FAKE_TIME_STEP_NANOS=16666667`. Each comparison used one warm-up and seven
measured runs in balanced alternating order
`B,C,C,B,B,C,C,B,B,C,C,B,B,C`.

Native command shape:

```sh
/usr/bin/time -f '%e %U %S %M %x' \
  env AEMU_FAKE_TIME_STEP_NANOS=16666667 taskset -c 4 BUILD/aemu \
  run-apk-native /home/john/tmp/hgfs-deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk \
  --launch --until-swap --steps 400000000
```

Node/V8 used Node 20.19.2, V8 11.3.244.8-node.26, one million loop iterations
(five million guest ARM instructions), and the same one-warm-up/seven-run
alternation. Both builds returned checksum `0x1384628b`.

| Gate | Baseline median | Candidate median | Change |
| --- | ---: | ---: | ---: |
| MCPE first-swap elapsed | 5.81 s | 4.80 s | -17.38% |
| MCPE first-swap process CPU | 5.76 s | 4.75 s | -17.53% |
| Node/V8 Wasm ARM interpreter | 70.900949 ms | 57.493958 ms | -18.91% |

Native elapsed samples were baseline
`5.86,5.81,5.79,5.80,5.81,5.81,5.81` and candidate
`4.80,4.80,4.80,4.82,4.86,4.80,4.97` seconds. Process-CPU medians are computed
per run as user plus system time, not as the sum of separate medians.

Wasm interpreter samples were baseline
`69.705581,69.209002,72.584922,69.489629,74.541873,72.848751,70.900949` and
candidate
`60.824723,57.748974,57.493958,55.511793,55.818050,58.620458,56.997795`
milliseconds.

The separate `MappedMemory`-only Wasm control is not an interpreter gate. Its
median changed from 17.185373 ms to 19.026390 ms (+10.71%) even though its
source was unchanged; the change follows whole-module Wasm layout/inlining.
This batch therefore makes no claim of a general Wasm memory-helper gain.

## Artifact Sizes

The APK SHA-256 is
`ee6380c3d29b39744488acd7b986290d43037f4b210595889cec5c5a0ea04cdb`.

| Artifact | Baseline | Candidate | Change |
| --- | ---: | ---: | ---: |
| Native ELF file | 1,983,616 B | 1,992,072 B | +8,456 B |
| Native ELF `.text` | 1,644,185 B | 1,651,825 B | +7,640 B (+0.465%) |
| Wasm benchmark module | 315,237 B | 316,711 B | +1,474 B (+0.468%) |

Artifact SHA-256 values:

```text
baseline native  0b0cf2f024e0f3654e9612679c702da816538ff0e01323e347a3e953ef448ea9
candidate native e4d5374bdccd3a94af8e7b60aefeb06861540efc136a2c83c545832ac825124e
baseline Wasm    4c6daa1d3a7271d57a4e010136439a645cce7f09944054443da2406e66304c98
candidate Wasm   a0ab6d469a863d4c1c9bb1181859ee1b26c29eea9114feab7ae2c52ba84cfc40
```

## Generated-Code Audit

Complete linked native objdumps were compared. In the baseline, the generic
`Cpu::step` symbol was 0x3274 bytes and contained the heavily inlined decoder.
In the candidate, shared decoder leaves are outlined once: `execute_arm` is
0x590 bytes, `Cpu::step` is 0x6dc bytes, and `run_release_batch` is 0x883 bytes.
The candidate batch loop keeps its count and limit in registers and has four
simple exact-boundary comparisons before checked fetch. The A32 unshifted
register test compiles to a mask/test and jumps directly to
`exec_arm_data_processing`; rejected encodings fall through to
`exec_arm_misc` and the canonical subdecoders.

A 4,739-sample `cpu-clock:u` profile of the candidate attributes 19.39% to the
fused batch, 13.34% to Thumb decode, 7.72% to ARM decode, 7.26% to Thumb-2
decode, and only 0.74% to `exec_arm_misc`. This is consistent with removing the
per-instruction runtime wrapper and unrelated miscellaneous probes rather than
skipping guest work.

Hardware `cycles`, `instructions`, `branches`, and `branch-misses` could not be
measured on this host: VMware exposes perf events but returns literal zero for
all four counters even for an independent CPU loop. They are recorded as
unavailable, not as zero candidate counts. Process CPU, elapsed time, software
sampling, generated code, guest steps, and output hashes remain valid.

## Result

All measured interpreter acceptance ratios are below 1.00 and both native and
Wasm CPU reductions exceed the 5% target. The retained code growth is under
0.5% and implements the shared batch/side-exit interface plus differential
coverage; no cache or second decoder was introduced.
