use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aemu::armv7a::{Cpu, Isa, Memory};
use aemu::guest_memory::MappedMemory;

const DREG_COUNT: usize = 8;
const PROBE_WORDS: usize = 4;
const RESULT_WORDS: usize = 4 + 1 + 1 + (DREG_COUNT * 2) + PROBE_WORDS;
const RESULT_BYTES: usize = RESULT_WORDS * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestIsa {
    Arm,
    Thumb,
}

#[derive(Debug, Clone)]
struct OracleCase {
    name: &'static str,
    isa: GuestIsa,
    setup_asm: &'static str,
    body_asm: &'static str,
    data_asm: &'static str,
    setup_aemu: fn(&mut Cpu, &mut MappedMemory, &BTreeMap<String, u32>),
    compare: CompareMask,
    coverage: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct CompareMask {
    regs: [bool; 4],
    cpsr_nzcvq: bool,
    fpscr: bool,
    dregs: [bool; DREG_COUNT],
    probe_words: [bool; PROBE_WORDS],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OracleResult {
    regs: [u32; 4],
    cpsr: u32,
    fpscr: u32,
    dregs: [u64; DREG_COUNT],
    probe_words: [u32; PROBE_WORDS],
}

#[derive(Debug, Clone)]
struct AemuRun {
    result: OracleResult,
    trace: Vec<StepTrace>,
}

#[derive(Debug, Clone)]
struct StepTrace {
    step: usize,
    pc: u32,
    isa: Isa,
}

#[test]
fn armv7a_qemu_user_oracle_cases_match_aemu() {
    let cases = oracle_cases();
    assert_coverage_manifest(&cases);
    let artifact_root = PathBuf::from("target/armv7a-oracle");
    fs::create_dir_all(&artifact_root).unwrap();

    for case in cases {
        let artifact_dir = artifact_root.join(case.name);
        fs::create_dir_all(&artifact_dir).unwrap();
        let built = build_case(&case, &artifact_dir);
        let qemu = run_qemu(&built.elf);
        let aemu = run_aemu(&case, &built);
        let mismatch = first_mismatch(&case, &qemu, &aemu.result);
        write_replay_artifacts(
            &case,
            &built,
            &qemu,
            &aemu,
            mismatch.as_deref(),
            &artifact_dir,
        );
        if let Some(mismatch) = mismatch {
            panic!(
                "{} diverged at first compared output field: {mismatch}; artifacts in {}",
                case.name,
                artifact_dir.display()
            );
        }
    }
}

fn oracle_cases() -> Vec<OracleCase> {
    vec![
        OracleCase {
            name: "arm_adds_flags",
            isa: GuestIsa::Arm,
            setup_asm: "
                mov r0, #0x7f
                mov r1, #1
                lsl r0, r0, #24
            ",
            body_asm: "
                adds r0, r0, r1
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, 0x7f00_0000);
                cpu.set_reg(1, 1);
            },
            compare: compare_mask(&[0], true, false, &[], &[]),
            coverage: &["ARM", "integer-alu", "flags"],
        },
        OracleCase {
            name: "arm_umull",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r2, =0xfffffff1
                mov r3, #0x21
            ",
            body_asm: "
                umull r0, r1, r2, r3
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(2, 0xffff_fff1);
                cpu.set_reg(3, 0x21);
            },
            compare: compare_mask(&[0, 1], false, false, &[], &[]),
            coverage: &["ARM", "multiply", "long-multiply", "OpenSSL-arithmetic"],
        },
        OracleCase {
            name: "arm_openssl_mls_adc",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r0, =0xfedcba98
                ldr r1, =0x13579bdf
                ldr r2, =0x2468ace0
            ",
            body_asm: "
                umull r4, r5, r0, r1
                mls r2, r0, r1, r2
                adds r0, r4, r2
                adc r1, r5, #0
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, 0xfedc_ba98);
                cpu.set_reg(1, 0x1357_9bdf);
                cpu.set_reg(2, 0x2468_ace0);
            },
            compare: compare_mask(&[0, 1, 2], true, false, &[], &[]),
            coverage: &[
                "ARM",
                "integer-alu",
                "flags",
                "multiply",
                "OpenSSL-arithmetic",
            ],
        },
        OracleCase {
            name: "arm_openssl_umlal_adc",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r0, =0xfedcba98
                ldr r1, =0x13579bdf
                ldr r2, =0x89abcdef
                ldr r3, =0x01234567
                ldr r4, =0x76543210
            ",
            body_asm: "
                umlal r2, r3, r0, r1
                adds r0, r2, r4
                adc r1, r3, #0
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, 0xfedc_ba98);
                cpu.set_reg(1, 0x1357_9bdf);
                cpu.set_reg(2, 0x89ab_cdef);
                cpu.set_reg(3, 0x0123_4567);
                cpu.set_reg(4, 0x7654_3210);
            },
            compare: compare_mask(&[0, 1, 2, 3], true, false, &[], &[]),
            coverage: &[
                "ARM",
                "integer-alu",
                "flags",
                "multiply",
                "long-multiply",
                "OpenSSL-arithmetic",
            ],
        },
        OracleCase {
            name: "arm_bitops_rev_ubfx",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r0, =0x01234567
                ldr r1, =0xf0f0aa55
            ",
            body_asm: "
                rev r0, r0
                ubfx r1, r1, #4, #12
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, 0x0123_4567);
                cpu.set_reg(1, 0xf0f0_aa55);
            },
            compare: compare_mask(&[0, 1], false, false, &[], &[]),
            coverage: &["ARM", "bit-operations"],
        },
        OracleCase {
            name: "arm_interworking_thumb_return",
            isa: GuestIsa::Arm,
            setup_asm: "
                mov r0, #10
            ",
            body_asm: "
                mov r12, lr
                add r0, r0, #3
                adr r1, thumb_target
                orr r1, r1, #1
                blx r1
                add r0, r0, #7
                mov lr, r12
                bx lr
                .thumb
                .thumb_func
            thumb_target:
                adds r0, #5
                bx lr
                .arm
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, 10);
            },
            compare: compare_mask(&[0], false, false, &[], &[]),
            coverage: &["ARM", "Thumb-1", "branch", "interworking"],
        },
        OracleCase {
            name: "arm_load_store_writeback",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r0, =oracle_probe
                ldr r1, =0x11223344
            ",
            body_asm: "
                str r1, [r0, #4]!
                ldr r2, [r0], #-4
                str r2, [r0, #8]
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_reg(0, symbol(symbols, "oracle_probe"));
                cpu.set_reg(1, 0x1122_3344);
            },
            compare: compare_mask(&[0, 2], false, false, &[], &[1, 2]),
            coverage: &["ARM", "load-store"],
        },
        OracleCase {
            name: "thumb2_it_fallthrough_literal_ldr",
            isa: GuestIsa::Thumb,
            setup_asm: "
                ldr r0, =oracle_dst
                ldr r4, =oracle_src
            ",
            body_asm: "
                movs r2, #0
            1:
                adds r3, r0, r2
                itt ne
                ldrne r3, [r4, r2]
                strne r3, [r0, r2]
                adds r2, #4
                cmp.w r2, #0x400
                bne 1b
                ldr r2, =oracle_value
                ldr r0, [r2]
                ldr r3, =oracle_dst
                ldr r3, [r3, #0x3fc]
                ldr r2, =oracle_probe
                str r3, [r2]
                bx lr
                .ltorg
            ",
            data_asm: "
                .balign 4
                .global oracle_src
            oracle_src:
                .rept 256
                .word 0x11223344
                .endr
                .global oracle_dst
            oracle_dst:
                .space 1024
                .global oracle_value
            oracle_value:
                .word 0x55667788
            ",
            setup_aemu: |cpu, mem, symbols| {
                cpu.set_isa(Isa::Thumb);
                let src = symbol(symbols, "oracle_src");
                let dst = symbol(symbols, "oracle_dst");
                let value = symbol(symbols, "oracle_value");
                map_symbol_region(mem, src, 0x400);
                map_symbol_region(mem, dst, 0x400);
                map_symbol_region(mem, value, 4);
                for offset in (0..0x400).step_by(4) {
                    mem.store32(src + offset, 0x1122_3344).unwrap();
                }
                mem.store32(value, 0x5566_7788).unwrap();
                cpu.set_reg(0, dst);
                cpu.set_reg(4, src);
            },
            compare: compare_mask(&[0, 2], true, false, &[], &[0]),
            coverage: &["Thumb-2", "IT", "load-store", "branch", "MCPE-regression"],
        },
        OracleCase {
            name: "thumb2_localization_it_highreg_loop",
            isa: GuestIsa::Thumb,
            setup_asm: "
                ldr r4, =oracle_probe
                movs r0, #0
                movs r1, #0
                movs r2, #5
                str r2, [r4, #4]
                movs r2, #7
                str r2, [r4, #8]
                movs r2, #11
                str r2, [r4, #12]
                movw r8, #0
                movw r10, #4
            ",
            body_asm: "
            1:
                cmp.w r8, #0
                itt ne
                ldrne.w r1, [r4, r8, lsl #2]
                addne.w r0, r0, r1
                add.w r8, r8, #1
                cmp.w r8, r10
                blt 1b
                str.w r0, [r4]
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, mem, symbols| {
                cpu.set_isa(Isa::Thumb);
                let probe = symbol(symbols, "oracle_probe");
                cpu.set_reg(4, probe);
                cpu.set_reg(8, 0);
                cpu.set_reg(10, 4);
                for (idx, value) in [0, 5, 7, 11].into_iter().enumerate() {
                    mem.store32(probe + (idx as u32 * 4), value).unwrap();
                }
            },
            compare: compare_mask(&[0, 1], true, false, &[], &[0, 1, 2, 3]),
            coverage: &[
                "Thumb-2",
                "IT",
                "branch",
                "integer-alu",
                "load-store",
                "MCPE-regression",
                "localization-hot-loop",
            ],
        },
        OracleCase {
            name: "thumb1_cond_branch_store",
            isa: GuestIsa::Thumb,
            setup_asm: "
                movs r0, #0
                ldr r1, =oracle_probe
            ",
            body_asm: "
                cmp r0, #0
                beq 1f
                movs r2, #1
                b 2f
            1:
                movs r2, #42
            2:
                str r2, [r1]
                bx lr
            ",
            data_asm: "",
            setup_aemu: |cpu, _mem, symbols| {
                cpu.set_isa(Isa::Thumb);
                cpu.set_reg(0, 0);
                cpu.set_reg(1, symbol(symbols, "oracle_probe"));
            },
            compare: compare_mask(&[2], false, false, &[], &[0]),
            coverage: &["Thumb-1", "branch", "load-store"],
        },
        OracleCase {
            name: "arm_vfpv3_f32_arithmetic",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r4, =vfp_inputs
                vldr s0, [r4]
                vldr s1, [r4, #4]
            ",
            body_asm: "
                vadd.f32 s2, s0, s1
                vmul.f32 s3, s2, s1
                bx lr
            ",
            data_asm: "
                .balign 4
            vfp_inputs:
                .word 0x3fc00000
                .word 0x40200000
            ",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_sreg(0, 0x3fc0_0000);
                cpu.set_sreg(1, 0x4020_0000);
            },
            compare: compare_mask(&[], false, true, &[1], &[]),
            coverage: &["ARM", "VFPv3"],
        },
        OracleCase {
            name: "arm_neon_i32_add_xor",
            isa: GuestIsa::Arm,
            setup_asm: "
                ldr r4, =neon_inputs
                vld1.32 {d0}, [r4]!
                vld1.32 {d1}, [r4]
            ",
            body_asm: "
                vadd.i32 d2, d0, d1
                veor d3, d2, d1
                bx lr
            ",
            data_asm: "
                .balign 8
            neon_inputs:
                .word 0x00000001
                .word 0x7fffffff
                .word 0xffffffff
                .word 0x00000002
            ",
            setup_aemu: |cpu, _mem, _symbols| {
                cpu.set_isa(Isa::Arm);
                cpu.set_dreg(0, 0x7fff_ffff_0000_0001);
                cpu.set_dreg(1, 0x0000_0002_ffff_ffff);
            },
            compare: compare_mask(&[], false, false, &[2, 3], &[]),
            coverage: &["ARM", "NEON", "target-driven-NEON"],
        },
    ]
}

struct BuiltCase {
    elf: PathBuf,
    testcase_bin: PathBuf,
    disasm: PathBuf,
    testcase_bytes: Vec<u8>,
    symbols: BTreeMap<String, u32>,
}

fn build_case(case: &OracleCase, artifact_dir: &Path) -> BuiltCase {
    require_tool("clang");
    require_tool("llvm-objcopy");
    require_tool("llvm-nm");
    require_tool("llvm-objdump");
    let asm = render_case_asm(case);
    let asm_path = artifact_dir.join(format!("{}.s", case.name));
    let elf_path = artifact_dir.join(case.name);
    let bin_path = artifact_dir.join(format!("{}.testcase.bin", case.name));
    let disasm_path = artifact_dir.join(format!("{}.disasm", case.name));
    fs::write(&asm_path, asm).unwrap();

    let clang = Command::new("clang")
        .args([
            "--target=armv7a-linux-gnueabi",
            "-nostdlib",
            "-static",
            "-fuse-ld=lld",
            "-Wl,-Ttext=0x10000",
            "-o",
        ])
        .arg(&elf_path)
        .arg(&asm_path)
        .output()
        .unwrap();
    assert!(
        clang.status.success(),
        "clang failed for {}:\n{}",
        case.name,
        String::from_utf8_lossy(&clang.stderr)
    );

    let objcopy = Command::new("llvm-objcopy")
        .args(["-O", "binary", "--only-section=.testcase"])
        .arg(&elf_path)
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(
        objcopy.status.success(),
        "llvm-objcopy failed for {}:\n{}",
        case.name,
        String::from_utf8_lossy(&objcopy.stderr)
    );

    let testcase_bytes = fs::read(&bin_path).unwrap();
    assert!(
        !testcase_bytes.is_empty(),
        "{} has empty testcase section",
        case.name
    );
    let objdump = Command::new("llvm-objdump")
        .args(["-d"])
        .arg(&elf_path)
        .output()
        .unwrap();
    assert!(
        objdump.status.success(),
        "llvm-objdump failed for {}:\n{}",
        case.name,
        String::from_utf8_lossy(&objdump.stderr)
    );
    fs::write(&disasm_path, objdump.stdout).unwrap();
    let symbols = read_symbols(&elf_path);
    BuiltCase {
        elf: elf_path,
        testcase_bin: bin_path,
        disasm: disasm_path,
        testcase_bytes,
        symbols,
    }
}

fn render_case_asm(case: &OracleCase) -> String {
    let mode = match case.isa {
        GuestIsa::Arm => ".arm",
        GuestIsa::Thumb => ".thumb\n.thumb_func",
    };
    let call = match case.isa {
        GuestIsa::Arm => "bl case_start",
        GuestIsa::Thumb => "blx case_start",
    };
    format!(
        r#"
.syntax unified
.arch armv7-a
.fpu neon
.text
.global _start
_start:
{setup}
    {call}
    ldr r4, =oracle_result
    str r0, [r4, #0]
    str r1, [r4, #4]
    str r2, [r4, #8]
    str r3, [r4, #12]
    mrs r5, apsr
    str r5, [r4, #16]
    vmrs r5, fpscr
    str r5, [r4, #20]
    vstr d0, [r4, #24]
    vstr d1, [r4, #32]
    vstr d2, [r4, #40]
    vstr d3, [r4, #48]
    vstr d4, [r4, #56]
    vstr d5, [r4, #64]
    vstr d6, [r4, #72]
    vstr d7, [r4, #80]
    ldr r5, =oracle_probe
    ldr r5, [r5]
    str r5, [r4, #88]
    ldr r5, =oracle_probe
    ldr r5, [r5, #4]
    str r5, [r4, #92]
    ldr r5, =oracle_probe
    ldr r5, [r5, #8]
    str r5, [r4, #96]
    ldr r5, =oracle_probe
    ldr r5, [r5, #12]
    str r5, [r4, #100]
    mov r0, #1
    ldr r1, =oracle_result
    mov r2, #{result_bytes}
    mov r7, #4
    svc #0
    mov r0, #0
    mov r7, #1
    svc #0

.section .testcase,"ax"
{mode}
.global case_start
case_start:
{body}

.data
.balign 4
.global oracle_result
oracle_result:
    .space {result_bytes}
.global oracle_probe
oracle_probe:
    .space 16
{data}
"#,
        setup = case.setup_asm,
        call = call,
        mode = mode,
        body = case.body_asm,
        result_bytes = RESULT_BYTES,
        data = case.data_asm,
    )
}

fn run_qemu(elf: &Path) -> OracleResult {
    require_tool("timeout");
    require_tool("qemu-arm");
    let output = Command::new("timeout")
        .arg("10")
        .arg("qemu-arm")
        .arg(elf)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "qemu-arm failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_result_blob(&output.stdout)
}

fn run_aemu(case: &OracleCase, built: &BuiltCase) -> AemuRun {
    let mut cpu = Cpu::new();
    let mut memory = MappedMemory::new();
    let case_start = symbol(&built.symbols, "case_start");
    map_symbol_region(&mut memory, case_start, built.testcase_bytes.len() as u32);
    memory
        .load_bytes(case_start, &built.testcase_bytes)
        .unwrap();
    let probe = symbol(&built.symbols, "oracle_probe");
    map_symbol_region(&mut memory, probe, (PROBE_WORDS * 4) as u32);
    for idx in 0..PROBE_WORDS {
        memory.store32(probe + (idx as u32 * 4), 0).unwrap();
    }
    (case.setup_aemu)(&mut cpu, &mut memory, &built.symbols);
    cpu.set_reg(14, 0xffff_fffc);
    cpu.branch_exchange(match case.isa {
        GuestIsa::Arm => case_start,
        GuestIsa::Thumb => case_start | 1,
    });
    let mut trace = Vec::new();
    for step in 0..20_000 {
        if cpu.pc() == 0xffff_fffc {
            break;
        }
        trace.push(StepTrace {
            step,
            pc: cpu.pc(),
            isa: cpu.isa(),
        });
        cpu.step(&mut memory).unwrap_or_else(|err| {
            panic!(
                "AEMU failed in {} at pc={:#010x} isa={:?}: {err:?}",
                case.name,
                cpu.pc(),
                cpu.isa()
            )
        });
    }
    assert_eq!(cpu.pc(), 0xffff_fffc, "{} did not return", case.name);
    let mut dregs = [0u64; DREG_COUNT];
    for (idx, dreg) in dregs.iter_mut().enumerate() {
        *dreg = cpu.dreg(idx);
    }
    let mut probe_words = [0u32; PROBE_WORDS];
    for (idx, word) in probe_words.iter_mut().enumerate() {
        *word = memory.load32(probe + (idx as u32 * 4)).unwrap();
    }
    AemuRun {
        result: OracleResult {
            regs: [cpu.reg(0), cpu.reg(1), cpu.reg(2), cpu.reg(3)],
            cpsr: cpu.cpsr.to_u32(),
            fpscr: cpu.fpscr,
            dregs,
            probe_words,
        },
        trace,
    }
}

fn first_mismatch(case: &OracleCase, qemu: &OracleResult, aemu: &OracleResult) -> Option<String> {
    for idx in 0..4 {
        if case.compare.regs[idx] && aemu.regs[idx] != qemu.regs[idx] {
            return Some(format!(
                "r{idx}: qemu={:#010x} aemu={:#010x}",
                qemu.regs[idx], aemu.regs[idx]
            ));
        }
    }
    if case.compare.cpsr_nzcvq {
        let mask = 0xf800_0000;
        if (aemu.cpsr & mask) != (qemu.cpsr & mask) {
            return Some(format!(
                "CPSR.NZCVQ: qemu={:#010x} aemu={:#010x}",
                qemu.cpsr, aemu.cpsr
            ));
        }
    }
    if case.compare.fpscr && aemu.fpscr != qemu.fpscr {
        return Some(format!(
            "FPSCR: qemu={:#010x} aemu={:#010x}",
            qemu.fpscr, aemu.fpscr
        ));
    }
    for idx in 0..DREG_COUNT {
        if case.compare.dregs[idx] && aemu.dregs[idx] != qemu.dregs[idx] {
            return Some(format!(
                "d{idx}: qemu={:#018x} aemu={:#018x}",
                qemu.dregs[idx], aemu.dregs[idx]
            ));
        }
    }
    for idx in 0..PROBE_WORDS {
        if case.compare.probe_words[idx] && aemu.probe_words[idx] != qemu.probe_words[idx] {
            return Some(format!(
                "probe[{idx}]: qemu={:#010x} aemu={:#010x}",
                qemu.probe_words[idx], aemu.probe_words[idx]
            ));
        }
    }
    None
}

fn compare_mask(
    regs: &[usize],
    cpsr_nzcvq: bool,
    fpscr: bool,
    dregs: &[usize],
    probe_words: &[usize],
) -> CompareMask {
    let mut mask = CompareMask {
        regs: [false; 4],
        cpsr_nzcvq,
        fpscr,
        dregs: [false; DREG_COUNT],
        probe_words: [false; PROBE_WORDS],
    };
    for &idx in regs {
        mask.regs[idx] = true;
    }
    for &idx in dregs {
        mask.dregs[idx] = true;
    }
    for &idx in probe_words {
        mask.probe_words[idx] = true;
    }
    mask
}

fn parse_result_blob(bytes: &[u8]) -> OracleResult {
    assert_eq!(
        bytes.len(),
        RESULT_WORDS * 4,
        "unexpected qemu result length"
    );
    let mut words = [0u32; RESULT_WORDS];
    for (idx, chunk) in bytes.chunks_exact(4).enumerate() {
        words[idx] = u32::from_le_bytes(chunk.try_into().unwrap());
    }
    let mut dregs = [0u64; DREG_COUNT];
    for idx in 0..DREG_COUNT {
        let lo = words[6 + (idx * 2)] as u64;
        let hi = words[6 + (idx * 2) + 1] as u64;
        dregs[idx] = lo | (hi << 32);
    }
    let mut probe_words = [0u32; PROBE_WORDS];
    let probe_base = 6 + (DREG_COUNT * 2);
    probe_words[..PROBE_WORDS].copy_from_slice(&words[probe_base..(PROBE_WORDS + probe_base)]);
    OracleResult {
        regs: [words[0], words[1], words[2], words[3]],
        cpsr: words[4],
        fpscr: words[5],
        dregs,
        probe_words,
    }
}

fn read_symbols(elf: &Path) -> BTreeMap<String, u32> {
    let output = Command::new("llvm-nm").arg("-n").arg(elf).output().unwrap();
    assert!(
        output.status.success(),
        "llvm-nm failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut symbols = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_whitespace();
        let Some(addr) = parts.next() else { continue };
        let Some(_kind) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        if let Ok(value) = u32::from_str_radix(addr, 16) {
            symbols.insert(name.to_string(), value);
        }
    }
    symbols
}

fn symbol(symbols: &BTreeMap<String, u32>, name: &str) -> u32 {
    *symbols
        .get(name)
        .unwrap_or_else(|| panic!("missing symbol {name}"))
}

fn map_symbol_region(memory: &mut MappedMemory, addr: u32, len: u32) {
    let base = addr & !0xfff;
    let end = addr.wrapping_add(len.max(1)).wrapping_add(0xfff) & !0xfff;
    memory
        .map_zeroed(base, end.wrapping_sub(base) as usize)
        .unwrap_or(());
}

fn assert_coverage_manifest(cases: &[OracleCase]) {
    let required = [
        "ARM",
        "Thumb-1",
        "Thumb-2",
        "IT",
        "branch",
        "integer-alu",
        "flags",
        "load-store",
        "multiply",
        "long-multiply",
        "bit-operations",
        "interworking",
        "VFPv3",
        "NEON",
        "target-driven-NEON",
        "OpenSSL-arithmetic",
        "MCPE-regression",
    ];
    for item in required {
        assert!(
            cases
                .iter()
                .any(|case| case.coverage.iter().any(|coverage| *coverage == item)),
            "missing armv7a oracle coverage bucket {item}"
        );
    }
}

fn write_replay_artifacts(
    case: &OracleCase,
    built: &BuiltCase,
    qemu: &OracleResult,
    aemu: &AemuRun,
    mismatch: Option<&str>,
    artifact_dir: &Path,
) {
    write_aemu_trace(&aemu.trace, &artifact_dir.join("aemu_trace.jsonl"));
    if let Some(mismatch) = mismatch {
        let divergence = format!(
            concat!(
                "{{\n",
                "  \"name\": \"{}\",\n",
                "  \"first_mismatch\": \"{}\",\n",
                "  \"body_asm\": {:?},\n",
                "  \"qemu\": {},\n",
                "  \"aemu\": {},\n",
                "  \"aemu_trace\": \"{}\",\n",
                "  \"disasm\": \"{}\"\n",
                "}}\n"
            ),
            case.name,
            json_escape(mismatch),
            case.body_asm,
            result_json(qemu),
            result_json(&aemu.result),
            artifact_dir.join("aemu_trace.jsonl").display(),
            built.disasm.display(),
        );
        fs::write(artifact_dir.join("divergence.json"), divergence).unwrap();
    }
    let replay = format!(
        concat!(
            "{{\n",
            "  \"name\": \"{}\",\n",
            "  \"elf\": \"{}\",\n",
            "  \"testcase_bin\": \"{}\",\n",
            "  \"disasm\": \"{}\",\n",
            "  \"aemu_trace\": \"{}\",\n",
            "  \"first_mismatch\": {},\n",
            "  \"qemu\": {},\n",
            "  \"aemu\": {},\n",
            "  \"coverage\": {:?}\n",
            "}}\n"
        ),
        case.name,
        built.elf.display(),
        built.testcase_bin.display(),
        built.disasm.display(),
        artifact_dir.join("aemu_trace.jsonl").display(),
        mismatch
            .map(|value| format!("\"{}\"", json_escape(value)))
            .unwrap_or_else(|| "null".to_string()),
        result_json(qemu),
        result_json(&aemu.result),
        case.coverage,
    );
    fs::write(artifact_dir.join("replay.json"), replay).unwrap();
}

fn result_json(result: &OracleResult) -> String {
    format!(
        concat!(
            "{{",
            "\"regs\":[{},{},{},{}],",
            "\"cpsr\":{},",
            "\"fpscr\":{},",
            "\"dregs\":[{},{},{},{},{},{},{},{}],",
            "\"probe_words\":[{},{},{},{}]",
            "}}"
        ),
        result.regs[0],
        result.regs[1],
        result.regs[2],
        result.regs[3],
        result.cpsr,
        result.fpscr,
        result.dregs[0],
        result.dregs[1],
        result.dregs[2],
        result.dregs[3],
        result.dregs[4],
        result.dregs[5],
        result.dregs[6],
        result.dregs[7],
        result.probe_words[0],
        result.probe_words[1],
        result.probe_words[2],
        result.probe_words[3],
    )
}

fn write_aemu_trace(trace: &[StepTrace], path: &Path) {
    let mut out = String::new();
    for item in trace {
        out.push_str(&format!(
            "{{\"step\":{},\"pc\":{},\"isa\":\"{:?}\"}}\n",
            item.step, item.pc, item.isa
        ));
    }
    fs::write(path, out).unwrap();
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn require_tool(tool: &str) {
    let output = Command::new("which").arg(tool).output().unwrap();
    assert!(
        output.status.success(),
        "required tool not found in PATH: {tool}"
    );
}
