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

Result: passing, with 38 unit tests, 83 QEMU oracle tests, and doc tests.

## Prompt-To-Artifact Checklist

| Requirement | Evidence | Status |
| --- | --- | --- |
| Write ARMv6 interpreter support | `src/armv6.rs` implements a custom ARM/Thumb interpreter with ARMv5TE, ARMv6 integer/media/sync/status subsets, CP15 TLS/barrier shims, and VFPv2 subsets; `docs/armv6_status.md` tracks coverage. | Partially complete; full instruction-by-instruction ARM ARM audit is still open. |
| Include ARM and Thumb behavior relevant to old Android `armeabi` games | ARM state, Thumb-1 state, interworking, checked memory, condition codes, common load/store, block transfer, multiply/DSP/media, status, sync, and traps are implemented in `src/armv6.rs`. | Partially complete; ARMv6T2/Thumb-2 is intentionally outside the ARMv6 baseline, and more edge-case audit work remains. |
| Handle privileged or unsupported instructions safely | `CPS`, `RFE`, `SRS`, Thumb `CPS`, SPSR access, CPSR control writes, invalid VFP `PC` core-register forms, and invalid CP15 TLS/barrier `PC` forms trap explicitly. General unsupported instructions return undefined traps. | Partially complete; broad privileged CP15/system behavior is not modeled. |
| Provide VFP support | `src/armv6.rs` implements VFPv2 move, arithmetic, compare, conversion, `VCVTR` FPSCR-rounded conversion, FPSCR short-vector arithmetic/unary handling, and load/store subsets. | Partially complete; FPSCR exception flags and uncommon edge cases remain simplified or missing. VFPv3-only fixed-point conversions are outside the ARMv6/VFPv2 baseline. |
| Verify with Minecraft PE `.so` | `cargo run -- probe-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk` probes the local APK and `docs/minecraft_pe_probe.md` records the result. | Blocked; local `libminecraftpe.so` is `armeabi-v7a`, ARMv7/Thumb-2/VFPv3/NEON, not ARMv6 `armeabi`. |
| Test with functions | Unit tests in `src/armv6.rs`, probe tests in `src/elf_probe.rs` and `src/zip_probe.rs`, plus QEMU oracle tests in `tests/qemu_oracle.rs`. | Partially complete; current tests pass but are representative rather than exhaustive/randomized. |

## Implemented By Audit Category

- ARM branches/interworking: `B`, `BL`, `BX`, `BXJ` as `BX`, `BLX <reg>`,
  `BLX <imm>`, Thumb `BL`, Thumb `BLX`, and ARM/Thumb writes to `PC`
- ARM conditional `B`, backward branch offsets, and `BL` link behavior now
  have QEMU oracle coverage
- ARM `BLX <imm>` interworking now has QEMU oracle coverage for switching to
  Thumb state and returning to ARM through `BX LR`
- ARM `BLX <reg>` interworking now has QEMU oracle coverage for register
  targets that enter Thumb state and return to ARM through `BX LR`
- ARM `BX <reg>` interworking now has QEMU oracle coverage for entering Thumb
  state and returning to ARM through another register exchange
- Thumb long `BLX` immediate interworking now has QEMU oracle coverage for
  switching to ARM state and returning to Thumb through `BX LR`
- Thumb `BLX <reg>` interworking now has QEMU oracle coverage for register
  targets that enter ARM state and return to Thumb through `BX LR`
- Thumb `BX <reg>` interworking now has QEMU oracle coverage for register
  targets that enter ARM state
- Thumb `BX PC` now has QEMU oracle coverage for the standard
  `BX PC; NOP; .arm` interworking idiom
- Thumb long `BL` immediate now has QEMU oracle coverage for Thumb-to-Thumb
  call/return link semantics
- Thumb high-register `MOV PC, Rm` and `ADD PC, Rm` PC-write behavior
- Thumb high-register `MOV PC, Rm` now has QEMU oracle coverage showing the
  Thumb ALU PC write stays in Thumb state and masks bit 0
- Thumb high-register non-PC `ADD`/`CMP`/`MOV` operations now have QEMU oracle
  matrix coverage for low/high and high/high register combinations
- Thumb ALU operations now have QEMU oracle matrix coverage for the 0x4000
  register ALU group, including flag-only `TST`, `CMP`, and `CMN` cases
- Thumb shift/add/sub/immediate formats now have QEMU oracle matrix coverage
  for immediate shifts, register/immediate add-subtract, and
  `MOV`/`CMP`/`ADD`/`SUB` immediate operations
- Thumb literal load, PC-relative add, SP-relative add, and SP adjust forms now
  have QEMU oracle coverage
- Thumb ARMv6 `SXTH`, `SXTB`, `UXTH`, `UXTB`, `REV`, `REV16`, and `REVSH`
  extension instructions now have QEMU oracle coverage
- Thumb conditional and unconditional branches now have QEMU oracle coverage for
  taken/not-taken condition evaluation plus forward and backward offsets
- ARM/Thumb core ALU, shifts, compares, flag-setting, and common load/store
- ARM data-processing register/shift forms now have deterministic
  pseudo-random QEMU oracle coverage across all 16 opcode slots, including
  result and `NZCV` folding
- ARM data-processing rotated-immediate forms now have deterministic QEMU
  oracle coverage across all 16 opcode slots, including immediate expansion,
  result, and `NZCV` folding
- ARM data-processing register-shifted-register invalid `PC` forms trap
  explicitly
- ARM halfword/signed/doubleword transfers: `LDRH`, `STRH`, `LDRSB`, `LDRSH`,
  `LDRD`, `STRD`
- ARM halfword/signed/doubleword transfers now have QEMU oracle matrix coverage
  for immediate/register offsets, signed byte/halfword loads, doubleword
  load/store, up/down addressing, and writeback
- ARM single word/byte transfers now have QEMU oracle matrix coverage for
  immediate up/down offsets, post-index writeback, pre-index writeback, byte
  transfers, and shifted register offsets
- ARM `STR pc, [...]` and `STM {..., pc}` stores now have QEMU oracle coverage
  for the ARM-state `PC + 8` source value
- ARM `LDR pc, [Rn]` interworking now has QEMU oracle coverage for loading an
  odd Thumb target address
- ARM/Thumb block transfer basics, including explicit traps for empty
  register lists, ARM user-mode/S-bit block-transfer forms, `PC` base forms,
  and `LDM` writeback with base in the register list
- ARM block transfers now have QEMU oracle matrix coverage for IA, IB, DA, and
  DB store addressing with writeback and paired loads from the resulting memory
- ARM `STM` writeback with the base register in the store list now has QEMU
  oracle coverage for storing the old base value before writeback
- ARM load/store invalid-form traps for writeback overlap, invalid `PC`
  byte/halfword/register-offset forms, and doubleword pair/writeback overlap
  cases
- Thumb `LDMIA` suppresses writeback when the base register is in the load
  list, matching ARMv6/QEMU behavior
- Thumb load/store forms now have QEMU oracle matrix coverage for immediate
  word/byte/halfword transfers, register-offset word/byte/halfword/signed
  transfers, and SP-relative transfers
- Thumb `POP {pc}` interworking now has QEMU oracle coverage for loading an
  ARM target address from the stack
- Thumb-1 invalid high-register `ADD/CMP`, empty `PUSH/POP`, and `STMIA`
  base-in-list forms trap explicitly
- ARMv5TE and ARMv6 multiply/DSP families listed in `docs/armv6_status.md`,
  with QEMU oracle coverage for base multiply, long multiply, dual 16-bit
  multiply, dual-long multiply, high-word multiply, and `UMAAL` variants, plus
  direct traps for invalid `PC` register forms
- Base ARM multiply and long multiply now have QEMU oracle matrix coverage for
  `MUL{S}`, `MLA{S}`, `UMULL{S}`, `UMLAL{S}`, `SMULL{S}`, `SMLAL{S}`, and
  multi-case `UMAAL`
- ARMv5TE signed halfword multiply now has QEMU oracle matrix coverage for all
  `SMLAxy`, `SMLALxy`, `SMULxy`, `SMLAWy`, and `SMULWy` variants
- ARMv6 dual 16-bit DSP multiply now has QEMU oracle matrix coverage for
  `SMLAD{X}`, `SMLSD{X}`, `SMUAD{X}`, `SMUSD{X}`, `SMLALD{X}`, and
  `SMLSLD{X}` variants
- ARMv6 high-word signed multiply now has QEMU oracle matrix coverage for
  `SMMUL`, `SMMULR`, `SMMLA`, `SMMLAR`, `SMMLS`, and `SMMLSR`
- ARMv6 sum-of-absolute-differences has QEMU oracle coverage for both `USAD8`
  and `USADA8`
- ARMv6 extension, packing, selection, reversal, saturation, parallel add/sub,
  and absolute-difference families listed in `docs/armv6_status.md`, with
  broader QEMU oracle coverage across representative signed, unsigned,
  halving, and saturating parallel add/sub variants, plus shifted scalar
  saturation forms and direct invalid `PC` form traps
- ARMv5/ARMv6 `CLZ`, `REV`, `REV16`, and `REVSH` now have QEMU oracle
  matrix coverage across zero, sign-bit, byte-pattern, and sign-extension cases
- ARMv6 halfword packing now has QEMU oracle shift-matrix coverage for
  `PKHBT` and `PKHTB`, including the `PKHTB ASR #32` encoding
- ARMv6 signed/unsigned extend-add families now have QEMU oracle matrix
  coverage across byte, halfword, dual-byte, add/non-add, and rotation forms
- ARMv6 `SEL` now has QEMU oracle coverage with `GE` flags generated by real
  parallel media instructions
- ARMv6 parallel add/sub families now also have a full 36-variant QEMU oracle
  matrix across signed, unsigned, saturating, and halving
  `*ADD16`/`*ASX`/`*SAX`/`*SUB16`/`*ADD8`/`*SUB8` encodings
- ARMv6 parallel media `GE[3:0]` updates now have QEMU oracle coverage across
  signed and unsigned add/sub/crossed variants
- ARMv6 scalar saturation and saturating arithmetic now have QEMU oracle
  coverage for sticky CPSR `Q` flag updates and non-saturating clear cases
- ARMv6 synchronization basics: `SWP`, `SWPB`, `LDREX*`, `STREX*`, `CLREX`,
  with explicit traps for invalid `PC`, `SWP` base overlap,
  doubleword-pair, and status-register overlap forms
- ARMv6 exclusive word operations now have QEMU oracle coverage for both
  successful `STREX` and failed `STREX` after `CLREX`
- ARMv6 exclusive byte, halfword, and doubleword operations now have QEMU
  oracle coverage for success and failed store-after-`CLREX` paths
- ARM conditional execution around exclusives now has QEMU oracle coverage
  showing a skipped conditional `STREX` preserves the monitor for a later store
- ARM/Thumb no-op/control/hint handling listed in `docs/armv6_status.md`
- Explicit user-mode privileged traps for `CPS`, `RFE`, and `SRS`
- Non-baseline ARMv6T2/ARMv7 A32 encodings for `MOVW`, `MOVT`, bitfield
  operations, `RBIT`, and integer divide trap as undefined instead of being
  misdecoded as ARMv6 data-processing instructions
- Status register access for user-mode APSR/CPSR flags, with privileged traps
  for SPSR and CPSR control-field writes, plus invalid `PC` form traps
- Explicit traps for data-processing exception-return forms that write `PC`
  with the `S` bit set
- ARM/Thumb `SVC`, `BKPT`, and `UDF` trap paths
- Explicit unpredictable traps for invalid VFP core-register `PC` forms
- VFPv2 arithmetic, compare, conversion, `VCVTR` FPSCR-rounded conversion,
  move, `VMOV.32` double-lane, and load/store subset listed in
  `docs/armv6_status.md`
- VFPv2 double-precision arithmetic now has QEMU oracle matrix coverage for
  add, subtract, multiply, divide, negate, absolute value, multiply-add, and
  square root
- VFPv2 single-precision arithmetic now has QEMU oracle matrix coverage for
  add, subtract, multiply, divide, negate, absolute value, multiply-add,
  multiply-subtract, negative multiply accumulate/subtract, negative multiply,
  and square root
- VFPv2 single/double load/store and multiple-transfer forms now have QEMU
  oracle coverage for scalar `VLDR`/`VSTR`, `VLDMIA`/`VSTMIA`, and double
  writeback transfers
- VFPv2 double multiple-transfer odd-immediate writeback behavior now has
  explicit QEMU oracle coverage
- VFPv2 double-conversion forms now have QEMU oracle coverage for
  `VCVT.F64.F32`, `VCVT.F32.F64`, double-to-integer, FPSCR-rounded
  double-to-integer, and integer-to-double paths
- VFP compare now has QEMU oracle matrix coverage for single and double
  precision less/equal/greater/unordered cases plus compare-with-zero forms
- VFPv2 FPSCR system-register moves remain supported, `VMRS FPSID` now returns
  an ARM1136-style VFPv2 ID value, `VMSR FPSID` is ignored, and user-mode
  `FPEXC`/`FPINST` accesses trap explicitly
- VFPv2 load/store invalid ranges, empty multiple-transfer lists, and
  writeback with `PC` as base trap explicitly
- VFPv2 double-register encodings that select D16-D31 now trap explicitly
  across core-register moves, `VMOV.32`, arithmetic, compare, and conversion
  paths instead of indexing beyond the modeled VFPv2 D0-D15 register file
- FPSCR VFP short-vector `LEN`/`STRIDE` modes now execute for vectorizable
  VFPv2 arithmetic and unary operations, with unit coverage for stride-1,
  stride-2, scalar source replication, double precision, and invalid
  length/stride traps, plus QEMU oracle coverage for the stride-1 path
- VFP compare and conversion paths now have unit and QEMU oracle coverage
  showing they remain scalar when FPSCR `LEN` is nonzero, matching the
  documented VFP scalar-only operation class
- VFPv3-only immediate moves and fixed-point conversion encodings now have
  direct undefined-trap coverage to keep the ARMv6/VFPv2 boundary explicit
- CP15 user thread ID shim: user `MRC` reads for `TPIDRURW`/`TPIDRURO`, user
  `MCR` writes for `TPIDRURW`, privileged traps for user `TPIDRURO` writes,
  explicit traps for invalid `PC` source/destination forms, and QEMU oracle
  coverage for the `TPIDRURW` roundtrip path
- CP15 barrier HLE no-ops for ARMv6-style DMB, DSB, and ISB/prefetch-flush
  `MCR` forms

## Known Incomplete Or Weakly Verified Areas

- Full ARMv6 instruction-by-instruction audit against the ARM ARM is not done.
- Privileged/system behavior is not fully modeled beyond user-mode HLE stubs
  and explicit traps; broad CP15 behavior remains unsupported beyond TLS and
  barrier idioms.
- General coprocessor instructions are not implemented beyond VFP and CP15
  TLS/barrier shims.
- VFP FPSCR exception flags and uncommon edge cases remain simplified or
  missing; VFPv3 fixed-point conversions remain outside the ARMv6/VFPv2
  baseline.
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
