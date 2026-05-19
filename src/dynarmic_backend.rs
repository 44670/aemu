use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::armv7a::{Cpsr, Cpu, Isa, Memory, Trap};

const CPSR_USER_MODE: u32 = 0x10;
const EXEC_PAGE_SHIFT: u32 = 12;
const EXEC_PAGE_COUNT: usize = 1usize << (32 - EXEC_PAGE_SHIFT);
const EXEC_PAGE_BITMAP_WORDS: usize = EXEC_PAGE_COUNT / 64;

#[repr(C)]
struct AemuDynarmicCallbacks {
    user: *mut c_void,
    read8: unsafe extern "C" fn(*mut c_void, u32, *mut bool) -> u8,
    read16: unsafe extern "C" fn(*mut c_void, u32, *mut bool) -> u16,
    read32: unsafe extern "C" fn(*mut c_void, u32, *mut bool) -> u32,
    read64: unsafe extern "C" fn(*mut c_void, u32, *mut bool) -> u64,
    write8: unsafe extern "C" fn(*mut c_void, u32, u8) -> bool,
    write16: unsafe extern "C" fn(*mut c_void, u32, u16) -> bool,
    write32: unsafe extern "C" fn(*mut c_void, u32, u32) -> bool,
    write64: unsafe extern "C" fn(*mut c_void, u32, u64) -> bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DynarmicStepResult {
    pub halt_reason: u32,
    pub exception_pc: u32,
    pub memory_abort_addr: u32,
    pub exception_kind: i32,
    pub svc: u32,
    pub ticks_used: u64,
    pub svc_valid: bool,
    pub memory_abort: bool,
    pub interpreter_fallback: bool,
}

#[allow(non_camel_case_types)]
enum AemuDynarmic {}

unsafe extern "C" {
    fn aemu_dynarmic_new(
        callbacks: AemuDynarmicCallbacks,
        page_table: *mut *mut u8,
    ) -> *mut AemuDynarmic;
    fn aemu_dynarmic_free(dynarmic: *mut AemuDynarmic);
    fn aemu_dynarmic_set_user(dynarmic: *mut AemuDynarmic, user: *mut c_void);
    fn aemu_dynarmic_set_regs(dynarmic: *mut AemuDynarmic, regs16: *const u32);
    fn aemu_dynarmic_get_regs(dynarmic: *const AemuDynarmic, regs16: *mut u32);
    fn aemu_dynarmic_set_ext_regs(dynarmic: *mut AemuDynarmic, regs64: *const u32);
    fn aemu_dynarmic_get_ext_regs(dynarmic: *const AemuDynarmic, regs64: *mut u32);
    fn aemu_dynarmic_set_cpsr(dynarmic: *mut AemuDynarmic, value: u32);
    fn aemu_dynarmic_get_cpsr(dynarmic: *const AemuDynarmic) -> u32;
    fn aemu_dynarmic_set_fpscr(dynarmic: *mut AemuDynarmic, value: u32);
    fn aemu_dynarmic_get_fpscr(dynarmic: *const AemuDynarmic) -> u32;
    fn aemu_dynarmic_set_cp15(
        dynarmic: *mut AemuDynarmic,
        tpidrurw: u32,
        tpidruro: u32,
        virtual_counter: u64,
    );
    fn aemu_dynarmic_get_cp15(
        dynarmic: *const AemuDynarmic,
        tpidrurw: *mut u32,
        tpidruro: *mut u32,
        virtual_counter: *mut u64,
    );
    fn aemu_dynarmic_step(dynarmic: *mut AemuDynarmic) -> DynarmicStepResult;
    fn aemu_dynarmic_run(dynarmic: *mut AemuDynarmic, ticks: u64) -> DynarmicStepResult;
    fn aemu_dynarmic_clear_cache(dynarmic: *mut AemuDynarmic);
    fn aemu_dynarmic_invalidate_cache_range(dynarmic: *mut AemuDynarmic, start: u32, len: usize);
}

pub struct DynarmicA32<M: Memory> {
    raw: NonNull<AemuDynarmic>,
    executable_pages: Vec<u64>,
    host_page_table: Option<Vec<*mut u8>>,
    _memory: PhantomData<fn(&mut M)>,
}

struct DynarmicMemoryContext<'a, M: Memory> {
    memory: &'a mut M,
    executable_pages: &'a [u64],
    dirty_executable_writes: Vec<(u32, u32)>,
    trap: Option<Trap>,
}

impl<M: Memory> DynarmicA32<M> {
    pub fn new() -> Self {
        Self::new_with_host_page_table(None, Vec::new())
    }

    fn new_with_host_page_table(
        mut host_page_table: Option<Vec<*mut u8>>,
        executable_pages: Vec<u64>,
    ) -> Self {
        let callbacks = AemuDynarmicCallbacks {
            user: std::ptr::null_mut(),
            read8: read8::<M>,
            read16: read16::<M>,
            read32: read32::<M>,
            read64: read64::<M>,
            write8: write8::<M>,
            write16: write16::<M>,
            write32: write32::<M>,
            write64: write64::<M>,
        };
        let page_table = host_page_table
            .as_mut()
            .map(|table| table.as_mut_ptr())
            .unwrap_or(std::ptr::null_mut());
        let raw = unsafe { aemu_dynarmic_new(callbacks, page_table) };
        let raw = NonNull::new(raw).expect("aemu_dynarmic_new returned null");
        Self {
            raw,
            executable_pages,
            host_page_table,
            _memory: PhantomData,
        }
    }

    pub fn set_executable_ranges<I>(&mut self, ranges: I)
    where
        I: IntoIterator<Item = (u32, u32)>,
    {
        let pages = executable_pages_for_ranges(ranges);
        self.exclude_host_page_table_pages(&pages);
        self.executable_pages = pages;
    }

    pub fn load_cpu(&mut self, cpu: &Cpu) {
        let regs: [u32; 16] = core::array::from_fn(|idx| cpu.reg(idx));
        let ext_regs: [u32; 64] = core::array::from_fn(|idx| cpu.sreg(idx));
        unsafe {
            aemu_dynarmic_set_regs(self.raw.as_ptr(), regs.as_ptr());
            aemu_dynarmic_set_ext_regs(self.raw.as_ptr(), ext_regs.as_ptr());
            aemu_dynarmic_set_cpsr(self.raw.as_ptr(), cpu.cpsr.to_u32() | CPSR_USER_MODE);
            aemu_dynarmic_set_fpscr(self.raw.as_ptr(), cpu.fpscr);
            aemu_dynarmic_set_cp15(
                self.raw.as_ptr(),
                cpu.cp15_tpidrurw,
                cpu.cp15_tpidruro,
                cpu.cp15_virtual_counter(),
            );
        }
    }

    pub fn store_cpu(&self, cpu: &mut Cpu) {
        let mut regs = [0u32; 16];
        let mut ext_regs = [0u32; 64];
        unsafe {
            aemu_dynarmic_get_regs(self.raw.as_ptr(), regs.as_mut_ptr());
            aemu_dynarmic_get_ext_regs(self.raw.as_ptr(), ext_regs.as_mut_ptr());
        }
        for (idx, value) in regs.into_iter().enumerate() {
            cpu.set_reg(idx, value);
        }
        for (idx, value) in ext_regs.into_iter().enumerate() {
            cpu.set_sreg(idx, value);
        }
        cpu.cpsr = Cpsr::from_u32(unsafe { aemu_dynarmic_get_cpsr(self.raw.as_ptr()) });
        cpu.fpscr = unsafe { aemu_dynarmic_get_fpscr(self.raw.as_ptr()) };
        let mut cp15_tpidrurw = 0;
        let mut cp15_tpidruro = 0;
        let mut cp15_virtual_counter = 0;
        unsafe {
            aemu_dynarmic_get_cp15(
                self.raw.as_ptr(),
                &mut cp15_tpidrurw,
                &mut cp15_tpidruro,
                &mut cp15_virtual_counter,
            );
        }
        cpu.cp15_tpidrurw = cp15_tpidrurw;
        cpu.cp15_tpidruro = cp15_tpidruro;
        cpu.set_cp15_virtual_counter(cp15_virtual_counter);
    }

    pub fn step_with_memory(&mut self, memory: &mut M) -> Result<DynarmicStepResult, Trap> {
        let mut context = DynarmicMemoryContext {
            memory,
            executable_pages: &self.executable_pages,
            dirty_executable_writes: Vec::new(),
            trap: None,
        };
        unsafe {
            aemu_dynarmic_set_user(
                self.raw.as_ptr(),
                (&mut context as *mut DynarmicMemoryContext<'_, M>).cast::<c_void>(),
            );
        }
        let result = unsafe { aemu_dynarmic_step(self.raw.as_ptr()) };
        unsafe {
            aemu_dynarmic_set_user(self.raw.as_ptr(), std::ptr::null_mut());
        }
        let dirty_executable_writes = std::mem::take(&mut context.dirty_executable_writes);
        let trap = context.trap.take();
        drop(context);
        self.invalidate_dirty_executable_writes(&dirty_executable_writes);
        if let Some(trap) = trap
            && !result.memory_abort
        {
            return Err(trap);
        }
        Ok(result)
    }

    pub fn run_with_memory(
        &mut self,
        memory: &mut M,
        ticks: u64,
    ) -> Result<DynarmicStepResult, Trap> {
        let mut context = DynarmicMemoryContext {
            memory,
            executable_pages: &self.executable_pages,
            dirty_executable_writes: Vec::new(),
            trap: None,
        };
        unsafe {
            aemu_dynarmic_set_user(
                self.raw.as_ptr(),
                (&mut context as *mut DynarmicMemoryContext<'_, M>).cast::<c_void>(),
            );
        }
        let result = unsafe { aemu_dynarmic_run(self.raw.as_ptr(), ticks.max(1)) };
        unsafe {
            aemu_dynarmic_set_user(self.raw.as_ptr(), std::ptr::null_mut());
        }
        let dirty_executable_writes = std::mem::take(&mut context.dirty_executable_writes);
        let trap = context.trap.take();
        drop(context);
        self.invalidate_dirty_executable_writes(&dirty_executable_writes);
        if let Some(trap) = trap
            && !result.memory_abort
        {
            return Err(trap);
        }
        Ok(result)
    }

    pub fn clear_cache(&mut self) {
        unsafe {
            aemu_dynarmic_clear_cache(self.raw.as_ptr());
        }
    }

    pub fn invalidate_cache_range(&mut self, start: u32, len: usize) {
        unsafe {
            aemu_dynarmic_invalidate_cache_range(self.raw.as_ptr(), start, len);
        }
    }

    fn invalidate_dirty_executable_writes(&mut self, writes: &[(u32, u32)]) {
        for &(start, len) in writes {
            self.invalidate_cache_range(start, len as usize);
        }
    }

    fn exclude_host_page_table_pages(&mut self, pages: &[u64]) {
        let Some(page_table) = self.host_page_table.as_mut() else {
            return;
        };
        for (word_idx, mut word) in pages.iter().copied().enumerate() {
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                let page = word_idx * 64 + bit;
                if page < page_table.len() {
                    page_table[page] = std::ptr::null_mut();
                }
                word &= word - 1;
            }
        }
    }
}

impl DynarmicA32<crate::guest_memory::MappedMemory> {
    pub fn new_for_mapped_memory<I>(
        memory: &crate::guest_memory::MappedMemory,
        executable_ranges: I,
    ) -> Self
    where
        I: IntoIterator<Item = (u32, u32)>,
    {
        let executable_pages = executable_pages_for_ranges(executable_ranges);
        #[cfg(all(target_os = "linux", not(debug_assertions)))]
        {
            let host_page_table = memory.dynarmic_host_page_table_excluding(&executable_pages);
            Self::new_with_host_page_table(Some(host_page_table), executable_pages)
        }
        #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
        {
            let _ = memory;
            Self::new_with_host_page_table(None, executable_pages)
        }
    }
}

impl<M: Memory> Drop for DynarmicA32<M> {
    fn drop(&mut self) {
        unsafe {
            aemu_dynarmic_free(self.raw.as_ptr());
        }
    }
}

unsafe extern "C" fn read8<M: Memory>(user: *mut c_void, addr: u32, ok: *mut bool) -> u8 {
    with_context(
        user,
        ok,
        |ctx: &mut DynarmicMemoryContext<'_, M>| ctx.memory.load8(addr),
        0,
    )
}

unsafe extern "C" fn read16<M: Memory>(user: *mut c_void, addr: u32, ok: *mut bool) -> u16 {
    with_context(
        user,
        ok,
        |ctx: &mut DynarmicMemoryContext<'_, M>| ctx.memory.load16(addr),
        0,
    )
}

unsafe extern "C" fn read32<M: Memory>(user: *mut c_void, addr: u32, ok: *mut bool) -> u32 {
    with_context(
        user,
        ok,
        |ctx: &mut DynarmicMemoryContext<'_, M>| ctx.memory.load32(addr),
        0,
    )
}

unsafe extern "C" fn read64<M: Memory>(user: *mut c_void, addr: u32, ok: *mut bool) -> u64 {
    with_context(
        user,
        ok,
        |ctx: &mut DynarmicMemoryContext<'_, M>| {
            let lo = u64::from(ctx.memory.load32(addr)?);
            let hi = u64::from(ctx.memory.load32(addr.wrapping_add(4))?);
            Ok(lo | (hi << 32))
        },
        0,
    )
}

unsafe extern "C" fn write8<M: Memory>(user: *mut c_void, addr: u32, value: u8) -> bool {
    with_write_context(user, addr, 1, |ctx: &mut DynarmicMemoryContext<'_, M>| {
        ctx.memory.store8(addr, value)
    })
}

unsafe extern "C" fn write16<M: Memory>(user: *mut c_void, addr: u32, value: u16) -> bool {
    with_write_context(user, addr, 2, |ctx: &mut DynarmicMemoryContext<'_, M>| {
        ctx.memory.store16(addr, value)
    })
}

unsafe extern "C" fn write32<M: Memory>(user: *mut c_void, addr: u32, value: u32) -> bool {
    with_write_context(user, addr, 4, |ctx: &mut DynarmicMemoryContext<'_, M>| {
        ctx.memory.store32(addr, value)
    })
}

unsafe extern "C" fn write64<M: Memory>(user: *mut c_void, addr: u32, value: u64) -> bool {
    with_write_context(user, addr, 8, |ctx: &mut DynarmicMemoryContext<'_, M>| {
        ctx.memory.store32(addr, value as u32)?;
        ctx.memory
            .store32(addr.wrapping_add(4), (value >> 32) as u32)
    })
}

fn with_context<M: Memory, T: Copy>(
    user: *mut c_void,
    ok: *mut bool,
    f: impl FnOnce(&mut DynarmicMemoryContext<'_, M>) -> crate::armv7a::Result<T>,
    fallback: T,
) -> T {
    let Some(mut context) = NonNull::new(user.cast::<DynarmicMemoryContext<'_, M>>()) else {
        unsafe {
            *ok = false;
        }
        return fallback;
    };
    match f(unsafe { context.as_mut() }) {
        Ok(value) => {
            unsafe {
                *ok = true;
            }
            value
        }
        Err(err) => {
            unsafe {
                context.as_mut().trap = Some(err);
                *ok = false;
            }
            fallback
        }
    }
}

fn with_write_context<M: Memory>(
    user: *mut c_void,
    addr: u32,
    len: u32,
    f: impl FnOnce(&mut DynarmicMemoryContext<'_, M>) -> crate::armv7a::Result<()>,
) -> bool {
    let Some(mut context) = NonNull::new(user.cast::<DynarmicMemoryContext<'_, M>>()) else {
        return false;
    };
    let context = unsafe { context.as_mut() };
    match f(context) {
        Ok(()) => {
            if executable_write_intersects(context.executable_pages, addr, len) {
                context.dirty_executable_writes.push((addr, len));
            }
            true
        }
        Err(err) => {
            context.trap = Some(err);
            false
        }
    }
}

fn executable_write_intersects(executable_pages: &[u64], addr: u32, len: u32) -> bool {
    if executable_pages.is_empty() || len == 0 {
        return false;
    }
    let end = addr.saturating_add(len.saturating_sub(1));
    let first_page = page_index(addr);
    let last_page = page_index(end);
    for page in first_page..=last_page {
        if executable_pages[page / 64] & (1u64 << (page % 64)) != 0 {
            return true;
        }
    }
    false
}

fn executable_pages_for_ranges<I>(ranges: I) -> Vec<u64>
where
    I: IntoIterator<Item = (u32, u32)>,
{
    let mut pages = vec![0; EXEC_PAGE_BITMAP_WORDS];
    for (start, len) in ranges {
        if len == 0 {
            continue;
        }
        let end = start.saturating_add(len.saturating_sub(1));
        let first_page = page_index(start);
        let last_page = page_index(end);
        for page in first_page..=last_page {
            pages[page / 64] |= 1u64 << (page % 64);
        }
    }
    pages
}

fn page_index(addr: u32) -> usize {
    (addr >> EXEC_PAGE_SHIFT) as usize
}

pub fn dynarmic_cpsr_for_cpu(cpu: &Cpu) -> u32 {
    cpu.cpsr.to_u32() | CPSR_USER_MODE
}

pub fn cpu_isa_from_dynarmic_cpsr(cpsr: u32) -> Isa {
    if cpsr & (1 << 5) != 0 {
        Isa::Thumb
    } else {
        Isa::Arm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armv7a::VecMemory;

    #[test]
    fn dynarmic_executes_arm_code_through_aemu_memory_callbacks() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_arm_words(
                0,
                &[
                    0xe3a0_0005, // mov r0, #5
                    0xe280_0007, // add r0, r0, #7
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Arm);

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.load_cpu(&cpu);
        jit.step_with_memory(&mut memory).unwrap();
        jit.step_with_memory(&mut memory).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(cpu.reg(0), 12);
        assert_eq!(cpu.pc(), 8);
        assert_eq!(cpu.isa(), Isa::Arm);
    }

    #[test]
    fn dynarmic_executes_thumb_code_through_aemu_memory_callbacks() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_thumb_halfwords(
                0,
                &[
                    0x2005, // movs r0, #5
                    0x3007, // adds r0, #7
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Thumb);

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.load_cpu(&cpu);
        jit.step_with_memory(&mut memory).unwrap();
        jit.step_with_memory(&mut memory).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(cpu.reg(0), 12);
        assert_eq!(cpu.pc(), 4);
        assert_eq!(cpu.isa(), Isa::Thumb);
    }

    #[test]
    fn dynarmic_preserves_thumb_it_state_across_cpu_sync() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_thumb_halfwords(
                0,
                &[
                    0x2800, // cmp r0, #0
                    0xbf14, // ite ne
                    0x4604, // movne r4, r0
                    0x2401, // moveq r4, #1
                    0x4620, // mov r0, r4
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Thumb);
        cpu.set_reg(0, 0x2c);

        let mut jit = DynarmicA32::<VecMemory>::new();
        for _ in 0..5 {
            jit.load_cpu(&cpu);
            jit.step_with_memory(&mut memory).unwrap();
            jit.store_cpu(&mut cpu);
        }

        assert_eq!(cpu.reg(4), 0x2c);
        assert_eq!(cpu.reg(0), 0x2c);
        assert_eq!(cpu.pc(), 10);
        assert_eq!(cpu.isa(), Isa::Thumb);
        assert_eq!(cpu.cpsr.it, 0);
    }

    #[test]
    fn dynarmic_run_executes_a_tick_budget_without_per_instruction_sync() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_arm_words(
                0,
                &[
                    0xe3a0_0001, // mov r0, #1
                    0xe280_0002, // add r0, r0, #2
                    0xe280_0003, // add r0, r0, #3
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Arm);

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.load_cpu(&cpu);
        let result = jit.run_with_memory(&mut memory, 3).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(result.ticks_used, 3);
        assert_eq!(cpu.reg(0), 6);
        assert_eq!(cpu.pc(), 12);
        assert_eq!(cpu.isa(), Isa::Arm);
    }

    #[test]
    fn dynarmic_invalidates_cached_blocks_when_guest_writes_executable_range() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_arm_words(
                0,
                &[
                    0xe582_1000, // str r1, [r2]
                    0xe12f_ff13, // bx r3
                ],
            )
            .unwrap();
        memory
            .load_arm_words(
                0x100,
                &[
                    0xe3a0_0001, // mov r0, #1
                    0xe12f_ff1e, // bx lr
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0x100);
        cpu.set_isa(Isa::Arm);

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.set_executable_ranges([(0x100, 0x100)]);
        jit.load_cpu(&cpu);
        jit.step_with_memory(&mut memory).unwrap();
        jit.store_cpu(&mut cpu);
        assert_eq!(cpu.reg(0), 1);

        cpu.set_pc(0);
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0xe3a0_0002); // mov r0, #2
        cpu.set_reg(2, 0x100);
        jit.load_cpu(&cpu);
        jit.step_with_memory(&mut memory).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(memory.load32(0x100).unwrap(), 0xe3a0_0002);

        cpu.set_pc(0x100);
        cpu.set_isa(Isa::Arm);
        jit.load_cpu(&cpu);
        jit.step_with_memory(&mut memory).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(cpu.reg(0), 2);
    }

    #[test]
    fn dynarmic_cp15_tls_barrier_and_timer_match_aemu_user_mode_facades() {
        let mut memory = VecMemory::new(0, 0x1000);
        memory
            .load_arm_words(
                0,
                &[
                    0xee1d_0f70, // mrc p15, #0, r0, c13, c0, #3
                    0xee1d_2f50, // mrc p15, #0, r2, c13, c0, #2
                    0xee0d_1f50, // mcr p15, #0, r1, c13, c0, #2
                    0xee07_0fba, // mcr p15, #0, r0, c7, c10, #5
                    0xec51_4f1e, // mrrc p15, #1, r4, r5, c14
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(1, 0xfeed_cafe);
        cpu.cp15_tpidruro = 0x1234_5678;
        cpu.cp15_tpidrurw = 0xaabb_ccdd;

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.load_cpu(&cpu);
        jit.run_with_memory(&mut memory, 5).unwrap();
        jit.store_cpu(&mut cpu);

        assert_eq!(cpu.reg(0), 0x1234_5678);
        assert_eq!(cpu.reg(2), 0xaabb_ccdd);
        assert_eq!(cpu.cp15_tpidrurw, 0xfeed_cafe);
        assert_eq!((u64::from(cpu.reg(5)) << 32) | u64::from(cpu.reg(4)), 1);
        assert_eq!(cpu.cp15_virtual_counter(), 1001);
        assert_eq!(cpu.pc(), 20);
    }

    #[test]
    fn dynarmic_preserves_lr_through_push_pop_pc() {
        let mut memory = VecMemory::new(0, 0x2000);
        memory
            .load_arm_words(
                0,
                &[
                    0xe92d_4070, // push {r4, r5, r6, lr}
                    0xe3a0_4001, // mov r4, #1
                    0xe8bd_8070, // pop {r4, r5, r6, pc}
                ],
            )
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.set_pc(0);
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(4, 0xabcdef01);
        cpu.set_reg(5, 0xabcdef02);
        cpu.set_reg(6, 0xabcdef03);
        cpu.set_reg(13, 0x1000);
        cpu.set_reg(14, 0xfffe_fffc);

        let mut jit = DynarmicA32::<VecMemory>::new();
        jit.load_cpu(&cpu);
        let result = jit.run_with_memory(&mut memory, 3).unwrap();
        jit.store_cpu(&mut cpu);

        assert!(result.memory_abort);
        assert_eq!(result.memory_abort_addr, 0xfffe_fffc);
        assert_eq!(cpu.reg(4), 0xabcdef01);
        assert_eq!(cpu.reg(5), 0xabcdef02);
        assert_eq!(cpu.reg(6), 0xabcdef03);
        assert_eq!(cpu.reg(13), 0x1000);
    }
}
