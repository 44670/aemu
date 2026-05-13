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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeError {
    MemoryMap(MappedMemoryError),
    Memory(String),
    AddressOverflow,
    Cpu(Trap),
    CpuAt { pc: u32, isa: Isa, source: Trap },
    UnknownHleTrap { pc: u32 },
    Hle { name: String, source: HleError },
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
        for _ in 0..max_steps {
            if self.cpu.pc() == CALL_RETURN_SENTINEL {
                return Ok(());
            }
            self.step()?;
        }
        Err(NativeRuntimeError::Cpu(Trap::StepLimit))
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
