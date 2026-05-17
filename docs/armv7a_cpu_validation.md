# ARMv7-A CPU Validation

This document describes the offline ARMv7-A oracle harness for AEMU's custom
Rust interpreter. The target name for the current Minecraft PE
`armeabi-v7a` probe is `armv7a`.

The production runtime CPU core is still AEMU's interpreter. QEMU user-mode is
used only by tests as an external oracle; it is not linked into AEMU and is not
a runtime backend.

## Command

Run the oracle suite with:

```sh
cargo test armv7a_qemu_user_oracle_cases_match_aemu -- --nocapture
```

Required host tools:

- `clang`
- `llvm-objcopy`
- `llvm-objdump`
- `llvm-nm`
- `qemu-arm`
- `timeout`

The test uses `clang --target=armv7a-linux-gnueabi -nostdlib -static` to build
small Linux ARM ELF binaries. Each ELF writes a normalized result blob under
`qemu-arm`; the same `.testcase` section bytes are loaded into AEMU guest
memory and executed by the Rust interpreter.

## Artifacts

Every case writes reproducible artifacts under:

```text
target/armv7a-oracle/<case>/
```

The artifact set includes:

- `<case>.s`: generated assembly source
- `<case>`: static ARM Linux oracle ELF
- `<case>.testcase.bin`: raw guest bytes executed by AEMU
- `<case>.disasm`: `llvm-objdump -d` output for the ELF
- `aemu_trace.jsonl`: AEMU step trace with step number, PC, and ISA
- `replay.json`: expected qemu-user output, AEMU output, compared coverage,
  paths to the binary inputs, and the first mismatched output field if any
- `divergence.json`: written on mismatch with the first compared divergent
  field, body assembly, qemu-user output, AEMU output, trace path, and disasm
  path

## Compared State

The normalized result blob currently covers:

- `r0-r3`
- CPSR `N/Z/C/V/Q` bits
- `FPSCR`
- `d0-d7`
- four 32-bit probe words at `oracle_probe`

Each structured case selects the specific registers, flags, VFP/NEON state, and
probe words that are semantically meaningful for that case. Ignored state is
still captured in artifacts, but it is not used for pass/fail decisions.

## Current Coverage Buckets

The suite enforces a manifest that requires at least one case for each current
bucket:

- ARM
- Thumb-1
- Thumb-2
- IT
- branch
- interworking
- integer ALU and flags
- load/store addressing and writeback
- multiply and long multiply
- bit operations
- VFPv3
- NEON and target-driven NEON
- OpenSSL-style arithmetic
- MCPE regression behavior

The first MCPE-specific regression case is the Thumb-2 IT/fallthrough literal
load/store loop that previously matched a fragile path in the local
`armeabi-v7a` Minecraft PE probe.

The first-visible-draw localization profile added another MCPE-specific
regression case, `thumb2_localization_it_highreg_loop`. It covers a compact
Thumb-2 loop shaped like the observed `Localization::_appendTranslations`
hot path: high-register `cmp.w`, `ITT NE`, predicated `ldr.w` and `add.w`,
high-register loop control, and a backward conditional branch.

## Adding Cases

Add new cases in `tests/armv7a_oracle.rs` by defining:

- deterministic qemu-user setup assembly
- body assembly for the guest bytes under test
- optional data assembly
- equivalent AEMU setup for registers, memory, and vector state
- a compare mask for meaningful output fields
- coverage bucket tags

Prefer small, focused cases. A failing focused case gives a useful first
semantic divergence: the first mismatched compared output field, the exact
tested body assembly, the raw guest bytes, the disassembly, and the AEMU step
trace are all emitted in the case artifact directory.

Known future expansion buckets include ARM `RBIT`, SDIV/UDIV if target traces
need them, more Thumb-2 load/store forms, and additional OpenSSL bignum/EC
instruction kernels extracted from MCPE traces.
