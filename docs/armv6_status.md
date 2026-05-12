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
  `STRBT`, and ARM-state `PC + 8` source values for `STR pc, [...]`
- ARM halfword/signed/doubleword load/store forms: `LDRH`, `STRH`,
  `LDRSB`, `LDRSH`, `LDRD`, `STRD`
- ARM block transfer: `LDM`, `STM`, including ARM-state `PC + 8` source values
  for `STM {..., pc}` and old-base stores for `STM` writeback when the base
  register is in the store list
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
  doubleword-pair, and status-register overlap forms; normal guest stores
  that overlap the reserved byte range clear the local exclusive reservation
- ARM no-op/hint/system handling: `PLD`, ARM hint encodings, `SETEND LE`,
  and explicit privileged traps for `CPS`, `RFE`, and `SRS`
- Explicit undefined traps for non-baseline ARMv6T2/ARMv7 A32 encodings that
  otherwise overlap ARM data-processing space: `MOVW`, `MOVT`, bitfield
  extract/insert/clear, `RBIT`, `SDIV`, and `UDIV`
- Explicit exception/control traps: ARM/Thumb `SVC`, ARM/Thumb `BKPT`,
  ARM/Thumb `UDF`
- CP15 user-thread registers: user `MRC` reads for `TPIDRURW`/`TPIDRURO`,
  user `MCR` writes for `TPIDRURW`, privileged traps for user `TPIDRURO`
  writes, and explicit unpredictable traps for invalid `PC`
  source/destination forms
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
- FPSCR VFP short-vector `LEN`/`STRIDE` support for vectorizable VFPv2
  arithmetic and unary operations, including scalar destination/source-bank
  handling and invalid vector length/stride traps
- Basic cumulative FPSCR exception flags for VFPv2 divide-by-zero, invalid
  square-root/divide, invalid float-to-integer conversions, and selected
  arithmetic cases: `DZC` for finite nonzero divide by zero and `IOC` for
  negative square root, zero divided by zero, invalid single-precision
  arithmetic, NaN conversion, out-of-range conversion, and negative unsigned
  conversion; `IXC` for inexact single-precision arithmetic, inexact
  float-to-integer conversions that round without raising `IOC`, inexact
  integer-to-single conversions, and double-to-single narrowing; `OFC`/`UFC`
  for overflowing or underflowing single-precision arithmetic and
  double-to-single narrowing
- VFP status/compare subset: single and double `VCMP`/`VCMPE`, compare with
  zero, `VMRS`/`VMSR FPSCR`, `VMRS FPSID`, ignored `VMSR FPSID`, and explicit
  privileged traps for user-mode `FPEXC`/`FPINST` accesses; compare remains
  scalar when FPSCR short-vector `LEN` is nonzero and raises `IOC` for
  signaling-NaN `VCMP` or any-NaN `VCMPE`
- VFP conversion subset: `VCVT` between `F32`, `F64`, `S32`, and `U32`,
  plus `VCVTR` float-to-integer rounding through FPSCR rounding mode;
  conversion remains scalar when FPSCR short-vector `LEN` is nonzero
- Explicit unpredictable traps for invalid VFP core-register forms that use
  `PC`, including single-register `VMOV` and `VMSR FPSCR, PC`
- Explicit undefined traps for non-baseline VFPv3 immediate moves and
  fixed-point conversion encodings that are outside ARMv6/VFPv2
- Thumb-1 common instruction set: shifts, ALU ops, high-register ops,
  literal loads, load/store forms, push/pop, multiple load/store,
  including `LDMIA` base-in-list writeback suppression,
  conditional/unconditional branches, `BL`, `BLX`, `BX`, `SWI`/`SVC`
- Thumb high-register writes to `PC`, including `MOV PC, Rm` and
  `ADD PC, Rm`, keep Thumb state and mask bit 0; Thumb `BX`/`BLX` perform
  branch-exchange interworking, including the `BX PC; NOP; .arm` idiom
- Explicit unpredictable traps for invalid Thumb-1 high-register `ADD/CMP`,
  empty `PUSH/POP`, and `STMIA` base-in-list forms
- Thumb ARMv6 extensions: `SXTH`, `SXTB`, `UXTH`, `UXTB`, `REV`, `REV16`,
  `REVSH`, `BKPT`, `SETEND LE`, plus an explicit privileged trap for `CPS`

## Known Gaps

- Full ARMv6 media/DSP coverage outside the fully-oracled parallel add/sub,
  ARMv5TE signed-halfword multiply, dual 16-bit DSP multiply, high-word signed
  multiply, and absolute-difference cases has not been audited
  instruction-by-instruction against the ARM ARM.
- Full VFP/VFPv2 is not implemented; FPSCR exception flags remain incomplete
  beyond basic `IOC`/`DZC` divide, square-root, compare-NaN, selected
  single-precision arithmetic, and conversion invalid cases, plus basic
  conversion `IXC`/`OFC`/`UFC` cases; double-precision arithmetic flags and
  less common arithmetic/conversion edge cases still need an
  instruction-by-instruction audit. VFPv3 fixed-point conversions are outside
  the ARMv6/VFPv2 baseline.
- General coprocessor instructions are not implemented beyond the CP15
  user-thread/barrier shims and VFP paths listed above.
- Thumb-2 is intentionally not implemented for the ARMv6 baseline, but
  ARMv6T2 targets would need it.
- Exception, signal, and privileged/system behavior is only stubbed or trapped
  enough for user-mode HLE work.
- Exclusive monitor behavior is still a single-core approximation and does not
  model multiprocessor/global monitor effects.
- Some unpredictable edge cases are simplified.
- QEMU oracle coverage is still family-focused; no broad randomized
  differential suite exists yet.

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
