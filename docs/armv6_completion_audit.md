# ARMv6 Completion Audit

Objective:

```text
write all armv6 instructions interpreter support, verify with minecraft pe's so,
test with functions
```

## Success Criteria

- ARMv6/ARMv5TE user-mode ARM and Thumb-1 instructions needed by `armeabi`
  Android native code are decoded and executed, or explicitly trapped when they
  are privileged/unsupported.
- VFPv2 instructions needed by ARMv6 hard-float or softfp game code are decoded
  and executed well enough for user-mode HLE.
- Android runtime-critical CPU hooks, especially CP15 TLS reads, have HLE
  behavior.
- Tests cover each implemented instruction family with direct function tests,
  plus QEMU oracle checks for representative arithmetic/VFP behavior.
- A real Minecraft PE ARMv6 `lib/armeabi/libminecraftpe.so` is probed and used
  for target verification.

## Current Evidence

- Interpreter implementation: `src/armv6.rs`
- ELF probe and Minecraft APK probe tests: `src/elf_probe.rs`
- QEMU oracle tests: `tests/qemu_oracle.rs`
- Coverage tracker: `docs/armv6_status.md`
- Local Minecraft probe: `docs/minecraft_pe_probe.md`

Latest verified test command:

```sh
cargo test
```

Result: passing, with 31 unit tests, 32 QEMU oracle tests, and doc tests.

## Prompt-To-Artifact Checklist

| Requirement | Evidence | Status |
| --- | --- | --- |
| Write ARMv6 interpreter support | `src/armv6.rs` implements a custom ARM/Thumb interpreter with ARMv5TE, ARMv6 integer/media/sync/status subsets, CP15 TLS/barrier shims, and VFPv2 subsets; `docs/armv6_status.md` tracks coverage. | Partially complete; full instruction-by-instruction ARM ARM audit is still open. |
| Include ARM and Thumb behavior relevant to old Android `armeabi` games | ARM state, Thumb-1 state, interworking, checked memory, condition codes, common load/store, block transfer, multiply/DSP/media, status, sync, and traps are implemented in `src/armv6.rs`. | Partially complete; ARMv6T2/Thumb-2 is intentionally outside the ARMv6 baseline, and more edge-case audit work remains. |
| Handle privileged or unsupported instructions safely | `CPS`, `RFE`, `SRS`, Thumb `CPS`, SPSR access, CPSR control writes, invalid VFP `PC` core-register forms, and invalid CP15 TLS/barrier `PC` forms trap explicitly. General unsupported instructions return undefined traps. | Partially complete; broad privileged CP15/system behavior is not modeled. |
| Provide VFP support | `src/armv6.rs` implements VFPv2 move, arithmetic, compare, conversion, `VCVTR` FPSCR-rounded conversion, and load/store subsets. | Partially complete; FPSCR exception flags, vector modes, fixed-point conversions, and uncommon forms remain simplified or missing. |
| Verify with Minecraft PE `.so` | `cargo run -- probe-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk` probes the local APK and `docs/minecraft_pe_probe.md` records the result. | Blocked; local `libminecraftpe.so` is `armeabi-v7a`, ARMv7/Thumb-2/VFPv3/NEON, not ARMv6 `armeabi`. |
| Test with functions | Unit tests in `src/armv6.rs`, probe tests in `src/elf_probe.rs` and `src/zip_probe.rs`, plus QEMU oracle tests in `tests/qemu_oracle.rs`. | Partially complete; current tests pass but are representative rather than exhaustive/randomized. |

## Implemented By Audit Category

- ARM branches/interworking: `B`, `BL`, `BX`, `BXJ` as `BX`, `BLX <reg>`,
  `BLX <imm>`, Thumb `BL`, Thumb `BLX`, and ARM/Thumb writes to `PC`
- Thumb high-register `MOV PC, Rm` and `ADD PC, Rm` branch-exchange
  interworking
- ARM/Thumb core ALU, shifts, compares, flag-setting, and common load/store
- ARM data-processing register-shifted-register invalid `PC` forms trap
  explicitly
- ARM halfword/signed/doubleword transfers: `LDRH`, `STRH`, `LDRSB`, `LDRSH`,
  `LDRD`, `STRD`
- ARM/Thumb block transfer basics, including explicit traps for empty
  register lists, ARM user-mode/S-bit block-transfer forms, `PC` base forms,
  and `LDM` writeback with base in the register list
- ARM load/store invalid-form traps for writeback overlap, invalid `PC`
  byte/halfword/register-offset forms, and doubleword pair/writeback overlap
  cases
- Thumb `LDMIA` suppresses writeback when the base register is in the load
  list, matching ARMv6/QEMU behavior
- ARMv5TE and ARMv6 multiply/DSP families listed in `docs/armv6_status.md`,
  with QEMU oracle coverage for representative dual 16-bit multiply,
  dual-long multiply, and high-word multiply variants, plus direct traps for
  invalid `PC` register forms
- ARMv6 extension, packing, selection, reversal, saturation, parallel add/sub,
  and absolute-difference families listed in `docs/armv6_status.md`, with
  broader QEMU oracle coverage across representative signed, unsigned,
  halving, and saturating parallel add/sub variants, plus shifted scalar
  saturation forms and direct invalid `PC` form traps
- ARMv6 synchronization basics: `SWP`, `SWPB`, `LDREX*`, `STREX*`, `CLREX`,
  with explicit traps for invalid `PC`, `SWP` base overlap,
  doubleword-pair, and status-register overlap forms
- ARM/Thumb no-op/control/hint handling listed in `docs/armv6_status.md`
- Explicit user-mode privileged traps for `CPS`, `RFE`, and `SRS`
- Status register access for user-mode APSR/CPSR flags, with privileged traps
  for SPSR and CPSR control-field writes, plus invalid `PC` form traps
- Explicit traps for data-processing exception-return forms that write `PC`
  with the `S` bit set
- ARM/Thumb `SVC`, `BKPT`, and `UDF` trap paths
- Explicit unpredictable traps for invalid VFP core-register `PC` forms
- VFPv2 arithmetic, compare, conversion, `VCVTR` FPSCR-rounded conversion,
  move, `VMOV.32` double-lane, and load/store subset listed in
  `docs/armv6_status.md`
- CP15 user thread ID shim: `TPIDRURW`, `TPIDRURO`, with explicit traps for
  invalid `PC` source/destination forms
- CP15 barrier HLE no-ops for ARMv6-style DMB, DSB, and ISB/prefetch-flush
  `MCR` forms

## Known Incomplete Or Weakly Verified Areas

- Full ARMv6 instruction-by-instruction audit against the ARM ARM is not done.
- Privileged/system behavior is not fully modeled beyond user-mode HLE stubs
  and explicit traps; broad CP15 behavior remains unsupported beyond TLS and
  barrier idioms.
- General coprocessor instructions are not implemented beyond VFP and CP15
  TLS/barrier shims.
- VFP FPSCR exception flags, vector stride/length behavior, and fixed-point
  conversion details are simplified or missing.
- Unpredictable cases are only partially checked; many are simplified to keep
  the HLE interpreter practical.
- QEMU oracle coverage is representative, not exhaustive or randomized.
- Minecraft PE target verification is blocked: the only local Minecraft APK
  contains ARMv7/Thumb-2/VFPv3/NEON libraries, not ARMv6 `armeabi`.

## Minecraft PE Blocker

Current local APK:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

Current native library status:

```text
lib/armeabi-v7a/libminecraftpe.so
```

Probe result: ARMv7, Thumb-2, VFPv3, NEON.

Required to complete target verification:

```text
lib/armeabi/libminecraftpe.so
```

from an older ARMv6 Minecraft PE APK, or an extracted standalone copy of that
library.
