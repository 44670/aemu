use std::fmt;

use crate::armv6::{Cpu, Trap};
use crate::guest_memory::MappedMemoryError;
use crate::hle_imports::{HLE_TRAP_ARM_INSTR, HleError, HleRuntime};
use crate::native_loader::NativeLinkReport;

pub const DEFAULT_STACK_BASE: u32 = 0x6d00_0000;
pub const DEFAULT_STACK_SIZE: usize = 0x0010_0000;
pub const DEFAULT_TLS_BASE: u32 = 0x6e00_0000;
pub const DEFAULT_TLS_SIZE: usize = 0x0001_0000;
pub const DEFAULT_HEAP_BASE: u32 = 0x6000_0000;
pub const DEFAULT_HEAP_SIZE: usize = 0x0400_0000;
const ERRNO_OFFSET: u32 = 0x100;

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
pub enum NativeRuntimeStep {
    GuestInstruction,
    HleCall { name: String, address: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeError {
    MemoryMap(MappedMemoryError),
    AddressOverflow,
    Cpu(Trap),
    UnknownHleTrap { pc: u32 },
    Hle { name: String, source: HleError },
}

impl fmt::Display for NativeRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryMap(err) => write!(f, "{err}"),
            Self::AddressOverflow => write!(f, "runtime address range overflow"),
            Self::Cpu(err) => write!(f, "{err}"),
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
        let sp = config
            .stack_base
            .checked_add(config.stack_size as u32)
            .ok_or(NativeRuntimeError::AddressOverflow)?
            & !7;
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
            Err(err) => Err(NativeRuntimeError::Cpu(err)),
        }
    }

    pub fn run(&mut self, max_steps: usize) -> Result<(), NativeRuntimeError> {
        for _ in 0..max_steps {
            self.step()?;
        }
        Err(NativeRuntimeError::Cpu(Trap::StepLimit))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::armv6::Memory;
    use crate::guest_memory::MappedMemory;
    use crate::hle_imports::{HleCallBehavior, HleSymbolKind, HleSymbolShape};
    use crate::native_loader::{HleSymbol, NativeLinkReport};

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
}
