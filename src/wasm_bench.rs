use crate::armv7a::{Cpu, Memory, RELEASE_BATCH_BUDGET, ReleaseBatchStops};
use crate::guest_memory::MappedMemory;

const CODE_BASE: u32 = 0x0001_0000;
const DATA_BASE: u32 = 0x0002_0000;

#[unsafe(no_mangle)]
pub extern "C" fn aemu_wasm_memory_benchmark(iterations: u32) -> u32 {
    let mut memory = MappedMemory::new();
    memory.map_zeroed(DATA_BASE, 0x1000).unwrap();
    for offset in (0..0x400).step_by(4) {
        memory
            .store32(DATA_BASE + offset, offset ^ 0x9e37_79b9)
            .unwrap();
    }

    let mut state = 0x243f_6a88u32;
    for iteration in 0..iterations {
        let addr = DATA_BASE + ((state >> 20) & 0x3fc);
        let value = memory.load32(addr).unwrap();
        state = state
            .rotate_left(7)
            .wrapping_add(value)
            .wrapping_add(iteration ^ 0xa5a5_5a5a);
        memory.store32(addr, state).unwrap();
    }
    state ^ memory.load32(DATA_BASE + 0x3fc).unwrap()
}

#[unsafe(no_mangle)]
pub extern "C" fn aemu_wasm_cpu_benchmark(iterations: u32) -> u32 {
    let mut memory = MappedMemory::new();
    memory.map_zeroed(CODE_BASE, 0x1000).unwrap();
    memory.map_zeroed(DATA_BASE, 0x1000).unwrap();
    memory
        .load_bytes(
            CODE_BASE,
            &[
                0x00, 0x20, 0x90, 0xe5, // ldr r2, [r0]
                0x01, 0x20, 0x82, 0xe0, // add r2, r2, r1
                0x00, 0x20, 0x80, 0xe5, // str r2, [r0]
                0x01, 0x30, 0x53, 0xe2, // subs r3, r3, #1
                0xfa, 0xff, 0xff, 0x1a, // bne CODE_BASE
            ],
        )
        .unwrap();
    memory.store32(DATA_BASE, 0x1357_9bdf).unwrap();

    let iterations = iterations.max(1);
    let mut cpu = Cpu::new();
    cpu.set_reg(0, DATA_BASE);
    cpu.set_reg(1, 3);
    cpu.set_reg(3, iterations);
    cpu.set_reg(15, CODE_BASE);
    let mut trap = None;
    let outcome = cpu.run_release_batch(
        &mut memory,
        iterations.saturating_mul(5),
        ReleaseBatchStops::default(),
        &mut trap,
    );
    assert_eq!(outcome.reason, RELEASE_BATCH_BUDGET);
    assert!(trap.is_none());

    memory.load32(DATA_BASE).unwrap() ^ cpu.reg(3) ^ cpu.reg(15)
}
