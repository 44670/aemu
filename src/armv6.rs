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
struct ThumbItState {
    conds: [u32; 4],
    len: usize,
    pos: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExclusiveReservation {
    addr: u32,
    size: u8,
}

#[derive(Debug, Clone)]
pub struct Cpu {
    regs: [u32; 16],
    sregs: [u32; 64],
    pub fpscr: u32,
    pub cp15_tpidrurw: u32,
    pub cp15_tpidruro: u32,
    cp15_virtual_counter: u64,
    pub cpsr: Cpsr,
    thumb_bl_prefix: Option<u32>,
    thumb_it: Option<ThumbItState>,
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
            sregs: [0; 64],
            fpscr: 0,
            cp15_tpidrurw: 0,
            cp15_tpidruro: 0,
            cp15_virtual_counter: 1,
            cpsr: Cpsr::default(),
            thumb_bl_prefix: None,
            thumb_it: None,
            exclusive_reservation: None,
        }
    }

    pub fn isa(&self) -> Isa {
        if self.cpsr.t { Isa::Thumb } else { Isa::Arm }
    }

    pub fn set_isa(&mut self, isa: Isa) {
        self.cpsr.t = isa == Isa::Thumb;
        if isa == Isa::Arm {
            self.thumb_it = None;
        }
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
        if !self.cpsr.t {
            self.thumb_it = None;
        }
        self.regs[15] = target & !1;
    }

    pub fn step<M: Memory>(&mut self, mem: &mut M) -> Result<()> {
        let pc = self.regs[15];
        if self.cpsr.t {
            let instr = mem.load16(pc)?;
            if thumb_is_32bit_prefix(instr) {
                let second = mem.load16(pc.wrapping_add(2))?;
                self.regs[15] = pc.wrapping_add(4);
                if let Some(cond) = self.consume_thumb_it_condition() {
                    if !condition_passed(cond, self.cpsr) {
                        return Ok(());
                    }
                }
                self.execute_thumb32(instr, second, pc, mem)
            } else {
                self.regs[15] = pc.wrapping_add(2);
                if (instr & 0xff00) != 0xbf00 {
                    if let Some(cond) = self.consume_thumb_it_condition() {
                        if !condition_passed(cond, self.cpsr) {
                            return Ok(());
                        }
                    }
                }
                self.execute_thumb(instr, pc, mem)
            }
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
            if self.exec_arm_neon(instr, pc, mem)? {
                return Ok(());
            }
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

    fn exec_arm_neon<M: Memory>(&mut self, instr: u32, pc: u32, mem: &mut M) -> Result<bool> {
        if (instr & 0xff90_0000) == 0xf400_0000 {
            return self.exec_arm_neon_load_store_multiple(instr, pc, mem);
        }

        if (instr & 0xffb0_0c00) == 0xf4a0_0c00 {
            return self.exec_arm_neon_load_all_lanes(instr, pc, mem);
        }

        if (instr & 0xff90_0000) == 0xf480_0000 {
            return self.exec_arm_neon_load_store_single(instr, pc, mem);
        }

        if (instr & 0x0e80_0000) == 0x0200_0000 {
            return self.exec_arm_neon_3same(instr, pc);
        }

        if (instr & 0x0e80_0050) == 0x0280_0000 {
            if self.exec_arm_neon_3diff(instr, pc)? {
                return Ok(true);
            }
        }

        if (instr & 0xfeb8_0090) == 0xf280_0010 {
            return self.exec_arm_neon_modified_immediate(instr, pc);
        }

        if (instr & 0xfe80_0010) == 0xf280_0010 {
            return self.exec_arm_neon_2reg_shift(instr, pc);
        }

        if (instr & 0xffb0_0010) == 0xf2b0_0000 {
            return self.exec_arm_neon_vext(instr, pc);
        }

        if (instr & 0xffb0_0c10) == 0xf3b0_0800 {
            return self.exec_arm_neon_table_lookup(instr, pc);
        }

        if (instr & 0xffb0_0f90) == 0xf3b0_0c00 {
            return self.exec_arm_neon_vdup_scalar(instr, pc);
        }

        if (instr & 0xffb0_0810) == 0xf3b0_0000 {
            return self.exec_arm_neon_2reg_misc(instr, pc);
        }

        if (instr & 0xfe80_0050) == 0xf280_0040 {
            if self.exec_arm_neon_2reg_scalar(instr, pc)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn exec_arm_neon_load_store_multiple<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<bool> {
        let load = instr & (1 << 21) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let vd = vfp_double_d(instr);
        let itype = ((instr >> 8) & 0xf) as u8;
        let size = ((instr >> 6) & 0x3) as usize;
        let align = ((instr >> 4) & 0x3) as u8;
        let rm = (instr & 0xf) as usize;

        let Some((nelem, regs, inc)) = neon_decode_ls_multiple(itype, size, align) else {
            return Err(Trap::UndefinedArm { pc, instr });
        };
        let d_last = vd + inc * (nelem - 1);
        if rn == 15 || d_last + regs > 32 {
            return Err(Trap::Unpredictable(
                "NEON multiple structure transfer register range",
            ));
        }

        let ebytes = 1usize << size;
        let elements = 8 / ebytes;
        let mut address = self.regs[rn];

        if load {
            for r in 0..regs {
                for i in 0..nelem {
                    self.set_dreg_bits(vd + i * inc + r, 0);
                }
            }
            for r in 0..regs {
                for e in 0..elements {
                    for i in 0..nelem {
                        let ext_reg = vd + i * inc + r;
                        let element = load_neon_memory_elem(mem, address, ebytes)?;
                        let shift = e * ebytes * 8;
                        let value = self.dreg_bits(ext_reg) | (element << shift);
                        self.set_dreg_bits(ext_reg, value);
                        address = address.wrapping_add(ebytes as u32);
                    }
                }
            }
        } else {
            for r in 0..regs {
                for e in 0..elements {
                    for i in 0..nelem {
                        let ext_reg = vd + i * inc + r;
                        let element =
                            (self.dreg_bits(ext_reg) >> (e * ebytes * 8)) & neon_byte_mask(ebytes);
                        store_neon_memory_elem(mem, address, ebytes, element)?;
                        address = address.wrapping_add(ebytes as u32);
                    }
                }
            }
        }

        if rm != 15 {
            let increment = if rm == 13 {
                (8 * nelem * regs) as u32
            } else {
                self.regs[rm]
            };
            self.regs[rn] = self.regs[rn].wrapping_add(increment);
        }
        Ok(true)
    }

    fn exec_arm_neon_load_all_lanes<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<bool> {
        let rn = ((instr >> 16) & 0xf) as usize;
        let vd = vfp_double_d(instr);
        let nregs = ((instr >> 8) & 0x3) as usize + 1;
        let mut size = ((instr >> 6) & 0x3) as usize;
        let stride = if instr & (1 << 5) != 0 { 2 } else { 1 };
        let align = instr & (1 << 4) != 0;
        let rm = (instr & 0xf) as usize;

        if size == 3 {
            if nregs != 4 || !align {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            size = 2;
        } else if align {
            match nregs {
                1 if size == 0 => return Err(Trap::UndefinedArm { pc, instr }),
                3 => return Err(Trap::UndefinedArm { pc, instr }),
                _ => {}
            }
        }

        let last_reg = if nregs == 1 && stride == 2 {
            vd + 1
        } else {
            vd + stride * (nregs - 1)
        };
        if rn == 15 || last_reg >= 32 {
            return Err(Trap::Unpredictable(
                "NEON load single structure to all lanes register range",
            ));
        }
        for reg in 0..nregs {
            self.check_dreg(vd + reg * stride)?;
        }
        if nregs == 1 && stride == 2 {
            self.check_dreg(vd + 1)?;
        }

        let ebytes = 1usize << size;
        let lanes = 8 / ebytes;
        let mut address = self.regs[rn];

        for reg in 0..nregs {
            let value = load_neon_memory_elem(mem, address, ebytes)?;
            let mut out = [0u8; 16];
            for lane in 0..lanes {
                neon_write_elem(&mut out, lane, size, value);
            }
            let ext_reg = vd + reg * stride;
            self.set_dreg_bits(
                ext_reg,
                u64::from_le_bytes(out[..8].try_into().expect("all-lanes dreg")),
            );
            if nregs == 1 && stride == 2 {
                self.set_dreg_bits(
                    ext_reg + 1,
                    u64::from_le_bytes(out[..8].try_into().expect("all-lanes dreg")),
                );
            }
            address = address.wrapping_add(ebytes as u32);
        }

        if rm != 15 {
            let increment = if rm == 13 {
                (ebytes * nregs) as u32
            } else {
                self.regs[rm]
            };
            self.regs[rn] = self.regs[rn].wrapping_add(increment);
        }
        Ok(true)
    }

    fn exec_arm_neon_load_store_single<M: Memory>(
        &mut self,
        instr: u32,
        pc: u32,
        mem: &mut M,
    ) -> Result<bool> {
        let load = instr & (1 << 21) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let vd = vfp_double_d(instr);
        let nregs = ((instr >> 8) & 0x3) as usize + 1;
        let form = (instr >> 10) & 0x3;
        let rm = (instr & 0xf) as usize;

        let (size, reg_idx, stride, align) = match form {
            0 => (
                0,
                ((instr >> 5) & 0x7) as usize,
                1,
                ((instr >> 4) & 1) as u8,
            ),
            1 => (
                1,
                ((instr >> 6) & 0x3) as usize,
                ((instr >> 5) & 1) as usize + 1,
                ((instr >> 4) & 1) as u8,
            ),
            2 => (
                2,
                ((instr >> 7) & 0x1) as usize,
                ((instr >> 6) & 1) as usize + 1,
                ((instr >> 4) & 0x3) as u8,
            ),
            _ => return Ok(false),
        };

        match nregs {
            1 => {
                if stride != 1 || align & (1 << size) != 0 || (size == 2 && matches!(align, 1 | 2))
                {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
            }
            2 => {
                if size == 2 && align & 2 != 0 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
            }
            3 => {
                if align != 0 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
            }
            4 => {
                if size == 2 && align == 3 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
            }
            _ => unreachable!(),
        }

        let last_reg = vd + stride * (nregs - 1);
        if rn == 15 || last_reg >= 32 {
            return Err(Trap::Unpredictable(
                "NEON single structure transfer register range",
            ));
        }
        for reg in 0..nregs {
            self.check_dreg(vd + reg * stride)?;
        }

        let ebytes = 1usize << size;
        let mut address = self.regs[rn];
        for reg in 0..nregs {
            let ext_reg = vd + reg * stride;
            let mut bytes = self.neon_reg_bytes(ext_reg, false);
            if load {
                let value = load_neon_memory_elem(mem, address, ebytes)?;
                neon_write_elem(&mut bytes, reg_idx, size, value);
                self.set_dreg_bits(
                    ext_reg,
                    u64::from_le_bytes(bytes[..8].try_into().expect("single-lane dreg")),
                );
            } else {
                let value = neon_read_elem(&bytes, reg_idx, size);
                store_neon_memory_elem(mem, address, ebytes, value)?;
            }
            address = address.wrapping_add(ebytes as u32);
        }

        if rm != 15 {
            let increment = if rm == 13 {
                (ebytes * nregs) as u32
            } else {
                self.regs[rm]
            };
            self.regs[rn] = self.regs[rn].wrapping_add(increment);
        }
        Ok(true)
    }

    fn exec_arm_neon_modified_immediate(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let vd = vfp_double_d(instr);
        let q = instr & (1 << 6) != 0;
        let op = instr & (1 << 5) != 0;
        let cmode = ((instr >> 8) & 0xf) as u8;
        let imm = ((instr >> 17) & 0x80) | ((instr >> 12) & 0x70) | (instr & 0xf);

        self.check_dreg(vd)?;
        if q {
            if vd & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
        }
        if cmode == 15 && op {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let imm64 = neon_expand_modified_imm(imm as u8, cmode, op);
        let regs = if q { 2 } else { 1 };
        for idx in 0..regs {
            let reg = vd + idx;
            let old = self.dreg_bits(reg);
            let value = if cmode & 1 != 0 && cmode < 12 {
                if op { old & !imm64 } else { old | imm64 }
            } else if op && cmode != 14 {
                !imm64
            } else {
                imm64
            };
            self.set_dreg_bits(reg, value);
        }
        Ok(true)
    }

    fn exec_arm_neon_2reg_shift(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let u = instr & (1 << 24) != 0;
        let imm6 = (instr >> 16) & 0x3f;
        let vd = vfp_double_d(instr);
        let opcode = ((instr >> 8) & 0xf) as u8;
        let l = instr & (1 << 7) != 0;
        let q = instr & (1 << 6) != 0;
        let vm = vfp_double_m(instr);

        if opcode == 10 {
            return self.exec_arm_neon_2reg_shift_long(instr, pc, u, imm6, vd, q, vm);
        }

        if matches!(opcode, 14 | 15) {
            if l {
                return Ok(false);
            }
            return self.exec_arm_neon_vcvt_fixed(instr, pc, u, imm6, vd, q, vm, opcode == 15);
        }

        if matches!(opcode, 8 | 9) {
            return self.exec_arm_neon_2reg_shift_narrow(instr, pc, u, imm6, vd, opcode, l, q, vm);
        }

        let right_shift = matches!(opcode, 0..=4);
        let Some((size, shift)) = neon_decode_shift_imm(right_shift, l, imm6) else {
            return Ok(false);
        };

        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let lanes = neon_lanes(q, size);
        let src = self.neon_reg_bytes(vm, q);
        let old_dst = self.neon_reg_bytes(vd, q);
        let mut out = [0u8; 16];
        let mask = neon_elem_mask(size);
        for lane in 0..lanes {
            let value = neon_read_elem(&src, lane, size);
            let old = neon_read_elem(&old_dst, lane, size);
            let result = match opcode {
                0 => neon_shift_right(value, shift, size, !u, false),
                1 => old.wrapping_add(neon_shift_right(value, shift, size, !u, false)) & mask,
                2 => neon_shift_right(value, shift, size, !u, true),
                3 => old.wrapping_add(neon_shift_right(value, shift, size, !u, true)) & mask,
                4 if u => {
                    let shifted = neon_shift_right(value, shift, size, false, false);
                    let insert_mask = if shift == neon_elem_bits(size) {
                        0
                    } else {
                        mask >> shift
                    };
                    (old & !insert_mask) | (shifted & insert_mask)
                }
                5 if !u => {
                    if shift >= neon_elem_bits(size) {
                        0
                    } else {
                        (value << shift) & mask
                    }
                }
                5 if u => {
                    let insert_mask = if shift >= neon_elem_bits(size) {
                        0
                    } else {
                        (mask << shift) & mask
                    };
                    (old & !insert_mask) | ((value << shift) & insert_mask)
                }
                6 if u => self.neon_qshlu_immediate(value, size, shift),
                7 => self.neon_variable_shift(value, i64::from(shift), size, !u, true, false),
                _ => return Ok(false),
            };
            neon_write_elem(&mut out, lane, size, result);
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_shift_long(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        imm6: u32,
        vd: usize,
        q: bool,
        vm: usize,
    ) -> Result<bool> {
        if q || vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        let Some((size, shift)) = neon_decode_shift_imm(false, false, imm6) else {
            return Ok(false);
        };
        if size == 3 {
            return Ok(false);
        }

        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vm)?;

        let signed = !u;
        let src = self.neon_reg_bytes(vm, false);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let lanes = 8 >> size;
        for lane in 0..lanes {
            let value = neon_read_elem(&src, lane, size);
            let widened = neon_extend_elem(value, size, signed);
            let shifted = (widened << shift) & neon_elem_mask(wide_size);
            neon_write_elem(&mut out, lane, wide_size, shifted);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_2reg_shift_narrow(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        imm6: u32,
        vd: usize,
        opcode: u8,
        rounding: bool,
        q: bool,
        vm: usize,
    ) -> Result<bool> {
        if q {
            return Ok(false);
        }
        let Some((size, shift)) = neon_decode_shift_imm(true, false, imm6) else {
            return Ok(false);
        };
        if size == 3 {
            return Ok(false);
        }
        self.check_dreg(vd)?;
        if vm & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vm)?;
        self.check_dreg(vm + 1)?;

        let src_size = size + 1;
        let src = self.neon_reg_bytes(vm, true);
        let mut out = [0u8; 16];
        let dest_mask = neon_elem_mask(size);
        let lanes = 8 >> size;

        for lane in 0..lanes {
            let value = neon_read_elem(&src, lane, src_size);
            let shifted = match (opcode, u) {
                (8, true) | (9, false) => neon_shift_right(value, shift, src_size, true, rounding),
                _ => neon_shift_right(value, shift, src_size, false, rounding),
            };
            let result = match (opcode, u) {
                (8, false) => shifted & dest_mask,
                (8, true) => self.neon_saturating_narrow_to_unsigned(shifted, size, true),
                (9, false) => self.neon_saturating_narrow_to_signed(shifted, size),
                (9, true) => self.neon_saturating_narrow_to_unsigned(shifted, size, false),
                _ => return Ok(false),
            };
            neon_write_elem(&mut out, lane, size, result);
        }

        self.set_dreg_bits(
            vd,
            u64::from_le_bytes(out[..8].try_into().expect("narrow dreg")),
        );
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_vcvt_fixed(
        &mut self,
        instr: u32,
        pc: u32,
        unsigned: bool,
        imm6: u32,
        vd: usize,
        q: bool,
        vm: usize,
        to_fixed: bool,
    ) -> Result<bool> {
        if imm6 & 0x20 == 0 {
            return Ok(false);
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let fbits = 64 - imm6;
        let scale = 2f64.powi(fbits as i32);
        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];

        for lane in 0..(if q { 4 } else { 2 }) {
            let value = neon_read_elem(&src, lane, 2) as u32;
            let result = if to_fixed {
                let input = f64::from(f32::from_bits(value)) * scale;
                let (bits, flags) = if unsigned {
                    vfp_trunc_to_u32_bits(input)
                } else {
                    vfp_trunc_to_i32_bits(input)
                };
                self.fpscr |= flags;
                bits
            } else {
                let input = if unsigned {
                    f64::from(value)
                } else {
                    f64::from(value as i32)
                } / scale;
                let result = input as f32;
                self.fpscr |= vfp_int_to_f32_inexact_flag(input, result);
                result.to_bits()
            };
            neon_write_elem(&mut out, lane, 2, u64::from(result));
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_vext(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let vd = vfp_double_d(instr);
        let vn = vfp_double_n(instr);
        let vm = vfp_double_m(instr);
        let q = instr & (1 << 6) != 0;
        let imm = ((instr >> 8) & 0xf) as usize;

        self.check_neon_3same_regs(vd, vn, vm, q, pc, instr)?;
        if !q && imm & 0x8 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let bytes = neon_reg_bytes(q);
        let src_n = self.neon_reg_bytes(vn, q);
        let src_m = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        for idx in 0..bytes {
            let src = idx + imm;
            out[idx] = if src < bytes {
                src_n[src]
            } else {
                src_m[src - bytes]
            };
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_table_lookup(&mut self, instr: u32, _pc: u32) -> Result<bool> {
        let vd = vfp_double_d(instr);
        let vn = vfp_double_n(instr);
        let vm = vfp_double_m(instr);
        let len = ((instr >> 8) & 0x3) as usize + 1;
        let tbx = instr & (1 << 6) != 0;

        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if vn + len > 32 {
            return Err(Trap::Unpredictable("NEON table lookup register range"));
        }
        for idx in 0..len {
            self.check_dreg(vn + idx)?;
        }

        let indices = self.neon_reg_bytes(vm, false);
        let mut table = [0u8; 32];
        for reg_idx in 0..len {
            let bytes = self.dreg_bits(vn + reg_idx).to_le_bytes();
            table[reg_idx * 8..reg_idx * 8 + 8].copy_from_slice(&bytes);
        }
        let old_dst = self.neon_reg_bytes(vd, false);
        let mut out = [0u8; 16];
        for lane in 0..8 {
            let index = indices[lane] as usize;
            out[lane] = if index < len * 8 {
                table[index]
            } else if tbx {
                old_dst[lane]
            } else {
                0
            };
        }
        self.set_neon_reg_bytes(vd, false, out);
        Ok(true)
    }

    fn exec_arm_neon_vdup_scalar(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let vd = vfp_double_d(instr);
        let vm = vfp_double_m(instr);
        let q = instr & (1 << 6) != 0;
        let imm4 = ((instr >> 16) & 0xf) as u8;
        if imm4 & 0x7 == 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
        }

        let size = imm4.trailing_zeros() as usize;
        if size > 2 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        let index = (imm4 >> (size + 1)) as usize;
        let src = self.neon_reg_bytes(vm, false);
        let element = neon_read_elem(&src, index, size);
        let mut out = [0u8; 16];
        for lane in 0..neon_lanes(q, size) {
            neon_write_elem(&mut out, lane, size, element);
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_misc(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let vd = vfp_double_d(instr);
        let vm = vfp_double_m(instr);
        let q = instr & (1 << 6) != 0;
        let size = ((instr >> 18) & 0x3) as usize;
        let op1 = (instr >> 16) & 0x3;
        let op2 = (instr >> 7) & 0xf;

        match (op1, op2) {
            (0, 4 | 5) => self.exec_arm_neon_pairwise_add_long(
                instr,
                pc,
                size,
                op2 & 1 != 0,
                false,
                q,
                vd,
                vm,
            ),
            (0, 8..=11) => self.exec_arm_neon_2reg_count_or_not(instr, pc, size, op2, q, vd, vm),
            (0, 12 | 13) => {
                self.exec_arm_neon_pairwise_add_long(instr, pc, size, op2 & 1 != 0, true, q, vd, vm)
            }
            (0, 14 | 15) => {
                self.exec_arm_neon_2reg_saturating_abs_neg(instr, pc, size, op2 == 15, q, vd, vm)
            }
            (1, 0..=4 | 8..=12) => {
                self.exec_arm_neon_2reg_compare_zero(instr, pc, size, op2, q, vd, vm)
            }
            (0, 0..=2) => {
                self.check_dreg(vd)?;
                self.check_dreg(vm)?;
                if q {
                    if vd & 1 != 0 || vm & 1 != 0 {
                        return Err(Trap::UndefinedArm { pc, instr });
                    }
                    self.check_dreg(vd + 1)?;
                    self.check_dreg(vm + 1)?;
                }
                if op2 as usize + size >= 3 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }

                let group_bytes = match op2 {
                    0 => 8,
                    1 => 4,
                    2 => 2,
                    _ => unreachable!(),
                };
                let elem_bytes = 1usize << size;
                let bytes = neon_reg_bytes(q);
                let src = self.neon_reg_bytes(vm, q);
                let mut out = [0u8; 16];
                for group_start in (0..bytes).step_by(group_bytes) {
                    let elems = group_bytes / elem_bytes;
                    for elem in 0..elems {
                        let dst_off = group_start + elem * elem_bytes;
                        let src_off = group_start + (elems - 1 - elem) * elem_bytes;
                        out[dst_off..dst_off + elem_bytes]
                            .copy_from_slice(&src[src_off..src_off + elem_bytes]);
                    }
                }
                self.set_neon_reg_bytes(vd, q, out);
                Ok(true)
            }
            (2, 4) if q => {
                self.exec_arm_neon_2reg_saturating_narrow(instr, pc, size, true, true, vd, vm)
            }
            (2, 5) => self.exec_arm_neon_2reg_saturating_narrow(instr, pc, size, !q, false, vd, vm),
            (2, 4) if !q => {
                if size == 3 || vm & 1 != 0 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
                self.check_dreg(vd)?;
                self.check_dreg(vm)?;
                self.check_dreg(vm + 1)?;

                let src = self.neon_reg_bytes(vm, true);
                let mut out = [0u8; 16];
                let wide_size = size + 1;
                for lane in 0..(8 >> size) {
                    let value = neon_read_elem(&src, lane, wide_size) & neon_elem_mask(size);
                    neon_write_elem(&mut out, lane, size, value);
                }
                self.set_dreg_bits(
                    vd,
                    u64::from_le_bytes(out[..8].try_into().expect("vmovn dreg")),
                );
                Ok(true)
            }
            (2, 6) if !q => {
                if size == 3 || vd & 1 != 0 {
                    return Err(Trap::UndefinedArm { pc, instr });
                }
                self.check_dreg(vd)?;
                self.check_dreg(vd + 1)?;
                self.check_dreg(vm)?;

                let src = self.neon_reg_bytes(vm, false);
                let mut out = [0u8; 16];
                let wide_size = size + 1;
                let shift = neon_elem_bits(size);
                for lane in 0..(8 >> size) {
                    let value = neon_read_elem(&src, lane, size);
                    let widened = (value << shift) & neon_elem_mask(wide_size);
                    neon_write_elem(&mut out, lane, wide_size, widened);
                }
                self.set_neon_reg_bytes(vd, true, out);
                Ok(true)
            }
            (2, 0..=3) => self.exec_arm_neon_2reg_permute(instr, pc, size, op2, q, vd, vm),
            (3, 8..=11) => self.exec_arm_neon_2reg_recip_estimate(instr, pc, size, op2, q, vd, vm),
            (3, 12..=15) => {
                self.exec_arm_neon_2reg_convert_integer(instr, pc, size, op2, q, vd, vm)
            }
            (1, 6 | 7 | 14 | 15) => {
                self.exec_arm_neon_2reg_abs_neg(instr, pc, size, op2, q, vd, vm)
            }
            _ => Ok(false),
        }
    }

    fn exec_arm_neon_2reg_compare_zero(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        let float = op2 & 0b1000 != 0;
        let kind = op2 & 0b111;
        if kind > 4 || size == 3 || (float && size != 2) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        if float {
            for lane in 0..(if q { 4 } else { 2 }) {
                let value = f32::from_bits(neon_read_elem(&src, lane, 2) as u32);
                let pass = match kind {
                    0 => value > 0.0,
                    1 => value >= 0.0,
                    2 => value == 0.0,
                    3 => value <= 0.0,
                    4 => value < 0.0,
                    _ => unreachable!(),
                };
                neon_write_elem(&mut out, lane, 2, u64::from(neon_f32_compare(pass)));
            }
        } else {
            let mask = neon_elem_mask(size);
            for lane in 0..neon_lanes(q, size) {
                let value = neon_sign_extend(neon_read_elem(&src, lane, size), size);
                let pass = match kind {
                    0 => value > 0,
                    1 => value >= 0,
                    2 => value == 0,
                    3 => value <= 0,
                    4 => value < 0,
                    _ => unreachable!(),
                };
                neon_write_elem(&mut out, lane, size, if pass { mask } else { 0 });
            }
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_recip_estimate(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        if size != 2 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let float = op2 >= 10;
        let rsqrt = op2 & 1 != 0;
        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        for lane in 0..(if q { 4 } else { 2 }) {
            let value = neon_read_elem(&src, lane, 2) as u32;
            let result = if float {
                let value = f32::from_bits(value);
                if rsqrt {
                    (1.0f32 / value.sqrt()).to_bits()
                } else {
                    (1.0f32 / value).to_bits()
                }
            } else if rsqrt {
                neon_rsqrte_u32(value)
            } else {
                neon_recpe_u32(value)
            };
            neon_write_elem(&mut out, lane, 2, u64::from(result));
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_2reg_saturating_narrow(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        signed_input: bool,
        unsigned_result: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        if size == 3 || vm & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        self.check_dreg(vm + 1)?;

        let src = self.neon_reg_bytes(vm, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        for lane in 0..(8 >> size) {
            let value = neon_read_elem(&src, lane, wide_size);
            let result = if unsigned_result {
                self.neon_saturating_narrow_to_unsigned(value, size, signed_input)
            } else if signed_input {
                self.neon_saturating_narrow_to_signed(value, size)
            } else {
                self.neon_saturating_narrow_unsigned_to_signed(value, size)
            };
            neon_write_elem(&mut out, lane, size, result);
        }

        self.set_dreg_bits(
            vd,
            u64::from_le_bytes(out[..8].try_into().expect("saturating narrow dreg")),
        );
        Ok(true)
    }

    fn exec_arm_neon_2reg_convert_integer(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        if size != 2 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let to_integer = op2 & 0b10 != 0;
        let unsigned = op2 & 1 != 0;
        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        for lane in 0..(if q { 4 } else { 2 }) {
            let bits = neon_read_elem(&src, lane, 2) as u32;
            let result = if to_integer {
                let value = f32::from_bits(bits);
                if unsigned {
                    value as u32
                } else {
                    value as i32 as u32
                }
            } else if unsigned {
                (bits as f32).to_bits()
            } else {
                ((bits as i32) as f32).to_bits()
            };
            neon_write_elem(&mut out, lane, 2, u64::from(result));
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_pairwise_add_long(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        unsigned: bool,
        accumulate: bool,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        if size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let src = self.neon_reg_bytes(vm, q);
        let old = self.neon_reg_bytes(vd, q);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let mask = neon_elem_mask(wide_size);

        for lane in 0..neon_lanes(q, wide_size) {
            let a = neon_read_elem(&src, lane * 2, size);
            let b = neon_read_elem(&src, lane * 2 + 1, size);
            let pair = if unsigned {
                a.wrapping_add(b)
            } else {
                (neon_sign_extend(a, size) as i128 + neon_sign_extend(b, size) as i128) as u64
            } & mask;
            let result = if accumulate {
                pair.wrapping_add(neon_read_elem(&old, lane, wide_size)) & mask
            } else {
                pair
            };
            neon_write_elem(&mut out, lane, wide_size, result);
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_count_or_not(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        match op2 {
            8 | 9 if size == 3 => return Err(Trap::UndefinedArm { pc, instr }),
            10 | 11 if size != 0 => return Err(Trap::UndefinedArm { pc, instr }),
            _ => {}
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        let bytes = neon_reg_bytes(q);
        if op2 == 11 {
            for idx in 0..bytes {
                out[idx] = !src[idx];
            }
        } else if op2 == 10 {
            for idx in 0..bytes {
                out[idx] = src[idx].count_ones() as u8;
            }
        } else {
            let bits = neon_elem_bits(size);
            let mask = neon_elem_mask(size);
            for lane in 0..neon_lanes(q, size) {
                let value = neon_read_elem(&src, lane, size) & mask;
                let count_source = if op2 == 8 && value & (1u64 << (bits - 1)) != 0 {
                    !value & mask
                } else {
                    value
                };
                let leading = if count_source == 0 {
                    bits
                } else {
                    count_source.leading_zeros() - (64 - bits)
                };
                let result = if op2 == 8 { leading - 1 } else { leading };
                neon_write_elem(&mut out, lane, size, u64::from(result));
            }
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_saturating_abs_neg(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        negate: bool,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        if size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let bits = neon_elem_bits(size);
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        for lane in 0..neon_lanes(q, size) {
            let value = i128::from(neon_sign_extend(neon_read_elem(&src, lane, size), size));
            let result = if value == min {
                self.cpsr.q = true;
                max
            } else if negate {
                -value
            } else {
                value.abs()
            };
            neon_write_elem(&mut out, lane, size, result as i64 as u64);
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_permute(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        if size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if matches!(op2, 2 | 3) && !q && size == 2 {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let lhs = self.neon_reg_bytes(vd, q);
        let rhs = self.neon_reg_bytes(vm, q);
        let mut out_d = [0u8; 16];
        let mut out_m = [0u8; 16];
        let lanes = neon_lanes(q, size);

        match op2 {
            0 => {
                self.set_neon_reg_bytes(vd, q, rhs);
                self.set_neon_reg_bytes(vm, q, lhs);
                return Ok(true);
            }
            1 => {
                for pair in (0..lanes).step_by(2) {
                    neon_write_elem(&mut out_d, pair, size, neon_read_elem(&lhs, pair, size));
                    neon_write_elem(&mut out_d, pair + 1, size, neon_read_elem(&rhs, pair, size));
                    neon_write_elem(&mut out_m, pair, size, neon_read_elem(&lhs, pair + 1, size));
                    neon_write_elem(
                        &mut out_m,
                        pair + 1,
                        size,
                        neon_read_elem(&rhs, pair + 1, size),
                    );
                }
            }
            2 => {
                let mut d_lane = 0;
                let mut m_lane = 0;
                for lane in 0..(lanes * 2) {
                    let value = if lane < lanes {
                        neon_read_elem(&lhs, lane, size)
                    } else {
                        neon_read_elem(&rhs, lane - lanes, size)
                    };
                    if lane & 1 == 0 {
                        neon_write_elem(&mut out_d, d_lane, size, value);
                        d_lane += 1;
                    } else {
                        neon_write_elem(&mut out_m, m_lane, size, value);
                        m_lane += 1;
                    }
                }
            }
            3 => {
                for lane in 0..lanes {
                    let out_lane = lane * 2;
                    let (target, target_lane) = if out_lane < lanes {
                        (&mut out_d, out_lane)
                    } else {
                        (&mut out_m, out_lane - lanes)
                    };
                    neon_write_elem(target, target_lane, size, neon_read_elem(&lhs, lane, size));

                    let out_lane = out_lane + 1;
                    let (target, target_lane) = if out_lane < lanes {
                        (&mut out_d, out_lane)
                    } else {
                        (&mut out_m, out_lane - lanes)
                    };
                    neon_write_elem(target, target_lane, size, neon_read_elem(&rhs, lane, size));
                }
            }
            _ => unreachable!(),
        }

        self.set_neon_reg_bytes(vd, q, out_d);
        self.set_neon_reg_bytes(vm, q, out_m);
        Ok(true)
    }

    fn exec_arm_neon_2reg_abs_neg(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        op2: u32,
        q: bool,
        vd: usize,
        vm: usize,
    ) -> Result<bool> {
        let float = op2 >= 14;
        let negate = matches!(op2, 7 | 15);
        if size == 3 || (float && size != 2) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vm)?;
        if q {
            if vd & 1 != 0 || vm & 1 != 0 {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd + 1)?;
            self.check_dreg(vm + 1)?;
        }

        let src = self.neon_reg_bytes(vm, q);
        let mut out = [0u8; 16];
        if float {
            for lane in 0..(if q { 4 } else { 2 }) {
                let bits = neon_read_elem(&src, lane, 2) as u32;
                let result = if negate {
                    bits ^ 0x8000_0000
                } else {
                    bits & 0x7fff_ffff
                };
                neon_write_elem(&mut out, lane, 2, u64::from(result));
            }
        } else {
            let mask = neon_elem_mask(size);
            for lane in 0..neon_lanes(q, size) {
                let value = neon_read_elem(&src, lane, size);
                let signed = neon_sign_extend(value, size);
                let result = if negate {
                    (0u64.wrapping_sub(value)) & mask
                } else if signed < 0 {
                    (0u64.wrapping_sub(value)) & mask
                } else {
                    value & mask
                };
                neon_write_elem(&mut out, lane, size, result);
            }
        }
        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_3same(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let u = instr & (1 << 24) != 0;
        let size = ((instr >> 20) & 0x3) as usize;
        let opcode = ((instr >> 8) & 0xf) as u8;
        let q = instr & (1 << 6) != 0;
        let op = instr & (1 << 4) != 0;
        let vd = vfp_double_d(instr);
        let vn = vfp_double_n(instr);
        let vm = vfp_double_m(instr);
        self.check_neon_3same_regs(vd, vn, vm, q, pc, instr)?;

        if opcode == 1 && op {
            let dst = self.neon_reg_bytes(vd, q);
            let lhs = self.neon_reg_bytes(vn, q);
            let rhs = self.neon_reg_bytes(vm, q);
            let mut out = [0u8; 16];
            let bytes = neon_reg_bytes(q);
            for idx in 0..bytes {
                out[idx] = match (u, size) {
                    (false, 0) => lhs[idx] & rhs[idx],
                    (false, 1) => lhs[idx] & !rhs[idx],
                    (false, 2) => lhs[idx] | rhs[idx],
                    (false, 3) => lhs[idx] | !rhs[idx],
                    (true, 0) => lhs[idx] ^ rhs[idx],
                    (true, 1) => (lhs[idx] & dst[idx]) | (rhs[idx] & !dst[idx]),
                    (true, 2) => (lhs[idx] & rhs[idx]) | (dst[idx] & !rhs[idx]),
                    (true, 3) => (dst[idx] & rhs[idx]) | (lhs[idx] & !rhs[idx]),
                    _ => unreachable!(),
                };
            }
            self.set_neon_reg_bytes(vd, q, out);
            return Ok(true);
        }

        if matches!(opcode, 4 | 5) {
            return self.exec_arm_neon_3same_shift(instr, pc, u, size, opcode, q, op, vd, vn, vm);
        }

        if matches!(opcode, 0 | 1 | 2 | 3 | 6 | 7 | 8 | 9 | 10 | 11) {
            return self.exec_arm_neon_3same_int(instr, pc, u, size, opcode, q, op, vd, vn, vm);
        }

        if matches!(opcode, 12..=15) {
            return self.exec_arm_neon_3same_f32(instr, pc, u, size, opcode, q, op, vd, vn, vm);
        }

        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3same_int(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        opcode: u8,
        q: bool,
        op: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if matches!(opcode, 3 | 6 | 7) && size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if matches!(opcode, 0 | 2) && size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if opcode == 10 && (size == 3 || q) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if opcode == 11 && op && q {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if opcode == 11 && !op && (size == 0 || size == 3) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if opcode == 9 && op && u && size != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        if opcode == 10 {
            let lanes = neon_lanes(false, size);
            let lhs = self.neon_reg_bytes(vn, false);
            let rhs = self.neon_reg_bytes(vm, false);
            let mut out = [0u8; 16];
            for lane in 0..lanes {
                let a = if lane * 2 < lanes {
                    neon_read_elem(&lhs, lane * 2, size)
                } else {
                    neon_read_elem(&rhs, lane * 2 - lanes, size)
                };
                let b = if lane * 2 + 1 < lanes {
                    neon_read_elem(&lhs, lane * 2 + 1, size)
                } else {
                    neon_read_elem(&rhs, lane * 2 + 1 - lanes, size)
                };
                let result = neon_minmax(a, b, size, !u, !op);
                neon_write_elem(&mut out, lane, size, result);
            }
            self.set_neon_reg_bytes(vd, false, out);
            return Ok(true);
        }

        let lanes = neon_lanes(q, size);
        let lhs = self.neon_reg_bytes(vn, q);
        let rhs = self.neon_reg_bytes(vm, q);
        let old_dst = self.neon_reg_bytes(vd, q);
        let mut out = [0u8; 16];

        for lane in 0..lanes {
            let a = neon_read_elem(&lhs, lane, size);
            let b = neon_read_elem(&rhs, lane, size);
            let d = neon_read_elem(&old_dst, lane, size);
            let mask = neon_elem_mask(size);
            let result = match (opcode, u, op) {
                (0, unsigned, false) => neon_halving_add(a, b, size, !unsigned, false),
                (0, unsigned, true) => self.neon_saturating_add(a, b, size, !unsigned),
                (1, unsigned, false) => neon_halving_add(a, b, size, !unsigned, true),
                (1, _, _) => unreachable!(),
                (2, unsigned, false) => neon_halving_sub(a, b, size, !unsigned),
                (2, unsigned, true) => self.neon_saturating_sub(a, b, size, !unsigned),
                (3, unsigned, false) => neon_compare_gt(a, b, size, !unsigned),
                (3, unsigned, true) => neon_compare_ge(a, b, size, !unsigned),
                (6, unsigned, false) => neon_minmax(a, b, size, !unsigned, true),
                (6, unsigned, true) => neon_minmax(a, b, size, !unsigned, false),
                (7, unsigned, false) => neon_abs_diff(a, b, size, !unsigned),
                (7, unsigned, true) => d.wrapping_add(neon_abs_diff(a, b, size, !unsigned)) & mask,
                (8, false, false) => a.wrapping_add(b) & mask,
                (8, true, false) => a.wrapping_sub(b) & mask,
                (8, false, true) => {
                    if a & b != 0 {
                        mask
                    } else {
                        0
                    }
                }
                (8, true, true) => {
                    if a == b {
                        mask
                    } else {
                        0
                    }
                }
                (9, false, false) => d.wrapping_add(a.wrapping_mul(b)) & mask,
                (9, true, false) => d.wrapping_sub(a.wrapping_mul(b)) & mask,
                (9, false, true) => a.wrapping_mul(b) & mask,
                (9, true, true) => neon_polynomial_mul8(a as u8, b as u8) as u64,
                (11, rounding, false) => {
                    self.neon_saturating_doubling_mul_high(a, b, size, rounding)
                }
                (11, false, true) => neon_pairwise_add_lane(&lhs, &rhs, lane, size),
                _ => return Ok(false),
            };
            neon_write_elem(&mut out, lane, size, result);
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3same_shift(
        &mut self,
        _instr: u32,
        _pc: u32,
        u: bool,
        size: usize,
        opcode: u8,
        q: bool,
        op: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        let signed = !u;
        let rounding = opcode == 5;
        let saturating = op;
        let lanes = neon_lanes(q, size);
        let values = self.neon_reg_bytes(vm, q);
        let shifts = self.neon_reg_bytes(vn, q);
        let mut out = [0u8; 16];

        for lane in 0..lanes {
            let value = neon_read_elem(&values, lane, size);
            let shift = (neon_read_elem(&shifts, lane, size) as u8) as i8 as i64;
            let result = self.neon_variable_shift(value, shift, size, signed, saturating, rounding);
            neon_write_elem(&mut out, lane, size, result);
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    fn exec_arm_neon_3diff(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let u = instr & (1 << 24) != 0;
        let size = ((instr >> 20) & 0x3) as usize;
        let opcode = ((instr >> 8) & 0xf) as u8;
        let vd = vfp_double_d(instr);
        let vn = vfp_double_n(instr);
        let vm = vfp_double_m(instr);

        if size == 3 {
            return Ok(false);
        }

        match opcode {
            0 | 1 | 2 | 3 => {
                self.exec_arm_neon_3diff_add_sub_wide(instr, pc, u, size, opcode, vd, vn, vm)
            }
            4 | 6 => {
                self.exec_arm_neon_3diff_add_sub_high_narrow(instr, pc, opcode, u, size, vd, vn, vm)
            }
            5 | 7 => self.exec_arm_neon_3diff_absdiff(instr, pc, u, size, opcode == 5, vd, vn, vm),
            8 | 10 => {
                self.exec_arm_neon_3diff_mul_accum(instr, pc, u, size, opcode == 10, vd, vn, vm)
            }
            9 if !u => {
                self.exec_arm_neon_3diff_qdmul_long(instr, pc, size, Some(false), vd, vn, vm)
            }
            11 if !u => {
                self.exec_arm_neon_3diff_qdmul_long(instr, pc, size, Some(true), vd, vn, vm)
            }
            12 => self.exec_arm_neon_3diff_mul_long(instr, pc, u, size, false, vd, vn, vm),
            13 if !u => self.exec_arm_neon_3diff_qdmul_long(instr, pc, size, None, vd, vn, vm),
            14 if !u && size != 2 => {
                self.exec_arm_neon_3diff_mul_long(instr, pc, false, size, true, vd, vn, vm)
            }
            _ => Ok(false),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_add_sub_wide(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        opcode: u8,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        let widen_both = matches!(opcode, 0 | 2);
        let subtract = matches!(opcode, 2 | 3);
        if vd & 1 != 0 || (!widen_both && vn & 1 != 0) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;
        if !widen_both {
            self.check_dreg(vn + 1)?;
        }

        let signed = !u;
        let mut out = [0u8; 16];
        let rhs = self.neon_reg_bytes(vm, false);
        let lhs = self.neon_reg_bytes(vn, !widen_both);
        let wide_size = size + 1;
        let mask = neon_elem_mask(wide_size);
        for lane in 0..(8 >> size) {
            let a = if widen_both {
                neon_extend_elem(neon_read_elem(&lhs, lane, size), size, signed)
            } else {
                neon_read_elem(&lhs, lane, wide_size)
            };
            let b = neon_extend_elem(neon_read_elem(&rhs, lane, size), size, signed);
            let result = if subtract {
                a.wrapping_sub(b)
            } else {
                a.wrapping_add(b)
            } & mask;
            neon_write_elem(&mut out, lane, wide_size, result);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_add_sub_high_narrow(
        &mut self,
        instr: u32,
        pc: u32,
        opcode: u8,
        rounding: bool,
        size: usize,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if vn & 1 != 0 || vm & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vn)?;
        self.check_dreg(vn + 1)?;
        self.check_dreg(vm)?;
        self.check_dreg(vm + 1)?;

        let lhs = self.neon_reg_bytes(vn, true);
        let rhs = self.neon_reg_bytes(vm, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let narrow_bits = neon_elem_bits(size);
        let round = if rounding {
            1u64 << (narrow_bits - 1)
        } else {
            0
        };
        for lane in 0..(8 >> size) {
            let a = neon_read_elem(&lhs, lane, wide_size);
            let b = neon_read_elem(&rhs, lane, wide_size);
            let wide = if opcode == 6 {
                a.wrapping_sub(b)
            } else {
                a.wrapping_add(b)
            };
            neon_write_elem(
                &mut out,
                lane,
                size,
                wide.wrapping_add(round) >> narrow_bits,
            );
        }
        self.set_dreg_bits(
            vd,
            u64::from_le_bytes(out[..8].try_into().expect("high narrow dreg")),
        );
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_absdiff(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        accumulate: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;

        let signed = !u;
        let lhs = self.neon_reg_bytes(vn, false);
        let rhs = self.neon_reg_bytes(vm, false);
        let old = self.neon_reg_bytes(vd, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let mask = neon_elem_mask(wide_size);
        for lane in 0..(8 >> size) {
            let a = neon_extend_i128(neon_read_elem(&lhs, lane, size), size, signed);
            let b = neon_extend_i128(neon_read_elem(&rhs, lane, size), size, signed);
            let mut result = a.abs_diff(b) as u64 & mask;
            if accumulate {
                result = result.wrapping_add(neon_read_elem(&old, lane, wide_size)) & mask;
            }
            neon_write_elem(&mut out, lane, wide_size, result);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_mul_accum(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        subtract: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;

        let signed = !u;
        let lhs = self.neon_reg_bytes(vn, false);
        let rhs = self.neon_reg_bytes(vm, false);
        let old = self.neon_reg_bytes(vd, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let mask = neon_elem_mask(wide_size);
        for lane in 0..(8 >> size) {
            let a = neon_extend_i128(neon_read_elem(&lhs, lane, size), size, signed);
            let b = neon_extend_i128(neon_read_elem(&rhs, lane, size), size, signed);
            let product = neon_mask_i128(a * b, wide_size);
            let acc = neon_read_elem(&old, lane, wide_size);
            let result = if subtract {
                acc.wrapping_sub(product)
            } else {
                acc.wrapping_add(product)
            } & mask;
            neon_write_elem(&mut out, lane, wide_size, result);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_qdmul_long(
        &mut self,
        instr: u32,
        pc: u32,
        size: usize,
        subtract_acc: Option<bool>,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if !matches!(size, 1 | 2) {
            return Ok(false);
        }
        if vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;

        let lhs = self.neon_reg_bytes(vn, false);
        let rhs = self.neon_reg_bytes(vm, false);
        let old = self.neon_reg_bytes(vd, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        for lane in 0..(8 >> size) {
            let a = neon_read_elem(&lhs, lane, size);
            let b = neon_read_elem(&rhs, lane, size);
            let product = self.neon_saturating_doubling_mul_long(a, b, size);
            let result = if let Some(subtract) = subtract_acc {
                let acc = neon_read_elem(&old, lane, wide_size);
                self.neon_saturating_add_sub_wide(acc, product, wide_size, subtract)
            } else {
                product
            };
            neon_write_elem(&mut out, lane, wide_size, result);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3diff_mul_long(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        polynomial: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;

        let signed = !u;
        let lhs = self.neon_reg_bytes(vn, false);
        let rhs = self.neon_reg_bytes(vm, false);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        for lane in 0..(8 >> size) {
            let product = if polynomial {
                if size == 0 {
                    u64::from(neon_polynomial_mul8(
                        neon_read_elem(&lhs, lane, size) as u8,
                        neon_read_elem(&rhs, lane, size) as u8,
                    ))
                } else {
                    return Ok(false);
                }
            } else {
                let a = neon_extend_i128(neon_read_elem(&lhs, lane, size), size, signed);
                let b = neon_extend_i128(neon_read_elem(&rhs, lane, size), size, signed);
                neon_mask_i128(a * b, wide_size)
            };
            neon_write_elem(&mut out, lane, wide_size, product);
        }
        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    fn exec_arm_neon_2reg_scalar(&mut self, instr: u32, pc: u32) -> Result<bool> {
        let q_or_u = instr & (1 << 24) != 0;
        let size = ((instr >> 20) & 0x3) as usize;
        let vd = vfp_double_d(instr);
        let vn = vfp_double_n(instr);
        let vm = vfp_double_m(instr);
        let opcode = ((instr >> 8) & 0xf) as u8;

        if (instr & 0xfe80_0a50) == 0xf280_0040
            || (instr & 0xfe80_0e50) == 0xf280_0840
            || (instr & 0xfe80_0f50) == 0xf280_0c40
            || (instr & 0xfe80_0f50) == 0xf280_0d40
        {
            return self.exec_arm_neon_scalar_mul(instr, pc, q_or_u, size, opcode, vd, vn, vm);
        }

        if (instr & 0xfe80_0b50) == 0xf280_0240
            || (instr & 0xff80_0f50) == 0xf280_0340
            || (instr & 0xff80_0f50) == 0xf280_0740
            || (instr & 0xfe80_0f50) == 0xf280_0a40
            || (instr & 0xff80_0f50) == 0xf280_0b40
        {
            return self.exec_arm_neon_scalar_long_mul(instr, pc, q_or_u, size, opcode, vd, vn, vm);
        }

        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_scalar_mul(
        &mut self,
        instr: u32,
        pc: u32,
        q: bool,
        size: usize,
        opcode: u8,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if size == 0 || size == 3 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if q && (vd & 1 != 0 || vn & 1 != 0) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vn)?;
        if q {
            self.check_dreg(vd + 1)?;
            self.check_dreg(vn + 1)?;
        }

        let float = matches!(opcode, 1 | 5 | 9);
        if float && size != 2 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if matches!(opcode, 12 | 13) && size == 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let (scalar_reg, scalar_lane) = neon_scalar_location(size, vm);
        self.check_dreg(scalar_reg)?;
        let scalar_src = self.neon_reg_bytes(scalar_reg, false);
        let scalar = neon_read_elem(&scalar_src, scalar_lane, size);
        let src = self.neon_reg_bytes(vn, q);
        let old = self.neon_reg_bytes(vd, q);
        let mut out = [0u8; 16];
        let lanes = neon_lanes(q, size);
        let mask = neon_elem_mask(size);

        for lane in 0..lanes {
            let n = neon_read_elem(&src, lane, size);
            let d = neon_read_elem(&old, lane, size);
            let result = if float {
                let n = f32::from_bits(n as u32);
                let s = f32::from_bits(scalar as u32);
                let product = n * s;
                let d = f32::from_bits(d as u32);
                match opcode {
                    1 => (d + product).to_bits() as u64,
                    5 => (d - product).to_bits() as u64,
                    9 => product.to_bits() as u64,
                    _ => unreachable!(),
                }
            } else {
                match opcode {
                    0 => d.wrapping_add(n.wrapping_mul(scalar)) & mask,
                    4 => d.wrapping_sub(n.wrapping_mul(scalar)) & mask,
                    8 => n.wrapping_mul(scalar) & mask,
                    12 => self.neon_saturating_doubling_mul_high(n, scalar, size, false),
                    13 => self.neon_saturating_doubling_mul_high(n, scalar, size, true),
                    _ => return Ok(false),
                }
            };
            neon_write_elem(&mut out, lane, size, result);
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_scalar_long_mul(
        &mut self,
        instr: u32,
        pc: u32,
        unsigned: bool,
        size: usize,
        opcode: u8,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if size == 0 || size == 3 || vd & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        self.check_dreg(vd)?;
        self.check_dreg(vd + 1)?;
        self.check_dreg(vn)?;

        let (scalar_reg, scalar_lane) = neon_scalar_location(size, vm);
        self.check_dreg(scalar_reg)?;
        let scalar_src = self.neon_reg_bytes(scalar_reg, false);
        let scalar = neon_read_elem(&scalar_src, scalar_lane, size);
        let src = self.neon_reg_bytes(vn, false);
        let old = self.neon_reg_bytes(vd, true);
        let mut out = [0u8; 16];
        let wide_size = size + 1;
        let mask = neon_elem_mask(wide_size);
        let signed = !unsigned;

        for lane in 0..(8 >> size) {
            let n = neon_read_elem(&src, lane, size);
            let product = match opcode {
                3 | 7 | 11 => self.neon_saturating_doubling_mul_long(n, scalar, size),
                _ => {
                    let lhs = neon_extend_i128(n, size, signed);
                    let rhs = neon_extend_i128(scalar, size, signed);
                    neon_mask_i128(lhs * rhs, wide_size)
                }
            };
            let acc = neon_read_elem(&old, lane, wide_size);
            let result = match opcode {
                2 => acc.wrapping_add(product) & mask,
                3 => self.neon_saturating_add_sub_wide(acc, product, wide_size, false),
                6 => acc.wrapping_sub(product) & mask,
                7 => self.neon_saturating_add_sub_wide(acc, product, wide_size, true),
                10 | 11 => product,
                _ => return Ok(false),
            };
            neon_write_elem(&mut out, lane, wide_size, result);
        }

        self.set_neon_reg_bytes(vd, true, out);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_arm_neon_3same_f32(
        &mut self,
        instr: u32,
        pc: u32,
        u: bool,
        size: usize,
        opcode: u8,
        q: bool,
        op: bool,
        vd: usize,
        vn: usize,
        vm: usize,
    ) -> Result<bool> {
        if size & 1 != 0 {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let subop = size >> 1;
        if q && ((opcode == 13 && u && subop == 0 && !op) || (opcode == 15 && u && !op)) {
            return Err(Trap::UndefinedArm { pc, instr });
        }

        let lhs = self.neon_reg_bytes(vn, q);
        let rhs = self.neon_reg_bytes(vm, q);
        let old_dst = self.neon_reg_bytes(vd, q);
        let mut out = [0u8; 16];
        let lanes = if q { 4 } else { 2 };

        if opcode == 13 && u && subop == 0 && !op {
            let left0 = f32::from_bits(neon_read_elem(&lhs, 0, 2) as u32);
            let left1 = f32::from_bits(neon_read_elem(&lhs, 1, 2) as u32);
            let right0 = f32::from_bits(neon_read_elem(&rhs, 0, 2) as u32);
            let right1 = f32::from_bits(neon_read_elem(&rhs, 1, 2) as u32);
            neon_write_elem(&mut out, 0, 2, u64::from((left0 + left1).to_bits()));
            neon_write_elem(&mut out, 1, 2, u64::from((right0 + right1).to_bits()));
            self.set_neon_reg_bytes(vd, false, out);
            return Ok(true);
        }

        if opcode == 15 && u && !op {
            let max = subop == 0;
            let pair = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
            let left0 = f32::from_bits(neon_read_elem(&lhs, 0, 2) as u32);
            let left1 = f32::from_bits(neon_read_elem(&lhs, 1, 2) as u32);
            let right0 = f32::from_bits(neon_read_elem(&rhs, 0, 2) as u32);
            let right1 = f32::from_bits(neon_read_elem(&rhs, 1, 2) as u32);
            neon_write_elem(&mut out, 0, 2, u64::from(pair(left0, left1).to_bits()));
            neon_write_elem(&mut out, 1, 2, u64::from(pair(right0, right1).to_bits()));
            self.set_neon_reg_bytes(vd, false, out);
            return Ok(true);
        }

        for lane in 0..lanes {
            let a_bits = neon_read_elem(&lhs, lane, 2) as u32;
            let b_bits = neon_read_elem(&rhs, lane, 2) as u32;
            let d_bits = neon_read_elem(&old_dst, lane, 2) as u32;
            let a = f32::from_bits(a_bits);
            let b = f32::from_bits(b_bits);
            let d = f32::from_bits(d_bits);
            let bits = match (opcode, u, subop, op) {
                (12, false, 0, true) => d.mul_add(a, b).to_bits(),
                (12, false, 1, true) => d.mul_add(a, -b).to_bits(),
                (13, false, 0, false) => (a + b).to_bits(),
                (13, false, 1, false) => (a - b).to_bits(),
                (13, false, 0, true) => (d + a * b).to_bits(),
                (13, false, 1, true) => (d - a * b).to_bits(),
                (13, true, 0, true) => (a * b).to_bits(),
                (13, true, 1, false) => (a - b).abs().to_bits(),
                (14, false, 0, false) => neon_f32_compare(a == b),
                (14, true, 0, false) => neon_f32_compare(a >= b),
                (14, true, 1, false) => neon_f32_compare(a > b),
                (14, true, 0, true) => neon_f32_compare(a.abs() >= b.abs()),
                (14, true, 1, true) => neon_f32_compare(a.abs() > b.abs()),
                (15, false, 0, false) => a.max(b).to_bits(),
                (15, false, 1, false) => a.min(b).to_bits(),
                (15, false, 0, true) => (2.0 - a * b).to_bits(),
                (15, false, 1, true) => ((3.0 - a * b) * 0.5).to_bits(),
                _ => return Ok(false),
            };
            neon_write_elem(&mut out, lane, 2, u64::from(bits));
        }

        self.set_neon_reg_bytes(vd, q, out);
        Ok(true)
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
                if regs == 0 || regs > 32 || first + regs > 32 {
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
                    if dd >= 32 {
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
                if dd >= 32 {
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

        if (instr & 0x0fd0_0f3f) == 0x0e00_0b30 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV from PC"));
            }
            let dn = vfp_double_n(instr);
            let index = (((instr >> 21) & 1) << 1) | ((instr >> 6) & 1);
            let mut bytes = self.neon_reg_bytes(dn, false);
            neon_write_elem(&mut bytes, index as usize, 1, u64::from(self.regs[rt]));
            self.set_dreg_bits(
                dn,
                u64::from_le_bytes(bytes[..8].try_into().expect("VMOV i16 dreg")),
            );
            return Ok(true);
        }

        if (instr & 0x0fd0_0f1f) == 0x0e40_0b10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV from PC"));
            }
            let dn = vfp_double_n(instr);
            let index = (((instr >> 21) & 1) << 2) | ((instr >> 5) & 0x3);
            let mut bytes = self.neon_reg_bytes(dn, false);
            neon_write_elem(&mut bytes, index as usize, 0, u64::from(self.regs[rt]));
            self.set_dreg_bits(
                dn,
                u64::from_le_bytes(bytes[..8].try_into().expect("VMOV i8 dreg")),
            );
            return Ok(true);
        }

        if (instr & 0x0f50_0f3f) == 0x0e10_0b30 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV to PC"));
            }
            let dn = vfp_double_n(instr);
            let index = (((instr >> 21) & 1) << 1) | ((instr >> 6) & 1);
            let bytes = self.neon_reg_bytes(dn, false);
            let value = neon_read_elem(&bytes, index as usize, 1);
            self.write_reg_arm(
                rt,
                if instr & (1 << 23) != 0 {
                    value as u32
                } else {
                    sign_extend(value as u32, 16) as u32
                },
            );
            return Ok(true);
        }

        if (instr & 0x0f50_0f1f) == 0x0e50_0b10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VMOV to PC"));
            }
            let dn = vfp_double_n(instr);
            let index = (((instr >> 21) & 1) << 2) | ((instr >> 5) & 0x3);
            let bytes = self.neon_reg_bytes(dn, false);
            let value = neon_read_elem(&bytes, index as usize, 0);
            self.write_reg_arm(
                rt,
                if instr & (1 << 23) != 0 {
                    value as u32
                } else {
                    sign_extend(value as u32, 8) as u32
                },
            );
            return Ok(true);
        }

        if (instr & 0x0f90_0f5f) == 0x0e80_0b10 {
            let rt = ((instr >> 12) & 0xf) as usize;
            if rt == 15 {
                return Err(Trap::Unpredictable("VFP VDUP from PC"));
            }
            let vd = vfp_double_n(instr);
            let q = instr & (1 << 21) != 0;
            let be = ((instr >> 21) & 0x2) | ((instr >> 5) & 1);
            if be == 3 || (q && vd & 1 != 0) {
                return Err(Trap::UndefinedArm { pc, instr });
            }
            self.check_dreg(vd)?;
            if q {
                self.check_dreg(vd + 1)?;
            }
            let size = match be {
                0 => 2,
                1 => 1,
                2 => 0,
                _ => unreachable!(),
            };
            let mask = neon_elem_mask(size);
            let mut out = [0u8; 16];
            for lane in 0..neon_lanes(q, size) {
                neon_write_elem(&mut out, lane, size, u64::from(self.regs[rt]) & mask);
            }
            self.set_neon_reg_bytes(vd, q, out);
            return Ok(true);
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

        let vfp_imm = instr & 0x0fb0_0ff0;
        if vfp_imm == 0x0eb0_0a00 || vfp_imm == 0x0eb0_0b00 {
            let imm8 = (((instr >> 16) & 0xf) << 4) | (instr & 0xf);
            if vfp_imm == 0x0eb0_0b00 {
                let dd = vfp_double_d(instr);
                self.set_checked_dreg_bits(dd, vfp_expand_imm_f64(imm8 as u8))?;
            } else {
                let sd = vfp_single_d(instr);
                self.sregs[sd] = vfp_expand_imm_f32(imm8 as u8);
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
        if (instr & 0x0ff0_0000) == 0x0c50_0000 || (instr & 0x0ff0_0000) == 0x0c40_0000 {
            let load = instr & (1 << 20) != 0;
            let rt2 = ((instr >> 16) & 0xf) as usize;
            let rt = ((instr >> 12) & 0xf) as usize;
            let coproc = (instr >> 8) & 0xf;
            let opc1 = (instr >> 4) & 0xf;
            let crm = instr & 0xf;

            if coproc == 15 && opc1 == 1 && crm == 14 {
                if rt == 15 || rt2 == 15 || (load && rt == rt2) {
                    return Err(Trap::Unpredictable("CP15 MRRC/MCRR invalid core registers"));
                }
                if !load {
                    return Err(Trap::Privileged {
                        pc,
                        instr,
                        operation: "MCRR CP15 timer",
                    });
                }

                let value = self.cp15_virtual_counter;
                self.cp15_virtual_counter = self.cp15_virtual_counter.wrapping_add(1_000);
                self.write_reg_arm(rt, value as u32);
                self.write_reg_arm(rt2, (value >> 32) as u32);
                return Ok(true);
            }

            return Ok(false);
        }

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
        if (instr & 0x0ff0_0000) == 0x0300_0000 || (instr & 0x0ff0_0000) == 0x0340_0000 {
            let rd = ((instr >> 12) & 0xf) as usize;
            if rd == 15 {
                return Err(Trap::Unpredictable("MOVW/MOVT with PC destination"));
            }
            let imm16 = (((instr >> 16) & 0xf) << 12) | (instr & 0xfff);
            if instr & (1 << 22) != 0 {
                self.write_reg_arm(rd, (self.regs[rd] & 0x0000_ffff) | (imm16 << 16));
            } else {
                self.write_reg_arm(rd, imm16);
            }
            return Ok(true);
        }

        if matches!(instr & 0x0fe0_0070, 0x07a0_0050 | 0x07e0_0050) {
            let rn = (instr & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let widthm1 = (instr >> 16) & 0x1f;
            let lsb = (instr >> 7) & 0x1f;
            if rd == 15 || rn == 15 {
                return Err(Trap::Unpredictable("bitfield extract with PC register"));
            }
            if lsb + widthm1 >= 32 {
                return Err(Trap::Unpredictable("bitfield extract range"));
            }
            let width = widthm1 + 1;
            let raw = self.arm_read_reg(rn, pc) >> lsb;
            let value = if (instr & 0x0fe0_0070) == 0x07a0_0050 {
                sign_extend(raw, width) as u32
            } else if width == 32 {
                raw
            } else {
                raw & ((1u32 << width) - 1)
            };
            self.write_reg_arm(rd, value);
            return Ok(true);
        }

        if (instr & 0x0fe0_0070) == 0x07c0_0010 {
            let rn = (instr & 0xf) as usize;
            let rd = ((instr >> 12) & 0xf) as usize;
            let msb = (instr >> 16) & 0x1f;
            let lsb = (instr >> 7) & 0x1f;
            if rd == 15 {
                return Err(Trap::Unpredictable("bitfield insert with PC destination"));
            }
            if msb < lsb {
                return Err(Trap::Unpredictable("bitfield insert range"));
            }
            let width = msb - lsb + 1;
            let mask = if width == 32 {
                u32::MAX
            } else {
                ((1u32 << width) - 1) << lsb
            };
            let source = if rn == 15 {
                0
            } else {
                self.arm_read_reg(rn, pc) << lsb
            };
            let value = (self.arm_read_reg(rd, pc) & !mask) | (source & mask);
            self.write_reg_arm(rd, value);
            return Ok(true);
        }

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

    pub fn execute_thumb32<M: Memory>(
        &mut self,
        first: u16,
        second: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        let arm_like = (u32::from(first) << 16) | u32::from(second);
        if (arm_like & 0xef00_0000) == 0xef00_0000 {
            let a32_neon = (arm_like & 0xe2ff_ffff) | ((arm_like & (1 << 28)) >> 4) | (1 << 28);
            if self.exec_arm_neon(a32_neon, pc, mem)? {
                return Ok(());
            }
        }
        if (arm_like & 0xff10_0000) == 0xf900_0000 {
            let a32_neon = (arm_like & 0x00ff_ffff) | 0xf400_0000;
            if self.exec_arm_neon(a32_neon, pc, mem)? {
                return Ok(());
            }
        }
        if self.exec_arm_vfp(arm_like, pc, mem)? {
            return Ok(());
        }

        if self.exec_thumb32_hint(first, second)? {
            return Ok(());
        }

        if self.exec_thumb32_exclusive(first, second, pc, mem)? {
            return Ok(());
        }

        if (first & 0xfe00) == 0xea00 && (second & 0x8000) == 0 {
            return self.exec_thumb32_shifted_register(first, second, pc);
        }

        if matches!(first & 0xfff0, 0xf340 | 0xf360 | 0xf3c0) && (second & 0x8020) == 0 {
            return self.exec_thumb32_bitfield(first, second, pc);
        }

        if (first & 0xf800) == 0xf000 && (second & 0x8000) != 0 {
            return self.exec_thumb32_branch(first, second, pc);
        }

        if (first & 0xfa00) == 0xf200 && (second & 0x8000) == 0 {
            if self.exec_thumb32_plain_immediate(first, second, pc)? {
                return Ok(());
            }
        }

        if (first & 0xfa00) == 0xf000 && (second & 0x8000) == 0 {
            return self.exec_thumb32_modified_immediate(first, second, pc);
        }

        if (first & 0xff00) == 0xfb00 {
            if self.exec_thumb32_multiply(first, second, pc)? {
                return Ok(());
            }
        }

        if (first & 0xfff0) == 0xfab0 && (second & 0xf0f0) == 0xf080 {
            let rn = usize::from(first & 0xf);
            let rd = usize::from((second >> 8) & 0xf);
            let rm = usize::from(second & 0xf);
            if rn != rm || rd == 15 || rm == 15 {
                return Err(Trap::UndefinedThumb { pc, instr: first });
            }
            self.regs[rd] = self.regs[rm].leading_zeros();
            return Ok(());
        }

        if (first & 0xff80) == 0xfa00 && (second & 0xf0f0) == 0xf000 {
            return self.exec_thumb32_register_shift(first, second, pc);
        }

        if first == 0xe8bd {
            return self.exec_thumb32_pop(second, pc, mem);
        }

        if (first & 0xfff0) == 0xe8d0 && (second & 0xffe0) == 0xf000 {
            return self.exec_thumb32_table_branch(first, second, pc, mem);
        }

        if matches!(first & 0xffc0, 0xe880 | 0xe900) {
            return self.exec_thumb32_block_transfer(first, second, pc, mem);
        }

        if (first & 0xfe40) == 0xe840 && (first & 0x0120) != 0 {
            return self.exec_thumb32_doubleword_transfer(first, second, pc, mem);
        }

        if matches!(
            first & 0xfff0,
            0xf800
                | 0xf810
                | 0xf820
                | 0xf830
                | 0xf840
                | 0xf850
                | 0xf880
                | 0xf890
                | 0xf8a0
                | 0xf8b0
                | 0xf8c0
                | 0xf8d0
                | 0xf910
                | 0xf930
                | 0xf990
                | 0xf9b0
        ) {
            return self.exec_thumb32_immediate_transfer(first, second, pc, mem);
        }

        Err(Trap::UndefinedThumb { pc, instr: first })
    }

    fn exec_thumb32_hint(&mut self, first: u16, second: u16) -> Result<bool> {
        if first == 0xf3af && (second & 0xfff0) == 0x8000 {
            return Ok(true);
        }
        if first == 0xf3bf && (second & 0xfff0) == 0x8f20 {
            self.exclusive_reservation = None;
            return Ok(true);
        }
        if first == 0xf3bf && matches!(second & 0xfff0, 0x8f40 | 0x8f50 | 0x8f60) {
            return Ok(true);
        }
        Ok(false)
    }

    fn exec_thumb32_register_shift(&mut self, first: u16, second: u16, pc: u32) -> Result<()> {
        let op = (first >> 5) & 0x3;
        let set_flags = first & (1 << 4) != 0;
        let rm = usize::from(first & 0xf);
        let rd = usize::from((second >> 8) & 0xf);
        let rs = usize::from(second & 0xf);
        if rd == 15 || rm == 15 || rs == 15 {
            return Err(Trap::Unpredictable("Thumb register shift with PC register"));
        }
        let shift = match op {
            0 => Shift::Lsl,
            1 => Shift::Lsr,
            2 => Shift::Asr,
            3 => Shift::Ror,
            _ => unreachable!(),
        };
        let shifted = shift_operand(
            self.regs[rm],
            shift,
            self.regs[rs] & 0xff,
            self.cpsr.c,
            false,
        );
        self.regs[rd] = shifted.value;
        if set_flags {
            self.cpsr.set_nzc(shifted.value, shifted.carry);
        }
        let _ = pc;
        Ok(())
    }

    fn exec_thumb32_shifted_register(&mut self, first: u16, second: u16, pc: u32) -> Result<()> {
        let op = (first >> 5) & 0xf;
        let set_flags = first & (1 << 4) != 0;
        let rn = usize::from(first & 0xf);
        let rd = usize::from((second >> 8) & 0xf);
        let rm = usize::from(second & 0xf);
        if rm == 15 {
            return Err(Trap::Unpredictable(
                "Thumb shifted register with PC operand",
            ));
        }

        let imm3 = u32::from((second >> 12) & 0x7);
        let imm2 = u32::from((second >> 6) & 0x3);
        let shift = decode_arm_shift(u32::from((second >> 4) & 0x3));
        let shifted = shift_operand(self.regs[rm], shift, (imm3 << 2) | imm2, self.cpsr.c, true);
        let lhs = if rn == 15 {
            if matches!(op, 0x8 | 0xd | 0xe) {
                pc.wrapping_add(4) & !3
            } else {
                0
            }
        } else {
            self.regs[rn]
        };

        match op {
            0x0 | 0x1 | 0x2 | 0x3 | 0x4 => {
                if rd == 15 && !matches!(op, 0x0 | 0x4) {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let value = match op {
                    0x0 => lhs & shifted.value,
                    0x1 => lhs & !shifted.value,
                    0x2 => lhs | shifted.value,
                    0x3 => lhs | !shifted.value,
                    0x4 => lhs ^ shifted.value,
                    _ => unreachable!(),
                };
                if rd == 15 {
                    self.cpsr.set_nzc(value, shifted.carry);
                } else {
                    self.regs[rd] = value;
                    if set_flags {
                        self.cpsr.set_nzc(value, shifted.carry);
                    }
                }
                Ok(())
            }
            0x8 | 0xa | 0xb | 0xd | 0xe => {
                if rd == 15 && !matches!(op, 0x8 | 0xd) {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let (value, carry, overflow) = match op {
                    0x8 => add_with_flags(lhs, shifted.value),
                    0xa => add_with_carry(lhs, shifted.value, self.cpsr.c),
                    0xb => add_with_carry(lhs, !shifted.value, self.cpsr.c),
                    0xd => sub_with_flags(lhs, shifted.value),
                    0xe => sub_with_flags(shifted.value, lhs),
                    _ => unreachable!(),
                };
                if rd == 15 {
                    self.cpsr.set_nzcv(value, carry, overflow);
                } else {
                    self.regs[rd] = value;
                    if set_flags {
                        self.cpsr.set_nzcv(value, carry, overflow);
                    }
                }
                Ok(())
            }
            _ => Err(Trap::UndefinedThumb { pc, instr: first }),
        }
    }

    fn exec_thumb32_multiply(&mut self, first: u16, second: u16, pc: u32) -> Result<bool> {
        let op1 = (first >> 4) & 0xf;
        let rn = usize::from(first & 0xf);
        let ra = usize::from((second >> 12) & 0xf);
        let rd = usize::from((second >> 8) & 0xf);
        let op2 = (second >> 4) & 0xf;
        let rm = usize::from(second & 0xf);

        match (op1, op2) {
            (0, 0) => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let product = self.regs[rn].wrapping_mul(self.regs[rm]);
                self.regs[rd] = if ra == 15 {
                    product
                } else {
                    product.wrapping_add(self.regs[ra])
                };
                Ok(true)
            }
            (0, 1) => {
                if [rd, rn, rm, ra].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                self.regs[rd] =
                    self.regs[ra].wrapping_sub(self.regs[rn].wrapping_mul(self.regs[rm]));
                Ok(true)
            }
            (1, 0..=3) => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let n_half = select_i16(self.regs[rn], op2 & 2 != 0);
                let m_half = select_i16(self.regs[rm], op2 & 1 != 0);
                let product = i64::from(n_half) * i64::from(m_half);
                let result = if ra == 15 {
                    product
                } else {
                    product + i64::from(self.regs[ra] as i32)
                };
                if ra != 15 && (result > i64::from(i32::MAX) || result < i64::from(i32::MIN)) {
                    self.cpsr.q = true;
                }
                self.regs[rd] = result as i32 as u32;
                Ok(true)
            }
            (2, 0 | 1) | (4, 0 | 1) => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let (lo, hi) = signed_dual_products(self.regs[rn], self.regs[rm], op2 & 1 != 0);
                let product = if op1 == 2 {
                    i64::from(lo) + i64::from(hi)
                } else {
                    i64::from(lo) - i64::from(hi)
                };
                let result = if ra == 15 {
                    product
                } else {
                    product + i64::from(self.regs[ra] as i32)
                };
                if result > i64::from(i32::MAX) || result < i64::from(i32::MIN) {
                    self.cpsr.q = true;
                }
                self.regs[rd] = result as i32 as u32;
                Ok(true)
            }
            (3, 0 | 1) => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let half = select_i16(self.regs[rm], op2 & 1 != 0);
                let product = i64::from(self.regs[rn] as i32) * i64::from(half);
                let word = product >> 16;
                let result = if ra == 15 {
                    word
                } else {
                    word + i64::from(self.regs[ra] as i32)
                };
                if ra != 15 && (result > i64::from(i32::MAX) || result < i64::from(i32::MIN)) {
                    self.cpsr.q = true;
                }
                self.regs[rd] = result as i32 as u32;
                Ok(true)
            }
            (5, 0 | 1) | (6, 0 | 1) => {
                if [rd, rn, rm].contains(&15) || (op1 == 6 && ra == 15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let product =
                    i64::from(self.regs[rn] as i32).wrapping_mul(i64::from(self.regs[rm] as i32));
                let result = if ra == 15 {
                    signed_high_word(product, op2 & 1 != 0)
                } else {
                    let addend = (u64::from(self.regs[ra]) << 32) as i64;
                    let value = if op1 == 5 {
                        addend.wrapping_add(product)
                    } else {
                        addend.wrapping_sub(product)
                    };
                    signed_high_word(value, op2 & 1 != 0)
                };
                self.regs[rd] = result;
                Ok(true)
            }
            (7, 0) => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                let mut sum = 0u32;
                for lane in 0..4 {
                    let shift = lane * 8;
                    let n = ((self.regs[rn] >> shift) & 0xff) as u8;
                    let m = ((self.regs[rm] >> shift) & 0xff) as u8;
                    sum = sum.wrapping_add(u32::from(n.abs_diff(m)));
                }
                self.regs[rd] = if ra == 15 {
                    sum
                } else {
                    sum.wrapping_add(self.regs[ra])
                };
                Ok(true)
            }
            (8 | 10 | 12 | 14, 0) => {
                let rd_lo = ra;
                let rd_hi = rd;
                if [rd_lo, rd_hi, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                if rd_lo == rd_hi {
                    return Err(Trap::Unpredictable(
                        "Thumb long multiply with same high/low register",
                    ));
                }
                let signed = matches!(op1, 8 | 12);
                let accumulate = matches!(op1, 12 | 14);
                let product = if signed {
                    (self.regs[rn] as i32 as i64).wrapping_mul(self.regs[rm] as i32 as i64) as u64
                } else {
                    u64::from(self.regs[rn]).wrapping_mul(u64::from(self.regs[rm]))
                };
                let addend = if accumulate {
                    (u64::from(self.regs[rd_hi]) << 32) | u64::from(self.regs[rd_lo])
                } else {
                    0
                };
                let result = product.wrapping_add(addend);
                self.regs[rd_lo] = result as u32;
                self.regs[rd_hi] = (result >> 32) as u32;
                Ok(true)
            }
            (12, 8..=13) | (13, 12 | 13) => {
                let rd_lo = ra;
                let rd_hi = rd;
                if [rd_lo, rd_hi, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                if rd_lo == rd_hi {
                    return Err(Trap::Unpredictable(
                        "Thumb long multiply with same high/low register",
                    ));
                }
                let product = if op2 < 12 {
                    let n_half = select_i16(self.regs[rn], op2 & 2 != 0);
                    let m_half = select_i16(self.regs[rm], op2 & 1 != 0);
                    i64::from(n_half) * i64::from(m_half)
                } else {
                    let (lo, hi) = signed_dual_products(self.regs[rn], self.regs[rm], op2 & 1 != 0);
                    if op1 == 13 {
                        i64::from(lo) - i64::from(hi)
                    } else {
                        i64::from(lo) + i64::from(hi)
                    }
                };
                let addend =
                    ((u64::from(self.regs[rd_hi]) << 32) | u64::from(self.regs[rd_lo])) as i64;
                let result = addend.wrapping_add(product);
                self.regs[rd_lo] = result as u32;
                self.regs[rd_hi] = (result >> 32) as u32;
                Ok(true)
            }
            (14, 6) => {
                let rd_lo = ra;
                let rd_hi = rd;
                if [rd_lo, rd_hi, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb multiply with PC register"));
                }
                if rd_lo == rd_hi {
                    return Err(Trap::Unpredictable(
                        "Thumb long multiply with same high/low register",
                    ));
                }
                let result = u64::from(self.regs[rn])
                    .wrapping_mul(u64::from(self.regs[rm]))
                    .wrapping_add(u64::from(self.regs[rd_lo]))
                    .wrapping_add(u64::from(self.regs[rd_hi]));
                self.regs[rd_lo] = result as u32;
                self.regs[rd_hi] = (result >> 32) as u32;
                Ok(true)
            }
            (9 | 11, 15) if ra == 15 => {
                if [rd, rn, rm].contains(&15) {
                    return Err(Trap::Unpredictable("Thumb divide with PC register"));
                }
                self.regs[rd] = if self.regs[rm] == 0 {
                    0
                } else if op1 == 9 {
                    (self.regs[rn] as i32).wrapping_div(self.regs[rm] as i32) as u32
                } else {
                    self.regs[rn] / self.regs[rm]
                };
                Ok(true)
            }
            _ => {
                let _ = pc;
                Ok(false)
            }
        }
    }

    fn exec_thumb32_bitfield(&mut self, first: u16, second: u16, pc: u32) -> Result<()> {
        let rn = usize::from(first & 0xf);
        let rd = usize::from((second >> 8) & 0xf);
        if rd == 15 {
            return Err(Trap::Unpredictable(
                "Thumb bitfield instruction with PC destination",
            ));
        }

        let imm3 = u32::from((second >> 12) & 0x7);
        let imm2 = u32::from((second >> 6) & 0x3);
        let lsb = (imm3 << 2) | imm2;
        let width_or_msb = u32::from(second & 0x1f);

        match first & 0xfff0 {
            0xf340 | 0xf3c0 => {
                if rn == 15 {
                    return Err(Trap::Unpredictable("Thumb bitfield extract with PC source"));
                }
                let width = width_or_msb + 1;
                if lsb + width > 32 {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let raw = if width == 32 {
                    self.regs[rn]
                } else {
                    (self.regs[rn] >> lsb) & ((1u32 << width) - 1)
                };
                self.regs[rd] = if (first & 0xfff0) == 0xf340 {
                    sign_extend(raw, width) as u32
                } else {
                    raw
                };
                Ok(())
            }
            0xf360 => {
                if width_or_msb < lsb {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let width = width_or_msb - lsb + 1;
                let field_mask = if width == 32 {
                    u32::MAX
                } else {
                    ((1u32 << width) - 1) << lsb
                };
                let insert = if rn == 15 {
                    0
                } else {
                    (self.regs[rn] << lsb) & field_mask
                };
                self.regs[rd] = (self.regs[rd] & !field_mask) | insert;
                Ok(())
            }
            _ => Err(Trap::UndefinedThumb { pc, instr: first }),
        }
    }

    fn exec_thumb32_exclusive<M: Memory>(
        &mut self,
        first: u16,
        second: u16,
        _pc: u32,
        mem: &mut M,
    ) -> Result<bool> {
        if (first & 0xfff0) == 0xe8d0
            && (second & 0x0f0f) == 0x0f0f
            && matches!((second >> 4) & 0xf, 4 | 5)
        {
            let rn = usize::from(first & 0xf);
            let rt = usize::from((second >> 12) & 0xf);
            if rn == 15 || rt == 15 {
                return Err(Trap::Unpredictable("Thumb LDREX with PC register"));
            }
            let addr = self.regs[rn];
            let (value, size) = if (second & 0x00f0) == 0x0040 {
                (u32::from(mem.load8(addr)?), 1)
            } else {
                (u32::from(mem.load16(addr)?), 2)
            };
            self.regs[rt] = value;
            self.exclusive_reservation = Some(ExclusiveReservation { addr, size });
            return Ok(true);
        }

        if (first & 0xfff0) == 0xe850 && (second & 0x0f00) == 0x0f00 {
            let rn = usize::from(first & 0xf);
            let rt = usize::from((second >> 12) & 0xf);
            if rn == 15 || rt == 15 {
                return Err(Trap::Unpredictable("Thumb LDREX with PC register"));
            }
            let addr = self.regs[rn].wrapping_add(u32::from(second & 0xff) << 2);
            self.regs[rt] = mem.load32(addr)?;
            self.exclusive_reservation = Some(ExclusiveReservation { addr, size: 4 });
            return Ok(true);
        }

        if (first & 0xfff0) == 0xe8c0
            && (second & 0x0f00) == 0x0f00
            && matches!((second >> 4) & 0xf, 4 | 5)
        {
            let rn = usize::from(first & 0xf);
            let rt = usize::from((second >> 12) & 0xf);
            let rd = usize::from(second & 0xf);
            if rn == 15 || rd == 15 || rt == 15 {
                return Err(Trap::Unpredictable("Thumb STREX with PC register"));
            }
            if rd == rn || rd == rt {
                return Err(Trap::Unpredictable(
                    "Thumb STREX status register overlaps operand",
                ));
            }
            let addr = self.regs[rn];
            let size = if (second & 0x00f0) == 0x0040 { 1 } else { 2 };
            if self
                .exclusive_reservation
                .map(|reservation| reservation.addr == addr && reservation.size == size)
                .unwrap_or(false)
            {
                if size == 1 {
                    mem.store8(addr, self.regs[rt] as u8)?;
                } else {
                    mem.store16(addr, self.regs[rt] as u16)?;
                }
                self.regs[rd] = 0;
            } else {
                self.regs[rd] = 1;
            }
            self.exclusive_reservation = None;
            return Ok(true);
        }

        if (first & 0xfff0) == 0xe840 {
            let rn = usize::from(first & 0xf);
            let rt = usize::from((second >> 12) & 0xf);
            let rd = usize::from((second >> 8) & 0xf);
            if rn == 15 || rd == 15 || rt == 15 {
                return Err(Trap::Unpredictable("Thumb STREX with PC register"));
            }
            if rd == rn || rd == rt {
                return Err(Trap::Unpredictable(
                    "Thumb STREX status register overlaps operand",
                ));
            }
            let addr = self.regs[rn].wrapping_add(u32::from(second & 0xff) << 2);
            if self
                .exclusive_reservation
                .map(|reservation| reservation.addr == addr && reservation.size == 4)
                .unwrap_or(false)
            {
                mem.store32(addr, self.regs[rt])?;
                self.regs[rd] = 0;
            } else {
                self.regs[rd] = 1;
            }
            self.exclusive_reservation = None;
            return Ok(true);
        }

        Ok(false)
    }

    fn exec_thumb32_plain_immediate(&mut self, first: u16, second: u16, pc: u32) -> Result<bool> {
        let rd = usize::from((second >> 8) & 0xf);
        if rd == 15 {
            return Err(Trap::Unpredictable(
                "Thumb plain immediate with PC destination",
            ));
        }

        let imm12 = (u32::from((first >> 10) & 1) << 11)
            | (u32::from((second >> 12) & 0x7) << 8)
            | u32::from(second & 0xff);
        let imm16 = (u32::from(first & 0xf) << 12) | imm12;

        match first & 0xfbf0 {
            0xf200 => {
                let rn = usize::from(first & 0xf);
                let lhs = if rn == 15 {
                    pc.wrapping_add(4) & !3
                } else {
                    self.regs[rn]
                };
                self.regs[rd] = lhs.wrapping_add(imm12);
                Ok(true)
            }
            0xf2a0 => {
                let rn = usize::from(first & 0xf);
                let lhs = if rn == 15 {
                    pc.wrapping_add(4) & !3
                } else {
                    self.regs[rn]
                };
                self.regs[rd] = lhs.wrapping_sub(imm12);
                Ok(true)
            }
            0xf240 => {
                self.regs[rd] = imm16;
                Ok(true)
            }
            0xf2c0 => {
                self.regs[rd] = (self.regs[rd] & 0x0000_ffff) | (imm16 << 16);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn exec_thumb32_modified_immediate(&mut self, first: u16, second: u16, pc: u32) -> Result<()> {
        let op = (first >> 5) & 0xf;
        let set_flags = first & (1 << 4) != 0;
        let rn = usize::from(first & 0xf);
        let rd = usize::from((second >> 8) & 0xf);
        let imm12 = (u32::from((first >> 10) & 1) << 11)
            | (u32::from((second >> 12) & 0x7) << 8)
            | u32::from(second & 0xff);
        let (imm, carry) = thumb_expand_imm(imm12, self.cpsr.c);

        match op {
            0x0 | 0x1 | 0x2 | 0x3 | 0x4 => {
                if rd == 15 && !matches!(op, 0x0 | 0x4) {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let lhs = if rn == 15 { 0 } else { self.regs[rn] };
                let value = match op {
                    0x0 => lhs & imm,
                    0x1 => lhs & !imm,
                    0x2 => lhs | imm,
                    0x3 => lhs | !imm,
                    0x4 => lhs ^ imm,
                    _ => unreachable!(),
                };
                if rd == 15 {
                    self.cpsr.set_nzc(value, carry);
                } else {
                    self.regs[rd] = value;
                    if set_flags {
                        self.cpsr.set_nzc(value, carry);
                    }
                }
                Ok(())
            }
            0x8 | 0xa | 0xb | 0xd | 0xe => {
                if rd == 15 && !matches!(op, 0x8 | 0xd) {
                    return Err(Trap::UndefinedThumb { pc, instr: first });
                }
                let lhs = if rn == 15 {
                    pc.wrapping_add(4) & !3
                } else {
                    self.regs[rn]
                };
                let (value, carry, overflow) = match op {
                    0x8 => add_with_flags(lhs, imm),
                    0xa => add_with_carry(lhs, imm, self.cpsr.c),
                    0xb => add_with_carry(lhs, !imm, self.cpsr.c),
                    0xd => sub_with_flags(lhs, imm),
                    0xe => sub_with_flags(imm, lhs),
                    _ => unreachable!(),
                };
                if rd == 15 {
                    self.cpsr.set_nzcv(value, carry, overflow);
                } else {
                    self.regs[rd] = value;
                    if set_flags {
                        self.cpsr.set_nzcv(value, carry, overflow);
                    }
                }
                Ok(())
            }
            _ => Err(Trap::UndefinedThumb { pc, instr: first }),
        }
    }

    fn exec_thumb32_branch(&mut self, first: u16, second: u16, pc: u32) -> Result<()> {
        if (second >> 14) & 0x3 == 0b10 && second & (1 << 12) == 0 {
            let cond = u32::from((first >> 6) & 0xf);
            if cond >= 0xe {
                return Err(Trap::UndefinedThumb { pc, instr: first });
            }

            let s = u32::from((first >> 10) & 1);
            let imm6 = u32::from(first & 0x003f);
            let i1 = u32::from((second >> 13) & 1);
            let i2 = u32::from((second >> 11) & 1);
            let imm11 = u32::from(second & 0x07ff);
            let imm = sign_extend(
                (s << 20) | (i2 << 19) | (i1 << 18) | (imm6 << 12) | (imm11 << 1),
                21,
            );
            if condition_passed(cond, self.cpsr) {
                self.regs[15] = pc.wrapping_add(4).wrapping_add(imm as u32);
            }
            return Ok(());
        }

        let s = u32::from((first >> 10) & 1);
        let imm10 = u32::from(first & 0x03ff);
        let j1 = u32::from((second >> 13) & 1);
        let j2 = u32::from((second >> 11) & 1);
        let imm11 = u32::from(second & 0x07ff);
        let i1 = u32::from((j1 ^ s) == 0);
        let i2 = u32::from((j2 ^ s) == 0);
        let imm = sign_extend(
            (s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1),
            25,
        );

        match (second >> 14) & 0x3 {
            0b10 => {
                self.regs[15] = pc.wrapping_add(4).wrapping_add(imm as u32);
                Ok(())
            }
            0b11 if second & (1 << 12) == 0 => {
                self.regs[14] = pc.wrapping_add(4) | 1;
                let target = (pc.wrapping_add(4) & !3).wrapping_add(imm as u32) & !3;
                self.branch_exchange(target);
                Ok(())
            }
            0b11 => {
                self.regs[14] = pc.wrapping_add(4) | 1;
                self.regs[15] = pc.wrapping_add(4).wrapping_add(imm as u32);
                Ok(())
            }
            _ => Err(Trap::UndefinedThumb { pc, instr: first }),
        }
    }

    fn exec_thumb32_pop<M: Memory>(&mut self, reglist: u16, pc: u32, mem: &mut M) -> Result<()> {
        if reglist == 0 {
            return Err(Trap::Unpredictable("empty Thumb pop register list"));
        }
        let mut addr = self.regs[13];
        for reg in 0..16 {
            if reglist & (1 << reg) == 0 {
                continue;
            }
            let value = mem.load32(addr)?;
            addr = addr.wrapping_add(4);
            if reg == 15 {
                self.branch_exchange(value);
            } else {
                self.regs[reg] = value;
            }
        }
        self.regs[13] = addr;
        if reglist & (1 << 15) == 0 {
            self.regs[15] = pc.wrapping_add(4);
        }
        Ok(())
    }

    fn exec_thumb32_table_branch<M: Memory>(
        &mut self,
        first: u16,
        second: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        let rn = usize::from(first & 0xf);
        let rm = usize::from(second & 0xf);
        if rm == 15 {
            return Err(Trap::Unpredictable(
                "Thumb table branch with PC index register",
            ));
        }

        let base = if rn == 15 {
            pc.wrapping_add(4)
        } else {
            self.regs[rn]
        };
        let index = self.regs[rm];
        let table_value = if second & (1 << 4) != 0 {
            u32::from(mem.load16(base.wrapping_add(index.wrapping_mul(2)))?)
        } else {
            u32::from(mem.load8(base.wrapping_add(index))?)
        };
        self.regs[15] = pc.wrapping_add(4).wrapping_add(table_value << 1);
        Ok(())
    }

    fn exec_thumb32_block_transfer<M: Memory>(
        &mut self,
        first: u16,
        reglist: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        if reglist == 0 {
            return Err(Trap::Unpredictable("empty Thumb load/store multiple list"));
        }
        let rn = usize::from(first & 0xf);
        if rn == 15 {
            return Err(Trap::Unpredictable(
                "Thumb load/store multiple with PC base register",
            ));
        }
        let load = first & (1 << 4) != 0;
        let writeback = first & (1 << 5) != 0;
        let decrement_before = (first & 0xffc0) == 0xe900;
        if !load && reglist & (1 << 15) != 0 {
            return Err(Trap::Unpredictable("Thumb store multiple with PC register"));
        }
        if load && writeback && reglist & (1 << rn) != 0 {
            return Err(Trap::Unpredictable(
                "Thumb load multiple writeback with base in list",
            ));
        }
        if !load && writeback && reglist & (1 << rn) != 0 && rn as u32 != reglist.trailing_zeros() {
            return Err(Trap::Unpredictable(
                "Thumb store multiple writeback base not first in register list",
            ));
        }

        let count = reglist.count_ones();
        let base = self.regs[rn];
        let mut addr = if decrement_before {
            base.wrapping_sub(count * 4)
        } else {
            base
        };
        for reg in 0..16 {
            if reglist & (1 << reg) == 0 {
                continue;
            }
            if load {
                let value = mem.load32(addr)?;
                if reg == 15 {
                    self.branch_exchange(value);
                } else {
                    self.regs[reg] = value;
                }
            } else {
                self.store32(mem, addr, self.regs[reg])?;
            }
            addr = addr.wrapping_add(4);
        }
        if writeback {
            self.regs[rn] = if decrement_before {
                base.wrapping_sub(count * 4)
            } else {
                base.wrapping_add(count * 4)
            };
        }
        let _ = pc;
        Ok(())
    }

    fn exec_thumb32_doubleword_transfer<M: Memory>(
        &mut self,
        first: u16,
        second: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        let rn = usize::from(first & 0xf);
        let rt = usize::from((second >> 12) & 0xf);
        let rt2 = usize::from((second >> 8) & 0xf);
        if rn == 15 || rt == 15 || rt2 == 15 {
            return Err(Trap::Unpredictable(
                "Thumb doubleword transfer with PC register",
            ));
        }
        if rt == rt2 {
            return Err(Trap::Unpredictable(
                "Thumb doubleword transfer with duplicate destination",
            ));
        }
        let load = first & (1 << 4) != 0;
        let writeback = first & (1 << 5) != 0;
        let pre_index = first & (1 << 8) != 0;
        let add = first & (1 << 7) != 0;
        if load && writeback && (rn == rt || rn == rt2) {
            return Err(Trap::Unpredictable(
                "Thumb LDRD writeback with base as destination",
            ));
        }

        let base = self.regs[rn];
        let offset = u32::from(second & 0xff) << 2;
        let offset_addr = if add {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if pre_index { offset_addr } else { base };
        if load {
            self.regs[rt] = mem.load32(addr)?;
            self.regs[rt2] = mem.load32(addr.wrapping_add(4))?;
        } else {
            self.store32(mem, addr, self.regs[rt])?;
            self.store32(mem, addr.wrapping_add(4), self.regs[rt2])?;
        }
        if writeback {
            self.regs[rn] = offset_addr;
        }
        let _ = pc;
        Ok(())
    }

    fn exec_thumb32_immediate_transfer<M: Memory>(
        &mut self,
        first: u16,
        second: u16,
        pc: u32,
        mem: &mut M,
    ) -> Result<()> {
        let op = first & 0xfff0;
        let signed_load = matches!(op, 0xf910 | 0xf930 | 0xf990 | 0xf9b0);
        let imm12_form = matches!(
            op,
            0xf880 | 0xf890 | 0xf8a0 | 0xf8b0 | 0xf8c0 | 0xf8d0 | 0xf990 | 0xf9b0
        );
        let load = signed_load || matches!(op, 0xf810 | 0xf830 | 0xf850 | 0xf890 | 0xf8b0 | 0xf8d0);
        let size = match op {
            0xf800 | 0xf810 | 0xf880 | 0xf890 | 0xf910 | 0xf990 => 1,
            0xf820 | 0xf830 | 0xf8a0 | 0xf8b0 | 0xf930 | 0xf9b0 => 2,
            0xf840 | 0xf850 | 0xf8c0 | 0xf8d0 => 4,
            _ => unreachable!(),
        };
        let rn = usize::from(first & 0xf);
        let rt = usize::from((second >> 12) & 0xf);
        if rn == 15 {
            if !load {
                return Err(Trap::Unpredictable(
                    "Thumb immediate store with PC base register",
                ));
            }
            let base = pc.wrapping_add(4) & !3;
            let offset = u32::from(second & 0x0fff);
            let addr = if first & (1 << 7) != 0 {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            match size {
                1 => {
                    if rt == 15 {
                        return Err(Trap::Unpredictable("Thumb byte load with PC destination"));
                    }
                    let value = u32::from(mem.load8(addr)?);
                    self.regs[rt] = if signed_load {
                        sign_extend(value, 8) as u32
                    } else {
                        value
                    };
                }
                2 => {
                    if rt == 15 {
                        return Err(Trap::Unpredictable(
                            "Thumb halfword load with PC destination",
                        ));
                    }
                    let value = u32::from(mem.load16(addr)?);
                    self.regs[rt] = if signed_load {
                        sign_extend(value, 16) as u32
                    } else {
                        value
                    };
                }
                4 => {
                    let value = mem.load32(addr)?;
                    if rt == 15 {
                        self.branch_exchange(value);
                    } else {
                        self.regs[rt] = value;
                    }
                }
                _ => unreachable!(),
            }
            return Ok(());
        }

        if !imm12_form && second & (1 << 11) == 0 {
            if second & 0x0fc0 != 0 {
                return Err(Trap::UndefinedThumb { pc, instr: first });
            }
            let rm = usize::from(second & 0xf);
            if rm == 15 {
                return Err(Trap::Unpredictable(
                    "Thumb register transfer with PC offset register",
                ));
            }
            let shift = u32::from((second >> 4) & 0x3);
            let addr = self.regs[rn].wrapping_add(self.regs[rm] << shift);
            if load {
                match size {
                    1 => {
                        if rt == 15 {
                            return Err(Trap::Unpredictable("Thumb byte load with PC destination"));
                        }
                        let value = u32::from(mem.load8(addr)?);
                        self.regs[rt] = if signed_load {
                            sign_extend(value, 8) as u32
                        } else {
                            value
                        };
                    }
                    2 => {
                        if rt == 15 {
                            return Err(Trap::Unpredictable(
                                "Thumb halfword load with PC destination",
                            ));
                        }
                        let value = u32::from(mem.load16(addr)?);
                        self.regs[rt] = if signed_load {
                            sign_extend(value, 16) as u32
                        } else {
                            value
                        };
                    }
                    4 => {
                        let value = mem.load32(addr)?;
                        if rt == 15 {
                            self.branch_exchange(value);
                        } else {
                            self.regs[rt] = value;
                        }
                    }
                    _ => unreachable!(),
                }
            } else {
                if rt == 15 {
                    return Err(Trap::Unpredictable(
                        "Thumb register store with PC source register",
                    ));
                }
                match size {
                    1 => self.store8(mem, addr, self.regs[rt] as u8)?,
                    2 => self.store16(mem, addr, self.regs[rt] as u16)?,
                    4 => self.store32(mem, addr, self.regs[rt])?,
                    _ => unreachable!(),
                }
            }
            return Ok(());
        }

        let (addr, writeback_addr, writeback) = if imm12_form {
            (
                self.regs[rn].wrapping_add(u32::from(second & 0x0fff)),
                0,
                false,
            )
        } else {
            if second & (1 << 11) == 0 {
                return Err(Trap::UndefinedThumb { pc, instr: first });
            }
            let pre_index = second & (1 << 10) != 0;
            let add = second & (1 << 9) != 0;
            let writeback = second & (1 << 8) != 0;
            if !pre_index && !writeback {
                return Err(Trap::UndefinedThumb { pc, instr: first });
            }
            let offset = u32::from(second & 0x00ff);
            let base = self.regs[rn];
            let offset_addr = if add {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            let addr = if pre_index { offset_addr } else { base };
            (addr, offset_addr, writeback)
        };
        if load {
            if writeback && rn == rt {
                return Err(Trap::Unpredictable(
                    "Thumb immediate load writeback overlaps destination",
                ));
            }
            match size {
                1 => {
                    if rt == 15 {
                        return Err(Trap::Unpredictable("Thumb byte load with PC destination"));
                    }
                    let value = u32::from(mem.load8(addr)?);
                    self.regs[rt] = if signed_load {
                        sign_extend(value, 8) as u32
                    } else {
                        value
                    };
                }
                2 => {
                    if rt == 15 {
                        return Err(Trap::Unpredictable(
                            "Thumb halfword load with PC destination",
                        ));
                    }
                    let value = u32::from(mem.load16(addr)?);
                    self.regs[rt] = if signed_load {
                        sign_extend(value, 16) as u32
                    } else {
                        value
                    };
                }
                4 => {
                    let value = mem.load32(addr)?;
                    if writeback {
                        self.regs[rn] = writeback_addr;
                    }
                    if rt == 15 {
                        self.branch_exchange(value);
                    } else {
                        self.regs[rt] = value;
                    }
                }
                _ => unreachable!(),
            }
            if writeback && size != 4 {
                self.regs[rn] = writeback_addr;
            }
        } else {
            if rt == 15 {
                return Err(Trap::Unpredictable(
                    "Thumb immediate store with PC source register",
                ));
            }
            match size {
                1 => self.store8(mem, addr, self.regs[rt] as u8)?,
                2 => self.store16(mem, addr, self.regs[rt] as u16)?,
                4 => self.store32(mem, addr, self.regs[rt])?,
                _ => unreachable!(),
            }
            if writeback {
                self.regs[rn] = writeback_addr;
            }
        }
        Ok(())
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

        if instr & 0xff00 == 0xbf00 {
            let mask = instr & 0xf;
            if mask == 0 {
                return Ok(());
            }
            let cond = u32::from((instr >> 4) & 0xf);
            return self.start_thumb_it(cond, mask, pc, instr);
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

        if instr & 0xf500 == 0xb100 {
            let rn = (instr & 0x7) as usize;
            let nonzero = instr & 0x0800 != 0;
            let imm = (u32::from((instr >> 9) & 1) << 6) | (u32::from((instr >> 3) & 0x1f) << 1);
            if (self.regs[rn] != 0) == nonzero {
                self.regs[15] = pc.wrapping_add(4).wrapping_add(imm);
            }
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

    fn neon_reg_bytes(&self, dreg: usize, q: bool) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.dreg_bits(dreg).to_le_bytes());
        if q {
            bytes[8..].copy_from_slice(&self.dreg_bits(dreg + 1).to_le_bytes());
        }
        bytes
    }

    fn set_neon_reg_bytes(&mut self, dreg: usize, q: bool, bytes: [u8; 16]) {
        self.set_dreg_bits(
            dreg,
            u64::from_le_bytes(bytes[..8].try_into().expect("D register is 8 bytes")),
        );
        if q {
            self.set_dreg_bits(
                dreg + 1,
                u64::from_le_bytes(bytes[8..].try_into().expect("D register is 8 bytes")),
            );
        }
    }

    fn check_neon_3same_regs(
        &self,
        vd: usize,
        vn: usize,
        vm: usize,
        q: bool,
        pc: u32,
        instr: u32,
    ) -> Result<()> {
        self.check_dreg(vd)?;
        self.check_dreg(vn)?;
        self.check_dreg(vm)?;
        if q && (vd & 1 != 0 || vn & 1 != 0 || vm & 1 != 0) {
            return Err(Trap::UndefinedArm { pc, instr });
        }
        if q {
            self.check_dreg(vd + 1)?;
            self.check_dreg(vn + 1)?;
            self.check_dreg(vm + 1)?;
        }
        Ok(())
    }

    fn neon_saturating_add(&mut self, a: u64, b: u64, size: usize, signed: bool) -> u64 {
        let bits = neon_elem_bits(size);
        if signed {
            let min = -(1i128 << (bits - 1));
            let max = (1i128 << (bits - 1)) - 1;
            let raw = i128::from(neon_sign_extend(a, size)) + i128::from(neon_sign_extend(b, size));
            if raw < min || raw > max {
                self.cpsr.q = true;
            }
            (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(size)
        } else {
            let max = neon_elem_mask(size);
            let raw = u128::from(a) + u128::from(b);
            if raw > u128::from(max) {
                self.cpsr.q = true;
            }
            raw.min(u128::from(max)) as u64
        }
    }

    fn neon_saturating_sub(&mut self, a: u64, b: u64, size: usize, signed: bool) -> u64 {
        let bits = neon_elem_bits(size);
        if signed {
            let min = -(1i128 << (bits - 1));
            let max = (1i128 << (bits - 1)) - 1;
            let raw = i128::from(neon_sign_extend(a, size)) - i128::from(neon_sign_extend(b, size));
            if raw < min || raw > max {
                self.cpsr.q = true;
            }
            (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(size)
        } else {
            if a < b {
                self.cpsr.q = true;
                0
            } else {
                a - b
            }
        }
    }

    fn neon_saturating_doubling_mul_high(
        &mut self,
        a: u64,
        b: u64,
        size: usize,
        rounding: bool,
    ) -> u64 {
        let bits = neon_elem_bits(size);
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let av = i128::from(neon_sign_extend(a, size));
        let bv = i128::from(neon_sign_extend(b, size));
        let round = if rounding { 1i128 << (bits - 1) } else { 0 };
        let raw = ((av * bv * 2) + round) >> bits;
        if raw < min || raw > max {
            self.cpsr.q = true;
        }
        (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(size)
    }

    fn neon_saturating_doubling_mul_long(&mut self, a: u64, b: u64, size: usize) -> u64 {
        let wide_size = size + 1;
        let bits = neon_elem_bits(wide_size);
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let av = i128::from(neon_sign_extend(a, size));
        let bv = i128::from(neon_sign_extend(b, size));
        let raw = av * bv * 2;
        if raw < min || raw > max {
            self.cpsr.q = true;
        }
        (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(wide_size)
    }

    fn neon_saturating_add_sub_wide(&mut self, a: u64, b: u64, size: usize, subtract: bool) -> u64 {
        let bits = neon_elem_bits(size);
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let av = i128::from(neon_sign_extend(a, size));
        let bv = i128::from(neon_sign_extend(b, size));
        let raw = if subtract { av - bv } else { av + bv };
        if raw < min || raw > max {
            self.cpsr.q = true;
        }
        (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(size)
    }

    fn neon_variable_shift(
        &mut self,
        value: u64,
        shift: i64,
        size: usize,
        signed: bool,
        saturating: bool,
        rounding: bool,
    ) -> u64 {
        let bits = neon_elem_bits(size);
        let mask = neon_elem_mask(size);
        if shift == 0 {
            return value & mask;
        }

        if shift < 0 {
            let amount = shift.unsigned_abs().min(255) as u32;
            return neon_shift_right(value, amount, size, signed, rounding);
        }

        let amount = shift.min(255) as u32;
        if !saturating {
            if amount >= bits {
                0
            } else {
                (value << amount) & mask
            }
        } else if signed {
            let min = -(1i128 << (bits - 1));
            let max = (1i128 << (bits - 1)) - 1;
            let input = i128::from(neon_sign_extend(value, size));
            let raw = if amount >= bits {
                if input == 0 {
                    0
                } else if input > 0 {
                    max + 1
                } else {
                    min - 1
                }
            } else {
                input << amount
            };
            if raw < min || raw > max {
                self.cpsr.q = true;
            }
            (raw.clamp(min, max) as i64 as u64) & mask
        } else {
            let raw = if amount >= bits {
                if value & mask == 0 {
                    0
                } else {
                    u128::from(mask) + 1
                }
            } else {
                u128::from(value & mask) << amount
            };
            if raw > u128::from(mask) {
                self.cpsr.q = true;
            }
            raw.min(u128::from(mask)) as u64
        }
    }

    fn neon_qshlu_immediate(&mut self, value: u64, size: usize, shift: u32) -> u64 {
        let mask = neon_elem_mask(size);
        let input = i128::from(neon_sign_extend(value, size));
        let raw = if input < 0 {
            -1
        } else if shift >= neon_elem_bits(size) {
            if input == 0 { 0 } else { i128::from(mask) + 1 }
        } else {
            input << shift
        };
        if raw < 0 || raw > i128::from(mask) {
            self.cpsr.q = true;
        }
        raw.clamp(0, i128::from(mask)) as u64
    }

    fn neon_saturating_narrow_to_unsigned(
        &mut self,
        value: u64,
        size: usize,
        signed_input: bool,
    ) -> u64 {
        let mask = neon_elem_mask(size);
        let raw = if signed_input {
            i128::from(neon_sign_extend(value, size + 1))
        } else {
            i128::from(value)
        };
        if raw < 0 || raw > i128::from(mask) {
            self.cpsr.q = true;
        }
        raw.clamp(0, i128::from(mask)) as u64
    }

    fn neon_saturating_narrow_to_signed(&mut self, value: u64, size: usize) -> u64 {
        let bits = neon_elem_bits(size);
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let raw = i128::from(neon_sign_extend(value, size + 1));
        if raw < min || raw > max {
            self.cpsr.q = true;
        }
        (raw.clamp(min, max) as i64 as u64) & neon_elem_mask(size)
    }

    fn neon_saturating_narrow_unsigned_to_signed(&mut self, value: u64, size: usize) -> u64 {
        let bits = neon_elem_bits(size);
        let max = (1u128 << (bits - 1)) - 1;
        let raw = u128::from(value & neon_elem_mask(size + 1));
        if raw > max {
            self.cpsr.q = true;
        }
        raw.min(max) as u64
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

    fn start_thumb_it(&mut self, cond: u32, mask: u16, pc: u32, instr: u16) -> Result<()> {
        if cond == 0xf || mask == 0 {
            return Err(Trap::UndefinedThumb { pc, instr });
        }
        let len = 4 - mask.trailing_zeros() as usize;
        let mut conds = [0; 4];
        conds[0] = cond;
        for (idx, slot) in conds.iter_mut().enumerate().take(len).skip(1) {
            let mask_bit = (mask >> (4 - idx)) & 1 != 0;
            let same_cond = mask_bit == (cond & 1 != 0);
            *slot = if same_cond { cond } else { cond ^ 1 };
        }
        self.thumb_it = Some(ThumbItState { conds, len, pos: 0 });
        Ok(())
    }

    fn consume_thumb_it_condition(&mut self) -> Option<u32> {
        let mut state = self.thumb_it?;
        let cond = state.conds[state.pos];
        state.pos += 1;
        if state.pos >= state.len {
            self.thumb_it = None;
        } else {
            self.thumb_it = Some(state);
        }
        Some(cond)
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
        if idx >= 32 {
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
            pc.wrapping_add(4)
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

fn vfp_expand_imm_f64(imm8: u8) -> u64 {
    let imm8 = u64::from(imm8);
    let sign = if imm8 & 0x80 != 0 { 0x8000 } else { 0 };
    let exponent = if imm8 & 0x40 != 0 { 0x3fc0 } else { 0x4000 };
    (sign | exponent | (imm8 & 0x3f)) << 48
}

fn vfp_expand_imm_f32(imm8: u8) -> u32 {
    let imm8 = u32::from(imm8);
    let sign = if imm8 & 0x80 != 0 { 0x8000 } else { 0 };
    let exponent = if imm8 & 0x40 != 0 { 0x3e00 } else { 0x4000 };
    (sign | exponent | ((imm8 & 0x3f) << 3)) << 16
}

fn neon_reg_bytes(q: bool) -> usize {
    if q { 16 } else { 8 }
}

fn neon_elem_bits(size: usize) -> u32 {
    8 << size
}

fn neon_elem_mask(size: usize) -> u64 {
    let bits = neon_elem_bits(size);
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

fn neon_byte_mask(bytes: usize) -> u64 {
    if bytes == 8 {
        u64::MAX
    } else {
        (1u64 << (bytes * 8)) - 1
    }
}

fn neon_decode_ls_multiple(itype: u8, size: usize, align: u8) -> Option<(usize, usize, usize)> {
    match itype {
        0b0111 if align & 0b10 == 0 => Some((1, 1, 0)),
        0b1010 if align != 0b11 => Some((1, 2, 0)),
        0b0110 if align & 0b10 == 0 => Some((1, 3, 0)),
        0b0010 => Some((1, 4, 0)),
        0b1000 | 0b1001 if size != 0b11 && align != 0b11 => {
            Some((2, 1, if itype == 0b1000 { 1 } else { 2 }))
        }
        0b0011 if size != 0b11 => Some((2, 2, 2)),
        0b0100 | 0b0101 if size != 0b11 && align & 0b10 == 0 => {
            Some((3, 1, if itype == 0b0100 { 1 } else { 2 }))
        }
        0b0000 | 0b0001 if size != 0b11 => Some((4, 1, if itype == 0b0000 { 1 } else { 2 })),
        _ => None,
    }
}

fn neon_decode_shift_imm(right_shift: bool, l: bool, imm6: u32) -> Option<(usize, u32)> {
    if l {
        return Some((3, if right_shift { 64 - imm6 } else { imm6 }));
    }

    let top = imm6 >> 3;
    if top == 0 {
        return None;
    }
    let size = 31 - top.leading_zeros();
    let bits = 8 << size;
    let shift = if right_shift {
        (bits * 2) - imm6
    } else {
        imm6 - bits
    };
    Some((size as usize, shift))
}

fn load_neon_memory_elem<M: Memory>(mem: &mut M, addr: u32, bytes: usize) -> Result<u64> {
    let mut value = 0u64;
    for idx in 0..bytes {
        value |= u64::from(mem.load8(addr.wrapping_add(idx as u32))?) << (idx * 8);
    }
    Ok(value)
}

fn store_neon_memory_elem<M: Memory>(
    mem: &mut M,
    addr: u32,
    bytes: usize,
    value: u64,
) -> Result<()> {
    for idx in 0..bytes {
        mem.store8(
            addr.wrapping_add(idx as u32),
            ((value >> (idx * 8)) & 0xff) as u8,
        )?;
    }
    Ok(())
}

fn neon_lanes(q: bool, size: usize) -> usize {
    neon_reg_bytes(q) >> size
}

fn neon_scalar_location(size: usize, vm: usize) -> (usize, usize) {
    match size {
        1 => (vm & 0x7, ((vm >> 4) << 1) | ((vm >> 3) & 1)),
        2 => (vm & 0xf, vm >> 4),
        _ => (vm, 0),
    }
}

fn neon_read_elem(bytes: &[u8; 16], lane: usize, size: usize) -> u64 {
    let off = lane << size;
    match size {
        0 => u64::from(bytes[off]),
        1 => u64::from(u16::from_le_bytes([bytes[off], bytes[off + 1]])),
        2 => u64::from(u32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ])),
        3 => u64::from_le_bytes(bytes[off..off + 8].try_into().expect("u64 lane")),
        _ => unreachable!(),
    }
}

fn neon_write_elem(bytes: &mut [u8; 16], lane: usize, size: usize, value: u64) {
    let off = lane << size;
    match size {
        0 => bytes[off] = value as u8,
        1 => bytes[off..off + 2].copy_from_slice(&(value as u16).to_le_bytes()),
        2 => bytes[off..off + 4].copy_from_slice(&(value as u32).to_le_bytes()),
        3 => bytes[off..off + 8].copy_from_slice(&value.to_le_bytes()),
        _ => unreachable!(),
    }
}

fn neon_sign_extend(value: u64, size: usize) -> i64 {
    let bits = neon_elem_bits(size);
    if bits == 64 {
        value as i64
    } else {
        let shift = 64 - bits;
        ((value << shift) as i64) >> shift
    }
}

fn neon_extend_elem(value: u64, size: usize, signed: bool) -> u64 {
    let wide_size = size + 1;
    if signed {
        (neon_sign_extend(value, size) as u64) & neon_elem_mask(wide_size)
    } else {
        value & neon_elem_mask(size)
    }
}

fn neon_extend_i128(value: u64, size: usize, signed: bool) -> i128 {
    if signed {
        i128::from(neon_sign_extend(value, size))
    } else {
        i128::from(value & neon_elem_mask(size))
    }
}

fn neon_mask_i128(value: i128, size: usize) -> u64 {
    (value & i128::from(neon_elem_mask(size))) as u64
}

fn neon_shift_right(value: u64, amount: u32, size: usize, signed: bool, rounding: bool) -> u64 {
    let bits = neon_elem_bits(size);
    let mask = neon_elem_mask(size);
    if amount == 0 {
        return value & mask;
    }
    if signed {
        let input = i128::from(neon_sign_extend(value, size));
        if amount >= bits + 1 {
            return if input < 0 { mask } else { 0 };
        }
        let rounded = if rounding && amount <= 126 {
            input + (1i128 << (amount - 1))
        } else {
            input
        };
        ((rounded >> amount.min(126)) as i64 as u64) & mask
    } else if amount > bits {
        0
    } else if amount == bits {
        if rounding {
            (value >> (bits - 1)) & 1
        } else {
            0
        }
    } else {
        let rounded = if rounding {
            value.wrapping_add(1u64 << (amount - 1))
        } else {
            value
        };
        (rounded >> amount) & mask
    }
}

fn neon_expand_modified_imm(imm: u8, cmode: u8, op: bool) -> u64 {
    let imm = u64::from(imm);
    let imm32 = match cmode {
        0 | 1 => imm,
        2 | 3 => imm << 8,
        4 | 5 => imm << 16,
        6 | 7 => imm << 24,
        8 | 9 => imm | (imm << 16),
        10 | 11 => (imm << 8) | (imm << 24),
        12 => (imm << 8) | 0xff,
        13 => (imm << 16) | 0xffff,
        14 if op => {
            let mut value = 0u64;
            for bit in 0..8 {
                if imm & (1 << bit) != 0 {
                    value |= 0xffu64 << (bit * 8);
                }
            }
            return value;
        }
        14 => imm | (imm << 8) | (imm << 16) | (imm << 24),
        15 => {
            ((imm & 0x80) << 24)
                | ((imm & 0x3f) << 19)
                | if imm & 0x40 != 0 { 0x1f << 25 } else { 1 << 30 }
        }
        _ => unreachable!(),
    };
    imm32 | (imm32 << 32)
}

fn neon_halving_add(a: u64, b: u64, size: usize, signed: bool, rounding: bool) -> u64 {
    if signed {
        let addend = if rounding { 1 } else { 0 };
        (((neon_sign_extend(a, size) as i128 + neon_sign_extend(b, size) as i128 + addend) >> 1)
            as i64 as u64)
            & neon_elem_mask(size)
    } else {
        let addend = if rounding { 1 } else { 0 };
        ((u128::from(a) + u128::from(b) + addend) >> 1) as u64
    }
}

fn neon_halving_sub(a: u64, b: u64, size: usize, signed: bool) -> u64 {
    if signed {
        (((neon_sign_extend(a, size) as i128 - neon_sign_extend(b, size) as i128) >> 1) as i64
            as u64)
            & neon_elem_mask(size)
    } else {
        (((i128::from(a) - i128::from(b)) >> 1) as i64 as u64) & neon_elem_mask(size)
    }
}

fn neon_compare_gt(a: u64, b: u64, size: usize, signed: bool) -> u64 {
    if if signed {
        neon_sign_extend(a, size) > neon_sign_extend(b, size)
    } else {
        a > b
    } {
        neon_elem_mask(size)
    } else {
        0
    }
}

fn neon_compare_ge(a: u64, b: u64, size: usize, signed: bool) -> u64 {
    if if signed {
        neon_sign_extend(a, size) >= neon_sign_extend(b, size)
    } else {
        a >= b
    } {
        neon_elem_mask(size)
    } else {
        0
    }
}

fn neon_minmax(a: u64, b: u64, size: usize, signed: bool, max: bool) -> u64 {
    if signed {
        let av = neon_sign_extend(a, size);
        let bv = neon_sign_extend(b, size);
        let selected = if (av >= bv) == max { av } else { bv };
        (selected as u64) & neon_elem_mask(size)
    } else if (a >= b) == max {
        a
    } else {
        b
    }
}

fn neon_abs_diff(a: u64, b: u64, size: usize, signed: bool) -> u64 {
    if signed {
        neon_sign_extend(a, size).abs_diff(neon_sign_extend(b, size)) & neon_elem_mask(size)
    } else {
        a.abs_diff(b) & neon_elem_mask(size)
    }
}

fn neon_pairwise_add_lane(lhs: &[u8; 16], rhs: &[u8; 16], lane: usize, size: usize) -> u64 {
    let half_lanes = 4 >> size;
    let (source, source_lane) = if lane < half_lanes {
        (lhs, lane * 2)
    } else {
        (rhs, (lane - half_lanes) * 2)
    };
    neon_read_elem(source, source_lane, size).wrapping_add(neon_read_elem(
        source,
        source_lane + 1,
        size,
    )) & neon_elem_mask(size)
}

fn neon_polynomial_mul8(a: u8, b: u8) -> u16 {
    let mut result = 0u16;
    for bit in 0..8 {
        if b & (1 << bit) != 0 {
            result ^= u16::from(a) << bit;
        }
    }
    result
}

fn neon_f32_compare(value: bool) -> u32 {
    if value { u32::MAX } else { 0 }
}

fn neon_recip_estimate(input: u32) -> u32 {
    debug_assert!((256..512).contains(&input));
    let a = input * 2 + 1;
    let b = (1 << 19) / a;
    (b + 1) >> 1
}

fn neon_recip_sqrt_estimate(mut input: u32) -> u32 {
    debug_assert!((128..512).contains(&input));
    if input < 256 {
        input = input * 2 + 1;
    } else {
        input = ((input >> 1) << 1).wrapping_add(1) * 2;
    }

    let mut b = 512u32;
    while u64::from(input) * u64::from(b + 1) * u64::from(b + 1) < (1u64 << 28) {
        b += 1;
    }
    (b + 1) / 2
}

fn neon_recpe_u32(value: u32) -> u32 {
    if value & 0x8000_0000 == 0 {
        return u32::MAX;
    }

    neon_recip_estimate((value >> 23) & 0x1ff) << 23
}

fn neon_rsqrte_u32(value: u32) -> u32 {
    if value & 0xc000_0000 == 0 {
        return u32::MAX;
    }

    neon_recip_sqrt_estimate((value >> 23) & 0x1ff) << 23
}

fn thumb_is_32bit_prefix(instr: u16) -> bool {
    matches!(instr & 0xf800, 0xe800 | 0xf000 | 0xf800)
}

fn thumb_expand_imm(imm12: u32, old_carry: bool) -> (u32, bool) {
    if imm12 & 0x0c00 == 0 {
        let imm8 = imm12 & 0xff;
        let value = match (imm12 >> 8) & 0x3 {
            0 => imm8,
            1 => (imm8 << 16) | imm8,
            2 => (imm8 << 24) | (imm8 << 8),
            3 => (imm8 << 24) | (imm8 << 16) | (imm8 << 8) | imm8,
            _ => unreachable!(),
        };
        (value, old_carry)
    } else {
        let unrotated = 0x80 | (imm12 & 0x7f);
        let rotation = (imm12 >> 7) & 0x1f;
        let value = unrotated.rotate_right(rotation);
        (value, value & 0x8000_0000 != 0)
    }
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

    fn enc_neon_3same(
        u: bool,
        size: u32,
        vn: usize,
        vd: usize,
        opcode: u32,
        q: bool,
        op: bool,
        vm: usize,
    ) -> u32 {
        0xf200_0000
            | enc_bit(u, 24)
            | (((vd >> 4) as u32 & 1) << 22)
            | ((size & 0x3) << 20)
            | (((vn & 0xf) as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((opcode & 0xf) << 8)
            | (((vn >> 4) as u32 & 1) << 7)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | enc_bit(op, 4)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_3diff(u: bool, size: u32, vn: usize, vd: usize, opcode: u32, vm: usize) -> u32 {
        0xf280_0000
            | enc_bit(u, 24)
            | (((vd >> 4) as u32 & 1) << 22)
            | ((size & 0x3) << 20)
            | (((vn & 0xf) as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((opcode & 0xf) << 8)
            | (((vn >> 4) as u32 & 1) << 7)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_ls_multiple(
        load: bool,
        rn: usize,
        vd: usize,
        itype: u32,
        size: u32,
        align: u32,
        rm: usize,
    ) -> u32 {
        0xf400_0000
            | (((vd >> 4) as u32 & 1) << 22)
            | enc_bit(load, 21)
            | ((rn as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((itype & 0xf) << 8)
            | ((size & 0x3) << 6)
            | ((align & 0x3) << 4)
            | ((rm & 0xf) as u32)
    }

    fn enc_neon_ls_all_lanes(
        rn: usize,
        vd: usize,
        nregs: u32,
        size: u32,
        stride: bool,
        align: bool,
        rm: usize,
    ) -> u32 {
        0xf4a0_0c00
            | (((vd >> 4) as u32 & 1) << 22)
            | ((rn as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | (((nregs - 1) & 0x3) << 8)
            | ((size & 0x3) << 6)
            | enc_bit(stride, 5)
            | enc_bit(align, 4)
            | ((rm & 0xf) as u32)
    }

    fn enc_neon_ls_single(
        load: bool,
        rn: usize,
        vd: usize,
        nregs: u32,
        size: u32,
        reg_idx: u32,
        stride: bool,
        align: u32,
        rm: usize,
    ) -> u32 {
        let (form, index_bits, stride_bit, align_bits) = match size {
            0 => (0, (reg_idx & 0x7) << 5, 0, (align & 1) << 4),
            1 => (
                1,
                (reg_idx & 0x3) << 6,
                enc_bit(stride, 5),
                (align & 1) << 4,
            ),
            2 => (
                2,
                (reg_idx & 0x1) << 7,
                enc_bit(stride, 6),
                (align & 0x3) << 4,
            ),
            _ => unreachable!(),
        };
        0xf480_0000
            | enc_bit(load, 21)
            | (((vd >> 4) as u32 & 1) << 22)
            | ((rn as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((form & 0x3) << 10)
            | (((nregs - 1) & 0x3) << 8)
            | index_bits
            | stride_bit
            | align_bits
            | ((rm & 0xf) as u32)
    }

    fn thumb_neon_ls_from_a32(instr: u32) -> (u16, u16) {
        let thumb = (instr & 0x00ff_ffff) | 0xf900_0000;
        ((thumb >> 16) as u16, thumb as u16)
    }

    fn thumb_neon_dp_from_a32(instr: u32) -> (u16, u16) {
        let thumb = (instr & 0xe0ff_ffff) | ((instr & (1 << 24)) << 4) | 0x0f00_0000;
        ((thumb >> 16) as u16, thumb as u16)
    }

    fn enc_neon_table(vn: usize, vd: usize, len: u32, tbx: bool, vm: usize) -> u32 {
        0xf3b0_0800
            | (((vd >> 4) as u32 & 1) << 22)
            | (((vn & 0xf) as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((len & 0x3) << 8)
            | (((vn >> 4) as u32 & 1) << 7)
            | enc_bit(tbx, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_vext(vn: usize, vd: usize, imm: u32, q: bool, vm: usize) -> u32 {
        0xf2b0_0000
            | (((vd >> 4) as u32 & 1) << 22)
            | (((vn & 0xf) as u32) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((imm & 0xf) << 8)
            | (((vn >> 4) as u32 & 1) << 7)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_vdup(vd: usize, imm4: u32, q: bool, vm: usize) -> u32 {
        0xf3b0_0c00
            | (((vd >> 4) as u32 & 1) << 22)
            | ((imm4 & 0xf) << 16)
            | (((vd & 0xf) as u32) << 12)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_vrev(vd: usize, size: u32, op: u32, q: bool, vm: usize) -> u32 {
        0xf3b0_0000
            | (((vd >> 4) as u32 & 1) << 22)
            | ((size & 0x3) << 18)
            | (((vd & 0xf) as u32) << 12)
            | ((op & 0xf) << 7)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_2reg_misc(vd: usize, size: u32, op1: u32, op2: u32, q: bool, vm: usize) -> u32 {
        0xf3b0_0000
            | (((vd >> 4) as u32 & 1) << 22)
            | ((size & 0x3) << 18)
            | ((op1 & 0x3) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((op2 & 0xf) << 7)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
    }

    fn enc_neon_modified_imm(vd: usize, imm: u32, cmode: u32, q: bool, op: bool) -> u32 {
        0xf280_0010
            | (((imm >> 7) & 1) << 24)
            | (((vd >> 4) as u32 & 1) << 22)
            | (((imm >> 4) & 0x7) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((cmode & 0xf) << 8)
            | enc_bit(q, 6)
            | enc_bit(op, 5)
            | (imm & 0xf)
    }

    fn enc_neon_2reg_shift(
        u: bool,
        imm6: u32,
        vd: usize,
        opcode: u32,
        l: bool,
        q: bool,
        vm: usize,
    ) -> u32 {
        0xf280_0010
            | enc_bit(u, 24)
            | (((vd >> 4) as u32 & 1) << 22)
            | ((imm6 & 0x3f) << 16)
            | (((vd & 0xf) as u32) << 12)
            | ((opcode & 0xf) << 8)
            | enc_bit(l, 7)
            | enc_bit(q, 6)
            | (((vm >> 4) as u32 & 1) << 5)
            | ((vm & 0xf) as u32)
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
            0xe6ff_0f30, // rbit r0, r0
            0xe710_f010, // sdiv r0, r0, r0
            0xe730_f010, // udiv r0, r0, r0
        ] {
            let err = cpu.execute_arm(instr, 0x80, &mut mem).unwrap_err();
            assert_eq!(err, Trap::UndefinedArm { pc: 0x80, instr });
        }
    }

    #[test]
    fn armv7_movw_movt_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.execute_arm(0xe302_60ab, 0, &mut mem).unwrap(); // movw r6, #0x20ab
        assert_eq!(cpu.reg(6), 0x20ab);

        cpu.execute_arm(0xe34c_6def, 0, &mut mem).unwrap(); // movt r6, #0xcdef
        assert_eq!(cpu.reg(6), 0xcdef_20ab);

        let err = cpu.execute_arm(0xe300_f000, 0x100, &mut mem).unwrap_err(); // movw pc, #0
        assert_eq!(err, Trap::Unpredictable("MOVW/MOVT with PC destination"));
    }

    #[test]
    fn neon_three_same_integer_and_logic_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(1, 0x0807_0605_0403_0201);
        cpu.set_dreg(2, 0x0101_0101_0101_0101);
        cpu.execute_arm(
            enc_neon_3same(false, 0, 1, 0, 8, false, false, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vadd.i8 d0, d1, d2
        assert_eq!(cpu.dreg(0), 0x0908_0706_0504_0302);

        cpu.set_dreg(3, 0x0008_0007_0006_0005);
        cpu.set_dreg(4, 0x0001_0002_0003_0004);
        cpu.execute_arm(
            enc_neon_3same(true, 1, 3, 5, 8, false, false, 4),
            0,
            &mut mem,
        )
        .unwrap(); // vsub.i16 d5, d3, d4
        assert_eq!(cpu.dreg(5), 0x0007_0005_0003_0001);

        cpu.set_dreg(3, 0xffff_8000_0001_0000);
        cpu.set_dreg(4, 0x0001_ffff_0002_0001);
        cpu.execute_arm(
            enc_neon_3same(true, 1, 3, 7, 2, false, false, 4),
            0,
            &mut mem,
        )
        .unwrap(); // vhsub.u16 d7, d3, d4
        assert_eq!(cpu.dreg(7), 0x7fff_c000_ffff_ffff);

        cpu.set_dreg(1, 0x0007_0003_0009_0001);
        cpu.set_dreg(2, 0x0004_0008_0002_0005);
        cpu.execute_arm(
            enc_neon_3same(true, 1, 1, 0, 10, false, false, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vpmax.u16 d0, d1, d2
        assert_eq!(cpu.dreg(0), 0x0008_0005_0007_0009);

        cpu.set_dreg(2, 0x00ff_00ff_00ff_00ff);
        cpu.set_dreg(3, 0xff00_ff00_ff00_ff00);
        cpu.set_dreg(4, 0x0f0f_0f0f_0f0f_0f0f);
        cpu.set_dreg(5, 0xf0f0_f0f0_f0f0_f0f0);
        cpu.execute_arm(
            enc_neon_3same(false, 2, 2, 0, 1, true, true, 4),
            0,
            &mut mem,
        )
        .unwrap(); // vorr q0, q1, q2
        assert_eq!(cpu.dreg(0), 0x0fff_0fff_0fff_0fff);
        assert_eq!(cpu.dreg(1), 0xfff0_fff0_fff0_fff0);

        let err = cpu
            .execute_arm(
                enc_neon_3same(false, 0, 2, 1, 8, true, false, 4),
                0x200,
                &mut mem,
            )
            .unwrap_err(); // q-register form with odd Vd
        assert_eq!(
            err,
            Trap::UndefinedArm {
                pc: 0x200,
                instr: enc_neon_3same(false, 0, 2, 1, 8, true, false, 4)
            }
        );

        let (first, second) =
            thumb_neon_dp_from_a32(enc_neon_3same(false, 0, 1, 0, 8, false, false, 2));
        cpu.set_isa(Isa::Thumb);
        cpu.set_dreg(1, 0x0807_0605_0403_0201);
        cpu.set_dreg(2, 0x0101_0101_0101_0101);
        cpu.execute_thumb32(first, second, 0, &mut mem).unwrap(); // vadd.i8 d0, d1, d2
        assert_eq!(cpu.dreg(0), 0x0908_0706_0504_0302);
    }

    #[test]
    fn neon_three_same_saturating_compare_and_float_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(1, 0x7f78_8081_0102_f0f1);
        cpu.set_dreg(2, 0x010a_ff80_0304_2020);
        cpu.execute_arm(
            enc_neon_3same(false, 0, 1, 0, 0, false, true, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vqadd.s8 d0, d1, d2
        assert_eq!(cpu.dreg(0), 0x7f7f_8080_0406_1011);
        assert!(cpu.cpsr.q);

        cpu.execute_arm(
            enc_neon_3same(false, 0, 1, 3, 3, false, false, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vcgt.s8 d3, d1, d2
        assert_eq!(cpu.dreg(3), 0xffff_00ff_0000_0000);

        cpu.cpsr.q = false;
        cpu.set_dreg(8, u64::from_le_bytes([0, 0, 0, 0x40, 0, 0, 0, 0x80]));
        cpu.set_dreg(9, u64::from_le_bytes([0, 0, 0, 0x40, 0, 0, 0, 0x80]));
        cpu.execute_arm(
            enc_neon_3same(false, 2, 8, 10, 11, false, false, 9),
            0,
            &mut mem,
        )
        .unwrap(); // vqdmulh.s32 d10, d8, d9
        assert_eq!(cpu.dreg(10), 0x7fff_ffff_2000_0000);
        assert!(cpu.cpsr.q);

        let pack2 = |a: f32, b: f32| u64::from(a.to_bits()) | (u64::from(b.to_bits()) << 32);
        cpu.set_dreg(4, pack2(1.5, 2.0));
        cpu.set_dreg(5, pack2(2.5, -3.0));
        cpu.execute_arm(
            enc_neon_3same(false, 0, 4, 6, 13, false, false, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vadd.f32 d6, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(12)), 4.0);
        assert_eq!(f32::from_bits(cpu.sreg(13)), -1.0);

        cpu.set_dreg(6, pack2(10.0, 10.0));
        cpu.execute_arm(
            enc_neon_3same(false, 0, 4, 6, 13, false, true, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vmla.f32 d6, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(12)), 13.75);
        assert_eq!(f32::from_bits(cpu.sreg(13)), 4.0);

        cpu.set_dreg(4, pack2(1.0, 2.5));
        cpu.set_dreg(5, pack2(10.0, -3.0));
        cpu.execute_arm(
            enc_neon_3same(true, 0, 4, 7, 13, false, false, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vpadd.f32 d7, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(14)), 3.5);
        assert_eq!(f32::from_bits(cpu.sreg(15)), 7.0);

        cpu.execute_arm(
            enc_neon_3same(true, 2, 4, 8, 13, false, false, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vabd.f32 d8, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(16)), 9.0);
        assert_eq!(f32::from_bits(cpu.sreg(17)), 5.5);

        cpu.execute_arm(
            enc_neon_3same(true, 0, 4, 9, 15, false, false, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vpmax.f32 d9, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(18)), 2.5);
        assert_eq!(f32::from_bits(cpu.sreg(19)), 10.0);

        cpu.execute_arm(
            enc_neon_3same(true, 2, 4, 10, 15, false, false, 5),
            0,
            &mut mem,
        )
        .unwrap(); // vpmin.f32 d10, d4, d5
        assert_eq!(f32::from_bits(cpu.sreg(20)), 1.0);
        assert_eq!(f32::from_bits(cpu.sreg(21)), -3.0);
    }

    #[test]
    fn neon_structure_load_store_and_misc_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x1000, 0x200);

        let mut interleaved = Vec::new();
        for lane in 1..=4u16 {
            interleaved.extend_from_slice(&(0x1000 | lane).to_le_bytes());
            interleaved.extend_from_slice(&(0x2000 | lane).to_le_bytes());
            interleaved.extend_from_slice(&(0x3000 | lane).to_le_bytes());
        }
        mem.load_bytes(0x1000, &interleaved).unwrap();
        cpu.set_reg(0, 0x1000);
        cpu.execute_arm(
            enc_neon_ls_multiple(true, 0, 4, 0b0100, 1, 0, 13),
            0,
            &mut mem,
        )
        .unwrap(); // vld3.16 {d4, d5, d6}, [r0]!
        assert_eq!(cpu.reg(0), 0x1018);
        assert_eq!(cpu.dreg(4), 0x1004_1003_1002_1001);
        assert_eq!(cpu.dreg(5), 0x2004_2003_2002_2001);
        assert_eq!(cpu.dreg(6), 0x3004_3003_3002_3001);

        cpu.set_reg(1, 0x1040);
        cpu.execute_arm(
            enc_neon_ls_multiple(false, 1, 4, 0b0100, 1, 0, 15),
            0,
            &mut mem,
        )
        .unwrap(); // vst3.16 {d4, d5, d6}, [r1]
        for (idx, expected) in interleaved.iter().copied().enumerate() {
            assert_eq!(mem.load8(0x1040 + idx as u32).unwrap(), expected);
        }

        cpu.set_dreg(15, u64::from_le_bytes(*b"abcdefgh"));
        cpu.set_dreg(16, u64::from_le_bytes(*b"ABCDEFGH"));
        cpu.set_dreg(4, u64::from_le_bytes([0, 7, 8, 15, 16, 1, 10, 99]));
        assert_eq!(enc_neon_table(15, 3, 1, false, 4), 0xf3bf_3904);
        cpu.execute_arm(enc_neon_table(15, 3, 1, false, 4), 0, &mut mem)
            .unwrap(); // vtbl.8 d3, {d15, d16}, d4
        assert_eq!(cpu.dreg(3), u64::from_le_bytes(*b"ahAH\0bC\0"));

        cpu.set_dreg(3, u64::from_le_bytes(*b"zzzzzzzz"));
        cpu.set_dreg(4, u64::from_le_bytes([0, 17, 1, 18, 2, 19, 3, 20]));
        cpu.execute_arm(enc_neon_table(15, 3, 1, true, 4), 0, &mut mem)
            .unwrap(); // vtbx.8 d3, {d15, d16}, d4
        assert_eq!(cpu.dreg(3), u64::from_le_bytes(*b"azbzczdz"));

        cpu.set_dreg(0, u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]));
        cpu.set_dreg(1, u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]));
        cpu.set_dreg(4, u64::from_le_bytes([16, 17, 18, 19, 20, 21, 22, 23]));
        cpu.set_dreg(5, u64::from_le_bytes([24, 25, 26, 27, 28, 29, 30, 31]));
        cpu.execute_arm(enc_neon_vext(0, 8, 12, true, 4), 0, &mut mem)
            .unwrap(); // vext.8 q4, q0, q2, #12
        assert_eq!(
            cpu.dreg(8),
            u64::from_le_bytes([12, 13, 14, 15, 16, 17, 18, 19])
        );
        assert_eq!(
            cpu.dreg(9),
            u64::from_le_bytes([20, 21, 22, 23, 24, 25, 26, 27])
        );

        cpu.set_dreg(6, 0x4444_3333_2222_1111);
        cpu.execute_arm(enc_neon_vdup(10, 0xa, false, 6), 0, &mut mem)
            .unwrap(); // vdup.16 d10, d6[2]
        assert_eq!(cpu.dreg(10), 0x3333_3333_3333_3333);

        cpu.set_dreg(11, u64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8]));
        cpu.execute_arm(enc_neon_vrev(12, 0, 1, false, 11), 0, &mut mem)
            .unwrap(); // vrev32.8 d12, d11
        assert_eq!(cpu.dreg(12), u64::from_le_bytes([4, 3, 2, 1, 8, 7, 6, 5]));

        let pack2 = |a: f32, b: f32| u64::from(a.to_bits()) | (u64::from(b.to_bits()) << 32);
        cpu.set_dreg(20, pack2(-1.5, 2.25));
        cpu.execute_arm(enc_neon_2reg_misc(21, 2, 1, 14, false, 20), 0, &mut mem)
            .unwrap(); // vabs.f32 d21, d20
        assert_eq!(f32::from_bits(cpu.sreg(42)), 1.5);
        assert_eq!(f32::from_bits(cpu.sreg(43)), 2.25);

        cpu.set_dreg(23, u64::from_le_bytes([1, 0, 0, 0, 0xfe, 0xff, 0xff, 0xff]));
        cpu.execute_arm(enc_neon_2reg_misc(22, 2, 1, 7, false, 23), 0, &mut mem)
            .unwrap(); // vneg.s32 d22, d23
        assert_eq!(
            cpu.dreg(22),
            u64::from_le_bytes([0xff, 0xff, 0xff, 0xff, 2, 0, 0, 0])
        );

        cpu.execute_arm(
            enc_neon_modified_imm(14, 0x80, 14, true, false),
            0,
            &mut mem,
        )
        .unwrap(); // vmov.i8 q7, #0x80
        assert_eq!(cpu.dreg(14), 0x8080_8080_8080_8080);
        assert_eq!(cpu.dreg(15), 0x8080_8080_8080_8080);

        mem.store16(0x1080, 0x1234).unwrap();
        cpu.set_reg(2, 0x1080);
        cpu.execute_arm(
            enc_neon_ls_all_lanes(2, 24, 1, 1, false, false, 13),
            0,
            &mut mem,
        )
        .unwrap(); // vld1.16 {d24[]}, [r2]!
        assert_eq!(cpu.reg(2), 0x1082);
        assert_eq!(cpu.dreg(24), 0x1234_1234_1234_1234);

        cpu.set_dreg(25, 0x8877_6655_4433_2211);
        cpu.set_reg(3, 0x1090);
        cpu.set_reg(4, 5);
        cpu.execute_arm(
            enc_neon_ls_single(false, 3, 25, 1, 0, 3, false, 0, 4),
            0,
            &mut mem,
        )
        .unwrap(); // vst1.8 {d25[3]}, [r3], r4
        assert_eq!(mem.load8(0x1090).unwrap(), 0x44);
        assert_eq!(cpu.reg(3), 0x1095);

        mem.store32(0x10a0, 0xaabb_ccdd).unwrap();
        cpu.set_reg(5, 0x10a0);
        cpu.set_reg(6, 4);
        cpu.set_dreg(26, 0x1122_3344_5566_7788);
        cpu.execute_arm(
            enc_neon_ls_single(true, 5, 26, 1, 2, 1, false, 0, 6),
            0,
            &mut mem,
        )
        .unwrap(); // vld1.32 {d26[1]}, [r5], r6
        assert_eq!(cpu.dreg(26), 0xaabb_ccdd_5566_7788);
        assert_eq!(cpu.reg(5), 0x10a4);

        let (first, second) =
            thumb_neon_ls_from_a32(enc_neon_ls_multiple(false, 0, 16, 0b1010, 2, 0, 15));
        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(0, 0x10c0);
        cpu.set_dreg(16, 0x0403_0201_ddcc_bbaa);
        cpu.set_dreg(17, 0x0c0b_0a09_0807_0605);
        cpu.execute_thumb32(first, second, 0, &mut mem).unwrap(); // vst1.32 {d16, d17}, [r0]
        assert_eq!(mem.load32(0x10c0).unwrap(), 0xddcc_bbaa);
        assert_eq!(mem.load32(0x10c4).unwrap(), 0x0403_0201);
        assert_eq!(mem.load32(0x10c8).unwrap(), 0x0807_0605);
        assert_eq!(mem.load32(0x10cc).unwrap(), 0x0c0b_0a09);

        mem.store8(0x10e0, 0xfe).unwrap();
        let (first, second) =
            thumb_neon_ls_from_a32(enc_neon_ls_single(true, 0, 0, 1, 0, 0, false, 0, 2));
        cpu.set_reg(0, 0x10e0);
        cpu.set_reg(2, 3);
        cpu.set_dreg(0, 0x8877_6655_4433_2211);
        cpu.execute_thumb32(first, second, 0, &mut mem).unwrap(); // vld1.8 {d0[0]}, [r0], r2
        assert_eq!(cpu.dreg(0), 0x8877_6655_4433_22fe);
        assert_eq!(cpu.reg(0), 0x10e3);
    }

    #[test]
    fn neon_mcpe_kernel_patterns_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x1000, 0x400);

        mem.load_bytes(
            0x1100,
            &[
                0xaa, 0xbb, 0xcc, 0xdd, 0x10, 0x11, 0x12, 0x13, 0x20, 0x21, 0x22, 0x23, 0x30, 0x31,
                0x32, 0x33, 0x40, 0x41, 0x42, 0x43, 0x50, 0x51, 0x52, 0x53, 0x60, 0x61, 0x62, 0x63,
                0x70, 0x71, 0x72, 0x73, 0x80, 0x81, 0x82, 0x83,
            ],
        )
        .unwrap();

        cpu.set_reg(2, 0x1100);
        cpu.set_dreg(28, 0x8877_6655_4433_2211);
        cpu.execute_arm(0xf4e2_c83d, 0, &mut mem).unwrap(); // vld1.32 {d28[0]}, [r2:32]!
        assert_eq!(cpu.dreg(28), 0x8877_6655_ddcc_bbaa);
        assert_eq!(cpu.reg(2), 0x1104);

        cpu.set_reg(1, 0x1104);
        cpu.execute_arm(0xf421_028d, 0, &mut mem).unwrap(); // vld1.32 {d0, d1, d2, d3}, [r1]!
        assert_eq!(cpu.dreg(0), 0x2322_2120_1312_1110);
        assert_eq!(cpu.dreg(1), 0x4342_4140_3332_3130);
        assert_eq!(cpu.dreg(2), 0x6362_6160_5352_5150);
        assert_eq!(cpu.dreg(3), 0x8382_8180_7372_7170);
        assert_eq!(cpu.reg(1), 0x1124);

        cpu.set_dreg(28, 0x0004_0003_0002_0001);
        cpu.set_dreg(8, 0x0014_0013_0012_0011);
        cpu.execute_arm(0xf3f6_c188, 0, &mut mem).unwrap(); // vzip.16 d28, d8
        assert_eq!(cpu.dreg(28), 0x0012_0002_0011_0001);
        assert_eq!(cpu.dreg(8), 0x0014_0004_0013_0003);

        cpu.set_dreg(28, 0x0000_0004_0000_0003);
        cpu.set_dreg(0, 0x0000_0007_0000_0005);
        cpu.execute_arm(0xf3ac_cac0, 0, &mut mem).unwrap(); // vmull.u32 q6, d28, d0[0]
        assert_eq!(cpu.dreg(12), 15);
        assert_eq!(cpu.dreg(13), 20);

        cpu.set_dreg(29, 0x0000_0003_0000_0002);
        cpu.set_dreg(4, 0x0000_000d_0000_000b);
        cpu.execute_arm(0xf3ad_c2c4, 0, &mut mem).unwrap(); // vmlal.u32 q6, d29, d4[0]
        assert_eq!(cpu.dreg(12), 37);
        assert_eq!(cpu.dreg(13), 53);

        cpu.set_dreg(13, 0x0000_0000_0001_0002);
        cpu.execute_arm(0xf290_a59d, 0, &mut mem).unwrap(); // vshl.i64 d10, d13, #16
        assert_eq!(cpu.dreg(10), 0x0000_0001_0002_0000);
        cpu.set_dreg(12, 5);
        cpu.execute_arm(0xf23a_a80c, 0, &mut mem).unwrap(); // vadd.i64 d10, d10, d12
        assert_eq!(cpu.dreg(10), 0x0000_0001_0002_0005);
        cpu.execute_arm(0xf3b0_a09a, 0, &mut mem).unwrap(); // vshr.u64 d10, d10, #16
        assert_eq!(cpu.dreg(10), 0x0000_0000_0001_0002);

        cpu.set_dreg(16, 0x0706_0504_0302_0100);
        cpu.set_dreg(17, 0x1716_1514_1312_1110);
        cpu.execute_arm(0xf3f0_0060, 0, &mut mem).unwrap(); // vrev64.8 q8, q8
        assert_eq!(cpu.dreg(16), 0x0001_0203_0405_0607);
        assert_eq!(cpu.dreg(17), 0x1011_1213_1415_1617);

        cpu.set_dreg(6, 0x1111_1111_1111_1111);
        cpu.set_dreg(7, 0x2222_2222_2222_2222);
        cpu.execute_arm(0xf2f6_4846, 0, &mut mem).unwrap(); // vext.64 q10, q3, q3, #1
        assert_eq!(cpu.dreg(20), 0x2222_2222_2222_2222);
        assert_eq!(cpu.dreg(21), 0x1111_1111_1111_1111);

        cpu.set_reg(7, 0x1200);
        cpu.set_dreg(12, 0x0706_0504_0302_0100);
        cpu.set_dreg(13, 0x1716_1514_1312_1110);
        cpu.set_dreg(14, 0x2726_2524_2322_2120);
        cpu.set_dreg(15, 0x3736_3534_3332_3130);
        cpu.execute_arm(0xf407_c2fd, 0, &mut mem).unwrap(); // vst1.64 {d12, d13, d14, d15}, [r7:256]!
        assert_eq!(mem.load32(0x1200).unwrap(), 0x0302_0100);
        assert_eq!(mem.load32(0x1204).unwrap(), 0x0706_0504);
        assert_eq!(mem.load32(0x121c).unwrap(), 0x3736_3534);
        assert_eq!(cpu.reg(7), 0x1220);
    }

    #[test]
    fn neon_core_lane_moves_and_scalar_multiply_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(0, 0x1122_3344);
        cpu.execute_arm(0xee88_0b10, 0, &mut mem).unwrap(); // vdup.32 d8, r0
        assert_eq!(cpu.dreg(8), 0x1122_3344_1122_3344);

        cpu.execute_arm(0xee88_0b30, 0, &mut mem).unwrap(); // vdup.16 d8, r0
        assert_eq!(cpu.dreg(8), 0x3344_3344_3344_3344);

        cpu.set_dreg(8, 0);
        cpu.set_reg(0, 0xffff_ffaa);
        cpu.execute_arm(0xee48_0b30, 0, &mut mem).unwrap(); // vmov.8 d8[1], r0
        assert_eq!(cpu.dreg(8), 0x0000_0000_0000_aa00);

        cpu.execute_arm(0xeed8_1b30, 0, &mut mem).unwrap(); // vmov.u8 r1, d8[1]
        assert_eq!(cpu.reg(1), 0xaa);
        cpu.execute_arm(0xee58_2b30, 0, &mut mem).unwrap(); // vmov.s8 r2, d8[1]
        assert_eq!(cpu.reg(2), 0xffff_ffaa);

        cpu.set_dreg(8, 0);
        cpu.set_reg(0, 0xffff_8765);
        cpu.execute_arm(0xee28_0b30, 0, &mut mem).unwrap(); // vmov.16 d8[2], r0
        assert_eq!(cpu.dreg(8), 0x0000_8765_0000_0000);

        cpu.execute_arm(0xeeb8_3b30, 0, &mut mem).unwrap(); // vmov.u16 r3, d8[2]
        assert_eq!(cpu.reg(3), 0x8765);
        cpu.execute_arm(0xee38_4b30, 0, &mut mem).unwrap(); // vmov.s16 r4, d8[2]
        assert_eq!(cpu.reg(4), 0xffff_8765);

        cpu.set_dreg(0, 0x0000_0007_0000_0005);
        cpu.set_dreg(12, 0x0000_0004_0000_0003);
        cpu.execute_arm(0xf2ec_6840, 0, &mut mem).unwrap(); // vmul.i32 d22, d12, d0[0]
        assert_eq!(cpu.dreg(22), 0x0000_0014_0000_000f);

        cpu.set_dreg(2, 0x0004_0003_0002_0001);
        cpu.set_dreg(4, 0x000a_0009_0008_0007);
        cpu.set_dreg(19, 0x0190_012c_00c8_0064);
        cpu.execute_arm(0xf2d2_306c, 0, &mut mem).unwrap(); // vmla.i16 d19, d2, d4[3]
        assert_eq!(cpu.dreg(19), 0x01b8_014a_00dc_006e);
    }

    #[test]
    fn neon_register_shift_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(
            1,
            u64::from_le_bytes([1, 0x80, 0x7f, 2, 0xff, 8, 0x11, 0x40]),
        );
        cpu.set_dreg(13, u64::from_le_bytes([1, 1, 1, 0xff, 0xff, 8, 0xf8, 0]));
        cpu.execute_arm(0xf20d_4401, 0, &mut mem).unwrap(); // vshl.s8 d4, d1, d13
        assert_eq!(
            cpu.dreg(4),
            u64::from_le_bytes([2, 0, 0xfe, 1, 0xff, 0, 0, 0x40])
        );

        cpu.set_dreg(2, u64::from_le_bytes([0x40, 0x7f, 0x80, 0, 1, 2, 3, 4]));
        cpu.set_dreg(3, u64::from_le_bytes([1, 1, 1, 4, 8, 0xff, 0xfe, 0]));
        cpu.execute_arm(
            enc_neon_3same(false, 0, 3, 5, 4, false, true, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vqshl.s8 d5, d2, d3
        assert_eq!(
            cpu.dreg(5),
            u64::from_le_bytes([0x7f, 0x7f, 0x80, 0, 0x7f, 1, 0, 4])
        );
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_dreg(2, u64::from_le_bytes([1, 0, 0, 0x80, 0, 0x40, 0xff, 0]));
        cpu.set_dreg(3, u64::from_le_bytes([1, 1, 0xff, 1, 1, 0xff, 0, 0]));
        cpu.execute_arm(
            enc_neon_3same(false, 1, 3, 6, 4, false, false, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vshl.s16 d6, d2, d3
        assert_eq!(
            cpu.dreg(6),
            u64::from_le_bytes([2, 0, 0, 0xc0, 0, 0x80, 0xff, 0])
        );

        cpu.set_dreg(6, u64::from_le_bytes([0, 0x40, 1, 0, 0xff, 0xff, 0, 0x80]));
        cpu.set_dreg(7, u64::from_le_bytes([2, 0, 1, 1, 0xff, 0, 1, 0x7f]));
        cpu.execute_arm(
            enc_neon_3same(true, 1, 7, 8, 4, false, true, 6),
            0,
            &mut mem,
        )
        .unwrap(); // vqshl.u16 d8, d6, d7
        assert_eq!(
            cpu.dreg(8),
            u64::from_le_bytes([0xff, 0xff, 2, 0, 0xff, 0x7f, 0xff, 0xff])
        );
        assert!(cpu.cpsr.q);
    }

    #[test]
    fn neon_two_register_permute_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(0, u64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8]));
        cpu.set_dreg(1, u64::from_le_bytes([11, 12, 13, 14, 15, 16, 17, 18]));
        cpu.execute_arm(enc_neon_2reg_misc(0, 0, 2, 1, false, 1), 0, &mut mem)
            .unwrap(); // vtrn.8 d0, d1
        assert_eq!(
            cpu.dreg(0),
            u64::from_le_bytes([1, 11, 3, 13, 5, 15, 7, 17])
        );
        assert_eq!(
            cpu.dreg(1),
            u64::from_le_bytes([2, 12, 4, 14, 6, 16, 8, 18])
        );

        cpu.set_dreg(2, u64::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8]));
        cpu.set_dreg(3, u64::from_le_bytes([11, 12, 13, 14, 15, 16, 17, 18]));
        cpu.execute_arm(enc_neon_2reg_misc(2, 0, 2, 2, false, 3), 0, &mut mem)
            .unwrap(); // vuzp.8 d2, d3
        assert_eq!(
            cpu.dreg(2),
            u64::from_le_bytes([1, 3, 5, 7, 11, 13, 15, 17])
        );
        assert_eq!(
            cpu.dreg(3),
            u64::from_le_bytes([2, 4, 6, 8, 12, 14, 16, 18])
        );

        cpu.set_dreg(4, u64::from_le_bytes([1, 0, 2, 0, 3, 0, 4, 0]));
        cpu.set_dreg(5, u64::from_le_bytes([11, 0, 12, 0, 13, 0, 14, 0]));
        cpu.execute_arm(enc_neon_2reg_misc(4, 1, 2, 3, false, 5), 0, &mut mem)
            .unwrap(); // vzip.16 d4, d5
        assert_eq!(cpu.dreg(4), u64::from_le_bytes([1, 0, 11, 0, 2, 0, 12, 0]));
        assert_eq!(cpu.dreg(5), u64::from_le_bytes([3, 0, 13, 0, 4, 0, 14, 0]));
    }

    #[test]
    fn neon_two_register_misc_pairwise_count_and_saturating_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        let pack_u16x4 = |values: [u16; 4]| {
            let mut bytes = [0u8; 8];
            for (idx, value) in values.into_iter().enumerate() {
                bytes[idx * 2..idx * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
            u64::from_le_bytes(bytes)
        };
        let pack_i16x4 = |values: [i16; 4]| {
            let mut bytes = [0u8; 8];
            for (idx, value) in values.into_iter().enumerate() {
                bytes[idx * 2..idx * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
            u64::from_le_bytes(bytes)
        };

        cpu.set_dreg(28, u64::from_le_bytes([1, 2, 250, 5, 0xff, 1, 0, 16]));
        cpu.set_dreg(29, u64::from_le_bytes([10, 20, 30, 40, 50, 60, 70, 80]));
        cpu.execute_arm(enc_neon_2reg_misc(18, 0, 0, 5, true, 28), 0, &mut mem)
            .unwrap(); // vpaddl.u8 q9, q14
        assert_eq!(cpu.dreg(18), pack_u16x4([3, 255, 256, 16]));
        assert_eq!(cpu.dreg(19), pack_u16x4([30, 70, 110, 150]));

        cpu.set_dreg(14, u64::from(10u32) | (u64::from(20u32) << 32));
        cpu.set_dreg(16, pack_i16x4([-1, -2, 1000, -2000]));
        cpu.execute_arm(enc_neon_2reg_misc(14, 1, 0, 12, false, 16), 0, &mut mem)
            .unwrap(); // vpadal.s16 d14, d16
        assert_eq!(
            cpu.dreg(14),
            u64::from(7u32) | (u64::from((-980i32) as u32) << 32)
        );

        cpu.set_dreg(
            2,
            u64::from_le_bytes([0x00, 0x01, 0x80, 0xff, 0x0f, 0xf0, 0x55, 0xaa]),
        );
        cpu.execute_arm(enc_neon_2reg_misc(4, 0, 0, 9, false, 2), 0, &mut mem)
            .unwrap(); // vclz.i8 d4, d2
        assert_eq!(cpu.dreg(4), u64::from_le_bytes([8, 7, 0, 0, 4, 0, 1, 0]));

        cpu.execute_arm(enc_neon_2reg_misc(5, 0, 0, 10, false, 2), 0, &mut mem)
            .unwrap(); // vcnt.8 d5, d2
        assert_eq!(cpu.dreg(5), u64::from_le_bytes([0, 1, 1, 8, 4, 4, 4, 4]));

        cpu.set_dreg(8, u64::from_le_bytes([0, 1, 0xff, 0x7f, 0x80, 2, 0xfe, 3]));
        cpu.execute_arm(enc_neon_2reg_misc(9, 0, 1, 0, false, 8), 0, &mut mem)
            .unwrap(); // vcgt.s8 d9, d8, #0
        assert_eq!(
            cpu.dreg(9),
            u64::from_le_bytes([0, 0xff, 0, 0xff, 0, 0xff, 0, 0xff])
        );

        cpu.set_dreg(10, pack_i16x4([-2, 0, 3, i16::MIN]));
        cpu.execute_arm(enc_neon_2reg_misc(11, 1, 1, 3, false, 10), 0, &mut mem)
            .unwrap(); // vcle.s16 d11, d10, #0
        assert_eq!(cpu.dreg(11), pack_u16x4([0xffff, 0xffff, 0, 0xffff]));

        cpu.set_dreg(
            12,
            u64::from(0.0f32.to_bits()) | (u64::from((-2.0f32).to_bits()) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(13, 2, 1, 10, false, 12), 0, &mut mem)
            .unwrap(); // vceq.f32 d13, d12, #0
        assert_eq!(cpu.dreg(13), 0x0000_0000_ffff_ffff);

        cpu.set_dreg(
            14,
            u64::from(0x8000_0000u32) | (u64::from(0x4000_0000u32) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(15, 2, 3, 8, false, 14), 0, &mut mem)
            .unwrap(); // vrecpe.u32 d15, d14
        assert_eq!(
            cpu.dreg(15),
            u64::from(0xff80_0000u32) | (u64::from(u32::MAX) << 32)
        );

        cpu.set_dreg(
            16,
            u64::from(0x8000_0000u32) | (u64::from(0x2000_0000u32) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(17, 2, 3, 9, false, 16), 0, &mut mem)
            .unwrap(); // vrsqrte.u32 d17, d16
        assert_eq!(
            cpu.dreg(17),
            u64::from(0xb480_0000u32) | (u64::from(u32::MAX) << 32)
        );

        cpu.set_dreg(
            14,
            u64::from(2.0f32.to_bits()) | (u64::from(4.0f32.to_bits()) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(15, 2, 3, 10, false, 14), 0, &mut mem)
            .unwrap(); // vrecpe.f32 d15, d14
        assert_eq!(
            cpu.dreg(15),
            u64::from(0.5f32.to_bits()) | (u64::from(0.25f32.to_bits()) << 32)
        );

        cpu.set_dreg(
            16,
            u64::from(4.0f32.to_bits()) | (u64::from(16.0f32.to_bits()) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(17, 2, 3, 11, false, 16), 0, &mut mem)
            .unwrap(); // vrsqrte.f32 d17, d16
        assert_eq!(
            cpu.dreg(17),
            u64::from(0.5f32.to_bits()) | (u64::from(0.25f32.to_bits()) << 32)
        );

        cpu.set_dreg(
            18,
            u64::from(1.75f32.to_bits()) | (u64::from((-2.25f32).to_bits()) << 32),
        );
        cpu.execute_arm(enc_neon_2reg_misc(19, 2, 3, 14, false, 18), 0, &mut mem)
            .unwrap(); // vcvt.s32.f32 d19, d18
        assert_eq!(
            cpu.dreg(19),
            u64::from(1u32) | (u64::from((-2i32) as u32) << 32)
        );

        cpu.set_dreg(20, u64::from((-3i32) as u32) | (u64::from(10u32) << 32));
        cpu.execute_arm(enc_neon_2reg_misc(21, 2, 3, 12, false, 20), 0, &mut mem)
            .unwrap(); // vcvt.f32.s32 d21, d20
        assert_eq!(
            cpu.dreg(21),
            u64::from((-3.0f32).to_bits()) | (u64::from(10.0f32.to_bits()) << 32)
        );

        let pack_i16x8 = |values: [i16; 8]| {
            let mut bytes = [0u8; 16];
            for (idx, value) in values.into_iter().enumerate() {
                bytes[idx * 2..idx * 2 + 2].copy_from_slice(&value.to_le_bytes());
            }
            [
                u64::from_le_bytes(bytes[..8].try_into().expect("low q half")),
                u64::from_le_bytes(bytes[8..].try_into().expect("high q half")),
            ]
        };
        let q = pack_i16x8([-1, 0, 127, 128, 255, 300, -300, i16::MIN]);
        cpu.cpsr.q = false;
        cpu.set_dreg(24, q[0]);
        cpu.set_dreg(25, q[1]);
        cpu.execute_arm(enc_neon_2reg_misc(22, 0, 2, 4, true, 24), 0, &mut mem)
            .unwrap(); // vqmovun.s16 d22, q12
        assert_eq!(
            cpu.dreg(22),
            u64::from_le_bytes([0, 0, 127, 128, 255, 255, 0, 0])
        );
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.execute_arm(enc_neon_2reg_misc(23, 0, 2, 5, false, 24), 0, &mut mem)
            .unwrap(); // vqmovn.s16 d23, q12
        assert_eq!(
            cpu.dreg(23),
            u64::from_le_bytes([0xff, 0, 0x7f, 0x7f, 0x7f, 0x7f, 0x80, 0x80])
        );
        assert!(cpu.cpsr.q);

        let q = pack_i16x8([0, 1, 127, 128, 255, 300, -1, 42]);
        cpu.cpsr.q = false;
        cpu.set_dreg(24, q[0]);
        cpu.set_dreg(25, q[1]);
        cpu.execute_arm(enc_neon_2reg_misc(24, 0, 2, 5, true, 24), 0, &mut mem)
            .unwrap(); // vqmovn.u16 d24, q12
        assert_eq!(
            cpu.dreg(24),
            u64::from_le_bytes([0, 1, 127, 127, 127, 127, 127, 42])
        );
        assert!(cpu.cpsr.q);

        cpu.cpsr.q = false;
        cpu.set_dreg(6, u64::from_le_bytes([0x80, 0xff, 0x7f, 0x01, 0, 0, 0, 0]));
        cpu.execute_arm(enc_neon_2reg_misc(7, 0, 0, 14, false, 6), 0, &mut mem)
            .unwrap(); // vqabs.s8 d7, d6
        assert_eq!(
            cpu.dreg(7),
            u64::from_le_bytes([0x7f, 1, 0x7f, 1, 0, 0, 0, 0])
        );
        assert!(cpu.cpsr.q);
    }

    #[test]
    fn neon_immediate_shift_and_narrow_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(16, 0x8000_0000_1234_5678);
        cpu.set_dreg(17, 0xffff_0000_0001_0000);
        assert_eq!(
            enc_neon_2reg_shift(true, 48, 20, 0, false, true, 16),
            0xf3f0_4070
        );
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 48, 20, 0, false, true, 16),
            0,
            &mut mem,
        )
        .unwrap(); // vshr.u32 q10, q8, #16
        assert_eq!(cpu.dreg(20), 0x0000_8000_0000_1234);
        assert_eq!(cpu.dreg(21), 0x0000_ffff_0000_0001);

        cpu.set_dreg(22, 0xaaaa_aaaa_aaaa_aaaa);
        cpu.set_dreg(18, 0x0000_ffff_0000_ffff);
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 48, 22, 5, false, false, 18),
            0,
            &mut mem,
        )
        .unwrap(); // vsli.32 d22, d18, #16
        assert_eq!(cpu.dreg(22), 0xffff_aaaa_ffff_aaaa);

        cpu.set_dreg(16, 2);
        cpu.set_dreg(17, (-2i64) as u64);
        assert_eq!(
            enc_neon_2reg_shift(true, 63, 7, 8, false, false, 16),
            0xf3bf_7830
        );
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 63, 7, 8, false, false, 16),
            0,
            &mut mem,
        )
        .unwrap(); // vqshrun.s64 d7, q8, #1
        assert_eq!(cpu.dreg(7), 1);
        assert!(cpu.cpsr.q);

        let pack2 = |a: f32, b: f32| u64::from(a.to_bits()) | (u64::from(b.to_bits()) << 32);
        cpu.fpscr = 0;
        cpu.set_dreg(26, pack2(1.5, 2.0));
        cpu.set_dreg(27, pack2(0.5, 3.0));
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 41, 22, 15, false, true, 26),
            0,
            &mut mem,
        )
        .unwrap(); // vcvt.u32.f32 q11, q13, #23
        assert_eq!(cpu.dreg(22), 0x0100_0000_00c0_0000);
        assert_eq!(cpu.dreg(23), 0x0180_0000_0040_0000);
        assert_eq!(cpu.fpscr & FPSCR_IOC, 0);

        cpu.set_dreg(2, 0x0100_0000_00c0_0000);
        cpu.set_dreg(3, 0x0180_0000_0040_0000);
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 41, 22, 14, false, true, 2),
            0,
            &mut mem,
        )
        .unwrap(); // vcvt.f32.u32 q11, q1, #23
        assert_eq!(f32::from_bits(cpu.sreg(44)), 1.5);
        assert_eq!(f32::from_bits(cpu.sreg(45)), 2.0);
        assert_eq!(f32::from_bits(cpu.sreg(46)), 0.5);
        assert_eq!(f32::from_bits(cpu.sreg(47)), 3.0);
    }

    #[test]
    fn neon_three_different_length_integer_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(2, u64::from_le_bytes([1, 2, 250, 255, 0, 128, 3, 4]));
        cpu.set_dreg(3, u64::from_le_bytes([10, 20, 10, 2, 1, 1, 7, 8]));
        cpu.execute_arm(enc_neon_3diff(true, 0, 2, 0, 0, 3), 0, &mut mem)
            .unwrap(); // vaddl.u8 q0, d2, d3
        assert_eq!(cpu.dreg(0), u64::from_le_bytes([11, 0, 22, 0, 4, 1, 1, 1]));
        assert_eq!(
            cpu.dreg(1),
            u64::from_le_bytes([1, 0, 129, 0, 10, 0, 12, 0])
        );

        cpu.set_dreg(8, 10);
        cpu.set_dreg(9, 20);
        cpu.set_dreg(10, u64::from_le_bytes([3, 0, 0, 0, 4, 0, 0, 0]));
        cpu.set_dreg(11, u64::from_le_bytes([5, 0, 0, 0, 6, 0, 0, 0]));
        cpu.execute_arm(enc_neon_3diff(true, 2, 10, 8, 8, 11), 0, &mut mem)
            .unwrap(); // vmlal.u32 q4, d10, d11
        assert_eq!(cpu.dreg(8), 25);
        assert_eq!(cpu.dreg(9), 44);

        cpu.set_dreg(12, u64::from_le_bytes([0xe8, 3, 5, 0, 0x40, 0x9c, 0, 0]));
        cpu.set_dreg(
            13,
            u64::from_le_bytes([1, 0, 10, 0, 0x30, 0x75, 0xff, 0xff]),
        );
        cpu.execute_arm(enc_neon_3diff(true, 1, 12, 14, 7, 13), 0, &mut mem)
            .unwrap(); // vabdl.u16 q7, d12, d13
        assert_eq!(
            cpu.dreg(14),
            u64::from_le_bytes([0xe7, 3, 0, 0, 5, 0, 0, 0])
        );
        assert_eq!(
            cpu.dreg(15),
            u64::from_le_bytes([0x10, 0x27, 0, 0, 0xff, 0xff, 0, 0])
        );

        cpu.set_dreg(
            0,
            u64::from_le_bytes([0xff, 0, 0, 1, 0x34, 0x12, 0xff, 0xff]),
        );
        cpu.set_dreg(1, u64::from_le_bytes([0, 0, 0, 0, 0x80, 0, 0, 0]));
        cpu.set_dreg(4, u64::from_le_bytes([0, 0, 1, 0, 0, 1, 1, 0]));
        cpu.set_dreg(5, u64::from_le_bytes([0, 0, 0, 0, 0x80, 0, 0, 0]));
        cpu.execute_arm(enc_neon_3diff(true, 0, 0, 18, 4, 4), 0, &mut mem)
            .unwrap(); // vraddhn.i16 d18, q0, q2
        assert_eq!(
            cpu.dreg(18),
            u64::from_le_bytes([1, 1, 0x13, 0, 0, 0, 1, 0])
        );

        cpu.cpsr.q = false;
        cpu.set_dreg(2, u64::from_le_bytes([0, 0x40, 0, 0xc0, 0, 0, 1, 0]));
        cpu.set_dreg(3, u64::from_le_bytes([0, 0x40, 0, 0x40, 0, 0, 1, 0]));
        cpu.execute_arm(enc_neon_3diff(false, 1, 2, 20, 13, 3), 0, &mut mem)
            .unwrap(); // vqdmull.s16 q10, d2, d3
        assert_eq!(cpu.dreg(20), 0xe000_0000_2000_0000);
        assert_eq!(cpu.dreg(21), 0x0000_0002_0000_0000);
        assert!(!cpu.cpsr.q);

        cpu.set_dreg(22, 1);
        cpu.set_dreg(23, 0);
        cpu.execute_arm(enc_neon_3diff(false, 1, 2, 22, 9, 3), 0, &mut mem)
            .unwrap(); // vqdmlal.s16 q11, d2, d3
        assert_eq!(cpu.dreg(22), 0xe000_0000_2000_0001);
        assert_eq!(cpu.dreg(23), 0x0000_0002_0000_0000);
    }

    #[test]
    fn neon_long_move_and_shift_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_dreg(
            16,
            u64::from_le_bytes([0x34, 0x12, 0xff, 0, 0xcd, 0xab, 0, 0]),
        );
        cpu.set_dreg(17, u64::from_le_bytes([1, 0, 2, 0, 3, 0, 4, 0]));
        cpu.execute_arm(enc_neon_2reg_misc(18, 0, 2, 4, false, 16), 0, &mut mem)
            .unwrap(); // vmovn.i16 d18, q8
        assert_eq!(
            cpu.dreg(18),
            u64::from_le_bytes([0x34, 0xff, 0xcd, 0, 1, 2, 3, 4])
        );

        cpu.set_dreg(18, u64::from_le_bytes([1, 0, 0, 0, 0, 0, 0, 0x80]));
        cpu.execute_arm(
            enc_neon_2reg_shift(true, 36, 20, 10, false, false, 18),
            0,
            &mut mem,
        )
        .unwrap(); // vshll.u32 q10, d18, #4
        assert_eq!(cpu.dreg(20), 16);
        assert_eq!(cpu.dreg(21), 0x0000_0008_0000_0000);

        cpu.execute_arm(enc_neon_2reg_misc(22, 2, 2, 6, false, 18), 0, &mut mem)
            .unwrap(); // vshll.i32 q11, d18, #32
        assert_eq!(cpu.dreg(22), 0x0000_0001_0000_0000);
        assert_eq!(cpu.dreg(23), 0x8000_0000_0000_0000);
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
    fn arm_cp15_timer_mrrc_is_user_mode_hle_counter() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.execute_arm(0xec51_0f1e, 0, &mut mem).unwrap(); // mrrc p15, #1, r0, r1, c14
        assert_eq!((u64::from(cpu.reg(1)) << 32) | u64::from(cpu.reg(0)), 1);
        cpu.execute_arm(0xec51_2f1e, 4, &mut mem).unwrap(); // mrrc p15, #1, r2, r1, c14
        assert_eq!((u64::from(cpu.reg(1)) << 32) | u64::from(cpu.reg(2)), 1001);

        let err = cpu.execute_arm(0xec50_0f1e, 0x10, &mut mem).unwrap_err(); // mrrc p15, #1, r0, r0, c14
        assert_eq!(
            err,
            Trap::Unpredictable("CP15 MRRC/MCRR invalid core registers")
        );
        let err = cpu.execute_arm(0xec41_0f1e, 0x14, &mut mem).unwrap_err(); // mcrr p15, #1, r0, r1, c14
        assert_eq!(
            err,
            Trap::Privileged {
                pc: 0x14,
                instr: 0xec41_0f1e,
                operation: "MCRR CP15 timer",
            }
        );
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
    fn thumb_high_register_add_reads_unaligned_pc_plus_four() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(0, 0x0006_32fa);
        cpu.execute_thumb(0x4478, 0x0004_bad6, &mut mem).unwrap(); // add r0, pc
        assert_eq!(cpu.reg(0), 0x000a_edd4);
    }

    #[test]
    fn thumb_compare_and_branch_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.set_reg(5, 0);
        cpu.execute_thumb(0xb325, 0xa202e2, &mut mem).unwrap(); // cbz r5, +0x48
        assert_eq!(cpu.pc(), 0xa2032e);

        cpu.set_pc(0);
        cpu.set_reg(5, 1);
        cpu.execute_thumb(0xb325, 0xa202e2, &mut mem).unwrap(); // not taken
        assert_eq!(cpu.pc(), 0);

        cpu.execute_thumb(0xbb25, 0xa202e2, &mut mem).unwrap(); // cbnz r5, +0x48
        assert_eq!(cpu.pc(), 0xa2032e);
    }

    #[test]
    fn thumb_it_block_conditions_follow_mask() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x3000, 0x100);
        mem.load_thumb_halfwords(
            0x3000,
            &[
                0xbf14, // ite ne
                0x2001, // movs r0, #1
                0x2002, // movs r0, #2
                0xbf94, // ite ls
                0x2103, // movs r1, #3
                0x2104, // movs r1, #4
            ],
        )
        .unwrap();

        cpu.set_isa(Isa::Thumb);
        cpu.set_pc(0x3000);
        cpu.cpsr.z = false;
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(0), 1);

        cpu.set_pc(0x3000);
        cpu.set_reg(0, 0);
        cpu.cpsr.z = true;
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(0), 2);

        cpu.set_pc(0x3006);
        cpu.cpsr.c = false;
        cpu.cpsr.z = false;
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(1), 3);

        cpu.set_pc(0x3006);
        cpu.set_reg(1, 0);
        cpu.cpsr.c = true;
        cpu.cpsr.z = false;
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(cpu.reg(1), 4);
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
    fn thumb32_branch_clz_and_pop_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0x4b000, 0x2000);
        cpu.set_isa(Isa::Thumb);

        cpu.set_pc(0x4b694);
        mem.load_thumb_halfwords(0x4b694, &[0xf7ff, 0xece6])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // blx 0x4b064
        assert_eq!(cpu.pc(), 0x4b064);
        assert!(!cpu.cpsr.t);
        assert_eq!(cpu.reg(14), 0x4b699);

        cpu.set_isa(Isa::Thumb);
        cpu.set_pc(0x4b6b0);
        mem.load_thumb_halfwords(0x4b6b0, &[0xf000, 0xbd6e])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // b.w 0x4c190
        assert_eq!(cpu.pc(), 0x4c190);
        assert!(cpu.cpsr.t);

        cpu.set_pc(0x4b6c0);
        mem.load_thumb_halfwords(0x4b6c0, &[0xf3bf, 0x8f5b])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // dmb ish
        assert_eq!(cpu.pc(), 0x4b6c4);
        assert!(cpu.cpsr.t);

        cpu.set_pc(0x4b6d8);
        cpu.set_reg(14, 0);
        mem.load_thumb_halfwords(0x4b6d8, &[0xf1be, 0x0f00])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // cmp.w lr, #0
        assert!(cpu.cpsr.z);

        cpu.set_pc(0x4b6dc);
        cpu.set_reg(14, 1);
        mem.load_thumb_halfwords(0x4b6dc, &[0xf1be, 0x0f00])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // cmp.w lr, #0
        assert!(!cpu.cpsr.z);

        cpu.set_pc(0x7036_177a);
        cpu.cpsr.z = true;
        cpu.execute_thumb32(0xf000, 0x80b1, 0x7036_1776, &mut mem)
            .unwrap(); // beq.w 0x703618dc
        assert_eq!(cpu.pc(), 0x7036_18dc);

        cpu.set_pc(0x7036_177a);
        cpu.cpsr.z = false;
        cpu.execute_thumb32(0xf000, 0x80b1, 0x7036_1776, &mut mem)
            .unwrap(); // beq.w not taken
        assert_eq!(cpu.pc(), 0x7036_177a);

        cpu.set_pc(0x4ba20);
        cpu.set_reg(2, 3);
        mem.load_thumb_halfwords(0x4ba20, &[0xe8df, 0xf002])
            .unwrap();
        mem.load_bytes(0x4ba24, &[2, 7, 9, 15]).unwrap();
        cpu.step(&mut mem).unwrap(); // tbb [pc, r2]
        assert_eq!(cpu.pc(), 0x4ba42);

        cpu.set_pc(0x4ba24);
        cpu.set_reg(1, 0x4c000);
        cpu.set_reg(3, 2);
        mem.load_thumb_halfwords(0x4ba24, &[0xe8d1, 0xf013])
            .unwrap();
        mem.store16(0x4c004, 0x12).unwrap();
        cpu.step(&mut mem).unwrap(); // tbh [r1, r3, lsl #1]
        assert_eq!(cpu.pc(), 0x4ba4c);

        mem.store32(0x4c000, 0x1111_2222).unwrap();
        cpu.set_reg(3, 0x4c000);
        cpu.set_pc(0x4b6c4);
        mem.load_thumb_halfwords(0x4b6c4, &[0xe853, 0x5f00])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrex r5, [r3]
        assert_eq!(cpu.reg(5), 0x1111_2222);

        cpu.set_reg(2, 0x3333_4444);
        cpu.set_pc(0x4b6c8);
        mem.load_thumb_halfwords(0x4b6c8, &[0xe843, 0x2100])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // strex r1, r2, [r3]
        assert_eq!(cpu.reg(1), 0);
        assert_eq!(mem.load32(0x4c000).unwrap(), 0x3333_4444);

        mem.store8(0x4c010, 0x7f).unwrap();
        cpu.set_reg(0, 0x4c010);
        cpu.set_pc(0x4b6cc);
        mem.load_thumb_halfwords(0x4b6cc, &[0xe8d0, 0x2f4f])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrexb r2, [r0]
        assert_eq!(cpu.reg(2), 0x7f);

        cpu.set_reg(1, 0xab);
        cpu.set_pc(0x4b6d0);
        mem.load_thumb_halfwords(0x4b6d0, &[0xe8c0, 0x1f43])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // strexb r3, r1, [r0]
        assert_eq!(cpu.reg(3), 0);
        assert_eq!(mem.load8(0x4c010).unwrap(), 0xab);

        mem.store16(0x4c012, 0x1234).unwrap();
        cpu.set_reg(0, 0x4c012);
        cpu.set_pc(0x4b6d4);
        mem.load_thumb_halfwords(0x4b6d4, &[0xe8d0, 0x2f5f])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrexh r2, [r0]
        assert_eq!(cpu.reg(2), 0x1234);

        cpu.set_reg(1, 0x5678);
        cpu.set_pc(0x4b6d8);
        mem.load_thumb_halfwords(0x4b6d8, &[0xe8c0, 0x1f53])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // strexh r3, r1, [r0]
        assert_eq!(cpu.reg(3), 0);
        assert_eq!(mem.load16(0x4c012).unwrap(), 0x5678);

        cpu.set_pc(0x4b6a2);
        cpu.set_reg(0, 0x0000_00f0);
        mem.load_thumb_halfwords(0x4b6a2, &[0xfab0, 0xf380])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // clz r3, r0
        assert_eq!(cpu.reg(3), 24);
        assert_eq!(cpu.pc(), 0x4b6a6);

        cpu.set_pc(0x4b6a6);
        cpu.set_reg(8, 1);
        cpu.set_reg(4, 4);
        mem.load_thumb_halfwords(0x4b6a6, &[0xfa08, 0xf104])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // lsl.w r1, r8, r4
        assert_eq!(cpu.reg(1), 16);

        cpu.set_pc(0x4b6aa);
        cpu.set_reg(3, 0xe0);
        mem.load_thumb_halfwords(0x4b6aa, &[0xea4f, 0x1353])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // lsr.w r3, r3, #5
        assert_eq!(cpu.reg(3), 7);

        cpu.set_pc(0x4b6ae);
        cpu.set_reg(2, 0x1234_abcd);
        mem.load_thumb_halfwords(0x4b6ae, &[0xf3c2, 0x020b])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ubfx r2, r2, #0, #12
        assert_eq!(cpu.reg(2), 0xbcd);

        cpu.set_pc(0x4b6d0);
        cpu.set_reg(2, 0x8000_0001);
        mem.load_thumb_halfwords(0x4b6d0, &[0xf012, 0x0201])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ands r2, r2, #1
        assert_eq!(cpu.reg(2), 1);
        assert!(!cpu.cpsr.n);
        assert!(!cpu.cpsr.z);

        cpu.set_pc(0x4b74c);
        mem.load_thumb_halfwords(0x4b74c, &[0xf04f, 0x35ff])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // mov.w r5, #0xffffffff
        assert_eq!(cpu.reg(5), u32::MAX);

        cpu.set_pc(0x4b974);
        mem.load_thumb_halfwords(0x4b974, &[0xeeb0, 0x6b00])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // vmov.f64 d6, #2.0
        assert_eq!(f64::from_bits(cpu.dreg(6)), 2.0);

        cpu.set_pc(0x4b978);
        mem.load_thumb_halfwords(0x4b978, &[0xf44f, 0x7080])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // mov.w r0, #0x100
        assert_eq!(cpu.reg(0), 0x100);

        cpu.set_pc(0x4b97c);
        cpu.set_reg(13, 0x4c200);
        cpu.set_reg(4, 0x4444);
        cpu.set_reg(11, 0xbbbb);
        cpu.set_reg(14, 0xeeee);
        mem.load_thumb_halfwords(0x4b97c, &[0xe92d, 0x4ff0])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // push.w {r4-r11, lr}
        assert_eq!(cpu.reg(13), 0x4c1dc);
        assert_eq!(mem.load32(0x4c1dc).unwrap(), 0x4444);
        assert_eq!(mem.load32(0x4c1f8).unwrap(), 0xbbbb);
        assert_eq!(mem.load32(0x4c1fc).unwrap(), 0xeeee);

        cpu.set_pc(0x4b990);
        cpu.set_reg(6, 0x1000);
        mem.load_thumb_halfwords(0x4b990, &[0xf506, 0x78a8])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // add.w r8, r6, #0x150
        assert_eq!(cpu.reg(8), 0x1150);

        cpu.set_pc(0x4b994);
        cpu.set_reg(4, 0x703b_4dbc);
        mem.load_thumb_halfwords(0x4b994, &[0xf204, 0x101d])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // addw r0, r4, #0x11d
        assert_eq!(cpu.reg(0), 0x703b_4ed9);

        cpu.set_pc(0x4b998);
        mem.load_thumb_halfwords(0x4b998, &[0xf2a4, 0x111d])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // subw r1, r4, #0x11d
        assert_eq!(cpu.reg(1), 0x703b_4c9f);

        cpu.set_pc(0x4b99c);
        mem.load_thumb_halfwords(0x4b99c, &[0xf64a, 0x32cd])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // movw r2, #0xabcd
        assert_eq!(cpu.reg(2), 0xabcd);

        cpu.set_pc(0x4b9a0);
        mem.load_thumb_halfwords(0x4b9a0, &[0xf2c1, 0x2234])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // movt r2, #0x1234
        assert_eq!(cpu.reg(2), 0x1234_abcd);

        cpu.set_pc(0x4b9a4);
        mem.load_thumb_halfwords(0x4b9a4, &[0xf20f, 0x0320])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // adr.w r3, #0x20
        assert_eq!(cpu.reg(3), 0x4b9c8);

        cpu.set_pc(0x4b9a8);
        cpu.set_reg(3, 7);
        cpu.set_reg(4, 11);
        mem.load_thumb_halfwords(0x4b9a8, &[0xfb04, 0xf203])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // mul r2, r4, r3
        assert_eq!(cpu.reg(2), 77);

        cpu.set_pc(0x4b9ac);
        cpu.set_reg(1, 5);
        mem.load_thumb_halfwords(0x4b9ac, &[0xfb04, 0x1203])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // mla r2, r4, r3, r1
        assert_eq!(cpu.reg(2), 82);

        cpu.set_pc(0x4b9b0);
        cpu.set_reg(2, 6);
        cpu.set_reg(3, 7);
        cpu.set_reg(4, 100);
        mem.load_thumb_halfwords(0x4b9b0, &[0xfb02, 0x4513])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // mls r5, r2, r3, r4
        assert_eq!(cpu.reg(5), 58);

        cpu.set_pc(0x4b9b4);
        cpu.set_reg(4, 0x0d20_c51b);
        cpu.set_reg(7, 0xcccc_cccd);
        mem.load_thumb_halfwords(0x4b9b4, &[0xfba4, 0x0107])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // umull r0, r1, r4, r7
        let product = 0x0d20_c51bu64 * 0xcccc_cccdu64;
        assert_eq!(cpu.reg(0), product as u32);
        assert_eq!(cpu.reg(1), (product >> 32) as u32);

        cpu.set_pc(0x4b9b2);
        cpu.set_reg(2, 0x1111_2222);
        cpu.set_reg(3, 0x3333_4444);
        cpu.set_reg(6, 0x4c200);
        mem.load_thumb_halfwords(0x4b9b2, &[0xe9c6, 0x2314])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // strd r2, r3, [r6, #80]
        assert_eq!(mem.load32(0x4c250).unwrap(), 0x1111_2222);
        assert_eq!(mem.load32(0x4c254).unwrap(), 0x3333_4444);
        assert_eq!(cpu.reg(6), 0x4c200);

        cpu.set_pc(0x4b9b6);
        mem.store32(0x4c278, 0x5555_6666).unwrap();
        mem.store32(0x4c27c, 0x7777_8888).unwrap();
        mem.load_thumb_halfwords(0x4b9b6, &[0xe9d6, 0x451e])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrd r4, r5, [r6, #120]
        assert_eq!(cpu.reg(4), 0x5555_6666);
        assert_eq!(cpu.reg(5), 0x7777_8888);
        assert_eq!(cpu.reg(6), 0x4c200);

        cpu.set_pc(0x4ba04);
        cpu.set_reg(1, 0x5555_6666);
        cpu.set_reg(6, 0x4c200);
        mem.load_thumb_halfwords(0x4ba04, &[0xf8c6, 0x1098])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // str.w r1, [r6, #0x98]
        assert_eq!(mem.load32(0x4c298).unwrap(), 0x5555_6666);

        cpu.set_pc(0x4ba08);
        cpu.set_reg(3, 0x4c2a0);
        cpu.set_reg(14, 0x1234_56ee);
        mem.load_thumb_halfwords(0x4ba08, &[0xf883, 0xe000])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // strb.w lr, [r3]
        assert_eq!(mem.load8(0x4c2a0).unwrap(), 0xee);

        cpu.set_pc(0x4ba0c);
        mem.load_thumb_halfwords(0x4ba0c, &[0xf893, 0x2000])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrb.w r2, [r3]
        assert_eq!(cpu.reg(2), 0xee);

        cpu.set_pc(0x4ba10);
        mem.store8(0x4c2a1, 0xf1).unwrap();
        mem.load_thumb_halfwords(0x4ba10, &[0xf993, 0x1001])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrsb.w r1, [r3, #1]
        assert_eq!(cpu.reg(1), 0xffff_fff1);

        cpu.set_pc(0x4ba14);
        mem.store16(0x4c2ae, 0x8001).unwrap();
        mem.load_thumb_halfwords(0x4ba14, &[0xf9b3, 0x000e])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrsh.w r0, [r3, #0xe]
        assert_eq!(cpu.reg(0), 0xffff_8001);

        cpu.set_pc(0x4ba18);
        cpu.set_reg(5, 0x4c2c0);
        cpu.set_reg(2, 6);
        mem.store16(0x4c2cc, 0xff80).unwrap();
        mem.load_thumb_halfwords(0x4ba18, &[0xf935, 0x4012])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldrsh.w r4, [r5, r2, lsl #1]
        assert_eq!(cpu.reg(4), 0xffff_ff80);

        cpu.set_pc(0x4ba10);
        mem.load_thumb_halfwords(0x4ba10, &[0xf8df, 0x4498])
            .unwrap();
        mem.store32(0x4beac, 0x1234_5678).unwrap();
        cpu.step(&mut mem).unwrap(); // ldr.w r4, [pc, #0x498]
        assert_eq!(cpu.reg(4), 0x1234_5678);

        cpu.set_pc(0x4ba14);
        cpu.set_reg(13, 0x4c260);
        mem.store32(0x4c260, 0x4bb01).unwrap();
        mem.load_thumb_halfwords(0x4ba14, &[0xf85d, 0xfb04])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldr pc, [sp], #4
        assert_eq!(cpu.pc(), 0x4bb00);
        assert!(cpu.cpsr.t);
        assert_eq!(cpu.reg(13), 0x4c264);

        cpu.set_isa(Isa::Thumb);
        cpu.set_pc(0x4ba18);
        cpu.set_reg(2, 0xffff_fffe);
        cpu.set_reg(3, 3);
        mem.load_thumb_halfwords(0x4ba18, &[0xfb82, 0x0103])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // smull r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 0xffff_fffa);
        assert_eq!(cpu.reg(1), 0xffff_ffff);

        cpu.set_pc(0x4ba1c);
        cpu.set_reg(1, 0x0002_0003);
        cpu.set_reg(2, 0x0004_0005);
        cpu.set_reg(3, 10);
        mem.load_thumb_halfwords(0x4ba1c, &[0xfb21, 0x3002])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // smlad r0, r1, r2, r3
        assert_eq!(cpu.reg(0), 33);

        cpu.set_pc(0x4ba18);
        cpu.set_reg(6, 0x4c280);
        cpu.set_reg(7, 3);
        mem.store32(0x4c28c, 0xfeed_face).unwrap();
        mem.load_thumb_halfwords(0x4ba18, &[0xf856, 0x0027])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldr.w r0, [r6, r7, lsl #2]
        assert_eq!(cpu.reg(0), 0xfeed_face);

        cpu.set_pc(0x4ba1c);
        cpu.set_reg(4, 0x4c300);
        mem.store32(0x4c300, 0x1234_5678).unwrap();
        mem.store32(0x4c304, 0x9abc_def0).unwrap();
        mem.load_thumb_halfwords(0x4ba1c, &[0xe894, 0x0060])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // ldm.w r4, {r5, r6}
        assert_eq!(cpu.reg(5), 0x1234_5678);
        assert_eq!(cpu.reg(6), 0x9abc_def0);
        assert_eq!(cpu.reg(4), 0x4c300);

        cpu.set_pc(0x4b758);
        cpu.set_reg(0, 0xaaaa_bbbb);
        cpu.set_reg(1, 0x4c100);
        cpu.set_reg(2, 0xcccc_dddd);
        mem.load_thumb_halfwords(0x4b758, &[0xe881, 0x0005])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // stm.w r1, {r0, r2}
        assert_eq!(mem.load32(0x4c100).unwrap(), 0xaaaa_bbbb);
        assert_eq!(mem.load32(0x4c104).unwrap(), 0xcccc_dddd);
        assert_eq!(cpu.reg(1), 0x4c100);

        cpu.set_pc(0x4b7b8);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0x4c180);
        cpu.cpsr.n = false;
        mem.store32(0x4c180, 0).unwrap();
        mem.load_thumb_halfwords(0x4b7b8, &[0xbf5c, 0x2101, 0x6011])
            .unwrap();
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(mem.load32(0x4c180).unwrap(), 1);

        cpu.set_pc(0x4b7b8);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0x4c180);
        cpu.cpsr.n = true;
        mem.store32(0x4c180, 0).unwrap();
        for _ in 0..3 {
            cpu.step(&mut mem).unwrap();
        }
        assert_eq!(mem.load32(0x4c180).unwrap(), 0);

        cpu.set_pc(0x4b6ac);
        cpu.set_reg(13, 0x4c000);
        mem.store32(0x4c000, 0x1234_5678).unwrap();
        mem.store32(0x4c004, 0x8765_4321).unwrap();
        mem.load_thumb_halfwords(0x4b6ac, &[0xe8bd, 0x4010])
            .unwrap();
        cpu.step(&mut mem).unwrap(); // pop.w {r4, lr}
        assert_eq!(cpu.reg(4), 0x1234_5678);
        assert_eq!(cpu.reg(14), 0x8765_4321);
        assert_eq!(cpu.reg(13), 0x4c008);
        assert_eq!(cpu.pc(), 0x4b6b0);
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

        cpu.set_reg(1, 0xaf28_6bcb);
        cpu.execute_arm(0xe7eb_0651, 0, &mut mem).unwrap(); // ubfx r0, r1, #12, #12
        assert_eq!(cpu.reg(0), 0x286);

        cpu.set_reg(1, 0x00f0_0000);
        cpu.execute_arm(0xe7ab_0651, 0, &mut mem).unwrap(); // sbfx r0, r1, #12, #12
        assert_eq!(cpu.reg(0), 0xffff_ff00);

        cpu.set_reg(0, 0xffff_ffff);
        cpu.execute_arm(0xe7cf_041f, 0, &mut mem).unwrap(); // bfc r0, #8, #8
        assert_eq!(cpu.reg(0), 0xffff_00ff);

        cpu.set_reg(0, 0xffff_00ff);
        cpu.set_reg(1, 0x0000_00ab);
        cpu.execute_arm(0xe7cf_0411, 0, &mut mem).unwrap(); // bfi r0, r1, #8, #8
        assert_eq!(cpu.reg(0), 0xffff_abff);

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
        let err = cpu.execute_arm(0xe7eb_065f, 0, &mut mem).unwrap_err(); // ubfx r0, pc, #12, #12
        assert!(matches!(
            err,
            Trap::Unpredictable("bitfield extract with PC register")
        ));
        let err = cpu.execute_arm(0xe7cf_f411, 0, &mut mem).unwrap_err(); // bfi pc, r1, #8, #8
        assert!(matches!(
            err,
            Trap::Unpredictable("bitfield insert with PC destination")
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
    fn vfpv3_immediate_moves_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 4);

        cpu.execute_arm(0xeeb0_6b00, 0, &mut mem).unwrap(); // vmov.f64 d6, #2.0
        assert_eq!(f64::from_bits(cpu.dreg(6)), 2.0);

        cpu.execute_arm(0xeeb7_0a00, 0, &mut mem).unwrap(); // vmov.f32 s0, #1.0
        assert_eq!(f32::from_bits(cpu.sreg(0)), 1.0);
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
    fn vfpv3_d16_to_d31_double_registers_function() {
        let mut cpu = Cpu::new();
        let mut mem = VecMemory::new(0, 0x100);

        cpu.set_dreg(13, 2.0f64.to_bits());
        cpu.set_dreg(14, 1.0f64.to_bits());
        cpu.execute_arm(0xee3e_fb0d, 0, &mut mem).unwrap(); // vadd.f64 d15, d14, d13
        assert_eq!(f64::from_bits(cpu.dreg(15)), 3.0);

        cpu.set_reg(2, 0x5566_7788);
        cpu.set_reg(3, 0x1122_3344);
        cpu.execute_arm(0xec43_2b30, 0, &mut mem).unwrap(); // vmov d16, r2, r3
        assert_eq!(cpu.dreg(16), 0x1122_3344_5566_7788);

        cpu.execute_arm(0xec51_0b30, 0, &mut mem).unwrap(); // vmov r0, r1, d16
        assert_eq!(cpu.reg(0), 0x5566_7788);
        assert_eq!(cpu.reg(1), 0x1122_3344);

        cpu.set_dreg(0, 1.5f64.to_bits());
        cpu.set_dreg(1, 2.5f64.to_bits());
        cpu.execute_arm(0xee70_0b01, 0, &mut mem).unwrap(); // vadd.f64 d16, d0, d1
        assert_eq!(f64::from_bits(cpu.dreg(16)), 4.0);

        cpu.execute_arm(0xee30_2b81, 0, &mut mem).unwrap(); // vadd.f64 d2, d16, d1
        assert_eq!(f64::from_bits(cpu.dreg(2)), 6.5);

        cpu.set_sreg(1, 3.25f32.to_bits());
        cpu.execute_arm(0xeef7_0ae0, 0, &mut mem).unwrap(); // vcvt.f64.f32 d16, s1
        assert_eq!(f64::from_bits(cpu.dreg(16)), 3.25);

        cpu.execute_arm(0xeeb7_1be0, 0, &mut mem).unwrap(); // vcvt.f32.f64 s2, d16
        assert_eq!(f32::from_bits(cpu.sreg(2)), 3.25);
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

        cpu.execute_arm(0xedd0_0b00, 0, &mut mem).unwrap(); // vldr d16, [r0]

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

        let err = cpu.execute_arm(0xece0_fb04, 0, &mut mem).unwrap_err(); // vstmia r0!, {d31-d32}
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
