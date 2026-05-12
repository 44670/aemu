# ARMv6 Interpreter Status

This file tracks implemented interpreter coverage. Passing tests do not mean the
ARMv6 goal is complete; this is a working checklist for the remaining CPU work.

## Implemented

- CPU register file, CPSR `N/Z/C/V/Q/GE/T`, ARM/Thumb mode switching
- Checked little-endian byte/halfword/word memory
- ARM condition codes
- ARM branches/interworking: `B`, `BL`, `BX`, `BXJ` as `BX`,
  `BLX <reg>`, `BLX <imm>`, plus branch-exchange behavior for ARM/Thumb
  writes to `PC`
- ARM data processing: `AND`, `EOR`, `SUB`, `RSB`, `ADD`, `ADC`, `SBC`,
  `RSC`, `TST`, `TEQ`, `CMP`, `CMN`, `ORR`, `MOV`, `BIC`, `MVN`
- Explicit unpredictable traps for data-processing register-shifted-register
  forms that use `PC` as an operand, shift register, or destination
- ARM multiply: `MUL`, `MLA`, `UMULL`, `UMLAL`, `SMULL`, `SMLAL`
- ARMv5TE signed halfword multiply/accumulate: `SMLAxy`, `SMLALxy`,
  `SMULxy`, `SMLAWy`, `SMULWy`
- ARMv6 DSP multiply families: `SMLAD`, `SMLADX`, `SMLSD`, `SMLSDX`,
  `SMUAD`, `SMUADX`, `SMUSD`, `SMUSDX`, `SMLALD`, `SMLALDX`, `SMLSLD`,
  `SMLSLDX`, `SMMUL`, `SMMULR`, `SMMLA`, `SMMLAR`, `SMMLS`, `SMMLSR`,
  `UMAAL`
- Explicit unpredictable traps for invalid `PC` multiply/DSP register forms,
  while preserving accumulator-as-`PC` no-accumulate encodings where valid
- ARMv6 unsigned sum of absolute differences: `USAD8`, `USADA8`
- ARM load/store word and byte forms, including `LDRT`, `STRT`, `LDRBT`,
  and `STRBT`
- ARM halfword/signed/doubleword load/store forms: `LDRH`, `STRH`,
  `LDRSB`, `LDRSH`, `LDRD`, `STRD`
- ARM block transfer: `LDM`, `STM`
- Explicit unpredictable traps for invalid ARM load/store register forms:
  writeback overlap, invalid `PC` byte/halfword/register-offset forms,
  doubleword pair/writeback overlap cases, `LDM` writeback with base in the
  register list, and block-transfer `PC` base forms
- ARM status register access: `MRS CPSR`, `MSR APSR_nzcvq`,
  `MSR APSR_nzcvqg`, and explicit privileged traps for SPSR/control-field
  access, plus invalid `PC` source/destination traps
- Explicit privileged traps for data-processing exception-return forms that
  write `PC` with the `S` bit set
- ARM synchronization/swap subset: `SWP`, `SWPB`, `LDREX`, `STREX`,
  `LDREXB`, `LDREXH`, `LDREXD`, `STREXB`, `STREXH`, `STREXD`, `CLREX`
  with explicit traps for invalid `PC`, `SWP` base overlap,
  doubleword-pair, and status-register overlap forms
- ARM no-op/hint/system handling: `PLD`, ARM hint encodings, `SETEND LE`,
  and explicit privileged traps for `CPS`, `RFE`, and `SRS`
- Explicit undefined traps for non-baseline ARMv6T2/ARMv7 A32 encodings that
  otherwise overlap ARM data-processing space: `MOVW`, `MOVT`, bitfield
  extract/insert/clear, `RBIT`, `SDIV`, and `UDIV`
- Explicit exception/control traps: ARM/Thumb `SVC`, ARM/Thumb `BKPT`,
  ARM/Thumb `UDF`
- CP15 user-thread registers: `MRC`/`MCR` for `TPIDRURW` and `TPIDRURO`,
  with explicit unpredictable traps for invalid `PC` source/destination forms
- CP15 barrier idioms: user-mode HLE no-ops for ARMv6-style DMB, DSB, and
  ISB/prefetch-flush `MCR` forms
- ARMv5/ARMv6 misc integer: `CLZ`, `REV`, `REV16`, `REVSH`
- ARMv6 extension/saturation: `SXTB`, `SXTH`, `UXTB`, `UXTH`, `QADD`,
  `QSUB`, `QDADD`, `QDSUB`, `SSAT`, `SSAT16`, `USAT`, `USAT16`
- ARMv6 extend-and-add: `SXTAB`, `SXTAB16`, `SXTAH`, `SXTB16`, `UXTAB`,
  `UXTAB16`, `UXTAH`, `UXTB16`
- ARMv6 packing/selection: `PKHBT`, `PKHTB`, `SEL`
- ARMv6 parallel add/sub families: signed, unsigned, saturating, and halving
  `*ADD16`, `*ASX`, `*SAX`, `*SUB16`, `*ADD8`, `*SUB8`
- Explicit unpredictable traps for invalid `PC` forms across implemented
  ARMv6 misc, extension, saturation, packing, selection, and parallel media
  instructions
- VFPv2 subset: `VMOV` between ARM core and `S`/`D` registers, including
  `VMOV.32` low/high double-register lane moves, register `VMOV`, single and
  double `VADD`, `VSUB`, `VMUL`, `VNMUL`, `VMLA`, `VMLS`, `VNMLA`, `VNMLS`,
  `VDIV`, `VABS`, `VNEG`, `VSQRT`, plus `VLDR`/`VSTR` and
  `VLDM`/`VSTM`/`VPUSH`/`VPOP` forms
- Explicit unpredictable traps for invalid VFPv2 load/store register ranges,
  empty VFP multiple-transfer lists, and writeback with `PC` as the base
- Explicit unpredictable traps for VFPv2 double-register encodings that select
  D16-D31 across core-register moves, `VMOV.32`, arithmetic, compare, and
  conversion paths
- Explicit unpredictable traps for unsupported FPSCR VFP short-vector
  `LEN`/`STRIDE` modes on vectorizable VFP arithmetic and unary operations
- VFP status/compare subset: single and double `VCMP`, compare with zero,
  `VMRS`/`VMSR FPSCR`, `VMRS FPSID`, ignored `VMSR FPSID`, and explicit
  privileged traps for user-mode `FPEXC`/`FPINST` accesses
- VFP conversion subset: `VCVT` between `F32`, `F64`, `S32`, and `U32`,
  plus `VCVTR` float-to-integer rounding through FPSCR rounding mode
- Explicit unpredictable traps for invalid VFP core-register forms that use
  `PC`, including single-register `VMOV` and `VMSR FPSCR, PC`
- Thumb-1 common instruction set: shifts, ALU ops, high-register ops,
  literal loads, load/store forms, push/pop, multiple load/store,
  including `LDMIA` base-in-list writeback suppression,
  conditional/unconditional branches, `BL`, `BLX`, `BX`, `SWI`/`SVC`
- Thumb high-register writes to `PC`, including `MOV PC, Rm` and
  `ADD PC, Rm`, use branch-exchange interworking semantics
- Explicit unpredictable traps for invalid Thumb-1 high-register `ADD/CMP`,
  empty `PUSH/POP`, and `STMIA` base-in-list forms
- Thumb ARMv6 extensions: `SXTH`, `SXTB`, `UXTH`, `UXTB`, `REV`, `REV16`,
  `REVSH`, `BKPT`, `SETEND LE`, plus an explicit privileged trap for `CPS`

## Known Gaps

- Full ARMv6 media/DSP coverage outside the fully-oracled parallel add/sub and
  ARMv5TE signed-halfword multiply matrices has not been audited
  instruction-by-instruction against the ARM ARM.
- Full VFP/VFPv2 is not implemented; FPSCR exception flags, vector-stride
  emulation, fixed-point conversions, and several less common conversion/move
  forms still need an instruction-by-instruction audit.
- General coprocessor instructions are not implemented beyond the CP15
  user-thread/barrier shims and VFP paths listed above.
- Thumb-2 is intentionally not implemented for the ARMv6 baseline, but
  ARMv6T2 targets would need it.
- Exception, signal, and privileged/system behavior is only stubbed or trapped
  enough for user-mode HLE work.
- Exclusive monitor behavior is approximate and single-address only.
- Some unpredictable edge cases are simplified.
- Only small QEMU oracle smoke tests exist; no broad randomized differential
  suite exists yet.

## Minecraft PE Verification Status

The only local APK currently found is:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

It contains only `lib/armeabi-v7a/*.so`. Its `libminecraftpe.so` probes as ARM
v7, Thumb-2, VFPv3, and NEON, so it is outside the ARMv6/`armeabi` baseline.
A broader search under `/mnt/hgfs/deb13` also found no older Minecraft PE APK
and no standalone `libminecraftpe.so`.

An older Minecraft PE APK with `lib/armeabi/libminecraftpe.so` is still needed
for true ARMv6 Minecraft PE verification.
