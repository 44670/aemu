use std::fmt;

pub type Result<T> = std::result::Result<T, Trap>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    Memory(String),
    UndefinedArm {
        pc: u32,
        instr: u32,
    },
    UndefinedThumb {
        pc: u32,
        instr: u16,
    },
    Unpredictable(&'static str),
    Privileged {
        pc: u32,
        instr: u32,
        operation: &'static str,
    },
    SoftwareInterrupt {
        pc: u32,
        comment: u32,
    },
    Breakpoint {
        pc: u32,
        comment: u32,
    },
    StepLimit,
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(err) => write!(f, "memory error: {err}"),
            Self::UndefinedArm { pc, instr } => {
                write!(f, "undefined ARM instruction {instr:#010x} at {pc:#010x}")
            }
            Self::UndefinedThumb { pc, instr } => {
                write!(f, "undefined Thumb instruction {instr:#06x} at {pc:#010x}")
            }
            Self::Unpredictable(what) => write!(f, "unpredictable instruction behavior: {what}"),
            Self::Privileged {
                pc,
                instr,
                operation,
            } => write!(
                f,
                "privileged {operation} instruction {instr:#010x} at {pc:#010x}"
            ),
            Self::SoftwareInterrupt { pc, comment } => {
                write!(f, "software interrupt {comment:#x} at {pc:#010x}")
            }
            Self::Breakpoint { pc, comment } => write!(f, "breakpoint {comment:#x} at {pc:#010x}"),
            Self::StepLimit => write!(f, "step limit reached"),
        }
    }
}

impl std::error::Error for Trap {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isa {
    Arm,
    Thumb,
}

const VFP_FPSID_ARM1136: u32 = 0x4101_20b4;
const FPSCR_IOC: u32 = 1 << 0;
const FPSCR_DZC: u32 = 1 << 1;
const FPSCR_OFC: u32 = 1 << 2;
const FPSCR_UFC: u32 = 1 << 3;
const FPSCR_IXC: u32 = 1 << 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cpsr {
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    pub ge: u8,
    pub t: bool,
}

impl Cpsr {
    pub fn to_u32(self) -> u32 {
        (u32::from(self.n) << 31)
            | (u32::from(self.z) << 30)
            | (u32::from(self.c) << 29)
            | (u32::from(self.v) << 28)
            | (u32::from(self.q) << 27)
            | (u32::from(self.ge & 0xf) << 16)
            | (u32::from(self.t) << 5)
    }

    pub fn from_u32(value: u32) -> Self {
        Self {
            n: value & (1 << 31) != 0,
            z: value & (1 << 30) != 0,
            c: value & (1 << 29) != 0,
            v: value & (1 << 28) != 0,
            q: value & (1 << 27) != 0,
            ge: ((value >> 16) & 0xf) as u8,
            t: value & (1 << 5) != 0,
        }
    }

    fn set_nz(&mut self, value: u32) {
        self.n = value & 0x8000_0000 != 0;
        self.z = value == 0;
    }

    fn set_nzc(&mut self, value: u32, carry: bool) {
        self.set_nz(value);
        self.c = carry;
    }

    fn set_nzcv(&mut self, value: u32, carry: bool, overflow: bool) {
        self.set_nzc(value, carry);
        self.v = overflow;
    }
}

#[derive(Debug, Clone, Copy)]
struct VfpVectorPlan {
    count: usize,
    delta_d: usize,
    delta_m: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExclusiveReservation {
    addr: u32,
    size: u8,
}

#[derive(Debug, Clone)]
pub struct Cpu {
    regs: [u32; 16],
    sregs: [u32; 32],
    pub fpscr: u32,
    pub cp15_tpidrurw: u32,
    pub cp15_tpidruro: u32,
    pub cpsr: Cpsr,
    thumb_bl_prefix: Option<u32>,
    exclusive_reservation: Option<ExclusiveReservation>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: [0; 16],
            sregs: [0; 32],
            fpscr: 0,
            cp15_tpidrurw: 0,
            cp15_tpidruro: 0,
            cpsr: Cpsr::default(),
            thumb_bl_prefix: None,
            exclusive_reservation: None,
        }
    }

    pub fn isa(&self) -> Isa {
        if self.cpsr.t { Isa::Thumb } else { Isa::Arm }
    }

    pub fn set_isa(&mut self, isa: Isa) {
        self.cpsr.t = isa == Isa::Thumb;
    }

    pub fn reg(&self, idx: usize) -> u32 {
        self.regs[idx]
    }

    pub fn set_reg(&mut self, idx: usize, value: u32) {
        self.regs[idx] = value;
    }

    pub fn sreg(&self, idx: usize) -> u32 {
        self.sregs[idx]
    }

    pub fn set_sreg(&mut self, idx: usize, value: u32) {
        self.sregs[idx] = value;
    }

    pub fn dreg(&self, idx: usize) -> u64 {
        self.dreg_bits(idx)
    }

    pub fn set_dreg(&mut self, idx: usize, value: u64) {
        self.set_dreg_bits(idx, value);
    }

    pub fn pc(&self) -> u32 {
        self.regs[15]
    }

    pub fn set_pc(&mut self, value: u32) {
        self.regs[15] = value;
    }

    pub fn branch_exchange(&mut self, target: u32) {
        self.cpsr.t = target & 1 != 0;
        self.regs[15] = target & !1;
    }

    pub fn step<M: Memory>(&mut self, mem: &mut M) -> Result<()> {
        let pc = self.regs[15];
        if self.cpsr.t {
            let instr = mem.load16(pc)?;
            self.regs[15] = pc.wrapping_add(2);
            self.execute_thumb(instr, pc, mem)
        } else {
            let instr = mem.load32(pc)?;
            self.regs[15] = pc.wrapping_add(4);
            self.execute_arm(instr, pc, mem)
        }
    }

    pub fn run<M: Memory>(&mut self, mem: &mut M, max_steps: usize) -> Result<()> {
        for _ in 0..max_steps {
            self.step(mem)?;
        }
        Err(Trap::StepLimit)
    }

    pub fn execute_arm<M: Memory>(&mut self, instr: u32, pc: u32, mem: &mut M) -> Result<()> {
        if (instr & 0xfe00_0000) == 0xfa00_0000 {
            return self.exec_arm_blx_immediate(instr, pc);
        }
        if instr == 0xf57f_f01f {
            self.exclusive_reservation = None;
            return Ok(());
        }

        let cond = (instr >> 28) & 0xf;
        if cond == 0xf {
            if self.exec_arm_unconditional(instr, pc)? {
                return Ok(());
            }
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if !condition_passed(cond, self.cpsr) {
            return Ok(());
        }

        if (instr & 0x0fff_fff0) == 0x012f_ff10 {
            let rm = (instr & 0xf) as usize;
            self.branch_exchange(self.arm_read_reg(rm, pc));
            return Ok(());
        }
        if (instr & 0x0fff_fff0) == 0x012f_ff20 {
            let rm = (instr & 0xf) as usize;
            self.branch_exchange(self.arm_read_reg(rm, pc));
            return Ok(());
        }
        if (instr & 0x0fff_fff0) == 0x012f_ff30 {
            let rm = (instr & 0xf) as usize;
            self.regs[14] = pc.wrapping_add(4);
            self.branch_exchange(self.arm_read_reg(rm, pc));
            return Ok(());
        }
        if (instr & 0x0ff0_00f0) == 0x0120_0070 {
            return Err(Trap::Breakpoint {
                pc,
                comment: ((instr >> 4) & 0xfff0) | (instr & 0xf),
            });
        }
        if (instr & 0xfff0_00f0) == 0xe7f0_00f0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if (instr & 0x0f00_0000) == 0x0f00_0000 {
            return Err(Trap::SoftwareInterrupt {
                pc,
                comment: instr & 0x00ff_ffff,
            });
        }
        if (instr & 0x0fff_f000) == 0x0320_f000 {
            return Ok(());
        }

        if self.exec_arm_vfp(instr, pc, mem)? {
            return Ok(());
        }
        if self.exec_arm_coprocessor(instr, pc)? {
            return Ok(());
        }
        if self.exec_arm_misc(instr, pc, mem)? {
            return Ok(());
        }
        if self.exec_arm_multiply(instr, pc)? {
            return Ok(());
        }
        if self.exec_arm_halfword_transfer(instr, pc, mem)? {
            return Ok(());
        }

        match (instr >> 25) & 0b111 {
            0b101 => self.exec_arm_branch(instr, pc),
            0b100 => self.exec_arm_block_transfer(instr, pc, mem),
            0b010 | 0b011 => self.exec_arm_single_transfer(instr, pc, mem),
            _ if (instr >> 26) & 0b11 == 0 => self.exec_arm_data_processing(instr, pc, mem),
            _ => Err(Trap::UndefinedArm { pc, instr }),
        }
    }

    fn exec_arm_branch(&mut self, instr: u32, pc: u32) -> Result<()> {
        let link = instr & (1 << 24) != 0;
        let imm = sign_extend((instr & 0x00ff_ffff) << 2, 26);
        if link {
            self.regs[14] = pc.wrapping_add(4);
        }
        self.regs[15] = pc.wrapping_add(8).wrapping_add(imm as u32);
        Ok(())
    }

    fn exec_arm_unconditional(&mut self, instr: u32, pc: u32) -> Result<bool> {
        if (instr & 0x0fff_f000) == 0x0320_f000 {
            return Ok(true);
        }

        if (instr & 0xff70_f000) == 0xf550_f000 || (instr & 0xff70_f010) == 0xf750_f000 {
            return Ok(true);
        }

        if (instr & 0xffff_fdff) == 0xf101_0000 {
            if instr & (1 << 9) != 0 {
                return Err(Trap::Unpredictable("SETEND big-endian mode unsupported"));
            }
            return Ok(true);
        }

        if (instr & 0xfff1_fe20) == 0xf100_0000 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "CPS",
            });
        }

        if (instr & 0xfe50_ffff) == 0xf810_0a00 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "RFE",
            });
        }

        if (instr & 0xfe5f_ffe0) == 0xf84d_0500 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "SRS",
            });
        }

        Ok(false)
    }

    fn exec_arm_blx_immediate(&mut self, instr: u32, pc: u32) -> Result<()> {
        let h = (instr >> 24) & 1;
        let imm = sign_extend(((instr & 0x00ff_ffff) << 2) | (h << 1), 26);
        self.regs[14] = pc.wrapping_add(4);
        self.cpsr.t = true;
        self.regs[15] = pc.wrapping_add(8).wrapping_add(imm as u32);
        Ok(())
    }

    fn exec_arm_vfp<M: Memory>(&mut self, instr: u32, pc: u32, mem: &mut M) -> Result<bool> {
        let vfp_multi = if (instr & 0x0e00_0a00) == 0x0c00_0a00 {
            let p = instr & (1 << 24) != 0;
            let u = instr & (1 << 23) != 0;
            let w = instr & (1 << 21) != 0;
            (!p && u) || (p && !u && w)
        } else {
            false
        };
        if vfp_multi {
            let double = instr & (1 << 8) != 0;
            let p = instr & (1 << 24) != 0;
            let u = instr & (1 << 23) != 0;
            let w = instr & (1 << 21) != 0;
            let load = instr & (1 << 20) != 0;
            let rn = ((instr >> 16) & 0xf) as usize;
            let words = instr & 0xff;
            let bytes = words * 4;
            if rn == 15 && w {
                return Err(Trap::Unpredictable(
                    "VFP multiple transfer writeback with PC base",
                ));
            }
            if words == 0 {
                return Err(Trap::Unpredictable(
                    "VFP multiple transfer empty register list",
                ));
            }
            let base = self.arm_read_reg(rn, pc);
            let start = match (u, p) {
                (true, false) => base,
                (true, true) => base.wrapping_add(4),
                (false, true) => base.wrapping_sub(bytes),
                (false, false) => base.wrapping_sub(bytes).wrapping_add(4),
            };
            let mut addr = start;
            if double {
                let first = vfp_double_d(instr);
                let regs = (words / 2) as usize;
                if regs == 0 || regs > 16 || first + regs > 16 {
                    return Err(Trap::Unpredictable("VFP multiple transfer register range"));
                }
                for idx in 0..regs {
                    if load {
                        let lo = u64::from(mem.load32(addr)?);
                        let hi = u64::from(mem.load32(addr.wrapping_add(4))?);
                        self.set_dreg_bits(first + idx, lo | (hi << 32));
                    } else {
                        let value = self.dreg_bits(first + idx);
                        self.store32(mem, addr, value as u32)?;
                        self.store32(mem, addr.wrapping_add(4), (value >> 32) as u32)?;
                    }
                    addr = addr.wrapping_add(8);
                }
            } else {
                let first = vfp_single_d(instr);
                let regs = words as usize;
                if first + regs > 32 {
                    return Err(Trap::Unpredictable("VFP multiple transfer register range"));
                }
                for idx in 0..regs {
                    if load {
                        self.sregs[first + idx] = mem.load32(addr)?;
                    } else {
                        self.store32(mem, addr, self.sregs[first + idx])?;
                    }
                    addr = addr.wrapping_add(4);
                }
            }
            if w {
                self.write_reg_arm(
                    rn,
                    if u {
                        base.wrapping_add(bytes)
                    } else {
                        base.wrapping_sub(bytes)
                    },
                );
            }
            return Ok(true);
        }

        if (instr & 0x0f00_0f00) == 0x0d00_0a00 || (instr & 0x0f00_0f00) == 0x0d00_0b00 {
            let double = (instr & 0x0f00_0f00) == 0x0d00_0b00;
            let rn = ((instr >> 16) & 0xf) as usize;
            let imm = (instr & 0xff) * 4;
            let base = self.arm_read_reg(rn, pc);
            let addr = if instr & (1 << 23) != 0 {
                base.wrapping_add(imm)
            } else {
                base.wrapping_sub(imm)
            };
            if instr & (1 << 20) != 0 {
                if double {
                    let dd = vfp_double_d(instr);
                    if dd >= 16 {
                        return Err(Trap::Unpredictable("VFP double register out of range"));
                    }
                    let lo = u64::from(mem.load32(addr)?);
                    let hi = u64::from(mem.load32(addr.wrapping_add(4))?);
                    self.set_dreg_bits(dd, lo | (hi << 32));
                } else {
                    let sd = vfp_single_d(instr);
                    self.sregs[sd] = mem.load32(addr)?;
                }
            } else if double {
                let dd = vfp_double_d(instr);
                if dd >= 16 {
                    return Err(Trap::Unpredictable("VFP double register out of range"));
                }
                let value = self.dreg_bits(dd);
                self.store32(mem, addr, value as u32)?;
                self.store32(mem, addr.wrapping_add(4), (value >> 32) as u32)?;
            } else {
                let sd = vfp_single_d(instr);
                self.store32(mem, addr, self.sregs[sd])?;
            }
            return Ok(true);
        }

        match instr & 0x0ff0_0fd0 {
            0x0c40_0a10 | 0x0c40_0b10 => {
                let rt2 = ((instr >> 16) & 0xf) as usize;
                let rt = ((instr >> 12) & 0xf) as usize;
                if rt == 15 || rt2 == 15 {
                    return Err(Trap::Unpredictable("VFP VMOV from PC"));
                }
                if (instr & 0x0f00) == 0x0b00 {
                    let dm = vfp_double_m(instr);
                    self.set_checked_dreg_bits(
                        dm,
                        u64::from(self.regs[rt]) | (u64::from(self.regs[rt2]) << 32),
                    )?;
                } else {
                    let sm = vfp_single_m(instr);
                    if sm == 31 {
                        return Err(Trap::Unpredictable("VFP VMOV past S31"));
                    }
                    self.sregs[sm] = self.regs[rt];
                    self.sregs[sm + 1] = self.regs[rt2];
                }
                return Ok(true);
            }
            0x0c50_0a10 | 0x0c50_0b10 => {
                let rt2 = ((instr >> 16) & 0xf) as usize;
                let rt = ((instr >> 12) & 0xf) as usize;
                if rt == 15 || rt2 == 15 || rt == rt2 {
                    return Err(Trap::Unpredictable("VFP VMOV to invalid core registers"));
                }
                if (instr & 0x0f00) == 0x0b00 {
                    let value = self.checked_dreg_bits(vfp_double_m(instr))?;
                    self.write_reg_arm(rt, value as u32);
                    self.write_reg_arm(rt2, (value >> 32) as u32);
                } else {
                    let sm = vfp_single_m(instr);
                    if sm == 31 {
                        return Err(Trap::Unpredictable("VFP VMOV past S31"));
                    }
                    self.write_reg_arm(rt, self.sregs[sm]);
                    self.write_reg_arm(rt2, self.sregs[sm + 1]);
                }
                return Ok(true);
            }
            _ => {}
        }

        if (instr & 0x0fb0_0f7f) == 0x0e00_0a10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV from PC"));
            }
            let sn = vfp_single_n(instr);
            self.sregs[sn] = self.regs[rt];
            return Ok(true);
        }

        if (instr & 0x0fb0_0f7f) == 0x0e10_0a10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV to PC"));
            }
            let sn = vfp_single_n(instr);
            self.write_reg_arm(rt, self.sregs[sn]);
            return Ok(true);
        }

        if (instr & 0x0f90_0f7f) == 0x0e00_0b10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV from PC"));
            }
            let dn = vfp_double_n(instr);
            let shift = ((instr >> 21) & 1) * 32;
            let value = self.checked_dreg_bits(dn)?;
            let mask = 0xffff_ffff_u64 << shift;
            self.set_checked_dreg_bits(dn, (value & !mask) | (u64::from(self.regs[rt]) << shift))?;
            return Ok(true);
        }

        if (instr & 0x0f90_0f7f) == 0x0e10_0b10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV to PC"));
            }
            let dn = vfp_double_n(instr);
            let shift = ((instr >> 21) & 1) * 32;
            self.write_reg_arm(rt, (self.checked_dreg_bits(dn)? >> shift) as u32);
            return Ok(true);
        }

        if (instr & 0x0fe0_0fff) == 0x0ee0_0a10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            let load = instr & (1 << 20) != 0;
            let sysreg = (instr >> 16) & 0xf;
            if load {
                let value = match sysreg {
                    0 => VFP_FPSID_ARM1136,
                    1 => self.fpscr,
                    8 => {
                        return Err(Trap::Privileged {
                            pc,
                            instr,
                            operation: "VMRS FPEXC",
                        });
                    }
                    9 | 10 => {
                        return Err(Trap::Privileged {
                            pc,
                            instr,
                            operation: "VMRS FPINST",
                        });
                    }
                    _ => return Err(Trap::UndefinedArm { pc, instr }),
                };
                if rt == 15 {
                    if sysreg != 1 {
                        return Err(Trap::Unpredictable("VFP VMRS non-FPSCR to PC"));
                    }
                    self.cpsr.n = value & (1 << 31) != 0;
                    self.cpsr.z = value & (1 << 30) != 0;
                    self.cpsr.c = value & (1 << 29) != 0;
                    self.cpsr.v = value & (1 << 28) != 0;
                } else {
                    self.write_reg_arm(rt, value);
                }
            } else {
                if rt == 15 {
                    return Err(Trap::Unpredictable("VFP VMSR from PC"));
                }
                match sysreg {
                    0 => {}
                    1 => self.fpscr = self.regs[rt],
                    8 => {
                        return Err(Trap::Privileged {
                            pc,
                            instr,
                            operation: "VMSR FPEXC",
                        });
                    }
                    9 | 10 => {
                        return Err(Trap::Privileged {
                            pc,
                            instr,
                            operation: "VMSR FPINST",
                        });
                    }
                    _ => return Err(Trap::UndefinedArm { pc, instr }),
                }
            }
            return Ok(true);
        }

        let compare = instr & 0x0fbf_0f40;
        if compare == 0x0eb4_0a40 {
            let sn = vfp_single_d(instr);
            let sm = vfp_single_m(instr);
            self.set_fpscr_compare_f32_bits(self.sregs[sn], self.sregs[sm], instr & 0x80 != 0);
            return Ok(true);
        }
        if (instr & 0x0fbf_0f7f) == 0x0eb5_0a40 {
            let sn = vfp_single_d(instr);
            self.set_fpscr_compare_f32_bits(self.sregs[sn], 0, instr & 0x80 != 0);
            return Ok(true);
        }
        if compare == 0x0eb4_0b40 {
            let dn = vfp_double_d(instr);
            let dm = vfp_double_m(instr);
            self.set_fpscr_compare_f64_bits(
                self.checked_dreg_bits(dn)?,
                self.checked_dreg_bits(dm)?,
                instr & 0x80 != 0,
            );
            return Ok(true);
        }
        if (instr & 0x0fbf_0f7f) == 0x0eb5_0b40 {
            let dn = vfp_double_d(instr);
            self.set_fpscr_compare_f64_bits(self.checked_dreg_bits(dn)?, 0, instr & 0x80 != 0);
            return Ok(true);
        }

        let convert = instr & 0x0fbf_0fc0;
        match convert {
            0x0eb7_0ac0 => {
                let dd = vfp_double_d(instr);
                let sm = vfp_single_m(instr);
                self.set_checked_dreg_bits(
                    dd,
                    f64::from(f32::from_bits(self.sregs[sm])).to_bits(),
                )?;
                return Ok(true);
            }
            0x0eb7_0bc0 => {
                let sd = vfp_single_d(instr);
                let dm = vfp_double_m(instr);
                let (result, exception_flags) = vfp_f64_to_f32_bits(self.checked_dreg_bits(dm)?);
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebd_0ac0 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let (result, exception_flags) =
                    vfp_trunc_to_i32_bits(f64::from(f32::from_bits(self.sregs[sm])));
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebd_0a40 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let (result, exception_flags) =
                    vfp_round_to_i32_bits(f64::from(f32::from_bits(self.sregs[sm])), self.fpscr);
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebc_0ac0 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let (result, exception_flags) =
                    vfp_trunc_to_u32_bits(f64::from(f32::from_bits(self.sregs[sm])));
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebc_0a40 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let (result, exception_flags) =
                    vfp_round_to_u32_bits(f64::from(f32::from_bits(self.sregs[sm])), self.fpscr);
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebd_0bc0 => {
                let sd = vfp_single_d(instr);
                let dm = vfp_double_m(instr);
                let (result, exception_flags) =
                    vfp_trunc_to_i32_bits(f64::from_bits(self.checked_dreg_bits(dm)?));
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebd_0b40 => {
                let sd = vfp_single_d(instr);
                let dm = vfp_double_m(instr);
                let (result, exception_flags) =
                    vfp_round_to_i32_bits(f64::from_bits(self.checked_dreg_bits(dm)?), self.fpscr);
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebc_0bc0 => {
                let sd = vfp_single_d(instr);
                let dm = vfp_double_m(instr);
                let (result, exception_flags) =
                    vfp_trunc_to_u32_bits(f64::from_bits(self.checked_dreg_bits(dm)?));
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0ebc_0b40 => {
                let sd = vfp_single_d(instr);
                let dm = vfp_double_m(instr);
                let (result, exception_flags) =
                    vfp_round_to_u32_bits(f64::from_bits(self.checked_dreg_bits(dm)?), self.fpscr);
                self.sregs[sd] = result;
                self.fpscr |= exception_flags;
                return Ok(true);
            }
            0x0eb8_0ac0 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let value = self.sregs[sm] as i32;
                let result = value as f32;
                self.sregs[sd] = result.to_bits();
                self.fpscr |= vfp_int_to_f32_inexact_flag(f64::from(value), result);
                return Ok(true);
            }
            0x0eb8_0a40 => {
                let sd = vfp_single_d(instr);
                let sm = vfp_single_m(instr);
                let value = self.sregs[sm];
                let result = value as f32;
                self.sregs[sd] = result.to_bits();
                self.fpscr |= vfp_int_to_f32_inexact_flag(f64::from(value), result);
                return Ok(true);
            }
            0x0eb8_0bc0 => {
                let dd = vfp_double_d(instr);
                let sm = vfp_single_m(instr);
                self.set_checked_dreg_bits(dd, ((self.sregs[sm] as i32) as f64).to_bits())?;
                return Ok(true);
            }
            0x0eb8_0b40 => {
                let dd = vfp_double_d(instr);
                let sm = vfp_single_m(instr);
                self.set_checked_dreg_bits(dd, (self.sregs[sm] as f64).to_bits())?;
                return Ok(true);
            }
            _ => {}
        }

        let unary = instr & 0x0fbf_0fc0;
        if matches!(unary, 0x0eb0_0a40 | 0x0eb1_0a40 | 0x0eb0_0ac0 | 0x0eb1_0ac0) {
            let sd = vfp_single_d(instr);
            let sm = vfp_single_m(instr);
            self.exec_vfp_2op_f32(sd, sm, |value| match unary {
                0x0eb0_0a40 => (value, 0),
                0x0eb1_0a40 => (value ^ 0x8000_0000, 0),
                0x0eb0_0ac0 => (value & 0x7fff_ffff, 0),
                0x0eb1_0ac0 => vfp_sqrt_f32(value),
                _ => unreachable!(),
            })?;
            return Ok(true);
        }
        if matches!(unary, 0x0eb0_0b40 | 0x0eb1_0b40 | 0x0eb0_0bc0 | 0x0eb1_0bc0) {
            let dd = vfp_double_d(instr);
            let dm = vfp_double_m(instr);
            self.exec_vfp_2op_f64(dd, dm, |value| match unary {
                0x0eb0_0b40 => (value, 0),
                0x0eb1_0b40 => (value ^ 0x8000_0000_0000_0000, 0),
                0x0eb0_0bc0 => (value & 0x7fff_ffff_ffff_ffff, 0),
                0x0eb1_0bc0 => vfp_sqrt_f64(value),
                _ => unreachable!(),
            })?;
            return Ok(true);
        }

        let op = instr & 0x0fb0_0f40;
        let op_kind = match op & !0x100 {
            0x0e00_0a00 => Some(VfpBinaryOp::MulAdd),
            0x0e00_0a40 => Some(VfpBinaryOp::MulSub),
            0x0e10_0a00 => Some(VfpBinaryOp::NegMulSub),
            0x0e10_0a40 => Some(VfpBinaryOp::NegMulAdd),
            0x0e20_0a00 => Some(VfpBinaryOp::Mul),
            0x0e20_0a40 => Some(VfpBinaryOp::NegMul),
            0x0e30_0a00 => Some(VfpBinaryOp::Add),
            0x0e30_0a40 => Some(VfpBinaryOp::Sub),
            0x0e80_0a00 => Some(VfpBinaryOp::Div),
            _ => None,
        };
        if let Some(op_kind) = op_kind {
            if op & 0x100 == 0 {
                let sd = vfp_single_d(instr);
                let sn = vfp_single_n(instr);
                let sm = vfp_single_m(instr);
                self.exec_vfp_3op_f32(sd, sn, sm, |dst, lhs, rhs| {
                    op_kind.eval_f32_with_flags(dst, lhs, rhs)
                })?;
            } else {
                let dd = vfp_double_d(instr);
                let dn = vfp_double_n(instr);
                let dm = vfp_double_m(instr);
                self.exec_vfp_3op_f64(dd, dn, dm, |dst, lhs, rhs| {
                    op_kind.eval_f64_with_flags(dst, lhs, rhs)
                })?;
            }
            return Ok(true);
        }

        Ok(false)
    }

    fn exec_arm_coprocessor(&mut self, instr: u32, pc: u32) -> Result<bool> {
        if (instr & 0x0e00_0010) != 0x0e00_0010 {
            return Ok(false);
        }

        let coproc = (instr >> 8) & 0xf;
        let load = instr & (1 << 20) != 0;
        let opc1 = (instr >> 21) & 0x7;
        let crn = (instr >> 16) & 0xf;
        let rt = ((instr >> 12) & 0xf) as usize;
        let crm = instr & 0xf;
        let opc2 = (instr >> 5) & 0x7;

        if coproc == 15
            && !load
            && opc1 == 0
            && crn == 7
            && matches!((crm, opc2), (10, 4) | (10, 5) | (5, 4))
        {
            if rt == 15 {
                return Err(Trap::Unpredictable("CP15 barrier MCR from PC"));
            }
            return Ok(true);
        }

        if coproc == 15 && opc1 == 0 && crn == 13 && crm == 0 && matches!(opc2, 2 | 3) {
            if load {
                if rt == 15 {
                    return Err(Trap::Unpredictable("CP15 TLS MRC to PC"));
                }
                let value = if opc2 == 2 {
                    self.cp15_tpidrurw
                } else {
                    self.cp15_tpidruro
                };
                self.write_reg_arm(rt, value);
            } else {
                if rt == 15 {
                    return Err(Trap::Unpredictable("CP15 TLS MCR from PC"));
                }
                if opc2 == 3 {
                    return Err(Trap::Privileged {
                        pc,
                        instr,
                        operation: "MCR TPIDRURO",
                    });
                }
                let value = self.arm_read_reg(rt, pc);
                self.cp15_tpidrurw = value;
            }
            return Ok(true);
        }

        Err(Trap::UndefinedArm { pc, instr })
    }

    fn write_cpsr_fields(
        &mut self,
        value: u32,
        field_mask: u32,
        pc: u32,
        instr: u32,
    ) -> Result<()> {
        if field_mask & 0b0011 != 0 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "MSR CPSR control",
            });
        }
        if field_mask & 0b1000 != 0 {
            self.cpsr.n = value & (1 << 31) != 0;
            self.cpsr.z = value & (1 << 30) != 0;
            self.cpsr.c = value & (1 << 29) != 0;
            self.cpsr.v = value & (1 << 28) != 0;
            self.cpsr.q = value & (1 << 27) != 0;
        }
        if field_mask & 0b0100 != 0 {
            self.cpsr.ge = ((value >> 16) & 0xf) as u8;
        }
        Ok(())
    }

    fn exec_arm_misc<M: Memory>(&mut self, instr: u32, pc: u32, mem: &mut M) -> Result<bool> {
        if is_unsupported_a32_newer_than_armv6(instr) {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        if (instr & 0x0fff_0ff0) == 0x016f_0f10 {
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("misc instruction with PC register"));
            }
            let value = self.arm_read_reg(rm, pc).leading_zeros();
            self.write_reg_arm(rd, value);
            return Ok(true);
        }

        if (instr & 0x0fff_0fff) == 0x014f_0000 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "MRS SPSR",
            });
        }

        if (instr & 0x0fff_0fff) == 0x010f_0000 {
            let rd = ((instr >> 12) & 0xf) as usize;
            if rd == 15 {
                return Err(Trap::Unpredictable(
                    "status register access with PC register",
                ));
            }
            self.write_reg_arm(rd, self.cpsr.to_u32());
            return Ok(true);
        }

        if (instr & 0x0ff0_fff0) == 0x0160_f000 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "MSR SPSR",
            });
        }

        if (instr & 0x0ff0_f000) == 0x0360_f000 {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "MSR SPSR",
            });
        }

        if (instr & 0x0ff0_fff0) == 0x0120_f000 {
            let field_mask = (instr >> 16) & 0xf;
            let rm = (instr & 0xf) as usize;
            if rm == 15 {
                return Err(Trap::Unpredictable(
                    "status register access with PC register",
                ));
            }
            if field_mask == 0 {
                return Err(Trap::Unpredictable(
                    "status register access with empty mask",
                ));
            }
            self.write_cpsr_fields(self.arm_read_reg(rm, pc), field_mask, pc, instr)?;
            return Ok(true);
        }

        if (instr & 0x0ff0_f000) == 0x0320_f000 {
            let field_mask = (instr >> 16) & 0xf;
            let (value, _) = arm_expand_imm(instr & 0xfff);
            self.write_cpsr_fields(value, field_mask, pc, instr)?;
            return Ok(true);
        }

        if (instr & 0x0fff_0ff0) == 0x06bf_0f30 {
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("misc instruction with PC register"));
            }
            self.write_reg_arm(rd, self.arm_read_reg(rm, pc).swap_bytes());
            return Ok(true);
        }

        if (instr & 0x0fff_0ff0) == 0x06bf_0fb0 {
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("misc instruction with PC register"));
            }
            let value = self.arm_read_reg(rm, pc);
            self.write_reg_arm(
                rd,
                ((value & 0x00ff_00ff) << 8) | ((value & 0xff00_ff00) >> 8),
            );
            return Ok(true);
        }

        if (instr & 0x0fff_0ff0) == 0x06ff_0fb0 {
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("misc instruction with PC register"));
            }
            let value = self.arm_read_reg(rm, pc);
            let half = ((value & 0xff) << 8) | ((value >> 8) & 0xff);
            self.write_reg_arm(rd, sign_extend(half, 16) as u32);
            return Ok(true);
        }

        let extend_add_key = instr & 0x0ff0_0070;
        if matches!(
            extend_add_key,
            0x0680_0070 | 0x06a0_0070 | 0x06b0_0070 | 0x06c0_0070 | 0x06e0_0070 | 0x06f0_0070
        ) {
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("extend with PC register"));
            }
            let rotated = self
                .arm_read_reg(rm, pc)
                .rotate_right(((instr >> 10) & 0x3) * 8);
            let base = if rn == 15 {
                0
            } else {
                self.arm_read_reg(rn, pc)
            };
            let value = match extend_add_key {
                0x0680_0070 => extend_add_16(base, rotated, true),
                0x06a0_0070 => base.wrapping_add(sign_extend(rotated & 0xff, 8) as u32),
                0x06b0_0070 => base.wrapping_add(sign_extend(rotated & 0xffff, 16) as u32),
                0x06c0_0070 => extend_add_16(base, rotated, false),
                0x06e0_0070 => base.wrapping_add(rotated & 0xff),
                0x06f0_0070 => base.wrapping_add(rotated & 0xffff),
                _ => unreachable!(),
            };
            self.write_reg_arm(rd, value);
            return Ok(true);
        }

        if (instr & 0x0ff0_00f0) == 0x0780_0010 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let rn = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rs == 15 || rm == 15 {
                return Err(Trap::Unpredictable("USAD8 with PC register"));
            }
            let mut sum = if rn == 15 {
                0
            } else {
                self.arm_read_reg(rn, pc)
            };
            let a = self.arm_read_reg(rm, pc);
            let b = self.arm_read_reg(rs, pc);
            for lane in 0..4 {
                let shift = lane * 8;
                let av = ((a >> shift) & 0xff) as i32;
                let bv = ((b >> shift) & 0xff) as i32;
                sum = sum.wrapping_add(av.abs_diff(bv));
            }
            self.write_reg_arm(rd, sum);
            return Ok(true);
        }

        if (instr & 0x0ff0_0090) == 0x0680_0010 {
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rn, rd, rm].contains(&15) {
                return Err(Trap::Unpredictable("packing with PC register"));
            }
            let shift = (instr >> 7) & 0x1f;
            let rn_value = self.arm_read_reg(rn, pc);
            let rm_value = self.arm_read_reg(rm, pc);
            let value = if instr & (1 << 6) == 0 {
                (rn_value & 0x0000_ffff) | (rm_value.wrapping_shl(shift) & 0xffff_0000)
            } else {
                let shifted = if shift == 0 {
                    if rm_value & 0x8000_0000 != 0 {
                        u32::MAX
                    } else {
                        0
                    }
                } else {
                    ((rm_value as i32) >> shift) as u32
                };
                (rn_value & 0xffff_0000) | (shifted & 0x0000_ffff)
            };
            self.write_reg_arm(rd, value);
            return Ok(true);
        }

        if (instr & 0x0ff0_0ff0) == 0x0680_0fb0 {
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rn, rd, rm].contains(&15) {
                return Err(Trap::Unpredictable("SEL with PC register"));
            }
            let rn_value = self.arm_read_reg(rn, pc);
            let rm_value = self.arm_read_reg(rm, pc);
            let mut out = 0u32;
            for lane in 0..4 {
                let shift = lane * 8;
                let from_rn = self.cpsr.ge & (1 << lane) != 0;
                let source = if from_rn { rn_value } else { rm_value };
                out |= source & (0xff << shift);
            }
            self.write_reg_arm(rd, out);
            return Ok(true);
        }

        if (instr & 0x0f80_0f10) == 0x0600_0f10 {
            let family = (instr >> 20) & 0x7;
            let op = (instr >> 5) & 0x7;
            if matches!(family, 1 | 2 | 3 | 5 | 6 | 7) && matches!(op, 0 | 1 | 2 | 3 | 4 | 7) {
                let rn = ((instr >> 16) & 0xf) as usize;
                let rd = ((instr >> 12) & 0xf) as usize;
                let rm = (instr & 0xf) as usize;
                if [rn, rd, rm].contains(&15) {
                    return Err(Trap::Unpredictable("parallel media with PC register"));
                }
                let value = self.exec_arm_parallel_op(
                    family,
                    op,
                    self.arm_read_reg(rn, pc),
                    self.arm_read_reg(rm, pc),
                );
                self.write_reg_arm(rd, value);
                return Ok(true);
            }
        }

        if (instr & 0x0f90_0ff0) == 0x0100_0050 {
            let op = (instr >> 21) & 0x3;
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rn, rd, rm].contains(&15) {
                return Err(Trap::Unpredictable(
                    "saturating arithmetic with PC register",
                ));
            }
            let rn_value = self.arm_read_reg(rn, pc) as i32;
            let rm_value = self.arm_read_reg(rm, pc) as i32;
            let value = match op {
                0 => saturating_add_i32(rm_value, rn_value, &mut self.cpsr),
                1 => saturating_sub_i32(rm_value, rn_value, &mut self.cpsr),
                2 => {
                    let doubled = saturating_add_i32(rn_value, rn_value, &mut self.cpsr);
                    saturating_add_i32(rm_value, doubled, &mut self.cpsr)
                }
                3 => {
                    let doubled = saturating_add_i32(rn_value, rn_value, &mut self.cpsr);
                    saturating_sub_i32(rm_value, doubled, &mut self.cpsr)
                }
                _ => unreachable!(),
            };
            self.write_reg_arm(rd, value as u32);
            return Ok(true);
        }

        let sat16_key = instr & 0x0ff0_0ff0;
        if sat16_key == 0x06a0_0f30 || sat16_key == 0x06e0_0f30 {
            let unsigned = sat16_key == 0x06e0_0f30;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rn = (instr & 0xf) as usize;
            if rd == 15 || rn == 15 {
                return Err(Trap::Unpredictable("saturation with PC register"));
            }
            let sat_bits = (instr >> 16) & 0xf;
            let sat_bits = if unsigned { sat_bits } else { sat_bits + 1 };
            let value = self.arm_read_reg(rn, pc);
            let lo = value as u16 as i16 as i32;
            let hi = (value >> 16) as u16 as i16 as i32;
            let lo = if unsigned {
                saturate_u32(lo, sat_bits, &mut self.cpsr)
            } else {
                saturate_i32(lo, sat_bits, &mut self.cpsr) as u32
            } & 0xffff;
            let hi = if unsigned {
                saturate_u32(hi, sat_bits, &mut self.cpsr)
            } else {
                saturate_i32(hi, sat_bits, &mut self.cpsr) as u32
            } & 0xffff;
            self.write_reg_arm(rd, lo | (hi << 16));
            return Ok(true);
        }

        if (instr & 0x0fe0_0030) == 0x06a0_0010 || (instr & 0x0fe0_0030) == 0x06e0_0010 {
            let unsigned = (instr & 0x0fe0_0030) == 0x06e0_0010;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rn = (instr & 0xf) as usize;
            if rd == 15 || rn == 15 {
                return Err(Trap::Unpredictable("saturation with PC register"));
            }
            let shift_amount = (instr >> 7) & 0x1f;
            let value = if instr & (1 << 6) != 0 {
                shift_operand(
                    self.arm_read_reg(rn, pc),
                    Shift::Asr,
                    shift_amount,
                    self.cpsr.c,
                    true,
                )
                .value
            } else {
                self.arm_read_reg(rn, pc).wrapping_shl(shift_amount)
            };
            let saturated = if unsigned {
                let sat_bits = (instr >> 16) & 0x1f;
                saturate_u32(value as i32, sat_bits, &mut self.cpsr)
            } else {
                let sat_bits = ((instr >> 16) & 0x1f) + 1;
                saturate_i32(value as i32, sat_bits, &mut self.cpsr) as u32
            };
            self.write_reg_arm(rd, saturated);
            return Ok(true);
        }

        let swp_key = instr & 0x0ff0_0ff0;
        if swp_key == 0x0100_0090 || swp_key == 0x0140_0090 {
            let byte = swp_key == 0x0140_0090;
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rn == 15 || rd == 15 || rm == 15 || rn == rd || rn == rm {
                return Err(Trap::Unpredictable("SWP with invalid register form"));
            }
            let addr = self.arm_read_reg(rn, pc);
            let old = if byte {
                u32::from(mem.load8(addr)?)
            } else {
                mem.load32(addr)?
            };
            if byte {
                self.store8(mem, addr, self.arm_read_reg(rm, pc) as u8)?;
            } else {
                self.store32(mem, addr, self.arm_read_reg(rm, pc))?;
            }
            self.write_reg_arm(rd, old);
            return Ok(true);
        }

        let exclusive_load_key = instr & 0x0ff0_0fff;
        if matches!(
            exclusive_load_key,
            0x0190_0f9f | 0x01b0_0f9f | 0x01d0_0f9f | 0x01f0_0f9f
        ) {
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            if rn == 15 || rd == 15 {
                return Err(Trap::Unpredictable("exclusive load with PC register"));
            }
            let addr = self.arm_read_reg(rn, pc);
            match exclusive_load_key {
                0x0190_0f9f => {
                    let value = mem.load32(addr)?;
                    self.write_reg_arm(rd, value);
                }
                0x01b0_0f9f => {
                    if rd >= 14 {
                        return Err(Trap::Unpredictable("LDREXD with invalid register pair"));
                    }
                    let lo = mem.load32(addr)?;
                    let hi = mem.load32(addr.wrapping_add(4))?;
                    self.write_reg_arm(rd, lo);
                    self.write_reg_arm(rd + 1, hi);
                }
                0x01d0_0f9f => {
                    let value = u32::from(mem.load8(addr)?);
                    self.write_reg_arm(rd, value);
                }
                0x01f0_0f9f => {
                    let value = u32::from(mem.load16(addr)?);
                    self.write_reg_arm(rd, value);
                }
                _ => unreachable!(),
            }
            self.exclusive_reservation = Some(ExclusiveReservation {
                addr,
                size: match exclusive_load_key {
                    0x0190_0f9f => 4,
                    0x01b0_0f9f => 8,
                    0x01d0_0f9f => 1,
                    0x01f0_0f9f => 2,
                    _ => unreachable!(),
                },
            });
            return Ok(true);
        }

        let exclusive_store_key = instr & 0x0ff0_0ff0;
        if matches!(
            exclusive_store_key,
            0x0180_0f90 | 0x01a0_0f90 | 0x01c0_0f90 | 0x01e0_0f90
        ) {
            let rn = ((instr >> 16) & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rn == 15 || rd == 15 || rm == 15 {
                return Err(Trap::Unpredictable("exclusive store with PC register"));
            }
            if rd == rn || rd == rm {
                return Err(Trap::Unpredictable(
                    "exclusive store status register overlaps operand",
                ));
            }
            if exclusive_store_key == 0x01a0_0f90 {
                if rm >= 14 || rm % 2 != 0 {
                    return Err(Trap::Unpredictable("STREXD with invalid register pair"));
                }
                if rd == rm + 1 {
                    return Err(Trap::Unpredictable(
                        "exclusive store status register overlaps operand",
                    ));
                }
            }
            let addr = self.arm_read_reg(rn, pc);
            if self
                .exclusive_reservation
                .map(|reservation| reservation.addr == addr)
                .unwrap_or(false)
            {
                match exclusive_store_key {
                    0x0180_0f90 => mem.store32(addr, self.arm_read_reg(rm, pc))?,
                    0x01a0_0f90 => {
                        mem.store32(addr, self.arm_read_reg(rm, pc))?;
                        mem.store32(addr.wrapping_add(4), self.arm_read_reg(rm + 1, pc))?;
                    }
                    0x01c0_0f90 => mem.store8(addr, self.arm_read_reg(rm, pc) as u8)?,
                    0x01e0_0f90 => mem.store16(addr, self.arm_read_reg(rm, pc) as u16)?,
                    _ => unreachable!(),
                }
                self.write_reg_arm(rd, 0);
            } else {
                self.write_reg_arm(rd, 1);
            }
            self.exclusive_reservation = None;
            return Ok(true);
        }

        Ok(false)
    }

    fn exec_arm_multiply(&mut self, instr: u32, pc: u32) -> Result<bool> {
        if (instr & 0x0ff0_00f0) == 0x0040_0090 {
            let rd_hi = ((instr >> 16) & 0xf) as usize;
            let rd_lo = ((instr >> 12) & 0xf) as usize;
            let rm = ((instr >> 8) & 0xf) as usize;
            let rn = (instr & 0xf) as usize;
            if [rd_hi, rd_lo, rm, rn].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            if rd_hi == rd_lo {
                return Err(Trap::Unpredictable("UMAAL with same high/low register"));
            }
            let result = u64::from(self.arm_read_reg(rn, pc))
                .wrapping_mul(u64::from(self.arm_read_reg(rm, pc)))
                .wrapping_add(u64::from(self.regs[rd_hi]))
                .wrapping_add(u64::from(self.regs[rd_lo]));
            self.regs[rd_lo] = result as u32;
            self.regs[rd_hi] = (result >> 32) as u32;
            return Ok(true);
        }

        let dual_key = instr & 0x0ff0_00d0;
        if matches!(
            dual_key,
            0x0700_0010 | 0x0700_0050 | 0x0740_0010 | 0x0740_0050
        ) {
            let rd_or_hi = ((instr >> 16) & 0xf) as usize;
            let ra_or_lo = ((instr >> 12) & 0xf) as usize;
            let rm = ((instr >> 8) & 0xf) as usize;
            let rn = (instr & 0xf) as usize;
            let exchange = instr & (1 << 5) != 0;
            let long = dual_key & 0x0040_0000 != 0;
            if rd_or_hi == 15 || rm == 15 || rn == 15 || (long && ra_or_lo == 15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let (lo, hi) = signed_dual_products(
                self.arm_read_reg(rn, pc),
                self.arm_read_reg(rm, pc),
                exchange,
            );
            let product = if dual_key & 0x40 == 0 {
                i64::from(lo) + i64::from(hi)
            } else {
                i64::from(lo) - i64::from(hi)
            };

            if long {
                if rd_or_hi == ra_or_lo {
                    return Err(Trap::Unpredictable(
                        "dual long multiply with same high/low register",
                    ));
                }
                let addend = ((u64::from(self.regs[rd_or_hi]) << 32)
                    | u64::from(self.regs[ra_or_lo])) as i64;
                let result = addend.wrapping_add(product);
                self.regs[ra_or_lo] = result as u32;
                self.regs[rd_or_hi] = (result >> 32) as u32;
            } else if ra_or_lo == 15 {
                let result = product as i32;
                if product > i64::from(i32::MAX) || product < i64::from(i32::MIN) {
                    self.cpsr.q = true;
                }
                self.write_reg_arm(rd_or_hi, result as u32);
            } else {
                let addend = i64::from(self.arm_read_reg(ra_or_lo, pc) as i32);
                let result = product + addend;
                if result > i64::from(i32::MAX) || result < i64::from(i32::MIN) {
                    self.cpsr.q = true;
                }
                self.write_reg_arm(rd_or_hi, result as i32 as u32);
            }
            return Ok(true);
        }

        let high_mul_key = instr & 0x0ff0_00d0;
        if high_mul_key == 0x0750_0010 || high_mul_key == 0x0750_00d0 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let ra = ((instr >> 12) & 0xf) as usize;
            let rm = ((instr >> 8) & 0xf) as usize;
            let rn = (instr & 0xf) as usize;
            let round = instr & (1 << 5) != 0;
            if rd == 15 || rm == 15 || rn == 15 || (high_mul_key == 0x0750_00d0 && ra == 15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let product = i64::from(self.arm_read_reg(rn, pc) as i32)
                .wrapping_mul(i64::from(self.arm_read_reg(rm, pc) as i32));
            let result = if ra == 15 {
                signed_high_word(product, round)
            } else {
                let addend = ((u64::from(self.arm_read_reg(ra, pc))) << 32) as i64;
                let temp = if high_mul_key == 0x0750_0010 {
                    addend.wrapping_add(product)
                } else {
                    addend.wrapping_sub(product)
                };
                signed_high_word(temp, round)
            };
            self.write_reg_arm(rd, result);
            return Ok(true);
        }

        if (instr & 0x0ff0_0090) == 0x0100_0080 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let rn = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd, rn, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let rm_half = select_i16(self.arm_read_reg(rm, pc), instr & (1 << 5) != 0);
            let rs_half = select_i16(self.arm_read_reg(rs, pc), instr & (1 << 6) != 0);
            let product = i64::from(rm_half) * i64::from(rs_half);
            let acc = i64::from(self.arm_read_reg(rn, pc) as i32);
            let result = product + acc;
            if result > i64::from(i32::MAX) || result < i64::from(i32::MIN) {
                self.cpsr.q = true;
            }
            self.write_reg_arm(rd, result as i32 as u32);
            return Ok(true);
        }

        if (instr & 0x0ff0_0090) == 0x0140_0080 {
            let rd_hi = ((instr >> 16) & 0xf) as usize;
            let rd_lo = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd_hi, rd_lo, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            if rd_hi == rd_lo {
                return Err(Trap::Unpredictable(
                    "dual long multiply with same high/low register",
                ));
            }
            let rm_half = select_i16(self.arm_read_reg(rm, pc), instr & (1 << 5) != 0);
            let rs_half = select_i16(self.arm_read_reg(rs, pc), instr & (1 << 6) != 0);
            let product = i64::from(rm_half) * i64::from(rs_half);
            let acc = ((u64::from(self.regs[rd_hi]) << 32) | u64::from(self.regs[rd_lo])) as i64;
            let result = acc.wrapping_add(product);
            self.regs[rd_lo] = result as u32;
            self.regs[rd_hi] = (result >> 32) as u32;
            return Ok(true);
        }

        if (instr & 0x0ff0_00b0) == 0x0120_0080 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let rn = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd, rn, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let rs_half = select_i16(self.arm_read_reg(rs, pc), instr & (1 << 6) != 0);
            let product = i64::from(self.arm_read_reg(rm, pc) as i32) * i64::from(rs_half);
            let rounded = product >> 16;
            let result = rounded + i64::from(self.arm_read_reg(rn, pc) as i32);
            if result > i64::from(i32::MAX) || result < i64::from(i32::MIN) {
                self.cpsr.q = true;
            }
            self.write_reg_arm(rd, result as i32 as u32);
            return Ok(true);
        }

        if (instr & 0x0ff0_00b0) == 0x0120_00a0 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let rs_half = select_i16(self.arm_read_reg(rs, pc), instr & (1 << 6) != 0);
            let product = i64::from(self.arm_read_reg(rm, pc) as i32) * i64::from(rs_half);
            self.write_reg_arm(rd, (product >> 16) as i32 as u32);
            return Ok(true);
        }

        if (instr & 0x0ff0_0090) == 0x0160_0080 {
            let rd = ((instr >> 16) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let rm_half = select_i16(self.arm_read_reg(rm, pc), instr & (1 << 5) != 0);
            let rs_half = select_i16(self.arm_read_reg(rs, pc), instr & (1 << 6) != 0);
            let result = i32::from(rm_half).wrapping_mul(i32::from(rs_half));
            self.write_reg_arm(rd, result as u32);
            return Ok(true);
        }

        if (instr & 0x0f80_00f0) == 0x0080_0090 {
            let signed = instr & (1 << 22) != 0;
            let accumulate = instr & (1 << 21) != 0;
            let set_flags = instr & (1 << 20) != 0;
            let rd_hi = ((instr >> 16) & 0xf) as usize;
            let rd_lo = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if [rd_hi, rd_lo, rs, rm].contains(&15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            if rd_hi == rd_lo {
                return Err(Trap::Unpredictable(
                    "long multiply with same high/low register",
                ));
            }
            let product = if signed {
                (self.arm_read_reg(rm, pc) as i32 as i64)
                    .wrapping_mul(self.arm_read_reg(rs, pc) as i32 as i64) as u64
            } else {
                u64::from(self.arm_read_reg(rm, pc))
                    .wrapping_mul(u64::from(self.arm_read_reg(rs, pc)))
            };
            let acc = if accumulate {
                (u64::from(self.regs[rd_hi]) << 32) | u64::from(self.regs[rd_lo])
            } else {
                0
            };
            let result = product.wrapping_add(acc);
            self.regs[rd_lo] = result as u32;
            self.regs[rd_hi] = (result >> 32) as u32;
            if set_flags {
                self.cpsr.n = result & (1 << 63) != 0;
                self.cpsr.z = result == 0;
            }
            return Ok(true);
        }

        if (instr & 0x0fc0_00f0) == 0x0000_0090 {
            let accumulate = instr & (1 << 21) != 0;
            let set_flags = instr & (1 << 20) != 0;
            let rd = ((instr >> 16) & 0xf) as usize;
            let rn = ((instr >> 12) & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let rm = (instr & 0xf) as usize;
            if rd == 15 || rs == 15 || rm == 15 || (accumulate && rn == 15) {
                return Err(Trap::Unpredictable("multiply with PC register"));
            }
            let mut result = self
                .arm_read_reg(rm, pc)
                .wrapping_mul(self.arm_read_reg(rs, pc));
            if accumulate {
                result = result.wrapping_add(self.arm_read_reg(rn, pc));
            }
            self.write_reg_arm(rd, result);
            if set_flags {
                self.cpsr.set_nz(result);
            }
            return Ok(true);
        }

        Ok(false)
    }

    fn exec_arm_halfword_transfer<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<bool> {
        if instr & 0x90 != 0x90 || ((instr >> 25) & 0b111) != 0 {
            return Ok(false);
        }
        let op = (instr >> 5) & 0b11;
        if op == 0 {
            return Ok(false);
        }
        let p = instr & (1 << 24) != 0;
        let u = instr & (1 << 23) != 0;
        let i = instr & (1 << 22) != 0;
        let w = instr & (1 << 21) != 0;
        let l = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        let rm = if i {
            None
        } else {
            Some((instr & 0xf) as usize)
        };
        let wback = !p || w;

        if let Some(rm) = rm {
            if rm == 15 {
                return Err(Trap::Unpredictable(
                    "halfword transfer with PC offset register",
                ));
            }
        }

        if l {
            if rd == 15 {
                return Err(Trap::Unpredictable(
                    "halfword load with PC destination register",
                ));
            }
            if wback && (rn == 15 || rn == rd) {
                return Err(Trap::Unpredictable(
                    "halfword load writeback with invalid base register",
                ));
            }
        } else if op == 0b01 {
            if rd == 15 {
                return Err(Trap::Unpredictable(
                    "halfword store with PC source register",
                ));
            }
            if wback && (rn == 15 || rn == rd) {
                return Err(Trap::Unpredictable(
                    "halfword store writeback with invalid base register",
                ));
            }
        } else {
            if !p && w {
                return Err(Trap::Unpredictable(
                    "doubleword transfer with invalid post-index form",
                ));
            }
            if rd >= 14 || rd % 2 != 0 {
                return Err(Trap::Unpredictable(if op == 0b10 {
                    "LDRD with invalid register pair"
                } else {
                    "STRD with invalid register pair"
                }));
            }
            if wback && (rn == 15 || rn == rd || rn == rd + 1) {
                return Err(Trap::Unpredictable(
                    "doubleword transfer writeback with invalid base register",
                ));
            }
            if let Some(rm) = rm {
                if op == 0b10 && (rm == rd || rm == rd + 1) {
                    return Err(Trap::Unpredictable(
                        "LDRD register offset overlaps destination",
                    ));
                }
            }
        }

        let offset = if i {
            ((instr >> 4) & 0xf0) | (instr & 0xf)
        } else {
            self.arm_read_reg(rm.expect("register halfword offset"), pc)
        };
        let base = self.arm_read_reg(rn, pc);
        let off_addr = if u {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if p { off_addr } else { base };
        if l {
            let value = match op {
                0b01 => u32::from(mem.load16(addr)?),
                0b10 => sign_extend(u32::from(mem.load8(addr)?), 8) as u32,
                0b11 => sign_extend(u32::from(mem.load16(addr)?), 16) as u32,
                _ => unreachable!(),
            };
            self.write_reg_arm(rd, value);
        } else if op == 0b01 {
            self.store16(mem, addr, self.arm_read_reg(rd, pc) as u16)?;
        } else if op == 0b10 {
            let lo = mem.load32(addr)?;
            let hi = mem.load32(addr.wrapping_add(4))?;
            self.write_reg_arm(rd, lo);
            self.write_reg_arm(rd + 1, hi);
        } else if op == 0b11 {
            self.store32(mem, addr, self.arm_read_reg(rd, pc))?;
            self.store32(mem, addr.wrapping_add(4), self.arm_read_reg(rd + 1, pc))?;
        } else {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if !p || w {
            self.write_reg_arm(rn, off_addr);
        }
        Ok(true)
    }

    fn exec_arm_single_transfer<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        if (instr >> 25) & 0b111 == 0b011 && instr & (1 << 4) != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        let i = instr & (1 << 25) != 0;
        let p = instr & (1 << 24) != 0;
        let u = instr & (1 << 23) != 0;
        let b = instr & (1 << 22) != 0;
        let w = instr & (1 << 21) != 0;
        let l = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        let wback = !p || w;

        if i {
            let rm = (instr & 0xf) as usize;
            if rm == 15 {
                return Err(Trap::Unpredictable(
                    "single transfer with PC offset register",
                ));
            }
        }
        if b && rd == 15 {
            return Err(Trap::Unpredictable(
                "byte transfer with PC destination/source register",
            ));
        }
        if wback && rn == 15 {
            return Err(Trap::Unpredictable(
                "single transfer writeback with PC base register",
            ));
        }
        if wback && rn == rd {
            return Err(Trap::Unpredictable(
                "single transfer writeback overlaps transferred register",
            ));
        }

        let base = self.arm_read_reg(rn, pc);
        let offset = if i {
            let rm = (instr & 0xf) as usize;
            let shift = decode_arm_shift((instr >> 5) & 0b11);
            let amount = (instr >> 7) & 0x1f;
            shift_operand(self.arm_read_reg(rm, pc), shift, amount, self.cpsr.c, true).value
        } else {
            instr & 0xfff
        };
        let off_addr = if u {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if p { off_addr } else { base };
        if l {
            let value = if b {
                u32::from(mem.load8(addr)?)
            } else {
                mem.load32(addr)?
            };
            self.write_reg_arm(rd, value);
        } else if b {
            self.store8(mem, addr, self.arm_read_reg(rd, pc) as u8)?;
        } else {
            self.store32(mem, addr, self.arm_read_reg(rd, pc))?;
        }
        if !p || w {
            self.write_reg_arm(rn, off_addr);
        }
        Ok(())
    }

    fn exec_arm_block_transfer<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        let p = instr & (1 << 24) != 0;
        let u = instr & (1 << 23) != 0;
        let s = instr & (1 << 22) != 0;
        let w = instr & (1 << 21) != 0;
        let l = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let reglist = instr & 0xffff;
        if rn == 15 {
            return Err(Trap::Unpredictable("block transfer with PC base register"));
        }
        if s {
            return Err(Trap::Unpredictable("block transfer user-mode/S bit"));
        }
        if reglist == 0 {
            return Err(Trap::Unpredictable("empty block transfer register list"));
        }
        if l && w && reglist & (1 << rn) != 0 {
            return Err(Trap::Unpredictable(
                "LDM writeback with base in register list",
            ));
        }
        let count = reglist.count_ones();
        let base = self.arm_read_reg(rn, pc);
        let start = match (u, p) {
            (true, false) => base,
            (true, true) => base.wrapping_add(4),
            (false, false) => base.wrapping_sub(4 * (count - 1)),
            (false, true) => base.wrapping_sub(4 * count),
        };
        let final_base = if u {
            base.wrapping_add(4 * count)
        } else {
            base.wrapping_sub(4 * count)
        };
        let mut addr = start;
        for reg in 0..16 {
            if reglist & (1 << reg) == 0 {
                continue;
            }
            if l {
                let value = mem.load32(addr)?;
                self.write_reg_arm(reg, value);
            } else {
                self.store32(mem, addr, self.arm_read_reg(reg, pc))?;
            }
            addr = addr.wrapping_add(4);
        }
        if w {
            self.write_reg_arm(rn, final_base);
        }
        Ok(())
    }

    fn exec_arm_data_processing<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        _mem: &mut M,
    ) -> Result<()> {
        let immediate = instr & (1 << 25) != 0;
        let opcode = (instr >> 21) & 0xf;
        let set_flags = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        if !immediate && instr & (1 << 4) != 0 {
            let rm = (instr & 0xf) as usize;
            let rs = ((instr >> 8) & 0xf) as usize;
            let write_result = !matches!(opcode, 0x8..=0xb);
            if rn == 15 || rm == 15 || rs == 15 || (write_result && rd == 15) {
                return Err(Trap::Unpredictable(
                    "data-processing register shift with PC register",
                ));
            }
        }
        let lhs = self.arm_read_reg(rn, pc);
        let shifted = if immediate {
            let (value, carry) = arm_expand_imm(instr & 0xfff);
            Shifted {
                value,
                carry: carry.unwrap_or(self.cpsr.c),
            }
        } else {
            let rm = (instr & 0xf) as usize;
            let shift = decode_arm_shift((instr >> 5) & 0b11);
            if instr & (1 << 4) != 0 {
                let rs = ((instr >> 8) & 0xf) as usize;
                let amount = self.arm_read_reg(rs, pc) & 0xff;
                shift_operand(self.arm_read_reg(rm, pc), shift, amount, self.cpsr.c, false)
            } else {
                let amount = (instr >> 7) & 0x1f;
                shift_operand(self.arm_read_reg(rm, pc), shift, amount, self.cpsr.c, true)
            }
        };

        let op2 = shifted.value;
        let (result, carry, overflow, write_result) = match opcode {
            0x0 => (lhs & op2, shifted.carry, self.cpsr.v, true),
            0x1 => (lhs ^ op2, shifted.carry, self.cpsr.v, true),
            0x2 => {
                let (r, c, v) = sub_with_flags(lhs, op2);
                (r, c, v, true)
            }
            0x3 => {
                let (r, c, v) = sub_with_flags(op2, lhs);
                (r, c, v, true)
            }
            0x4 => {
                let (r, c, v) = add_with_flags(lhs, op2);
                (r, c, v, true)
            }
            0x5 => {
                let (r, c, v) = add_with_carry(lhs, op2, self.cpsr.c);
                (r, c, v, true)
            }
            0x6 => {
                let (r, c, v) = add_with_carry(lhs, !op2, self.cpsr.c);
                (r, c, v, true)
            }
            0x7 => {
                let (r, c, v) = add_with_carry(op2, !lhs, self.cpsr.c);
                (r, c, v, true)
            }
            0x8 => (lhs & op2, shifted.carry, self.cpsr.v, false),
            0x9 => (lhs ^ op2, shifted.carry, self.cpsr.v, false),
            0xa => {
                let (r, c, v) = sub_with_flags(lhs, op2);
                (r, c, v, false)
            }
            0xb => {
                let (r, c, v) = add_with_flags(lhs, op2);
                (r, c, v, false)
            }
            0xc => (lhs | op2, shifted.carry, self.cpsr.v, true),
            0xd => (op2, shifted.carry, self.cpsr.v, true),
            0xe => (lhs & !op2, shifted.carry, self.cpsr.v, true),
            0xf => (!op2, shifted.carry, self.cpsr.v, true),
            _ => unreachable!(),
        };

        if write_result && rd == 15 && set_flags {
            return Err(Trap::Privileged {
                pc,
                instr,
                operation: "data-processing exception return",
            });
        }

        if write_result {
            self.write_reg_arm(rd, result);
        }
        if set_flags || !write_result {
            match opcode {
                0x0 | 0x1 | 0x8 | 0x9 | 0xc | 0xd | 0xe | 0xf => {
                    self.cpsr.set_nzc(result, carry);
                }
                _ => self.cpsr.set_nzcv(result, carry, overflow),
            }
        }
        Ok(())
    }

    pub fn execute_thumb<M: Memory>(&mut self, instr: u16, pc: u32, mem: &mut M) -> Result<()> {
        let op = instr >> 13;
        match op {
            0b000 => self.exec_thumb_shift_add_sub(instr, pc),
            0b001 => self.exec_thumb_imm(instr),
            0b010 => self.exec_thumb_alu_hi_bx_load_literal(instr, pc, mem),
            0b011 | 0b100 => self.exec_thumb_load_store(instr, pc, mem),
            0b101 => self.exec_thumb_misc(instr, pc, mem),
            0b110 => self.exec_thumb_multi_or_branch(instr, pc, mem),
            0b111 => self.exec_thumb_branch_long(instr, pc),
            _ => unreachable!(),
        }
    }

    fn exec_thumb_shift_add_sub(&mut self, instr: u16, pc: u32) -> Result<()> {
        if instr & 0x1800 == 0x1800 {
            let immediate = instr & 0x0400 != 0;
            let subtract = instr & 0x0200 != 0;
            let rn_or_imm = ((instr >> 6) & 0x7) as usize;
            let rs = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let rhs = if immediate {
                rn_or_imm as u32
            } else {
                self.regs[rn_or_imm]
            };
            let (value, carry, overflow) = if subtract {
                sub_with_flags(self.regs[rs], rhs)
            } else {
                add_with_flags(self.regs[rs], rhs)
            };
            self.regs[rd] = value;
            self.cpsr.set_nzcv(value, carry, overflow);
            return Ok(());
        }

        let kind = (instr >> 11) & 0x3;
        let amount = ((instr >> 6) & 0x1f) as u32;
        let rs = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let shifted = match kind {
            0 => shift_operand(self.regs[rs], Shift::Lsl, amount, self.cpsr.c, true),
            1 => shift_operand(self.regs[rs], Shift::Lsr, amount, self.cpsr.c, true),
            2 => shift_operand(self.regs[rs], Shift::Asr, amount, self.cpsr.c, true),
            _ => return Err(Trap::UndefinedThumb { pc, instr }),
        };
        self.regs[rd] = shifted.value;
        self.cpsr.set_nzc(shifted.value, shifted.carry);
        Ok(())
    }

    fn exec_thumb_imm(&mut self, instr: u16) -> Result<()> {
        let op = (instr >> 11) & 0x3;
        let rd = ((instr >> 8) & 0x7) as usize;
        let imm = u32::from(instr & 0xff);
        match op {
            0 => {
                self.regs[rd] = imm;
                self.cpsr.set_nz(imm);
            }
            1 => {
                let (value, carry, overflow) = sub_with_flags(self.regs[rd], imm);
                self.cpsr.set_nzcv(value, carry, overflow);
            }
            2 => {
                let (value, carry, overflow) = add_with_flags(self.regs[rd], imm);
                self.regs[rd] = value;
                self.cpsr.set_nzcv(value, carry, overflow);
            }
            3 => {
                let (value, carry, overflow) = sub_with_flags(self.regs[rd], imm);
                self.regs[rd] = value;
                self.cpsr.set_nzcv(value, carry, overflow);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn exec_thumb_alu_hi_bx_load_literal<M: Memory>(
        &mut self,
        instr: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        if instr & 0xf800 == 0x4800 {
            let rd = ((instr >> 8) & 0x7) as usize;
            let addr = pc.wrapping_add(4) & !3;
            self.regs[rd] = mem.load32(addr.wrapping_add(u32::from(instr & 0xff) << 2))?;
            return Ok(());
        }

        if instr & 0xfc00 == 0x4000 {
            let op = (instr >> 6) & 0xf;
            let rm = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let lhs = self.regs[rd];
            let rhs = self.regs[rm];
            match op {
                0x0 => {
                    self.regs[rd] &= rhs;
                    self.cpsr.set_nz(self.regs[rd]);
                }
                0x1 => {
                    self.regs[rd] ^= rhs;
                    self.cpsr.set_nz(self.regs[rd]);
                }
                0x2 => {
                    let s = shift_operand(lhs, Shift::Lsl, rhs & 0xff, self.cpsr.c, false);
                    self.regs[rd] = s.value;
                    self.cpsr.set_nzc(s.value, s.carry);
                }
                0x3 => {
                    let s = shift_operand(lhs, Shift::Lsr, rhs & 0xff, self.cpsr.c, false);
                    self.regs[rd] = s.value;
                    self.cpsr.set_nzc(s.value, s.carry);
                }
                0x4 => {
                    let s = shift_operand(lhs, Shift::Asr, rhs & 0xff, self.cpsr.c, false);
                    self.regs[rd] = s.value;
                    self.cpsr.set_nzc(s.value, s.carry);
                }
                0x5 => {
                    let (value, carry, overflow) = add_with_carry(lhs, rhs, self.cpsr.c);
                    self.regs[rd] = value;
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                0x6 => {
                    let (value, carry, overflow) = add_with_carry(lhs, !rhs, self.cpsr.c);
                    self.regs[rd] = value;
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                0x7 => {
                    let s = shift_operand(lhs, Shift::Ror, rhs & 0xff, self.cpsr.c, false);
                    self.regs[rd] = s.value;
                    self.cpsr.set_nzc(s.value, s.carry);
                }
                0x8 => {
                    let value = lhs & rhs;
                    self.cpsr.set_nz(value);
                }
                0x9 => {
                    let (value, carry, overflow) = sub_with_flags(0, rhs);
                    self.regs[rd] = value;
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                0xa => {
                    let (value, carry, overflow) = sub_with_flags(lhs, rhs);
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                0xb => {
                    let (value, carry, overflow) = add_with_flags(lhs, rhs);
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                0xc => {
                    self.regs[rd] |= rhs;
                    self.cpsr.set_nz(self.regs[rd]);
                }
                0xd => {
                    self.regs[rd] = lhs.wrapping_mul(rhs);
                    self.cpsr.set_nz(self.regs[rd]);
                }
                0xe => {
                    self.regs[rd] = lhs & !rhs;
                    self.cpsr.set_nz(self.regs[rd]);
                }
                0xf => {
                    self.regs[rd] = !rhs;
                    self.cpsr.set_nz(self.regs[rd]);
                }
                _ => unreachable!(),
            }
            return Ok(());
        }

        if instr & 0xfc00 == 0x4400 {
            let op = (instr >> 8) & 0x3;
            let h1 = instr & 0x80 != 0;
            let h2 = instr & 0x40 != 0;
            let rd = usize::from((instr & 0x7) | if h1 { 8 } else { 0 });
            let rm = usize::from(((instr >> 3) & 0x7) | if h2 { 8 } else { 0 });
            let lhs = self.thumb_read_reg(rd, pc);
            let rhs = self.thumb_read_reg(rm, pc);
            match op {
                0 => {
                    if rd == 15 && rm == 15 {
                        return Err(Trap::Unpredictable(
                            "Thumb high-register ADD with PC operands",
                        ));
                    }
                    self.write_reg_thumb(rd, lhs.wrapping_add(rhs));
                }
                1 => {
                    if (rd < 8 && rm < 8) || rd == 15 || rm == 15 {
                        return Err(Trap::Unpredictable(
                            "Thumb high-register CMP with invalid registers",
                        ));
                    }
                    let (value, carry, overflow) = sub_with_flags(lhs, rhs);
                    self.cpsr.set_nzcv(value, carry, overflow);
                }
                2 => self.write_reg_thumb(rd, rhs),
                3 => {
                    if h1 {
                        self.regs[14] = pc.wrapping_add(2) | 1;
                    }
                    self.branch_exchange(rhs);
                }
                _ => unreachable!(),
            }
            return Ok(());
        }

        if instr & 0xf000 == 0x5000 {
            return self.exec_thumb_load_store(instr, pc, mem);
        }

        Err(Trap::UndefinedThumb { pc, instr })
    }

    fn exec_thumb_load_store<M: Memory>(&mut self, instr: u16, pc: u32, mem: &mut M) -> Result<()> {
        if instr & 0xf000 == 0x5000 {
            let op = (instr >> 9) & 0x7;
            let ro = ((instr >> 6) & 0x7) as usize;
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let addr = self.regs[rb].wrapping_add(self.regs[ro]);
            match op {
                0 => self.store32(mem, addr, self.regs[rd])?,
                1 => self.store16(mem, addr, self.regs[rd] as u16)?,
                2 => self.store8(mem, addr, self.regs[rd] as u8)?,
                3 => self.regs[rd] = sign_extend(u32::from(mem.load8(addr)?), 8) as u32,
                4 => self.regs[rd] = mem.load32(addr)?,
                5 => self.regs[rd] = u32::from(mem.load16(addr)?),
                6 => self.regs[rd] = u32::from(mem.load8(addr)?),
                7 => self.regs[rd] = sign_extend(u32::from(mem.load16(addr)?), 16) as u32,
                _ => unreachable!(),
            }
            return Ok(());
        }

        if instr & 0xe000 == 0x6000 {
            let load = instr & 0x0800 != 0;
            let byte = instr & 0x1000 != 0;
            let imm5 = u32::from((instr >> 6) & 0x1f);
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let offset = if byte { imm5 } else { imm5 << 2 };
            let addr = self.regs[rb].wrapping_add(offset);
            if load {
                self.regs[rd] = if byte {
                    u32::from(mem.load8(addr)?)
                } else {
                    mem.load32(addr)?
                };
            } else if byte {
                self.store8(mem, addr, self.regs[rd] as u8)?;
            } else {
                self.store32(mem, addr, self.regs[rd])?;
            }
            return Ok(());
        }

        if instr & 0xf000 == 0x8000 {
            let load = instr & 0x0800 != 0;
            let imm5 = u32::from((instr >> 6) & 0x1f);
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let addr = self.regs[rb].wrapping_add(imm5 << 1);
            if load {
                self.regs[rd] = u32::from(mem.load16(addr)?);
            } else {
                self.store16(mem, addr, self.regs[rd] as u16)?;
            }
            return Ok(());
        }

        if instr & 0xf000 == 0x9000 {
            let load = instr & 0x0800 != 0;
            let rd = ((instr >> 8) & 0x7) as usize;
            let addr = self.regs[13].wrapping_add(u32::from(instr & 0xff) << 2);
            if load {
                self.regs[rd] = mem.load32(addr)?;
            } else {
                self.store32(mem, addr, self.regs[rd])?;
            }
            return Ok(());
        }

        Err(Trap::UndefinedThumb { pc, instr })
    }

    fn exec_thumb_misc<M: Memory>(&mut self, instr: u16, pc: u32, mem: &mut M) -> Result<()> {
        if instr & 0xff00 == 0xb200 {
            let op = (instr >> 6) & 0x3;
            let rm = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            self.regs[rd] = match op {
                0 => sign_extend(self.regs[rm] & 0xffff, 16) as u32,
                1 => sign_extend(self.regs[rm] & 0xff, 8) as u32,
                2 => self.regs[rm] & 0xffff,
                3 => self.regs[rm] & 0xff,
                _ => unreachable!(),
            };
            return Ok(());
        }

        if instr & 0xff00 == 0xba00 {
            let op = (instr >> 6) & 0x3;
            let rm = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let value = self.regs[rm];
            self.regs[rd] = match op {
                0 => value.swap_bytes(),
                1 => ((value & 0x00ff_00ff) << 8) | ((value & 0xff00_ff00) >> 8),
                3 => {
                    let half = ((value & 0xff) << 8) | ((value >> 8) & 0xff);
                    sign_extend(half, 16) as u32
                }
                _ => return Err(Trap::UndefinedThumb { pc, instr }),
            };
            return Ok(());
        }

        if instr & 0xff00 == 0xbe00 {
            return Err(Trap::Breakpoint {
                pc,
                comment: u32::from(instr & 0xff),
            });
        }

        if instr & 0xfff7 == 0xb650 {
            if instr & 0x0008 != 0 {
                return Err(Trap::Unpredictable("SETEND big-endian mode unsupported"));
            }
            return Ok(());
        }

        if instr & 0xffe8 == 0xb660 {
            return Err(Trap::Privileged {
                pc,
                instr: u32::from(instr),
                operation: "CPS",
            });
        }

        if instr & 0xf000 == 0xa000 {
            let sp = instr & 0x0800 != 0;
            let rd = ((instr >> 8) & 0x7) as usize;
            let base = if sp {
                self.regs[13]
            } else {
                pc.wrapping_add(4) & !3
            };
            self.regs[rd] = base.wrapping_add(u32::from(instr & 0xff) << 2);
            return Ok(());
        }

        if instr & 0xff00 == 0xb000 {
            let offset = u32::from(instr & 0x7f) << 2;
            if instr & 0x80 != 0 {
                self.regs[13] = self.regs[13].wrapping_sub(offset);
            } else {
                self.regs[13] = self.regs[13].wrapping_add(offset);
            }
            return Ok(());
        }

        if instr & 0xf600 == 0xb400 {
            let pop = instr & 0x0800 != 0;
            let r = instr & 0x0100 != 0;
            let list = instr & 0xff;
            if list == 0 && !r {
                return Err(Trap::Unpredictable("empty Thumb push/pop register list"));
            }
            if pop {
                for reg in 0..8 {
                    if list & (1 << reg) != 0 {
                        self.regs[reg] = mem.load32(self.regs[13])?;
                        self.regs[13] = self.regs[13].wrapping_add(4);
                    }
                }
                if r {
                    let target = mem.load32(self.regs[13])?;
                    self.regs[13] = self.regs[13].wrapping_add(4);
                    self.branch_exchange(target);
                }
            } else {
                let count = list.count_ones() + u32::from(r);
                self.regs[13] = self.regs[13].wrapping_sub(4 * count);
                let mut addr = self.regs[13];
                for reg in 0..8 {
                    if list & (1 << reg) != 0 {
                        self.store32(mem, addr, self.regs[reg])?;
                        addr = addr.wrapping_add(4);
                    }
                }
                if r {
                    self.store32(mem, addr, self.regs[14])?;
                }
            }
            return Ok(());
        }

        Err(Trap::UndefinedThumb { pc, instr })
    }

    fn exec_thumb_multi_or_branch<M: Memory>(
        &mut self,
        instr: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        if instr & 0xf000 == 0xc000 {
            let load = instr & 0x0800 != 0;
            let rb = ((instr >> 8) & 0x7) as usize;
            let list = instr & 0xff;
            let mut addr = self.regs[rb];
            if list == 0 {
                return Err(Trap::Unpredictable("empty Thumb load/store multiple list"));
            }
            if !load && list & (1 << rb) != 0 && rb as u32 != list.trailing_zeros() {
                return Err(Trap::Unpredictable(
                    "Thumb STM writeback base not first in register list",
                ));
            }
            for reg in 0..8 {
                if list & (1 << reg) == 0 {
                    continue;
                }
                if load {
                    self.regs[reg] = mem.load32(addr)?;
                } else {
                    self.store32(mem, addr, self.regs[reg])?;
                }
                addr = addr.wrapping_add(4);
            }
            if !load || list & (1 << rb) == 0 {
                self.regs[rb] = addr;
            }
            return Ok(());
        }

        if instr & 0xf000 == 0xd000 {
            let cond = (instr >> 8) & 0xf;
            if cond == 0xf {
                return Err(Trap::SoftwareInterrupt {
                    pc,
                    comment: u32::from(instr & 0xff),
                });
            }
            if cond == 0xe {
                return Err(Trap::UndefinedThumb { pc, instr });
            }
            if condition_passed(u32::from(cond), self.cpsr) {
                let off = sign_extend(u32::from(instr & 0xff) << 1, 9);
                self.regs[15] = pc.wrapping_add(4).wrapping_add(off as u32);
            }
            return Ok(());
        }

        Err(Trap::UndefinedThumb { pc, instr })
    }

    fn exec_thumb_branch_long(&mut self, instr: u16, pc: u32) -> Result<()> {
        if instr & 0xf800 == 0xe000 {
            let off = sign_extend(u32::from(instr & 0x7ff) << 1, 12);
            self.regs[15] = pc.wrapping_add(4).wrapping_add(off as u32);
            return Ok(());
        }

        if instr & 0xf800 == 0xe800 {
            let Some(prefix) = self.thumb_bl_prefix.take() else {
                return Err(Trap::Unpredictable("Thumb BLX suffix without prefix"));
            };
            self.regs[14] = pc.wrapping_add(2) | 1;
            let target = prefix.wrapping_add(u32::from(instr & 0x7ff) << 1) & !3;
            self.branch_exchange(target);
            return Ok(());
        }

        if instr & 0xf800 == 0xf000 {
            let off = sign_extend(u32::from(instr & 0x7ff) << 12, 23);
            self.thumb_bl_prefix = Some(pc.wrapping_add(4).wrapping_add(off as u32));
            return Ok(());
        }

        if instr & 0xf800 == 0xf800 {
            let Some(prefix) = self.thumb_bl_prefix.take() else {
                return Err(Trap::Unpredictable("Thumb BL suffix without prefix"));
            };
            self.regs[14] = (pc.wrapping_add(2)) | 1;
            self.regs[15] = prefix.wrapping_add(u32::from(instr & 0x7ff) << 1);
            return Ok(());
        }

        Err(Trap::UndefinedThumb { pc, instr })
    }

    fn exec_arm_parallel_op(&mut self, family: u32, op: u32, rn: u32, rm: u32) -> u32 {
        let unsigned = family >= 5;
        let saturating = family == 2 || family == 6;
        let halving = family == 3 || family == 7;

        match op {
            0 | 1 | 2 | 3 => {
                let rn_lo = rn as u16;
                let rn_hi = (rn >> 16) as u16;
                let rm_lo = rm as u16;
                let rm_hi = (rm >> 16) as u16;
                let (lo, hi, ge_lo, ge_hi) = match op {
                    0 => (
                        parallel_lane16(rn_lo, rm_lo, true, unsigned, saturating, halving),
                        parallel_lane16(rn_hi, rm_hi, true, unsigned, saturating, halving),
                        parallel_ge16(rn_lo, rm_lo, true, unsigned),
                        parallel_ge16(rn_hi, rm_hi, true, unsigned),
                    ),
                    1 => (
                        parallel_lane16(rn_lo, rm_hi, false, unsigned, saturating, halving),
                        parallel_lane16(rn_hi, rm_lo, true, unsigned, saturating, halving),
                        parallel_ge16(rn_lo, rm_hi, false, unsigned),
                        parallel_ge16(rn_hi, rm_lo, true, unsigned),
                    ),
                    2 => (
                        parallel_lane16(rn_lo, rm_hi, true, unsigned, saturating, halving),
                        parallel_lane16(rn_hi, rm_lo, false, unsigned, saturating, halving),
                        parallel_ge16(rn_lo, rm_hi, true, unsigned),
                        parallel_ge16(rn_hi, rm_lo, false, unsigned),
                    ),
                    3 => (
                        parallel_lane16(rn_lo, rm_lo, false, unsigned, saturating, halving),
                        parallel_lane16(rn_hi, rm_hi, false, unsigned, saturating, halving),
                        parallel_ge16(rn_lo, rm_lo, false, unsigned),
                        parallel_ge16(rn_hi, rm_hi, false, unsigned),
                    ),
                    _ => unreachable!(),
                };
                if !saturating && !halving {
                    self.cpsr.ge =
                        (if ge_lo { 0b0011 } else { 0 }) | (if ge_hi { 0b1100 } else { 0 });
                }
                u32::from(lo) | (u32::from(hi) << 16)
            }
            4 | 7 => {
                let add = op == 4;
                let mut result = 0u32;
                let mut ge = 0u8;
                for lane in 0..4 {
                    let shift = lane * 8;
                    let a = ((rn >> shift) & 0xff) as u8;
                    let b = ((rm >> shift) & 0xff) as u8;
                    let out = parallel_lane8(a, b, add, unsigned, saturating, halving);
                    result |= u32::from(out) << shift;
                    if parallel_ge8(a, b, add, unsigned) {
                        ge |= 1 << lane;
                    }
                }
                if !saturating && !halving {
                    self.cpsr.ge = ge;
                }
                result
            }
            _ => unreachable!(),
        }
    }

    fn dreg_bits(&self, idx: usize) -> u64 {
        let lo = u64::from(self.sregs[idx * 2]);
        let hi = u64::from(self.sregs[idx * 2 + 1]);
        lo | (hi << 32)
    }

    fn set_dreg_bits(&mut self, idx: usize, value: u64) {
        self.sregs[idx * 2] = value as u32;
        self.sregs[idx * 2 + 1] = (value >> 32) as u32;
    }

    fn checked_dreg_bits(&self, idx: usize) -> Result<u64> {
        self.check_dreg(idx)?;
        Ok(self.dreg_bits(idx))
    }

    fn set_checked_dreg_bits(&mut self, idx: usize, value: u64) -> Result<()> {
        self.check_dreg(idx)?;
        self.set_dreg_bits(idx, value);
        Ok(())
    }

    fn store8<M: Memory>(&mut self, mem: &mut M, addr: u32, value: u8) -> Result<()> {
        mem.store8(addr, value)?;
        self.clear_exclusive_if_store_overlaps(addr, 1);
        Ok(())
    }

    fn store16<M: Memory>(&mut self, mem: &mut M, addr: u32, value: u16) -> Result<()> {
        mem.store16(addr, value)?;
        self.clear_exclusive_if_store_overlaps(addr, 2);
        Ok(())
    }

    fn store32<M: Memory>(&mut self, mem: &mut M, addr: u32, value: u32) -> Result<()> {
        mem.store32(addr, value)?;
        self.clear_exclusive_if_store_overlaps(addr, 4);
        Ok(())
    }

    fn clear_exclusive_if_store_overlaps(&mut self, addr: u32, size: u8) {
        let Some(reservation) = self.exclusive_reservation else {
            return;
        };
        let store_start = u64::from(addr);
        let store_end = store_start + u64::from(size);
        let res_start = u64::from(reservation.addr);
        let res_end = res_start + u64::from(reservation.size);
        if store_start < res_end && res_start < store_end {
            self.exclusive_reservation = None;
        }
    }

    fn check_dreg(&self, idx: usize) -> Result<()> {
        if idx >= 16 {
            Err(Trap::Unpredictable("VFP double register out of range"))
        } else {
            Ok(())
        }
    }

    fn exec_vfp_2op_f32<F>(&mut self, mut vd: usize, mut vm: usize, op: F) -> Result<()>
    where
        F: Fn(u32) -> (u32, u32),
    {
        let plan = self.vfp_vector_plan(false, vd, vm)?;
        let mut src = self.sregs[vm];
        for lane in 0..plan.count {
            let (result, exception_flags) = op(src);
            self.sregs[vd] = result;
            self.fpscr |= exception_flags;
            if lane + 1 == plan.count {
                break;
            }
            vd = Self::vfp_advance_sreg(vd, plan.delta_d);
            if plan.delta_m != 0 {
                vm = Self::vfp_advance_sreg(vm, plan.delta_m);
                src = self.sregs[vm];
            }
        }
        Ok(())
    }

    fn exec_vfp_2op_f64<F>(&mut self, mut vd: usize, mut vm: usize, op: F) -> Result<()>
    where
        F: Fn(u64) -> (u64, u32),
    {
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        let plan = self.vfp_vector_plan(true, vd, vm)?;
        let mut src = self.dreg_bits(vm);
        for lane in 0..plan.count {
            let (result, exception_flags) = op(src);
            self.set_dreg_bits(vd, result);
            self.fpscr |= exception_flags;
            if lane + 1 == plan.count {
                break;
            }
            vd = Self::vfp_advance_dreg(vd, plan.delta_d);
            if plan.delta_m != 0 {
                vm = Self::vfp_advance_dreg(vm, plan.delta_m);
                src = self.dreg_bits(vm);
            }
        }
        Ok(())
    }

    fn exec_vfp_3op_f32<F>(
        &mut self,
        mut vd: usize,
        mut vn: usize,
        mut vm: usize,
        op: F,
    ) -> Result<()>
    where
        F: Fn(f32, f32, f32) -> (u32, u32),
    {
        let plan = self.vfp_vector_plan(false, vd, vm)?;
        let mut lhs = self.sregs[vn];
        let mut rhs = self.sregs[vm];
        for lane in 0..plan.count {
            let dst = f32::from_bits(self.sregs[vd]);
            let (result, exception_flags) = op(dst, f32::from_bits(lhs), f32::from_bits(rhs));
            self.sregs[vd] = result;
            self.fpscr |= exception_flags;
            if lane + 1 == plan.count {
                break;
            }
            vd = Self::vfp_advance_sreg(vd, plan.delta_d);
            vn = Self::vfp_advance_sreg(vn, plan.delta_d);
            lhs = self.sregs[vn];
            if plan.delta_m != 0 {
                vm = Self::vfp_advance_sreg(vm, plan.delta_m);
                rhs = self.sregs[vm];
            }
        }
        Ok(())
    }

    fn exec_vfp_3op_f64<F>(
        &mut self,
        mut vd: usize,
        mut vn: usize,
        mut vm: usize,
        op: F,
    ) -> Result<()>
    where
        F: Fn(f64, f64, f64) -> (u64, u32),
    {
        self.check_dreg(vd)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;
        let plan = self.vfp_vector_plan(true, vd, vm)?;
        let mut lhs = self.dreg_bits(vn);
        let mut rhs = self.dreg_bits(vm);
        for lane in 0..plan.count {
            let dst = f64::from_bits(self.dreg_bits(vd));
            let (result, exception_flags) = op(dst, f64::from_bits(lhs), f64::from_bits(rhs));
            self.set_dreg_bits(vd, result);
            self.fpscr |= exception_flags;
            if lane + 1 == plan.count {
                break;
            }
            vd = Self::vfp_advance_dreg(vd, plan.delta_d);
            vn = Self::vfp_advance_dreg(vn, plan.delta_d);
            lhs = self.dreg_bits(vn);
            if plan.delta_m != 0 {
                vm = Self::vfp_advance_dreg(vm, plan.delta_m);
                rhs = self.dreg_bits(vm);
            }
        }
        Ok(())
    }

    fn vfp_vector_plan(&self, double: bool, vd: usize, vm: usize) -> Result<VfpVectorPlan> {
        let len = ((self.fpscr >> 16) & 0x7) as usize + 1;
        let raw_stride = (self.fpscr >> 20) & 0x3;
        let stride = match raw_stride {
            0b00 => 1,
            0b11 => 2,
            _ => return Err(Trap::Unpredictable("invalid VFP vector stride")),
        };

        if len == 1 {
            if stride != 1 {
                return Err(Trap::Unpredictable("invalid VFP vector stride"));
            }
            return Ok(VfpVectorPlan {
                count: 1,
                delta_d: 0,
                delta_m: 0,
            });
        }

        let dst_scalar = if double {
            Self::vfp_dreg_is_scalar(vd)
        } else {
            Self::vfp_sreg_is_scalar(vd)
        };
        if dst_scalar {
            return Ok(VfpVectorPlan {
                count: 1,
                delta_d: 0,
                delta_m: 0,
            });
        }

        let bank_size = if double { 4 } else { 8 };
        if stride * len > bank_size {
            return Err(Trap::Unpredictable("invalid VFP vector length/stride"));
        }

        let m_scalar = if double {
            Self::vfp_dreg_is_scalar(vm)
        } else {
            Self::vfp_sreg_is_scalar(vm)
        };
        Ok(VfpVectorPlan {
            count: len,
            delta_d: stride,
            delta_m: if m_scalar { 0 } else { stride },
        })
    }

    fn vfp_sreg_is_scalar(reg: usize) -> bool {
        (reg & 0x18) == 0
    }

    fn vfp_dreg_is_scalar(reg: usize) -> bool {
        (reg & 0x0c) == 0
    }

    fn vfp_advance_sreg(reg: usize, delta: usize) -> usize {
        ((reg + delta) & 0x7) | (reg & !0x7)
    }

    fn vfp_advance_dreg(reg: usize, delta: usize) -> usize {
        ((reg + delta) & 0x3) | (reg & !0x3)
    }

    fn set_fpscr_compare_f32_bits(&mut self, lhs: u32, rhs: u32, signal_all_nans: bool) {
        let lhs_value = f32::from_bits(lhs);
        let rhs_value = f32::from_bits(rhs);
        let unordered = lhs_value.partial_cmp(&rhs_value).is_none();
        self.set_fpscr_compare_order(lhs_value.partial_cmp(&rhs_value));
        if unordered
            && (signal_all_nans || f32_is_signaling_nan_bits(lhs) || f32_is_signaling_nan_bits(rhs))
        {
            self.fpscr |= FPSCR_IOC;
        }
    }

    fn set_fpscr_compare_f64_bits(&mut self, lhs: u64, rhs: u64, signal_all_nans: bool) {
        let lhs_value = f64::from_bits(lhs);
        let rhs_value = f64::from_bits(rhs);
        let unordered = lhs_value.partial_cmp(&rhs_value).is_none();
        self.set_fpscr_compare_order(lhs_value.partial_cmp(&rhs_value));
        if unordered
            && (signal_all_nans || f64_is_signaling_nan_bits(lhs) || f64_is_signaling_nan_bits(rhs))
        {
            self.fpscr |= FPSCR_IOC;
        }
    }

    fn set_fpscr_compare_order(&mut self, order: Option<std::cmp::Ordering>) {
        self.fpscr &= !0xf000_0000;
        let bits = match order {
            Some(std::cmp::Ordering::Less) => 0x8000_0000,
            Some(std::cmp::Ordering::Equal) => 0x6000_0000,
            Some(std::cmp::Ordering::Greater) => 0x2000_0000,
            None => 0x3000_0000,
        };
        self.fpscr |= bits;
    }

    fn arm_read_reg(&self, idx: usize, pc: u32) -> u32 {
        if idx == 15 {
            pc.wrapping_add(8)
        } else {
            self.regs[idx]
        }
    }

    fn thumb_read_reg(&self, idx: usize, pc: u32) -> u32 {
        if idx == 15 {
            pc.wrapping_add(4) & !3
        } else {
            self.regs[idx]
        }
    }

    fn write_reg_arm(&mut self, idx: usize, value: u32) {
        if idx == 15 {
            self.branch_exchange(value);
        } else {
            self.regs[idx] = value;
        }
    }

    fn write_reg_thumb(&mut self, idx: usize, value: u32) {
        if idx == 15 {
            self.regs[15] = value & !1;
        } else {
            self.regs[idx] = value;
        }
    }
}

pub trait Memory {
    fn load8(&mut self, addr: u32) -> Result<u8>;
    fn store8(&mut self, addr: u32, value: u8) -> Result<()>;

    fn load16(&mut self, addr: u32) -> Result<u16> {
        let b0 = self.load8(addr)?;
        let b1 = self.load8(addr.wrapping_add(1))?;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn store16(&mut self, addr: u32, value: u16) -> Result<()> {
        for (idx, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.store8(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }

    fn load32(&mut self, addr: u32) -> Result<u32> {
        let b0 = self.load8(addr)?;
        let b1 = self.load8(addr.wrapping_add(1))?;
        let b2 = self.load8(addr.wrapping_add(2))?;
        let b3 = self.load8(addr.wrapping_add(3))?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    fn store32(&mut self, addr: u32, value: u32) -> Result<()> {
        for (idx, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.store8(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct VecMemory {
    base: u32,
    data: Vec<u8>,
}

impl VecMemory {
    pub fn new(base: u32, size: usize) -> Self {
        Self {
            base,
            data: vec![0; size],
        }
    }

    pub fn base(&self) -> u32 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn load_bytes(&mut self, addr: u32, bytes: &[u8]) -> Result<()> {
        for (idx, byte) in bytes.iter().copied().enumerate() {
            self.store8(addr.wrapping_add(idx as u32), byte)?;
        }
        Ok(())
    }

    pub fn load_arm_words(&mut self, addr: u32, words: &[u32]) -> Result<()> {
        for (idx, word) in words.iter().copied().enumerate() {
            self.store32(addr.wrapping_add((idx * 4) as u32), word)?;
        }
        Ok(())
    }

    pub fn load_thumb_halfwords(&mut self, addr: u32, halfwords: &[u16]) -> Result<()> {
        for (idx, halfword) in halfwords.iter().copied().enumerate() {
            self.store16(addr.wrapping_add((idx * 2) as u32), halfword)?;
        }
        Ok(())
    }

    fn offset(&self, addr: u32) -> Result<usize> {
        let rel = addr.checked_sub(self.base).ok_or_else(|| {
            Trap::Memory(format!(
                "address {addr:#010x} below base {:#010x}",
                self.base
            ))
        })?;
        let rel = rel as usize;
        if rel >= self.data.len() {
            return Err(Trap::Memory(format!(
                "address {addr:#010x} outside memory range {:#010x}..{:#010x}",
                self.base,
                self.base.wrapping_add(self.data.len() as u32)
            )));
        }
        Ok(rel)
    }
}

impl Memory for VecMemory {
    fn load8(&mut self, addr: u32) -> Result<u8> {
        let off = self.offset(addr)?;
        Ok(self.data[off])
    }

    fn store8(&mut self, addr: u32, value: u8) -> Result<()> {
        let off = self.offset(addr)?;
        self.data[off] = value;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Shifted {
    value: u32,
    carry: bool,
}

#[derive(Debug, Clone, Copy)]
enum Shift {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VfpBinaryOp {
    MulAdd,
    MulSub,
    NegMulSub,
    NegMulAdd,
    Mul,
    NegMul,
    Add,
    Sub,
    Div,
}

impl VfpBinaryOp {
    fn eval_f32_with_flags(self, dst: f32, lhs: f32, rhs: f32) -> (u32, u32) {
        match self {
            Self::MulAdd => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f32(lhs, rhs, product);
                let result = dst + product;
                flags |= vfp_add_exception_flags_f32(dst, product, result);
                (result.to_bits(), flags)
            }
            Self::MulSub => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f32(lhs, rhs, product);
                let result = dst - product;
                flags |= vfp_sub_exception_flags_f32(dst, product, result);
                (result.to_bits(), flags)
            }
            Self::NegMulSub => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f32(lhs, rhs, product);
                let result = -dst + product;
                flags |= vfp_add_exception_flags_f32(-dst, product, result);
                (result.to_bits(), flags)
            }
            Self::NegMulAdd => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f32(lhs, rhs, product);
                let result = -dst - product;
                flags |= vfp_sub_exception_flags_f32(-dst, product, result);
                (result.to_bits(), flags)
            }
            Self::Mul => {
                let result = lhs * rhs;
                (
                    result.to_bits(),
                    vfp_mul_exception_flags_f32(lhs, rhs, result),
                )
            }
            Self::NegMul => {
                let product = lhs * rhs;
                (
                    (-product).to_bits(),
                    vfp_mul_exception_flags_f32(lhs, rhs, product),
                )
            }
            Self::Add => {
                let result = lhs + rhs;
                (
                    result.to_bits(),
                    vfp_add_exception_flags_f32(lhs, rhs, result),
                )
            }
            Self::Sub => {
                let result = lhs - rhs;
                (
                    result.to_bits(),
                    vfp_sub_exception_flags_f32(lhs, rhs, result),
                )
            }
            Self::Div => {
                let result = lhs / rhs;
                (
                    result.to_bits(),
                    vfp_div_exception_flags_f32(lhs, rhs, result),
                )
            }
        }
    }

    fn eval_f64_with_flags(self, dst: f64, lhs: f64, rhs: f64) -> (u64, u32) {
        match self {
            Self::MulAdd => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f64(lhs, rhs, product);
                let result = dst + product;
                flags |= vfp_add_exception_flags_f64(dst, product, result);
                (result.to_bits(), flags)
            }
            Self::MulSub => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f64(lhs, rhs, product);
                let result = dst - product;
                flags |= vfp_sub_exception_flags_f64(dst, product, result);
                (result.to_bits(), flags)
            }
            Self::NegMulSub => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f64(lhs, rhs, product);
                let result = -dst + product;
                flags |= vfp_add_exception_flags_f64(-dst, product, result);
                (result.to_bits(), flags)
            }
            Self::NegMulAdd => {
                let product = lhs * rhs;
                let mut flags = vfp_mul_exception_flags_f64(lhs, rhs, product);
                let result = -dst - product;
                flags |= vfp_sub_exception_flags_f64(-dst, product, result);
                (result.to_bits(), flags)
            }
            Self::Mul => {
                let result = lhs * rhs;
                (
                    result.to_bits(),
                    vfp_mul_exception_flags_f64(lhs, rhs, result),
                )
            }
            Self::NegMul => {
                let product = lhs * rhs;
                (
                    (-product).to_bits(),
                    vfp_mul_exception_flags_f64(lhs, rhs, product),
                )
            }
            Self::Add => {
                let result = lhs + rhs;
                (
                    result.to_bits(),
                    vfp_add_exception_flags_f64(lhs, rhs, result),
                )
            }
            Self::Sub => {
                let result = lhs - rhs;
                (
                    result.to_bits(),
                    vfp_sub_exception_flags_f64(lhs, rhs, result),
                )
            }
            Self::Div => {
                let result = lhs / rhs;
                (
                    result.to_bits(),
                    vfp_div_exception_flags_f64(lhs, rhs, result),
                )
            }
        }
    }
}

fn vfp_sqrt_f32(value: u32) -> (u32, u32) {
    let value = f32::from_bits(value);
    let result = value.sqrt();
    let flags = vfp_sqrt_exception_flags_f32(value, result);
    (result.to_bits(), flags)
}

fn vfp_sqrt_f64(value: u64) -> (u64, u32) {
    let value = f64::from_bits(value);
    let flags = vfp_sqrt_exception_flags_f64(value);
    (value.sqrt().to_bits(), flags)
}

fn vfp_add_exception_flags_f32(lhs: f32, rhs: f32, result: f32) -> u32 {
    if f32_is_signaling_nan_bits(lhs.to_bits())
        || f32_is_signaling_nan_bits(rhs.to_bits())
        || (lhs.is_infinite()
            && rhs.is_infinite()
            && lhs.is_sign_positive() != rhs.is_sign_positive())
    {
        return FPSCR_IOC;
    }

    if lhs.is_finite() && rhs.is_finite() {
        vfp_rounded_f32_exception_flags(result, f64::from(lhs) + f64::from(rhs))
    } else {
        0
    }
}

fn vfp_sub_exception_flags_f32(lhs: f32, rhs: f32, result: f32) -> u32 {
    if f32_is_signaling_nan_bits(lhs.to_bits())
        || f32_is_signaling_nan_bits(rhs.to_bits())
        || (lhs.is_infinite()
            && rhs.is_infinite()
            && lhs.is_sign_positive() == rhs.is_sign_positive())
    {
        return FPSCR_IOC;
    }

    if lhs.is_finite() && rhs.is_finite() {
        vfp_rounded_f32_exception_flags(result, f64::from(lhs) - f64::from(rhs))
    } else {
        0
    }
}

fn vfp_mul_exception_flags_f32(lhs: f32, rhs: f32, result: f32) -> u32 {
    if f32_is_signaling_nan_bits(lhs.to_bits())
        || f32_is_signaling_nan_bits(rhs.to_bits())
        || ((lhs == 0.0 && rhs.is_infinite()) || (rhs == 0.0 && lhs.is_infinite()))
    {
        return FPSCR_IOC;
    }

    if lhs.is_finite() && rhs.is_finite() {
        vfp_rounded_f32_exception_flags(result, f64::from(lhs) * f64::from(rhs))
    } else {
        0
    }
}

fn vfp_div_exception_flags_f32(lhs: f32, rhs: f32, result: f32) -> u32 {
    if f32_is_signaling_nan_bits(lhs.to_bits())
        || f32_is_signaling_nan_bits(rhs.to_bits())
        || ((lhs == 0.0 && rhs == 0.0) || (lhs.is_infinite() && rhs.is_infinite()))
    {
        return FPSCR_IOC;
    }

    if rhs == 0.0 {
        if lhs.is_finite() { FPSCR_DZC } else { 0 }
    } else if lhs.is_finite() && rhs.is_finite() {
        vfp_rounded_f32_exception_flags(result, f64::from(lhs) / f64::from(rhs))
    } else {
        0
    }
}

fn vfp_sqrt_exception_flags_f32(value: f32, result: f32) -> u32 {
    if f32_is_signaling_nan_bits(value.to_bits()) || value < 0.0 {
        return FPSCR_IOC;
    }

    if value.is_finite() {
        vfp_rounded_f32_exception_flags(result, f64::from(value).sqrt())
    } else {
        0
    }
}

fn vfp_rounded_f32_exception_flags(result: f32, exact: f64) -> u32 {
    if !exact.is_finite() {
        return 0;
    }

    if result.is_infinite() {
        return FPSCR_OFC | FPSCR_IXC;
    }

    if f64::from(result) == exact {
        return 0;
    }

    let mut flags = FPSCR_IXC;
    if result == 0.0 || result.is_subnormal() {
        flags |= FPSCR_UFC;
    }
    flags
}

fn vfp_div_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if f64_is_signaling_nan_bits(lhs.to_bits())
        || f64_is_signaling_nan_bits(rhs.to_bits())
        || ((lhs == 0.0 && rhs == 0.0) || (lhs.is_infinite() && rhs.is_infinite()))
    {
        return FPSCR_IOC;
    }

    if rhs == 0.0 {
        if lhs.is_finite() { FPSCR_DZC } else { 0 }
    } else if lhs.is_finite() && rhs.is_finite() {
        vfp_overflow_exception_flags_f64(result)
            | vfp_div_rounding_exception_flags_f64(lhs, rhs, result)
    } else {
        0
    }
}

fn vfp_add_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if f64_is_signaling_nan_bits(lhs.to_bits())
        || f64_is_signaling_nan_bits(rhs.to_bits())
        || (lhs.is_infinite()
            && rhs.is_infinite()
            && lhs.is_sign_positive() != rhs.is_sign_positive())
    {
        FPSCR_IOC
    } else if lhs.is_finite() && rhs.is_finite() {
        vfp_overflow_exception_flags_f64(result)
            | vfp_absorbed_add_inexact_exception_flags_f64(lhs, rhs, result)
    } else {
        0
    }
}

fn vfp_sub_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if f64_is_signaling_nan_bits(lhs.to_bits())
        || f64_is_signaling_nan_bits(rhs.to_bits())
        || (lhs.is_infinite()
            && rhs.is_infinite()
            && lhs.is_sign_positive() == rhs.is_sign_positive())
    {
        FPSCR_IOC
    } else if lhs.is_finite() && rhs.is_finite() {
        vfp_overflow_exception_flags_f64(result)
            | vfp_absorbed_sub_inexact_exception_flags_f64(lhs, rhs, result)
    } else {
        0
    }
}

fn vfp_mul_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if f64_is_signaling_nan_bits(lhs.to_bits())
        || f64_is_signaling_nan_bits(rhs.to_bits())
        || ((lhs == 0.0 && rhs.is_infinite()) || (rhs == 0.0 && lhs.is_infinite()))
    {
        FPSCR_IOC
    } else if lhs.is_finite() && rhs.is_finite() {
        vfp_overflow_exception_flags_f64(result)
            | vfp_mul_rounding_exception_flags_f64(lhs, rhs, result)
    } else {
        0
    }
}

fn vfp_overflow_exception_flags_f64(result: f64) -> u32 {
    if result.is_infinite() {
        FPSCR_OFC | FPSCR_IXC
    } else {
        0
    }
}

fn vfp_absorbed_add_inexact_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if result.is_finite() && ((result == lhs && rhs != 0.0) || (result == rhs && lhs != 0.0)) {
        FPSCR_IXC
    } else {
        0
    }
}

fn vfp_absorbed_sub_inexact_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if result.is_finite() && ((result == lhs && rhs != 0.0) || (result == -rhs && lhs != 0.0)) {
        FPSCR_IXC
    } else {
        0
    }
}

fn vfp_mul_rounding_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if result.is_infinite() || f64_mul_result_exact(lhs, rhs, result) {
        return 0;
    }

    let mut flags = FPSCR_IXC;
    if result == 0.0 || result.is_subnormal() {
        flags |= FPSCR_UFC;
    }
    flags
}

fn vfp_div_rounding_exception_flags_f64(lhs: f64, rhs: f64, result: f64) -> u32 {
    if result.is_infinite() || f64_div_result_exact(lhs, rhs, result) {
        return 0;
    }

    let mut flags = FPSCR_IXC;
    if result == 0.0 || result.is_subnormal() {
        flags |= FPSCR_UFC;
    }
    flags
}

fn f64_mul_result_exact(lhs: f64, rhs: f64, result: f64) -> bool {
    let lhs = f64_finite_magnitude(lhs);
    let rhs = f64_finite_magnitude(rhs);
    let result = f64_finite_magnitude(result);
    scaled_magnitudes_equal(
        lhs.significand * rhs.significand,
        lhs.exponent + rhs.exponent,
        result.significand,
        result.exponent,
    )
}

fn f64_div_result_exact(lhs: f64, rhs: f64, result: f64) -> bool {
    let lhs = f64_finite_magnitude(lhs);
    let rhs = f64_finite_magnitude(rhs);
    let result = f64_finite_magnitude(result);
    if lhs.significand == 0 || rhs.significand == 0 {
        return lhs.significand == 0 && result.significand == 0;
    }

    let divisor = gcd_u128(lhs.significand, rhs.significand);
    let numerator = lhs.significand / divisor;
    let denominator = rhs.significand / divisor;
    let twos = denominator.trailing_zeros();
    if denominator >> twos != 1 {
        return false;
    }

    scaled_magnitudes_equal(
        numerator,
        lhs.exponent - rhs.exponent - twos as i32,
        result.significand,
        result.exponent,
    )
}

#[derive(Debug, Clone, Copy)]
struct F64Magnitude {
    significand: u128,
    exponent: i32,
}

fn f64_finite_magnitude(value: f64) -> F64Magnitude {
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0 {
        F64Magnitude {
            significand: u128::from(fraction),
            exponent: -1074,
        }
    } else {
        F64Magnitude {
            significand: u128::from((1u64 << 52) | fraction),
            exponent: exponent - 1023 - 52,
        }
    }
}

fn scaled_magnitudes_equal(lhs: u128, lhs_exp: i32, rhs: u128, rhs_exp: i32) -> bool {
    if lhs == 0 || rhs == 0 {
        return lhs == rhs;
    }

    if lhs_exp >= rhs_exp {
        let shift = (lhs_exp - rhs_exp) as u32;
        shift < 128 && lhs.checked_shl(shift) == Some(rhs)
    } else {
        let shift = (rhs_exp - lhs_exp) as u32;
        shift < 128 && rhs.checked_shl(shift) == Some(lhs)
    }
}

fn gcd_u128(mut lhs: u128, mut rhs: u128) -> u128 {
    while rhs != 0 {
        let remainder = lhs % rhs;
        lhs = rhs;
        rhs = remainder;
    }
    lhs
}

fn vfp_sqrt_exception_flags_f64(value: f64) -> u32 {
    if f64_is_signaling_nan_bits(value.to_bits()) || value < 0.0 {
        FPSCR_IOC
    } else {
        0
    }
}

fn vfp_trunc_to_i32_bits(value: f64) -> (u32, u32) {
    vfp_i32_result_bits(value, value.trunc())
}

fn vfp_round_to_i32_bits(value: f64, fpscr: u32) -> (u32, u32) {
    vfp_i32_result_bits(value, vfp_round_by_fpscr(value, fpscr))
}

fn vfp_trunc_to_u32_bits(value: f64) -> (u32, u32) {
    vfp_u32_result_bits(value, value.trunc())
}

fn vfp_round_to_u32_bits(value: f64, fpscr: u32) -> (u32, u32) {
    vfp_u32_result_bits(value, vfp_round_by_fpscr(value, fpscr))
}

fn vfp_i32_result_bits(value: f64, rounded: f64) -> (u32, u32) {
    let (result, exception_flags) = if rounded.is_nan() {
        (0, FPSCR_IOC)
    } else if rounded > f64::from(i32::MAX) {
        (i32::MAX as u32, FPSCR_IOC)
    } else if rounded < f64::from(i32::MIN) {
        (i32::MIN as u32, FPSCR_IOC)
    } else {
        (rounded as i32 as u32, 0)
    };
    (
        result,
        exception_flags | vfp_conversion_inexact_flag(value, rounded, exception_flags),
    )
}

fn vfp_u32_result_bits(value: f64, rounded: f64) -> (u32, u32) {
    let (result, exception_flags) = if rounded.is_nan() {
        (0, FPSCR_IOC)
    } else if rounded < 0.0 {
        (0, FPSCR_IOC)
    } else if rounded > f64::from(u32::MAX) {
        (u32::MAX, FPSCR_IOC)
    } else {
        (rounded as u32, 0)
    };
    (
        result,
        exception_flags | vfp_conversion_inexact_flag(value, rounded, exception_flags),
    )
}

fn vfp_conversion_inexact_flag(value: f64, rounded: f64, exception_flags: u32) -> u32 {
    if exception_flags & FPSCR_IOC == 0 && value.is_finite() && rounded != value {
        FPSCR_IXC
    } else {
        0
    }
}

fn f32_is_signaling_nan_bits(value: u32) -> bool {
    (value & 0x7f80_0000) == 0x7f80_0000 && (value & 0x007f_ffff) != 0 && (value & 0x0040_0000) == 0
}

fn f64_is_signaling_nan_bits(value: u64) -> bool {
    (value & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000
        && (value & 0x000f_ffff_ffff_ffff) != 0
        && (value & 0x0008_0000_0000_0000) == 0
}

fn vfp_int_to_f32_inexact_flag(value: f64, result: f32) -> u32 {
    if f64::from(result) != value {
        FPSCR_IXC
    } else {
        0
    }
}

fn vfp_f64_to_f32_bits(value: u64) -> (u32, u32) {
    let input = f64::from_bits(value);
    let result = input as f32;
    let mut exception_flags = if f64_is_signaling_nan_bits(value) {
        FPSCR_IOC
    } else {
        0
    };

    if input.is_finite() {
        if result.is_infinite() {
            exception_flags |= FPSCR_OFC | FPSCR_IXC;
        } else if f64::from(result) != input {
            exception_flags |= FPSCR_IXC;
            if result == 0.0 || result.is_subnormal() {
                exception_flags |= FPSCR_UFC;
            }
        }
    }

    (result.to_bits(), exception_flags)
}

fn vfp_round_by_fpscr(value: f64, fpscr: u32) -> f64 {
    if !value.is_finite() {
        return value;
    }

    match (fpscr >> 22) & 0x3 {
        0 => round_ties_to_even(value),
        1 => value.ceil(),
        2 => value.floor(),
        3 => value.trunc(),
        _ => unreachable!(),
    }
}

fn round_ties_to_even(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 {
        floor + 1.0
    } else if (floor / 2.0).fract() == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

fn decode_arm_shift(value: u32) -> Shift {
    match value {
        0 => Shift::Lsl,
        1 => Shift::Lsr,
        2 => Shift::Asr,
        3 => Shift::Ror,
        _ => unreachable!(),
    }
}

fn shift_operand(
    value: u32,
    shift: Shift,
    amount: u32,
    carry_in: bool,
    immediate: bool,
) -> Shifted {
    match shift {
        Shift::Lsl => {
            if amount == 0 {
                Shifted {
                    value,
                    carry: carry_in,
                }
            } else if amount < 32 {
                Shifted {
                    value: value << amount,
                    carry: value & (1 << (32 - amount)) != 0,
                }
            } else if amount == 32 {
                Shifted {
                    value: 0,
                    carry: value & 1 != 0,
                }
            } else {
                Shifted {
                    value: 0,
                    carry: false,
                }
            }
        }
        Shift::Lsr => {
            let amount = if immediate && amount == 0 { 32 } else { amount };
            if amount == 0 {
                Shifted {
                    value,
                    carry: carry_in,
                }
            } else if amount < 32 {
                Shifted {
                    value: value >> amount,
                    carry: value & (1 << (amount - 1)) != 0,
                }
            } else if amount == 32 {
                Shifted {
                    value: 0,
                    carry: value & 0x8000_0000 != 0,
                }
            } else {
                Shifted {
                    value: 0,
                    carry: false,
                }
            }
        }
        Shift::Asr => {
            let amount = if immediate && amount == 0 { 32 } else { amount };
            if amount == 0 {
                Shifted {
                    value,
                    carry: carry_in,
                }
            } else if amount < 32 {
                Shifted {
                    value: ((value as i32) >> amount) as u32,
                    carry: value & (1 << (amount - 1)) != 0,
                }
            } else {
                let carry = value & 0x8000_0000 != 0;
                Shifted {
                    value: if carry { u32::MAX } else { 0 },
                    carry,
                }
            }
        }
        Shift::Ror => {
            if immediate && amount == 0 {
                Shifted {
                    value: (u32::from(carry_in) << 31) | (value >> 1),
                    carry: value & 1 != 0,
                }
            } else if amount == 0 {
                Shifted {
                    value,
                    carry: carry_in,
                }
            } else {
                let rot = amount % 32;
                let result = value.rotate_right(rot);
                Shifted {
                    value: result,
                    carry: result & 0x8000_0000 != 0,
                }
            }
        }
    }
}

fn arm_expand_imm(imm12: u32) -> (u32, Option<bool>) {
    let imm8 = imm12 & 0xff;
    let rot = ((imm12 >> 8) & 0xf) * 2;
    if rot == 0 {
        (imm8, None)
    } else {
        let value = imm8.rotate_right(rot);
        (value, Some(value & 0x8000_0000 != 0))
    }
}

fn condition_passed(cond: u32, cpsr: Cpsr) -> bool {
    match cond {
        0x0 => cpsr.z,
        0x1 => !cpsr.z,
        0x2 => cpsr.c,
        0x3 => !cpsr.c,
        0x4 => cpsr.n,
        0x5 => !cpsr.n,
        0x6 => cpsr.v,
        0x7 => !cpsr.v,
        0x8 => cpsr.c && !cpsr.z,
        0x9 => !cpsr.c || cpsr.z,
        0xa => cpsr.n == cpsr.v,
        0xb => cpsr.n != cpsr.v,
        0xc => !cpsr.z && cpsr.n == cpsr.v,
        0xd => cpsr.z || cpsr.n != cpsr.v,
        0xe => true,
        _ => false,
    }
}

fn add_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    add_with_carry(a, b, false)
}

fn add_with_carry(a: u32, b: u32, carry_in: bool) -> (u32, bool, bool) {
    let carry_value = u64::from(carry_in);
    let unsigned = u64::from(a) + u64::from(b) + carry_value;
    let signed = i64::from(a as i32) + i64::from(b as i32) + i64::from(carry_in);
    let result = unsigned as u32;
    let carry = unsigned > u64::from(u32::MAX);
    let overflow = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (result, carry, overflow)
}

fn sub_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    add_with_carry(a, !b, true)
}

fn saturating_add_i32(a: i32, b: i32, cpsr: &mut Cpsr) -> i32 {
    let wide = i64::from(a) + i64::from(b);
    if wide > i64::from(i32::MAX) {
        cpsr.q = true;
        i32::MAX
    } else if wide < i64::from(i32::MIN) {
        cpsr.q = true;
        i32::MIN
    } else {
        wide as i32
    }
}

fn saturating_sub_i32(a: i32, b: i32, cpsr: &mut Cpsr) -> i32 {
    let wide = i64::from(a) - i64::from(b);
    if wide > i64::from(i32::MAX) {
        cpsr.q = true;
        i32::MAX
    } else if wide < i64::from(i32::MIN) {
        cpsr.q = true;
        i32::MIN
    } else {
        wide as i32
    }
}

fn saturate_i32(value: i32, bits: u32, cpsr: &mut Cpsr) -> i32 {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let wide = i64::from(value);
    if wide > max {
        cpsr.q = true;
        max as i32
    } else if wide < min {
        cpsr.q = true;
        min as i32
    } else {
        value
    }
}

fn saturate_u32(value: i32, bits: u32, cpsr: &mut Cpsr) -> u32 {
    let max = if bits == 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    if value < 0 {
        cpsr.q = true;
        0
    } else if value as u32 > max {
        cpsr.q = true;
        max
    } else {
        value as u32
    }
}

fn extend_add_16(base: u32, rotated: u32, signed: bool) -> u32 {
    let low = if signed {
        sign_extend(rotated & 0xff, 8) as u32
    } else {
        rotated & 0xff
    };
    let high = if signed {
        sign_extend((rotated >> 16) & 0xff, 8) as u32
    } else {
        (rotated >> 16) & 0xff
    };
    let low = base.wrapping_add(low) & 0xffff;
    let high = ((base >> 16).wrapping_add(high) & 0xffff) << 16;
    high | low
}

fn parallel_lane16(
    a: u16,
    b: u16,
    add: bool,
    unsigned: bool,
    saturating: bool,
    halving: bool,
) -> u16 {
    if unsigned {
        let av = u32::from(a);
        let bv = u32::from(b);
        let value = if add {
            av as i32 + bv as i32
        } else {
            av as i32 - bv as i32
        };
        if saturating {
            value.clamp(0, 0xffff) as u16
        } else if halving {
            if add {
                ((av + bv) >> 1) as u16
            } else {
                ((av.wrapping_sub(bv)) >> 1) as u16
            }
        } else {
            value as u16
        }
    } else {
        let av = a as i16 as i32;
        let bv = b as i16 as i32;
        let value = if add { av + bv } else { av - bv };
        if saturating {
            value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16 as u16
        } else if halving {
            (value >> 1) as i16 as u16
        } else {
            value as i16 as u16
        }
    }
}

fn parallel_lane8(a: u8, b: u8, add: bool, unsigned: bool, saturating: bool, halving: bool) -> u8 {
    if unsigned {
        let av = u16::from(a);
        let bv = u16::from(b);
        let value = if add {
            av as i16 + bv as i16
        } else {
            av as i16 - bv as i16
        };
        if saturating {
            value.clamp(0, 0xff) as u8
        } else if halving {
            if add {
                ((av + bv) >> 1) as u8
            } else {
                ((av.wrapping_sub(bv)) >> 1) as u8
            }
        } else {
            value as u8
        }
    } else {
        let av = a as i8 as i16;
        let bv = b as i8 as i16;
        let value = if add { av + bv } else { av - bv };
        if saturating {
            value.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8 as u8
        } else if halving {
            (value >> 1) as i8 as u8
        } else {
            value as i8 as u8
        }
    }
}

fn parallel_ge16(a: u16, b: u16, add: bool, unsigned: bool) -> bool {
    if unsigned {
        if add {
            u32::from(a) + u32::from(b) >= 0x1_0000
        } else {
            a >= b
        }
    } else {
        let value = if add {
            a as i16 as i32 + b as i16 as i32
        } else {
            a as i16 as i32 - b as i16 as i32
        };
        value >= 0
    }
}

fn parallel_ge8(a: u8, b: u8, add: bool, unsigned: bool) -> bool {
    if unsigned {
        if add {
            u16::from(a) + u16::from(b) >= 0x100
        } else {
            a >= b
        }
    } else {
        let value = if add {
            a as i8 as i16 + b as i8 as i16
        } else {
            a as i8 as i16 - b as i8 as i16
        };
        value >= 0
    }
}

fn select_i16(value: u32, top: bool) -> i16 {
    if top {
        (value >> 16) as u16 as i16
    } else {
        value as u16 as i16
    }
}

fn signed_dual_products(rn: u32, rm: u32, exchange_rm: bool) -> (i32, i32) {
    let rn_lo = rn as u16 as i16 as i32;
    let rn_hi = (rn >> 16) as u16 as i16 as i32;
    let mut rm_lo = rm as u16 as i16 as i32;
    let mut rm_hi = (rm >> 16) as u16 as i16 as i32;
    if exchange_rm {
        std::mem::swap(&mut rm_lo, &mut rm_hi);
    }
    (rn_lo.wrapping_mul(rm_lo), rn_hi.wrapping_mul(rm_hi))
}

fn signed_high_word(value: i64, round: bool) -> u32 {
    let value = if round {
        value.wrapping_add(0x8000_0000)
    } else {
        value
    };
    (value >> 32) as u32
}

fn vfp_single_d(instr: u32) -> usize {
    ((((instr >> 12) & 0xf) << 1) | ((instr >> 22) & 1)) as usize
}

fn vfp_single_n(instr: u32) -> usize {
    ((((instr >> 16) & 0xf) << 1) | ((instr >> 7) & 1)) as usize
}

fn vfp_single_m(instr: u32) -> usize {
    (((instr & 0xf) << 1) | ((instr >> 5) & 1)) as usize
}

fn vfp_double_d(instr: u32) -> usize {
    (((instr >> 12) & 0xf) | (((instr >> 22) & 1) << 4)) as usize
}

fn vfp_double_n(instr: u32) -> usize {
    (((instr >> 16) & 0xf) | (((instr >> 7) & 1) << 4)) as usize
}

fn vfp_double_m(instr: u32) -> usize {
    ((instr & 0xf) | (((instr >> 5) & 1) << 4)) as usize
}

fn is_unsupported_a32_newer_than_armv6(instr: u32) -> bool {
    const ARMV6T2_ARMV7_PATTERNS: &[(u32, u32)] = &[
        (0x0fe0_007f, 0x07c0_001f), // BFC
        (0x0fe0_0070, 0x07c0_0010), // BFI
        (0x0ff0_0000, 0x0340_0000), // MOVT
        (0x0ff0_0000, 0x0300_0000), // MOVW
        (0x0fe0_0070, 0x07a0_0050), // SBFX
        (0x0fe0_0070, 0x07e0_0050), // UBFX
        (0x0fff_0ff0, 0x06ff_0f30), // RBIT
        (0x0ff0_f0f0, 0x0710_f010), // SDIV
        (0x0ff0_f0f0, 0x0730_f010), // UDIV
    ];
    ARMV6T2_ARMV7_PATTERNS
        .iter()
        .any(|&(mask, value)| instr & mask == value)
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_bit(value: bool, shift: u32) -> u32 {
        u32::from(value) << shift
    }

    fn arm_single_transfer(
        i: bool,
        p: bool,
        u: bool,
        b: bool,
        w: bool,
        l: bool,
        rn: usize,
        rd: usize,
        offset: u32,
    ) -> u32 {
        0xe000_0000
            | 0x0400_0000
            | enc_bit(i, 25)
            | enc_bit(p, 24)
            | enc_bit(u, 23)
            | enc_bit(b, 22)
            | enc_bit(w, 21)
            | enc_bit(l, 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | offset
    }

    fn arm_halfword_transfer(
        p: bool,
        u: bool,
        i: bool,
        w: bool,
        l: bool,
        rn: usize,
        rd: usize,
        op: u32,
        offset: u32,
    ) -> u32 {
        let offset_bits = if i {
            ((offset & 0xf0) << 4) | (offset & 0xf)
        } else {
            offset & 0xf
        };
        0xe000_0000
            | enc_bit(p, 24)
            | enc_bit(u, 23)
            | enc_bit(i, 22)
            | enc_bit(w, 21)
            | enc_bit(l, 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | ((op & 0b11) << 5)
            | 0x90
            | offset_bits
    }

    fn arm_block_transfer(
        p: bool,
        u: bool,
        s: bool,
        w: bool,
        l: bool,
        rn: usize,
        reglist: u32,
    ) -> u32 {
        0xe000_0000
            | 0x0800_0000
            | enc_bit(p, 24)
            | enc_bit(u, 23)
            | enc_bit(s, 22)
            | enc_bit(w, 21)
            | enc_bit(l, 20)
            | ((rn as u32) << 16)
            | reglist
    }

    fn arm_data_processing_register_shift(
        opcode: u32,
        set_flags: bool,
        rn: usize,
        rd: usize,
        rm: usize,
        rs: usize,
    ) -> u32 {
        0xe000_0000
            | ((opcode & 0xf) << 21)
            | enc_bit(set_flags, 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | ((rs as u32) << 8)
            | 0x10
            | (rm as u32)
    }

    #[test]
    fn arm_data_processing_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x1000, 0x1000);
        mem.load_arm_words(
            0x1000,
            &[
                0xe3a0_0001, // mov r0, #1
                0xe280_1002, // add r1, r0, #2
                0xe351_0003, // cmp r1, #3
                0x13a0_2009, // movne r2, #9
                0x03a0_2007, // moveq r2, #7
                0xe12f_ff1e, // bx lr
            ],
        )
        .unwrap();
        cpu.set_pc(0x1000);
        cpu.set_reg(14, 0xffff_0000);
        for _ in 0..5 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(cpu.reg(1), 3);
        assert_eq!(cpu.reg(2), 7);
        assert!(cpu.cpsr.z);
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.pc(), 0xffff_0000);
    }

    #[test]
    fn arm_memory_and_block_transfer_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x1000, 0x2000);
        mem.load_arm_words(
            0x1000,
            &[
                0xe3a0_000a, // mov r0, #10
                0xe3a0_1014, // mov r1, #20
                0xe882_0003, // stm r2, {r0, r1}
                0xe3a0_0000, // mov r0, #0
                0xe3a0_1000, // mov r1, #0
                0xe892_000c, // ldm r2, {r2, r3}
            ],
        )
        .unwrap();
        cpu.set_pc(0x1000);
        cpu.set_reg(2, 0x1800);
        for _ in 0..6 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(2), 10);
        assert_eq!(cpu.reg(3), 20);

        let err = cpu.execute_arm(0xe8b0_0000, 0x1018, &mut mem).unwrap_err(); // ldmia r0!, {}
        assert!(matches!(
            err,
            Trap::Unpredictable("empty block transfer register list")
        ));

        let err = cpu.execute_arm(0xe8d0_0002, 0x101c, &mut mem).unwrap_err(); // ldmia r0, {r1}^
        assert!(matches!(
            err,
            Trap::Unpredictable("block transfer user-mode/S bit")
        ));

        mem.store32(0x1900, 0x1122_3344).unwrap();
        mem.store32(0x1904, 0x5566_7788).unwrap();
        cpu.set_reg(2, 0x1900);
        cpu.execute_arm(0xe1c2_00d0, 0, &mut mem).unwrap(); // ldrd r0, r1, [r2]
        assert_eq!(cpu.reg(0), 0x1122_3344);
        assert_eq!(cpu.reg(1), 0x5566_7788);

        cpu.set_reg(4, 0xaabb_ccdd);
        cpu.set_reg(5, 0xeeff_0011);
        cpu.set_reg(6, 0x1910);
        cpu.execute_arm(0xe1c6_40f0, 0, &mut mem).unwrap(); // strd r4, r5, [r6]
        assert_eq!(mem.load32(0x1910).unwrap(), 0xaabb_ccdd);
        assert_eq!(mem.load32(0x1914).unwrap(), 0xeeff_0011);

        cpu.set_reg(2, 0x1900);
        let err = cpu.execute_arm(0xe1c2_10d0, 0, &mut mem).unwrap_err(); // ldrd r1, r2, [r2]
        assert!(matches!(err, Trap::Unpredictable(_)));

        cpu.set_reg(3, 0x1910);
        let err = cpu.execute_arm(0xe1c3_e0d0, 0, &mut mem).unwrap_err(); // ldrd r14, r15, [r3]
        assert!(matches!(err, Trap::Unpredictable(_)));

        cpu.set_reg(3, 0x1910);
        let err = cpu.execute_arm(0xe1c3_10f0, 0, &mut mem).unwrap_err(); // strd r1, r2, [r3]
        assert!(matches!(err, Trap::Unpredictable(_)));

        cpu.set_reg(3, 0x1910);
        let err = cpu.execute_arm(0xe1c3_e0f0, 0, &mut mem).unwrap_err(); // strd r14, r15, [r3]
        assert!(matches!(err, Trap::Unpredictable(_)));

        mem.store32(0x1940, 0x7001).unwrap();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(0, 0x1940);
        cpu.execute_arm(0xe8b0_8000, 0, &mut mem).unwrap(); // ldmia r0!, {pc}
        assert_eq!(cpu.reg(0), 0x1944);
        assert_eq!(cpu.pc(), 0x7000);
        assert!(cpu.cpsr.t);

        mem.store32(0x1950, 0x8000).unwrap();
        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(13, 0x1950);
        cpu.execute_thumb(0xbd00, 0, &mut mem).unwrap(); // pop {pc}
        assert_eq!(cpu.reg(13), 0x1954);
        assert_eq!(cpu.pc(), 0x8000);
        assert!(!cpu.cpsr.t);

        cpu.set_reg(1, 0xdead_beef);
        cpu.set_reg(2, 0x1920);
        cpu.execute_arm(0xe4a2_1004, 0, &mut mem).unwrap(); // strt r1, [r2], #4
        assert_eq!(mem.load32(0x1920).unwrap(), 0xdead_beef);
        assert_eq!(cpu.reg(2), 0x1924);

        cpu.set_reg(2, 0x1920);
        cpu.execute_arm(0xe432_0004, 0, &mut mem).unwrap(); // ldrt r0, [r2], #-4
        assert_eq!(cpu.reg(0), 0xdead_beef);
        assert_eq!(cpu.reg(2), 0x191c);

        cpu.set_reg(3, 0xaa55);
        cpu.set_reg(4, 0x1930);
        cpu.set_reg(5, 3);
        cpu.execute_arm(0xe6e4_3005, 0, &mut mem).unwrap(); // strbt r3, [r4], r5
        assert_eq!(mem.load8(0x1930).unwrap(), 0x55);
        assert_eq!(cpu.reg(4), 0x1933);

        mem.store8(0x1933, 0x7e).unwrap();
        cpu.execute_arm(0xe674_6005, 0, &mut mem).unwrap(); // ldrbt r6, [r4], -r5
        assert_eq!(cpu.reg(6), 0x7e);
        assert_eq!(cpu.reg(4), 0x1930);
    }

    #[test]
    fn arm_load_store_unpredictable_register_forms() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x1000, 0x1000);

        let err = cpu
            .execute_arm(
                arm_single_transfer(false, false, true, false, false, true, 1, 1, 4),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldr r1, [r1], #4
        assert_eq!(
            err,
            Trap::Unpredictable("single transfer writeback overlaps transferred register")
        );

        let err = cpu
            .execute_arm(
                arm_single_transfer(true, true, true, false, false, true, 1, 0, 15),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldr r0, [r1, pc]
        assert_eq!(
            err,
            Trap::Unpredictable("single transfer with PC offset register")
        );

        let err = cpu
            .execute_arm(
                arm_single_transfer(false, true, true, true, false, true, 0, 15, 0),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrb pc, [r0]
        assert_eq!(
            err,
            Trap::Unpredictable("byte transfer with PC destination/source register")
        );

        let err = cpu
            .execute_arm(
                arm_single_transfer(false, false, true, false, false, true, 15, 0, 4),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldr r0, [pc], #4
        assert_eq!(
            err,
            Trap::Unpredictable("single transfer writeback with PC base register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(true, true, true, false, true, 0, 15, 0b01, 0),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrh pc, [r0]
        assert_eq!(
            err,
            Trap::Unpredictable("halfword load with PC destination register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(false, true, true, false, true, 2, 2, 0b11, 2),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrsh r2, [r2], #2
        assert_eq!(
            err,
            Trap::Unpredictable("halfword load writeback with invalid base register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(true, true, true, false, false, 0, 15, 0b01, 0),
                0,
                &mut mem,
            )
            .unwrap_err(); // strh pc, [r0]
        assert_eq!(
            err,
            Trap::Unpredictable("halfword store with PC source register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(true, true, false, false, true, 0, 1, 0b01, 15),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrh r1, [r0, pc]
        assert_eq!(
            err,
            Trap::Unpredictable("halfword transfer with PC offset register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(false, true, true, false, false, 2, 2, 0b10, 4),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrd r2, [r2], #4
        assert_eq!(
            err,
            Trap::Unpredictable("doubleword transfer writeback with invalid base register")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(true, true, false, false, false, 3, 2, 0b10, 2),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldrd r2, [r3, r2]
        assert_eq!(
            err,
            Trap::Unpredictable("LDRD register offset overlaps destination")
        );

        let err = cpu
            .execute_arm(
                arm_halfword_transfer(false, true, true, false, false, 4, 4, 0b11, 8),
                0,
                &mut mem,
            )
            .unwrap_err(); // strd r4, [r4], #8
        assert_eq!(
            err,
            Trap::Unpredictable("doubleword transfer writeback with invalid base register")
        );

        let err = cpu
            .execute_arm(
                arm_block_transfer(false, true, false, false, true, 15, 1),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldmia pc, {r0}
        assert_eq!(
            err,
            Trap::Unpredictable("block transfer with PC base register")
        );

        let err = cpu
            .execute_arm(
                arm_block_transfer(false, true, false, false, false, 15, 1),
                0,
                &mut mem,
            )
            .unwrap_err(); // stmia pc, {r0}
        assert_eq!(
            err,
            Trap::Unpredictable("block transfer with PC base register")
        );

        let err = cpu
            .execute_arm(
                arm_block_transfer(false, true, false, true, true, 0, 1),
                0,
                &mut mem,
            )
            .unwrap_err(); // ldmia r0!, {r0}
        assert_eq!(
            err,
            Trap::Unpredictable("LDM writeback with base in register list")
        );
    }

    #[test]
    fn arm_adc_sbc_flags_use_full_carry_input() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, u32::MAX);
        cpu.set_reg(2, 0);
        cpu.cpsr.c = true;
        cpu.execute_arm(0xe0b1_0002, 0, &mut mem).unwrap(); // adcs r0, r1, r2
        assert_eq!(cpu.reg(0), 0);
        assert!(cpu.cpsr.z);
        assert!(cpu.cpsr.c);

        cpu.set_reg(4, 0);
        cpu.set_reg(5, 0);
        cpu.cpsr.c = false;
        cpu.execute_arm(0xe0d4_3005, 0, &mut mem).unwrap(); // sbcs r3, r4, r5
        assert_eq!(cpu.reg(3), u32::MAX);
        assert!(!cpu.cpsr.c);
        assert!(cpu.cpsr.n);
    }

    #[test]
    fn arm_data_processing_register_shift_traps_pc_forms() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 1);
        cpu.set_reg(2, 3);
        cpu.set_reg(3, 4);
        cpu.execute_arm(
            arm_data_processing_register_shift(0x4, false, 1, 0, 2, 3),
            0,
            &mut mem,
        )
        .unwrap(); // add r0, r1, r2, lsl r3
        assert_eq!(cpu.reg(0), 49);

        let err = cpu
            .execute_arm(
                arm_data_processing_register_shift(0x4, false, 15, 0, 2, 3),
                0,
                &mut mem,
            )
            .unwrap_err(); // add r0, pc, r2, lsl r3
        assert_eq!(
            err,
            Trap::Unpredictable("data-processing register shift with PC register")
        );

        let err = cpu
            .execute_arm(
                arm_data_processing_register_shift(0x4, false, 1, 0, 15, 3),
                0,
                &mut mem,
            )
            .unwrap_err(); // add r0, r1, pc, lsl r3
        assert_eq!(
            err,
            Trap::Unpredictable("data-processing register shift with PC register")
        );

        let err = cpu
            .execute_arm(
                arm_data_processing_register_shift(0x4, false, 1, 0, 2, 15),
                0,
                &mut mem,
            )
            .unwrap_err(); // add r0, r1, r2, lsl pc
        assert_eq!(
            err,
            Trap::Unpredictable("data-processing register shift with PC register")
        );

        let err = cpu
            .execute_arm(
                arm_data_processing_register_shift(0x4, false, 1, 15, 2, 3),
                0,
                &mut mem,
            )
            .unwrap_err(); // add pc, r1, r2, lsl r3
        assert_eq!(
            err,
            Trap::Unpredictable("data-processing register shift with PC register")
        );

        let err = cpu
            .execute_arm(
                arm_data_processing_register_shift(0xa, true, 15, 0, 2, 3),
                0,
                &mut mem,
            )
            .unwrap_err(); // cmp pc, r2, lsl r3
        assert_eq!(
            err,
            Trap::Unpredictable("data-processing register shift with PC register")
        );
    }

    #[test]
    fn arm_and_thumb_blx_interworking_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.execute_arm(0xfa00_0000, 0x1000, &mut mem).unwrap(); // blx #0
        assert_eq!(cpu.reg(14), 0x1004);
        assert_eq!(cpu.pc(), 0x1008);
        assert!(cpu.cpsr.t);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(3, 0x1801);
        cpu.execute_arm(0xe12f_ff23, 0x1000, &mut mem).unwrap(); // bxj r3
        assert_eq!(cpu.pc(), 0x1800);
        assert!(cpu.cpsr.t);

        cpu.set_isa(Isa::Thumb);
        cpu.execute_thumb(0xf000, 0x2000, &mut mem).unwrap(); // bl/blx prefix
        cpu.execute_thumb(0xe808, 0x2002, &mut mem).unwrap(); // blx suffix
        assert_eq!(cpu.reg(14), 0x2005);
        assert_eq!(cpu.pc(), 0x2014);
        assert!(!cpu.cpsr.t);

        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(0, 0x3000);
        cpu.execute_thumb(0x4780, 0x2100, &mut mem).unwrap(); // blx r0
        assert_eq!(cpu.reg(14), 0x2103);
        assert_eq!(cpu.pc(), 0x3000);
        assert!(!cpu.cpsr.t);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(0, 0x4001);
        cpu.execute_arm(0xe1a0_f000, 0x2200, &mut mem).unwrap(); // mov pc, r0
        assert_eq!(cpu.pc(), 0x4000);
        assert!(cpu.cpsr.t);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(1, 0);
        mem.store32(0, 0x5001).unwrap();
        cpu.execute_arm(0xe591_f000, 0x2300, &mut mem).unwrap(); // ldr pc, [r1]
        assert_eq!(cpu.pc(), 0x5000);
        assert!(cpu.cpsr.t);

        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(0, 0x6000);
        cpu.execute_thumb(0x4687, 0x2400, &mut mem).unwrap(); // mov pc, r0
        assert_eq!(cpu.pc(), 0x6000);
        assert!(cpu.cpsr.t);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(0, 0x7001);
        let err = cpu.execute_arm(0xe1b0_f000, 0x2500, &mut mem).unwrap_err(); // movs pc, r0
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x2500,
                instr: 0xe1b0_f000,
                operation: "data-processing exception return",
            }
        );
    }

    #[test]
    fn arm_unconditional_hints_are_explicitly_handled() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.execute_arm(0xf5d1_f000, 0, &mut mem).unwrap(); // pld [r1]
        cpu.execute_arm(0xf753_f004, 0, &mut mem).unwrap(); // pld [r3, -r4]
        cpu.execute_arm(0xf101_0000, 0, &mut mem).unwrap(); // setend le
        cpu.execute_arm(0xe320_f001, 0, &mut mem).unwrap(); // yield hint
        cpu.execute_arm(0xf320_f001, 0, &mut mem).unwrap(); // unconditional yield hint

        let err = cpu.execute_arm(0xf101_0200, 0, &mut mem).unwrap_err(); // setend be
        assert!(matches!(err, Trap::Unpredictable(_)));

        let err = cpu.execute_arm(0xf108_0080, 0x44, &mut mem).unwrap_err(); // cpsie i
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x44,
                instr: 0xf108_0080,
                operation: "CPS",
            }
        );

        let err = cpu.execute_arm(0xf89d_0a00, 0x48, &mut mem).unwrap_err(); // rfeia sp
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x48,
                instr: 0xf89d_0a00,
                operation: "RFE",
            }
        );

        let err = cpu.execute_arm(0xf96d_0513, 0x4c, &mut mem).unwrap_err(); // srsdb sp!, #19
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x4c,
                instr: 0xf96d_0513,
                operation: "SRS",
            }
        );

        let err = cpu.execute_arm(0xf000_0000, 0, &mut mem).unwrap_err();
        assert!(matches!(err, Trap::UndefinedArm { .. }));

        let err = cpu.execute_arm(0xe7f0_00f0, 0x40, &mut mem).unwrap_err(); // udf #0
        assert_eq!(
            err,
            Trap::UndefinedArm {
                pc: 0x40,
                instr: 0xe7f0_00f0
            }
        );
    }

    #[test]
    fn arm_unsupported_newer_a32_encodings_trap_undefined() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        for instr in [
            0xe300_0000, // movw r0, #0
            0xe340_0000, // movt r0, #0
            0xe7c0_001f, // bfc r0, #0, #1
            0xe7c0_0010, // bfi r0, r0, #0, #1
            0xe7a0_0050, // sbfx r0, r0, #0, #1
            0xe7e0_0050, // ubfx r0, r0, #0, #1
            0xe6ff_0f30, // rbit r0, r0
            0xe710_f010, // sdiv r0, r0, r0
            0xe730_f010, // udiv r0, r0, r0
        ] {
            let err = cpu.execute_arm(instr, 0x80, &mut mem).unwrap_err();
            assert_eq!(err, Trap::UndefinedArm { pc: 0x80, instr });
        }
    }

    #[test]
    fn arm_and_thumb_exception_traps_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        let err = cpu.execute_arm(0xef00_0034, 0x100, &mut mem).unwrap_err(); // svc #0x34
        assert_eq!(
            err,
            Trap::SoftwareInterrupt {
                pc: 0x100,
                comment: 0x34,
            }
        );

        let err = cpu.execute_arm(0xe120_0a7b, 0x104, &mut mem).unwrap_err(); // bkpt #0xab
        assert_eq!(
            err,
            Trap::Breakpoint {
                pc: 0x104,
                comment: 0xab,
            }
        );

        let err = cpu.execute_thumb(0xdf56, 0x108, &mut mem).unwrap_err(); // svc #0x56
        assert_eq!(
            err,
            Trap::SoftwareInterrupt {
                pc: 0x108,
                comment: 0x56,
            }
        );
    }

    #[test]
    fn arm_status_register_access_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(2, 0xf80a_0000);
        cpu.execute_arm(0xe12c_f002, 0, &mut mem).unwrap(); // msr APSR_nzcvqg, r2
        assert!(cpu.cpsr.n);
        assert!(cpu.cpsr.z);
        assert!(cpu.cpsr.c);
        assert!(cpu.cpsr.v);
        assert!(cpu.cpsr.q);
        assert_eq!(cpu.cpsr.ge, 0xa);

        cpu.execute_arm(0xe10f_0000, 0, &mut mem).unwrap(); // mrs r0, cpsr
        assert_eq!(cpu.reg(0), cpu.cpsr.to_u32());

        let err = cpu.execute_arm(0xe10f_f000, 0x10, &mut mem).unwrap_err(); // mrs pc, cpsr
        assert_eq!(
            err,
            Trap::Unpredictable("status register access with PC register")
        );

        cpu.set_reg(4, 0);
        cpu.execute_arm(0xe128_f004, 0, &mut mem).unwrap(); // msr APSR_nzcvq, r4
        assert!(!cpu.cpsr.n);
        assert!(!cpu.cpsr.z);
        assert!(!cpu.cpsr.c);
        assert!(!cpu.cpsr.v);
        assert!(!cpu.cpsr.q);
        assert_eq!(cpu.cpsr.ge, 0xa);

        cpu.execute_arm(0xe328_f20f, 0, &mut mem).unwrap(); // msr APSR_nzcvq, #0xf0000000
        assert!(cpu.cpsr.n);
        assert!(cpu.cpsr.z);
        assert!(cpu.cpsr.c);
        assert!(cpu.cpsr.v);
        assert!(!cpu.cpsr.q);

        cpu.execute_arm(0xe32c_f80f, 0, &mut mem).unwrap(); // msr APSR_nzcvqg, #0x000f0000
        assert!(!cpu.cpsr.n);
        assert!(!cpu.cpsr.z);
        assert!(!cpu.cpsr.c);
        assert!(!cpu.cpsr.v);
        assert!(!cpu.cpsr.q);
        assert_eq!(cpu.cpsr.ge, 0xf);

        let err = cpu.execute_arm(0xe128_f00f, 0x14, &mut mem).unwrap_err(); // msr APSR_nzcvq, pc
        assert_eq!(
            err,
            Trap::Unpredictable("status register access with PC register")
        );

        let err = cpu.execute_arm(0xe14f_1000, 0x50, &mut mem).unwrap_err(); // mrs r1, spsr
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x50,
                instr: 0xe14f_1000,
                operation: "MRS SPSR",
            }
        );

        let err = cpu.execute_arm(0xe121_f004, 0x54, &mut mem).unwrap_err(); // msr CPSR_c, r4
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x54,
                instr: 0xe121_f004,
                operation: "MSR CPSR control",
            }
        );

        let err = cpu.execute_arm(0xe168_f005, 0x58, &mut mem).unwrap_err(); // msr SPSR_f, r5
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x58,
                instr: 0xe168_f005,
                operation: "MSR SPSR",
            }
        );
    }

    #[test]
    fn arm_cp15_thread_id_registers_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.cp15_tpidruro = 0x1234_5678;
        cpu.execute_arm(0xee1d_0f70, 0, &mut mem).unwrap(); // mrc p15, #0, r0, c13, c0, #3
        assert_eq!(cpu.reg(0), 0x1234_5678);

        cpu.cp15_tpidrurw = 0xaabb_ccdd;
        cpu.execute_arm(0xee1d_2f50, 0, &mut mem).unwrap(); // mrc p15, #0, r2, c13, c0, #2
        assert_eq!(cpu.reg(2), 0xaabb_ccdd);

        cpu.set_reg(1, 0xfeed_cafe);
        cpu.execute_arm(0xee0d_1f50, 0, &mut mem).unwrap(); // mcr p15, #0, r1, c13, c0, #2
        assert_eq!(cpu.cp15_tpidrurw, 0xfeed_cafe);
        cpu.execute_arm(0xee1d_3f70, 0, &mut mem).unwrap(); // mrc p15, #0, r3, c13, c0, #3
        assert_eq!(cpu.reg(3), 0x1234_5678);

        let err = cpu.execute_arm(0xee1d_ff50, 0x10, &mut mem).unwrap_err(); // mrc p15, #0, pc, c13, c0, #2
        assert_eq!(err, Trap::Unpredictable("CP15 TLS MRC to PC"));

        let err = cpu.execute_arm(0xee0d_ff50, 0x14, &mut mem).unwrap_err(); // mcr p15, #0, pc, c13, c0, #2
        assert_eq!(err, Trap::Unpredictable("CP15 TLS MCR from PC"));
        let err = cpu.execute_arm(0xee0d_1f70, 0x18, &mut mem).unwrap_err(); // mcr p15, #0, r1, c13, c0, #3
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x18,
                instr: 0xee0d_1f70,
                operation: "MCR TPIDRURO",
            }
        );
        assert_eq!(cpu.cp15_tpidruro, 0x1234_5678);
    }

    #[test]
    fn arm_cp15_barriers_are_user_mode_hle_noops() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);
        cpu.set_reg(0, 0xaaaa_5555);

        cpu.execute_arm(0xee07_0fba, 0, &mut mem).unwrap(); // mcr p15, #0, r0, c7, c10, #5 (DMB)
        cpu.execute_arm(0xee07_0f9a, 4, &mut mem).unwrap(); // mcr p15, #0, r0, c7, c10, #4 (DSB)
        cpu.execute_arm(0xee07_0f95, 8, &mut mem).unwrap(); // mcr p15, #0, r0, c7, c5, #4 (ISB)
        assert_eq!(cpu.reg(0), 0xaaaa_5555);

        let err = cpu.execute_arm(0xee07_ffba, 0x0c, &mut mem).unwrap_err(); // mcr p15, #0, pc, c7, c10, #5
        assert_eq!(err, Trap::Unpredictable("CP15 barrier MCR from PC"));
    }

    #[test]
    fn thumb_alu_memory_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x2000, 0x1000);
        mem.load_thumb_halfwords(
            0x2000,
            &[
                0x2007, // movs r0, #7
                0x3003, // adds r0, #3
                0x6008, // str r0, [r1]
                0x680a, // ldr r2, [r1]
                0x4770, // bx lr
            ],
        )
        .unwrap();
        cpu.set_isa(Isa::Thumb);
        cpu.set_pc(0x2000);
        cpu.set_reg(1, 0x2400);
        cpu.set_reg(14, 0xffff_0001);
        for _ in 0..4 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(0), 10);
        assert_eq!(cpu.reg(2), 10);
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.pc(), 0xffff_0000);
        assert!(cpu.cpsr.t);
    }

    #[test]
    fn thumb_push_pop_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x3000, 0x2000);
        mem.load_thumb_halfwords(
            0x3000,
            &[
                0x2001, // movs r0, #1
                0x2102, // movs r1, #2
                0xb403, // push {r0, r1}
                0x2000, // movs r0, #0
                0x2100, // movs r1, #0
                0xbc0c, // pop {r2, r3}
            ],
        )
        .unwrap();
        cpu.set_isa(Isa::Thumb);
        cpu.set_pc(0x3000);
        cpu.set_reg(13, 0x4000);
        for _ in 0..6 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(2), 1);
        assert_eq!(cpu.reg(3), 2);
        assert_eq!(cpu.reg(13), 0x4000);

        mem.store32(0x4020, 0x33).unwrap();
        mem.store32(0x4024, 0x44).unwrap();
        cpu.set_reg(0, 0x4020);
        cpu.execute_thumb(0xc803, 0x300c, &mut mem).unwrap(); // ldmia r0!, {r0, r1}
        assert_eq!(cpu.reg(0), 0x33);
        assert_eq!(cpu.reg(1), 0x44);
    }

    #[test]
    fn thumb_unpredictable_register_forms() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x4000, 0x100);
        cpu.set_isa(Isa::Thumb);

        let err = cpu.execute_thumb(0xb400, 0, &mut mem).unwrap_err(); // push {}
        assert_eq!(
            err,
            Trap::Unpredictable("empty Thumb push/pop register list")
        );
        let err = cpu.execute_thumb(0xbc00, 0, &mut mem).unwrap_err(); // pop {}
        assert_eq!(
            err,
            Trap::Unpredictable("empty Thumb push/pop register list")
        );

        cpu.set_reg(0, 0x4020);
        cpu.set_reg(1, 0x1122_3344);
        cpu.execute_thumb(0xc003, 0, &mut mem).unwrap(); // stmia r0!, {r0, r1}
        assert_eq!(mem.load32(0x4020).unwrap(), 0x4020);
        assert_eq!(mem.load32(0x4024).unwrap(), 0x1122_3344);
        assert_eq!(cpu.reg(0), 0x4028);

        let err = cpu.execute_thumb(0xc103, 0, &mut mem).unwrap_err(); // stmia r1!, {r0, r1}
        assert_eq!(
            err,
            Trap::Unpredictable("Thumb STM writeback base not first in register list")
        );

        let err = cpu.execute_thumb(0x4508, 0, &mut mem).unwrap_err(); // cmp r0, r1 in high-register encoding
        assert_eq!(
            err,
            Trap::Unpredictable("Thumb high-register CMP with invalid registers")
        );
        let err = cpu.execute_thumb(0x4587, 0, &mut mem).unwrap_err(); // cmp pc, r0
        assert_eq!(
            err,
            Trap::Unpredictable("Thumb high-register CMP with invalid registers")
        );
        let err = cpu.execute_thumb(0x44ff, 0, &mut mem).unwrap_err(); // add pc, pc
        assert_eq!(
            err,
            Trap::Unpredictable("Thumb high-register ADD with PC operands")
        );
    }

    #[test]
    fn thumb_armv6_extend_reverse_and_breakpoint_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);
        cpu.set_isa(Isa::Thumb);

        cpu.set_reg(1, 0x0000_8001);
        cpu.execute_thumb(0xb208, 0, &mut mem).unwrap(); // sxth r0, r1
        assert_eq!(cpu.reg(0), 0xffff_8001);

        cpu.set_reg(3, 0x0000_00f1);
        cpu.execute_thumb(0xb25a, 0, &mut mem).unwrap(); // sxtb r2, r3
        assert_eq!(cpu.reg(2), 0xffff_fff1);

        cpu.set_reg(5, 0x1234_8001);
        cpu.execute_thumb(0xb2ac, 0, &mut mem).unwrap(); // uxth r4, r5
        assert_eq!(cpu.reg(4), 0x8001);

        cpu.set_reg(7, 0x1234_56ab);
        cpu.execute_thumb(0xb2fe, 0, &mut mem).unwrap(); // uxtb r6, r7
        assert_eq!(cpu.reg(6), 0xab);

        cpu.set_reg(1, 0x1122_3344);
        cpu.execute_thumb(0xba08, 0, &mut mem).unwrap(); // rev r0, r1
        assert_eq!(cpu.reg(0), 0x4433_2211);

        cpu.set_reg(3, 0x1122_3344);
        cpu.execute_thumb(0xba5a, 0, &mut mem).unwrap(); // rev16 r2, r3
        assert_eq!(cpu.reg(2), 0x2211_4433);

        cpu.set_reg(5, 0x0000_80ff);
        cpu.execute_thumb(0xbaec, 0, &mut mem).unwrap(); // revsh r4, r5
        assert_eq!(cpu.reg(4), 0xffff_ff80);

        let err = cpu.execute_thumb(0xbe07, 0x1234, &mut mem).unwrap_err();
        assert_eq!(
            err,
            Trap::Breakpoint {
                pc: 0x1234,
                comment: 7
            }
        );

        cpu.execute_thumb(0xb650, 0, &mut mem).unwrap(); // setend le
        let err = cpu.execute_thumb(0xb662, 0x24, &mut mem).unwrap_err(); // cpsie i
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x24,
                instr: 0xb662,
                operation: "CPS",
            }
        );
        let err = cpu.execute_thumb(0xb672, 0x28, &mut mem).unwrap_err(); // cpsid i
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x28,
                instr: 0xb672,
                operation: "CPS",
            }
        );
        let err = cpu.execute_thumb(0xb658, 0, &mut mem).unwrap_err(); // setend be
        assert!(matches!(err, Trap::Unpredictable(_)));
    }

    #[test]
    fn armv6_misc_reversal_extension_and_saturation() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 0x1122_3344);
        cpu.execute_arm(0xe6bf_0f31, 0, &mut mem).unwrap(); // rev r0, r1
        assert_eq!(cpu.reg(0), 0x4433_2211);

        cpu.set_reg(3, 0x1122_3344);
        cpu.execute_arm(0xe6bf_2fb3, 0, &mut mem).unwrap(); // rev16 r2, r3
        assert_eq!(cpu.reg(2), 0x2211_4433);

        cpu.set_reg(5, 0x0000_80ff);
        cpu.execute_arm(0xe6ff_4fb5, 0, &mut mem).unwrap(); // revsh r4, r5
        assert_eq!(cpu.reg(4), 0xffff_ff80);

        cpu.set_reg(1, 0x0000_00f1);
        cpu.execute_arm(0xe6af_0071, 0, &mut mem).unwrap(); // sxtb r0, r1
        assert_eq!(cpu.reg(0), 0xffff_fff1);

        cpu.set_reg(3, 0x0000_8001);
        cpu.execute_arm(0xe6bf_2073, 0, &mut mem).unwrap(); // sxth r2, r3
        assert_eq!(cpu.reg(2), 0xffff_8001);

        cpu.set_reg(5, 0x1234_56ab);
        cpu.execute_arm(0xe6ef_4075, 0, &mut mem).unwrap(); // uxtb r4, r5
        assert_eq!(cpu.reg(4), 0xab);

        cpu.set_reg(7, 0x1234_8001);
        cpu.execute_arm(0xe6ff_6077, 0, &mut mem).unwrap(); // uxth r6, r7
        assert_eq!(cpu.reg(6), 0x8001);

        cpu.set_reg(1, 10);
        cpu.set_reg(2, 0x0000_00f6);
        cpu.execute_arm(0xe6a1_0072, 0, &mut mem).unwrap(); // sxtab r0, r1, r2
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(4, 0x0001_0002);
        cpu.set_reg(5, 0x00fe_00ff);
        cpu.execute_arm(0xe684_3075, 0, &mut mem).unwrap(); // sxtab16 r3, r4, r5
        assert_eq!(cpu.reg(3), 0xffff_0001);

        cpu.set_reg(7, 5);
        cpu.set_reg(8, 0x0000_fff0);
        cpu.execute_arm(0xe6b7_6078, 0, &mut mem).unwrap(); // sxtah r6, r7, r8
        assert_eq!(cpu.reg(6), (-11i32) as u32);

        cpu.set_reg(10, 0x00fe_00ff);
        cpu.execute_arm(0xe68f_907a, 0, &mut mem).unwrap(); // sxtb16 r9, r10
        assert_eq!(cpu.reg(9), 0xfffe_ffff);

        cpu.set_reg(1, 10);
        cpu.set_reg(2, 0x0000_00f6);
        cpu.execute_arm(0xe6e1_0072, 0, &mut mem).unwrap(); // uxtab r0, r1, r2
        assert_eq!(cpu.reg(0), 256);

        cpu.set_reg(4, 0x0001_0002);
        cpu.set_reg(5, 0x00fe_00ff);
        cpu.execute_arm(0xe6c4_3075, 0, &mut mem).unwrap(); // uxtab16 r3, r4, r5
        assert_eq!(cpu.reg(3), 0x00ff_0101);

        cpu.set_reg(7, 5);
        cpu.set_reg(8, 0x0000_fff0);
        cpu.execute_arm(0xe6f7_6078, 0, &mut mem).unwrap(); // uxtah r6, r7, r8
        assert_eq!(cpu.reg(6), 0xfff5);

        cpu.set_reg(10, 0x00fe_00ff);
        cpu.execute_arm(0xe6cf_907a, 0, &mut mem).unwrap(); // uxtb16 r9, r10
        assert_eq!(cpu.reg(9), 0x00fe_00ff);

        cpu.set_reg(1, 0x7fff_ffff);
        cpu.set_reg(2, 1);
        cpu.execute_arm(0xe102_0051, 0, &mut mem).unwrap(); // qadd r0, r1, r2
        assert_eq!(cpu.reg(0), 0x7fff_ffff);
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_reg(1, 200);
        cpu.execute_arm(0xe6a7_0011, 0, &mut mem).unwrap(); // ssat r0, #8, r1
        assert_eq!(cpu.reg(0), 127);
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_reg(3, u32::MAX);
        cpu.execute_arm(0xe6e7_2013, 0, &mut mem).unwrap(); // usat r2, #7, r3
        assert_eq!(cpu.reg(2), 0);
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_reg(1, 0x0100_ff00);
        cpu.execute_arm(0xe6a7_0f31, 0, &mut mem).unwrap(); // ssat16 r0, #8, r1
        assert_eq!(cpu.reg(0), 0x007f_ff80);
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_reg(3, 0x0100_fffe);
        cpu.execute_arm(0xe6e7_2f33, 0, &mut mem).unwrap(); // usat16 r2, #7, r3
        assert_eq!(cpu.reg(2), 0x007f_0000);
        assert!(cpu.cpsr.q);

        let err = cpu.execute_arm(0xe16f_ff11, 0, &mut mem).unwrap_err(); // clz pc, r1
        assert!(matches!(
            err,
            Trap::Unpredictable("misc instruction with PC register")
        ));
        let err = cpu.execute_arm(0xe6bf_ff31, 0, &mut mem).unwrap_err(); // rev pc, r1
        assert!(matches!(
            err,
            Trap::Unpredictable("misc instruction with PC register")
        ));
        let err = cpu.execute_arm(0xe6af_f071, 0, &mut mem).unwrap_err(); // sxtb pc, r1
        assert!(matches!(
            err,
            Trap::Unpredictable("extend with PC register")
        ));
        let err = cpu.execute_arm(0xe10f_0051, 0, &mut mem).unwrap_err(); // qadd r0, r1, pc
        assert!(matches!(
            err,
            Trap::Unpredictable("saturating arithmetic with PC register")
        ));
        let err = cpu.execute_arm(0xe6a7_0f3f, 0, &mut mem).unwrap_err(); // ssat16 r0, #8, pc
        assert!(matches!(
            err,
            Trap::Unpredictable("saturation with PC register")
        ));
    }

    #[test]
    fn armv6_swap_and_exclusive_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x5000, 0x100);
        mem.store32(0x5040, 0x1122_3344).unwrap();

        cpu.set_reg(1, 0xaabb_ccdd);
        cpu.set_reg(2, 0x5040);
        cpu.execute_arm(0xe102_0091, 0, &mut mem).unwrap(); // swp r0, r1, [r2]
        assert_eq!(cpu.reg(0), 0x1122_3344);
        assert_eq!(mem.load32(0x5040).unwrap(), 0xaabb_ccdd);

        mem.store32(0x5044, 0x1122_3344).unwrap();
        cpu.set_reg(1, 0xaa);
        cpu.set_reg(2, 0x5044);
        cpu.execute_arm(0xe142_0091, 0, &mut mem).unwrap(); // swpb r0, r1, [r2]
        assert_eq!(cpu.reg(0), 0x44);
        assert_eq!(mem.load32(0x5044).unwrap(), 0x1122_33aa);

        let err = cpu.execute_arm(0xe100_0091, 0, &mut mem).unwrap_err(); // swp r0, r1, [r0]
        assert_eq!(err, Trap::Unpredictable("SWP with invalid register form"));
        let err = cpu.execute_arm(0xe10f_0091, 0, &mut mem).unwrap_err(); // swp r0, r1, [pc]
        assert_eq!(err, Trap::Unpredictable("SWP with invalid register form"));

        cpu.set_reg(1, 0x5040);
        cpu.execute_arm(0xe191_0f9f, 0, &mut mem).unwrap(); // ldrex r0, [r1]
        assert_eq!(cpu.reg(0), 0xaabb_ccdd);

        cpu.set_reg(3, 0xfeed_cafe);
        cpu.set_reg(4, 0x5040);
        cpu.execute_arm(0xe184_2f93, 0, &mut mem).unwrap(); // strex r2, r3, [r4]
        assert_eq!(cpu.reg(2), 0);
        assert_eq!(mem.load32(0x5040).unwrap(), 0xfeed_cafe);

        mem.store8(0x5048, 0x12).unwrap();
        cpu.set_reg(1, 0x5048);
        cpu.execute_arm(0xe1d1_0f9f, 0, &mut mem).unwrap(); // ldrexb r0, [r1]
        assert_eq!(cpu.reg(0), 0x12);
        cpu.set_reg(1, 0x34);
        cpu.set_reg(2, 0x5048);
        cpu.execute_arm(0xe1c2_0f91, 0, &mut mem).unwrap(); // strexb r0, r1, [r2]
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(mem.load8(0x5048).unwrap(), 0x34);

        mem.store16(0x504a, 0x5678).unwrap();
        cpu.set_reg(3, 0x504a);
        cpu.execute_arm(0xe1f3_2f9f, 0, &mut mem).unwrap(); // ldrexh r2, [r3]
        assert_eq!(cpu.reg(2), 0x5678);
        cpu.set_reg(4, 0x9abc);
        cpu.set_reg(5, 0x504a);
        cpu.execute_arm(0xe1e5_3f94, 0, &mut mem).unwrap(); // strexh r3, r4, [r5]
        assert_eq!(cpu.reg(3), 0);
        assert_eq!(mem.load16(0x504a).unwrap(), 0x9abc);

        mem.store32(0x5050, 0x1111_2222).unwrap();
        mem.store32(0x5054, 0x3333_4444).unwrap();
        cpu.set_reg(6, 0x5050);
        cpu.execute_arm(0xe1b6_4f9f, 0, &mut mem).unwrap(); // ldrexd r4, r5, [r6]
        assert_eq!(cpu.reg(4), 0x1111_2222);
        assert_eq!(cpu.reg(5), 0x3333_4444);
        cpu.set_reg(7, 0x5050);
        cpu.set_reg(8, 0xaaaa_bbbb);
        cpu.set_reg(9, 0xcccc_dddd);
        cpu.execute_arm(0xe1a7_6f98, 0, &mut mem).unwrap(); // strexd r6, r8, r9, [r7]
        assert_eq!(cpu.reg(6), 0);
        assert_eq!(mem.load32(0x5050).unwrap(), 0xaaaa_bbbb);
        assert_eq!(mem.load32(0x5054).unwrap(), 0xcccc_dddd);

        mem.store32(0x5060, 0x1111_1111).unwrap();
        cpu.set_reg(4, 0x5060);
        cpu.execute_arm(0xe194_0f9f, 0, &mut mem).unwrap(); // ldrex r0, [r4]
        cpu.set_reg(2, 0x2222_2222);
        cpu.execute_arm(0xe584_2000, 0, &mut mem).unwrap(); // str r2, [r4]
        cpu.set_reg(3, 0x3333_3333);
        cpu.execute_arm(0xe184_0f93, 0, &mut mem).unwrap(); // strex r0, r3, [r4]
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(mem.load32(0x5060).unwrap(), 0x2222_2222);

        mem.store32(0x5068, 0x1111_1111).unwrap();
        mem.store32(0x506c, 0x4444_4444).unwrap();
        cpu.set_reg(4, 0x5068);
        cpu.execute_arm(0xe194_0f9f, 0, &mut mem).unwrap(); // ldrex r0, [r4]
        cpu.set_reg(2, 0x2222_2222);
        cpu.execute_arm(0xe584_2004, 0, &mut mem).unwrap(); // str r2, [r4, #4]
        cpu.set_reg(3, 0x3333_3333);
        cpu.execute_arm(0xe184_0f93, 0, &mut mem).unwrap(); // strex r0, r3, [r4]
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(mem.load32(0x5068).unwrap(), 0x3333_3333);
        assert_eq!(mem.load32(0x506c).unwrap(), 0x2222_2222);

        mem.store16(0x5070, 0x1111).unwrap();
        cpu.set_reg(4, 0x5070);
        cpu.execute_arm(0xe1f4_0f9f, 0, &mut mem).unwrap(); // ldrexh r0, [r4]
        cpu.set_reg(2, 0x22);
        cpu.execute_arm(0xe5c4_2001, 0, &mut mem).unwrap(); // strb r2, [r4, #1]
        cpu.set_reg(3, 0x3333);
        cpu.execute_arm(0xe1e4_0f93, 0, &mut mem).unwrap(); // strexh r0, r3, [r4]
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(mem.load16(0x5070).unwrap(), 0x2211);

        cpu.execute_arm(0xf57f_f01f, 0, &mut mem).unwrap(); // clrex
        cpu.set_reg(3, 0x1234_5678);
        cpu.set_reg(4, 0x5040);
        cpu.execute_arm(0xe184_2f93, 0, &mut mem).unwrap(); // strex r2, r3, [r4]
        assert_eq!(cpu.reg(2), 1);
        assert_eq!(mem.load32(0x5040).unwrap(), 0xfeed_cafe);

        let err = cpu.execute_arm(0xe191_ff9f, 0, &mut mem).unwrap_err(); // ldrex pc, [r1]
        assert!(matches!(
            err,
            Trap::Unpredictable("exclusive load with PC register")
        ));
        let err = cpu.execute_arm(0xe19f_0f9f, 0, &mut mem).unwrap_err(); // ldrex r0, [pc]
        assert!(matches!(
            err,
            Trap::Unpredictable("exclusive load with PC register")
        ));
        let err = cpu.execute_arm(0xe1b6_ef9f, 0, &mut mem).unwrap_err(); // ldrexd lr, [r6]
        assert!(matches!(
            err,
            Trap::Unpredictable("LDREXD with invalid register pair")
        ));
        let err = cpu.execute_arm(0xe184_4f93, 0, &mut mem).unwrap_err(); // strex r4, r3, [r4]
        assert!(matches!(
            err,
            Trap::Unpredictable("exclusive store status register overlaps operand")
        ));
        let err = cpu.execute_arm(0xe184_2f9f, 0, &mut mem).unwrap_err(); // strex r2, pc, [r4]
        assert!(matches!(
            err,
            Trap::Unpredictable("exclusive store with PC register")
        ));
        let err = cpu.execute_arm(0xe1a7_6f99, 0, &mut mem).unwrap_err(); // strexd r6, r9, [r7]
        assert!(matches!(
            err,
            Trap::Unpredictable("STREXD with invalid register pair")
        ));
        let err = cpu.execute_arm(0xe1a7_9f98, 0, &mut mem).unwrap_err(); // strexd r9, r8, [r7]
        assert!(matches!(
            err,
            Trap::Unpredictable("exclusive store status register overlaps operand")
        ));
    }

    #[test]
    fn armv6_pack_select_and_parallel_media_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 0xaaaa_5555);
        cpu.set_reg(2, 0x1122_3344);
        cpu.execute_arm(0xe681_0012, 0, &mut mem).unwrap(); // pkhbt r0, r1, r2
        assert_eq!(cpu.reg(0), 0x1122_5555);

        cpu.set_reg(4, 0xaaaa_5555);
        cpu.set_reg(5, 0x8123_4567);
        cpu.execute_arm(0xe684_3455, 0, &mut mem).unwrap(); // pkhtb r3, r4, r5, asr #8
        assert_eq!(cpu.reg(3), 0xaaaa_2345);

        cpu.cpsr.ge = 0b1010;
        cpu.set_reg(1, 0x1111_1111);
        cpu.set_reg(2, 0x2222_2222);
        cpu.execute_arm(0xe681_0fb2, 0, &mut mem).unwrap(); // sel r0, r1, r2
        assert_eq!(cpu.reg(0), 0x1122_1122);

        cpu.set_reg(1, 0x0001_ffff);
        cpu.set_reg(2, 0x0001_0001);
        cpu.execute_arm(0xe611_0f12, 0, &mut mem).unwrap(); // sadd16 r0, r1, r2
        assert_eq!(cpu.reg(0), 0x0002_0000);
        assert_eq!(cpu.cpsr.ge, 0b1111);

        cpu.set_reg(1, 0x0000_7f80);
        cpu.set_reg(2, 0x0000_0201);
        cpu.execute_arm(0xe621_0f92, 0, &mut mem).unwrap(); // qadd8 r0, r1, r2
        assert_eq!(cpu.reg(0), 0x0000_7f81);

        cpu.set_reg(1, 0x10f0_ffff);
        cpu.set_reg(2, 0xf020_0102);
        cpu.execute_arm(0xe661_0f92, 0, &mut mem).unwrap(); // uqadd8 r0, r1, r2
        assert_eq!(cpu.reg(0), 0xffff_ffff);

        cpu.set_reg(1, 0x0004_0008);
        cpu.set_reg(2, 0x0002_0002);
        cpu.execute_arm(0xe671_0f12, 0, &mut mem).unwrap(); // uhadd16 r0, r1, r2
        assert_eq!(cpu.reg(0), 0x0003_0005);

        let err = cpu.execute_arm(0xe68f_0012, 0, &mut mem).unwrap_err(); // pkhbt r0, pc, r2
        assert!(matches!(
            err,
            Trap::Unpredictable("packing with PC register")
        ));
        let err = cpu.execute_arm(0xe68f_0fb2, 0, &mut mem).unwrap_err(); // sel r0, pc, r2
        assert!(matches!(err, Trap::Unpredictable("SEL with PC register")));
        let err = cpu.execute_arm(0xe611_0f1f, 0, &mut mem).unwrap_err(); // sadd16 r0, r1, pc
        assert!(matches!(
            err,
            Trap::Unpredictable("parallel media with PC register")
        ));
    }

    #[test]
    fn armv5te_signed_halfword_multiply_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 3);
        cpu.set_reg(2, 4);
        cpu.set_reg(3, 5);
        cpu.execute_arm(0xe100_3281, 0, &mut mem).unwrap(); // smlabb r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 17);

        cpu.set_reg(1, 0xfffe_0000);
        cpu.set_reg(2, 0x0007_0000);
        cpu.set_reg(3, 1);
        cpu.execute_arm(0xe100_32e1, 0, &mut mem).unwrap(); // smlatt r0, r1, r2, r3
        assert_eq!(cpu.reg(0), (-13i32) as u32);

        cpu.set_reg(5, 3);
        cpu.set_reg(6, 0xfffc_0000);
        cpu.execute_arm(0xe164_06c5, 0, &mut mem).unwrap(); // smulbt r4, r5, r6
        assert_eq!(cpu.reg(4), (-12i32) as u32);

        cpu.set_reg(1, 0x0002_0000);
        cpu.set_reg(2, 0x0000_4000);
        cpu.set_reg(3, 2);
        cpu.execute_arm(0xe120_3281, 0, &mut mem).unwrap(); // smlawb r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 32770);

        cpu.set_reg(5, 0x0003_0000);
        cpu.set_reg(6, 0x4000_0000);
        cpu.execute_arm(0xe124_06e5, 0, &mut mem).unwrap(); // smulwt r4, r5, r6
        assert_eq!(cpu.reg(4), 49152);

        cpu.set_reg(0, 10);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 2);
        cpu.set_reg(3, 3);
        cpu.execute_arm(0xe141_0382, 0, &mut mem).unwrap(); // smlalbb r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 16);
        assert_eq!(cpu.reg(1), 0);

        let err = cpu.execute_arm(0xe00f_0291, 0, &mut mem).unwrap_err(); // mul pc, r1, r2
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe020_f291, 0, &mut mem).unwrap_err(); // mla r0, r1, r2, pc
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe081_039f, 0, &mut mem).unwrap_err(); // umull r0, r1, pc, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe10f_3281, 0, &mut mem).unwrap_err(); // smlabb pc, r1, r2, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe141_f382, 0, &mut mem).unwrap_err(); // smlalbb pc, r1, r2, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
    }

    #[test]
    fn armv6_unsigned_sum_absolute_differences_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 0x10_20_30_40);
        cpu.set_reg(2, 0x18_10_28_50);
        cpu.execute_arm(0xe780_f211, 0, &mut mem).unwrap(); // usad8 r0, r1, r2
        assert_eq!(cpu.reg(0), 8 + 16 + 8 + 16);

        cpu.set_reg(3, 5);
        cpu.execute_arm(0xe780_3211, 0, &mut mem).unwrap(); // usada8 r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 5 + 8 + 16 + 8 + 16);
    }

    #[test]
    fn armv6_dsp_multiply_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(1, 0x0002_0003);
        cpu.set_reg(2, 0x0005_0007);
        cpu.set_reg(3, 10);
        cpu.execute_arm(0xe700_3211, 0, &mut mem).unwrap(); // smlad r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 41);

        cpu.set_reg(5, 0x0002_0003);
        cpu.set_reg(6, 0x0005_0007);
        cpu.set_reg(7, 1);
        cpu.execute_arm(0xe704_7635, 0, &mut mem).unwrap(); // smladx r4, r5, r6, r7
        assert_eq!(cpu.reg(4), 30);

        cpu.set_reg(1, 0x0002_0003);
        cpu.set_reg(2, 0x0005_0007);
        cpu.set_reg(3, 10);
        cpu.execute_arm(0xe700_3251, 0, &mut mem).unwrap(); // smlsd r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 21);

        cpu.execute_arm(0xe700_f211, 0, &mut mem).unwrap(); // smuad r0, r1, r2
        assert_eq!(cpu.reg(0), 31);
        cpu.set_reg(7, 0x0002_0003);
        cpu.set_reg(8, 0x0005_0007);
        cpu.execute_arm(0xe706_f857, 0, &mut mem).unwrap(); // smusd r6, r7, r8
        assert_eq!(cpu.reg(6), 11);

        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0x0002_0003);
        cpu.set_reg(3, 0x0005_0007);
        cpu.execute_arm(0xe741_0312, 0, &mut mem).unwrap(); // smlald r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 32);
        assert_eq!(cpu.reg(1), 0);

        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0);
        cpu.execute_arm(0xe741_0352, 0, &mut mem).unwrap(); // smlsld r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 12);
        assert_eq!(cpu.reg(1), 0);

        cpu.set_reg(1, 0x4000_0000);
        cpu.set_reg(2, 4);
        cpu.execute_arm(0xe750_f211, 0, &mut mem).unwrap(); // smmul r0, r1, r2
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(4, 0x4000_0000);
        cpu.set_reg(5, 2);
        cpu.execute_arm(0xe753_f534, 0, &mut mem).unwrap(); // smmulr r3, r4, r5
        assert_eq!(cpu.reg(3), 1);

        cpu.set_reg(7, 0x4000_0000);
        cpu.set_reg(8, 4);
        cpu.set_reg(9, 5);
        cpu.execute_arm(0xe756_9817, 0, &mut mem).unwrap(); // smmla r6, r7, r8, r9
        assert_eq!(cpu.reg(6), 6);

        cpu.set_reg(2, 0x4000_0000);
        cpu.set_reg(3, 4);
        cpu.set_reg(4, 5);
        cpu.execute_arm(0xe751_43d2, 0, &mut mem).unwrap(); // smmls r1, r2, r3, r4
        assert_eq!(cpu.reg(1), 4);

        cpu.set_reg(0, 5);
        cpu.set_reg(1, 6);
        cpu.set_reg(2, 3);
        cpu.set_reg(3, 4);
        cpu.execute_arm(0xe041_0392, 0, &mut mem).unwrap(); // umaal r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 23);
        assert_eq!(cpu.reg(1), 0);

        let err = cpu.execute_arm(0xe70f_3211, 0, &mut mem).unwrap_err(); // smlad pc, r1, r2, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe74f_0312, 0, &mut mem).unwrap_err(); // smlald r0, pc, r2, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe751_f3d2, 0, &mut mem).unwrap_err(); // smmls r1, r2, r3, pc
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
        let err = cpu.execute_arm(0xe041_039f, 0, &mut mem).unwrap_err(); // umaal r0, r1, pc, r3
        assert!(matches!(
            err,
            Trap::Unpredictable("multiply with PC register")
        ));
    }

    #[test]
    fn vfpv2_single_register_moves_and_arithmetic_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x200);

        cpu.set_reg(0, 1.5f32.to_bits());
        cpu.execute_arm(0xee00_0a10, 0, &mut mem).unwrap(); // vmov s0, r0
        assert_eq!(f32::from_bits(cpu.sreg(0)), 1.5);

        cpu.set_reg(2, 2.25f32.to_bits());
        cpu.execute_arm(0xee00_2a90, 0, &mut mem).unwrap(); // vmov s1, r2
        assert_eq!(f32::from_bits(cpu.sreg(1)), 2.25);

        cpu.execute_arm(0xee30_1a20, 0, &mut mem).unwrap(); // vadd.f32 s2, s0, s1
        assert_eq!(f32::from_bits(cpu.sreg(2)), 3.75);

        cpu.execute_arm(0xee71_1a60, 0, &mut mem).unwrap(); // vsub.f32 s3, s2, s1
        assert_eq!(f32::from_bits(cpu.sreg(3)), 1.5);

        cpu.execute_arm(0xee21_2a21, 0, &mut mem).unwrap(); // vmul.f32 s4, s2, s3
        assert_eq!(f32::from_bits(cpu.sreg(4)), 5.625);

        cpu.execute_arm(0xeec2_2a00, 0, &mut mem).unwrap(); // vdiv.f32 s5, s4, s0
        assert_eq!(f32::from_bits(cpu.sreg(5)), 3.75);

        cpu.execute_arm(0xeeb1_3a62, 0, &mut mem).unwrap(); // vneg.f32 s6, s5
        assert_eq!(f32::from_bits(cpu.sreg(6)), -3.75);

        cpu.execute_arm(0xeef0_3ac3, 0, &mut mem).unwrap(); // vabs.f32 s7, s6
        assert_eq!(f32::from_bits(cpu.sreg(7)), 3.75);

        cpu.set_sreg(0, 10.0f32.to_bits());
        cpu.set_sreg(1, 2.0f32.to_bits());
        cpu.set_sreg(2, 3.0f32.to_bits());
        cpu.execute_arm(0xee00_0a81, 0, &mut mem).unwrap(); // vmla.f32 s0, s1, s2
        assert_eq!(f32::from_bits(cpu.sreg(0)), 16.0);

        cpu.set_sreg(3, 20.0f32.to_bits());
        cpu.set_sreg(4, 4.0f32.to_bits());
        cpu.set_sreg(5, 1.5f32.to_bits());
        cpu.execute_arm(0xee42_1a62, 0, &mut mem).unwrap(); // vmls.f32 s3, s4, s5
        assert_eq!(f32::from_bits(cpu.sreg(3)), 14.0);

        cpu.set_sreg(6, 2.0f32.to_bits());
        cpu.set_sreg(7, 3.0f32.to_bits());
        cpu.set_sreg(8, 4.0f32.to_bits());
        cpu.execute_arm(0xee13_3ac4, 0, &mut mem).unwrap(); // vnmla.f32 s6, s7, s8
        assert_eq!(f32::from_bits(cpu.sreg(6)), -14.0);

        cpu.set_sreg(9, 20.0f32.to_bits());
        cpu.set_sreg(10, 3.0f32.to_bits());
        cpu.set_sreg(11, 4.0f32.to_bits());
        cpu.execute_arm(0xee55_4a25, 0, &mut mem).unwrap(); // vnmls.f32 s9, s10, s11
        assert_eq!(f32::from_bits(cpu.sreg(9)), -8.0);

        cpu.set_sreg(13, 2.0f32.to_bits());
        cpu.set_sreg(14, 5.0f32.to_bits());
        cpu.execute_arm(0xee26_6ac7, 0, &mut mem).unwrap(); // vnmul.f32 s12, s13, s14
        assert_eq!(f32::from_bits(cpu.sreg(12)), -10.0);

        cpu.set_sreg(16, 9.0f32.to_bits());
        cpu.execute_arm(0xeef1_7ac8, 0, &mut mem).unwrap(); // vsqrt.f32 s15, s16
        assert_eq!(f32::from_bits(cpu.sreg(15)), 3.0);

        cpu.set_sreg(18, 6.25f32.to_bits());
        cpu.execute_arm(0xeef0_8a49, 0, &mut mem).unwrap(); // vmov.f32 s17, s18
        assert_eq!(f32::from_bits(cpu.sreg(17)), 6.25);

        cpu.set_sreg(19, 0.0f32.to_bits());
        cpu.execute_arm(0xeef5_9a40, 0, &mut mem).unwrap(); // vcmp.f32 s19, #0.0
        cpu.execute_arm(0xeef1_fa10, 0, &mut mem).unwrap(); // vmrs APSR_nzcv, fpscr
        assert!(!cpu.cpsr.n);
        assert!(cpu.cpsr.z);
        assert!(cpu.cpsr.c);
        assert!(!cpu.cpsr.v);

        cpu.set_sreg(4, 0x1111_2222);
        cpu.set_sreg(5, 0x3333_4444);
        cpu.execute_arm(0xec51_0a12, 0, &mut mem).unwrap(); // vmov r0, r1, s4, s5
        assert_eq!(cpu.reg(0), 0x1111_2222);
        assert_eq!(cpu.reg(1), 0x3333_4444);

        cpu.set_reg(2, 0x5555_6666);
        cpu.set_reg(3, 0x7777_8888);
        cpu.execute_arm(0xec43_2a13, 0, &mut mem).unwrap(); // vmov s6, s7, r2, r3
        assert_eq!(cpu.sreg(6), 0x5555_6666);
        assert_eq!(cpu.sreg(7), 0x7777_8888);

        cpu.set_reg(0, 0x100);
        mem.store32(0x100, 6.5f32.to_bits()).unwrap();
        cpu.execute_arm(0xed90_0a00, 0, &mut mem).unwrap(); // vldr s0, [r0]
        assert_eq!(f32::from_bits(cpu.sreg(0)), 6.5);

        cpu.set_sreg(1, 7.25f32.to_bits());
        cpu.execute_arm(0xedc0_0a01, 0, &mut mem).unwrap(); // vstr s1, [r0, #4]
        assert_eq!(f32::from_bits(mem.load32(0x104).unwrap()), 7.25);

        cpu.execute_arm(0xeeb4_0a60, 0, &mut mem).unwrap(); // vcmp.f32 s0, s1
        cpu.execute_arm(0xeef1_fa10, 0, &mut mem).unwrap(); // vmrs APSR_nzcv, fpscr
        assert!(cpu.cpsr.n);
        assert!(!cpu.cpsr.z);
        assert!(!cpu.cpsr.c);
        assert!(!cpu.cpsr.v);

        cpu.set_reg(2, 0xf000_0000);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.execute_arm(0xeef1_1a10, 0, &mut mem).unwrap(); // vmrs r1, fpscr
        assert_eq!(cpu.reg(1), 0xf000_0000);

        cpu.execute_arm(0xeef0_0a10, 0, &mut mem).unwrap(); // vmrs r0, fpsid
        assert_eq!(cpu.reg(0), VFP_FPSID_ARM1136);
        cpu.set_reg(2, 0xffff_ffff);
        cpu.execute_arm(0xeee0_2a10, 0, &mut mem).unwrap(); // vmsr fpsid, r2
        cpu.execute_arm(0xeef0_0a10, 0, &mut mem).unwrap(); // vmrs r0, fpsid
        assert_eq!(cpu.reg(0), VFP_FPSID_ARM1136);

        let err = cpu.execute_arm(0xee00_fa10, 0x20, &mut mem).unwrap_err(); // vmov s0, pc
        assert!(matches!(err, Trap::Unpredictable("VFP VMOV from PC")));
        let err = cpu.execute_arm(0xee10_fa10, 0x24, &mut mem).unwrap_err(); // vmov pc, s0
        assert!(matches!(err, Trap::Unpredictable("VFP VMOV to PC")));
        let err = cpu.execute_arm(0xeee1_fa10, 0x28, &mut mem).unwrap_err(); // vmsr fpscr, pc
        assert!(matches!(err, Trap::Unpredictable("VFP VMSR from PC")));
        let err = cpu.execute_arm(0xeef0_fa10, 0x2c, &mut mem).unwrap_err(); // vmrs pc, fpsid
        assert!(matches!(
            err,
            Trap::Unpredictable("VFP VMRS non-FPSCR to PC")
        ));
        let err = cpu.execute_arm(0xeef8_0a10, 0x30, &mut mem).unwrap_err(); // vmrs r0, fpexc
        assert!(matches!(
            err,
            Trap::Privileged {
                operation: "VMRS FPEXC",
                ..
            }
        ));

        cpu.set_sreg(2, 3.75f32.to_bits());
        cpu.execute_arm(0xeefd_1ac1, 0, &mut mem).unwrap(); // vcvt.s32.f32 s3, s2
        assert_eq!(cpu.sreg(3), 3);

        cpu.set_reg(2, 0);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_sreg(2, 2.5f32.to_bits());
        cpu.execute_arm(0xeefd_1a41, 0, &mut mem).unwrap(); // vcvtr.s32.f32 s3, s2
        assert_eq!(cpu.sreg(3), 2);

        cpu.set_sreg(2, 3.5f32.to_bits());
        cpu.execute_arm(0xeefd_1a41, 0, &mut mem).unwrap(); // vcvtr.s32.f32 s3, s2
        assert_eq!(cpu.sreg(3), 4);

        cpu.set_reg(2, 1 << 22);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_sreg(2, 2.25f32.to_bits());
        cpu.execute_arm(0xeebc_2a41, 0, &mut mem).unwrap(); // vcvtr.u32.f32 s4, s2
        assert_eq!(cpu.sreg(4), 3);

        cpu.set_reg(2, 2 << 22);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_sreg(2, (-2.25f32).to_bits());
        cpu.execute_arm(0xeefd_1a41, 0, &mut mem).unwrap(); // vcvtr.s32.f32 s3, s2
        assert_eq!(cpu.sreg(3) as i32, -3);

        cpu.execute_arm(0xeef8_2ae1, 0, &mut mem).unwrap(); // vcvt.f32.s32 s5, s3
        assert_eq!(f32::from_bits(cpu.sreg(5)), -3.0);

        cpu.set_sreg(2, 3.75f32.to_bits());
        cpu.execute_arm(0xeebc_2ac1, 0, &mut mem).unwrap(); // vcvt.u32.f32 s4, s2
        assert_eq!(cpu.sreg(4), 3);

        cpu.execute_arm(0xeeb8_3a42, 0, &mut mem).unwrap(); // vcvt.f32.u32 s6, s4
        assert_eq!(f32::from_bits(cpu.sreg(6)), 3.0);

        cpu.execute_arm(0xee10_1a10, 0, &mut mem).unwrap(); // vmov r1, s0
        assert_eq!(cpu.reg(1), 6.5f32.to_bits());
    }

    #[test]
    fn vfpv2_short_vector_arithmetic_and_unary() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.fpscr = 1 << 16; // LEN=2, stride=1.
        cpu.set_sreg(16, 1.0f32.to_bits());
        cpu.set_sreg(17, 2.0f32.to_bits());
        cpu.set_sreg(24, 10.0f32.to_bits());
        cpu.set_sreg(25, 20.0f32.to_bits());
        cpu.execute_arm(0xee38_4a0c, 0, &mut mem).unwrap(); // vadd.f32 s8, s16, s24
        assert_eq!(f32::from_bits(cpu.sreg(8)), 11.0);
        assert_eq!(f32::from_bits(cpu.sreg(9)), 22.0);

        cpu.set_sreg(0, (-3.5f32).to_bits());
        cpu.execute_arm(0xeeb0_5ac0, 0, &mut mem).unwrap(); // vabs.f32 s10, s0
        assert_eq!(f32::from_bits(cpu.sreg(10)), 3.5);
        assert_eq!(f32::from_bits(cpu.sreg(11)), 3.5);

        cpu.fpscr = (1 << 16) | (3 << 20); // LEN=2, stride=2.
        cpu.set_sreg(16, 1.0f32.to_bits());
        cpu.set_sreg(18, 3.0f32.to_bits());
        cpu.set_sreg(24, 10.0f32.to_bits());
        cpu.set_sreg(26, 30.0f32.to_bits());
        cpu.execute_arm(0xee38_4a0c, 0, &mut mem).unwrap(); // vadd.f32 s8, s16, s24
        assert_eq!(f32::from_bits(cpu.sreg(8)), 11.0);
        assert_eq!(f32::from_bits(cpu.sreg(10)), 33.0);

        cpu.fpscr = 1 << 16; // LEN=2, stride=1.
        cpu.set_dreg(8, 1.5f64.to_bits());
        cpu.set_dreg(9, 2.5f64.to_bits());
        cpu.set_dreg(12, 10.0f64.to_bits());
        cpu.set_dreg(13, 20.0f64.to_bits());
        cpu.execute_arm(0xee38_4b0c, 0, &mut mem).unwrap(); // vadd.f64 d4, d8, d12
        assert_eq!(f64::from_bits(cpu.dreg(4)), 11.5);
        assert_eq!(f64::from_bits(cpu.dreg(5)), 22.5);

        cpu.fpscr = 3 << 20;
        let err = cpu.execute_arm(0xee38_4a0c, 0, &mut mem).unwrap_err(); // vadd.f32 s8, s16, s24
        assert_eq!(err, Trap::Unpredictable("invalid VFP vector stride"));

        cpu.fpscr = (4 << 16) | (3 << 20);
        let err = cpu.execute_arm(0xee38_4a0c, 0, &mut mem).unwrap_err(); // vadd.f32 s8, s16, s24
        assert_eq!(err, Trap::Unpredictable("invalid VFP vector length/stride"));
    }

    #[test]
    fn vfpv2_compare_and_conversion_remain_scalar_in_vector_mode() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.fpscr = 1 << 16; // LEN=2, stride=1.
        cpu.set_sreg(8, 1.0f32.to_bits());
        cpu.set_sreg(9, 2.0f32.to_bits());
        cpu.execute_arm(0xeeb4_4a64, 0, &mut mem).unwrap(); // vcmp.f32 s8, s9
        cpu.execute_arm(0xeef1_fa10, 0, &mut mem).unwrap(); // vmrs APSR_nzcv, fpscr
        assert!(cpu.cpsr.n);
        assert!(!cpu.cpsr.z);
        assert!(!cpu.cpsr.c);
        assert!(!cpu.cpsr.v);

        cpu.set_sreg(10, 0xaaaa_aaaa);
        cpu.set_sreg(11, 0xbbbb_bbbb);
        cpu.set_sreg(16, 3.5f32.to_bits());
        cpu.set_sreg(17, 9.5f32.to_bits());
        cpu.execute_arm(0xeebd_5ac8, 0, &mut mem).unwrap(); // vcvt.s32.f32 s10, s16
        assert_eq!(cpu.sreg(10), 3);
        assert_eq!(cpu.sreg(11), 0xbbbb_bbbb);

        cpu.fpscr = (1 << 16) | (1 << 22); // LEN=2, round toward plus infinity.
        cpu.set_sreg(12, 0xcccc_cccc);
        cpu.set_sreg(13, 0xdddd_dddd);
        cpu.set_sreg(18, 2.25f32.to_bits());
        cpu.set_sreg(19, 7.25f32.to_bits());
        cpu.execute_arm(0xeebc_6a49, 0, &mut mem).unwrap(); // vcvtr.u32.f32 s12, s18
        assert_eq!(cpu.sreg(12), 3);
        assert_eq!(cpu.sreg(13), 0xdddd_dddd);

        cpu.set_sreg(14, 0xeeee_eeee);
        cpu.set_sreg(15, 0xffff_ffff);
        cpu.set_dreg(8, 2.5f64.to_bits());
        cpu.set_dreg(9, 7.5f64.to_bits());
        cpu.execute_arm(0xeeb7_7bc8, 0, &mut mem).unwrap(); // vcvt.f32.f64 s14, d8
        assert_eq!(f32::from_bits(cpu.sreg(14)), 2.5);
        assert_eq!(cpu.sreg(15), 0xffff_ffff);
    }

    #[test]
    fn vfpv2_basic_exception_flags_are_cumulative() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_sreg(0, 1.0f32.to_bits());
        cpu.set_sreg(1, 0.0f32.to_bits());
        cpu.execute_arm(0xee80_1a20, 0, &mut mem).unwrap(); // vdiv.f32 s2, s0, s1
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_DZC);

        cpu.set_sreg(6, (-1.0f32).to_bits());
        cpu.execute_arm(0xeeb1_2ac3, 0, &mut mem).unwrap(); // vsqrt.f32 s4, s6
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_IOC | FPSCR_DZC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0.0f32.to_bits());
        cpu.set_sreg(1, 0.0f32.to_bits());
        cpu.execute_arm(0xee80_1a20, 0, &mut mem).unwrap(); // vdiv.f32 s2, s0, s1
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 1.0f64.to_bits());
        cpu.set_dreg(1, 0.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_DZC);

        cpu.set_dreg(6, (-1.0f64).to_bits());
        cpu.execute_arm(0xeeb1_4bc6, 0, &mut mem).unwrap(); // vsqrt.f64 d4, d6
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_IOC | FPSCR_DZC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 0.0f64.to_bits());
        cpu.set_dreg(1, 0.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_DZC), FPSCR_IOC);
    }

    #[test]
    fn vfpv2_single_arithmetic_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_sreg(0, 1.0f32.to_bits());
        cpu.set_sreg(1, (2.0f32).powi(-25).to_bits());
        cpu.execute_arm(0xee30_1a20, 0, &mut mem).unwrap(); // vadd.f32 s2, s0, s1
        assert_eq!(f32::from_bits(cpu.sreg(2)), 1.0);
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, f32::MAX.to_bits());
        cpu.set_sreg(1, 2.0f32.to_bits());
        cpu.execute_arm(0xee20_1a20, 0, &mut mem).unwrap(); // vmul.f32 s2, s0, s1
        assert_eq!(f32::from_bits(cpu.sreg(2)), f32::INFINITY);
        assert_eq!(cpu.fpscr & mask, FPSCR_OFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, f32::MIN_POSITIVE.to_bits());
        cpu.set_sreg(1, 0.5f32.to_bits());
        cpu.execute_arm(0xee20_1a20, 0, &mut mem).unwrap(); // vmul.f32 s2, s0, s1
        assert_eq!(cpu.sreg(2), 0x0040_0000);
        assert_eq!(cpu.fpscr & mask, 0);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 1);
        cpu.set_sreg(1, 0.5f32.to_bits());
        cpu.execute_arm(0xee20_1a20, 0, &mut mem).unwrap(); // vmul.f32 s2, s0, s1
        assert_eq!(cpu.sreg(2), 0);
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 1.0f32.to_bits());
        cpu.set_sreg(1, 3.0f32.to_bits());
        cpu.execute_arm(0xee80_1a20, 0, &mut mem).unwrap(); // vdiv.f32 s2, s0, s1
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(16, 2.0f32.to_bits());
        cpu.execute_arm(0xeef1_7ac8, 0, &mut mem).unwrap(); // vsqrt.f32 s15, s16
        assert_eq!(f32::from_bits(cpu.sreg(15)), 2.0f32.sqrt());
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, f32::INFINITY.to_bits());
        cpu.set_sreg(1, f32::NEG_INFINITY.to_bits());
        cpu.execute_arm(0xee30_1a20, 0, &mut mem).unwrap(); // vadd.f32 s2, s0, s1
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 1.0f32.to_bits());
        cpu.set_sreg(1, 1.0f32.to_bits());
        cpu.set_sreg(2, (2.0f32).powi(-25).to_bits());
        cpu.execute_arm(0xee00_0a81, 0, &mut mem).unwrap(); // vmla.f32 s0, s1, s2
        assert_eq!(f32::from_bits(cpu.sreg(0)), 1.0);
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);
    }

    #[test]
    fn vfpv2_double_invalid_arithmetic_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC;

        cpu.set_dreg(0, f64::INFINITY.to_bits());
        cpu.set_dreg(1, f64::NEG_INFINITY.to_bits());
        cpu.execute_arm(0xee30_2b01, 0, &mut mem).unwrap(); // vadd.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 0.0f64.to_bits());
        cpu.set_dreg(1, f64::INFINITY.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::INFINITY.to_bits());
        cpu.set_dreg(1, f64::INFINITY.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(6, 0x7ff0_0000_0000_0001);
        cpu.execute_arm(0xeeb1_4bc6, 0, &mut mem).unwrap(); // vsqrt.f64 d4, d6
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(6, 1.0f64.to_bits());
        cpu.set_dreg(7, 0.0f64.to_bits());
        cpu.set_dreg(8, f64::INFINITY.to_bits());
        cpu.execute_arm(0xee07_6b08, 0, &mut mem).unwrap(); // vmla.f64 d6, d7, d8
        assert_eq!(cpu.fpscr & mask, FPSCR_IOC);
    }

    #[test]
    fn vfpv2_double_overflow_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_dreg(0, f64::MAX.to_bits());
        cpu.set_dreg(1, f64::MAX.to_bits());
        cpu.execute_arm(0xee30_2b01, 0, &mut mem).unwrap(); // vadd.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), f64::INFINITY);
        assert_eq!(cpu.fpscr & mask, FPSCR_OFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MAX.to_bits());
        cpu.set_dreg(1, 2.0f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), f64::INFINITY);
        assert_eq!(cpu.fpscr & mask, FPSCR_OFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MAX.to_bits());
        cpu.set_dreg(1, f64::MIN_POSITIVE.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), f64::INFINITY);
        assert_eq!(cpu.fpscr & mask, FPSCR_OFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(6, 1.0f64.to_bits());
        cpu.set_dreg(7, f64::MAX.to_bits());
        cpu.set_dreg(8, 2.0f64.to_bits());
        cpu.execute_arm(0xee07_6b08, 0, &mut mem).unwrap(); // vmla.f64 d6, d7, d8
        assert_eq!(f64::from_bits(cpu.dreg(6)), f64::INFINITY);
        assert_eq!(cpu.fpscr & mask, FPSCR_OFC | FPSCR_IXC);
    }

    #[test]
    fn vfpv2_double_absorbed_inexact_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_dreg(0, 1.0f64.to_bits());
        cpu.set_dreg(1, (f64::EPSILON / 2.0).to_bits());
        cpu.execute_arm(0xee30_2b01, 0, &mut mem).unwrap(); // vadd.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), 1.0);
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 1.0f64.to_bits());
        cpu.set_dreg(1, (f64::EPSILON / 4.0).to_bits());
        cpu.execute_arm(0xee30_2b41, 0, &mut mem).unwrap(); // vsub.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), 1.0);
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);
    }

    #[test]
    fn vfpv2_double_multiply_inexact_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_dreg(0, 1.1f64.to_bits());
        cpu.set_dreg(1, 1.1f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MIN_POSITIVE.to_bits());
        cpu.set_dreg(1, 0.1f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert!(f64::from_bits(cpu.dreg(2)).is_subnormal());
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MIN_POSITIVE.to_bits());
        cpu.set_dreg(1, 0.5f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(cpu.dreg(2), 0x0008_0000_0000_0000);
        assert_eq!(cpu.fpscr & mask, 0);
    }

    #[test]
    fn vfpv2_double_divide_inexact_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_dreg(0, 1.0f64.to_bits());
        cpu.set_dreg(1, 10.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.fpscr & mask, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MIN_POSITIVE.to_bits());
        cpu.set_dreg(1, 10.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert!(f64::from_bits(cpu.dreg(2)).is_subnormal());
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MIN_POSITIVE.to_bits());
        cpu.set_dreg(1, 2.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.dreg(2), 0x0008_0000_0000_0000);
        assert_eq!(cpu.fpscr & mask, 0);
    }

    #[test]
    fn vfpv2_double_zero_underflow_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        let mask = FPSCR_IOC | FPSCR_DZC | FPSCR_OFC | FPSCR_UFC | FPSCR_IXC;

        cpu.set_dreg(0, 1);
        cpu.set_dreg(1, 0.5f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(cpu.dreg(2), 0);
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::MIN_POSITIVE.to_bits());
        cpu.set_dreg(1, 0.5f64.to_bits());
        cpu.execute_arm(0xee20_2b01, 0, &mut mem).unwrap(); // vmul.f64 d2, d0, d1
        assert_eq!(cpu.dreg(2), 0x0008_0000_0000_0000);
        assert_eq!(cpu.fpscr & mask, 0);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 1);
        cpu.set_dreg(1, 2.0f64.to_bits());
        cpu.execute_arm(0xee80_2b01, 0, &mut mem).unwrap(); // vdiv.f64 d2, d0, d1
        assert_eq!(cpu.dreg(2), 0);
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(6, 1.0f64.to_bits());
        cpu.set_dreg(7, 1);
        cpu.set_dreg(8, 0.5f64.to_bits());
        cpu.execute_arm(0xee07_6b08, 0, &mut mem).unwrap(); // vmla.f64 d6, d7, d8
        assert_eq!(f64::from_bits(cpu.dreg(6)), 1.0);
        assert_eq!(cpu.fpscr & mask, FPSCR_UFC | FPSCR_IXC);
    }

    #[test]
    fn vfpv2_conversion_invalid_flags_and_saturation() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_sreg(0, f32::NAN.to_bits());
        cpu.execute_arm(0xeefd_0ac0, 0, &mut mem).unwrap(); // vcvt.s32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, (-1.0f32).to_bits());
        cpu.execute_arm(0xeefc_0ac0, 0, &mut mem).unwrap(); // vcvt.u32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 2_147_483_648.0f32.to_bits());
        cpu.execute_arm(0xeefd_0ac0, 0, &mut mem).unwrap(); // vcvt.s32.f32 s1, s0
        assert_eq!(cpu.sreg(1), i32::MAX as u32);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::NEG_INFINITY.to_bits());
        cpu.execute_arm(0xeefd_0bc0, 0, &mut mem).unwrap(); // vcvt.s32.f64 s1, d0
        assert_eq!(cpu.sreg(1), i32::MIN as u32);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::NAN.to_bits());
        cpu.execute_arm(0xeefc_0bc0, 0, &mut mem).unwrap(); // vcvt.u32.f64 s1, d0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);

        cpu.set_reg(2, 2 << 22); // Round toward minus infinity.
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_sreg(0, (-0.25f32).to_bits());
        cpu.execute_arm(0xeefc_0a40, 0, &mut mem).unwrap(); // vcvtr.u32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);
    }

    #[test]
    fn vfpv2_conversion_inexact_flags_follow_rounding() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_sreg(0, 3.5f32.to_bits());
        cpu.execute_arm(0xeefd_0ac0, 0, &mut mem).unwrap(); // vcvt.s32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 3);
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_IXC), FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, (-0.25f32).to_bits());
        cpu.execute_arm(0xeefc_0ac0, 0, &mut mem).unwrap(); // vcvt.u32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_IXC), FPSCR_IXC);

        cpu.set_reg(2, 2 << 22); // Round toward minus infinity.
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_sreg(0, (-0.25f32).to_bits());
        cpu.execute_arm(0xeefc_0a40, 0, &mut mem).unwrap(); // vcvtr.u32.f32 s1, s0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_IXC), FPSCR_IOC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 2.5f64.to_bits());
        cpu.execute_arm(0xeefd_0b40, 0, &mut mem).unwrap(); // vcvtr.s32.f64 s1, d0
        assert_eq!(cpu.sreg(1), 2);
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_IXC), FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 4.0f64.to_bits());
        cpu.execute_arm(0xeefd_0bc0, 0, &mut mem).unwrap(); // vcvt.s32.f64 s1, d0
        assert_eq!(cpu.sreg(1), 4);
        assert_eq!(cpu.fpscr & (FPSCR_IOC | FPSCR_IXC), 0);
    }

    #[test]
    fn vfpv2_compare_nan_exception_flags_match_vcmp_variant() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_sreg(0, 0x7fc0_0000);
        cpu.set_sreg(1, 1.0f32.to_bits());
        cpu.execute_arm(0xeeb4_0a60, 0, &mut mem).unwrap(); // vcmp.f32 s0, s1
        assert_eq!(cpu.fpscr & FPSCR_IOC, 0);
        assert_eq!(cpu.fpscr & 0xf000_0000, 0x3000_0000);

        cpu.fpscr = 0;
        cpu.execute_arm(0xeeb4_0ae0, 0, &mut mem).unwrap(); // vcmpe.f32 s0, s1
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);
        assert_eq!(cpu.fpscr & 0xf000_0000, 0x3000_0000);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0x7fa0_0000);
        cpu.execute_arm(0xeeb4_0a60, 0, &mut mem).unwrap(); // vcmp.f32 s0, s1
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);
        assert_eq!(cpu.fpscr & 0xf000_0000, 0x3000_0000);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0x7fc0_0000);
        cpu.execute_arm(0xeeb5_0ac0, 0, &mut mem).unwrap(); // vcmpe.f32 s0, #0
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);
        assert_eq!(cpu.fpscr & 0xf000_0000, 0x3000_0000);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 0x7ff8_0000_0000_0000);
        cpu.set_dreg(1, 1.0f64.to_bits());
        cpu.execute_arm(0xeeb4_0bc1, 0, &mut mem).unwrap(); // vcmpe.f64 d0, d1
        assert_eq!(cpu.fpscr & FPSCR_IOC, FPSCR_IOC);
        assert_eq!(cpu.fpscr & 0xf000_0000, 0x3000_0000);
    }

    #[test]
    fn vfpv2_integer_to_single_inexact_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_sreg(0, 0x0100_0001);
        cpu.execute_arm(0xeef8_0ac0, 0, &mut mem).unwrap(); // vcvt.f32.s32 s1, s0
        assert_eq!(f32::from_bits(cpu.sreg(1)), 16_777_216.0);
        assert_eq!(cpu.fpscr & FPSCR_IXC, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0x0100_0001);
        cpu.execute_arm(0xeef8_0a40, 0, &mut mem).unwrap(); // vcvt.f32.u32 s1, s0
        assert_eq!(f32::from_bits(cpu.sreg(1)), 16_777_216.0);
        assert_eq!(cpu.fpscr & FPSCR_IXC, FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0x0100_0000);
        cpu.execute_arm(0xeef8_0ac0, 0, &mut mem).unwrap(); // vcvt.f32.s32 s1, s0
        assert_eq!(f32::from_bits(cpu.sreg(1)), 16_777_216.0);
        assert_eq!(cpu.fpscr & FPSCR_IXC, 0);

        cpu.fpscr = 0;
        cpu.set_sreg(0, 0xffff_ffff);
        cpu.execute_arm(0xeeb8_0b40, 0, &mut mem).unwrap(); // vcvt.f64.u32 d0, s0
        assert_eq!(f64::from_bits(cpu.dreg(0)), f64::from(u32::MAX));
        assert_eq!(cpu.fpscr & FPSCR_IXC, 0);
    }

    #[test]
    fn vfpv2_double_to_single_exception_flags() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_dreg(0, 1.1f64.to_bits());
        cpu.execute_arm(0xeef7_0bc0, 0, &mut mem).unwrap(); // vcvt.f32.f64 s1, d0
        assert_eq!(f32::from_bits(cpu.sreg(1)), 1.1f32);
        assert_eq!(cpu.fpscr & (FPSCR_OFC | FPSCR_UFC | FPSCR_IXC), FPSCR_IXC);

        cpu.fpscr = 0;
        cpu.set_dreg(0, 1.0e40f64.to_bits());
        cpu.execute_arm(0xeef7_0bc0, 0, &mut mem).unwrap(); // vcvt.f32.f64 s1, d0
        assert_eq!(f32::from_bits(cpu.sreg(1)), f32::INFINITY);
        assert_eq!(
            cpu.fpscr & (FPSCR_OFC | FPSCR_UFC | FPSCR_IXC),
            FPSCR_OFC | FPSCR_IXC
        );

        cpu.fpscr = 0;
        cpu.set_dreg(0, 1.0e-50f64.to_bits());
        cpu.execute_arm(0xeef7_0bc0, 0, &mut mem).unwrap(); // vcvt.f32.f64 s1, d0
        assert_eq!(cpu.sreg(1), 0);
        assert_eq!(
            cpu.fpscr & (FPSCR_OFC | FPSCR_UFC | FPSCR_IXC),
            FPSCR_UFC | FPSCR_IXC
        );

        cpu.fpscr = 0;
        cpu.set_dreg(0, f64::from(f32::from_bits(1)).to_bits());
        cpu.execute_arm(0xeef7_0bc0, 0, &mut mem).unwrap(); // vcvt.f32.f64 s1, d0
        assert_eq!(cpu.sreg(1), 1);
        assert_eq!(cpu.fpscr & (FPSCR_OFC | FPSCR_UFC | FPSCR_IXC), 0);
    }

    #[test]
    fn vfpv3_nonbaseline_encodings_trap_undefined() {
        let cases = [
            0xeeb7_0a00, // vmov.f32 s0, #1.0
            0xeeba_1a44, // vcvt.f32.s16 s2, s2, #8
            0xeebe_2a44, // vcvt.s16.f32 s4, s4, #8
        ];

        for (idx, instr) in cases.into_iter().enumerate() {
            let mut cpu = Cpu::new();
            let mut mem = VecMemory::new(0, 4);
            let pc = 0x6000 + (idx as u32 * 4);
            let err = cpu.execute_arm(instr, pc, &mut mem).unwrap_err();
            assert_eq!(err, Trap::UndefinedArm { pc, instr });
        }
    }

    #[test]
    fn vfpv2_double_arithmetic_and_memory_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x200);

        cpu.set_dreg(0, 1.5f64.to_bits());
        cpu.set_dreg(1, 2.25f64.to_bits());
        cpu.execute_arm(0xee30_2b01, 0, &mut mem).unwrap(); // vadd.f64 d2, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), 3.75);

        cpu.execute_arm(0xee32_3b41, 0, &mut mem).unwrap(); // vsub.f64 d3, d2, d1
        assert_eq!(f64::from_bits(cpu.dreg(3)), 1.5);

        cpu.execute_arm(0xee22_4b03, 0, &mut mem).unwrap(); // vmul.f64 d4, d2, d3
        assert_eq!(f64::from_bits(cpu.dreg(4)), 5.625);

        cpu.execute_arm(0xee84_5b00, 0, &mut mem).unwrap(); // vdiv.f64 d5, d4, d0
        assert_eq!(f64::from_bits(cpu.dreg(5)), 3.75);

        cpu.execute_arm(0xeeb1_6b45, 0, &mut mem).unwrap(); // vneg.f64 d6, d5
        assert_eq!(f64::from_bits(cpu.dreg(6)), -3.75);

        cpu.execute_arm(0xeeb0_7bc6, 0, &mut mem).unwrap(); // vabs.f64 d7, d6
        assert_eq!(f64::from_bits(cpu.dreg(7)), 3.75);

        cpu.set_dreg(6, 1.0f64.to_bits());
        cpu.set_dreg(7, 2.0f64.to_bits());
        cpu.set_dreg(8, 3.0f64.to_bits());
        cpu.execute_arm(0xee07_6b08, 0, &mut mem).unwrap(); // vmla.f64 d6, d7, d8
        assert_eq!(f64::from_bits(cpu.dreg(6)), 7.0);

        cpu.set_dreg(10, 16.0f64.to_bits());
        cpu.execute_arm(0xeeb1_9bca, 0, &mut mem).unwrap(); // vsqrt.f64 d9, d10
        assert_eq!(f64::from_bits(cpu.dreg(9)), 4.0);

        cpu.set_dreg(2, 0x1122_3344_5566_7788);
        cpu.execute_arm(0xec51_0b12, 0, &mut mem).unwrap(); // vmov r0, r1, d2
        assert_eq!(cpu.reg(0), 0x5566_7788);
        assert_eq!(cpu.reg(1), 0x1122_3344);

        cpu.set_reg(2, 0xaabb_ccdd);
        cpu.set_reg(3, 0x1234_5678);
        cpu.execute_arm(0xec43_2b13, 0, &mut mem).unwrap(); // vmov d3, r2, r3
        assert_eq!(cpu.dreg(3), 0x1234_5678_aabb_ccdd);

        cpu.set_dreg(4, 0xface_cafe_feed_beef);
        cpu.execute_arm(0xee14_6b10, 0, &mut mem).unwrap(); // vmov.32 r6, d4[0]
        assert_eq!(cpu.reg(6), 0xfeed_beef);

        cpu.execute_arm(0xee34_6b10, 0, &mut mem).unwrap(); // vmov.32 r6, d4[1]
        assert_eq!(cpu.reg(6), 0xface_cafe);

        cpu.set_dreg(5, 0x1111_2222_3333_4444);
        cpu.set_reg(7, 0x9999_aaaa);
        cpu.execute_arm(0xee05_7b10, 0, &mut mem).unwrap(); // vmov.32 d5[0], r7
        assert_eq!(cpu.dreg(5), 0x1111_2222_9999_aaaa);

        cpu.set_reg(7, 0xbbbb_cccc);
        cpu.execute_arm(0xee25_7b10, 0, &mut mem).unwrap(); // vmov.32 d5[1], r7
        assert_eq!(cpu.dreg(5), 0xbbbb_cccc_9999_aaaa);

        cpu.set_reg(0, 0x100);
        mem.store32(0x100, 8.5f64.to_bits() as u32).unwrap();
        mem.store32(0x104, (8.5f64.to_bits() >> 32) as u32).unwrap();
        cpu.execute_arm(0xed90_0b00, 0, &mut mem).unwrap(); // vldr d0, [r0]
        assert_eq!(f64::from_bits(cpu.dreg(0)), 8.5);

        cpu.set_dreg(1, 9.25f64.to_bits());
        cpu.execute_arm(0xed80_1b02, 0, &mut mem).unwrap(); // vstr d1, [r0, #8]
        let lo = u64::from(mem.load32(0x108).unwrap());
        let hi = u64::from(mem.load32(0x10c).unwrap());
        assert_eq!(f64::from_bits(lo | (hi << 32)), 9.25);

        cpu.execute_arm(0xeeb4_0b41, 0, &mut mem).unwrap(); // vcmp.f64 d0, d1
        cpu.execute_arm(0xeef1_fa10, 0, &mut mem).unwrap(); // vmrs APSR_nzcv, fpscr
        assert!(cpu.cpsr.n);
        assert!(!cpu.cpsr.z);
        assert!(!cpu.cpsr.c);
        assert!(!cpu.cpsr.v);

        cpu.set_sreg(1, 1.25f32.to_bits());
        cpu.execute_arm(0xeeb7_0ae0, 0, &mut mem).unwrap(); // vcvt.f64.f32 d0, s1
        assert_eq!(f64::from_bits(cpu.dreg(0)), 1.25);

        cpu.set_dreg(1, 2.5f64.to_bits());
        cpu.execute_arm(0xeeb7_1bc1, 0, &mut mem).unwrap(); // vcvt.f32.f64 s2, d1
        assert_eq!(f32::from_bits(cpu.sreg(2)), 2.5);

        cpu.set_dreg(2, 4.75f64.to_bits());
        cpu.execute_arm(0xeebd_4bc2, 0, &mut mem).unwrap(); // vcvt.s32.f64 s8, d2
        assert_eq!(cpu.sreg(8), 4);

        cpu.set_reg(2, 0);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_dreg(2, 2.5f64.to_bits());
        cpu.execute_arm(0xeebd_4b42, 0, &mut mem).unwrap(); // vcvtr.s32.f64 s8, d2
        assert_eq!(cpu.sreg(8), 2);

        cpu.set_reg(2, 1 << 22);
        cpu.execute_arm(0xeee1_2a10, 0, &mut mem).unwrap(); // vmsr fpscr, r2
        cpu.set_dreg(2, 2.25f64.to_bits());
        cpu.execute_arm(0xeefc_4b42, 0, &mut mem).unwrap(); // vcvtr.u32.f64 s9, d2
        assert_eq!(cpu.sreg(9), 3);

        cpu.set_sreg(8, 4);
        cpu.execute_arm(0xeeb8_3bc4, 0, &mut mem).unwrap(); // vcvt.f64.s32 d3, s8
        assert_eq!(f64::from_bits(cpu.dreg(3)), 4.0);
    }

    #[test]
    fn vfpv2_double_register_ranges_trap() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_dreg(13, 2.0f64.to_bits());
        cpu.set_dreg(14, 1.0f64.to_bits());
        cpu.execute_arm(0xee3e_fb0d, 0, &mut mem).unwrap(); // vadd.f64 d15, d14, d13
        assert_eq!(f64::from_bits(cpu.dreg(15)), 3.0);

        for (instr, comment) in [
            (0xec43_2b30, "vmov d16, r2, r3"),
            (0xec51_0b30, "vmov r0, r1, d16"),
            (0xee00_7b90, "vmov.32 d16[0], r7"),
            (0xee10_6b90, "vmov.32 r6, d16[0]"),
            (0xee70_0b01, "vadd.f64 d16, d0, d1"),
            (0xee30_2b81, "vadd.f64 d2, d16, d1"),
            (0xee30_2b20, "vadd.f64 d2, d0, d16"),
            (0xeef4_0b41, "vcmp.f64 d16, d1"),
            (0xeeb4_0b60, "vcmp.f64 d0, d16"),
            (0xeef7_0ae0, "vcvt.f64.f32 d16, s1"),
            (0xeeb7_1be0, "vcvt.f32.f64 s2, d16"),
            (0xeef8_0bc4, "vcvt.f64.s32 d16, s8"),
        ] {
            let err = cpu.execute_arm(instr, 0, &mut mem).unwrap_err();
            assert_eq!(
                err,
                Trap::Unpredictable("VFP double register out of range"),
                "{comment}"
            );
        }
    }

    #[test]
    fn vfpv2_multiple_transfer_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x300);

        cpu.set_reg(13, 0x200);
        for idx in 0..4 {
            cpu.set_sreg(idx, (idx as f32 + 1.0).to_bits());
        }
        cpu.execute_arm(0xed2d_0a04, 0, &mut mem).unwrap(); // vpush {s0-s3}
        assert_eq!(cpu.reg(13), 0x1f0);
        for idx in 0..4 {
            cpu.set_sreg(idx, 0);
        }
        cpu.execute_arm(0xecbd_0a04, 0, &mut mem).unwrap(); // vpop {s0-s3}
        assert_eq!(cpu.reg(13), 0x200);
        for idx in 0..4 {
            assert_eq!(f32::from_bits(cpu.sreg(idx)), idx as f32 + 1.0);
        }

        cpu.set_reg(1, 0x120);
        for idx in 0..4 {
            cpu.set_sreg(idx, (idx as f32 + 10.0).to_bits());
        }
        cpu.execute_arm(0xec81_0a04, 0, &mut mem).unwrap(); // vstmia r1, {s0-s3}
        assert_eq!(cpu.reg(1), 0x120);
        for idx in 0..4 {
            cpu.set_sreg(idx, 0);
        }
        cpu.execute_arm(0xec91_0a04, 0, &mut mem).unwrap(); // vldmia r1, {s0-s3}
        assert_eq!(cpu.reg(1), 0x120);
        for idx in 0..4 {
            assert_eq!(f32::from_bits(cpu.sreg(idx)), idx as f32 + 10.0);
        }

        cpu.set_reg(0, 0x100);
        cpu.set_dreg(0, 1.25f64.to_bits());
        cpu.set_dreg(1, 2.5f64.to_bits());
        cpu.execute_arm(0xeca0_0b04, 0, &mut mem).unwrap(); // vstmia r0!, {d0-d1}
        assert_eq!(cpu.reg(0), 0x110);
        cpu.set_dreg(0, 0);
        cpu.set_dreg(1, 0);
        cpu.set_reg(0, 0x100);
        cpu.execute_arm(0xecb0_0b04, 0, &mut mem).unwrap(); // vldmia r0!, {d0-d1}
        assert_eq!(cpu.reg(0), 0x110);
        assert_eq!(f64::from_bits(cpu.dreg(0)), 1.25);
        assert_eq!(f64::from_bits(cpu.dreg(1)), 2.5);
    }

    #[test]
    fn vfpv2_load_store_invalid_ranges_trap() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);
        cpu.set_reg(0, 0x40);

        let err = cpu.execute_arm(0xedd0_0b00, 0, &mut mem).unwrap_err(); // vldr d16, [r0]
        assert_eq!(err, Trap::Unpredictable("VFP double register out of range"));

        let err = cpu.execute_arm(0xeca0_0a00, 0, &mut mem).unwrap_err(); // vstmia r0!, {}
        assert_eq!(
            err,
            Trap::Unpredictable("VFP multiple transfer empty register list")
        );

        let err = cpu.execute_arm(0xecaf_0a01, 0, &mut mem).unwrap_err(); // vstmia pc!, {s0}
        assert_eq!(
            err,
            Trap::Unpredictable("VFP multiple transfer writeback with PC base")
        );

        let err = cpu.execute_arm(0xece0_0b04, 0, &mut mem).unwrap_err(); // vstmia r0!, {d16-d17}
        assert_eq!(
            err,
            Trap::Unpredictable("VFP multiple transfer register range")
        );

        let err = cpu.execute_arm(0xece0_fa02, 0, &mut mem).unwrap_err(); // vstmia r0!, {s31-s32}
        assert_eq!(
            err,
            Trap::Unpredictable("VFP multiple transfer register range")
        );
    }

    #[test]
    fn reports_undefined_for_unimplemented_thumb_control_instruction() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);
        cpu.set_isa(Isa::Thumb);
        let err = cpu.execute_thumb(0xde00, 0, &mut mem).unwrap_err();
        assert!(matches!(err, Trap::UndefinedThumb { .. }));
    }
}
