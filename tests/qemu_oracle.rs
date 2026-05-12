use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aemu::armv6::{Cpu, Memory, VecMemory};

fn run_arm_linux_exit(asm: &str) -> Option<i32> {
    if Command::new("clang").arg("--version").output().is_err()
        || Command::new("qemu-arm").arg("--version").output().is_err()
    {
        return None;
    }

    let mut dir = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    dir.push(format!("aemu-qemu-oracle-{stamp}"));
    fs::create_dir_all(&dir).ok()?;

    let asm_path = dir.join("test.S");
    let elf_path = dir.join("test");
    fs::write(&asm_path, asm).ok()?;

    let clang = Command::new("clang")
        .arg("--target=arm-linux-gnueabi")
        .arg("-march=armv6")
        .arg("-nostdlib")
        .arg("-static")
        .arg("-fuse-ld=lld")
        .arg("-Wl,-e,_start")
        .arg(&asm_path)
        .arg("-o")
        .arg(&elf_path)
        .output()
        .ok()?;
    if !clang.status.success() {
        eprintln!(
            "skipping qemu oracle test; clang failed: {}",
            String::from_utf8_lossy(&clang.stderr)
        );
        return None;
    }

    let status = Command::new("qemu-arm").arg(&elf_path).status().ok()?;
    status.code()
}

fn oracle_program(body: &str) -> String {
    format!(
        ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .global _start\n\
         _start:\n\
         {body}\n\
         and r0, r0, #255\n\
         mov r7, #1\n\
         svc #0\n"
    )
}

fn arm_parallel_media_instr(family: u32, op: u32, rd: usize, rn: usize, rm: usize) -> u32 {
    0xe600_0f10 | (family << 20) | ((rn as u32) << 16) | ((rd as u32) << 12) | (op << 5) | rm as u32
}

fn arm_sel_instr(rd: usize, rn: usize, rm: usize) -> u32 {
    0xe680_0fb0 | ((rn as u32) << 16) | ((rd as u32) << 12) | rm as u32
}

fn arm_extend_instr(key: u32, rn: usize, rd: usize, rm: usize, rotate: u32) -> u32 {
    0xe000_0000
        | key
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | ((rotate & 0x3) << 10)
        | rm as u32
}

fn arm_signed_halfword_instr(
    base: u32,
    rd_or_hi: usize,
    rn_or_lo: usize,
    rs: usize,
    rm: usize,
    x: u32,
    y: u32,
) -> u32 {
    base | ((rd_or_hi as u32) << 16)
        | ((rn_or_lo as u32) << 12)
        | ((rs as u32) << 8)
        | (y << 6)
        | (x << 5)
        | rm as u32
}

fn arm_dual16_multiply_instr(
    long: bool,
    subtract: bool,
    exchange: bool,
    rd_or_hi: usize,
    ra_or_lo: usize,
    rm: usize,
    rn: usize,
) -> u32 {
    0xe700_0010
        | (u32::from(long) << 22)
        | ((rd_or_hi as u32) << 16)
        | ((ra_or_lo as u32) << 12)
        | ((rm as u32) << 8)
        | (u32::from(subtract) << 6)
        | (u32::from(exchange) << 5)
        | rn as u32
}

fn arm_high_word_multiply_instr(
    subtract: bool,
    round: bool,
    rd: usize,
    ra: usize,
    rm: usize,
    rn: usize,
) -> u32 {
    0xe750_0010
        | ((rd as u32) << 16)
        | ((ra as u32) << 12)
        | ((rm as u32) << 8)
        | (u32::from(subtract) << 7)
        | (u32::from(subtract) << 6)
        | (u32::from(round) << 5)
        | rn as u32
}

fn arm_multiply_instr(
    accumulate: bool,
    set_flags: bool,
    rd: usize,
    rn: usize,
    rs: usize,
    rm: usize,
) -> u32 {
    0xe000_0090
        | (u32::from(accumulate) << 21)
        | (u32::from(set_flags) << 20)
        | ((rd as u32) << 16)
        | ((rn as u32) << 12)
        | ((rs as u32) << 8)
        | rm as u32
}

fn arm_long_multiply_instr(
    signed: bool,
    accumulate: bool,
    set_flags: bool,
    rd_hi: usize,
    rd_lo: usize,
    rs: usize,
    rm: usize,
) -> u32 {
    0xe080_0090
        | (u32::from(signed) << 22)
        | (u32::from(accumulate) << 21)
        | (u32::from(set_flags) << 20)
        | ((rd_hi as u32) << 16)
        | ((rd_lo as u32) << 12)
        | ((rs as u32) << 8)
        | rm as u32
}

fn arm_umaal_instr(rd_lo: usize, rd_hi: usize, rn: usize, rm: usize) -> u32 {
    0xe040_0090 | ((rd_hi as u32) << 16) | ((rd_lo as u32) << 12) | ((rm as u32) << 8) | rn as u32
}

fn arm_branch_instr(cond: u32, link: bool, pc: u32, target: u32) -> u32 {
    let offset_words = (target as i32 - (pc as i32 + 8)) / 4;
    ((cond & 0xf) << 28)
        | 0x0a00_0000
        | (u32::from(link) << 24)
        | ((offset_words as u32) & 0x00ff_ffff)
}

fn arm_data_proc_reg_shift_instr(
    opcode: u32,
    set_flags: bool,
    rn: usize,
    rd: usize,
    rm: usize,
    shift: u32,
    amount: u32,
) -> u32 {
    0xe000_0000
        | ((opcode & 0xf) << 21)
        | (u32::from(set_flags) << 20)
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | ((amount & 0x1f) << 7)
        | ((shift & 0x3) << 5)
        | rm as u32
}

fn arm_data_proc_imm_instr(opcode: u32, set_flags: bool, rn: usize, rd: usize, imm12: u32) -> u32 {
    0xe200_0000
        | ((opcode & 0xf) << 21)
        | (u32::from(set_flags) << 20)
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | (imm12 & 0xfff)
}

fn arm_expand_imm_value(imm12: u32) -> u32 {
    (imm12 & 0xff).rotate_right(((imm12 >> 8) & 0xf) * 2)
}

fn arm_pack_halfword_instr(top: bool, rd: usize, rn: usize, rm: usize, shift: u32) -> u32 {
    0xe680_0010
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | ((shift & 0x1f) << 7)
        | (u32::from(top) << 6)
        | rm as u32
}

fn arm_vcmp_single_instr(sd: usize, sm: usize) -> u32 {
    0xeeb4_0a40
        | (((sd as u32) >> 1) << 12)
        | (((sd as u32) & 1) << 22)
        | ((sm as u32) >> 1)
        | (((sm as u32) & 1) << 5)
}

fn arm_vcmp_single_zero_instr(sd: usize) -> u32 {
    0xeeb5_0a40 | (((sd as u32) >> 1) << 12) | (((sd as u32) & 1) << 22)
}

fn arm_vcmp_double_instr(dd: usize, dm: usize) -> u32 {
    0xeeb4_0b40
        | (((dd as u32) & 0xf) << 12)
        | (((dd as u32) >> 4) << 22)
        | ((dm as u32) & 0xf)
        | (((dm as u32) >> 4) << 5)
}

fn arm_vcmp_double_zero_instr(dd: usize) -> u32 {
    0xeeb5_0b40 | (((dd as u32) & 0xf) << 12) | (((dd as u32) >> 4) << 22)
}

fn arm_single_transfer_imm(
    load: bool,
    byte: bool,
    pre: bool,
    up: bool,
    writeback: bool,
    rn: usize,
    rd: usize,
    offset: u32,
) -> u32 {
    0xe400_0000
        | (u32::from(pre) << 24)
        | (u32::from(up) << 23)
        | (u32::from(byte) << 22)
        | (u32::from(writeback) << 21)
        | (u32::from(load) << 20)
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | (offset & 0xfff)
}

fn arm_single_transfer_reg(
    load: bool,
    byte: bool,
    pre: bool,
    up: bool,
    writeback: bool,
    rn: usize,
    rd: usize,
    rm: usize,
    shift: u32,
    amount: u32,
) -> u32 {
    0xe600_0000
        | (u32::from(pre) << 24)
        | (u32::from(up) << 23)
        | (u32::from(byte) << 22)
        | (u32::from(writeback) << 21)
        | (u32::from(load) << 20)
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | ((amount & 0x1f) << 7)
        | ((shift & 0x3) << 5)
        | rm as u32
}

fn arm_halfword_transfer(
    pre: bool,
    up: bool,
    immediate: bool,
    writeback: bool,
    load: bool,
    rn: usize,
    rd: usize,
    op: u32,
    offset: u32,
) -> u32 {
    let offset_bits = if immediate {
        ((offset & 0xf0) << 4) | (offset & 0xf)
    } else {
        offset & 0xf
    };
    0xe000_0090
        | (u32::from(pre) << 24)
        | (u32::from(up) << 23)
        | (u32::from(immediate) << 22)
        | (u32::from(writeback) << 21)
        | (u32::from(load) << 20)
        | ((rn as u32) << 16)
        | ((rd as u32) << 12)
        | ((op & 0b11) << 5)
        | offset_bits
}

fn arm_block_transfer(
    pre: bool,
    up: bool,
    writeback: bool,
    load: bool,
    rn: usize,
    regs: u32,
) -> u32 {
    0xe800_0000
        | (u32::from(pre) << 24)
        | (u32::from(up) << 23)
        | (u32::from(writeback) << 21)
        | (u32::from(load) << 20)
        | ((rn as u32) << 16)
        | regs
}

fn thumb_reg_offset_transfer(op: u16, ro: usize, rb: usize, rd: usize) -> u16 {
    0x5000 | ((op & 0x7) << 9) | ((ro as u16) << 6) | ((rb as u16) << 3) | rd as u16
}

fn thumb_imm_word_byte_transfer(load: bool, byte: bool, imm5: u16, rb: usize, rd: usize) -> u16 {
    0x6000
        | (u16::from(byte) << 12)
        | (u16::from(load) << 11)
        | ((imm5 & 0x1f) << 6)
        | ((rb as u16) << 3)
        | rd as u16
}

fn thumb_imm_halfword_transfer(load: bool, imm5: u16, rb: usize, rd: usize) -> u16 {
    0x8000 | (u16::from(load) << 11) | ((imm5 & 0x1f) << 6) | ((rb as u16) << 3) | rd as u16
}

fn thumb_sp_relative_transfer(load: bool, rd: usize, imm8: u16) -> u16 {
    0x9000 | (u16::from(load) << 11) | ((rd as u16) << 8) | (imm8 & 0xff)
}

fn thumb_literal_load(rd: usize, imm8: u16) -> u16 {
    0x4800 | ((rd as u16) << 8) | (imm8 & 0xff)
}

fn thumb_add_pc_sp(sp: bool, rd: usize, imm8: u16) -> u16 {
    0xa000 | (u16::from(sp) << 11) | ((rd as u16) << 8) | (imm8 & 0xff)
}

fn thumb_adjust_sp(subtract: bool, imm7: u16) -> u16 {
    0xb000 | (u16::from(subtract) << 7) | (imm7 & 0x7f)
}

fn thumb_cond_branch_instr(cond: u16, imm8: u16) -> u16 {
    0xd000 | ((cond & 0xf) << 8) | (imm8 & 0xff)
}

fn thumb_uncond_branch_instr(imm11: u16) -> u16 {
    0xe000 | (imm11 & 0x7ff)
}

fn thumb_alu_instr(op: u16, rm: usize, rd: usize) -> u16 {
    0x4000 | ((op & 0xf) << 6) | ((rm as u16) << 3) | rd as u16
}

fn thumb_high_reg_instr(op: u16, rm: usize, rd: usize) -> u16 {
    0x4400
        | ((op & 0x3) << 8)
        | (u16::from(rd >= 8) << 7)
        | (u16::from(rm >= 8) << 6)
        | (((rm as u16) & 0x7) << 3)
        | ((rd as u16) & 0x7)
}

fn thumb_shift_imm_instr(kind: u16, amount: u16, rs: usize, rd: usize) -> u16 {
    ((kind & 0x3) << 11) | ((amount & 0x1f) << 6) | ((rs as u16) << 3) | rd as u16
}

fn thumb_add_sub_instr(
    immediate: bool,
    subtract: bool,
    rn_or_imm: u16,
    rs: usize,
    rd: usize,
) -> u16 {
    0x1800
        | (u16::from(immediate) << 10)
        | (u16::from(subtract) << 9)
        | ((rn_or_imm & 0x7) << 6)
        | ((rs as u16) << 3)
        | rd as u16
}

fn thumb_imm_instr(op: u16, rd: usize, imm: u16) -> u16 {
    0x2000 | ((op & 0x3) << 11) | ((rd as u16) << 8) | (imm & 0xff)
}

fn byte_fold(value: u32) -> u32 {
    value ^ (value >> 8) ^ (value >> 16) ^ (value >> 24)
}

fn double_byte_fold(value: u64) -> u32 {
    byte_fold(value as u32) ^ byte_fold((value >> 32) as u32)
}

#[test]
fn qemu_oracle_usad8_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x10203040\n\
         ldr r2, =0x18102850\n\
         usad8 r0, r1, r2",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x1020_3040);
    cpu.set_reg(2, 0x1810_2850);
    cpu.execute_arm(0xe780_f211, 0, &mut mem).unwrap();

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_usad8_usada8_pair_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x10203040\n\
         ldr r2, =0x18102850\n\
         usad8 r0, r1, r2\n\
         mov r12, r0\n\
         mov r3, #13\n\
         usada8 r0, r1, r2, r3\n\
         eor r0, r0, r12",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x1020_3040);
    cpu.set_reg(2, 0x1810_2850);
    cpu.execute_arm(0xe780_f211, 0, &mut mem).unwrap(); // usad8 r0, r1, r2
    let folded = cpu.reg(0);
    cpu.set_reg(3, 13);
    cpu.execute_arm(0xe780_3211, 0, &mut mem).unwrap(); // usada8 r0, r1, r2, r3
    cpu.set_reg(0, cpu.reg(0) ^ folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_smlabb_matches_interpreter() {
    let asm = oracle_program(
        "mov r1, #3\n\
         mov r2, #4\n\
         mov r3, #5\n\
         smlabb r0, r1, r2, r3",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 3);
    cpu.set_reg(2, 4);
    cpu.set_reg(3, 5);
    cpu.execute_arm(0xe100_3281, 0, &mut mem).unwrap();

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_signed_halfword_multiply_matrix_matches_interpreter() {
    const XY: &[(&str, u32, u32)] = &[("bb", 0, 0), ("bt", 0, 1), ("tb", 1, 0), ("tt", 1, 1)];
    const WY: &[(&str, u32)] = &[("b", 0), ("t", 1)];

    let mut body = String::from("mov r12, #0\n");
    for (idx, (suffix, _, _)) in XY.iter().enumerate() {
        let rm = 0x8001_7fffu32.rotate_left(idx as u32 * 3);
        let rs = 0x0002_fffeu32.rotate_right(idx as u32 * 5);
        let rn = 0x1000_0000u32.wrapping_add(idx as u32 * 17);
        body.push_str(&format!(
            "ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n\
             ldr r3, ={rn:#010x}\n\
             smla{suffix} r0, r1, r2, r3\n\
             eor r12, r12, r0\n"
        ));
    }
    for (idx, (suffix, _, _)) in XY.iter().enumerate() {
        let rm = 0x7fff_8001u32.rotate_right(idx as u32 * 4);
        let rs = 0xfffd_0003u32.rotate_left(idx as u32 * 7);
        let lo = 0x0000_0100u32.wrapping_add(idx as u32);
        let hi = 0xffff_ffffu32.wrapping_sub(idx as u32);
        body.push_str(&format!(
            "ldr r4, ={lo:#010x}\n\
             ldr r5, ={hi:#010x}\n\
             ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n\
             smlal{suffix} r4, r5, r1, r2\n\
             eor r12, r12, r4\n\
             eor r12, r12, r5\n"
        ));
    }
    for (idx, (suffix, _, _)) in XY.iter().enumerate() {
        let rm = 0x0003_fffc_u32.rotate_left(idx as u32 * 6);
        let rs = 0x7ffe_8002_u32.rotate_right(idx as u32 * 2);
        body.push_str(&format!(
            "ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n\
             smul{suffix} r0, r1, r2\n\
             eor r12, r12, r0\n"
        ));
    }
    for (idx, (suffix, _)) in WY.iter().enumerate() {
        let rm = 0x4000_8000u32.rotate_left(idx as u32 * 5);
        let rs = 0x0003_fffeu32.rotate_right(idx as u32 * 8);
        let rn = 0x2000_0000u32.wrapping_add(idx as u32 * 13);
        body.push_str(&format!(
            "ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n\
             ldr r3, ={rn:#010x}\n\
             smlaw{suffix} r0, r1, r2, r3\n\
             eor r12, r12, r0\n"
        ));

        let rm = rm ^ 0x1357_2468;
        let rs = rs ^ 0x89ab_cdef;
        body.push_str(&format!(
            "ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n\
             smulw{suffix} r0, r1, r2\n\
             eor r12, r12, r0\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0u32;
    for (idx, (_, x, y)) in XY.iter().enumerate() {
        cpu.set_reg(1, 0x8001_7fffu32.rotate_left(idx as u32 * 3));
        cpu.set_reg(2, 0x0002_fffeu32.rotate_right(idx as u32 * 5));
        cpu.set_reg(3, 0x1000_0000u32.wrapping_add(idx as u32 * 17));
        cpu.execute_arm(
            arm_signed_halfword_instr(0xe100_0080, 0, 3, 2, 1, *x, *y),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);
    }
    for (idx, (_, x, y)) in XY.iter().enumerate() {
        cpu.set_reg(4, 0x0000_0100u32.wrapping_add(idx as u32));
        cpu.set_reg(5, 0xffff_ffffu32.wrapping_sub(idx as u32));
        cpu.set_reg(1, 0x7fff_8001u32.rotate_right(idx as u32 * 4));
        cpu.set_reg(2, 0xfffd_0003u32.rotate_left(idx as u32 * 7));
        cpu.execute_arm(
            arm_signed_halfword_instr(0xe140_0080, 5, 4, 2, 1, *x, *y),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(4) ^ cpu.reg(5);
    }
    for (idx, (_, x, y)) in XY.iter().enumerate() {
        cpu.set_reg(1, 0x0003_fffc_u32.rotate_left(idx as u32 * 6));
        cpu.set_reg(2, 0x7ffe_8002_u32.rotate_right(idx as u32 * 2));
        cpu.execute_arm(
            arm_signed_halfword_instr(0xe160_0080, 0, 0, 2, 1, *x, *y),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);
    }
    for (idx, (_, y)) in WY.iter().enumerate() {
        let rm = 0x4000_8000u32.rotate_left(idx as u32 * 5);
        let rs = 0x0003_fffeu32.rotate_right(idx as u32 * 8);
        cpu.set_reg(1, rm);
        cpu.set_reg(2, rs);
        cpu.set_reg(3, 0x2000_0000u32.wrapping_add(idx as u32 * 13));
        cpu.execute_arm(
            arm_signed_halfword_instr(0xe120_0080, 0, 3, 2, 1, 0, *y),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);

        cpu.set_reg(1, rm ^ 0x1357_2468);
        cpu.set_reg(2, rs ^ 0x89ab_cdef);
        cpu.execute_arm(
            arm_signed_halfword_instr(0xe120_00a0, 0, 0, 2, 1, 0, *y),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_smlad_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x00020003\n\
         ldr r2, =0x00050007\n\
         mov r3, #10\n\
         smlad r0, r1, r2, r3",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x0002_0003);
    cpu.set_reg(2, 0x0005_0007);
    cpu.set_reg(3, 10);
    cpu.execute_arm(0xe700_3211, 0, &mut mem).unwrap();

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_dual16_dsp_multiply_matrix_matches_interpreter() {
    const WORD_CASES: &[(&str, bool, bool, bool)] = &[
        ("smlad", false, false, true),
        ("smladx", false, true, true),
        ("smlsd", true, false, true),
        ("smlsdx", true, true, true),
        ("smuad", false, false, false),
        ("smuadx", false, true, false),
        ("smusd", true, false, false),
        ("smusdx", true, true, false),
    ];
    const LONG_CASES: &[(&str, bool, bool)] = &[
        ("smlald", false, false),
        ("smlaldx", false, true),
        ("smlsld", true, false),
        ("smlsldx", true, true),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (idx, (mnemonic, _, _, accumulate)) in WORD_CASES.iter().enumerate() {
        let rn = 0x0003_fffeu32.rotate_left(idx as u32 * 3);
        let rm = 0x7ffd_0004u32.rotate_right(idx as u32 * 5);
        let ra = 0x0100_0000u32.wrapping_add(idx as u32 * 23);
        body.push_str(&format!(
            "ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n"
        ));
        if *accumulate {
            body.push_str(&format!(
                "ldr r3, ={ra:#010x}\n\
                 {mnemonic} r0, r1, r2, r3\n"
            ));
        } else {
            body.push_str(&format!("{mnemonic} r0, r1, r2\n"));
        }
        body.push_str("eor r12, r12, r0\n");
    }
    for (idx, (mnemonic, _, _)) in LONG_CASES.iter().enumerate() {
        let rn = 0x8001_7fffu32.rotate_left(idx as u32 * 4);
        let rm = 0xfffd_0002u32.rotate_right(idx as u32 * 6);
        let lo = 0x0000_0100u32.wrapping_add(idx as u32 * 3);
        let hi = 0xffff_fffeu32.wrapping_sub(idx as u32);
        body.push_str(&format!(
            "ldr r4, ={lo:#010x}\n\
             ldr r5, ={hi:#010x}\n\
             ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n\
             {mnemonic} r4, r5, r1, r2\n\
             eor r12, r12, r4\n\
             eor r12, r12, r5\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (idx, (_, subtract, exchange, accumulate)) in WORD_CASES.iter().enumerate() {
        cpu.set_reg(1, 0x0003_fffeu32.rotate_left(idx as u32 * 3));
        cpu.set_reg(2, 0x7ffd_0004u32.rotate_right(idx as u32 * 5));
        let ra = if *accumulate { 3 } else { 15 };
        if *accumulate {
            cpu.set_reg(3, 0x0100_0000u32.wrapping_add(idx as u32 * 23));
        }
        cpu.execute_arm(
            arm_dual16_multiply_instr(false, *subtract, *exchange, 0, ra, 2, 1),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);
    }
    for (idx, (_, subtract, exchange)) in LONG_CASES.iter().enumerate() {
        cpu.set_reg(4, 0x0000_0100u32.wrapping_add(idx as u32 * 3));
        cpu.set_reg(5, 0xffff_fffeu32.wrapping_sub(idx as u32));
        cpu.set_reg(1, 0x8001_7fffu32.rotate_left(idx as u32 * 4));
        cpu.set_reg(2, 0xfffd_0002u32.rotate_right(idx as u32 * 6));
        cpu.execute_arm(
            arm_dual16_multiply_instr(true, *subtract, *exchange, 5, 4, 2, 1),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(4) ^ cpu.reg(5);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_dsp_multiply_matrix_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x00030004\n\
         ldr r2, =0x00070002\n\
         mov r3, #9\n\
         smlsd r0, r1, r2, r3\n\
         mov r12, r0\n\
         ldr r5, =0xfffe0003\n\
         ldr r6, =0x0004fff9\n\
         mov r7, #11\n\
         smlsdx r4, r5, r6, r7\n\
         eor r12, r12, r4\n\
         ldr r1, =0x00020003\n\
         ldr r2, =0x00050007\n\
         smuad r0, r1, r2\n\
         eor r12, r12, r0\n\
         ldr r4, =0x00020003\n\
         ldr r5, =0x00050007\n\
         smuadx r3, r4, r5\n\
         eor r12, r12, r3\n\
         ldr r7, =0xfffe0003\n\
         ldr r8, =0x0004fff9\n\
         smusd r6, r7, r8\n\
         eor r12, r12, r6\n\
         ldr r10, =0xfffe0003\n\
         ldr r11, =0x0004fff9\n\
         smusdx r9, r10, r11\n\
         eor r12, r12, r9\n\
         mov r0, #5\n\
         mov r1, #0\n\
         ldr r2, =0xfffe0003\n\
         ldr r3, =0x0004fff9\n\
         smlaldx r0, r1, r2, r3\n\
         eor r12, r12, r0\n\
         eor r12, r12, r1\n\
         mov r4, #7\n\
         mov r5, #0\n\
         ldr r6, =0x00080009\n\
         ldr r7, =0x00020005\n\
         smlsldx r4, r5, r6, r7\n\
         eor r12, r12, r4\n\
         eor r12, r12, r5\n\
         ldr r9, =0x70000000\n\
         ldr r10, =0x30000000\n\
         ldr r11, =0x01000000\n\
         smmlar r8, r9, r10, r11\n\
         eor r12, r12, r8\n\
         ldr r2, =0x01000000\n\
         ldr r3, =0x60000000\n\
         ldr r4, =0x20000000\n\
         smmlsr r1, r2, r3, r4\n\
         eor r0, r12, r1",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);

    cpu.set_reg(1, 0x0003_0004);
    cpu.set_reg(2, 0x0007_0002);
    cpu.set_reg(3, 9);
    cpu.execute_arm(0xe700_3251, 0, &mut mem).unwrap(); // smlsd r0, r1, r2, r3
    let mut folded = cpu.reg(0);

    cpu.set_reg(5, 0xfffe_0003);
    cpu.set_reg(6, 0x0004_fff9);
    cpu.set_reg(7, 11);
    cpu.execute_arm(0xe704_7675, 0, &mut mem).unwrap(); // smlsdx r4, r5, r6, r7
    folded ^= cpu.reg(4);

    cpu.set_reg(1, 0x0002_0003);
    cpu.set_reg(2, 0x0005_0007);
    cpu.execute_arm(0xe700_f211, 0, &mut mem).unwrap(); // smuad r0, r1, r2
    folded ^= cpu.reg(0);

    cpu.set_reg(4, 0x0002_0003);
    cpu.set_reg(5, 0x0005_0007);
    cpu.execute_arm(0xe703_f534, 0, &mut mem).unwrap(); // smuadx r3, r4, r5
    folded ^= cpu.reg(3);

    cpu.set_reg(7, 0xfffe_0003);
    cpu.set_reg(8, 0x0004_fff9);
    cpu.execute_arm(0xe706_f857, 0, &mut mem).unwrap(); // smusd r6, r7, r8
    folded ^= cpu.reg(6);

    cpu.set_reg(10, 0xfffe_0003);
    cpu.set_reg(11, 0x0004_fff9);
    cpu.execute_arm(0xe709_fb7a, 0, &mut mem).unwrap(); // smusdx r9, r10, r11
    folded ^= cpu.reg(9);

    cpu.set_reg(0, 5);
    cpu.set_reg(1, 0);
    cpu.set_reg(2, 0xfffe_0003);
    cpu.set_reg(3, 0x0004_fff9);
    cpu.execute_arm(0xe741_0332, 0, &mut mem).unwrap(); // smlaldx r0, r1, r2, r3
    folded ^= cpu.reg(0) ^ cpu.reg(1);

    cpu.set_reg(4, 7);
    cpu.set_reg(5, 0);
    cpu.set_reg(6, 0x0008_0009);
    cpu.set_reg(7, 0x0002_0005);
    cpu.execute_arm(0xe745_4776, 0, &mut mem).unwrap(); // smlsldx r4, r5, r6, r7
    folded ^= cpu.reg(4) ^ cpu.reg(5);

    cpu.set_reg(9, 0x7000_0000);
    cpu.set_reg(10, 0x3000_0000);
    cpu.set_reg(11, 0x0100_0000);
    cpu.execute_arm(0xe758_ba39, 0, &mut mem).unwrap(); // smmlar r8, r9, r10, r11
    folded ^= cpu.reg(8);

    cpu.set_reg(2, 0x0100_0000);
    cpu.set_reg(3, 0x6000_0000);
    cpu.set_reg(4, 0x2000_0000);
    cpu.execute_arm(0xe751_43f2, 0, &mut mem).unwrap(); // smmlsr r1, r2, r3, r4
    folded ^= cpu.reg(1);
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_high_word_multiply_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x70000000\n\
         ldr r2, =0x20000000\n\
         smmul r0, r1, r2\n\
         ldr r4, =0x7fffffff\n\
         ldr r5, =0x12345678\n\
         smmulr r3, r4, r5\n\
         eor r0, r0, r3\n\
         ldr r7, =0x40000000\n\
         ldr r8, =0x40000000\n\
         ldr r9, =0x01000000\n\
         smmla r6, r7, r8, r9\n\
         eor r0, r0, r6\n\
         ldr r11, =0x60000000\n\
         ldr r12, =0x30000000\n\
         smmls r10, r11, r12, r0\n\
         eor r0, r0, r10",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x7000_0000);
    cpu.set_reg(2, 0x2000_0000);
    cpu.execute_arm(0xe750_f211, 0, &mut mem).unwrap(); // smmul r0, r1, r2
    let mut folded = cpu.reg(0);

    cpu.set_reg(4, 0x7fff_ffff);
    cpu.set_reg(5, 0x1234_5678);
    cpu.execute_arm(0xe753_f534, 0, &mut mem).unwrap(); // smmulr r3, r4, r5
    folded ^= cpu.reg(3);

    cpu.set_reg(7, 0x4000_0000);
    cpu.set_reg(8, 0x4000_0000);
    cpu.set_reg(9, 0x0100_0000);
    cpu.execute_arm(0xe756_9817, 0, &mut mem).unwrap(); // smmla r6, r7, r8, r9
    folded ^= cpu.reg(6);

    cpu.set_reg(0, folded);
    cpu.set_reg(11, 0x6000_0000);
    cpu.set_reg(12, 0x3000_0000);
    cpu.execute_arm(0xe75a_0cdb, 0, &mut mem).unwrap(); // smmls r10, r11, r12, r0
    folded ^= cpu.reg(10);
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_high_word_multiply_matrix_matches_interpreter() {
    const CASES: &[(&str, bool, bool, bool)] = &[
        ("smmul", false, false, false),
        ("smmulr", false, true, false),
        ("smmla", false, false, true),
        ("smmlar", false, true, true),
        ("smmls", true, false, true),
        ("smmlsr", true, true, true),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (idx, (mnemonic, _, _, accumulate)) in CASES.iter().enumerate() {
        let rn = 0x7000_0000u32.wrapping_sub(idx as u32 * 0x0111_1111);
        let rm = 0x3000_0000u32.wrapping_add(idx as u32 * 0x0100_0000);
        let ra = 0x0100_0000u32.wrapping_add(idx as u32 * 0x0010_0000);
        body.push_str(&format!(
            "ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n"
        ));
        if *accumulate {
            body.push_str(&format!(
                "ldr r3, ={ra:#010x}\n\
                 {mnemonic} r0, r1, r2, r3\n"
            ));
        } else {
            body.push_str(&format!("{mnemonic} r0, r1, r2\n"));
        }
        body.push_str("eor r12, r12, r0\n");
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (idx, (_, subtract, round, accumulate)) in CASES.iter().enumerate() {
        cpu.set_reg(1, 0x7000_0000u32.wrapping_sub(idx as u32 * 0x0111_1111));
        cpu.set_reg(2, 0x3000_0000u32.wrapping_add(idx as u32 * 0x0100_0000));
        let ra = if *accumulate { 3 } else { 15 };
        if *accumulate {
            cpu.set_reg(3, 0x0100_0000u32.wrapping_add(idx as u32 * 0x0010_0000));
        }
        cpu.execute_arm(
            arm_high_word_multiply_instr(*subtract, *round, 0, ra, 2, 1),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= cpu.reg(0);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_arm_multiply_matrix_matches_interpreter() {
    const CASES: &[(&str, bool, bool, u32, u32, u32)] = &[
        ("mul", false, false, 0x0001_2345, 0x0001_0003, 0),
        ("muls", false, true, 0, 0x9876_5432, 0),
        ("mla", true, false, 0xffff_fffe, 3, 7),
        ("mlas", true, true, 0x8000_0000, 1, 0),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (mnemonic, accumulate, set_flags, rm, rs, rn) in CASES {
        body.push_str(&format!(
            "ldr r1, ={rm:#010x}\n\
             ldr r2, ={rs:#010x}\n"
        ));
        if *accumulate {
            body.push_str(&format!(
                "ldr r3, ={rn:#010x}\n\
                 {mnemonic} r0, r1, r2, r3\n"
            ));
        } else {
            body.push_str(&format!("{mnemonic} r0, r1, r2\n"));
        }
        body.push_str(
            "eor r12, r12, r0\n\
             eor r12, r12, r0, lsr #8\n\
             eor r12, r12, r0, lsr #16\n\
             eor r12, r12, r0, lsr #24\n",
        );
        if *set_flags {
            body.push_str(
                "mrs r4, cpsr\n\
                 eor r12, r12, r4, lsr #30\n",
            );
        }
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (_, accumulate, set_flags, rm, rs, rn) in CASES {
        cpu.set_reg(1, *rm);
        cpu.set_reg(2, *rs);
        let rn_reg = if *accumulate {
            cpu.set_reg(3, *rn);
            3
        } else {
            0
        };
        cpu.execute_arm(
            arm_multiply_instr(*accumulate, *set_flags, 0, rn_reg, 2, 1),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= byte_fold(cpu.reg(0));
        if *set_flags {
            folded ^= (cpu.cpsr.to_u32() >> 30) & 0x3;
        }
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_dual_long_multiply_matches_interpreter() {
    let asm = oracle_program(
        "mov r0, #1\n\
         mov r1, #0\n\
         ldr r2, =0x00020003\n\
         ldr r3, =0x00050007\n\
         smlald r0, r1, r2, r3\n\
         mov r4, #2\n\
         mov r5, #0\n\
         ldr r6, =0xfffe0003\n\
         ldr r7, =0x0004fff9\n\
         smlaldx r4, r5, r6, r7\n\
         mov r8, #3\n\
         mov r9, #0\n\
         ldr r10, =0x00080009\n\
         ldr r11, =0x00020005\n\
         smlsld r8, r9, r10, r11\n\
         eor r0, r0, r1\n\
         eor r0, r0, r4\n\
         eor r0, r0, r5\n\
         eor r0, r0, r8\n\
         eor r0, r0, r9",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(0, 1);
    cpu.set_reg(1, 0);
    cpu.set_reg(2, 0x0002_0003);
    cpu.set_reg(3, 0x0005_0007);
    cpu.execute_arm(0xe741_0312, 0, &mut mem).unwrap(); // smlald r0, r1, r2, r3

    cpu.set_reg(4, 2);
    cpu.set_reg(5, 0);
    cpu.set_reg(6, 0xfffe_0003);
    cpu.set_reg(7, 0x0004_fff9);
    cpu.execute_arm(0xe745_4736, 0, &mut mem).unwrap(); // smlaldx r4, r5, r6, r7

    cpu.set_reg(8, 3);
    cpu.set_reg(9, 0);
    cpu.set_reg(10, 0x0008_0009);
    cpu.set_reg(11, 0x0002_0005);
    cpu.execute_arm(0xe749_8b5a, 0, &mut mem).unwrap(); // smlsld r8, r9, r10, r11

    let folded = cpu.reg(0) ^ cpu.reg(1) ^ cpu.reg(4) ^ cpu.reg(5) ^ cpu.reg(8) ^ cpu.reg(9);
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_umaal_matches_interpreter() {
    let asm = oracle_program(
        "ldr r0, =0x11111111\n\
         ldr r1, =0x22222222\n\
         ldr r2, =0x12345678\n\
         ldr r3, =0x00010002\n\
         umaal r0, r1, r2, r3\n\
         eor r0, r0, r1",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(0, 0x1111_1111);
    cpu.set_reg(1, 0x2222_2222);
    cpu.set_reg(2, 0x1234_5678);
    cpu.set_reg(3, 0x0001_0002);
    cpu.execute_arm(0xe041_0392, 0, &mut mem).unwrap(); // umaal r0, r1, r2, r3
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(1));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_long_multiply_umaal_matrix_matches_interpreter() {
    const LONG_CASES: &[(&str, bool, bool, bool, u32, u32, u32, u32)] = &[
        ("umull", false, false, false, 0x1234_5678, 0x0001_0002, 0, 0),
        ("umulls", false, false, true, 0, 0x1111_1111, 0, 0),
        (
            "umlal",
            false,
            true,
            false,
            0xffff_0001,
            0x0000_0003,
            0x1111_1111,
            0x2222_2222,
        ),
        ("umlals", false, true, true, 0x8000_0000, 0x0000_0002, 0, 0),
        ("smull", true, false, false, 0xffff_fffe, 0x0000_0003, 0, 0),
        ("smulls", true, false, true, 0x8000_0000, 0x0000_0002, 0, 0),
        (
            "smlal",
            true,
            true,
            false,
            0xffff_0000,
            0x0000_0010,
            0x0101_0101,
            0,
        ),
        ("smlals", true, true, true, 0xffff_ffff, 1, 1, 0),
    ];
    const UMAAL_CASES: &[(u32, u32, u32, u32)] = &[
        (0x1111_1111, 0x2222_2222, 0x1234_5678, 0x0001_0002),
        (0xffff_ffff, 0, 2, 3),
        (0, 0xffff_ffff, 0x8000_0000, 2),
        (0x89ab_cdef, 0x7654_3210, 0xfedc_ba98, 0x1357_2468),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (mnemonic, _, _accumulate, set_flags, rm, rs, acc_lo, acc_hi) in LONG_CASES {
        body.push_str(&format!(
            "ldr r0, ={acc_lo:#010x}\n\
             ldr r1, ={acc_hi:#010x}\n\
             ldr r2, ={rm:#010x}\n\
             ldr r3, ={rs:#010x}\n\
             {mnemonic} r0, r1, r2, r3\n"
        ));
        body.push_str(
            "eor r12, r12, r0\n\
             eor r12, r12, r0, lsr #8\n\
             eor r12, r12, r0, lsr #16\n\
             eor r12, r12, r0, lsr #24\n\
             eor r12, r12, r1\n\
             eor r12, r12, r1, lsr #8\n\
             eor r12, r12, r1, lsr #16\n\
             eor r12, r12, r1, lsr #24\n",
        );
        if *set_flags {
            body.push_str(
                "mrs r4, cpsr\n\
                 eor r12, r12, r4, lsr #30\n",
            );
        }
    }
    for (lo, hi, rn, rm) in UMAAL_CASES {
        body.push_str(&format!(
            "ldr r0, ={lo:#010x}\n\
             ldr r1, ={hi:#010x}\n\
             ldr r2, ={rn:#010x}\n\
             ldr r3, ={rm:#010x}\n\
             umaal r0, r1, r2, r3\n\
             eor r12, r12, r0\n\
             eor r12, r12, r0, lsr #8\n\
             eor r12, r12, r0, lsr #16\n\
             eor r12, r12, r0, lsr #24\n\
             eor r12, r12, r1\n\
             eor r12, r12, r1, lsr #8\n\
             eor r12, r12, r1, lsr #16\n\
             eor r12, r12, r1, lsr #24\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (_, signed, accumulate, set_flags, rm, rs, acc_lo, acc_hi) in LONG_CASES {
        cpu.set_reg(0, *acc_lo);
        cpu.set_reg(1, *acc_hi);
        cpu.set_reg(2, *rm);
        cpu.set_reg(3, *rs);
        cpu.execute_arm(
            arm_long_multiply_instr(*signed, *accumulate, *set_flags, 1, 0, 3, 2),
            0,
            &mut mem,
        )
        .unwrap();
        folded ^= byte_fold(cpu.reg(0));
        folded ^= byte_fold(cpu.reg(1));
        if *set_flags {
            folded ^= (cpu.cpsr.to_u32() >> 30) & 0x3;
        }
    }
    for (lo, hi, rn, rm) in UMAAL_CASES {
        cpu.set_reg(0, *lo);
        cpu.set_reg(1, *hi);
        cpu.set_reg(2, *rn);
        cpu.set_reg(3, *rm);
        cpu.execute_arm(arm_umaal_instr(0, 1, 2, 3), 0, &mut mem)
            .unwrap();
        folded ^= byte_fold(cpu.reg(0));
        folded ^= byte_fold(cpu.reg(1));
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_scalar_saturation_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x7fffffff\n\
         mov r2, #1\n\
         qadd r0, r1, r2\n\
         mov r4, #0\n\
         ldr r5, =0x80000000\n\
         qsub r3, r4, r5\n\
         ldr r7, =0x70000000\n\
         ldr r8, =0x70000000\n\
         qdadd r6, r7, r8\n\
         ldr r10, =0x80000000\n\
         ldr r11, =0x70000000\n\
         qdsub r9, r10, r11\n\
         eor r0, r0, r3\n\
         eor r0, r0, r6\n\
         eor r0, r0, r9",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x7fff_ffff);
    cpu.set_reg(2, 1);
    cpu.execute_arm(0xe102_0051, 0, &mut mem).unwrap(); // qadd r0, r1, r2

    cpu.set_reg(4, 0);
    cpu.set_reg(5, 0x8000_0000);
    cpu.execute_arm(0xe125_3054, 0, &mut mem).unwrap(); // qsub r3, r4, r5

    cpu.set_reg(7, 0x7000_0000);
    cpu.set_reg(8, 0x7000_0000);
    cpu.execute_arm(0xe148_6057, 0, &mut mem).unwrap(); // qdadd r6, r7, r8

    cpu.set_reg(10, 0x8000_0000);
    cpu.set_reg(11, 0x7000_0000);
    cpu.execute_arm(0xe16b_905a, 0, &mut mem).unwrap(); // qdsub r9, r10, r11

    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(3) ^ cpu.reg(6) ^ cpu.reg(9));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_scalar_saturation_q_flag_matches_interpreter() {
    let asm = oracle_program(
        "mov r6, #0\n\
         mov r12, #0\n\
         msr APSR_nzcvq, r12\n\
         ldr r1, =0x7fffffff\n\
         mov r2, #1\n\
         qadd r0, r1, r2\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #1\n\
         msr APSR_nzcvq, r12\n\
         mov r1, #1\n\
         mov r2, #2\n\
         qadd r0, r1, r2\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #2\n\
         msr APSR_nzcvq, r12\n\
         mov r1, #0\n\
         ldr r2, =0x80000000\n\
         qsub r0, r1, r2\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #4\n\
         msr APSR_nzcvq, r12\n\
         ldr r1, =0x70000000\n\
         ldr r2, =0x70000000\n\
         qdadd r0, r1, r2\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #8\n\
         msr APSR_nzcvq, r12\n\
         ldr r1, =0x80000000\n\
         ldr r2, =0x70000000\n\
         qdsub r0, r1, r2\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #16\n\
         msr APSR_nzcvq, r12\n\
         mov r1, #200\n\
         ssat r0, #8, r1\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #32\n\
         msr APSR_nzcvq, r12\n\
         mov r1, #42\n\
         ssat r0, #8, r1\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #64\n\
         msr APSR_nzcvq, r12\n\
         mvn r3, #0\n\
         usat r2, #7, r3\n\
         mrs r3, cpsr\n\
         and r3, r3, #0x08000000\n\
         cmp r3, #0\n\
         eorne r6, r6, #128\n\
         mov r0, r6",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    fn fold_q(cpu: &mut Cpu, folded: &mut u32, bit: u32) {
        if cpu.cpsr.q {
            *folded ^= bit;
        }
        cpu.cpsr.q = false;
    }

    cpu.cpsr.q = false;
    cpu.set_reg(1, 0x7fff_ffff);
    cpu.set_reg(2, 1);
    cpu.execute_arm(0xe102_0051, 0, &mut mem).unwrap(); // qadd r0, r1, r2
    fold_q(&mut cpu, &mut folded, 1);

    cpu.set_reg(1, 1);
    cpu.set_reg(2, 2);
    cpu.execute_arm(0xe102_0051, 0, &mut mem).unwrap(); // qadd r0, r1, r2
    fold_q(&mut cpu, &mut folded, 2);

    cpu.set_reg(1, 0);
    cpu.set_reg(2, 0x8000_0000);
    cpu.execute_arm(0xe122_0051, 0, &mut mem).unwrap(); // qsub r0, r1, r2
    fold_q(&mut cpu, &mut folded, 4);

    cpu.set_reg(1, 0x7000_0000);
    cpu.set_reg(2, 0x7000_0000);
    cpu.execute_arm(0xe142_0051, 0, &mut mem).unwrap(); // qdadd r0, r1, r2
    fold_q(&mut cpu, &mut folded, 8);

    cpu.set_reg(1, 0x8000_0000);
    cpu.set_reg(2, 0x7000_0000);
    cpu.execute_arm(0xe162_0051, 0, &mut mem).unwrap(); // qdsub r0, r1, r2
    fold_q(&mut cpu, &mut folded, 16);

    cpu.set_reg(1, 200);
    cpu.execute_arm(0xe6a7_0011, 0, &mut mem).unwrap(); // ssat r0, #8, r1
    fold_q(&mut cpu, &mut folded, 32);

    cpu.set_reg(1, 42);
    cpu.execute_arm(0xe6a7_0011, 0, &mut mem).unwrap(); // ssat r0, #8, r1
    fold_q(&mut cpu, &mut folded, 64);

    cpu.set_reg(3, u32::MAX);
    cpu.execute_arm(0xe6e7_2013, 0, &mut mem).unwrap(); // usat r2, #7, r3
    fold_q(&mut cpu, &mut folded, 128);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_saturate_instructions_match_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x00000123\n\
         ssat r0, #8, r1\n\
         ldr r3, =0xffffff80\n\
         usat r2, #7, r3\n\
         ldr r5, =0x0100ff00\n\
         ssat16 r4, #8, r5\n\
         ldr r7, =0x0080ff80\n\
         usat16 r6, #7, r7\n\
         ldr r9, =0xfffff234\n\
         ssat r8, #12, r9, asr #4\n\
         ldr r11, =0x00000200\n\
         usat r10, #10, r11, lsl #2\n\
         eor r0, r0, r2\n\
         eor r0, r0, r4\n\
         eor r0, r0, r6\n\
         eor r0, r0, r8\n\
         eor r0, r0, r10",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x0000_0123);
    cpu.execute_arm(0xe6a7_0011, 0, &mut mem).unwrap(); // ssat r0, #8, r1

    cpu.set_reg(3, 0xffff_ff80);
    cpu.execute_arm(0xe6e7_2013, 0, &mut mem).unwrap(); // usat r2, #7, r3

    cpu.set_reg(5, 0x0100_ff00);
    cpu.execute_arm(0xe6a7_4f35, 0, &mut mem).unwrap(); // ssat16 r4, #8, r5

    cpu.set_reg(7, 0x0080_ff80);
    cpu.execute_arm(0xe6e7_6f37, 0, &mut mem).unwrap(); // usat16 r6, #7, r7

    cpu.set_reg(9, 0xffff_f234);
    cpu.execute_arm(0xe6ab_8259, 0, &mut mem).unwrap(); // ssat r8, #12, r9, asr #4

    cpu.set_reg(11, 0x0000_0200);
    cpu.execute_arm(0xe6ea_a11b, 0, &mut mem).unwrap(); // usat r10, #10, r11, lsl #2

    cpu.set_reg(
        0,
        cpu.reg(0) ^ cpu.reg(2) ^ cpu.reg(4) ^ cpu.reg(6) ^ cpu.reg(8) ^ cpu.reg(10),
    );

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_extend_and_reverse_match_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x11223344\n\
         rev r0, r1\n\
         ldr r3, =0x55667788\n\
         rev16 r2, r3\n\
         ldr r5, =0x000080ff\n\
         revsh r4, r5\n\
         mov r7, #5\n\
         ldr r8, =0x000000f6\n\
         sxtab r6, r7, r8\n\
         ldr r10, =0x00010002\n\
         ldr r11, =0x00fe00ff\n\
         uxtab16 r9, r10, r11\n\
         eor r0, r0, r2\n\
         eor r0, r0, r4\n\
         eor r0, r0, r6\n\
         eor r0, r0, r9",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x1122_3344);
    cpu.execute_arm(0xe6bf_0f31, 0, &mut mem).unwrap(); // rev r0, r1

    cpu.set_reg(3, 0x5566_7788);
    cpu.execute_arm(0xe6bf_2fb3, 0, &mut mem).unwrap(); // rev16 r2, r3

    cpu.set_reg(5, 0x0000_80ff);
    cpu.execute_arm(0xe6ff_4fb5, 0, &mut mem).unwrap(); // revsh r4, r5

    cpu.set_reg(7, 5);
    cpu.set_reg(8, 0x0000_00f6);
    cpu.execute_arm(0xe6a7_6078, 0, &mut mem).unwrap(); // sxtab r6, r7, r8

    cpu.set_reg(10, 0x0001_0002);
    cpu.set_reg(11, 0x00fe_00ff);
    cpu.execute_arm(0xe6ca_907b, 0, &mut mem).unwrap(); // uxtab16 r9, r10, r11

    cpu.set_reg(
        0,
        cpu.reg(0) ^ cpu.reg(2) ^ cpu.reg(4) ^ cpu.reg(6) ^ cpu.reg(9),
    );

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_extend_add_matrix_matches_interpreter() {
    const CASES: &[(&str, u32, bool)] = &[
        ("sxtb16", 0x0680_0070, false),
        ("sxtab16", 0x0680_0070, true),
        ("sxtb", 0x06a0_0070, false),
        ("sxtab", 0x06a0_0070, true),
        ("sxth", 0x06b0_0070, false),
        ("sxtah", 0x06b0_0070, true),
        ("uxtb16", 0x06c0_0070, false),
        ("uxtab16", 0x06c0_0070, true),
        ("uxtb", 0x06e0_0070, false),
        ("uxtab", 0x06e0_0070, true),
        ("uxth", 0x06f0_0070, false),
        ("uxtah", 0x06f0_0070, true),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (idx, (mnemonic, _, add)) in CASES.iter().enumerate() {
        let rotate = ((idx % 4) * 8) as u32;
        let base = 0x0100_2000u32.wrapping_add(idx as u32 * 0x0001_0101);
        let value = 0x807f_01feu32.rotate_left(idx as u32 * 3);
        let rotate_suffix = if rotate == 0 {
            String::new()
        } else {
            format!(", ror #{rotate}")
        };
        body.push_str(&format!(
            "ldr r1, ={base:#010x}\n\
             ldr r2, ={value:#010x}\n"
        ));
        if *add {
            body.push_str(&format!("{mnemonic} r0, r1, r2{rotate_suffix}\n"));
        } else {
            body.push_str(&format!("{mnemonic} r0, r2{rotate_suffix}\n"));
        }
        body.push_str("eor r12, r12, r0\n");
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (idx, (_, key, add)) in CASES.iter().enumerate() {
        let base = 0x0100_2000u32.wrapping_add(idx as u32 * 0x0001_0101);
        let value = 0x807f_01feu32.rotate_left(idx as u32 * 3);
        let rotate = (idx % 4) as u32;
        cpu.set_reg(1, base);
        cpu.set_reg(2, value);
        let rn = if *add { 1 } else { 15 };
        cpu.execute_arm(arm_extend_instr(*key, rn, 0, 2, rotate), 0, &mut mem)
            .unwrap();
        folded ^= cpu.reg(0);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_clz_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x00f00000\n\
         clz r0, r1",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x00f0_0000);
    cpu.execute_arm(0xe16f_0f11, 0, &mut mem).unwrap(); // clz r0, r1

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_adc_sbc_rsc_flags_match_interpreter() {
    let asm = oracle_program(
        "mov r9, #0\n\
         cmp r9, #0\n\
         mvn r1, #0\n\
         mov r2, #0\n\
         adcs r0, r1, r2\n\
         cmp r9, #1\n\
         mov r4, #0\n\
         mov r5, #0\n\
         sbcs r3, r4, r5\n\
         cmp r9, #0\n\
         mov r7, #1\n\
         mov r8, #5\n\
         rscs r6, r7, r8\n\
         eor r0, r0, r3\n\
         eor r0, r0, r6",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.cpsr.c = true;
    cpu.set_reg(1, u32::MAX);
    cpu.set_reg(2, 0);
    cpu.execute_arm(0xe0b1_0002, 0, &mut mem).unwrap(); // adcs r0, r1, r2

    cpu.cpsr.c = false;
    cpu.set_reg(4, 0);
    cpu.set_reg(5, 0);
    cpu.execute_arm(0xe0d4_3005, 0, &mut mem).unwrap(); // sbcs r3, r4, r5

    cpu.cpsr.c = true;
    cpu.set_reg(7, 1);
    cpu.set_reg(8, 5);
    cpu.execute_arm(0xe0f7_6008, 0, &mut mem).unwrap(); // rscs r6, r7, r8
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(3) ^ cpu.reg(6));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_pack_halfword_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0xaaaabbbb\n\
         ldr r2, =0x11223344\n\
         pkhbt r0, r1, r2, lsl #8\n\
         ldr r4, =0xccccdddd\n\
         ldr r5, =0x81234567\n\
         pkhtb r3, r4, r5, asr #8\n\
         eor r0, r0, r3",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0xaaaa_bbbb);
    cpu.set_reg(2, 0x1122_3344);
    cpu.execute_arm(0xe681_0412, 0, &mut mem).unwrap(); // pkhbt r0, r1, r2, lsl #8

    cpu.set_reg(4, 0xcccc_dddd);
    cpu.set_reg(5, 0x8123_4567);
    cpu.execute_arm(0xe684_3455, 0, &mut mem).unwrap(); // pkhtb r3, r4, r5, asr #8
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(3));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_pack_halfword_shift_matrix_matches_interpreter() {
    const CASES: &[(bool, u32, u32, u32)] = &[
        (false, 0, 0xaaa0_bbbb, 0x1122_3344),
        (false, 8, 0xcccc_dddd, 0x5566_7788),
        (false, 16, 0x1357_2468, 0x89ab_cdef),
        (true, 0, 0x7fff_1111, 0x8123_4567),
        (true, 8, 0x2222_8000, 0xf123_4567),
        (true, 16, 0xabcd_0001, 0x9234_5678),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (top, shift, rn, rm) in CASES {
        body.push_str(&format!(
            "ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n"
        ));
        if *top {
            let amount = if *shift == 0 { 32 } else { *shift };
            body.push_str(&format!("pkhtb r0, r1, r2, asr #{amount}\n"));
        } else if *shift == 0 {
            body.push_str("pkhbt r0, r1, r2\n");
        } else {
            body.push_str(&format!("pkhbt r0, r1, r2, lsl #{shift}\n"));
        }
        body.push_str(
            "eor r12, r12, r0\n\
             eor r12, r12, r0, lsr #8\n\
             eor r12, r12, r0, lsr #16\n\
             eor r12, r12, r0, lsr #24\n",
        );
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (top, shift, rn, rm) in CASES {
        cpu.set_reg(1, *rn);
        cpu.set_reg(2, *rm);
        cpu.execute_arm(arm_pack_halfword_instr(*top, 0, 1, 2, *shift), 0, &mut mem)
            .unwrap();
        folded ^= byte_fold(cpu.reg(0));
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_single_transfer_addressing_matrix_matches_interpreter() {
    let asm = oracle_program(
        "mov r12, #0\n\
         ldr r9, =data\n\
         ldr r1, =0x11223344\n\
         str r1, [r9]\n\
         ldr r0, [r9]\n\
         eor r12, r12, r0\n\
         eor r12, r12, r0, lsr #8\n\
         eor r12, r12, r0, lsr #16\n\
         eor r12, r12, r0, lsr #24\n\
         add r10, r9, #16\n\
         ldr r0, [r10, #-8]\n\
         eor r12, r12, r0\n\
         eor r12, r12, r0, lsr #8\n\
         eor r12, r12, r0, lsr #16\n\
         eor r12, r12, r0, lsr #24\n\
         mov r10, r9\n\
         ldr r2, [r10], #4\n\
         sub r8, r10, r9\n\
         eor r12, r12, r2\n\
         eor r12, r12, r2, lsr #8\n\
         eor r12, r12, r2, lsr #16\n\
         eor r12, r12, r2, lsr #24\n\
         eor r12, r12, r8\n\
         add r10, r9, #8\n\
         ldr r3, =0xaabbccdd\n\
         str r3, [r10, #-4]!\n\
         sub r8, r10, r9\n\
         ldr r0, [r10]\n\
         eor r12, r12, r0\n\
         eor r12, r12, r0, lsr #8\n\
         eor r12, r12, r0, lsr #16\n\
         eor r12, r12, r0, lsr #24\n\
         eor r12, r12, r8\n\
         mov r10, r9\n\
         mov r4, #0x55\n\
         strb r4, [r10, #1]\n\
         ldrb r5, [r10, #1]\n\
         eor r12, r12, r5\n\
         mov r10, r9\n\
         mov r11, #2\n\
         ldr r6, [r10, r11, lsl #2]\n\
         eor r12, r12, r6\n\
         eor r12, r12, r6, lsr #8\n\
         eor r12, r12, r6, lsr #16\n\
         eor r12, r12, r6, lsr #24\n\
         add r10, r9, #12\n\
         mov r11, #1\n\
         ldr r7, =0x55667788\n\
         str r7, [r10, -r11, lsl #2]\n\
         ldr r0, [r9, #8]\n\
         eor r12, r12, r0\n\
         eor r12, r12, r0, lsr #8\n\
         eor r12, r12, r0, lsr #16\n\
         eor r0, r12, r0, lsr #24\n\
         .data\n\
         .align 2\n\
         data: .word 0, 0x01020304, 0x99aabbcc, 0\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    let base = 0x100;
    mem.load_arm_words(base, &[0, 0x0102_0304, 0x99aa_bbcc, 0])
        .unwrap();
    let mut folded = 0;

    cpu.set_reg(9, base);
    cpu.set_reg(1, 0x1122_3344);
    cpu.execute_arm(
        arm_single_transfer_imm(false, false, true, true, false, 9, 1, 0),
        0,
        &mut mem,
    )
    .unwrap(); // str r1, [r9]
    cpu.execute_arm(
        arm_single_transfer_imm(true, false, true, true, false, 9, 0, 0),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r0, [r9]
    folded ^= byte_fold(cpu.reg(0));

    cpu.set_reg(10, base + 16);
    cpu.execute_arm(
        arm_single_transfer_imm(true, false, true, false, false, 10, 0, 8),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r0, [r10, #-8]
    folded ^= byte_fold(cpu.reg(0));

    cpu.set_reg(10, base);
    cpu.execute_arm(
        arm_single_transfer_imm(true, false, false, true, false, 10, 2, 4),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r2, [r10], #4
    folded ^= byte_fold(cpu.reg(2));
    folded ^= cpu.reg(10).wrapping_sub(base);

    cpu.set_reg(10, base + 8);
    cpu.set_reg(3, 0xaabb_ccdd);
    cpu.execute_arm(
        arm_single_transfer_imm(false, false, true, false, true, 10, 3, 4),
        0,
        &mut mem,
    )
    .unwrap(); // str r3, [r10, #-4]!
    cpu.execute_arm(
        arm_single_transfer_imm(true, false, true, true, false, 10, 0, 0),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r0, [r10]
    folded ^= byte_fold(cpu.reg(0));
    folded ^= cpu.reg(10).wrapping_sub(base);

    cpu.set_reg(10, base);
    cpu.set_reg(4, 0x55);
    cpu.execute_arm(
        arm_single_transfer_imm(false, true, true, true, false, 10, 4, 1),
        0,
        &mut mem,
    )
    .unwrap(); // strb r4, [r10, #1]
    cpu.execute_arm(
        arm_single_transfer_imm(true, true, true, true, false, 10, 5, 1),
        0,
        &mut mem,
    )
    .unwrap(); // ldrb r5, [r10, #1]
    folded ^= cpu.reg(5);

    cpu.set_reg(10, base);
    cpu.set_reg(11, 2);
    cpu.execute_arm(
        arm_single_transfer_reg(true, false, true, true, false, 10, 6, 11, 0, 2),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r6, [r10, r11, lsl #2]
    folded ^= byte_fold(cpu.reg(6));

    cpu.set_reg(10, base + 12);
    cpu.set_reg(11, 1);
    cpu.set_reg(7, 0x5566_7788);
    cpu.execute_arm(
        arm_single_transfer_reg(false, false, true, false, false, 10, 7, 11, 0, 2),
        0,
        &mut mem,
    )
    .unwrap(); // str r7, [r10, -r11, lsl #2]
    cpu.execute_arm(
        arm_single_transfer_imm(true, false, true, true, false, 9, 0, 8),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r0, [r9, #8]
    folded ^= byte_fold(cpu.reg(0));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_ldrt_strt_matches_interpreter() {
    let asm = oracle_program(
        "ldr r2, =data\n\
         mov r1, #42\n\
         strt r1, [r2], #4\n\
         sub r2, r2, #4\n\
         ldrt r0, [r2], #4\n\
         .data\n\
         .align 2\n\
         data: .word 0\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    cpu.set_reg(1, 42);
    cpu.set_reg(2, 0x100);
    cpu.execute_arm(0xe4a2_1004, 0, &mut mem).unwrap(); // strt r1, [r2], #4
    cpu.set_reg(2, cpu.reg(2).wrapping_sub(4));
    cpu.execute_arm(0xe4b2_0004, 0, &mut mem).unwrap(); // ldrt r0, [r2], #4

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
    assert_eq!(cpu.reg(2), 0x104);
}

#[test]
fn qemu_oracle_signed_and_doubleword_transfers_match_interpreter() {
    let asm = oracle_program(
        "ldr r1, =data\n\
         ldrsb r0, [r1]\n\
         ldrsh r2, [r1, #2]\n\
         ldrd r4, r5, [r1, #4]\n\
         strd r4, r5, [r1, #12]\n\
         ldr r6, [r1, #12]\n\
         ldr r7, [r1, #16]\n\
         eor r0, r0, r2\n\
         eor r0, r0, r4\n\
         eor r0, r0, r5\n\
         eor r0, r0, r6\n\
         eor r0, r0, r7\n\
         .data\n\
         .align 2\n\
         data:\n\
         .byte 0x80, 0x00, 0x01, 0x80\n\
         .word 0x11223344, 0x55667788, 0, 0\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x40);
    mem.load_bytes(0, &[0x80, 0x00, 0x01, 0x80]).unwrap();
    mem.load_arm_words(4, &[0x1122_3344, 0x5566_7788, 0, 0])
        .unwrap();
    cpu.set_reg(1, 0);
    cpu.execute_arm(0xe1d1_00d0, 0, &mut mem).unwrap(); // ldrsb r0, [r1]
    cpu.execute_arm(0xe1d1_20f2, 0, &mut mem).unwrap(); // ldrsh r2, [r1, #2]
    cpu.execute_arm(0xe1c1_40d4, 0, &mut mem).unwrap(); // ldrd r4, r5, [r1, #4]
    cpu.execute_arm(0xe1c1_40fc, 0, &mut mem).unwrap(); // strd r4, r5, [r1, #12]
    cpu.execute_arm(0xe591_600c, 0, &mut mem).unwrap(); // ldr r6, [r1, #12]
    cpu.execute_arm(0xe591_7010, 0, &mut mem).unwrap(); // ldr r7, [r1, #16]
    cpu.set_reg(
        0,
        cpu.reg(0) ^ cpu.reg(2) ^ cpu.reg(4) ^ cpu.reg(5) ^ cpu.reg(6) ^ cpu.reg(7),
    );

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_halfword_transfer_addressing_matrix_matches_interpreter() {
    let asm = oracle_program(
        "mov r12, #0\n\
         ldr r9, =data\n\
         ldr r1, =0x00007788\n\
         strh r1, [r9]\n\
         ldrh r0, [r9]\n\
         eor r12, r12, r0\n\
         eor r12, r12, r0, lsr #8\n\
         eor r12, r12, r0, lsr #16\n\
         eor r12, r12, r0, lsr #24\n\
         add r10, r9, #4\n\
         ldrsh r2, [r10, #-2]!\n\
         sub r8, r10, r9\n\
         eor r12, r12, r2\n\
         eor r12, r12, r2, lsr #8\n\
         eor r12, r12, r2, lsr #16\n\
         eor r12, r12, r2, lsr #24\n\
         eor r12, r12, r8\n\
         mov r10, r9\n\
         mov r11, #1\n\
         ldrsb r3, [r10, r11]\n\
         eor r12, r12, r3\n\
         eor r12, r12, r3, lsr #8\n\
         eor r12, r12, r3, lsr #16\n\
         eor r12, r12, r3, lsr #24\n\
         mov r10, r9\n\
         ldr r4, =0x11223344\n\
         ldr r5, =0x55667788\n\
         strd r4, r5, [r10, #8]\n\
         ldrd r6, r7, [r10, #8]\n\
         eor r12, r12, r6\n\
         eor r12, r12, r6, lsr #8\n\
         eor r12, r12, r6, lsr #16\n\
         eor r12, r12, r6, lsr #24\n\
         eor r12, r12, r7\n\
         eor r12, r12, r7, lsr #8\n\
         eor r12, r12, r7, lsr #16\n\
         eor r12, r12, r7, lsr #24\n\
         add r10, r9, #24\n\
         ldrd r4, r5, [r10, #-8]\n\
         eor r12, r12, r4\n\
         eor r12, r12, r4, lsr #8\n\
         eor r12, r12, r4, lsr #16\n\
         eor r12, r12, r4, lsr #24\n\
         eor r12, r12, r5\n\
         eor r12, r12, r5, lsr #8\n\
         eor r12, r12, r5, lsr #16\n\
         eor r0, r12, r5, lsr #24\n\
         .data\n\
         .align 2\n\
         data:\n\
         .byte 0, 0, 0x80, 0xff\n\
         .word 0, 0, 0, 0x99aabbcc, 0xddeeff00\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    let base = 0x100;
    mem.load_bytes(base, &[0, 0, 0x80, 0xff]).unwrap();
    mem.load_arm_words(base + 4, &[0, 0, 0, 0x99aa_bbcc, 0xddee_ff00])
        .unwrap();
    let mut folded = 0;

    cpu.set_reg(9, base);
    cpu.set_reg(1, 0x7788);
    cpu.execute_arm(
        arm_halfword_transfer(true, true, true, false, false, 9, 1, 0b01, 0),
        0,
        &mut mem,
    )
    .unwrap(); // strh r1, [r9]
    cpu.execute_arm(
        arm_halfword_transfer(true, true, true, false, true, 9, 0, 0b01, 0),
        0,
        &mut mem,
    )
    .unwrap(); // ldrh r0, [r9]
    folded ^= byte_fold(cpu.reg(0));

    cpu.set_reg(10, base + 4);
    cpu.execute_arm(
        arm_halfword_transfer(true, false, true, true, true, 10, 2, 0b11, 2),
        0,
        &mut mem,
    )
    .unwrap(); // ldrsh r2, [r10, #-2]!
    folded ^= byte_fold(cpu.reg(2));
    folded ^= cpu.reg(10).wrapping_sub(base);

    cpu.set_reg(10, base);
    cpu.set_reg(11, 1);
    cpu.execute_arm(
        arm_halfword_transfer(true, true, false, false, true, 10, 3, 0b10, 11),
        0,
        &mut mem,
    )
    .unwrap(); // ldrsb r3, [r10, r11]
    folded ^= byte_fold(cpu.reg(3));

    cpu.set_reg(10, base);
    cpu.set_reg(4, 0x1122_3344);
    cpu.set_reg(5, 0x5566_7788);
    cpu.execute_arm(
        arm_halfword_transfer(true, true, true, false, false, 10, 4, 0b11, 8),
        0,
        &mut mem,
    )
    .unwrap(); // strd r4, r5, [r10, #8]
    cpu.execute_arm(
        arm_halfword_transfer(true, true, true, false, false, 10, 6, 0b10, 8),
        0,
        &mut mem,
    )
    .unwrap(); // ldrd r6, r7, [r10, #8]
    folded ^= byte_fold(cpu.reg(6));
    folded ^= byte_fold(cpu.reg(7));

    cpu.set_reg(10, base + 24);
    cpu.execute_arm(
        arm_halfword_transfer(true, false, true, false, false, 10, 4, 0b10, 8),
        0,
        &mut mem,
    )
    .unwrap(); // ldrd r4, r5, [r10, #-8]
    folded ^= byte_fold(cpu.reg(4));
    folded ^= byte_fold(cpu.reg(5));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_exclusive_word_success_matches_interpreter() {
    let asm = oracle_program(
        ".arch armv6k\n\
         ldr r1, =data\n\
         ldrex r0, [r1]\n\
         add r3, r0, #5\n\
         strex r2, r3, [r1]\n\
         ldr r4, [r1]\n\
         eor r0, r0, r2\n\
         eor r0, r0, r4\n\
         clrex\n\
         .data\n\
         .align 2\n\
         data: .word 37\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x20);
    mem.load_arm_words(0, &[37]).unwrap();
    cpu.set_reg(1, 0);
    cpu.execute_arm(0xe191_0f9f, 0, &mut mem).unwrap(); // ldrex r0, [r1]
    cpu.set_reg(3, cpu.reg(0).wrapping_add(5));
    cpu.execute_arm(0xe181_2f93, 0, &mut mem).unwrap(); // strex r2, r3, [r1]
    cpu.execute_arm(0xe591_4000, 0, &mut mem).unwrap(); // ldr r4, [r1]
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(2) ^ cpu.reg(4));
    cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_exclusive_word_failure_matches_interpreter() {
    let asm = oracle_program(
        ".arch armv6k\n\
         ldr r1, =data\n\
         ldr r2, =0x11223344\n\
         str r2, [r1]\n\
         ldrex r0, [r1]\n\
         clrex\n\
         ldr r3, =0xaabbccdd\n\
         strex r4, r3, [r1]\n\
         ldr r5, [r1]\n\
         eor r0, r4, r5\n\
         eor r0, r0, r5, lsr #8\n\
         eor r0, r0, r5, lsr #16\n\
         eor r0, r0, r5, lsr #24\n\
         .data\n\
         .align 2\n\
         data: .word 0\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0x1000, 0x100);
    mem.store32(0x1040, 0x1122_3344).unwrap();
    cpu.set_reg(1, 0x1040);
    cpu.execute_arm(0xe191_0f9f, 0, &mut mem).unwrap(); // ldrex r0, [r1]
    cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex
    cpu.set_reg(3, 0xaabb_ccdd);
    cpu.execute_arm(0xe181_4f93, 0, &mut mem).unwrap(); // strex r4, r3, [r1]
    let value = mem.load32(0x1040).unwrap();
    cpu.set_reg(0, cpu.reg(4) ^ byte_fold(value));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_exclusive_width_success_failure_matches_interpreter() {
    let asm = oracle_program(
        ".arch armv6k\n\
         ldr r12, =data\n\
         mov r11, #0\n\
         ldrexb r0, [r12]\n\
         mov r2, #0xa5\n\
         strexb r1, r2, [r12]\n\
         ldrb r3, [r12]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r1\n\
         eor r11, r11, r3\n\
         add r4, r12, #1\n\
         ldrexb r0, [r4]\n\
         clrex\n\
         mov r2, #0x5a\n\
         strexb r1, r2, [r4]\n\
         ldrb r3, [r4]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r1\n\
         eor r11, r11, r3\n\
         add r4, r12, #4\n\
         ldrexh r0, [r4]\n\
         ldr r2, =0xcafe\n\
         strexh r1, r2, [r4]\n\
         ldrh r3, [r4]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r0, lsr #8\n\
         eor r11, r11, r1\n\
         eor r11, r11, r3\n\
         eor r11, r11, r3, lsr #8\n\
         add r4, r12, #6\n\
         ldrexh r0, [r4]\n\
         clrex\n\
         ldr r2, =0xface\n\
         strexh r1, r2, [r4]\n\
         ldrh r3, [r4]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r0, lsr #8\n\
         eor r11, r11, r1\n\
         eor r11, r11, r3\n\
         eor r11, r11, r3, lsr #8\n\
         add r4, r12, #8\n\
         ldrexd r0, r1, [r4]\n\
         ldr r2, =0x01020304\n\
         ldr r3, =0x05060708\n\
         strexd r5, r2, r3, [r4]\n\
         ldr r6, [r4]\n\
         ldr r7, [r4, #4]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r0, lsr #8\n\
         eor r11, r11, r0, lsr #16\n\
         eor r11, r11, r0, lsr #24\n\
         eor r11, r11, r1\n\
         eor r11, r11, r1, lsr #8\n\
         eor r11, r11, r1, lsr #16\n\
         eor r11, r11, r1, lsr #24\n\
         eor r11, r11, r5\n\
         eor r11, r11, r6\n\
         eor r11, r11, r6, lsr #8\n\
         eor r11, r11, r6, lsr #16\n\
         eor r11, r11, r6, lsr #24\n\
         eor r11, r11, r7\n\
         eor r11, r11, r7, lsr #8\n\
         eor r11, r11, r7, lsr #16\n\
         eor r11, r11, r7, lsr #24\n\
         add r4, r12, #16\n\
         ldrexd r0, r1, [r4]\n\
         clrex\n\
         ldr r2, =0x10203040\n\
         ldr r3, =0x50607080\n\
         strexd r5, r2, r3, [r4]\n\
         ldr r6, [r4]\n\
         ldr r7, [r4, #4]\n\
         eor r11, r11, r0\n\
         eor r11, r11, r0, lsr #8\n\
         eor r11, r11, r0, lsr #16\n\
         eor r11, r11, r0, lsr #24\n\
         eor r11, r11, r1\n\
         eor r11, r11, r1, lsr #8\n\
         eor r11, r11, r1, lsr #16\n\
         eor r11, r11, r1, lsr #24\n\
         eor r11, r11, r5\n\
         eor r11, r11, r6\n\
         eor r11, r11, r6, lsr #8\n\
         eor r11, r11, r6, lsr #16\n\
         eor r11, r11, r6, lsr #24\n\
         eor r11, r11, r7\n\
         eor r11, r11, r7, lsr #8\n\
         eor r11, r11, r7, lsr #16\n\
         eor r11, r11, r7, lsr #24\n\
         mov r0, r11\n\
         .data\n\
         .align 3\n\
         data:\n\
         .byte 0x11\n\
         .byte 0x22\n\
         .byte 0, 0\n\
         .hword 0x3344\n\
         .hword 0x5566\n\
         .word 0x11223344\n\
         .word 0x55667788\n\
         .word 0x99aabbcc\n\
         .word 0xddeeff00\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0x2000, 0x100);
    let base = 0x2040;
    mem.store8(base, 0x11).unwrap();
    mem.store8(base + 1, 0x22).unwrap();
    mem.store16(base + 4, 0x3344).unwrap();
    mem.store16(base + 6, 0x5566).unwrap();
    mem.store32(base + 8, 0x1122_3344).unwrap();
    mem.store32(base + 12, 0x5566_7788).unwrap();
    mem.store32(base + 16, 0x99aa_bbcc).unwrap();
    mem.store32(base + 20, 0xddee_ff00).unwrap();

    let mut folded = 0;

    cpu.set_reg(12, base);
    cpu.execute_arm(0xe1dc_0f9f, 0, &mut mem).unwrap(); // ldrexb r0, [r12]
    cpu.set_reg(2, 0xa5);
    cpu.execute_arm(0xe1cc_1f92, 0, &mut mem).unwrap(); // strexb r1, r2, [r12]
    folded ^= cpu.reg(0) ^ cpu.reg(1) ^ u32::from(mem.load8(base).unwrap());

    cpu.set_reg(4, base + 1);
    cpu.execute_arm(0xe1d4_0f9f, 0, &mut mem).unwrap(); // ldrexb r0, [r4]
    cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex
    cpu.set_reg(2, 0x5a);
    cpu.execute_arm(0xe1c4_1f92, 0, &mut mem).unwrap(); // strexb r1, r2, [r4]
    folded ^= cpu.reg(0) ^ cpu.reg(1) ^ u32::from(mem.load8(base + 1).unwrap());

    cpu.set_reg(4, base + 4);
    cpu.execute_arm(0xe1f4_0f9f, 0, &mut mem).unwrap(); // ldrexh r0, [r4]
    cpu.set_reg(2, 0xcafe);
    cpu.execute_arm(0xe1e4_1f92, 0, &mut mem).unwrap(); // strexh r1, r2, [r4]
    folded ^=
        byte_fold(cpu.reg(0)) ^ cpu.reg(1) ^ byte_fold(u32::from(mem.load16(base + 4).unwrap()));

    cpu.set_reg(4, base + 6);
    cpu.execute_arm(0xe1f4_0f9f, 0, &mut mem).unwrap(); // ldrexh r0, [r4]
    cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex
    cpu.set_reg(2, 0xface);
    cpu.execute_arm(0xe1e4_1f92, 0, &mut mem).unwrap(); // strexh r1, r2, [r4]
    folded ^=
        byte_fold(cpu.reg(0)) ^ cpu.reg(1) ^ byte_fold(u32::from(mem.load16(base + 6).unwrap()));

    cpu.set_reg(4, base + 8);
    cpu.execute_arm(0xe1b4_0f9f, 0, &mut mem).unwrap(); // ldrexd r0, r1, [r4]
    cpu.set_reg(2, 0x0102_0304);
    cpu.set_reg(3, 0x0506_0708);
    cpu.execute_arm(0xe1a4_5f92, 0, &mut mem).unwrap(); // strexd r5, r2, r3, [r4]
    folded ^= byte_fold(cpu.reg(0))
        ^ byte_fold(cpu.reg(1))
        ^ cpu.reg(5)
        ^ byte_fold(mem.load32(base + 8).unwrap())
        ^ byte_fold(mem.load32(base + 12).unwrap());

    cpu.set_reg(4, base + 16);
    cpu.execute_arm(0xe1b4_0f9f, 0, &mut mem).unwrap(); // ldrexd r0, r1, [r4]
    cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex
    cpu.set_reg(2, 0x1020_3040);
    cpu.set_reg(3, 0x5060_7080);
    cpu.execute_arm(0xe1a4_5f92, 0, &mut mem).unwrap(); // strexd r5, r2, r3, [r4]
    folded ^= byte_fold(cpu.reg(0))
        ^ byte_fold(cpu.reg(1))
        ^ cpu.reg(5)
        ^ byte_fold(mem.load32(base + 16).unwrap())
        ^ byte_fold(mem.load32(base + 20).unwrap());

    assert_eq!(qemu_exit as u32, folded & 0xff);
}

#[test]
fn qemu_oracle_swap_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =data\n\
         ldr r2, =0x11223344\n\
         swp r0, r2, [r1]\n\
         mov r4, #0xaa\n\
         swpb r3, r4, [r1]\n\
         ldr r5, [r1]\n\
         eor r0, r0, r3\n\
         eor r0, r0, r5\n\
         .data\n\
         .align 2\n\
         data: .word 0x55667788\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x20);
    mem.load_arm_words(0, &[0x5566_7788]).unwrap();
    cpu.set_reg(1, 0);
    cpu.set_reg(2, 0x1122_3344);
    cpu.execute_arm(0xe101_0092, 0, &mut mem).unwrap(); // swp r0, r2, [r1]
    cpu.set_reg(4, 0xaa);
    cpu.execute_arm(0xe141_3094, 0, &mut mem).unwrap(); // swpb r3, r4, [r1]
    cpu.execute_arm(0xe591_5000, 0, &mut mem).unwrap(); // ldr r5, [r1]
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(3) ^ cpu.reg(5));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_msr_apsr_flags_match_interpreter() {
    let asm = oracle_program(
        "mov r0, #1\n\
         msr APSR_nzcvq, #0x40000000\n\
         moveq r0, #77",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(0, 1);
    cpu.execute_arm(0xe328_f101, 0, &mut mem).unwrap(); // msr APSR_nzcvq, #0x40000000
    cpu.execute_arm(0x03a0_004d, 0, &mut mem).unwrap(); // moveq r0, #77

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_arm_branch_condition_matrix_matches_interpreter() {
    let asm = oracle_program(
        "mov r6, #0\n\
         mov r0, #0\n\
         cmp r0, #0\n\
         beq 1f\n\
         b fail\n\
         1:\n\
         eor r6, r6, #1\n\
         cmp r0, #0\n\
         bne fail\n\
         eor r6, r6, #2\n\
         cmp r0, #0\n\
         bcs 2f\n\
         b fail\n\
         2:\n\
         eor r6, r6, #4\n\
         cmp r0, #1\n\
         bcc 3f\n\
         b fail\n\
         3:\n\
         eor r6, r6, #8\n\
         cmp r0, #1\n\
         bmi 4f\n\
         b fail\n\
         4:\n\
         eor r6, r6, #16\n\
         mov r1, #1\n\
         cmp r1, #0\n\
         bpl 5f\n\
         b fail\n\
         5:\n\
         eor r6, r6, #32\n\
         mov r5, #0\n\
         6:\n\
         cmp r5, #0\n\
         bne 7f\n\
         add r5, r5, #1\n\
         b 6b\n\
         7:\n\
         eor r6, r6, #64\n\
         bl 8f\n\
         eor r6, r6, #128\n\
         mov r0, r6\n\
         b done\n\
         8:\n\
         bx lr\n\
         fail:\n\
         mov r0, #0xee\n\
         done:",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0u32;
    let pc = 0x100;
    let target = 0x120;
    let fallthrough = pc + 4;
    let cases: [(u32, bool, bool, bool, bool, bool, u32); 6] = [
        (0, false, true, true, false, true, 1),
        (1, false, true, true, false, false, 2),
        (2, false, false, true, false, true, 4),
        (3, false, false, false, false, true, 8),
        (4, true, false, false, false, true, 16),
        (5, false, false, false, false, true, 32),
    ];

    for (cond, n, z, c, v, expected_taken, bit) in cases {
        cpu.set_pc(fallthrough);
        cpu.cpsr.n = n;
        cpu.cpsr.z = z;
        cpu.cpsr.c = c;
        cpu.cpsr.v = v;
        cpu.execute_arm(arm_branch_instr(cond, false, pc, target), pc, &mut mem)
            .unwrap();
        let taken = cpu.pc() == target;
        if taken == expected_taken {
            folded ^= bit;
        }
    }

    let pc = 0x200;
    let target = 0x180;
    cpu.set_pc(pc + 4);
    cpu.execute_arm(arm_branch_instr(0xe, false, pc, target), pc, &mut mem)
        .unwrap();
    if cpu.pc() == target {
        folded ^= 64;
    }

    let pc = 0x220;
    let target = 0x240;
    cpu.set_pc(pc + 4);
    cpu.execute_arm(arm_branch_instr(0xe, true, pc, target), pc, &mut mem)
        .unwrap();
    if cpu.pc() == target && cpu.reg(14) == pc + 4 {
        folded ^= 128;
    }

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_arm_data_processing_shift_matrix_matches_interpreter() {
    const CASES: &[(&str, u32, bool, u32, u32, u32, u32, bool)] = &[
        ("ands", 0x0, true, 0xf0f0_0ff0, 0x1020_3040, 0, 3, true),
        ("eors", 0x1, true, 0x55aa_33cc, 0xff00_0ff0, 1, 5, true),
        ("subs", 0x2, true, 0x8000_0010, 0x7000_0001, 2, 7, true),
        ("rsbs", 0x3, true, 0x1000_0000, 0x8000_0001, 3, 8, true),
        ("adds", 0x4, true, 0x7fff_fff0, 0x0000_0011, 0, 1, true),
        ("adcs", 0x5, true, 0xffff_fffe, 0x0000_0020, 1, 3, true),
        ("sbcs", 0x6, true, 0x0000_0010, 0x8000_0000, 2, 2, false),
        ("rscs", 0x7, true, 0x8000_0000, 0x0000_000f, 3, 4, true),
        ("tst", 0x8, false, 0xf00f_00f0, 0x1111_2222, 0, 4, true),
        ("teq", 0x9, false, 0xaaaa_5555, 0x1234_5678, 1, 6, true),
        ("cmp", 0xa, false, 0x0000_0001, 0x8000_0000, 2, 3, true),
        ("cmn", 0xb, false, 0x7fff_fff0, 0x0000_0010, 3, 12, true),
        ("orrs", 0xc, true, 0x0f0f_0000, 0x0000_f0f0, 0, 2, true),
        ("movs", 0xd, true, 0, 0x8000_0001, 1, 1, true),
        ("bics", 0xe, true, 0xffff_00ff, 0x00ff_00ff, 2, 5, true),
        ("mvns", 0xf, true, 0, 0x0000_ffff, 3, 16, true),
    ];

    let shift_suffix = |shift: u32, amount: u32| match shift {
        0 => format!("lsl #{amount}"),
        1 => format!("lsr #{amount}"),
        2 => format!("asr #{amount}"),
        3 => format!("ror #{amount}"),
        _ => unreachable!(),
    };

    let mut body = String::from("mov r12, #0\n");
    for (mnemonic, opcode, writes_result, lhs, rhs, shift, amount, carry_in) in CASES {
        let suffix = shift_suffix(*shift, *amount);
        body.push_str("mov r4, #0\n");
        if *carry_in {
            body.push_str("cmp r4, #0\n");
        } else {
            body.push_str("cmp r4, #1\n");
        }
        body.push_str(&format!(
            "ldr r1, ={lhs:#010x}\n\
             ldr r2, ={rhs:#010x}\n"
        ));
        match *opcode {
            0x8..=0xb => {
                body.push_str(&format!("{mnemonic} r1, r2, {suffix}\n"));
            }
            0xd | 0xf => {
                body.push_str(&format!("{mnemonic} r0, r2, {suffix}\n"));
            }
            _ => {
                body.push_str(&format!("{mnemonic} r0, r1, r2, {suffix}\n"));
            }
        }
        if *writes_result {
            body.push_str(
                "eor r12, r12, r0\n\
                 eor r12, r12, r0, lsr #8\n\
                 eor r12, r12, r0, lsr #16\n\
                 eor r12, r12, r0, lsr #24\n",
            );
        }
        body.push_str(
            "mrs r3, cpsr\n\
             eor r12, r12, r3, lsr #28\n",
        );
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (_, opcode, writes_result, lhs, rhs, shift, amount, carry_in) in CASES {
        cpu.set_reg(0, 0);
        cpu.set_reg(1, *lhs);
        cpu.set_reg(2, *rhs);
        cpu.cpsr.n = false;
        cpu.cpsr.z = false;
        cpu.cpsr.c = *carry_in;
        cpu.cpsr.v = false;
        cpu.execute_arm(
            arm_data_proc_reg_shift_instr(*opcode, true, 1, 0, 2, *shift, *amount),
            0,
            &mut mem,
        )
        .unwrap();
        if *writes_result {
            folded ^= byte_fold(cpu.reg(0));
        }
        folded ^= (cpu.cpsr.to_u32() >> 28) & 0xf;
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_arm_data_processing_immediate_matrix_matches_interpreter() {
    const CASES: &[(&str, u32, bool, u32, u32, bool)] = &[
        ("ands", 0x0, true, 0xf0f0_0ff0, 0x05a, true),
        ("eors", 0x1, true, 0x55aa_33cc, 0x480, true),
        ("subs", 0x2, true, 0x8000_0010, 0xc3f, true),
        ("rsbs", 0x3, true, 0x1000_0000, 0x2f0, true),
        ("adds", 0x4, true, 0x7fff_fff0, 0x011, true),
        ("adcs", 0x5, true, 0xffff_fffe, 0x220, true),
        ("sbcs", 0x6, true, 0x0000_0010, 0x380, false),
        ("rscs", 0x7, true, 0x8000_0000, 0x40f, true),
        ("tst", 0x8, false, 0xf00f_00f0, 0x522, true),
        ("teq", 0x9, false, 0xaaaa_5555, 0x678, true),
        ("cmp", 0xa, false, 0x0000_0001, 0x780, true),
        ("cmn", 0xb, false, 0x7fff_fff0, 0x810, true),
        ("orrs", 0xc, true, 0x0f0f_0000, 0x9f0, true),
        ("movs", 0xd, true, 0, 0xa81, true),
        ("bics", 0xe, true, 0xffff_00ff, 0xbff, true),
        ("mvns", 0xf, true, 0, 0xc55, true),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (mnemonic, opcode, writes_result, lhs, imm12, carry_in) in CASES {
        let imm = arm_expand_imm_value(*imm12);
        body.push_str("mov r4, #0\n");
        if *carry_in {
            body.push_str("cmp r4, #0\n");
        } else {
            body.push_str("cmp r4, #1\n");
        }
        body.push_str(&format!("ldr r1, ={lhs:#010x}\n"));
        match *opcode {
            0x8..=0xb => {
                body.push_str(&format!("{mnemonic} r1, #{imm:#010x}\n"));
            }
            0xd | 0xf => {
                body.push_str(&format!("{mnemonic} r0, #{imm:#010x}\n"));
            }
            _ => {
                body.push_str(&format!("{mnemonic} r0, r1, #{imm:#010x}\n"));
            }
        }
        if *writes_result {
            body.push_str(
                "eor r12, r12, r0\n\
                 eor r12, r12, r0, lsr #8\n\
                 eor r12, r12, r0, lsr #16\n\
                 eor r12, r12, r0, lsr #24\n",
            );
        }
        body.push_str(
            "mrs r3, cpsr\n\
             eor r12, r12, r3, lsr #28\n",
        );
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (_, opcode, writes_result, lhs, imm12, carry_in) in CASES {
        cpu.set_reg(0, 0);
        cpu.set_reg(1, *lhs);
        cpu.cpsr.n = false;
        cpu.cpsr.z = false;
        cpu.cpsr.c = *carry_in;
        cpu.cpsr.v = false;
        cpu.execute_arm(
            arm_data_proc_imm_instr(*opcode, true, 1, 0, *imm12),
            0,
            &mut mem,
        )
        .unwrap();
        if *writes_result {
            folded ^= byte_fold(cpu.reg(0));
        }
        folded ^= (cpu.cpsr.to_u32() >> 28) & 0xf;
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_pc_write_interworking_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r1, =1f + 1\n\
         mov pc, r1\n\
         .thumb\n\
         1:\n\
         movs r0, #77\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x2001);
    cpu.execute_arm(0xe1a0_f001, 0, &mut mem).unwrap(); // mov pc, r1
    assert!(cpu.cpsr.t);
    assert_eq!(cpu.pc(), 0x2000);
    cpu.execute_thumb(0x204d, 0x2000, &mut mem).unwrap(); // movs r0, #77

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_add_pc_interworking_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r4, =thumb_start + 1\n\
         bx r4\n\
         .thumb\n\
         thumb_start:\n\
         ldr r0, =1f + 1 - (.Ladd + 4)\n\
         .Ladd:\n\
         add pc, r0\n\
         movs r0, #1\n\
         b 2f\n\
         1:\n\
         movs r0, #88\n\
         2:\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    cpu.set_reg(0, 0x2001_u32.wrapping_sub((0x100_u32 + 4) & !3));
    cpu.execute_thumb(0x4487, 0x100, &mut mem).unwrap(); // add pc, r0
    assert!(cpu.cpsr.t);
    assert_eq!(cpu.pc(), 0x2000);
    cpu.execute_thumb(0x2058, 0x2000, &mut mem).unwrap(); // movs r0, #88

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_ldm_pc_interworking_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =target_ptr\n\
         ldmia r0!, {pc}\n\
         .align 2\n\
         target_ptr: .word 1f + 1\n\
         .thumb\n\
         1:\n\
         movs r0, #81\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    mem.load_arm_words(0x100, &[0x2001]).unwrap();
    cpu.set_reg(0, 0x100);
    cpu.execute_arm(0xe8b0_8000, 0, &mut mem).unwrap(); // ldmia r0!, {pc}
    assert_eq!(cpu.reg(0), 0x104);
    assert!(cpu.cpsr.t);
    assert_eq!(cpu.pc(), 0x2000);
    cpu.execute_thumb(0x2051, 0x2000, &mut mem).unwrap(); // movs r0, #81

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_block_transfer_matches_interpreter() {
    let asm = oracle_program(
        "ldr r0, =data\n\
         mov r1, #11\n\
         mov r2, #22\n\
         mov r3, #33\n\
         stmia r0!, {r1-r3}\n\
         ldr r0, =data\n\
         ldmia r0, {r4-r6}\n\
         eor r0, r4, r5\n\
         eor r0, r0, r6\n\
         .data\n\
         .align 2\n\
         data: .word 0, 0, 0\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x20);
    cpu.set_reg(0, 0);
    cpu.set_reg(1, 11);
    cpu.set_reg(2, 22);
    cpu.set_reg(3, 33);
    cpu.execute_arm(0xe8a0_000e, 0, &mut mem).unwrap(); // stmia r0!, {r1-r3}
    cpu.set_reg(0, 0);
    cpu.execute_arm(0xe890_0070, 0, &mut mem).unwrap(); // ldmia r0, {r4-r6}
    cpu.set_reg(0, cpu.reg(4) ^ cpu.reg(5) ^ cpu.reg(6));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_block_transfer_addressing_matrix_matches_interpreter() {
    let asm = oracle_program(
        "mov r12, #0\n\
         ldr r0, =data\n\
         mov r9, r0\n\
         mov r1, #11\n\
         mov r2, #22\n\
         mov r3, #33\n\
         stmia r9!, {r1-r3}\n\
         sub r8, r9, r0\n\
         ldmia r0, {r4-r6}\n\
         eor r12, r12, r4\n\
         eor r12, r12, r5\n\
         eor r12, r12, r6\n\
         eor r12, r12, r8\n\
         ldr r0, =data + 16\n\
         mov r9, r0\n\
         mov r1, #44\n\
         mov r2, #55\n\
         stmib r9!, {r1-r2}\n\
         sub r8, r9, r0\n\
         add r10, r0, #4\n\
         ldmia r10, {r4-r5}\n\
         eor r12, r12, r4\n\
         eor r12, r12, r5\n\
         eor r12, r12, r8\n\
         ldr r0, =data + 40\n\
         mov r9, r0\n\
         mov r1, #66\n\
         mov r2, #77\n\
         stmda r9!, {r1-r2}\n\
         sub r8, r0, r9\n\
         sub r10, r0, #4\n\
         ldmia r10, {r4-r5}\n\
         eor r12, r12, r4\n\
         eor r12, r12, r5\n\
         eor r12, r12, r8\n\
         ldr r0, =data + 60\n\
         mov r9, r0\n\
         mov r1, #88\n\
         mov r2, #99\n\
         stmdb r9!, {r1-r2}\n\
         sub r8, r0, r9\n\
         ldmia r9, {r4-r5}\n\
         eor r12, r12, r4\n\
         eor r12, r12, r5\n\
         eor r0, r12, r8\n\
         .data\n\
         .align 2\n\
         data: .space 80\n\
         .text",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    let base = 0x100;
    let mut folded = 0;

    cpu.set_reg(0, base);
    cpu.set_reg(9, cpu.reg(0));
    cpu.set_reg(1, 11);
    cpu.set_reg(2, 22);
    cpu.set_reg(3, 33);
    cpu.execute_arm(
        arm_block_transfer(false, true, true, false, 9, 0x000e),
        0,
        &mut mem,
    )
    .unwrap(); // stmia r9!, {r1-r3}
    folded ^= cpu.reg(9).wrapping_sub(cpu.reg(0));
    cpu.execute_arm(
        arm_block_transfer(false, true, false, true, 0, 0x0070),
        0,
        &mut mem,
    )
    .unwrap(); // ldmia r0, {r4-r6}
    folded ^= cpu.reg(4) ^ cpu.reg(5) ^ cpu.reg(6);

    cpu.set_reg(0, base + 16);
    cpu.set_reg(9, cpu.reg(0));
    cpu.set_reg(1, 44);
    cpu.set_reg(2, 55);
    cpu.execute_arm(
        arm_block_transfer(true, true, true, false, 9, 0x0006),
        0,
        &mut mem,
    )
    .unwrap(); // stmib r9!, {r1-r2}
    folded ^= cpu.reg(9).wrapping_sub(cpu.reg(0));
    cpu.set_reg(10, cpu.reg(0) + 4);
    cpu.execute_arm(
        arm_block_transfer(false, true, false, true, 10, 0x0030),
        0,
        &mut mem,
    )
    .unwrap(); // ldmia r10, {r4-r5}
    folded ^= cpu.reg(4) ^ cpu.reg(5);

    cpu.set_reg(0, base + 40);
    cpu.set_reg(9, cpu.reg(0));
    cpu.set_reg(1, 66);
    cpu.set_reg(2, 77);
    cpu.execute_arm(
        arm_block_transfer(false, false, true, false, 9, 0x0006),
        0,
        &mut mem,
    )
    .unwrap(); // stmda r9!, {r1-r2}
    folded ^= cpu.reg(0).wrapping_sub(cpu.reg(9));
    cpu.set_reg(10, cpu.reg(0) - 4);
    cpu.execute_arm(
        arm_block_transfer(false, true, false, true, 10, 0x0030),
        0,
        &mut mem,
    )
    .unwrap(); // ldmia r10, {r4-r5}
    folded ^= cpu.reg(4) ^ cpu.reg(5);

    cpu.set_reg(0, base + 60);
    cpu.set_reg(9, cpu.reg(0));
    cpu.set_reg(1, 88);
    cpu.set_reg(2, 99);
    cpu.execute_arm(
        arm_block_transfer(true, false, true, false, 9, 0x0006),
        0,
        &mut mem,
    )
    .unwrap(); // stmdb r9!, {r1-r2}
    folded ^= cpu.reg(0).wrapping_sub(cpu.reg(9));
    cpu.execute_arm(
        arm_block_transfer(false, true, false, true, 9, 0x0030),
        0,
        &mut mem,
    )
    .unwrap(); // ldmia r9, {r4-r5}
    folded ^= cpu.reg(4) ^ cpu.reg(5);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_alu_memory_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r0, #7\n\
         adds r0, #3\n\
         ldr r1, =data\n\
         str r0, [r1]\n\
         ldr r2, [r1]\n\
         movs r7, #1\n\
         mov r0, r2\n\
         svc #0\n\
         .data\n\
         .align 2\n\
         data: .word 0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x20);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    cpu.set_reg(1, 0);
    cpu.execute_thumb(0x2007, 0, &mut mem).unwrap(); // movs r0, #7
    cpu.execute_thumb(0x3003, 2, &mut mem).unwrap(); // adds r0, #3
    cpu.execute_thumb(0x6008, 4, &mut mem).unwrap(); // str r0, [r1]
    cpu.execute_thumb(0x680a, 6, &mut mem).unwrap(); // ldr r2, [r1]
    cpu.set_reg(0, cpu.reg(2));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_alu_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r6, #0\n\
         movs r0, #0xf0\n\
         movs r1, #0x33\n\
         ands r0, r1\n\
         eors r6, r0\n\
         movs r0, #0xf0\n\
         movs r1, #0x33\n\
         eors r0, r1\n\
         eors r6, r0\n\
         movs r0, #1\n\
         movs r1, #3\n\
         lsls r0, r1\n\
         eors r6, r0\n\
         movs r0, #0x80\n\
         movs r1, #2\n\
         lsrs r0, r1\n\
         eors r6, r0\n\
         movs r0, #0x80\n\
         movs r1, #2\n\
         asrs r0, r1\n\
         eors r6, r0\n\
         movs r7, #0\n\
         cmp r7, #0\n\
         movs r0, #1\n\
         movs r1, #2\n\
         adcs r0, r1\n\
         eors r6, r0\n\
         movs r7, #0\n\
         cmp r7, #0\n\
         movs r0, #5\n\
         movs r1, #2\n\
         sbcs r0, r1\n\
         eors r6, r0\n\
         movs r0, #0x81\n\
         movs r1, #1\n\
         rors r0, r1\n\
         eors r6, r0\n\
         movs r0, #5\n\
         negs r0, r0\n\
         eors r6, r0\n\
         movs r0, #0x30\n\
         movs r1, #0x0f\n\
         orrs r0, r1\n\
         eors r6, r0\n\
         movs r0, #7\n\
         movs r1, #9\n\
         muls r0, r1\n\
         eors r6, r0\n\
         movs r0, #0xf0\n\
         movs r1, #0x33\n\
         bics r0, r1\n\
         eors r6, r0\n\
         movs r0, #0\n\
         mvns r0, r0\n\
         eors r6, r0\n\
         movs r4, #0\n\
         movs r0, #0xf0\n\
         movs r1, #0x0f\n\
         tst r0, r1\n\
         bne 1f\n\
         movs r4, #1\n\
         1:\n\
         eors r6, r4\n\
         movs r4, #0\n\
         movs r0, #0x22\n\
         movs r1, #0x22\n\
         cmp r0, r1\n\
         bne 1f\n\
         movs r4, #2\n\
         1:\n\
         eors r6, r4\n\
         movs r4, #0\n\
         movs r0, #1\n\
         movs r1, #0\n\
         mvns r1, r1\n\
         cmn r0, r1\n\
         bne 1f\n\
         movs r4, #3\n\
         1:\n\
         eors r6, r4\n\
         mov r0, r6\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    let mut folded = 0;

    cpu.set_reg(0, 0xf0);
    cpu.set_reg(1, 0x33);
    cpu.execute_thumb(thumb_alu_instr(0, 1, 0), 0, &mut mem)
        .unwrap(); // ands r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0xf0);
    cpu.set_reg(1, 0x33);
    cpu.execute_thumb(thumb_alu_instr(1, 1, 0), 0, &mut mem)
        .unwrap(); // eors r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 1);
    cpu.set_reg(1, 3);
    cpu.execute_thumb(thumb_alu_instr(2, 1, 0), 0, &mut mem)
        .unwrap(); // lsls r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0x80);
    cpu.set_reg(1, 2);
    cpu.execute_thumb(thumb_alu_instr(3, 1, 0), 0, &mut mem)
        .unwrap(); // lsrs r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0x80);
    cpu.set_reg(1, 2);
    cpu.execute_thumb(thumb_alu_instr(4, 1, 0), 0, &mut mem)
        .unwrap(); // asrs r0, r1
    folded ^= cpu.reg(0);

    cpu.cpsr.c = true;
    cpu.set_reg(0, 1);
    cpu.set_reg(1, 2);
    cpu.execute_thumb(thumb_alu_instr(5, 1, 0), 0, &mut mem)
        .unwrap(); // adcs r0, r1
    folded ^= cpu.reg(0);

    cpu.cpsr.c = true;
    cpu.set_reg(0, 5);
    cpu.set_reg(1, 2);
    cpu.execute_thumb(thumb_alu_instr(6, 1, 0), 0, &mut mem)
        .unwrap(); // sbcs r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0x81);
    cpu.set_reg(1, 1);
    cpu.execute_thumb(thumb_alu_instr(7, 1, 0), 0, &mut mem)
        .unwrap(); // rors r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 5);
    cpu.execute_thumb(thumb_alu_instr(9, 0, 0), 0, &mut mem)
        .unwrap(); // negs r0, r0
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0x30);
    cpu.set_reg(1, 0x0f);
    cpu.execute_thumb(thumb_alu_instr(12, 1, 0), 0, &mut mem)
        .unwrap(); // orrs r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 7);
    cpu.set_reg(1, 9);
    cpu.execute_thumb(thumb_alu_instr(13, 1, 0), 0, &mut mem)
        .unwrap(); // muls r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0xf0);
    cpu.set_reg(1, 0x33);
    cpu.execute_thumb(thumb_alu_instr(14, 1, 0), 0, &mut mem)
        .unwrap(); // bics r0, r1
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0);
    cpu.execute_thumb(thumb_alu_instr(15, 0, 0), 0, &mut mem)
        .unwrap(); // mvns r0, r0
    folded ^= cpu.reg(0);

    cpu.set_reg(0, 0xf0);
    cpu.set_reg(1, 0x0f);
    cpu.execute_thumb(thumb_alu_instr(8, 1, 0), 0, &mut mem)
        .unwrap(); // tst r0, r1
    folded ^= u32::from(cpu.cpsr.z);

    cpu.set_reg(0, 0x22);
    cpu.set_reg(1, 0x22);
    cpu.execute_thumb(thumb_alu_instr(10, 1, 0), 0, &mut mem)
        .unwrap(); // cmp r0, r1
    folded ^= u32::from(cpu.cpsr.z) << 1;

    cpu.set_reg(0, 1);
    cpu.set_reg(1, u32::MAX);
    cpu.execute_thumb(thumb_alu_instr(11, 1, 0), 0, &mut mem)
        .unwrap(); // cmn r0, r1
    folded ^= if cpu.cpsr.z { 3 } else { 0 };

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_high_register_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         mov r0, #0x21\n\
         mov r1, #0x10\n\
         mov r2, #0\n\
         mov r8, r0\n\
         mov r9, r1\n\
         ldr r3, =thumb_start + 1\n\
         bx r3\n\
         .thumb\n\
         thumb_start:\n\
         mov r4, r8\n\
         eors r2, r4\n\
         add r8, r1\n\
         mov r4, r8\n\
         eors r2, r4\n\
         add r8, r9\n\
         mov r4, r8\n\
         eors r2, r4\n\
         mov r10, r8\n\
         mov r5, r10\n\
         eors r2, r5\n\
         movs r6, #0\n\
         cmp r8, r10\n\
         bne 1f\n\
         movs r6, #1\n\
         1:\n\
         eors r2, r6\n\
         movs r6, #0\n\
         cmp r9, r8\n\
         bcs 2f\n\
         movs r6, #2\n\
         2:\n\
         eors r2, r6\n\
         mov r0, r2\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    cpu.set_reg(1, 0x10);
    cpu.set_reg(8, 0x21);
    cpu.set_reg(9, 0x10);
    let mut folded = 0;

    cpu.execute_thumb(thumb_high_reg_instr(2, 8, 4), 0, &mut mem)
        .unwrap(); // mov r4, r8
    folded ^= cpu.reg(4);

    cpu.execute_thumb(thumb_high_reg_instr(0, 1, 8), 0, &mut mem)
        .unwrap(); // add r8, r1
    folded ^= cpu.reg(8);

    cpu.execute_thumb(thumb_high_reg_instr(0, 9, 8), 0, &mut mem)
        .unwrap(); // add r8, r9
    folded ^= cpu.reg(8);

    cpu.execute_thumb(thumb_high_reg_instr(2, 8, 10), 0, &mut mem)
        .unwrap(); // mov r10, r8
    cpu.execute_thumb(thumb_high_reg_instr(2, 10, 5), 0, &mut mem)
        .unwrap(); // mov r5, r10
    folded ^= cpu.reg(5);

    cpu.execute_thumb(thumb_high_reg_instr(1, 10, 8), 0, &mut mem)
        .unwrap(); // cmp r8, r10
    folded ^= u32::from(cpu.cpsr.z);

    cpu.execute_thumb(thumb_high_reg_instr(1, 8, 9), 0, &mut mem)
        .unwrap(); // cmp r9, r8
    folded ^= if cpu.cpsr.c { 0 } else { 2 };

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_shift_add_sub_immediate_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r6, #0\n\
         movs r0, #1\n\
         lsls r1, r0, #3\n\
         eors r6, r1\n\
         movs r0, #0x80\n\
         lsrs r1, r0, #2\n\
         eors r6, r1\n\
         movs r0, #0x80\n\
         lsrs r1, r0, #32\n\
         eors r6, r1\n\
         movs r0, #0x80\n\
         asrs r1, r0, #2\n\
         eors r6, r1\n\
         movs r0, #7\n\
         movs r1, #5\n\
         adds r2, r0, r1\n\
         eors r6, r2\n\
         adds r2, r0, #3\n\
         eors r6, r2\n\
         subs r2, r0, r1\n\
         eors r6, r2\n\
         subs r2, r0, #3\n\
         eors r6, r2\n\
         movs r0, #0x12\n\
         eors r6, r0\n\
         movs r0, #0x34\n\
         movs r4, #0\n\
         cmp r0, #0x34\n\
         bne 1f\n\
         movs r4, #1\n\
         1:\n\
         eors r6, r4\n\
         adds r0, #5\n\
         eors r6, r0\n\
         subs r0, #2\n\
         eors r6, r0\n\
         mov r0, r6\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    let mut folded = 0;

    cpu.set_reg(0, 1);
    cpu.execute_thumb(thumb_shift_imm_instr(0, 3, 0, 1), 0, &mut mem)
        .unwrap(); // lsls r1, r0, #3
    folded ^= cpu.reg(1);

    cpu.set_reg(0, 0x80);
    cpu.execute_thumb(thumb_shift_imm_instr(1, 2, 0, 1), 0, &mut mem)
        .unwrap(); // lsrs r1, r0, #2
    folded ^= cpu.reg(1);

    cpu.set_reg(0, 0x80);
    cpu.execute_thumb(thumb_shift_imm_instr(1, 0, 0, 1), 0, &mut mem)
        .unwrap(); // lsrs r1, r0, #32
    folded ^= cpu.reg(1);

    cpu.set_reg(0, 0x80);
    cpu.execute_thumb(thumb_shift_imm_instr(2, 2, 0, 1), 0, &mut mem)
        .unwrap(); // asrs r1, r0, #2
    folded ^= cpu.reg(1);

    cpu.set_reg(0, 7);
    cpu.set_reg(1, 5);
    cpu.execute_thumb(thumb_add_sub_instr(false, false, 1, 0, 2), 0, &mut mem)
        .unwrap(); // adds r2, r0, r1
    folded ^= cpu.reg(2);

    cpu.execute_thumb(thumb_add_sub_instr(true, false, 3, 0, 2), 0, &mut mem)
        .unwrap(); // adds r2, r0, #3
    folded ^= cpu.reg(2);

    cpu.execute_thumb(thumb_add_sub_instr(false, true, 1, 0, 2), 0, &mut mem)
        .unwrap(); // subs r2, r0, r1
    folded ^= cpu.reg(2);

    cpu.execute_thumb(thumb_add_sub_instr(true, true, 3, 0, 2), 0, &mut mem)
        .unwrap(); // subs r2, r0, #3
    folded ^= cpu.reg(2);

    cpu.execute_thumb(thumb_imm_instr(0, 0, 0x12), 0, &mut mem)
        .unwrap(); // movs r0, #0x12
    folded ^= cpu.reg(0);

    cpu.execute_thumb(thumb_imm_instr(0, 0, 0x34), 0, &mut mem)
        .unwrap(); // movs r0, #0x34
    cpu.execute_thumb(thumb_imm_instr(1, 0, 0x34), 0, &mut mem)
        .unwrap(); // cmp r0, #0x34
    folded ^= u32::from(cpu.cpsr.z);

    cpu.execute_thumb(thumb_imm_instr(2, 0, 5), 0, &mut mem)
        .unwrap(); // adds r0, #5
    folded ^= cpu.reg(0);

    cpu.execute_thumb(thumb_imm_instr(3, 0, 2), 0, &mut mem)
        .unwrap(); // subs r0, #2
    folded ^= cpu.reg(0);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_literal_pc_sp_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r6, #0\n\
         .align 2\n\
         literal_load:\n\
         ldr r0, [pc, #0]\n\
         b after_literal\n\
         .word 0x12345678\n\
         after_literal:\n\
         eors r6, r0\n\
         .align 2\n\
         pc_adds:\n\
         add r2, pc, #12\n\
         add r3, pc, #20\n\
         subs r3, r3, r2\n\
         eors r6, r3\n\
         ldr r5, =stack_top\n\
         mov sp, r5\n\
         add r4, sp, #20\n\
         subs r4, r4, r5\n\
         eors r6, r4\n\
         add sp, #16\n\
         add r4, sp, #0\n\
         subs r4, r4, r5\n\
         eors r6, r4\n\
         sub sp, #8\n\
         add r4, sp, #0\n\
         subs r4, r4, r5\n\
         eors r6, r4\n\
         mov r0, r6\n\
         movs r7, #1\n\
         svc #0\n\
         .align 2\n\
         stack:\n\
         .space 64\n\
         stack_top:\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0x100, 0x4000);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    let mut folded = 0;

    mem.store32(0x104, 0x1234_5678).unwrap();
    cpu.execute_thumb(thumb_literal_load(0, 0), 0x100, &mut mem)
        .unwrap(); // ldr r0, [pc, #0]
    folded ^= cpu.reg(0);

    cpu.execute_thumb(thumb_add_pc_sp(false, 2, 3), 0x108, &mut mem)
        .unwrap(); // add r2, pc, #12
    cpu.execute_thumb(thumb_add_pc_sp(false, 3, 5), 0x10a, &mut mem)
        .unwrap(); // add r3, pc, #20
    cpu.execute_thumb(thumb_add_sub_instr(false, true, 2, 3, 3), 0, &mut mem)
        .unwrap(); // subs r3, r3, r2
    folded ^= cpu.reg(3);

    cpu.set_reg(5, 0x3000);
    cpu.set_reg(13, 0x3000);
    cpu.execute_thumb(thumb_add_pc_sp(true, 4, 5), 0, &mut mem)
        .unwrap(); // add r4, sp, #20
    cpu.execute_thumb(thumb_add_sub_instr(false, true, 5, 4, 4), 0, &mut mem)
        .unwrap(); // subs r4, r4, r5
    folded ^= cpu.reg(4);

    cpu.execute_thumb(thumb_adjust_sp(false, 4), 0, &mut mem)
        .unwrap(); // add sp, #16
    cpu.execute_thumb(thumb_add_pc_sp(true, 4, 0), 0, &mut mem)
        .unwrap(); // add r4, sp, #0
    cpu.execute_thumb(thumb_add_sub_instr(false, true, 5, 4, 4), 0, &mut mem)
        .unwrap(); // subs r4, r4, r5
    folded ^= cpu.reg(4);

    cpu.execute_thumb(thumb_adjust_sp(true, 2), 0, &mut mem)
        .unwrap(); // sub sp, #8
    cpu.execute_thumb(thumb_add_pc_sp(true, 4, 0), 0, &mut mem)
        .unwrap(); // add r4, sp, #0
    cpu.execute_thumb(thumb_add_sub_instr(false, true, 5, 4, 4), 0, &mut mem)
        .unwrap(); // subs r4, r4, r5
    folded ^= cpu.reg(4);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_branch_condition_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r6, #0\n\
         movs r0, #0\n\
         cmp r0, #0\n\
         beq 1f\n\
         b fail\n\
         1:\n\
         movs r4, #1\n\
         eors r6, r4\n\
         cmp r0, #0\n\
         bne fail\n\
         movs r4, #2\n\
         eors r6, r4\n\
         cmp r0, #0\n\
         bcs 2f\n\
         b fail\n\
         2:\n\
         movs r4, #4\n\
         eors r6, r4\n\
         cmp r0, #1\n\
         bcc 3f\n\
         b fail\n\
         3:\n\
         movs r4, #8\n\
         eors r6, r4\n\
         cmp r0, #1\n\
         bmi 4f\n\
         b fail\n\
         4:\n\
         movs r4, #16\n\
         eors r6, r4\n\
         movs r1, #1\n\
         cmp r1, #0\n\
         bpl 5f\n\
         b fail\n\
         5:\n\
         movs r4, #32\n\
         eors r6, r4\n\
         b 6f\n\
         b fail\n\
         6:\n\
         movs r4, #64\n\
         eors r6, r4\n\
         movs r5, #0\n\
         7:\n\
         cmp r5, #0\n\
         bne 8f\n\
         adds r5, #1\n\
         b 7b\n\
         8:\n\
         movs r4, #128\n\
         eors r6, r4\n\
         mov r0, r6\n\
         movs r7, #1\n\
         svc #0\n\
         fail:\n\
         movs r0, #0xee\n\
         movs r7, #1\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    let mut folded = 0;
    let pc = 0x100;
    let target = 0x110;
    let fallthrough = pc + 2;
    let imm8 = ((target as i32 - (pc as i32 + 4)) / 2) as u16;
    let cases: [(u16, bool, bool, bool, bool, bool, u32); 6] = [
        (0, false, true, true, false, true, 1),
        (1, false, true, true, false, false, 2),
        (2, false, false, true, false, true, 4),
        (3, false, false, false, false, true, 8),
        (4, true, false, false, false, true, 16),
        (5, false, false, false, false, true, 32),
    ];

    for (cond, n, z, c, v, expected_taken, bit) in cases {
        cpu.set_pc(fallthrough);
        cpu.cpsr.n = n;
        cpu.cpsr.z = z;
        cpu.cpsr.c = c;
        cpu.cpsr.v = v;
        cpu.execute_thumb(thumb_cond_branch_instr(cond, imm8), pc, &mut mem)
            .unwrap();
        let taken = cpu.pc() == target;
        if taken == expected_taken {
            folded ^= bit;
        }
    }

    let pc = 0x120;
    let target = 0x130;
    let imm11 = ((target as i32 - (pc as i32 + 4)) / 2) as u16;
    cpu.set_pc(pc + 2);
    cpu.execute_thumb(thumb_uncond_branch_instr(imm11), pc, &mut mem)
        .unwrap();
    if cpu.pc() == target {
        folded ^= 64;
    }

    let pc = 0x140;
    let target = 0x130;
    let imm11 = (((target as i32 - (pc as i32 + 4)) / 2) & 0x7ff) as u16;
    cpu.set_pc(pc + 2);
    cpu.execute_thumb(thumb_uncond_branch_instr(imm11), pc, &mut mem)
        .unwrap();
    if cpu.pc() == target {
        folded ^= 128;
    }

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_load_store_matrix_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r6, #0\n\
         ldr r0, =data\n\
         ldr r1, =0x11223344\n\
         str r1, [r0, #0]\n\
         ldr r2, [r0, #0]\n\
         eors r6, r2\n\
         movs r3, #0x55\n\
         strb r3, [r0, #5]\n\
         ldrb r4, [r0, #5]\n\
         eors r6, r4\n\
         ldr r5, =0xffff8001\n\
         strh r5, [r0, #6]\n\
         ldrh r2, [r0, #6]\n\
         eors r6, r2\n\
         movs r7, #1\n\
         ldrsb r1, [r0, r7]\n\
         eors r6, r1\n\
         movs r7, #6\n\
         ldrsh r2, [r0, r7]\n\
         eors r6, r2\n\
         movs r7, #12\n\
         ldr r3, =0xaabbccdd\n\
         str r3, [r0, r7]\n\
         ldr r4, [r0, r7]\n\
         eors r6, r4\n\
         ldr r5, =stack\n\
         mov sp, r5\n\
         movs r5, #0x66\n\
         str r5, [sp, #0]\n\
         ldr r0, [sp, #0]\n\
         eors r6, r0\n\
         mov r0, r6\n\
         movs r7, #1\n\
         svc #0\n\
         .data\n\
         .align 2\n\
         data: .space 32\n\
         stack: .space 16\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x200);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    let base = 0x100;
    let stack = 0x180;
    let mut folded = 0;

    cpu.set_reg(0, base);
    cpu.set_reg(1, 0x1122_3344);
    cpu.execute_thumb(
        thumb_imm_word_byte_transfer(false, false, 0, 0, 1),
        0,
        &mut mem,
    )
    .unwrap(); // str r1, [r0, #0]
    cpu.execute_thumb(
        thumb_imm_word_byte_transfer(true, false, 0, 0, 2),
        0,
        &mut mem,
    )
    .unwrap(); // ldr r2, [r0, #0]
    folded ^= cpu.reg(2);

    cpu.set_reg(3, 0x55);
    cpu.execute_thumb(
        thumb_imm_word_byte_transfer(false, true, 5, 0, 3),
        0,
        &mut mem,
    )
    .unwrap(); // strb r3, [r0, #5]
    cpu.execute_thumb(
        thumb_imm_word_byte_transfer(true, true, 5, 0, 4),
        0,
        &mut mem,
    )
    .unwrap(); // ldrb r4, [r0, #5]
    folded ^= cpu.reg(4);

    cpu.set_reg(5, 0xffff_8001);
    cpu.execute_thumb(thumb_imm_halfword_transfer(false, 3, 0, 5), 0, &mut mem)
        .unwrap(); // strh r5, [r0, #6]
    cpu.execute_thumb(thumb_imm_halfword_transfer(true, 3, 0, 2), 0, &mut mem)
        .unwrap(); // ldrh r2, [r0, #6]
    folded ^= cpu.reg(2);

    cpu.set_reg(7, 1);
    cpu.execute_thumb(thumb_reg_offset_transfer(3, 7, 0, 1), 0, &mut mem)
        .unwrap(); // ldrsb r1, [r0, r7]
    folded ^= cpu.reg(1);

    cpu.set_reg(7, 6);
    cpu.execute_thumb(thumb_reg_offset_transfer(7, 7, 0, 2), 0, &mut mem)
        .unwrap(); // ldrsh r2, [r0, r7]
    folded ^= cpu.reg(2);

    cpu.set_reg(7, 12);
    cpu.set_reg(3, 0xaabb_ccdd);
    cpu.execute_thumb(thumb_reg_offset_transfer(0, 7, 0, 3), 0, &mut mem)
        .unwrap(); // str r3, [r0, r7]
    cpu.execute_thumb(thumb_reg_offset_transfer(4, 7, 0, 4), 0, &mut mem)
        .unwrap(); // ldr r4, [r0, r7]
    folded ^= cpu.reg(4);

    cpu.set_reg(13, stack);
    cpu.set_reg(5, 0x66);
    cpu.execute_thumb(thumb_sp_relative_transfer(false, 5, 0), 0, &mut mem)
        .unwrap(); // str r5, [sp, #0]
    cpu.execute_thumb(thumb_sp_relative_transfer(true, 0, 0), 0, &mut mem)
        .unwrap(); // ldr r0, [sp, #0]
    folded ^= cpu.reg(0);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_push_pop_matches_interpreter() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         movs r0, #11\n\
         movs r1, #22\n\
         push {r0, r1}\n\
         pop {r2, r3}\n\
         eors r2, r3\n\
         movs r7, #1\n\
         mov r0, r2\n\
         svc #0\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x40);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    cpu.set_reg(13, 0x20);
    cpu.execute_thumb(0x200b, 0, &mut mem).unwrap(); // movs r0, #11
    cpu.execute_thumb(0x2116, 2, &mut mem).unwrap(); // movs r1, #22
    cpu.execute_thumb(0xb403, 4, &mut mem).unwrap(); // push {r0, r1}
    cpu.execute_thumb(0xbc0c, 6, &mut mem).unwrap(); // pop {r2, r3}
    cpu.set_reg(0, cpu.reg(2) ^ cpu.reg(3));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_thumb_ldm_base_in_list_suppresses_writeback() {
    let asm = ".syntax unified\n\
         .arch armv6\n\
         .text\n\
         .arm\n\
         .global _start\n\
         _start:\n\
         ldr r0, =thumb_start + 1\n\
         bx r0\n\
         .thumb\n\
         thumb_start:\n\
         ldr r2, =data\n\
         mov r0, r2\n\
         .hword 0xc803\n\
         movs r3, #0x33\n\
         cmp r0, r3\n\
         beq 1f\n\
         movs r0, #1\n\
         b 2f\n\
         1:\n\
         movs r0, #0\n\
         2:\n\
         movs r7, #1\n\
         svc #0\n\
         .data\n\
         .align 2\n\
         data: .word 0x33, 0x44\n"
        .to_string();
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 0x20);
    cpu.set_isa(aemu::armv6::Isa::Thumb);
    mem.load_arm_words(0, &[0x33, 0x44]).unwrap();
    cpu.set_reg(0, 0);
    cpu.execute_thumb(0xc803, 0, &mut mem).unwrap(); // ldmia r0!, {r0, r1}
    cpu.set_reg(0, if cpu.reg(0) == 0x33 { 0 } else { 1 });

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_parallel_media_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x10f0ffff\n\
         ldr r2, =0xf0200102\n\
         uqadd8 r0, r1, r2\n\
         ldr r3, =0xffffffff\n\
         cmp r0, r3\n\
         movne r0, #3\n\
         bne 1f\n\
         ldr r12, =0x000a0000\n\
         msr APSR_nzcvqg, r12\n\
         ldr r1, =0x11111111\n\
         ldr r2, =0x22222222\n\
         sel r0, r1, r2\n\
         ldr r3, =0x11221122\n\
         cmp r0, r3\n\
         moveq r0, #66\n\
         movne r0, #4\n\
         1:",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(1, 0x10f0_ffff);
    cpu.set_reg(2, 0xf020_0102);
    cpu.execute_arm(0xe661_0f92, 0, &mut mem).unwrap(); // uqadd8 r0, r1, r2
    if cpu.reg(0) != 0xffff_ffff {
        cpu.set_reg(0, 3);
    } else {
        cpu.set_reg(12, 0x000a_0000);
        cpu.execute_arm(0xe12c_f00c, 0, &mut mem).unwrap(); // msr APSR_nzcvqg, r12
        cpu.set_reg(1, 0x1111_1111);
        cpu.set_reg(2, 0x2222_2222);
        cpu.execute_arm(0xe681_0fb2, 0, &mut mem).unwrap(); // sel r0, r1, r2
        cpu.set_reg(0, if cpu.reg(0) == 0x1122_1122 { 66 } else { 4 });
    }

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_parallel_media_ge_flags_match_interpreter() {
    const CASES: &[(&str, u32, u32, u32, u32)] = &[
        ("sadd16", 1, 0, 0x0001_ffff, 0x0001_0001),
        ("sasx", 1, 1, 0x0001_8000, 0x7fff_0002),
        ("ssax", 1, 2, 0x7fff_8000, 0x0001_ffff),
        ("ssub16", 1, 3, 0x8001_7fff, 0x0002_ffff),
        ("uadd8", 5, 4, 0xf010_20ff, 0x1020_f001),
        ("usub8", 5, 7, 0x9010_9010, 0x1020_1020),
        ("uasx", 5, 1, 0x0001_ffff, 0x0100_0002),
        ("usax", 5, 2, 0x0100_0002, 0x0001_ffff),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (mnemonic, _, _, rn, rm) in CASES {
        body.push_str(&format!(
            "ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n\
             {mnemonic} r0, r1, r2\n\
             mrs r3, cpsr\n\
             and r3, r3, #0x000f0000\n\
             eor r12, r12, r3, lsr #16\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (_, family, op, rn, rm) in CASES {
        cpu.cpsr.ge = 0;
        cpu.set_reg(1, *rn);
        cpu.set_reg(2, *rm);
        cpu.execute_arm(arm_parallel_media_instr(*family, *op, 0, 1, 2), 0, &mut mem)
            .unwrap();
        folded ^= u32::from(cpu.cpsr.ge);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_sel_uses_parallel_ge_flags_matches_interpreter() {
    const CASES: &[(u32, u32, u32, u32)] = &[
        (0x9010_9010, 0x1020_1020, 0xa1a2_a3a4, 0xb1b2_b3b4),
        (0x1020_3040, 0x0102_5001, 0x0102_0304, 0xf1f2_f3f4),
        (0x0010_ff80, 0x010f_7f90, 0x5566_7788, 0x1122_3344),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (lhs, rhs, from_ge, from_clear) in CASES {
        body.push_str(&format!(
            "ldr r1, ={lhs:#010x}\n\
             ldr r2, ={rhs:#010x}\n\
             usub8 r0, r1, r2\n\
             ldr r3, ={from_ge:#010x}\n\
             ldr r4, ={from_clear:#010x}\n\
             sel r5, r3, r4\n\
             eor r12, r12, r5\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (lhs, rhs, from_ge, from_clear) in CASES {
        cpu.set_reg(1, *lhs);
        cpu.set_reg(2, *rhs);
        cpu.execute_arm(arm_parallel_media_instr(5, 7, 0, 1, 2), 0, &mut mem)
            .unwrap(); // usub8 r0, r1, r2
        cpu.set_reg(3, *from_ge);
        cpu.set_reg(4, *from_clear);
        cpu.execute_arm(arm_sel_instr(5, 3, 4), 0, &mut mem)
            .unwrap(); // sel r5, r3, r4
        folded ^= cpu.reg(5);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_double_lane_moves_match_interpreter() {
    let asm = oracle_program(
        ".fpu vfp\n\
         ldr r7, =0x11223344\n\
         ldr r8, =0x55667788\n\
         vmov.32 d5[0], r7\n\
         vmov.32 d5[1], r8\n\
         vmov.32 r1, d5[0]\n\
         vmov.32 r0, d5[1]\n\
         eor r0, r0, r1",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(7, 0x1122_3344);
    cpu.execute_arm(0xee05_7b10, 0, &mut mem).unwrap(); // vmov.32 d5[0], r7
    cpu.set_reg(8, 0x5566_7788);
    cpu.execute_arm(0xee25_8b10, 0, &mut mem).unwrap(); // vmov.32 d5[1], r8
    cpu.execute_arm(0xee15_1b10, 0, &mut mem).unwrap(); // vmov.32 r1, d5[0]
    cpu.execute_arm(0xee35_0b10, 0, &mut mem).unwrap(); // vmov.32 r0, d5[1]
    cpu.set_reg(0, cpu.reg(0) ^ cpu.reg(1));

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_parallel_media_matrix_matches_interpreter() {
    let asm = oracle_program(
        "ldr r1, =0x7fff8001\n\
         ldr r2, =0x00018002\n\
         sadd16 r0, r1, r2\n\
         mov r12, r0\n\
         ldr r4, =0x80017fff\n\
         ldr r5, =0x0002ffff\n\
         ssub16 r3, r4, r5\n\
         eor r12, r12, r3\n\
         ldr r7, =0x00018000\n\
         ldr r8, =0x7fff0002\n\
         sasx r6, r7, r8\n\
         eor r12, r12, r6\n\
         ldr r10, =0x7fff8000\n\
         ldr r11, =0x0001ffff\n\
         ssax r9, r10, r11\n\
         eor r12, r12, r9\n\
         ldr r1, =0x01020304\n\
         ldr r2, =0x11121314\n\
         shadd8 r0, r1, r2\n\
         eor r12, r12, r0\n\
         ldr r4, =0xffff0001\n\
         ldr r5, =0x0001ffff\n\
         uadd16 r3, r4, r5\n\
         eor r12, r12, r3\n\
         ldr r7, =0x0001ffff\n\
         ldr r8, =0x01000002\n\
         uasx r6, r7, r8\n\
         eor r12, r12, r6\n\
         ldr r10, =0xf0102030\n\
         ldr r11, =0x10203040\n\
         uhsub8 r9, r10, r11\n\
         eor r12, r12, r9\n\
         ldr r1, =0x807f00ff\n\
         ldr r2, =0x0180ff01\n\
         qsub8 r0, r1, r2\n\
         eor r12, r12, r0\n\
         ldr r4, =0x00010000\n\
         ldr r5, =0x0002ffff\n\
         uqsub16 r3, r4, r5\n\
         eor r0, r12, r3",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);

    cpu.set_reg(1, 0x7fff_8001);
    cpu.set_reg(2, 0x0001_8002);
    cpu.execute_arm(0xe611_0f12, 0, &mut mem).unwrap(); // sadd16 r0, r1, r2
    let mut folded = cpu.reg(0);

    cpu.set_reg(4, 0x8001_7fff);
    cpu.set_reg(5, 0x0002_ffff);
    cpu.execute_arm(0xe614_3f75, 0, &mut mem).unwrap(); // ssub16 r3, r4, r5
    folded ^= cpu.reg(3);

    cpu.set_reg(7, 0x0001_8000);
    cpu.set_reg(8, 0x7fff_0002);
    cpu.execute_arm(0xe617_6f38, 0, &mut mem).unwrap(); // sasx r6, r7, r8
    folded ^= cpu.reg(6);

    cpu.set_reg(10, 0x7fff_8000);
    cpu.set_reg(11, 0x0001_ffff);
    cpu.execute_arm(0xe61a_9f5b, 0, &mut mem).unwrap(); // ssax r9, r10, r11
    folded ^= cpu.reg(9);

    cpu.set_reg(1, 0x0102_0304);
    cpu.set_reg(2, 0x1112_1314);
    cpu.execute_arm(0xe631_0f92, 0, &mut mem).unwrap(); // shadd8 r0, r1, r2
    folded ^= cpu.reg(0);

    cpu.set_reg(4, 0xffff_0001);
    cpu.set_reg(5, 0x0001_ffff);
    cpu.execute_arm(0xe654_3f15, 0, &mut mem).unwrap(); // uadd16 r3, r4, r5
    folded ^= cpu.reg(3);

    cpu.set_reg(7, 0x0001_ffff);
    cpu.set_reg(8, 0x0100_0002);
    cpu.execute_arm(0xe657_6f38, 0, &mut mem).unwrap(); // uasx r6, r7, r8
    folded ^= cpu.reg(6);

    cpu.set_reg(10, 0xf010_2030);
    cpu.set_reg(11, 0x1020_3040);
    cpu.execute_arm(0xe67a_9ffb, 0, &mut mem).unwrap(); // uhsub8 r9, r10, r11
    folded ^= cpu.reg(9);

    cpu.set_reg(1, 0x807f_00ff);
    cpu.set_reg(2, 0x0180_ff01);
    cpu.execute_arm(0xe621_0ff2, 0, &mut mem).unwrap(); // qsub8 r0, r1, r2
    folded ^= cpu.reg(0);

    cpu.set_reg(4, 0x0001_0000);
    cpu.set_reg(5, 0x0002_ffff);
    cpu.execute_arm(0xe664_3f75, 0, &mut mem).unwrap(); // uqsub16 r3, r4, r5
    folded ^= cpu.reg(3);
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_parallel_media_full_matrix_matches_interpreter() {
    const CASES: &[(&str, u32, u32)] = &[
        ("sadd16", 1, 0),
        ("sasx", 1, 1),
        ("ssax", 1, 2),
        ("ssub16", 1, 3),
        ("sadd8", 1, 4),
        ("ssub8", 1, 7),
        ("qadd16", 2, 0),
        ("qasx", 2, 1),
        ("qsax", 2, 2),
        ("qsub16", 2, 3),
        ("qadd8", 2, 4),
        ("qsub8", 2, 7),
        ("shadd16", 3, 0),
        ("shasx", 3, 1),
        ("shsax", 3, 2),
        ("shsub16", 3, 3),
        ("shadd8", 3, 4),
        ("shsub8", 3, 7),
        ("uadd16", 5, 0),
        ("uasx", 5, 1),
        ("usax", 5, 2),
        ("usub16", 5, 3),
        ("uadd8", 5, 4),
        ("usub8", 5, 7),
        ("uqadd16", 6, 0),
        ("uqasx", 6, 1),
        ("uqsax", 6, 2),
        ("uqsub16", 6, 3),
        ("uqadd8", 6, 4),
        ("uqsub8", 6, 7),
        ("uhadd16", 7, 0),
        ("uhasx", 7, 1),
        ("uhsax", 7, 2),
        ("uhsub16", 7, 3),
        ("uhadd8", 7, 4),
        ("uhsub8", 7, 7),
    ];

    let mut body = String::from("mov r12, #0\n");
    for (idx, (mnemonic, _, _)) in CASES.iter().enumerate() {
        let rn = 0x7f80_00ffu32
            .wrapping_add((idx as u32).wrapping_mul(0x0102_0305))
            .rotate_left((idx % 13) as u32);
        let rm = 0x0180_ff01u32
            .wrapping_add((idx as u32).wrapping_mul(0x1020_3041))
            .rotate_right((idx % 11) as u32);
        body.push_str(&format!(
            "ldr r1, ={rn:#010x}\n\
             ldr r2, ={rm:#010x}\n\
             {mnemonic} r0, r1, r2\n\
             eor r12, r12, r0\n"
        ));
    }
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;
    for (idx, (_, family, op)) in CASES.iter().enumerate() {
        let rn = 0x7f80_00ffu32
            .wrapping_add((idx as u32).wrapping_mul(0x0102_0305))
            .rotate_left((idx % 13) as u32);
        let rm = 0x0180_ff01u32
            .wrapping_add((idx as u32).wrapping_mul(0x1020_3041))
            .rotate_right((idx % 11) as u32);
        cpu.set_reg(1, rn);
        cpu.set_reg(2, rm);
        cpu.execute_arm(arm_parallel_media_instr(*family, *op, 0, 1, 2), 0, &mut mem)
            .unwrap();
        folded ^= cpu.reg(0);
    }
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_add_matches_interpreter() {
    let asm = oracle_program(
        "ldr r0, =0x3fc00000\n\
         ldr r1, =0x40100000\n\
         vmov s0, r0\n\
         vmov s1, r1\n\
         vadd.f32 s2, s0, s1\n\
         vmov r0, s2\n\
         lsr r0, r0, #20",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(0, 1.5f32.to_bits());
    cpu.execute_arm(0xee00_0a10, 0, &mut mem).unwrap(); // vmov s0, r0
    cpu.set_reg(1, 2.25f32.to_bits());
    cpu.execute_arm(0xee00_1a90, 0, &mut mem).unwrap(); // vmov s1, r1
    cpu.execute_arm(0xee30_1a20, 0, &mut mem).unwrap(); // vadd.f32 s2, s0, s1

    assert_eq!(qemu_exit as u32, (cpu.sreg(2) >> 20) & 0xff);
}

#[test]
fn qemu_oracle_vfp_single_arithmetic_matrix_matches_interpreter() {
    let fold_single = "vmov r10, {reg}\n\
         eor r12, r12, r10\n\
         eor r12, r12, r10, lsr #8\n\
         eor r12, r12, r10, lsr #16\n\
         eor r12, r12, r10, lsr #24\n";
    let mut body = String::from(
        ".fpu vfp\n\
         mov r12, #0\n\
         ldr r0, =0x3fc00000\n\
         vmov s0, r0\n\
         ldr r0, =0x40100000\n\
         vmov s1, r0\n\
         vadd.f32 s2, s0, s1\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s2"));
    body.push_str("vsub.f32 s3, s2, s1\n");
    body.push_str(&fold_single.replace("{reg}", "s3"));
    body.push_str("vmul.f32 s4, s2, s3\n");
    body.push_str(&fold_single.replace("{reg}", "s4"));
    body.push_str("vdiv.f32 s5, s4, s0\n");
    body.push_str(&fold_single.replace("{reg}", "s5"));
    body.push_str("vneg.f32 s6, s5\n");
    body.push_str(&fold_single.replace("{reg}", "s6"));
    body.push_str("vabs.f32 s7, s6\n");
    body.push_str(&fold_single.replace("{reg}", "s7"));
    body.push_str(
        "ldr r0, =0x41200000\n\
         vmov s0, r0\n\
         ldr r0, =0x40000000\n\
         vmov s1, r0\n\
         ldr r0, =0x40400000\n\
         vmov s2, r0\n\
         vmla.f32 s0, s1, s2\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s0"));
    body.push_str(
        "ldr r0, =0x41a00000\n\
         vmov s3, r0\n\
         ldr r0, =0x40800000\n\
         vmov s4, r0\n\
         ldr r0, =0x3fc00000\n\
         vmov s5, r0\n\
         vmls.f32 s3, s4, s5\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s3"));
    body.push_str(
        "ldr r0, =0x40000000\n\
         vmov s6, r0\n\
         ldr r0, =0x40400000\n\
         vmov s7, r0\n\
         ldr r0, =0x40800000\n\
         vmov s8, r0\n\
         vnmla.f32 s6, s7, s8\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s6"));
    body.push_str(
        "ldr r0, =0x41a00000\n\
         vmov s9, r0\n\
         ldr r0, =0x40400000\n\
         vmov s10, r0\n\
         ldr r0, =0x40800000\n\
         vmov s11, r0\n\
         vnmls.f32 s9, s10, s11\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s9"));
    body.push_str(
        "ldr r0, =0x40000000\n\
         vmov s13, r0\n\
         ldr r0, =0x40a00000\n\
         vmov s14, r0\n\
         vnmul.f32 s12, s13, s14\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s12"));
    body.push_str(
        "ldr r0, =0x41100000\n\
         vmov s16, r0\n\
         vsqrt.f32 s15, s16\n",
    );
    body.push_str(&fold_single.replace("{reg}", "s15"));
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;

    cpu.set_sreg(0, 1.5f32.to_bits());
    cpu.set_sreg(1, 2.25f32.to_bits());
    cpu.execute_arm(0xee30_1a20, 0, &mut mem).unwrap(); // vadd.f32 s2, s0, s1
    folded ^= byte_fold(cpu.sreg(2));

    cpu.execute_arm(0xee71_1a60, 0, &mut mem).unwrap(); // vsub.f32 s3, s2, s1
    folded ^= byte_fold(cpu.sreg(3));

    cpu.execute_arm(0xee21_2a21, 0, &mut mem).unwrap(); // vmul.f32 s4, s2, s3
    folded ^= byte_fold(cpu.sreg(4));

    cpu.execute_arm(0xeec2_2a00, 0, &mut mem).unwrap(); // vdiv.f32 s5, s4, s0
    folded ^= byte_fold(cpu.sreg(5));

    cpu.execute_arm(0xeeb1_3a62, 0, &mut mem).unwrap(); // vneg.f32 s6, s5
    folded ^= byte_fold(cpu.sreg(6));

    cpu.execute_arm(0xeef0_3ac3, 0, &mut mem).unwrap(); // vabs.f32 s7, s6
    folded ^= byte_fold(cpu.sreg(7));

    cpu.set_sreg(0, 10.0f32.to_bits());
    cpu.set_sreg(1, 2.0f32.to_bits());
    cpu.set_sreg(2, 3.0f32.to_bits());
    cpu.execute_arm(0xee00_0a81, 0, &mut mem).unwrap(); // vmla.f32 s0, s1, s2
    folded ^= byte_fold(cpu.sreg(0));

    cpu.set_sreg(3, 20.0f32.to_bits());
    cpu.set_sreg(4, 4.0f32.to_bits());
    cpu.set_sreg(5, 1.5f32.to_bits());
    cpu.execute_arm(0xee42_1a62, 0, &mut mem).unwrap(); // vmls.f32 s3, s4, s5
    folded ^= byte_fold(cpu.sreg(3));

    cpu.set_sreg(6, 2.0f32.to_bits());
    cpu.set_sreg(7, 3.0f32.to_bits());
    cpu.set_sreg(8, 4.0f32.to_bits());
    cpu.execute_arm(0xee13_3ac4, 0, &mut mem).unwrap(); // vnmla.f32 s6, s7, s8
    folded ^= byte_fold(cpu.sreg(6));

    cpu.set_sreg(9, 20.0f32.to_bits());
    cpu.set_sreg(10, 3.0f32.to_bits());
    cpu.set_sreg(11, 4.0f32.to_bits());
    cpu.execute_arm(0xee55_4a25, 0, &mut mem).unwrap(); // vnmls.f32 s9, s10, s11
    folded ^= byte_fold(cpu.sreg(9));

    cpu.set_sreg(13, 2.0f32.to_bits());
    cpu.set_sreg(14, 5.0f32.to_bits());
    cpu.execute_arm(0xee26_6ac7, 0, &mut mem).unwrap(); // vnmul.f32 s12, s13, s14
    folded ^= byte_fold(cpu.sreg(12));

    cpu.set_sreg(16, 9.0f32.to_bits());
    cpu.execute_arm(0xeef1_7ac8, 0, &mut mem).unwrap(); // vsqrt.f32 s15, s16
    folded ^= byte_fold(cpu.sreg(15));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_vmla_sqrt_matches_interpreter() {
    let asm = oracle_program(
        "ldr r0, =0x41200000\n\
         ldr r1, =0x40000000\n\
         ldr r2, =0x40400000\n\
         vmov s0, r0\n\
         vmov s1, r1\n\
         vmov s2, r2\n\
         vmla.f32 s0, s1, s2\n\
         vsqrt.f32 s3, s0\n\
         vmov r0, s3\n\
         lsr r0, r0, #20",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_sreg(0, 10.0f32.to_bits());
    cpu.set_sreg(1, 2.0f32.to_bits());
    cpu.set_sreg(2, 3.0f32.to_bits());
    cpu.execute_arm(0xee00_0a81, 0, &mut mem).unwrap(); // vmla.f32 s0, s1, s2
    cpu.execute_arm(0xeef1_1ac0, 0, &mut mem).unwrap(); // vsqrt.f32 s3, s0

    assert_eq!(qemu_exit as u32, (cpu.sreg(3) >> 20) & 0xff);
}

#[test]
fn qemu_oracle_vfp_double_arithmetic_matrix_matches_interpreter() {
    let fold_double = "vmov r10, r11, {reg}\n\
         eor r12, r12, r10\n\
         eor r12, r12, r10, lsr #8\n\
         eor r12, r12, r10, lsr #16\n\
         eor r12, r12, r10, lsr #24\n\
         eor r12, r12, r11\n\
         eor r12, r12, r11, lsr #8\n\
         eor r12, r12, r11, lsr #16\n\
         eor r12, r12, r11, lsr #24\n";
    let mut body = String::from(
        ".fpu vfp\n\
         mov r12, #0\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x3ff80000\n\
         vmov d0, r0, r1\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x40020000\n\
         vmov d1, r0, r1\n\
         vadd.f64 d2, d0, d1\n",
    );
    body.push_str(&fold_double.replace("{reg}", "d2"));
    body.push_str("vsub.f64 d3, d2, d1\n");
    body.push_str(&fold_double.replace("{reg}", "d3"));
    body.push_str("vmul.f64 d4, d2, d3\n");
    body.push_str(&fold_double.replace("{reg}", "d4"));
    body.push_str("vdiv.f64 d5, d4, d0\n");
    body.push_str(&fold_double.replace("{reg}", "d5"));
    body.push_str("vneg.f64 d6, d5\n");
    body.push_str(&fold_double.replace("{reg}", "d6"));
    body.push_str("vabs.f64 d7, d6\n");
    body.push_str(&fold_double.replace("{reg}", "d7"));
    body.push_str(
        "ldr r0, =0x00000000\n\
         ldr r1, =0x3ff00000\n\
         vmov d6, r0, r1\n\
         ldr r1, =0x40000000\n\
         vmov d7, r0, r1\n\
         ldr r1, =0x40080000\n\
         vmov d8, r0, r1\n\
         vmla.f64 d6, d7, d8\n",
    );
    body.push_str(&fold_double.replace("{reg}", "d6"));
    body.push_str(
        "ldr r0, =0x00000000\n\
         ldr r1, =0x40300000\n\
         vmov d10, r0, r1\n\
         vsqrt.f64 d9, d10\n",
    );
    body.push_str(&fold_double.replace("{reg}", "d9"));
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;

    cpu.set_dreg(0, 1.5f64.to_bits());
    cpu.set_dreg(1, 2.25f64.to_bits());
    cpu.execute_arm(0xee30_2b01, 0, &mut mem).unwrap(); // vadd.f64 d2, d0, d1
    folded ^= double_byte_fold(cpu.dreg(2));

    cpu.execute_arm(0xee32_3b41, 0, &mut mem).unwrap(); // vsub.f64 d3, d2, d1
    folded ^= double_byte_fold(cpu.dreg(3));

    cpu.execute_arm(0xee22_4b03, 0, &mut mem).unwrap(); // vmul.f64 d4, d2, d3
    folded ^= double_byte_fold(cpu.dreg(4));

    cpu.execute_arm(0xee84_5b00, 0, &mut mem).unwrap(); // vdiv.f64 d5, d4, d0
    folded ^= double_byte_fold(cpu.dreg(5));

    cpu.execute_arm(0xeeb1_6b45, 0, &mut mem).unwrap(); // vneg.f64 d6, d5
    folded ^= double_byte_fold(cpu.dreg(6));

    cpu.execute_arm(0xeeb0_7bc6, 0, &mut mem).unwrap(); // vabs.f64 d7, d6
    folded ^= double_byte_fold(cpu.dreg(7));

    cpu.set_dreg(6, 1.0f64.to_bits());
    cpu.set_dreg(7, 2.0f64.to_bits());
    cpu.set_dreg(8, 3.0f64.to_bits());
    cpu.execute_arm(0xee07_6b08, 0, &mut mem).unwrap(); // vmla.f64 d6, d7, d8
    folded ^= double_byte_fold(cpu.dreg(6));

    cpu.set_dreg(10, 16.0f64.to_bits());
    cpu.execute_arm(0xeeb1_9bca, 0, &mut mem).unwrap(); // vsqrt.f64 d9, d10
    folded ^= double_byte_fold(cpu.dreg(9));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_load_store_matrix_matches_interpreter() {
    let mut body = String::from(
        ".fpu vfp\n\
         mov r12, #0\n\
         ldr r0, =data\n\
         ldr r2, =0x40d00000\n\
         str r2, [r0]\n\
         vldr s0, [r0]\n",
    );
    let fold_reg = |body: &mut String, reg: &str| {
        body.push_str(&format!(
            "eor r12, r12, {reg}\n\
             eor r12, r12, {reg}, lsr #8\n\
             eor r12, r12, {reg}, lsr #16\n\
             eor r12, r12, {reg}, lsr #24\n"
        ));
    };
    let fold_single = |body: &mut String, reg: &str| {
        body.push_str(&format!("vmov r10, {reg}\n"));
        fold_reg(body, "r10");
    };
    let fold_double = |body: &mut String, reg: &str| {
        body.push_str(&format!("vmov r10, r11, {reg}\n"));
        fold_reg(body, "r10");
        fold_reg(body, "r11");
    };

    fold_single(&mut body, "s0");
    body.push_str(
        "ldr r2, =0x40e80000\n\
         vmov s1, r2\n\
         vstr s1, [r0, #4]\n\
         ldr r10, [r0, #4]\n",
    );
    fold_reg(&mut body, "r10");
    body.push_str(
        "ldr r2, =0x00000000\n\
         ldr r3, =0x40210000\n\
         str r2, [r0, #8]\n\
         str r3, [r0, #12]\n\
         vldr d0, [r0, #8]\n",
    );
    fold_double(&mut body, "d0");
    body.push_str(
        "ldr r2, =0x00000000\n\
         ldr r3, =0x40228000\n\
         vmov d1, r2, r3\n\
         vstr d1, [r0, #16]\n\
         ldr r10, [r0, #16]\n\
         ldr r11, [r0, #20]\n",
    );
    fold_reg(&mut body, "r10");
    fold_reg(&mut body, "r11");
    body.push_str(
        "ldr r1, =data + 32\n\
         ldr r2, =0x41200000\n\
         vmov s0, r2\n\
         ldr r2, =0x41300000\n\
         vmov s1, r2\n\
         ldr r2, =0x41400000\n\
         vmov s2, r2\n\
         ldr r2, =0x41500000\n\
         vmov s3, r2\n\
         vstmia r1, {s0-s3}\n\
         mov r2, #0\n\
         vmov s0, r2\n\
         vmov s1, r2\n\
         vmov s2, r2\n\
         vmov s3, r2\n\
         vldmia r1, {s0-s3}\n",
    );
    for reg in ["s0", "s1", "s2", "s3"] {
        fold_single(&mut body, reg);
    }
    body.push_str(
        "ldr r0, =data + 64\n\
         mov r6, r0\n\
         ldr r2, =0x00000000\n\
         ldr r3, =0x3ff40000\n\
         vmov d0, r2, r3\n\
         ldr r3, =0x40040000\n\
         vmov d1, r2, r3\n\
         vstmia r0!, {d0-d1}\n\
         sub r7, r0, r6\n",
    );
    fold_reg(&mut body, "r7");
    body.push_str(
        "ldr r0, =data + 64\n\
         mov r6, r0\n\
         vmov d0, r2, r2\n\
         vmov d1, r2, r2\n\
         vldmia r0!, {d0-d1}\n\
         sub r7, r0, r6\n",
    );
    fold_reg(&mut body, "r7");
    fold_double(&mut body, "d0");
    fold_double(&mut body, "d1");
    body.push_str(
        "mov r0, r12\n\
         .pushsection .data\n\
         .align 3\n\
         data:\n\
         .space 128\n\
         .popsection\n",
    );

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0x1000, 0x200);
    let base = 0x1000;
    let mut folded = 0;

    cpu.set_reg(0, base);
    mem.store32(base, 6.5f32.to_bits()).unwrap();
    cpu.execute_arm(0xed90_0a00, 0, &mut mem).unwrap(); // vldr s0, [r0]
    folded ^= byte_fold(cpu.sreg(0));

    cpu.set_sreg(1, 7.25f32.to_bits());
    cpu.execute_arm(0xedc0_0a01, 0, &mut mem).unwrap(); // vstr s1, [r0, #4]
    folded ^= byte_fold(mem.load32(base + 4).unwrap());

    mem.store32(base + 8, 8.5f64.to_bits() as u32).unwrap();
    mem.store32(base + 12, (8.5f64.to_bits() >> 32) as u32)
        .unwrap();
    cpu.execute_arm(0xed90_0b02, 0, &mut mem).unwrap(); // vldr d0, [r0, #8]
    folded ^= double_byte_fold(cpu.dreg(0));

    cpu.set_dreg(1, 9.25f64.to_bits());
    cpu.execute_arm(0xed80_1b04, 0, &mut mem).unwrap(); // vstr d1, [r0, #16]
    let stored_d1 = u64::from(mem.load32(base + 16).unwrap())
        | (u64::from(mem.load32(base + 20).unwrap()) << 32);
    folded ^= double_byte_fold(stored_d1);

    cpu.set_reg(1, base + 32);
    for idx in 0..4 {
        cpu.set_sreg(idx, (idx as f32 + 10.0).to_bits());
    }
    cpu.execute_arm(0xec81_0a04, 0, &mut mem).unwrap(); // vstmia r1, {s0-s3}
    for idx in 0..4 {
        cpu.set_sreg(idx, 0);
    }
    cpu.execute_arm(0xec91_0a04, 0, &mut mem).unwrap(); // vldmia r1, {s0-s3}
    for idx in 0..4 {
        folded ^= byte_fold(cpu.sreg(idx));
    }

    cpu.set_reg(0, base + 64);
    cpu.set_dreg(0, 1.25f64.to_bits());
    cpu.set_dreg(1, 2.5f64.to_bits());
    cpu.execute_arm(0xeca0_0b04, 0, &mut mem).unwrap(); // vstmia r0!, {d0-d1}
    folded ^= byte_fold(cpu.reg(0).wrapping_sub(base + 64));

    cpu.set_reg(0, base + 64);
    cpu.set_dreg(0, 0);
    cpu.set_dreg(1, 0);
    cpu.execute_arm(0xecb0_0b04, 0, &mut mem).unwrap(); // vldmia r0!, {d0-d1}
    folded ^= byte_fold(cpu.reg(0).wrapping_sub(base + 64));
    folded ^= double_byte_fold(cpu.dreg(0));
    folded ^= double_byte_fold(cpu.dreg(1));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_odd_double_multiple_writeback_matches_interpreter() {
    let mut body = String::from(
        ".fpu vfp\n\
         mov r12, #0\n\
         ldr r0, =data\n\
         mov r6, r0\n\
         ldr r2, =0x11223344\n\
         ldr r3, =0x55667788\n\
         vmov d0, r2, r3\n\
         .word 0xeca00b03\n\
         sub r4, r0, r6\n\
         ldr r5, [r6]\n\
         ldr r7, [r6, #4]\n",
    );
    let fold_reg = |body: &mut String, reg: &str| {
        body.push_str(&format!(
            "eor r12, r12, {reg}\n\
             eor r12, r12, {reg}, lsr #8\n\
             eor r12, r12, {reg}, lsr #16\n\
             eor r12, r12, {reg}, lsr #24\n"
        ));
    };
    fold_reg(&mut body, "r4");
    fold_reg(&mut body, "r5");
    fold_reg(&mut body, "r7");
    body.push_str(
        "ldr r0, =data + 16\n\
         mov r6, r0\n\
         .word 0xecb01b03\n\
         sub r4, r0, r6\n\
         vmov r5, r7, d1\n",
    );
    fold_reg(&mut body, "r4");
    fold_reg(&mut body, "r5");
    fold_reg(&mut body, "r7");
    body.push_str(
        "mov r0, r12\n\
         .pushsection .data\n\
         .align 3\n\
         data:\n\
         .space 16\n\
         .word 0x99aabbcc\n\
         .word 0xddeeff00\n\
         .space 8\n\
         .popsection\n",
    );

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0x3000, 0x100);
    let base = 0x3000;
    let mut folded = 0;

    cpu.set_reg(0, base);
    cpu.set_dreg(0, 0x5566_7788_1122_3344);
    cpu.execute_arm(0xeca0_0b03, 0, &mut mem).unwrap(); // vstmia r0!, odd imm8=3
    folded ^= byte_fold(cpu.reg(0).wrapping_sub(base));
    folded ^= byte_fold(mem.load32(base).unwrap());
    folded ^= byte_fold(mem.load32(base + 4).unwrap());

    cpu.set_reg(0, base + 16);
    mem.store32(base + 16, 0x99aa_bbcc).unwrap();
    mem.store32(base + 20, 0xddee_ff00).unwrap();
    cpu.execute_arm(0xecb0_1b03, 0, &mut mem).unwrap(); // vldmia r0!, odd imm8=3
    folded ^= byte_fold(cpu.reg(0).wrapping_sub(base + 16));
    folded ^= double_byte_fold(cpu.dreg(1));

    assert_eq!(qemu_exit as u32, folded & 0xff);
}

#[test]
fn qemu_oracle_vcvt_matches_interpreter() {
    let asm = oracle_program(
        "ldr r0, =0x40600000\n\
         vmov s0, r0\n\
         vcvt.s32.f32 s1, s0\n\
         vmov r0, s1",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(0, 3.5f32.to_bits());
    cpu.execute_arm(0xee00_0a10, 0, &mut mem).unwrap(); // vmov s0, r0
    cpu.execute_arm(0xeefd_0ae0, 0, &mut mem).unwrap(); // vcvt.s32.f32 s1, s0
    cpu.execute_arm(0xee10_0a90, 0, &mut mem).unwrap(); // vmov r0, s1

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_double_conversion_matrix_matches_interpreter() {
    let mut body = String::from(
        ".fpu vfp\n\
         mov r12, #0\n\
         ldr r0, =0x3fa00000\n\
         vmov s1, r0\n\
         vcvt.f64.f32 d0, s1\n",
    );
    let fold_reg = |body: &mut String, reg: &str| {
        body.push_str(&format!(
            "eor r12, r12, {reg}\n\
             eor r12, r12, {reg}, lsr #8\n\
             eor r12, r12, {reg}, lsr #16\n\
             eor r12, r12, {reg}, lsr #24\n"
        ));
    };
    let fold_single = |body: &mut String, reg: &str| {
        body.push_str(&format!("vmov r10, {reg}\n"));
        fold_reg(body, "r10");
    };
    let fold_double = |body: &mut String, reg: &str| {
        body.push_str(&format!("vmov r10, r11, {reg}\n"));
        fold_reg(body, "r10");
        fold_reg(body, "r11");
    };

    fold_double(&mut body, "d0");
    body.push_str(
        "ldr r0, =0x00000000\n\
         ldr r1, =0x40040000\n\
         vmov d1, r0, r1\n\
         vcvt.f32.f64 s2, d1\n",
    );
    fold_single(&mut body, "s2");
    body.push_str(
        "ldr r0, =0x00000000\n\
         ldr r1, =0x40130000\n\
         vmov d2, r0, r1\n\
         vcvt.s32.f64 s8, d2\n",
    );
    fold_single(&mut body, "s8");
    body.push_str(
        "mov r2, #0\n\
         vmsr fpscr, r2\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x40040000\n\
         vmov d2, r0, r1\n\
         vcvtr.s32.f64 s8, d2\n",
    );
    fold_single(&mut body, "s8");
    body.push_str(
        "ldr r2, =0x00400000\n\
         vmsr fpscr, r2\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x40020000\n\
         vmov d2, r0, r1\n\
         vcvtr.u32.f64 s9, d2\n",
    );
    fold_single(&mut body, "s9");
    body.push_str(
        "mov r0, #4\n\
         vmov s8, r0\n\
         vcvt.f64.s32 d3, s8\n",
    );
    fold_double(&mut body, "d3");
    body.push_str("mov r0, r12");

    let asm = oracle_program(&body);
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;

    cpu.set_sreg(1, 1.25f32.to_bits());
    cpu.execute_arm(0xeeb7_0ae0, 0, &mut mem).unwrap(); // vcvt.f64.f32 d0, s1
    folded ^= double_byte_fold(cpu.dreg(0));

    cpu.set_dreg(1, 2.5f64.to_bits());
    cpu.execute_arm(0xeeb7_1bc1, 0, &mut mem).unwrap(); // vcvt.f32.f64 s2, d1
    folded ^= byte_fold(cpu.sreg(2));

    cpu.set_dreg(2, 4.75f64.to_bits());
    cpu.execute_arm(0xeebd_4bc2, 0, &mut mem).unwrap(); // vcvt.s32.f64 s8, d2
    folded ^= byte_fold(cpu.sreg(8));

    cpu.set_reg(2, 0);
    cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
    cpu.set_dreg(2, 2.5f64.to_bits());
    cpu.execute_arm(0xeebd_4b42, 0, &mut mem).unwrap(); // vcvtr.s32.f64 s8, d2
    folded ^= byte_fold(cpu.sreg(8));

    cpu.set_reg(2, 1 << 22);
    cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
    cpu.set_dreg(2, 2.25f64.to_bits());
    cpu.execute_arm(0xeefc_4b42, 0, &mut mem).unwrap(); // vcvtr.u32.f64 s9, d2
    folded ^= byte_fold(cpu.sreg(9));

    cpu.set_sreg(8, 4);
    cpu.execute_arm(0xeeb8_3bc4, 0, &mut mem).unwrap(); // vcvt.f64.s32 d3, s8
    folded ^= double_byte_fold(cpu.dreg(3));

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vcvtr_uses_fpscr_rounding_mode() {
    let asm = oracle_program(
        ".fpu vfp\n\
         mov r2, #0\n\
         vmsr fpscr, r2\n\
         ldr r1, =0x40200000\n\
         vmov s0, r1\n\
         vcvtr.s32.f32 s1, s0\n\
         vmov r0, s1\n\
         ldr r1, =0x40600000\n\
         vmov s0, r1\n\
         vcvtr.s32.f32 s1, s0\n\
         vmov r3, s1\n\
         eor r0, r0, r3\n\
         ldr r2, =0x00400000\n\
         vmsr fpscr, r2\n\
         ldr r1, =0x40100000\n\
         vmov s0, r1\n\
         vcvtr.u32.f32 s1, s0\n\
         vmov r3, s1\n\
         eor r0, r0, r3",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    cpu.set_reg(2, 0);
    cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
    cpu.set_sreg(0, 2.5f32.to_bits());
    cpu.execute_arm(0xeefd_0a40, 0, &mut mem).unwrap(); // vcvtr.s32.f32 s1, s0
    let mut folded = cpu.sreg(1);

    cpu.set_sreg(0, 3.5f32.to_bits());
    cpu.execute_arm(0xeefd_0a40, 0, &mut mem).unwrap(); // vcvtr.s32.f32 s1, s0
    folded ^= cpu.sreg(1);

    cpu.set_reg(2, 1 << 22);
    cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
    cpu.set_sreg(0, 2.25f32.to_bits());
    cpu.execute_arm(0xeefc_0a40, 0, &mut mem).unwrap(); // vcvtr.u32.f32 s1, s0
    folded ^= cpu.sreg(1);
    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}

#[test]
fn qemu_oracle_vfp_compare_matrix_matches_interpreter() {
    let asm = oracle_program(
        "mov r12, #0\n\
         ldr r0, =0x3f800000\n\
         vmov s0, r0\n\
         ldr r0, =0x40000000\n\
         vmov s1, r0\n\
         vcmp.f32 s0, s1\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x40400000\n\
         vmov s2, r0\n\
         ldr r0, =0x40400000\n\
         vmov s3, r0\n\
         vcmp.f32 s2, s3\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x40800000\n\
         vmov s4, r0\n\
         ldr r0, =0xbf800000\n\
         vmov s5, r0\n\
         vcmp.f32 s4, s5\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x7fc00000\n\
         vmov s6, r0\n\
         ldr r0, =0x3f800000\n\
         vmov s7, r0\n\
         vcmp.f32 s6, s7\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x80000000\n\
         vmov s8, r0\n\
         vcmp.f32 s8, #0.0\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x3ff00000\n\
         vmov d0, r0, r1\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x40000000\n\
         vmov d1, r0, r1\n\
         vcmp.f64 d0, d1\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x400c0000\n\
         vmov d2, r0, r1\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x400c0000\n\
         vmov d3, r0, r1\n\
         vcmp.f64 d2, d3\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x40100000\n\
         vmov d4, r0, r1\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0xbff00000\n\
         vmov d5, r0, r1\n\
         vcmp.f64 d4, d5\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x7ff80000\n\
         vmov d6, r0, r1\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x3ff00000\n\
         vmov d7, r0, r1\n\
         vcmp.f64 d6, d7\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r12, r12, r4, lsr #28\n\
         ldr r0, =0x00000000\n\
         ldr r1, =0x80000000\n\
         vmov d8, r0, r1\n\
         vcmp.f64 d8, #0.0\n\
         vmrs APSR_nzcv, fpscr\n\
         mrs r4, cpsr\n\
         eor r0, r12, r4, lsr #28",
    );
    let Some(qemu_exit) = run_arm_linux_exit(&asm) else {
        return;
    };

    let mut cpu = Cpu::new();
    let mut mem = VecMemory::new(0, 4);
    let mut folded = 0;

    let fold_flags = |cpu: &mut Cpu, folded: &mut u32, mem: &mut VecMemory| {
        cpu.execute_arm(0xeef1_fa10, 0, mem).unwrap(); // vmrs APSR_nzcv, fpscr
        *folded ^= cpu.cpsr.to_u32() >> 28;
    };

    cpu.set_sreg(0, 1.0f32.to_bits());
    cpu.set_sreg(1, 2.0f32.to_bits());
    cpu.execute_arm(arm_vcmp_single_instr(0, 1), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_sreg(2, 3.0f32.to_bits());
    cpu.set_sreg(3, 3.0f32.to_bits());
    cpu.execute_arm(arm_vcmp_single_instr(2, 3), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_sreg(4, 4.0f32.to_bits());
    cpu.set_sreg(5, (-1.0f32).to_bits());
    cpu.execute_arm(arm_vcmp_single_instr(4, 5), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_sreg(6, f32::NAN.to_bits());
    cpu.set_sreg(7, 1.0f32.to_bits());
    cpu.execute_arm(arm_vcmp_single_instr(6, 7), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_sreg(8, (-0.0f32).to_bits());
    cpu.execute_arm(arm_vcmp_single_zero_instr(8), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_dreg(0, 1.0f64.to_bits());
    cpu.set_dreg(1, 2.0f64.to_bits());
    cpu.execute_arm(arm_vcmp_double_instr(0, 1), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_dreg(2, 3.5f64.to_bits());
    cpu.set_dreg(3, 3.5f64.to_bits());
    cpu.execute_arm(arm_vcmp_double_instr(2, 3), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_dreg(4, 4.0f64.to_bits());
    cpu.set_dreg(5, (-1.0f64).to_bits());
    cpu.execute_arm(arm_vcmp_double_instr(4, 5), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_dreg(6, f64::NAN.to_bits());
    cpu.set_dreg(7, 1.0f64.to_bits());
    cpu.execute_arm(arm_vcmp_double_instr(6, 7), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_dreg(8, (-0.0f64).to_bits());
    cpu.execute_arm(arm_vcmp_double_zero_instr(8), 0, &mut mem)
        .unwrap();
    fold_flags(&mut cpu, &mut folded, &mut mem);

    cpu.set_reg(0, folded);

    assert_eq!(qemu_exit as u32, cpu.reg(0) & 0xff);
}
