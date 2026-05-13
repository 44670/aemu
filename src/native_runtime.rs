use std::collections::VecDeque;
use std::fmt;

use crate::armv6::{Cpu, Isa, Memory, Trap};
use crate::guest_memory::MappedMemoryError;
use crate::hle_imports::{HLE_TRAP_ARM_INSTR, HleError, HleRuntime};
use crate::native_loader::NativeLinkReport;

pub const DEFAULT_STACK_BASE: u32 = 0x6d00_0000;
pub const DEFAULT_STACK_SIZE: usize = 0x0010_0000;
pub const DEFAULT_TLS_BASE: u32 = 0x6e00_0000;
pub const DEFAULT_TLS_SIZE: usize = 0x0001_0000;
pub const DEFAULT_HEAP_BASE: u32 = 0x6000_0000;
pub const DEFAULT_HEAP_SIZE: usize = 0x0400_0000;
const STACK_ENTRY_HEADROOM_MAX: u32 = 0x1000;
const ERRNO_OFFSET: u32 = 0x100;
const CALL_RETURN_SENTINEL: u32 = 0xffff_fffc;
const RUN_FUNCTION_TRACE_LEN: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRuntimeConfig {
    pub stack_base: u32,
    pub stack_size: usize,
    pub tls_base: u32,
    pub tls_size: usize,
    pub heap_base: u32,
    pub heap_size: usize,
}

impl Default for NativeRuntimeConfig {
    fn default() -> Self {
        Self {
            stack_base: DEFAULT_STACK_BASE,
            stack_size: DEFAULT_STACK_SIZE,
            tls_base: DEFAULT_TLS_BASE,
            tls_size: DEFAULT_TLS_SIZE,
            heap_base: DEFAULT_HEAP_BASE,
            heap_size: DEFAULT_HEAP_SIZE,
        }
    }
}

pub struct NativeRuntime {
    pub link: NativeLinkReport,
    pub cpu: Cpu,
    pub hle: HleRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeConstructor {
    pub library_name: String,
    pub address: u32,
    pub source: NativeConstructorSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeConstructorSource {
    Init,
    InitArray,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeStep {
    GuestInstruction,
    HleCall { name: String, address: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRuntimeTraceEntry {
    pub pc: u32,
    pub isa: Isa,
    pub regs: [u32; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeError {
    MemoryMap(MappedMemoryError),
    Memory(String),
    AddressOverflow,
    Cpu(Trap),
    CpuAt {
        pc: u32,
        isa: Isa,
        source: Trap,
    },
    Traced {
        source: Box<NativeRuntimeError>,
        tail: Vec<NativeRuntimeTraceEntry>,
    },
    UnknownHleTrap {
        pc: u32,
    },
    Hle {
        name: String,
        source: HleError,
    },
}

impl fmt::Display for NativeRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryMap(err) => write!(f, "{err}"),
            Self::Memory(err) => write!(f, "{err}"),
            Self::AddressOverflow => write!(f, "runtime address range overflow"),
            Self::Cpu(err) => write!(f, "{err}"),
            Self::CpuAt { pc, isa, source } => {
                write!(f, "{source} while executing {isa:?} at {pc:#010x}")
            }
            Self::Traced { source, tail } => {
                write!(f, "{source}")?;
                if !tail.is_empty() {
                    write!(f, "\nrecent guest PCs:")?;
                    for entry in tail {
                        write!(
                            f,
                            "\n  {:?} pc={:#010x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} r4={:#010x} r5={:#010x} r6={:#010x} r7={:#010x} r8={:#010x} r9={:#010x} r10={:#010x} r11={:#010x} r12={:#010x} sp={:#010x} lr={:#010x}",
                            entry.isa,
                            entry.pc,
                            entry.regs[0],
                            entry.regs[1],
                            entry.regs[2],
                            entry.regs[3],
                            entry.regs[4],
                            entry.regs[5],
                            entry.regs[6],
                            entry.regs[7],
                            entry.regs[8],
                            entry.regs[9],
                            entry.regs[10],
                            entry.regs[11],
                            entry.regs[12],
                            entry.regs[13],
                            entry.regs[14],
                        )?;
                    }
                }
                Ok(())
            }
            Self::UnknownHleTrap { pc } => write!(f, "unknown HLE trap at {pc:#010x}"),
            Self::Hle { name, source } => write!(f, "HLE {name} failed: {source}"),
        }
    }
}

impl std::error::Error for NativeRuntimeError {}

impl NativeRuntime {
    pub fn new(
        mut link: NativeLinkReport,
        config: NativeRuntimeConfig,
    ) -> Result<Self, NativeRuntimeError> {
        link.memory
            .map_zeroed(config.stack_base, config.stack_size)
            .map_err(NativeRuntimeError::MemoryMap)?;
        link.memory
            .map_zeroed(config.tls_base, config.tls_size)
            .map_err(NativeRuntimeError::MemoryMap)?;
        link.memory
            .map_zeroed(config.heap_base, config.heap_size)
            .map_err(NativeRuntimeError::MemoryMap)?;

        let mut cpu = Cpu::new();
        let stack_size =
            u32::try_from(config.stack_size).map_err(|_| NativeRuntimeError::AddressOverflow)?;
        let stack_top = config
            .stack_base
            .checked_add(stack_size)
            .ok_or(NativeRuntimeError::AddressOverflow)?
            & !7;
        let entry_headroom = (stack_size / 16)
            .clamp(0x100, STACK_ENTRY_HEADROOM_MAX)
            .min(stack_size.saturating_sub(8));
        let sp = stack_top.wrapping_sub(entry_headroom) & !7;
        cpu.set_reg(13, sp);
        cpu.cp15_tpidrurw = config.tls_base;
        cpu.cp15_tpidruro = config.tls_base;

        let errno_addr = config
            .tls_base
            .checked_add(ERRNO_OFFSET)
            .ok_or(NativeRuntimeError::AddressOverflow)?;
        let hle = HleRuntime::new(errno_addr, config.heap_base, config.heap_size as u32);

        Ok(Self { link, cpu, hle })
    }

    pub fn step(&mut self) -> Result<NativeRuntimeStep, NativeRuntimeError> {
        let pc_before = self.cpu.pc();
        let isa_before = self.cpu.isa();
        match self.cpu.step(&mut self.link.memory) {
            Ok(()) => Ok(NativeRuntimeStep::GuestInstruction),
            Err(Trap::UndefinedArm { pc, instr }) if instr == HLE_TRAP_ARM_INSTR => {
                let Some(symbol) = self.link.hle_symbol_by_address(pc) else {
                    return Err(NativeRuntimeError::UnknownHleTrap { pc });
                };
                let name = symbol.name.clone();
                self.hle
                    .dispatch(&name, &mut self.cpu, &mut self.link.memory)
                    .map_err(|source| NativeRuntimeError::Hle {
                        name: name.clone(),
                        source,
                    })?;
                Ok(NativeRuntimeStep::HleCall { name, address: pc })
            }
            Err(Trap::Memory(err)) => Err(NativeRuntimeError::Memory(format!(
                "{err} while executing {isa_before:?} at {pc_before:#010x}"
            ))),
            Err(err) => Err(NativeRuntimeError::CpuAt {
                pc: pc_before,
                isa: isa_before,
                source: err,
            }),
        }
    }

    pub fn run(&mut self, max_steps: usize) -> Result<(), NativeRuntimeError> {
        for _ in 0..max_steps {
            self.step()?;
        }
        Err(NativeRuntimeError::Cpu(Trap::StepLimit))
    }

    pub fn constructors(&mut self) -> Result<Vec<NativeConstructor>, NativeRuntimeError> {
        let mut out = Vec::new();
        for object in &self.link.objects {
            if let Some(init) = object.init {
                if init != 0 {
                    out.push(NativeConstructor {
                        library_name: object.library_name.clone(),
                        address: init,
                        source: NativeConstructorSource::Init,
                    });
                }
            }
            if let Some(init_array) = object.init_array {
                for idx in 0..init_array.size / 4 {
                    let addr = init_array.addr.wrapping_add(idx * 4);
                    let value = self
                        .link
                        .memory
                        .load32(addr)
                        .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
                    if value == 0 {
                        continue;
                    }
                    out.push(NativeConstructor {
                        library_name: object.library_name.clone(),
                        address: value,
                        source: NativeConstructorSource::InitArray,
                    });
                }
            }
        }
        Ok(out)
    }

    pub fn run_function(
        &mut self,
        address: u32,
        max_steps: usize,
    ) -> Result<(), NativeRuntimeError> {
        self.cpu.branch_exchange(address);
        self.cpu.set_reg(14, CALL_RETURN_SENTINEL);
        let mut tail = VecDeque::with_capacity(RUN_FUNCTION_TRACE_LEN);
        let trace_range = parse_trace_pc_range();
        let trace_step_interval = parse_trace_step_interval();
        let trace_hle = parse_trace_hle_filter();
        let trace_pc_limit = parse_trace_limit("AEMU_TRACE_PC_LIMIT");
        let trace_hle_limit = parse_trace_limit("AEMU_TRACE_HLE_LIMIT");
        let mut trace_pc_count = 0usize;
        let mut trace_hle_count = 0usize;
        for step_idx in 0..max_steps {
            if self.cpu.pc() == CALL_RETURN_SENTINEL {
                return Ok(());
            }
            if tail.len() == RUN_FUNCTION_TRACE_LEN {
                tail.pop_front();
            }
            if trace_step_interval.is_some_and(|interval| step_idx != 0 && step_idx % interval == 0)
            {
                let pc = self.cpu.pc();
                eprintln!(
                    "STEP function={address:#010x} step={step_idx}/{max_steps} {:?} pc={pc:#010x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} sp={:#010x} lr={:#010x}",
                    self.cpu.isa(),
                    self.cpu.reg(0),
                    self.cpu.reg(1),
                    self.cpu.reg(2),
                    self.cpu.reg(3),
                    self.cpu.reg(13),
                    self.cpu.reg(14),
                );
            }
            if let Some((start, end)) = trace_range {
                let pc = self.cpu.pc();
                if pc >= start
                    && pc < end
                    && trace_pc_limit.map_or(true, |limit| trace_pc_count < limit)
                {
                    trace_pc_count += 1;
                    let instr16 = self.link.memory.load16(pc).unwrap_or(0xffff);
                    eprintln!(
                        "TRACE {:?} pc={pc:#010x} instr16={instr16:#06x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} r4={:#010x} r5={:#010x} r6={:#010x} r7={:#010x} r8={:#010x} r9={:#010x} r10={:#010x} r11={:#010x} r12={:#010x} sp={:#010x} lr={:#010x}",
                        self.cpu.isa(),
                        self.cpu.reg(0),
                        self.cpu.reg(1),
                        self.cpu.reg(2),
                        self.cpu.reg(3),
                        self.cpu.reg(4),
                        self.cpu.reg(5),
                        self.cpu.reg(6),
                        self.cpu.reg(7),
                        self.cpu.reg(8),
                        self.cpu.reg(9),
                        self.cpu.reg(10),
                        self.cpu.reg(11),
                        self.cpu.reg(12),
                        self.cpu.reg(13),
                        self.cpu.reg(14),
                    );
                }
            }
            tail.push_back(NativeRuntimeTraceEntry {
                pc: self.cpu.pc(),
                isa: self.cpu.isa(),
                regs: core::array::from_fn(|idx| self.cpu.reg(idx)),
            });
            match self.step() {
                Ok(NativeRuntimeStep::GuestInstruction) => {}
                Ok(NativeRuntimeStep::HleCall {
                    name,
                    address: hle_address,
                }) => {
                    if trace_hle
                        .as_ref()
                        .is_some_and(|filter| filter.matches(&name))
                        && trace_hle_limit.map_or(true, |limit| trace_hle_count < limit)
                    {
                        trace_hle_count += 1;
                        eprintln!(
                            "HLE function={:#010x} step={} pc={:#010x} name={} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x}",
                            address,
                            step_idx,
                            hle_address,
                            name,
                            self.cpu.reg(0),
                            self.cpu.reg(1),
                            self.cpu.reg(2),
                            self.cpu.reg(3),
                        );
                    }
                }
                Err(source) => {
                    return Err(NativeRuntimeError::Traced {
                        source: Box::new(source),
                        tail: tail.into_iter().collect(),
                    });
                }
            }
        }
        Err(NativeRuntimeError::Traced {
            source: Box::new(NativeRuntimeError::Cpu(Trap::StepLimit)),
            tail: tail.into_iter().collect(),
        })
    }

    pub fn run_constructors(
        &mut self,
        max_steps_per_constructor: usize,
    ) -> Result<(), NativeRuntimeError> {
        for constructor in self.constructors()? {
            self.run_function(constructor.address, max_steps_per_constructor)?;
        }
        Ok(())
    }
}

fn parse_trace_pc_range() -> Option<(u32, u32)> {
    let raw = std::env::var("AEMU_TRACE_PC_RANGE").ok()?;
    let (start, end) = raw.split_once(':')?;
    let start = parse_u32_env(start)?;
    let end = parse_u32_env(end)?;
    (start < end).then_some((start, end))
}

fn parse_trace_step_interval() -> Option<usize> {
    let raw = std::env::var("AEMU_TRACE_STEPS").ok()?;
    let interval = raw.trim().parse().ok()?;
    (interval != 0).then_some(interval)
}

fn parse_trace_limit(name: &str) -> Option<usize> {
    let raw = std::env::var(name).ok()?;
    let limit = raw.trim().parse().ok()?;
    (limit != 0).then_some(limit)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HleTraceFilter {
    All,
    Contains(String),
}

impl HleTraceFilter {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Contains(needle) => name.contains(needle),
        }
    }
}

fn parse_trace_hle_filter() -> Option<HleTraceFilter> {
    let raw = std::env::var("AEMU_TRACE_HLE").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else if raw == "*" {
        Some(HleTraceFilter::All)
    } else {
        Some(HleTraceFilter::Contains(raw.to_string()))
    }
}

fn parse_u32_env(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        raw.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::armv6::Memory;
    use crate::guest_memory::MappedMemory;
    use crate::hle_imports::{HleCallBehavior, HleSymbolKind, HleSymbolShape};
    use crate::native_loader::{HleSymbol, LoadedNativeObject, NativeLinkReport};

    use super::*;

    #[test]
    fn dispatches_hle_trap_by_guest_address_and_returns_through_lr() {
        let hle_address = 0x6f00_0000;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(hle_address, 0x1000).unwrap();
        memory.store32(hle_address, HLE_TRAP_ARM_INSTR).unwrap();
        memory.map_zeroed(0x5000_0000, 0x1000).unwrap();
        memory.load_bytes(0x5000_0100, b"hello\0").unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![HleSymbol {
                name: "strlen".to_string(),
                address: hle_address,
                kind: HleSymbolKind::Libc,
                shape: HleSymbolShape::Function,
                behavior: HleCallBehavior::Implemented,
            }],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };

        let config = NativeRuntimeConfig {
            stack_base: 0x5000_1000,
            stack_size: 0x1000,
            tls_base: 0x5000_2000,
            tls_size: 0x1000,
            heap_base: 0x5000_3000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        runtime.cpu.set_pc(hle_address);
        runtime.cpu.set_reg(0, 0x5000_0100);
        runtime.cpu.set_reg(14, 0x1234);

        let step = runtime.step().unwrap();

        assert_eq!(
            step,
            NativeRuntimeStep::HleCall {
                name: "strlen".to_string(),
                address: hle_address,
            }
        );
        assert_eq!(runtime.cpu.reg(0), 5);
        assert_eq!(runtime.cpu.pc(), 0x1234);
    }

    #[test]
    fn enumerates_and_runs_constructor_targets() {
        let hle_address = 0x6f00_0100;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x6f00_0000, 0x1000).unwrap();
        memory.store32(hle_address, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi".to_string(),
            memory,
            objects: vec![LoadedNativeObject {
                entry_name: "lib/armeabi/libgame.so".to_string(),
                library_name: "libgame.so".to_string(),
                load_bias: 0x7000_0000,
                memory_base: 0x7000_0000,
                memory_size: 0x1000,
                entry: 0x7000_0000,
                needed: Vec::new(),
                imports: Vec::new(),
                defined_symbols: Vec::new(),
                relocations: Vec::new(),
                relocation_count: 0,
                init: Some(hle_address),
                init_array: None,
            }],
            global_symbols: Vec::new(),
            hle_symbols: vec![HleSymbol {
                name: "pthread_self".to_string(),
                address: hle_address,
                kind: HleSymbolKind::Libc,
                shape: HleSymbolShape::Function,
                behavior: HleCallBehavior::Implemented,
            }],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };

        let config = NativeRuntimeConfig {
            stack_base: 0x5000_1000,
            stack_size: 0x1000,
            tls_base: 0x5000_2000,
            tls_size: 0x1000,
            heap_base: 0x5000_3000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();

        assert_eq!(
            runtime.constructors().unwrap(),
            vec![NativeConstructor {
                library_name: "libgame.so".to_string(),
                address: hle_address,
                source: NativeConstructorSource::Init,
            }]
        );

        runtime.run_constructors(8).unwrap();
        assert_eq!(runtime.cpu.reg(0), 1);
        assert_eq!(runtime.cpu.pc(), CALL_RETURN_SENTINEL);
    }
}
