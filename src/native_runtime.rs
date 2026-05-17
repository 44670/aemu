use std::collections::VecDeque;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::armv7a::{Cpu, Isa, Memory, Trap};
use crate::guest_memory::MappedMemoryError;
use crate::hle_imports::{
    CreatedPthread, HLE_TRAP_ARM_INSTR, HleError, HleRuntime, HleUnwindTable,
};
use crate::native_loader::NativeLinkReport;
use sha1::{Digest, Sha1};

pub const DEFAULT_STACK_BASE: u32 = 0x6c00_0000;
pub const DEFAULT_STACK_SIZE: usize = 0x0200_0000;
pub const DEFAULT_TLS_BASE: u32 = 0x6e00_0000;
pub const DEFAULT_TLS_SIZE: usize = 0x0001_0000;
pub const DEFAULT_HEAP_BASE: u32 = 0x6000_0000;
pub const DEFAULT_HEAP_SIZE: usize = 0x0800_0000;
const STACK_ENTRY_HEADROOM_MAX: u32 = 0x1000;
const ERRNO_OFFSET: u32 = 0x100;
const CALL_RETURN_SENTINEL: u32 = 0xffff_fffc;
const THREAD_RETURN_SENTINEL: u32 = 0xffff_fff8;
const RUN_FUNCTION_TRACE_LEN: usize = 24;
const GUEST_THREAD_TRACE_LEN: usize = 128;
const GUEST_THREAD_STACK_SIZE: u32 = 0x0004_0000;
const GUEST_THREAD_STACK_ALIGN: u32 = 8;
const GUEST_THREAD_SLICE_STEPS: usize = 4096;
const GUEST_THREAD_SERVICE_INTERVAL: usize = 50_000;
const GUEST_THREAD_SWAP_SERVICE_SLICES: usize = 64;
const MAIN_THREAD_WAIT_SPINS: usize = 1024;
const PTHREAD_ONCE_INIT_STEPS: usize = 100_000;
const GUEST_THREAD_SERVICE_HLE: &[&str] = &[
    "pthread_create",
    "pthread_cond_signal",
    "pthread_cond_broadcast",
    "pthread_cond_wait",
    "pthread_cond_timedwait",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    "sched_yield",
    "sem_post",
    "sem_wait",
    "usleep",
    "nanosleep",
];
const ANDROID_APP_CONFIG_OFFSET: u32 = 0x10;
const ANDROID_APP_LOOPER_OFFSET: u32 = 0x1c;
const ANDROID_APP_INPUT_QUEUE_OFFSET: u32 = 0x20;
const ANDROID_APP_WINDOW_OFFSET: u32 = 0x24;
const ANDROID_APP_ACTIVITY_STATE_OFFSET: u32 = 0x38;
const ANDROID_APP_DESTROY_REQUESTED_OFFSET: u32 = 0x3c;
const ANDROID_APP_INPUT_POLL_SOURCE_OFFSET: u32 = 0x60;
const ANDROID_APP_RUNNING_OFFSET: u32 = 0x6c;
const ANDROID_APP_PENDING_INPUT_QUEUE_OFFSET: u32 = 0x7c;
const ANDROID_APP_PENDING_WINDOW_OFFSET: u32 = 0x80;
const ANDROID_POLL_SOURCE_MAIN: u32 = 1;
const ANDROID_POLL_SOURCE_INPUT: u32 = 2;
const APP_CMD_INIT_WINDOW: u32 = 1;
const APP_CMD_GAINED_FOCUS: u32 = 6;
const APP_CMD_START: u32 = 10;
const APP_CMD_RESUME: u32 = 11;
const MCPE_LIBRARY: &str = "libminecraftpe.so";
const MCPE_GAME_RENDERER_RENDER: &str = "_ZN12GameRenderer6renderEf";
const MCPE_ON_RESOURCES_LOADED: &str = "_ZN15MinecraftClient17onResourcesLoadedEv";
const MCPE_GAME_RENDERER_RENDER_RESOURCE_GATE_OFFSET: u32 = 0x19e;
const MCPE_CLIENT_RESOURCES_READY_OFFSET: u32 = 0x23e;
const MCPE_ON_RESOURCES_LOADED_STEPS: usize = 100_000_000;
const MCPE_ON_RESOURCES_LOADED_STEPS_ENV: &str = "AEMU_MCPE_ON_RESOURCES_LOADED_STEPS";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCpuBackendKind {
    AemuInterpreter,
}

impl NativeCpuBackendKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "aemu" | "aemu-interpreter" => Some(Self::AemuInterpreter),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AemuInterpreter => "aemu",
        }
    }
}

pub struct NativeRuntime {
    pub link: NativeLinkReport,
    pub cpu: Cpu,
    pub hle: HleRuntime,
    cpu_backend: NativeCpuBackendKind,
    runtime_hle_traps: Vec<RuntimeHleTrap>,
    jni_methods: Vec<JniMethod>,
    jni_java_vm: u32,
    stack_base: u32,
    stack_size: u32,
    next_thread_stack_top: u32,
    guest_threads: VecDeque<GuestThread>,
    guest_mutexes: Vec<GuestMutex>,
    main_wait: GuestThreadWait,
    minecraft_resource_bridge: Option<MinecraftResourceBridge>,
    minecraft_resource_bridge_active: bool,
    trace_native_event_count: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeActivityHarness {
    pub activity: u32,
    pub callbacks: u32,
    pub java_vm: u32,
    pub jni_env: u32,
    pub activity_class: u32,
    pub asset_manager: u32,
    pub configuration: u32,
    pub looper: u32,
    pub input_queue: u32,
    pub window: u32,
    pub internal_data_path: u32,
    pub external_data_path: u32,
    pub obb_path: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeStep {
    GuestInstruction,
    HleCall {
        name: String,
        address: u32,
        args: [u32; 4],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeFunctionExit {
    Returned,
    HleCall {
        name: String,
        address: u32,
        args: [u32; 4],
        step: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRuntimeTraceEntry {
    pub pc: u32,
    pub isa: Isa,
    pub regs: [u32; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeHleTrap {
    address: u32,
    isa: Isa,
    name: &'static str,
    kind: RuntimeHleTrapKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeHleTrapKind {
    Jni(JniFunction),
    CxxDynamicCast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CxxTypeInfoKind {
    Class,
    SingleInheritance,
    VirtualMultipleInheritance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JniFunction {
    FindClass,
    RefIdentity,
    GetObjectClass,
    GetMethodId,
    GetStaticMethodId,
    GetFieldId,
    CallObjectMethod,
    CallStaticObjectMethod,
    CallIntMethod,
    CallStaticIntMethod,
    CallBooleanMethod,
    CallStaticBooleanMethod,
    CallVoidMethod,
    NewStringUtf,
    GetStringUtfLength,
    GetStringUtfChars,
    ReleaseStringUtfChars,
    GetArrayLength,
    GetIntArrayElements,
    ReleaseIntArrayElements,
    GetJavaVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JniMethod {
    id: u32,
    name: String,
    sig: String,
    is_static: bool,
}

#[derive(Debug, Clone)]
struct GuestThread {
    id: u32,
    cpu: Cpu,
    wait: GuestThreadWait,
    trace_tail: VecDeque<NativeRuntimeTraceEntry>,
    trace_pc_count: usize,
    trace_mem32_count: usize,
    trace_mem32_deref_count: usize,
    trace_cxx_string_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestThreadWait {
    Runnable,
    Condvar { cond: u32, mutex: u32 },
    Mutex { mutex: u32 },
}

impl GuestThreadWait {
    fn is_runnable(self) -> bool {
        matches!(self, Self::Runnable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuestMutex {
    addr: u32,
    owner: Option<u32>,
    recursion: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MinecraftResourceBridge {
    render_resource_gate_pc: u32,
    on_resources_loaded: u32,
}

fn push_trace_tail(tail: &mut VecDeque<NativeRuntimeTraceEntry>, cpu: &Cpu, max_len: usize) {
    if tail.len() == max_len {
        tail.pop_front();
    }
    tail.push_back(NativeRuntimeTraceEntry {
        pc: cpu.pc(),
        isa: cpu.isa(),
        regs: core::array::from_fn(|idx| cpu.reg(idx)),
    });
}

fn log_guest_thread_tail(thread: &GuestThread) {
    if thread.trace_tail.is_empty() {
        return;
    }
    eprintln!("THREAD recent id={} guest PCs:", thread.id);
    for entry in &thread.trace_tail {
        eprintln!(
            "  {:?} pc={:#010x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} sp={:#010x} lr={:#010x}",
            entry.isa,
            entry.pc,
            entry.regs[0],
            entry.regs[1],
            entry.regs[2],
            entry.regs[3],
            entry.regs[13],
            entry.regs[14],
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRuntimeError {
    MemoryMap(MappedMemoryError),
    Memory(String),
    AddressOverflow,
    UnsupportedCpuBackend(NativeCpuBackendKind),
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
            Self::UnsupportedCpuBackend(backend) => write!(
                f,
                "CPU backend '{}' is not wired into NativeRuntime yet",
                backend.as_str()
            ),
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
        link: NativeLinkReport,
        config: NativeRuntimeConfig,
    ) -> Result<Self, NativeRuntimeError> {
        Self::new_with_cpu_backend(link, config, NativeCpuBackendKind::AemuInterpreter)
    }

    pub fn new_with_cpu_backend(
        mut link: NativeLinkReport,
        config: NativeRuntimeConfig,
        cpu_backend: NativeCpuBackendKind,
    ) -> Result<Self, NativeRuntimeError> {
        if cpu_backend != NativeCpuBackendKind::AemuInterpreter {
            return Err(NativeRuntimeError::UnsupportedCpuBackend(cpu_backend));
        }

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
        let mut hle = HleRuntime::new(errno_addr, config.heap_base, config.heap_size as u32);
        match std::fs::read(&link.apk_path) {
            Ok(bytes) => hle.set_apk_bytes(bytes),
            Err(_) => hle.set_apk_path(link.apk_path.clone()),
        }
        hle.set_unwind_tables(collect_unwind_tables(&link));
        let runtime_hle_traps = collect_linked_runtime_hle_traps(&link);
        let minecraft_resource_bridge = if minecraft_resource_bridge_enabled() {
            build_minecraft_resource_bridge(&link)
        } else {
            None
        };

        Ok(Self {
            link,
            cpu,
            hle,
            cpu_backend,
            runtime_hle_traps,
            jni_methods: Vec::new(),
            jni_java_vm: 0,
            stack_base: config.stack_base,
            stack_size,
            next_thread_stack_top: config.stack_base,
            guest_threads: VecDeque::new(),
            guest_mutexes: Vec::new(),
            main_wait: GuestThreadWait::Runnable,
            minecraft_resource_bridge,
            minecraft_resource_bridge_active: false,
            trace_native_event_count: 0,
        })
    }

    pub fn cpu_backend(&self) -> NativeCpuBackendKind {
        self.cpu_backend
    }

    pub fn step(&mut self) -> Result<NativeRuntimeStep, NativeRuntimeError> {
        match self.cpu_backend {
            NativeCpuBackendKind::AemuInterpreter => self.step_aemu_interpreter(),
        }
    }

    fn step_aemu_interpreter(&mut self) -> Result<NativeRuntimeStep, NativeRuntimeError> {
        let pc_before = self.cpu.pc();
        let isa_before = self.cpu.isa();
        let args_before = core::array::from_fn(|idx| self.cpu.reg(idx));
        self.maybe_bridge_minecraft_resources_loaded(pc_before)?;
        if let Some(trap) = self.runtime_hle_entry(pc_before, isa_before) {
            self.dispatch_runtime_hle(trap)?;
            return Ok(NativeRuntimeStep::HleCall {
                name: trap.name.to_string(),
                address: pc_before,
                args: args_before,
            });
        }
        match self.cpu.step(&mut self.link.memory) {
            Ok(()) => Ok(NativeRuntimeStep::GuestInstruction),
            Err(Trap::UndefinedArm { pc, instr }) if instr == HLE_TRAP_ARM_INSTR => {
                if let Some(symbol) = self.link.hle_symbol_by_address(pc) {
                    let name = symbol.name.clone();
                    self.hle
                        .dispatch(&name, &mut self.cpu, &mut self.link.memory)
                        .map_err(|source| NativeRuntimeError::Hle {
                            name: name.clone(),
                            source,
                        })?;
                    return Ok(NativeRuntimeStep::HleCall {
                        name,
                        address: pc,
                        args: args_before,
                    });
                }
                if let Some(trap) = self
                    .runtime_hle_traps
                    .iter()
                    .find(|trap| trap.address == pc)
                    .copied()
                {
                    self.dispatch_runtime_hle(trap)?;
                    return Ok(NativeRuntimeStep::HleCall {
                        name: trap.name.to_string(),
                        address: pc,
                        args: args_before,
                    });
                }
                Err(NativeRuntimeError::UnknownHleTrap { pc })
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

    fn maybe_bridge_minecraft_resources_loaded(
        &mut self,
        pc: u32,
    ) -> Result<(), NativeRuntimeError> {
        let Some(bridge) = self.minecraft_resource_bridge else {
            return Ok(());
        };
        if self.minecraft_resource_bridge_active || pc != bridge.render_resource_gate_pc {
            return Ok(());
        }
        let client = self.cpu.reg(7);
        if client == 0 {
            return Ok(());
        }
        let ready_addr = client.wrapping_add(MCPE_CLIENT_RESOURCES_READY_OFFSET);
        let ready = self
            .link
            .memory
            .load8(ready_addr)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        if ready != 0 {
            return Ok(());
        }

        if std::env::var_os("AEMU_TRACE_MCPE_RESOURCE_BRIDGE").is_some() {
            eprintln!(
                "MCPE resource bridge: calling onResourcesLoaded={:#010x} client={client:#010x} ready@{ready_addr:#010x}",
                bridge.on_resources_loaded,
            );
        }

        self.minecraft_resource_bridge_active = true;
        let saved_cpu = self.cpu.clone();
        let result = self.run_function_with_args(
            bridge.on_resources_loaded,
            &[client],
            minecraft_on_resources_loaded_steps(),
        );
        self.cpu = saved_cpu;
        self.minecraft_resource_bridge_active = false;
        result?;

        if std::env::var_os("AEMU_TRACE_MCPE_RESOURCE_BRIDGE").is_some() {
            let ready_after = self
                .link
                .memory
                .load8(ready_addr)
                .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
            eprintln!("MCPE resource bridge: ready@{ready_addr:#010x} now {ready_after:#04x}");
        }
        Ok(())
    }

    pub fn run(&mut self, max_steps: usize) -> Result<(), NativeRuntimeError> {
        for _ in 0..max_steps {
            if let NativeRuntimeStep::HleCall { name, args, .. } = self.step()? {
                self.handle_thread_sync_hle(1, &name, args)?;
                self.wait_main_after_hle(&name, args)?;
            }
        }
        Err(NativeRuntimeError::Cpu(Trap::StepLimit))
    }

    pub fn symbol_address(&self, name: &str) -> Option<u32> {
        self.link
            .global_symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .map(|symbol| symbol.address)
    }

    pub fn symbol_address_in_library(&self, library_name: &str, name: &str) -> Option<u32> {
        self.link
            .objects
            .iter()
            .find(|object| object.library_name == library_name)
            .and_then(|object| {
                object
                    .defined_symbols
                    .iter()
                    .find(|symbol| symbol.name == name)
                    .map(|symbol| symbol.address)
            })
    }

    pub fn alloc_guest_zeroed(&mut self, size: u32, align: u32) -> Result<u32, NativeRuntimeError> {
        let ptr = self
            .hle
            .alloc(size, align)
            .map_err(|source| NativeRuntimeError::Hle {
                name: "runtime_alloc".to_string(),
                source,
            })?;
        for offset in 0..size {
            self.link
                .memory
                .store8(ptr.wrapping_add(offset), 0)
                .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        }
        Ok(ptr)
    }

    pub fn write_guest_bytes(&mut self, bytes: &[u8]) -> Result<u32, NativeRuntimeError> {
        let len = u32::try_from(bytes.len()).map_err(|_| NativeRuntimeError::AddressOverflow)?;
        let ptr = self.alloc_guest_zeroed(len.max(1), 1)?;
        self.link
            .memory
            .load_bytes(ptr, bytes)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        Ok(ptr)
    }

    pub fn write_guest_c_string(&mut self, value: &str) -> Result<u32, NativeRuntimeError> {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        self.write_guest_bytes(&bytes)
    }

    pub fn prepare_native_activity(&mut self) -> Result<NativeActivityHarness, NativeRuntimeError> {
        let (java_vm, jni_env) = self.prepare_jni_tables()?;
        let callbacks = self.alloc_guest_zeroed(0x40, 4)?;
        let activity = self.alloc_guest_zeroed(0x40, 4)?;
        let activity_class = self.alloc_guest_zeroed(0x10, 4)?;
        let asset_manager = self.alloc_guest_zeroed(0x10, 4)?;
        let configuration = self.alloc_guest_zeroed(0x20, 4)?;
        let looper = self.alloc_guest_zeroed(0x10, 4)?;
        let input_queue = self.alloc_guest_zeroed(0x10, 4)?;
        let window = self.alloc_guest_zeroed(0x10, 4)?;
        let internal_data_path = self.write_guest_c_string("/data/data/com.mojang.minecraftpe")?;
        let external_data_path =
            self.write_guest_c_string("/sdcard/Android/data/com.mojang.minecraftpe/files")?;
        let obb_path = self.write_guest_c_string("/sdcard/Android/obb/com.mojang.minecraftpe")?;

        self.store_runtime32(activity, callbacks)?;
        self.store_runtime32(activity.wrapping_add(0x04), java_vm)?;
        self.store_runtime32(activity.wrapping_add(0x08), jni_env)?;
        self.store_runtime32(activity.wrapping_add(0x0c), activity_class)?;
        self.store_runtime32(activity.wrapping_add(0x10), internal_data_path)?;
        self.store_runtime32(activity.wrapping_add(0x14), external_data_path)?;
        self.store_runtime32(activity.wrapping_add(0x18), 19)?; // Android 4.4 API level
        self.store_runtime32(activity.wrapping_add(0x1c), 0)?; // instance
        self.store_runtime32(activity.wrapping_add(0x20), asset_manager)?;
        self.store_runtime32(activity.wrapping_add(0x24), obb_path)?;
        self.hle.set_native_activity(activity);

        Ok(NativeActivityHarness {
            activity,
            callbacks,
            java_vm,
            jni_env,
            activity_class,
            asset_manager,
            configuration,
            looper,
            input_queue,
            window,
            internal_data_path,
            external_data_path,
            obb_path,
        })
    }

    pub fn prepare_android_app(
        &mut self,
        harness: NativeActivityHarness,
    ) -> Result<u32, NativeRuntimeError> {
        let app = self.alloc_guest_zeroed(0x94, 4)?;
        self.populate_android_app(app, harness)?;
        Ok(app)
    }

    pub fn populate_android_app(
        &mut self,
        app: u32,
        harness: NativeActivityHarness,
    ) -> Result<(), NativeRuntimeError> {
        self.store_runtime32(app.wrapping_add(0x0c), harness.activity)?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_CONFIG_OFFSET),
            harness.configuration,
        )?;
        self.store_runtime32(app.wrapping_add(ANDROID_APP_LOOPER_OFFSET), harness.looper)?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_PENDING_INPUT_QUEUE_OFFSET),
            harness.input_queue,
        )?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_PENDING_WINDOW_OFFSET),
            harness.window,
        )?;
        self.store_runtime32(app.wrapping_add(ANDROID_APP_WINDOW_OFFSET), 0)?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_INPUT_QUEUE_OFFSET),
            harness.input_queue,
        )?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_INPUT_POLL_SOURCE_OFFSET),
            ANDROID_POLL_SOURCE_INPUT,
        )?;
        self.store_runtime32(
            app.wrapping_add(ANDROID_APP_INPUT_POLL_SOURCE_OFFSET + 0x04),
            app,
        )?;
        self.hle
            .set_input_poll_source(app.wrapping_add(ANDROID_APP_INPUT_POLL_SOURCE_OFFSET));
        self.store_runtime32(app.wrapping_add(ANDROID_APP_ACTIVITY_STATE_OFFSET), 0)?;
        self.store_runtime32(app.wrapping_add(ANDROID_APP_DESTROY_REQUESTED_OFFSET), 0)?;
        self.store_runtime32(app.wrapping_add(ANDROID_APP_RUNNING_OFFSET), 1)?;
        self.queue_android_lifecycle_events(app)?;
        Ok(())
    }

    fn queue_android_lifecycle_events(&mut self, app: u32) -> Result<(), NativeRuntimeError> {
        let process = self.write_guest_words(&[
            0xe92d_4030, // push {r4, r5, lr}
            0xe1a0_4000, // mov r4, r0
            0xe591_500c, // ldr r5, [r1, #12]
            0xe355_0001, // cmp r5, #APP_CMD_INIT_WINDOW
            0x0594_3080, // ldreq r3, [r4, #pendingWindow]
            0x0584_3024, // streq r3, [r4, #window]
            0xe355_000a, // cmp r5, #APP_CMD_START
            0x0584_5038, // streq r5, [r4, #activityState]
            0xe355_000b, // cmp r5, #APP_CMD_RESUME
            0x0584_5038, // streq r5, [r4, #activityState]
            0xe355_000d, // cmp r5, #APP_CMD_PAUSE
            0x0584_5038, // streq r5, [r4, #activityState]
            0xe355_000e, // cmp r5, #APP_CMD_STOP
            0x0584_5038, // streq r5, [r4, #activityState]
            0xe355_000f, // cmp r5, #APP_CMD_DESTROY
            0x03a0_3001, // moveq r3, #1
            0x0584_303c, // streq r3, [r4, #destroyRequested]
            0xe594_3004, // ldr r3, [r4, #onAppCmd]
            0xe353_0000, // cmp r3, #0
            0x0a00_0002, // beq after_on_app_cmd
            0xe1a0_0004, // mov r0, r4
            0xe1a0_1005, // mov r1, r5
            0xe12f_ff33, // blx r3
            0xe355_0002, // cmp r5, #APP_CMD_TERM_WINDOW
            0x03a0_3000, // moveq r3, #0
            0x0584_3024, // streq r3, [r4, #window]
            0xe8bd_8030, // pop {r4, r5, pc}
        ])?;
        for command in [
            APP_CMD_START,
            APP_CMD_RESUME,
            APP_CMD_INIT_WINDOW,
            APP_CMD_GAINED_FOCUS,
        ] {
            let source = self.alloc_guest_zeroed(0x10, 4)?;
            self.store_runtime32(source, ANDROID_POLL_SOURCE_MAIN)?;
            self.store_runtime32(source.wrapping_add(0x04), app)?;
            self.store_runtime32(source.wrapping_add(0x08), process)?;
            self.store_runtime32(source.wrapping_add(0x0c), command)?;
            self.hle.queue_alooper_event(source);
        }
        Ok(())
    }

    fn prepare_jni_tables(&mut self) -> Result<(u32, u32), NativeRuntimeError> {
        let return_zero = self.write_guest_words(&[0xe3a0_0000, 0xe12f_ff1e])?;
        let find_class = self.write_runtime_trap("JNI FindClass", JniFunction::FindClass)?;
        let ref_identity = self.write_runtime_trap("JNI ref identity", JniFunction::RefIdentity)?;
        let get_object_class =
            self.write_runtime_trap("JNI GetObjectClass", JniFunction::GetObjectClass)?;
        let get_method_id = self.write_runtime_trap("JNI GetMethodID", JniFunction::GetMethodId)?;
        let get_static_method_id =
            self.write_runtime_trap("JNI GetStaticMethodID", JniFunction::GetStaticMethodId)?;
        let get_field_id = self.write_runtime_trap("JNI GetFieldID", JniFunction::GetFieldId)?;
        let call_object_method =
            self.write_runtime_trap("JNI CallObjectMethod", JniFunction::CallObjectMethod)?;
        let call_static_object_method = self.write_runtime_trap(
            "JNI CallStaticObjectMethod",
            JniFunction::CallStaticObjectMethod,
        )?;
        let call_int_method =
            self.write_runtime_trap("JNI CallIntMethod", JniFunction::CallIntMethod)?;
        let call_static_int_method =
            self.write_runtime_trap("JNI CallStaticIntMethod", JniFunction::CallStaticIntMethod)?;
        let call_boolean_method =
            self.write_runtime_trap("JNI CallBooleanMethod", JniFunction::CallBooleanMethod)?;
        let call_static_boolean_method = self.write_runtime_trap(
            "JNI CallStaticBooleanMethod",
            JniFunction::CallStaticBooleanMethod,
        )?;
        let call_void_method =
            self.write_runtime_trap("JNI CallVoidMethod", JniFunction::CallVoidMethod)?;
        let new_string_utf =
            self.write_runtime_trap("JNI NewStringUTF", JniFunction::NewStringUtf)?;
        let get_string_utf_length =
            self.write_runtime_trap("JNI GetStringUTFLength", JniFunction::GetStringUtfLength)?;
        let get_string_utf_chars =
            self.write_runtime_trap("JNI GetStringUTFChars", JniFunction::GetStringUtfChars)?;
        let release_string_utf_chars = self.write_runtime_trap(
            "JNI ReleaseStringUTFChars",
            JniFunction::ReleaseStringUtfChars,
        )?;
        let get_array_length =
            self.write_runtime_trap("JNI GetArrayLength", JniFunction::GetArrayLength)?;
        let get_int_array_elements =
            self.write_runtime_trap("JNI GetIntArrayElements", JniFunction::GetIntArrayElements)?;
        let release_int_array_elements = self.write_runtime_trap(
            "JNI ReleaseIntArrayElements",
            JniFunction::ReleaseIntArrayElements,
        )?;
        let get_java_vm = self.write_runtime_trap("JNI GetJavaVM", JniFunction::GetJavaVm)?;

        let env_vtable = self.alloc_guest_zeroed(0x400, 4)?;
        for offset in (0..0x400).step_by(4) {
            self.store_runtime32(env_vtable.wrapping_add(offset), return_zero)?;
        }
        let jni_env = self.alloc_guest_zeroed(4, 4)?;
        self.store_runtime32(jni_env, env_vtable)?;

        let vm_vtable = self.alloc_guest_zeroed(0x80, 4)?;
        for offset in (0..0x80).step_by(4) {
            self.store_runtime32(vm_vtable.wrapping_add(offset), return_zero)?;
        }
        let java_vm = self.alloc_guest_zeroed(4, 4)?;
        self.store_runtime32(java_vm, vm_vtable)?;
        self.jni_java_vm = java_vm;

        let store_env =
            self.write_guest_words(&[0xe59f_3008, 0xe581_3000, 0xe3a0_0000, 0xe12f_ff1e, jni_env])?;

        self.store_runtime32(env_vtable.wrapping_add(0x18), find_class)?; // FindClass
        self.store_runtime32(env_vtable.wrapping_add(0x54), ref_identity)?; // NewGlobalRef
        self.store_runtime32(env_vtable.wrapping_add(0x58), return_zero)?; // DeleteGlobalRef
        self.store_runtime32(env_vtable.wrapping_add(0x5c), return_zero)?; // DeleteLocalRef
        self.store_runtime32(env_vtable.wrapping_add(0x64), ref_identity)?; // NewLocalRef
        self.store_runtime32(env_vtable.wrapping_add(0x68), return_zero)?; // EnsureLocalCapacity
        self.store_runtime32(env_vtable.wrapping_add(0x7c), get_object_class)?; // GetObjectClass
        self.store_runtime32(env_vtable.wrapping_add(0x84), get_method_id)?; // GetMethodID
        self.store_runtime32(env_vtable.wrapping_add(0x88), call_object_method)?; // CallObjectMethod
        self.store_runtime32(env_vtable.wrapping_add(0x8c), call_object_method)?; // CallObjectMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x90), call_object_method)?; // CallObjectMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x94), call_boolean_method)?; // CallBooleanMethod
        self.store_runtime32(env_vtable.wrapping_add(0x98), call_boolean_method)?; // CallBooleanMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x9c), call_boolean_method)?; // CallBooleanMethodA
        self.store_runtime32(env_vtable.wrapping_add(0xc4), call_int_method)?; // CallIntMethod
        self.store_runtime32(env_vtable.wrapping_add(0xc8), call_int_method)?; // CallIntMethodV
        self.store_runtime32(env_vtable.wrapping_add(0xcc), call_int_method)?; // CallIntMethodA
        self.store_runtime32(env_vtable.wrapping_add(0xf4), call_void_method)?; // CallVoidMethod
        self.store_runtime32(env_vtable.wrapping_add(0xf8), call_void_method)?; // CallVoidMethodV
        self.store_runtime32(env_vtable.wrapping_add(0xfc), call_void_method)?; // CallVoidMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x178), get_field_id)?; // GetFieldID
        self.store_runtime32(env_vtable.wrapping_add(0x1c4), get_static_method_id)?; // GetStaticMethodID
        self.store_runtime32(env_vtable.wrapping_add(0x1c8), call_static_object_method)?; // CallStaticObjectMethod
        self.store_runtime32(env_vtable.wrapping_add(0x1cc), call_static_object_method)?; // CallStaticObjectMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x1d0), call_static_object_method)?; // CallStaticObjectMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x1d4), call_static_boolean_method)?; // CallStaticBooleanMethod
        self.store_runtime32(env_vtable.wrapping_add(0x1d8), call_static_boolean_method)?; // CallStaticBooleanMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x1dc), call_static_boolean_method)?; // CallStaticBooleanMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x204), call_static_int_method)?; // CallStaticIntMethod
        self.store_runtime32(env_vtable.wrapping_add(0x208), call_static_int_method)?; // CallStaticIntMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x20c), call_static_int_method)?; // CallStaticIntMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x234), call_void_method)?; // CallStaticVoidMethod
        self.store_runtime32(env_vtable.wrapping_add(0x238), call_void_method)?; // CallStaticVoidMethodV
        self.store_runtime32(env_vtable.wrapping_add(0x23c), call_void_method)?; // CallStaticVoidMethodA
        self.store_runtime32(env_vtable.wrapping_add(0x240), get_field_id)?; // GetStaticFieldID
        self.store_runtime32(env_vtable.wrapping_add(0x29c), new_string_utf)?; // NewStringUTF
        self.store_runtime32(env_vtable.wrapping_add(0x2a0), get_string_utf_length)?; // GetStringUTFLength
        self.store_runtime32(env_vtable.wrapping_add(0x2a4), get_string_utf_chars)?; // GetStringUTFChars
        self.store_runtime32(env_vtable.wrapping_add(0x2a8), release_string_utf_chars)?; // ReleaseStringUTFChars
        self.store_runtime32(env_vtable.wrapping_add(0x2ac), get_array_length)?; // GetArrayLength
        self.store_runtime32(env_vtable.wrapping_add(0x2ec), get_int_array_elements)?; // GetIntArrayElements
        self.store_runtime32(env_vtable.wrapping_add(0x30c), release_int_array_elements)?; // ReleaseIntArrayElements
        self.store_runtime32(env_vtable.wrapping_add(0x36c), get_java_vm)?; // GetJavaVM

        self.store_runtime32(vm_vtable.wrapping_add(0x10), store_env)?; // AttachCurrentThread
        self.store_runtime32(vm_vtable.wrapping_add(0x18), store_env)?; // GetEnv
        self.store_runtime32(vm_vtable.wrapping_add(0x1c), store_env)?; // AttachCurrentThreadAsDaemon
        Ok((java_vm, jni_env))
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
        self.run_function_with_args(address, &[], max_steps)
    }

    pub fn run_function_with_args(
        &mut self,
        address: u32,
        args: &[u32],
        max_steps: usize,
    ) -> Result<(), NativeRuntimeError> {
        match self.run_function_with_args_until_hle(address, args, max_steps, None)? {
            NativeRuntimeFunctionExit::Returned => Ok(()),
            NativeRuntimeFunctionExit::HleCall { .. } => Ok(()),
        }
    }

    pub fn run_function_with_args_until_hle(
        &mut self,
        address: u32,
        args: &[u32],
        max_steps: usize,
        stop_hle_name: Option<&str>,
    ) -> Result<NativeRuntimeFunctionExit, NativeRuntimeError> {
        for (idx, &value) in args.iter().take(4).enumerate() {
            self.cpu.set_reg(idx, value);
        }
        self.cpu.branch_exchange(address);
        self.cpu.set_reg(14, CALL_RETURN_SENTINEL);
        self.continue_function_until_hle(address, max_steps, stop_hle_name)
    }

    pub fn continue_until_hle(
        &mut self,
        max_steps: usize,
        stop_hle_name: Option<&str>,
    ) -> Result<NativeRuntimeFunctionExit, NativeRuntimeError> {
        self.continue_function_until_hle(self.cpu.pc(), max_steps, stop_hle_name)
    }

    fn continue_function_until_hle(
        &mut self,
        address: u32,
        max_steps: usize,
        stop_hle_name: Option<&str>,
    ) -> Result<NativeRuntimeFunctionExit, NativeRuntimeError> {
        let mut tail = VecDeque::with_capacity(RUN_FUNCTION_TRACE_LEN);
        let trace_ranges = parse_trace_pc_ranges();
        let trace_mem32 = parse_trace_mem32_specs();
        let trace_mem32_deref = parse_trace_mem32_deref_specs();
        let trace_cxx_strings = parse_trace_cxx_string_specs();
        let trace_native_events = parse_trace_native_event_specs();
        let trace_native_event_mem32 = parse_trace_native_event_mem32_specs();
        let trace_native_event_deref32 = parse_trace_native_event_deref32_specs();
        let trace_native_event_cxx_strings = parse_trace_native_event_cxx_string_specs();
        let trace_native_event_bytes = parse_trace_native_event_bytes_specs();
        let trace_native_events_path = trace_native_events_path();
        let trace_step_interval = parse_trace_step_interval();
        let trace_hle = parse_trace_hle_filter();
        let trace_pc_limit = parse_trace_limit("AEMU_TRACE_PC_LIMIT");
        let trace_mem32_limit = parse_trace_limit("AEMU_TRACE_MEM32_LIMIT");
        let trace_mem32_deref_limit = parse_trace_limit("AEMU_TRACE_MEM32_DEREF_LIMIT");
        let trace_cxx_string_limit = parse_trace_limit("AEMU_TRACE_CXX_STRING_LIMIT");
        let trace_native_event_limit = parse_trace_limit("AEMU_TRACE_NATIVE_EVENTS_LIMIT");
        let trace_hle_limit = parse_trace_limit("AEMU_TRACE_HLE_LIMIT");
        let mut trace_pc_count = 0usize;
        let mut trace_mem32_count = 0usize;
        let mut trace_mem32_deref_count = 0usize;
        let mut trace_cxx_string_count = 0usize;
        let mut trace_hle_count = 0usize;
        for step_idx in 0..max_steps {
            if self.cpu.pc() == CALL_RETURN_SENTINEL {
                return Ok(NativeRuntimeFunctionExit::Returned);
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
            if !trace_ranges.is_empty() {
                let pc = self.cpu.pc();
                if trace_ranges
                    .iter()
                    .any(|&(start, end)| pc >= start && pc < end)
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
            if !trace_mem32.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_mem32.iter().filter(|spec| spec.pc == pc) {
                    if trace_mem32_limit.is_some_and(|limit| trace_mem32_count >= limit) {
                        break;
                    }
                    trace_mem32_count += 1;
                    self.trace_mem32(step_idx, spec);
                }
            }
            if !trace_mem32_deref.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_mem32_deref.iter().filter(|spec| spec.pc == pc) {
                    if trace_mem32_deref_limit.is_some_and(|limit| trace_mem32_deref_count >= limit)
                    {
                        break;
                    }
                    trace_mem32_deref_count += 1;
                    self.trace_mem32_deref(step_idx, spec);
                }
            }
            if !trace_cxx_strings.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_cxx_strings.iter().filter(|spec| spec.pc == pc) {
                    if trace_cxx_string_limit.is_some_and(|limit| trace_cxx_string_count >= limit) {
                        break;
                    }
                    trace_cxx_string_count += 1;
                    self.trace_cxx_string(step_idx, spec);
                }
            }
            if let Some(path) = trace_native_events_path.as_deref() {
                let pc = self.cpu.pc();
                for spec in trace_native_events.iter().filter(|spec| spec.pc == pc) {
                    if trace_native_event_limit
                        .is_some_and(|limit| self.trace_native_event_count >= limit)
                    {
                        break;
                    }
                    self.trace_native_event_count += 1;
                    self.trace_native_event_jsonl(
                        step_idx,
                        self.hle.current_pthread(),
                        spec,
                        path,
                        &trace_native_event_mem32,
                        &trace_native_event_deref32,
                        &trace_native_event_cxx_strings,
                        &trace_native_event_bytes,
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
                    args,
                }) => {
                    self.handle_thread_sync_hle(1, &name, args)?;
                    self.wait_main_after_hle(&name, args)?;
                    let should_service_threads = self.hle.has_created_pthreads()
                        || (!self.guest_threads.is_empty()
                            && GUEST_THREAD_SERVICE_HLE.contains(&name.as_str()));
                    if trace_hle
                        .as_ref()
                        .is_some_and(|filter| filter.matches(&name))
                        && trace_hle_limit.map_or(true, |limit| trace_hle_count < limit)
                    {
                        trace_hle_count += 1;
                        eprintln!(
                            "HLE function={:#010x} step={} pc={:#010x} name={} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} ret0={:#010x} ret1={:#010x} ret2={:#010x} ret3={:#010x}",
                            address,
                            step_idx,
                            hle_address,
                            name,
                            args[0],
                            args[1],
                            args[2],
                            args[3],
                            self.cpu.reg(0),
                            self.cpu.reg(1),
                            self.cpu.reg(2),
                            self.cpu.reg(3),
                        );
                    }
                    if should_service_threads {
                        self.service_guest_threads(GUEST_THREAD_SLICE_STEPS)?;
                    }
                    if stop_hle_name.is_some_and(|stop| stop == name) {
                        self.service_guest_threads_at_stop_hle(&name)?;
                        return Ok(NativeRuntimeFunctionExit::HleCall {
                            name,
                            address: hle_address,
                            args,
                            step: step_idx,
                        });
                    }
                }
                Err(source) => {
                    return Err(NativeRuntimeError::Traced {
                        source: Box::new(source),
                        tail: tail.into_iter().collect(),
                    });
                }
            }
            if !self.guest_threads.is_empty()
                && step_idx != 0
                && step_idx % GUEST_THREAD_SERVICE_INTERVAL == 0
            {
                self.service_guest_threads(GUEST_THREAD_SLICE_STEPS)?;
            }
        }
        Err(NativeRuntimeError::Traced {
            source: Box::new(NativeRuntimeError::Cpu(Trap::StepLimit)),
            tail: tail.into_iter().collect(),
        })
    }

    fn service_guest_threads(&mut self, slice_steps: usize) -> Result<(), NativeRuntimeError> {
        self.drain_created_pthreads()?;
        let runnable = self.guest_threads.len();
        for _ in 0..runnable {
            let Some(mut thread) = self.guest_threads.pop_front() else {
                break;
            };
            if !thread.wait.is_runnable() {
                self.guest_threads.push_back(thread);
                continue;
            }
            let done = self.run_guest_thread_slice(&mut thread, slice_steps)?;
            self.drain_created_pthreads()?;
            if !done {
                self.guest_threads.push_back(thread);
            }
        }
        Ok(())
    }

    fn service_guest_threads_at_stop_hle(&mut self, name: &str) -> Result<(), NativeRuntimeError> {
        let slices = match name {
            "eglSwapBuffers" => parse_usize_env("AEMU_GUEST_THREAD_SWAP_SLICES")
                .unwrap_or(GUEST_THREAD_SWAP_SERVICE_SLICES),
            _ => 0,
        };
        for _ in 0..slices {
            if !self.hle.has_created_pthreads()
                && self
                    .guest_threads
                    .iter()
                    .all(|thread| !thread.wait.is_runnable())
            {
                break;
            }
            self.service_guest_threads(GUEST_THREAD_SLICE_STEPS)?;
        }
        Ok(())
    }

    fn drain_created_pthreads(&mut self) -> Result<(), NativeRuntimeError> {
        for created in self.hle.take_created_pthreads() {
            if !self.should_run_created_pthread(created) {
                if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                    eprintln!(
                        "THREAD skip id={} start={:#010x} arg={:#010x} library={}",
                        created.id,
                        created.start,
                        created.arg,
                        self.library_name_for_address(created.start & !1)
                            .unwrap_or("<unknown>"),
                    );
                }
                continue;
            }
            let thread = self.create_guest_thread(created)?;
            if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                let entry = self.pthread_callable_entry(created.arg).unwrap_or(0);
                eprintln!(
                    "THREAD create id={} start={:#010x} arg={:#010x} entry={:#010x} entry_lib={} sp={:#010x}",
                    thread.id,
                    thread.cpu.pc() | u32::from(thread.cpu.isa() == Isa::Thumb),
                    thread.cpu.reg(0),
                    entry,
                    self.library_name_for_address(entry & !1)
                        .unwrap_or("<unknown>"),
                    thread.cpu.reg(13),
                );
            }
            self.guest_threads.push_back(thread);
        }
        Ok(())
    }

    fn pthread_callable_entry(&mut self, arg: u32) -> Option<u32> {
        let storage = self.link.memory.load32(arg).ok()?;
        let vtable = self.link.memory.load32(storage).ok()?;
        self.link.memory.load32(vtable.wrapping_add(8)).ok()
    }

    fn should_run_created_pthread(&self, created: CreatedPthread) -> bool {
        if std::env::var_os("AEMU_RUN_ALL_PTHREADS").is_some() {
            return true;
        }
        self.library_name_for_address(created.start & !1)
            .map_or(true, |library| library == "libgnustl_shared.so")
    }

    fn library_name_for_address(&self, address: u32) -> Option<&str> {
        self.link.objects.iter().find_map(|object| {
            let end = object.memory_base.checked_add(object.memory_size)?;
            (address >= object.memory_base && address < end).then_some(object.library_name.as_str())
        })
    }

    fn create_guest_thread(
        &mut self,
        created: CreatedPthread,
    ) -> Result<GuestThread, NativeRuntimeError> {
        let stack_top = self.allocate_guest_thread_stack()?;
        let mut cpu = Cpu::new();
        cpu.cp15_tpidrurw = self.cpu.cp15_tpidrurw;
        cpu.cp15_tpidruro = self.cpu.cp15_tpidruro;
        cpu.fpscr = self.cpu.fpscr;
        cpu.set_reg(0, created.arg);
        cpu.set_reg(13, stack_top);
        cpu.set_reg(14, THREAD_RETURN_SENTINEL);
        cpu.branch_exchange(created.start);
        Ok(GuestThread {
            id: created.id,
            cpu,
            wait: GuestThreadWait::Runnable,
            trace_tail: VecDeque::with_capacity(GUEST_THREAD_TRACE_LEN),
            trace_pc_count: 0,
            trace_mem32_count: 0,
            trace_mem32_deref_count: 0,
            trace_cxx_string_count: 0,
        })
    }

    fn allocate_guest_thread_stack(&mut self) -> Result<u32, NativeRuntimeError> {
        let stack_end = self
            .stack_base
            .checked_add(self.stack_size)
            .ok_or(NativeRuntimeError::AddressOverflow)?;
        let next_top = self
            .next_thread_stack_top
            .checked_add(GUEST_THREAD_STACK_SIZE)
            .ok_or(NativeRuntimeError::AddressOverflow)?;
        let main_guard = STACK_ENTRY_HEADROOM_MAX.saturating_add(GUEST_THREAD_STACK_SIZE / 4);
        if next_top > stack_end.saturating_sub(main_guard) {
            return Err(NativeRuntimeError::Memory(format!(
                "guest thread stack exhausted in {:#010x}+{:#x}",
                self.stack_base, self.stack_size
            )));
        }
        self.next_thread_stack_top = next_top;
        Ok(next_top & !(GUEST_THREAD_STACK_ALIGN - 1))
    }

    fn handle_thread_sync_hle(
        &mut self,
        thread_id: u32,
        name: &str,
        args: [u32; 4],
    ) -> Result<(), NativeRuntimeError> {
        match name {
            "pthread_cond_signal" => self.wake_cond_threads(args[0], false),
            "pthread_cond_broadcast" => self.wake_cond_threads(args[0], true),
            "pthread_mutex_unlock" => self.unlock_mutex_for_thread(thread_id, args[0]),
            "pthread_mutex_trylock" if args[0] != 0 => {
                let result = if self.lock_mutex_for_thread(thread_id, args[0]) {
                    0
                } else {
                    16
                };
                self.cpu.set_reg(0, result);
            }
            "pthread_once" => self.run_pthread_once(thread_id, args[0], args[1])?,
            _ => {}
        }
        Ok(())
    }

    fn thread_wait_after_hle(
        &mut self,
        thread_id: u32,
        name: &str,
        args: [u32; 4],
    ) -> Option<GuestThreadWait> {
        match name {
            "pthread_mutex_lock" if args[0] != 0 => (!self
                .lock_mutex_for_thread(thread_id, args[0]))
            .then_some(GuestThreadWait::Mutex { mutex: args[0] }),
            "pthread_cond_wait" | "pthread_cond_timedwait" if args[0] != 0 => {
                self.unlock_mutex_for_thread(thread_id, args[1]);
                Some(GuestThreadWait::Condvar {
                    cond: args[0],
                    mutex: args[1],
                })
            }
            _ => None,
        }
    }

    fn wait_main_after_hle(
        &mut self,
        name: &str,
        args: [u32; 4],
    ) -> Result<(), NativeRuntimeError> {
        let Some(wait) = self.thread_wait_after_hle(1, name, args) else {
            return Ok(());
        };
        self.main_wait = wait;
        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
            eprintln!("THREAD wait id=1 {:?}", self.main_wait);
        }
        for _ in 0..MAIN_THREAD_WAIT_SPINS {
            if self.main_wait.is_runnable() {
                return Ok(());
            }
            self.try_wake_main_mutex();
            if self.main_wait.is_runnable() {
                return Ok(());
            }
            self.service_guest_threads(GUEST_THREAD_SLICE_STEPS)?;
        }
        Err(NativeRuntimeError::Memory(format!(
            "main guest thread stalled waiting for {:?}",
            self.main_wait
        )))
    }

    fn wake_cond_threads(&mut self, cond: u32, broadcast: bool) {
        if cond == 0 {
            return;
        }
        if let GuestThreadWait::Condvar {
            cond: main_cond,
            mutex,
        } = self.main_wait
        {
            if main_cond == cond {
                self.main_wait = if mutex == 0 || self.lock_mutex_for_thread(1, mutex) {
                    GuestThreadWait::Runnable
                } else {
                    GuestThreadWait::Mutex { mutex }
                };
                if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                    eprintln!(
                        "THREAD wake id=1 cond={cond:#010x} mutex={mutex:#010x} wait={:?}",
                        self.main_wait
                    );
                }
                if !broadcast {
                    return;
                }
            }
        }
        for idx in 0..self.guest_threads.len() {
            let wait = self.guest_threads[idx].wait;
            if let GuestThreadWait::Condvar {
                cond: thread_cond,
                mutex,
            } = wait
            {
                if thread_cond != cond {
                    continue;
                }
                let thread_id = self.guest_threads[idx].id;
                self.guest_threads[idx].wait =
                    if mutex == 0 || self.lock_mutex_for_thread(thread_id, mutex) {
                        GuestThreadWait::Runnable
                    } else {
                        GuestThreadWait::Mutex { mutex }
                    };
                if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                    eprintln!(
                        "THREAD wake id={} cond={cond:#010x} mutex={mutex:#010x} wait={:?}",
                        self.guest_threads[idx].id, self.guest_threads[idx].wait
                    );
                }
                if !broadcast {
                    break;
                }
            }
        }
    }

    fn mutex_index(&mut self, mutex: u32) -> usize {
        if let Some(idx) = self
            .guest_mutexes
            .iter()
            .position(|guest_mutex| guest_mutex.addr == mutex)
        {
            return idx;
        }
        self.guest_mutexes.push(GuestMutex {
            addr: mutex,
            owner: None,
            recursion: 0,
        });
        self.guest_mutexes.len() - 1
    }

    fn lock_mutex_for_thread(&mut self, thread_id: u32, mutex: u32) -> bool {
        if mutex == 0 {
            return true;
        }
        let idx = self.mutex_index(mutex);
        let guest_mutex = &mut self.guest_mutexes[idx];
        match guest_mutex.owner {
            None => {
                guest_mutex.owner = Some(thread_id);
                guest_mutex.recursion = 1;
                true
            }
            Some(owner) if owner == thread_id => {
                guest_mutex.recursion = guest_mutex.recursion.saturating_add(1).max(1);
                true
            }
            Some(_) => false,
        }
    }

    fn unlock_mutex_for_thread(&mut self, thread_id: u32, mutex: u32) {
        if mutex == 0 {
            return;
        }
        let Some(idx) = self
            .guest_mutexes
            .iter()
            .position(|guest_mutex| guest_mutex.addr == mutex)
        else {
            return;
        };
        let guest_mutex = &mut self.guest_mutexes[idx];
        if guest_mutex.owner != Some(thread_id) {
            return;
        }
        if guest_mutex.recursion > 1 {
            guest_mutex.recursion -= 1;
            return;
        }
        guest_mutex.owner = None;
        guest_mutex.recursion = 0;
        self.try_wake_main_mutex();
        self.wake_mutex_thread(mutex);
    }

    fn try_wake_main_mutex(&mut self) {
        let GuestThreadWait::Mutex { mutex } = self.main_wait else {
            return;
        };
        if self.lock_mutex_for_thread(1, mutex) {
            self.main_wait = GuestThreadWait::Runnable;
            if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                eprintln!("THREAD wake id=1 mutex={mutex:#010x}");
            }
        }
    }

    fn wake_mutex_thread(&mut self, mutex: u32) {
        let Some(idx) = self
            .guest_threads
            .iter()
            .position(|thread| thread.wait == (GuestThreadWait::Mutex { mutex }))
        else {
            return;
        };
        let thread_id = self.guest_threads[idx].id;
        if self.lock_mutex_for_thread(thread_id, mutex) {
            self.guest_threads[idx].wait = GuestThreadWait::Runnable;
            if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                eprintln!("THREAD wake id={thread_id} mutex={mutex:#010x}");
            }
        }
    }

    fn run_pthread_once(
        &mut self,
        thread_id: u32,
        once_control: u32,
        init_routine: u32,
    ) -> Result<(), NativeRuntimeError> {
        if once_control == 0 || init_routine == 0 {
            self.cpu.set_reg(0, 22);
            return Ok(());
        }
        let state = self
            .link
            .memory
            .load32(once_control)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        if state != 0 {
            self.cpu.set_reg(0, 0);
            return Ok(());
        }

        self.link
            .memory
            .store32(once_control, 1)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        let continuation = self.cpu.clone();
        self.cpu.branch_exchange(init_routine);
        self.cpu.set_reg(14, CALL_RETURN_SENTINEL);

        let max_steps =
            parse_trace_limit("AEMU_PTHREAD_ONCE_INIT_STEPS").unwrap_or(PTHREAD_ONCE_INIT_STEPS);
        let trace = std::env::var_os("AEMU_TRACE_PTHREAD_ONCE").is_some();
        let trace_interval = parse_trace_limit("AEMU_TRACE_PTHREAD_ONCE_STEPS").unwrap_or(1);
        let trace_limit = parse_trace_limit("AEMU_TRACE_PTHREAD_ONCE_LIMIT");
        let mut trace_count = 0usize;
        if trace {
            eprintln!(
                "PTHREAD_ONCE start thread={thread_id} once={once_control:#010x} init={init_routine:#010x} state={state:#010x} max_steps={max_steps}"
            );
        }

        for step_idx in 0..max_steps {
            if self.cpu.pc() == CALL_RETURN_SENTINEL {
                if trace {
                    eprintln!(
                        "PTHREAD_ONCE return thread={thread_id} once={once_control:#010x} init={init_routine:#010x} steps={step_idx}"
                    );
                }
                self.cpu = continuation;
                self.cpu.set_reg(0, 0);
                return Ok(());
            }
            if trace
                && step_idx % trace_interval == 0
                && trace_limit.map_or(true, |limit| trace_count < limit)
            {
                trace_count += 1;
                let pc = self.cpu.pc();
                eprintln!(
                    "PTHREAD_ONCE step thread={thread_id} step={step_idx}/{max_steps} {:?} pc={pc:#010x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} sp={:#010x} lr={:#010x}",
                    self.cpu.isa(),
                    self.cpu.reg(0),
                    self.cpu.reg(1),
                    self.cpu.reg(2),
                    self.cpu.reg(3),
                    self.cpu.reg(13),
                    self.cpu.reg(14),
                );
            }
            match self.step()? {
                NativeRuntimeStep::GuestInstruction => {}
                NativeRuntimeStep::HleCall {
                    name,
                    address,
                    args,
                } => {
                    if trace && trace_limit.map_or(true, |limit| trace_count < limit) {
                        trace_count += 1;
                        eprintln!(
                            "PTHREAD_ONCE hle thread={thread_id} step={step_idx}/{max_steps} pc={address:#010x} name={name} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} ret0={:#010x}",
                            args[0],
                            args[1],
                            args[2],
                            args[3],
                            self.cpu.reg(0),
                        );
                    }
                    self.handle_thread_sync_hle(thread_id, &name, args)?;
                    if thread_id == 1 {
                        self.wait_main_after_hle(&name, args)?;
                    } else if let Some(wait) = self.thread_wait_after_hle(thread_id, &name, args) {
                        return Err(NativeRuntimeError::Memory(format!(
                            "pthread_once init routine for thread {thread_id} blocked on {wait:?}"
                        )));
                    }
                }
            }
        }

        Err(NativeRuntimeError::Memory(format!(
            "pthread_once init routine at {init_routine:#010x} exceeded step limit {max_steps}"
        )))
    }

    fn run_guest_thread_slice(
        &mut self,
        thread: &mut GuestThread,
        max_steps: usize,
    ) -> Result<bool, NativeRuntimeError> {
        let trace_ranges = parse_trace_pc_ranges();
        let trace_mem32 = parse_trace_mem32_specs();
        let trace_mem32_deref = parse_trace_mem32_deref_specs();
        let trace_cxx_strings = parse_trace_cxx_string_specs();
        let trace_native_events = parse_trace_native_event_specs();
        let trace_native_event_mem32 = parse_trace_native_event_mem32_specs();
        let trace_native_event_deref32 = parse_trace_native_event_deref32_specs();
        let trace_native_event_cxx_strings = parse_trace_native_event_cxx_string_specs();
        let trace_native_event_bytes = parse_trace_native_event_bytes_specs();
        let trace_native_events_path = trace_native_events_path();
        let trace_pc_limit = parse_trace_limit("AEMU_TRACE_PC_LIMIT");
        let trace_mem32_limit = parse_trace_limit("AEMU_TRACE_MEM32_LIMIT");
        let trace_mem32_deref_limit = parse_trace_limit("AEMU_TRACE_MEM32_DEREF_LIMIT");
        let trace_cxx_string_limit = parse_trace_limit("AEMU_TRACE_CXX_STRING_LIMIT");
        let trace_native_event_limit = parse_trace_limit("AEMU_TRACE_NATIVE_EVENTS_LIMIT");
        let main_cpu = std::mem::replace(&mut self.cpu, thread.cpu.clone());
        let previous_thread = self.hle.current_pthread();
        self.hle.set_current_pthread(thread.id);
        let mut result = Ok(false);
        for step_idx in 0..max_steps {
            if self.cpu.pc() == THREAD_RETURN_SENTINEL {
                result = Ok(true);
                break;
            }
            if !trace_ranges.is_empty() {
                let pc = self.cpu.pc();
                if trace_ranges
                    .iter()
                    .any(|&(start, end)| pc >= start && pc < end)
                    && trace_pc_limit.map_or(true, |limit| thread.trace_pc_count < limit)
                {
                    thread.trace_pc_count += 1;
                    let instr16 = self.link.memory.load16(pc).unwrap_or(0xffff);
                    eprintln!(
                        "THREAD_TRACE id={} step={step_idx}/{max_steps} {:?} pc={pc:#010x} instr16={instr16:#06x} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} r4={:#010x} r5={:#010x} r6={:#010x} r7={:#010x} r8={:#010x} r9={:#010x} r10={:#010x} r11={:#010x} r12={:#010x} sp={:#010x} lr={:#010x}",
                        thread.id,
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
            if !trace_mem32.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_mem32.iter().filter(|spec| spec.pc == pc) {
                    if trace_mem32_limit.is_some_and(|limit| thread.trace_mem32_count >= limit) {
                        break;
                    }
                    thread.trace_mem32_count += 1;
                    eprint!("THREAD_TRACE id={} ", thread.id);
                    self.trace_mem32(step_idx, spec);
                }
            }
            if !trace_mem32_deref.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_mem32_deref.iter().filter(|spec| spec.pc == pc) {
                    if trace_mem32_deref_limit
                        .is_some_and(|limit| thread.trace_mem32_deref_count >= limit)
                    {
                        break;
                    }
                    thread.trace_mem32_deref_count += 1;
                    eprint!("THREAD_TRACE id={} ", thread.id);
                    self.trace_mem32_deref(step_idx, spec);
                }
            }
            if !trace_cxx_strings.is_empty() {
                let pc = self.cpu.pc();
                for spec in trace_cxx_strings.iter().filter(|spec| spec.pc == pc) {
                    if trace_cxx_string_limit
                        .is_some_and(|limit| thread.trace_cxx_string_count >= limit)
                    {
                        break;
                    }
                    thread.trace_cxx_string_count += 1;
                    eprint!("THREAD_TRACE id={} ", thread.id);
                    self.trace_cxx_string(step_idx, spec);
                }
            }
            if let Some(path) = trace_native_events_path.as_deref() {
                let pc = self.cpu.pc();
                for spec in trace_native_events.iter().filter(|spec| spec.pc == pc) {
                    if trace_native_event_limit
                        .is_some_and(|limit| self.trace_native_event_count >= limit)
                    {
                        break;
                    }
                    self.trace_native_event_count += 1;
                    self.trace_native_event_jsonl(
                        step_idx,
                        thread.id,
                        spec,
                        path,
                        &trace_native_event_mem32,
                        &trace_native_event_deref32,
                        &trace_native_event_cxx_strings,
                        &trace_native_event_bytes,
                    );
                }
            }
            push_trace_tail(&mut thread.trace_tail, &self.cpu, GUEST_THREAD_TRACE_LEN);
            match self.step() {
                Ok(NativeRuntimeStep::GuestInstruction) => {}
                Ok(NativeRuntimeStep::HleCall { name, args, .. }) => {
                    self.handle_thread_sync_hle(thread.id, &name, args)?;
                    if let Some(wait) = self.thread_wait_after_hle(thread.id, &name, args) {
                        thread.wait = wait;
                        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                            eprintln!("THREAD wait id={} {wait:?}", thread.id);
                        }
                        break;
                    }
                }
                Err(err) => {
                    if let NativeRuntimeError::Hle {
                        name,
                        source: HleError::Abort(abort_name),
                    } = &err
                    {
                        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                            eprintln!(
                                "THREAD abort id={} name={} abort={} pc={:#010x} {:?} lr={:#010x} r0={:#010x}",
                                thread.id,
                                name,
                                abort_name,
                                self.cpu.pc(),
                                self.cpu.isa(),
                                self.cpu.reg(14),
                                self.cpu.reg(0),
                            );
                            log_guest_thread_tail(thread);
                        }
                        result = Ok(true);
                    } else {
                        result = Err(err);
                    }
                    break;
                }
            }
        }
        if thread.wait.is_runnable()
            && matches!(result, Ok(false))
            && self.cpu.pc() == THREAD_RETURN_SENTINEL
        {
            result = Ok(true);
        }
        thread.cpu = self.cpu.clone();
        self.cpu = main_cpu;
        self.hle.set_current_pthread(previous_thread);
        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
            eprintln!(
                "THREAD slice id={} done={} pc={:#010x} {:?} r0={:#010x}",
                thread.id,
                result.as_ref().is_ok_and(|done| *done),
                thread.cpu.pc(),
                thread.cpu.isa(),
                thread.cpu.reg(0),
            );
        }
        result
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

    pub fn run_constructor_batch(
        &mut self,
        constructors: &[NativeConstructor],
        max_steps: usize,
    ) -> Result<(), NativeRuntimeError> {
        if constructors.is_empty() {
            return Ok(());
        }
        let mut words = Vec::with_capacity(constructors.len() * 4 + 2);
        words.push(0xe92d_4010); // push {r4, lr}
        for constructor in constructors {
            words.push(0xe59f_c004); // ldr ip, [pc, #4]
            words.push(0xe12f_ff3c); // blx ip
            words.push(0xea00_0000); // b after_literal
            words.push(constructor.address);
        }
        words.push(0xe8bd_8010); // pop {r4, pc}
        let trampoline = self.write_guest_words(&words)?;
        self.run_function(trampoline, max_steps)
    }

    fn store_runtime32(&mut self, addr: u32, value: u32) -> Result<(), NativeRuntimeError> {
        self.link
            .memory
            .store32(addr, value)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))
    }

    fn write_guest_words(&mut self, words: &[u32]) -> Result<u32, NativeRuntimeError> {
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for &word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let len = u32::try_from(bytes.len()).map_err(|_| NativeRuntimeError::AddressOverflow)?;
        let ptr = self.alloc_guest_zeroed(len.max(1), 4)?;
        self.link
            .memory
            .load_bytes(ptr, &bytes)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        Ok(ptr)
    }

    fn write_runtime_trap(
        &mut self,
        name: &'static str,
        function: JniFunction,
    ) -> Result<u32, NativeRuntimeError> {
        let address = self.write_guest_words(&[HLE_TRAP_ARM_INSTR])?;
        self.runtime_hle_traps.push(RuntimeHleTrap {
            address,
            isa: Isa::Arm,
            name,
            kind: RuntimeHleTrapKind::Jni(function),
        });
        Ok(address)
    }

    fn runtime_hle_entry(&self, pc: u32, isa: Isa) -> Option<RuntimeHleTrap> {
        self.runtime_hle_traps
            .iter()
            .find(|trap| trap.address == pc && trap.isa == isa)
            .copied()
    }

    fn dispatch_runtime_hle(&mut self, trap: RuntimeHleTrap) -> Result<(), NativeRuntimeError> {
        match trap.kind {
            RuntimeHleTrapKind::Jni(function) => self.dispatch_jni(function),
            RuntimeHleTrapKind::CxxDynamicCast => self.dispatch_cxx_dynamic_cast(),
        }
    }

    fn dispatch_cxx_dynamic_cast(&mut self) -> Result<(), NativeRuntimeError> {
        let src_ptr = self.cpu.reg(0);
        let src_type = self.cpu.reg(1);
        let dst_type = self.cpu.reg(2);
        let src2dst_offset = self.cpu.reg(3) as i32;
        let result = self.cxx_dynamic_cast(src_ptr, src_type, dst_type, src2dst_offset)?;
        if std::env::var_os("AEMU_TRACE_CXXABI").is_some() {
            eprintln!(
                "CXXABI __dynamic_cast src={src_ptr:#010x} src_type={src_type:#010x} dst_type={dst_type:#010x} src2dst={src2dst_offset} -> {result:#010x}"
            );
        }
        self.cpu.set_reg(0, result);
        self.cpu.branch_exchange(self.cpu.reg(14));
        Ok(())
    }

    fn cxx_dynamic_cast(
        &mut self,
        src_ptr: u32,
        src_type: u32,
        dst_type: u32,
        src2dst_offset: i32,
    ) -> Result<u32, NativeRuntimeError> {
        if src_ptr == 0 || src_type == 0 || dst_type == 0 {
            return Ok(0);
        }
        if src_type == dst_type {
            return Ok(src_ptr);
        }

        let object_vptr = self.load_runtime32(src_ptr)?;
        let offset_to_top = self.load_runtime32(object_vptr.wrapping_sub(8))? as i32;
        let dynamic_type = self.load_runtime32(object_vptr.wrapping_sub(4))?;
        let top_ptr = src_ptr.wrapping_add(offset_to_top as u32);
        if dynamic_type == dst_type {
            return Ok(top_ptr);
        }

        let mut visited = Vec::new();
        if let Some(dst_offset) =
            self.cxx_type_base_offset(dynamic_type, dst_type, object_vptr, &mut visited)?
        {
            return Ok(top_ptr.wrapping_add(dst_offset as u32));
        }

        if src2dst_offset >= 0 {
            let mut visited = Vec::new();
            if self
                .cxx_type_base_offset(dst_type, src_type, object_vptr, &mut visited)?
                .is_some_and(|offset| offset == src2dst_offset)
            {
                return Ok(top_ptr);
            }
        }

        Ok(0)
    }

    fn cxx_type_base_offset(
        &mut self,
        type_info: u32,
        target_type: u32,
        object_vptr: u32,
        visited: &mut Vec<u32>,
    ) -> Result<Option<i32>, NativeRuntimeError> {
        if type_info == target_type {
            return Ok(Some(0));
        }
        if type_info == 0 || visited.contains(&type_info) || visited.len() >= 128 {
            return Ok(None);
        }
        visited.push(type_info);
        let result = match self.cxx_type_info_kind(type_info)? {
            Some(CxxTypeInfoKind::Class) | None => Ok(None),
            Some(CxxTypeInfoKind::SingleInheritance) => {
                let base_type = self.load_runtime32(type_info.wrapping_add(8))?;
                self.cxx_type_base_offset(base_type, target_type, object_vptr, visited)
            }
            Some(CxxTypeInfoKind::VirtualMultipleInheritance) => {
                let base_count = self.load_runtime32(type_info.wrapping_add(12))?.min(128);
                for idx in 0..base_count {
                    let entry = type_info.wrapping_add(16).wrapping_add(idx * 8);
                    let base_type = self.load_runtime32(entry)?;
                    let offset_flags = self.load_runtime32(entry.wrapping_add(4))?;
                    let flags = offset_flags & 0xff;
                    let raw_offset = (offset_flags as i32) >> 8;
                    if flags & 0x2 == 0 {
                        continue;
                    }
                    let base_offset = if flags & 0x1 != 0 {
                        let offset_addr = object_vptr.wrapping_add(raw_offset as u32);
                        self.load_runtime32(offset_addr)? as i32
                    } else {
                        raw_offset
                    };
                    if base_type == target_type {
                        return Ok(Some(base_offset));
                    }
                    if let Some(nested_offset) =
                        self.cxx_type_base_offset(base_type, target_type, object_vptr, visited)?
                    {
                        return Ok(Some(base_offset.wrapping_add(nested_offset)));
                    }
                }
                Ok(None)
            }
        };
        visited.pop();
        result
    }

    fn cxx_type_info_kind(
        &mut self,
        type_info: u32,
    ) -> Result<Option<CxxTypeInfoKind>, NativeRuntimeError> {
        let vptr = self.load_runtime32(type_info)?;
        if self.cxx_vtable_matches(vptr, "_ZTVN10__cxxabiv117__class_type_infoE") {
            Ok(Some(CxxTypeInfoKind::Class))
        } else if self.cxx_vtable_matches(vptr, "_ZTVN10__cxxabiv120__si_class_type_infoE") {
            Ok(Some(CxxTypeInfoKind::SingleInheritance))
        } else if self.cxx_vtable_matches(vptr, "_ZTVN10__cxxabiv121__vmi_class_type_infoE") {
            Ok(Some(CxxTypeInfoKind::VirtualMultipleInheritance))
        } else {
            Ok(None)
        }
    }

    fn cxx_vtable_matches(&self, vptr: u32, symbol_name: &str) -> bool {
        self.symbol_address(symbol_name).is_some_and(|addr| {
            vptr == addr || vptr == addr.wrapping_add(8) || vptr == addr.wrapping_add(12)
        })
    }

    fn load_runtime32(&mut self, addr: u32) -> Result<u32, NativeRuntimeError> {
        self.link
            .memory
            .load32(addr)
            .map_err(|err| NativeRuntimeError::Memory(err.to_string()))
    }

    fn dispatch_jni(&mut self, function: JniFunction) -> Result<(), NativeRuntimeError> {
        let value = match function {
            JniFunction::FindClass => self.cpu.reg(1).max(1),
            JniFunction::RefIdentity | JniFunction::GetObjectClass => self.cpu.reg(1).max(1),
            JniFunction::GetMethodId => self.register_jni_method(false)?,
            JniFunction::GetStaticMethodId | JniFunction::GetFieldId => {
                self.register_jni_method(true)?
            }
            JniFunction::CallObjectMethod | JniFunction::CallStaticObjectMethod => {
                self.call_jni_object_method()?
            }
            JniFunction::CallIntMethod | JniFunction::CallStaticIntMethod => {
                self.call_jni_int_method()
            }
            JniFunction::CallBooleanMethod | JniFunction::CallStaticBooleanMethod => {
                self.call_jni_boolean_method()
            }
            JniFunction::CallVoidMethod | JniFunction::ReleaseStringUtfChars => 0,
            JniFunction::NewStringUtf => self.cpu.reg(1),
            JniFunction::GetStringUtfLength => {
                self.load_guest_c_string(self.cpu.reg(1), 4096)?.len() as u32
            }
            JniFunction::GetStringUtfChars => {
                if self.cpu.reg(2) != 0 {
                    self.link
                        .memory
                        .store8(self.cpu.reg(2), 0)
                        .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
                }
                self.cpu.reg(1)
            }
            JniFunction::GetArrayLength => self.jni_array_len()?,
            JniFunction::GetIntArrayElements => self.jni_int_array_elements()?,
            JniFunction::ReleaseIntArrayElements => 0,
            JniFunction::GetJavaVm => {
                let out = self.cpu.reg(1);
                if out != 0 {
                    self.store_runtime32(out, self.jni_java_vm)?;
                }
                0
            }
        };
        self.cpu.set_reg(0, value);
        self.cpu.branch_exchange(self.cpu.reg(14));
        Ok(())
    }

    fn register_jni_method(&mut self, is_static: bool) -> Result<u32, NativeRuntimeError> {
        let name = self.load_guest_c_string(self.cpu.reg(2), 256)?;
        let sig = self.load_guest_c_string(self.cpu.reg(3), 256)?;
        if let Some(method) = self.jni_methods.iter().find(|method| {
            method.name == name && method.sig == sig && method.is_static == is_static
        }) {
            return Ok(method.id);
        }
        let id = self.alloc_guest_zeroed(8, 4)?;
        self.store_runtime32(id, self.cpu.reg(2))?;
        self.store_runtime32(id.wrapping_add(4), self.cpu.reg(3))?;
        self.jni_methods.push(JniMethod {
            id,
            name,
            sig,
            is_static,
        });
        Ok(id)
    }

    fn call_jni_object_method(&mut self) -> Result<u32, NativeRuntimeError> {
        let Some(method) = self.jni_method(self.cpu.reg(2)) else {
            return Ok(0);
        };
        let method_name = method.name.clone();
        let method_sig = method.sig.clone();
        if matches!(method_name.as_str(), "_getImageData" | "getImageData") {
            return self.call_jni_get_image_data();
        }
        let value = match method_name.as_str() {
            "getLocale" => "en_US",
            "getDeviceModel" | "getDeviceModelName" => "AEMU",
            "getAndroidVersion" => "19",
            "getExternalStoragePath" => "/sdcard",
            "getPackageName" => "com.mojang.minecraftpe",
            "getFilesDir" | "getAbsolutePath" => "/data/data/com.mojang.minecraftpe/files",
            "getCacheDir" => "/data/data/com.mojang.minecraftpe/cache",
            "getUserInputString" => "",
            "getDeviceId" | "createUUID" => "00000000-0000-0000-0000-000000000000",
            _ if method_sig.ends_with("Ljava/lang/String;") => "",
            _ => return Ok(0),
        };
        self.write_guest_c_string(value)
    }

    fn call_jni_get_image_data(&mut self) -> Result<u32, NativeRuntimeError> {
        let Some((width, height, pixels)) = self.load_jni_image_data_arg()? else {
            return Ok(0);
        };
        let pixel_len =
            u32::try_from(pixels.len()).map_err(|_| NativeRuntimeError::AddressOverflow)?;
        let array_len = pixel_len
            .checked_add(2)
            .ok_or(NativeRuntimeError::AddressOverflow)?;
        let data_bytes = array_len
            .checked_mul(4)
            .ok_or(NativeRuntimeError::AddressOverflow)?;
        let data = self.alloc_guest_zeroed(data_bytes, 4)?;
        self.store_runtime32(data, width)?;
        self.store_runtime32(data.wrapping_add(4), height)?;
        for (idx, pixel) in pixels.into_iter().enumerate() {
            let offset = 8u32
                .checked_add(
                    u32::try_from(idx)
                        .map_err(|_| NativeRuntimeError::AddressOverflow)?
                        .checked_mul(4)
                        .ok_or(NativeRuntimeError::AddressOverflow)?,
                )
                .ok_or(NativeRuntimeError::AddressOverflow)?;
            self.store_runtime32(data.wrapping_add(offset), pixel)?;
        }

        let handle = self.alloc_guest_zeroed(8, 4)?;
        self.store_runtime32(handle, array_len)?;
        self.store_runtime32(handle.wrapping_add(4), data)?;
        Ok(handle)
    }

    fn load_jni_image_data_arg(
        &mut self,
    ) -> Result<Option<(u32, u32, Vec<u32>)>, NativeRuntimeError> {
        let arg = self.cpu.reg(3);
        for path_ptr in [Some(arg), self.load_runtime32(arg).ok()]
            .into_iter()
            .flatten()
        {
            let Ok(path) = self.load_guest_c_string(path_ptr, 4096) else {
                continue;
            };
            if let Ok(image) = self.hle.load_apk_image_argb_pixels(&path) {
                return Ok(Some(image));
            }
        }
        Ok(None)
    }

    fn jni_array_len(&mut self) -> Result<u32, NativeRuntimeError> {
        let array = self.cpu.reg(1);
        if array == 0 {
            return Ok(0);
        }
        self.load_runtime32(array)
    }

    fn jni_int_array_elements(&mut self) -> Result<u32, NativeRuntimeError> {
        if self.cpu.reg(2) != 0 {
            self.link
                .memory
                .store8(self.cpu.reg(2), 0)
                .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
        }
        let array = self.cpu.reg(1);
        if array == 0 {
            return Ok(0);
        }
        self.load_runtime32(array.wrapping_add(4))
    }

    fn call_jni_int_method(&self) -> u32 {
        let Some(method) = self.jni_method(self.cpu.reg(2)) else {
            return 0;
        };
        match method.name.as_str() {
            "getScreenWidth" | "_getScreenWidth" => 854,
            "getScreenHeight" | "_getScreenHeight" => 480,
            "getAndroidVersion" => 19,
            _ => 0,
        }
    }

    fn call_jni_boolean_method(&self) -> u32 {
        let Some(method) = self.jni_method(self.cpu.reg(2)) else {
            return 0;
        };
        match method.name.as_str() {
            "isTouchscreenAvailable" => 1,
            _ => 0,
        }
    }

    fn jni_method(&self, id: u32) -> Option<&JniMethod> {
        self.jni_methods.iter().find(|method| method.id == id)
    }

    fn load_guest_c_string(
        &mut self,
        ptr: u32,
        max_len: u32,
    ) -> Result<String, NativeRuntimeError> {
        if ptr == 0 {
            return Ok(String::new());
        }
        let mut bytes = Vec::new();
        for idx in 0..max_len {
            let byte = self
                .link
                .memory
                .load8(ptr.wrapping_add(idx))
                .map_err(|err| NativeRuntimeError::Memory(err.to_string()))?;
            if byte == 0 {
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(byte);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn trace_mem32(&mut self, step_idx: usize, spec: &TraceMem32Spec) {
        let base = spec.base.value(&self.cpu);
        let base_label = spec.base.label();
        eprint!(
            "MEM32 step={step_idx} pc={:#010x} {base_label}={base:#010x}",
            spec.pc,
        );
        for &offset in &spec.offsets {
            let addr = base.wrapping_add(offset);
            match self.link.memory.load32(addr) {
                Ok(value) => eprint!(" [{base_label}+{offset:#x}@{addr:#010x}]={value:#010x}"),
                Err(err) => eprint!(" [{base_label}+{offset:#x}@{addr:#010x}]=<{err}>"),
            }
        }
        eprintln!();
    }

    fn trace_mem32_deref(&mut self, step_idx: usize, spec: &TraceMem32Spec) {
        let mut base = spec.base.value(&self.cpu);
        let base_label = spec.base.label();
        eprint!(
            "MEM32_DEREF step={step_idx} pc={:#010x} {base_label}={base:#010x}",
            spec.pc,
        );
        for (depth, &offset) in spec.offsets.iter().enumerate() {
            let parent = if depth == 0 {
                base_label.clone()
            } else {
                format!("deref{}", depth - 1)
            };
            let addr = base.wrapping_add(offset);
            match self.link.memory.load32(addr) {
                Ok(value) => {
                    eprint!(" [{parent}+{offset:#x}@{addr:#010x}]=deref{depth}:{value:#010x}");
                    base = value;
                }
                Err(err) => {
                    eprint!(" [{parent}+{offset:#x}@{addr:#010x}]=<{err}>");
                    break;
                }
            }
        }
        eprintln!();
    }

    fn trace_cxx_string(&mut self, step_idx: usize, spec: &TraceCxxStringSpec) {
        let string = spec.base.value(&self.cpu).wrapping_add(spec.offset);
        let base_label = spec.base.label();
        match self.link.memory.load32(string) {
            Ok(data) if data != 0 => {
                let len = self
                    .link
                    .memory
                    .load32(data.wrapping_sub(12))
                    .unwrap_or(u32::MAX);
                let mut bytes = Vec::new();
                for idx in 0..len.min(spec.max_len) {
                    match self.link.memory.load8(data.wrapping_add(idx)) {
                        Ok(byte) => bytes.push(byte),
                        Err(err) => {
                            eprintln!(" data={data:#010x} len={len} bytes=<{}>", err);
                            return;
                        }
                    }
                }
                if !trace_bytes_match_env("AEMU_TRACE_CXX_STRING_CONTAINS", &bytes) {
                    return;
                }
                eprintln!(
                    "CXX_STRING step={step_idx} pc={:#010x} {base_label}+{:#x}={string:#010x} data={data:#010x} len={len} bytes={}",
                    spec.pc,
                    spec.offset,
                    format_trace_bytes(&bytes)
                );
            }
            Ok(data) => {
                if trace_bytes_match_env("AEMU_TRACE_CXX_STRING_CONTAINS", &[]) {
                    eprintln!(
                        "CXX_STRING step={step_idx} pc={:#010x} {base_label}+{:#x}={string:#010x} data={data:#010x} len=0 bytes=\"\"",
                        spec.pc, spec.offset,
                    );
                }
            }
            Err(err) => eprintln!(
                "CXX_STRING step={step_idx} pc={:#010x} {base_label}+{:#x}={string:#010x} data=<{}>",
                spec.pc, spec.offset, err
            ),
        }
    }

    fn trace_native_event_jsonl(
        &mut self,
        step_idx: usize,
        thread_id: u32,
        spec: &TraceNativeEventSpec,
        path: &Path,
        mem32_specs: &[TraceMem32Spec],
        deref32_specs: &[TraceMem32Spec],
        cxx_string_specs: &[TraceCxxStringSpec],
        byte_specs: &[TraceBytesSpec],
    ) {
        let regs: serde_json::Map<String, serde_json::Value> = (0..16)
            .map(|idx| (format!("r{idx}"), serde_json::json!(self.cpu.reg(idx))))
            .collect();
        let mut row = serde_json::json!({
            "step": step_idx,
            "thread": thread_id,
            "pc": spec.pc,
            "isa": format!("{:?}", self.cpu.isa()),
            "event": spec.name,
            "r0": self.cpu.reg(0),
            "r1": self.cpu.reg(1),
            "r2": self.cpu.reg(2),
            "r3": self.cpu.reg(3),
            "sp": self.cpu.reg(13),
            "lr": self.cpu.reg(14),
            "regs": regs,
            "gles_next_event_index": self.hle.trace_gles_event_index(),
            "gl_active_texture": self.hle.trace_gl_active_texture(),
            "gl_current_program": self.hle.trace_gl_current_program(),
            "gl_bound_texture_2d": self.hle.trace_gl_bound_texture_2d(),
        });
        let mem32 = self.native_event_mem32_values(spec.pc, mem32_specs);
        if !mem32.is_empty() {
            row["mem32"] = serde_json::Value::Array(mem32);
        }
        let deref32 = self.native_event_deref32_values(spec.pc, deref32_specs);
        if !deref32.is_empty() {
            row["deref32"] = serde_json::Value::Array(deref32);
        }
        let cxx_strings = self.native_event_cxx_string_values(spec.pc, cxx_string_specs);
        if !cxx_strings.is_empty() {
            row["cxx_strings"] = serde_json::Value::Array(cxx_strings);
        }
        let byte_samples = self.native_event_byte_values(spec.pc, byte_specs);
        if !byte_samples.is_empty() {
            row["byte_samples"] = serde_json::Value::Array(byte_samples);
        }
        if let Err(err) = append_trace_jsonl(path, &row) {
            eprintln!("native event trace append failed {path:?}: {err}");
        }
    }

    fn native_event_mem32_values(
        &mut self,
        pc: u32,
        specs: &[TraceMem32Spec],
    ) -> Vec<serde_json::Value> {
        specs
            .iter()
            .filter(|spec| spec.pc == pc)
            .map(|spec| self.native_event_mem32_value(spec))
            .collect()
    }

    fn native_event_mem32_value(&mut self, spec: &TraceMem32Spec) -> serde_json::Value {
        let base = spec.base.value(&self.cpu);
        let base_label = spec.base.label();
        let fields = spec
            .offsets
            .iter()
            .map(|&offset| {
                let addr = base.wrapping_add(offset);
                let mut field = serde_json::json!({
                    "label": format!("{base_label}+{offset:#x}"),
                    "offset": offset,
                    "addr": addr,
                });
                match self.link.memory.load32(addr) {
                    Ok(value) => field["value"] = serde_json::json!(value),
                    Err(err) => field["error"] = serde_json::json!(err.to_string()),
                }
                field
            })
            .collect();
        serde_json::json!({
            "base": base_label,
            "base_value": base,
            "fields": serde_json::Value::Array(fields),
        })
    }

    fn native_event_deref32_values(
        &mut self,
        pc: u32,
        specs: &[TraceMem32Spec],
    ) -> Vec<serde_json::Value> {
        specs
            .iter()
            .filter(|spec| spec.pc == pc)
            .map(|spec| self.native_event_deref32_value(spec))
            .collect()
    }

    fn native_event_deref32_value(&mut self, spec: &TraceMem32Spec) -> serde_json::Value {
        let mut base = spec.base.value(&self.cpu);
        let base_label = spec.base.label();
        let mut chain = Vec::new();
        for (depth, &offset) in spec.offsets.iter().enumerate() {
            let parent = if depth == 0 {
                base_label.clone()
            } else {
                format!("deref{}", depth - 1)
            };
            let addr = base.wrapping_add(offset);
            let label = format!("{parent}+{offset:#x}");
            let mut item = serde_json::json!({
                "depth": depth,
                "parent": parent,
                "label": label,
                "offset": offset,
                "addr": addr,
            });
            match self.link.memory.load32(addr) {
                Ok(value) => {
                    item["value"] = serde_json::json!(value);
                    base = value;
                }
                Err(err) => {
                    item["error"] = serde_json::json!(err.to_string());
                    chain.push(item);
                    break;
                }
            }
            chain.push(item);
        }
        serde_json::json!({
            "base": base_label,
            "base_value": spec.base.value(&self.cpu),
            "chain": chain,
        })
    }

    fn native_event_cxx_string_values(
        &mut self,
        pc: u32,
        specs: &[TraceCxxStringSpec],
    ) -> Vec<serde_json::Value> {
        specs
            .iter()
            .filter(|spec| spec.pc == pc)
            .map(|spec| self.native_event_cxx_string_value(spec))
            .collect()
    }

    fn native_event_cxx_string_value(&mut self, spec: &TraceCxxStringSpec) -> serde_json::Value {
        let string = spec.base.value(&self.cpu).wrapping_add(spec.offset);
        let base_label = spec.base.label();
        let mut row = serde_json::json!({
            "base": base_label,
            "offset": spec.offset,
            "addr": string,
            "max_len": spec.max_len,
        });
        match self.link.memory.load32(string) {
            Ok(data) if data != 0 => {
                row["data"] = serde_json::json!(data);
                let len = self
                    .link
                    .memory
                    .load32(data.wrapping_sub(12))
                    .unwrap_or(u32::MAX);
                row["len"] = serde_json::json!(len);
                let mut bytes = Vec::new();
                for idx in 0..len.min(spec.max_len) {
                    match self.link.memory.load8(data.wrapping_add(idx)) {
                        Ok(byte) => bytes.push(byte),
                        Err(err) => {
                            row["error"] = serde_json::json!(err.to_string());
                            return row;
                        }
                    }
                }
                row["bytes"] = serde_json::json!(String::from_utf8_lossy(&bytes).into_owned());
                row["escaped"] = serde_json::json!(format_trace_bytes(&bytes));
                row["truncated"] = serde_json::json!(len > spec.max_len);
            }
            Ok(data) => {
                row["data"] = serde_json::json!(data);
                row["len"] = serde_json::json!(0);
                row["bytes"] = serde_json::json!("");
                row["escaped"] = serde_json::json!("\"\"");
                row["truncated"] = serde_json::json!(false);
            }
            Err(err) => row["error"] = serde_json::json!(err.to_string()),
        }
        row
    }

    fn native_event_byte_values(
        &mut self,
        pc: u32,
        specs: &[TraceBytesSpec],
    ) -> Vec<serde_json::Value> {
        specs
            .iter()
            .filter(|spec| spec.pc == pc)
            .map(|spec| self.native_event_byte_value(spec))
            .collect()
    }

    fn native_event_byte_value(&mut self, spec: &TraceBytesSpec) -> serde_json::Value {
        let mut row = serde_json::json!({
            "source": spec.source.label(&self.cpu),
            "max_len": spec.max_len,
        });
        let addr = match spec.source.addr(&self.cpu, &mut self.link.memory) {
            Ok(addr) => addr,
            Err(err) => {
                row["error"] = serde_json::json!(err.to_string());
                return row;
            }
        };
        row["addr"] = serde_json::json!(addr);
        let mut bytes = Vec::new();
        for idx in 0..spec.max_len {
            match self.link.memory.load8(addr.wrapping_add(idx)) {
                Ok(byte) => bytes.push(byte),
                Err(err) => {
                    row["error"] = serde_json::json!(err.to_string());
                    break;
                }
            }
        }
        row["len"] = serde_json::json!(bytes.len());
        row["hex"] = serde_json::json!(hex_lower(&bytes));
        row["escaped"] = serde_json::json!(format_trace_bytes(&bytes));
        let mut sha1 = Sha1::new();
        sha1.update(&bytes);
        row["sha1"] = serde_json::json!(hex_lower(&sha1.finalize()));
        row
    }
}

fn parse_trace_pc_ranges() -> Vec<(u32, u32)> {
    let Some(raw) = std::env::var("AEMU_TRACE_PC_RANGE").ok() else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|range| {
            let (start, end) = range.trim().split_once(':')?;
            let start = parse_u32_env(start)?;
            let end = parse_u32_env(end)?;
            (start < end).then_some((start, end))
        })
        .collect()
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

fn parse_usize_env(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.trim().parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceMem32Spec {
    pc: u32,
    base: TraceMemBase,
    offsets: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceCxxStringSpec {
    pc: u32,
    base: TraceMemBase,
    offset: u32,
    max_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceBytesSpec {
    pc: u32,
    source: TraceBytesSource,
    max_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TraceBytesSource {
    Direct {
        base: TraceMemBase,
        offset: u32,
    },
    Deref32 {
        base: TraceMemBase,
        offset: u32,
    },
    Deref32Chain {
        base: TraceMemBase,
        offsets: Vec<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceNativeEventSpec {
    pc: u32,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceMemBase {
    Reg(usize),
    Addr(u32),
}

impl TraceMemBase {
    fn value(self, cpu: &Cpu) -> u32 {
        match self {
            Self::Reg(reg) => cpu.reg(reg),
            Self::Addr(addr) => addr,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Reg(13) => "sp".to_string(),
            Self::Reg(14) => "lr".to_string(),
            Self::Reg(15) => "pc".to_string(),
            Self::Reg(reg) => format!("r{reg}"),
            Self::Addr(addr) => format!("{addr:#010x}"),
        }
    }
}

impl TraceBytesSource {
    fn addr<M: Memory>(&self, cpu: &Cpu, memory: &mut M) -> Result<u32, Trap> {
        match *self {
            Self::Direct { base, offset } => Ok(base.value(cpu).wrapping_add(offset)),
            Self::Deref32 { base, offset } => memory.load32(base.value(cpu).wrapping_add(offset)),
            Self::Deref32Chain { base, ref offsets } => {
                let mut addr = base.value(cpu);
                for &offset in offsets {
                    addr = memory.load32(addr.wrapping_add(offset))?;
                }
                Ok(addr)
            }
        }
    }

    fn label(&self, cpu: &Cpu) -> String {
        match *self {
            Self::Direct { base, offset } => {
                format!("{}+{offset:#x}", base.label())
            }
            Self::Deref32 { base, offset } => {
                let ptr_addr = base.value(cpu).wrapping_add(offset);
                format!("*{}+{offset:#x}@{ptr_addr:#010x}", base.label())
            }
            Self::Deref32Chain { base, ref offsets } => {
                let mut label = format!("*{}", base.label());
                for &offset in offsets {
                    label.push_str(&format!("+{offset:#x}"));
                }
                label
            }
        }
    }
}

fn parse_trace_mem32_specs() -> Vec<TraceMem32Spec> {
    let Some(raw) = std::env::var("AEMU_TRACE_MEM32").ok() else {
        return Vec::new();
    };
    parse_trace_mem32_specs_raw(&raw)
}

fn parse_trace_mem32_deref_specs() -> Vec<TraceMem32Spec> {
    let Some(raw) = std::env::var("AEMU_TRACE_MEM32_DEREF").ok() else {
        return Vec::new();
    };
    parse_trace_mem32_specs_raw(&raw)
}

fn parse_trace_cxx_string_specs() -> Vec<TraceCxxStringSpec> {
    let Some(raw) = std::env::var("AEMU_TRACE_CXX_STRING").ok() else {
        return Vec::new();
    };
    parse_trace_cxx_string_specs_raw(&raw)
}

fn trace_native_events_path() -> Option<PathBuf> {
    std::env::var_os("AEMU_TRACE_NATIVE_EVENTS_JSONL").map(PathBuf::from)
}

fn parse_trace_native_event_specs() -> Vec<TraceNativeEventSpec> {
    let Some(raw) = std::env::var("AEMU_TRACE_NATIVE_EVENTS").ok() else {
        return Vec::new();
    };
    parse_trace_native_event_specs_raw(&raw)
}

fn parse_trace_native_event_mem32_specs() -> Vec<TraceMem32Spec> {
    let Some(raw) = std::env::var("AEMU_TRACE_NATIVE_EVENT_MEM32").ok() else {
        return Vec::new();
    };
    parse_trace_mem32_specs_raw(&raw)
}

fn parse_trace_native_event_deref32_specs() -> Vec<TraceMem32Spec> {
    let Some(raw) = std::env::var("AEMU_TRACE_NATIVE_EVENT_DEREF32").ok() else {
        return Vec::new();
    };
    parse_trace_mem32_specs_raw(&raw)
}

fn parse_trace_native_event_cxx_string_specs() -> Vec<TraceCxxStringSpec> {
    let Some(raw) = std::env::var("AEMU_TRACE_NATIVE_EVENT_CXX_STRING").ok() else {
        return Vec::new();
    };
    parse_trace_cxx_string_specs_raw(&raw)
}

fn parse_trace_native_event_bytes_specs() -> Vec<TraceBytesSpec> {
    let Some(raw) = std::env::var("AEMU_TRACE_NATIVE_EVENT_BYTES").ok() else {
        return Vec::new();
    };
    parse_trace_bytes_specs_raw(&raw)
}

fn parse_trace_native_event_specs_raw(raw: &str) -> Vec<TraceNativeEventSpec> {
    raw.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (pc, name) = entry
                .split_once(':')
                .map_or((entry, ""), |(pc, name)| (pc.trim(), name.trim()));
            let pc = parse_u32_env(pc)?;
            let name = if name.is_empty() {
                format!("{pc:#010x}")
            } else {
                name.to_string()
            };
            Some(TraceNativeEventSpec { pc, name })
        })
        .collect()
}

fn parse_trace_bytes_specs_raw(raw: &str) -> Vec<TraceBytesSpec> {
    raw.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (pc, fields) = entry.split_once(':')?;
            let pc = parse_u32_env(pc)?;
            let (source_raw, max_len_raw) = fields
                .rsplit_once(',')
                .map_or((fields.trim(), None), |(source, max_len)| {
                    (source.trim(), Some(max_len.trim()))
                });
            let source = parse_trace_bytes_source(source_raw)?;
            let max_len = max_len_raw.and_then(parse_u32_env).unwrap_or(64);
            Some(TraceBytesSpec {
                pc,
                source,
                max_len,
            })
        })
        .collect()
}

fn parse_trace_bytes_source(raw: &str) -> Option<TraceBytesSource> {
    let raw = raw.trim();
    if let Some(deref) = raw.strip_prefix('*') {
        let mut parts = deref
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty());
        let (base, first_offset) = parse_trace_mem32_base(parts.next()?)?;
        let mut offsets = vec![first_offset];
        for part in parts {
            offsets.push(parse_u32_env(part.trim_start_matches('+'))?);
        }
        if offsets.len() == 1 {
            Some(TraceBytesSource::Deref32 {
                base,
                offset: first_offset,
            })
        } else {
            Some(TraceBytesSource::Deref32Chain { base, offsets })
        }
    } else {
        let (base, offset) = parse_trace_mem32_base(raw)?;
        Some(TraceBytesSource::Direct { base, offset })
    }
}

fn parse_trace_cxx_string_specs_raw(raw: &str) -> Vec<TraceCxxStringSpec> {
    raw.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (pc, fields) = entry.split_once(':')?;
            let pc = parse_u32_env(pc)?;
            let mut fields = fields
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty());
            let (base, offset) = parse_trace_mem32_base(fields.next()?)?;
            let max_len = fields.next().and_then(parse_u32_env).unwrap_or(96);
            Some(TraceCxxStringSpec {
                pc,
                base,
                offset,
                max_len,
            })
        })
        .collect()
}

fn parse_trace_mem32_specs_raw(raw: &str) -> Vec<TraceMem32Spec> {
    raw.split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (pc, fields) = entry.split_once(':')?;
            let pc = parse_u32_env(pc)?;
            let mut fields = fields
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty());
            let first = fields.next()?;
            let (base, first_offset) = parse_trace_mem32_base(first)?;
            let mut offsets = vec![first_offset];
            for offset in fields {
                offsets.push(parse_trace_mem32_offset(offset)?);
            }
            Some(TraceMem32Spec { pc, base, offsets })
        })
        .collect()
}

fn parse_trace_mem32_base(raw: &str) -> Option<(TraceMemBase, u32)> {
    let raw = raw.trim();
    let (base, offset) = if let Some((base, offset)) = raw.split_once('+') {
        (base.trim(), parse_trace_mem32_offset(offset)?)
    } else {
        (raw, 0)
    };
    Some((parse_trace_mem32_base_value(base)?, offset))
}

fn parse_trace_mem32_base_value(raw: &str) -> Option<TraceMemBase> {
    let raw = raw.trim();
    match raw {
        "sp" => Some(TraceMemBase::Reg(13)),
        "lr" => Some(TraceMemBase::Reg(14)),
        "pc" => Some(TraceMemBase::Reg(15)),
        _ => {
            if let Some(reg) = raw.strip_prefix('r') {
                let reg = reg.parse::<usize>().ok()?;
                return (reg < 16).then_some(TraceMemBase::Reg(reg));
            }
            Some(TraceMemBase::Addr(parse_u32_env(raw)?))
        }
    }
}

fn parse_trace_mem32_offset(raw: &str) -> Option<u32> {
    parse_u32_env(raw.trim().trim_start_matches('+'))
}

fn format_trace_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    out.push('"');
    for byte in bytes.iter().copied() {
        for escaped in std::ascii::escape_default(byte) {
            out.push(char::from(escaped));
        }
    }
    out.push('"');
    out
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes.iter().copied() {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    out
}

fn trace_bytes_match_env(name: &str, bytes: &[u8]) -> bool {
    let Some(needle) = std::env::var(name).ok().filter(|needle| !needle.is_empty()) else {
        return true;
    };
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HleTraceMatcher {
    Contains(String),
    Exact(String),
}

impl HleTraceMatcher {
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            None
        } else if let Some(exact) = raw.strip_prefix('=') {
            let exact = exact.trim();
            if exact.is_empty() {
                None
            } else {
                Some(Self::Exact(exact.to_string()))
            }
        } else {
            Some(Self::Contains(raw.to_string()))
        }
    }

    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Contains(needle) => name.contains(needle),
            Self::Exact(exact) => name == exact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HleTraceFilter {
    All,
    One(HleTraceMatcher),
    Any(Vec<HleTraceMatcher>),
}

impl HleTraceFilter {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::One(matcher) => matcher.matches(name),
            Self::Any(matchers) => matchers.iter().any(|matcher| matcher.matches(name)),
        }
    }
}

fn parse_trace_hle_filter() -> Option<HleTraceFilter> {
    let raw = std::env::var("AEMU_TRACE_HLE").ok()?;
    parse_trace_hle_filter_raw(&raw)
}

fn parse_trace_hle_filter_raw(raw: &str) -> Option<HleTraceFilter> {
    let raw = raw.trim();
    if raw.is_empty() {
        None
    } else if raw == "*" {
        Some(HleTraceFilter::All)
    } else {
        let matchers = raw
            .split(',')
            .filter_map(HleTraceMatcher::parse)
            .collect::<Vec<_>>();
        match matchers.as_slice() {
            [] => None,
            [matcher] => Some(HleTraceFilter::One(matcher.clone())),
            _ => Some(HleTraceFilter::Any(matchers)),
        }
    }
}

fn collect_linked_runtime_hle_traps(link: &NativeLinkReport) -> Vec<RuntimeHleTrap> {
    link.global_symbols
        .iter()
        .filter_map(|symbol| match symbol.name.as_str() {
            "__dynamic_cast" => Some(RuntimeHleTrap {
                address: symbol.address & !1,
                isa: if symbol.address & 1 != 0 {
                    Isa::Thumb
                } else {
                    Isa::Arm
                },
                name: "__dynamic_cast",
                kind: RuntimeHleTrapKind::CxxDynamicCast,
            }),
            _ => None,
        })
        .collect()
}

fn build_minecraft_resource_bridge(link: &NativeLinkReport) -> Option<MinecraftResourceBridge> {
    let render = link_symbol_address_in_library(link, MCPE_LIBRARY, MCPE_GAME_RENDERER_RENDER)?;
    let on_resources_loaded =
        link_symbol_address_in_library(link, MCPE_LIBRARY, MCPE_ON_RESOURCES_LOADED)?;
    Some(MinecraftResourceBridge {
        render_resource_gate_pc: (render & !1)
            .wrapping_add(MCPE_GAME_RENDERER_RENDER_RESOURCE_GATE_OFFSET),
        on_resources_loaded,
    })
}

fn minecraft_resource_bridge_enabled() -> bool {
    if std::env::var_os("AEMU_DISABLE_MCPE_RESOURCE_BRIDGE").is_some() {
        return false;
    }
    std::env::var_os("AEMU_ENABLE_MCPE_RESOURCE_BRIDGE").is_some()
        || std::env::var_os("AEMU_MCPE_HLE_GAME_LOGIC").is_some()
}

fn minecraft_on_resources_loaded_steps() -> usize {
    std::env::var(MCPE_ON_RESOURCES_LOADED_STEPS_ENV)
        .ok()
        .and_then(|raw| parse_nonzero_usize(&raw))
        .unwrap_or(MCPE_ON_RESOURCES_LOADED_STEPS)
}

fn parse_nonzero_usize(raw: &str) -> Option<usize> {
    let value = raw.trim().parse().ok()?;
    (value != 0).then_some(value)
}

fn link_symbol_address_in_library(
    link: &NativeLinkReport,
    library_name: &str,
    name: &str,
) -> Option<u32> {
    link.objects
        .iter()
        .find(|object| object.library_name == library_name)
        .and_then(|object| {
            object
                .defined_symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .map(|symbol| symbol.address)
        })
}

fn collect_unwind_tables(link: &NativeLinkReport) -> Vec<HleUnwindTable> {
    link.objects
        .iter()
        .filter_map(|object| {
            let exidx = object.arm_exidx?;
            let memory_end = object.memory_base.checked_add(object.memory_size)?;
            Some(HleUnwindTable {
                memory_base: object.memory_base,
                memory_end,
                exidx_addr: exidx.addr,
                exidx_count: exidx.size / 8,
            })
        })
        .collect()
}

fn parse_u32_env(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        raw.parse().ok()
    }
}

fn append_trace_jsonl(path: &Path, row: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, row)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    file.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::armv7a::Memory;
    use crate::guest_memory::MappedMemory;
    use crate::hle_imports::{HleCallBehavior, HleSymbolKind, HleSymbolShape};
    use crate::native_loader::{
        DEFAULT_HLE_BASE, HleSymbol, LoadedNativeObject, NativeLinkReport, NativeSymbol,
    };

    use super::*;

    #[test]
    fn parses_mem32_trace_specs() {
        assert_eq!(
            parse_trace_mem32_specs_raw(
                "0x70bc79dc:r0+0x80,+0x84; 0x70bc543e:sp+16; 1234:0x60000000"
            ),
            vec![
                TraceMem32Spec {
                    pc: 0x70bc_79dc,
                    base: TraceMemBase::Reg(0),
                    offsets: vec![0x80, 0x84],
                },
                TraceMem32Spec {
                    pc: 0x70bc_543e,
                    base: TraceMemBase::Reg(13),
                    offsets: vec![16],
                },
                TraceMem32Spec {
                    pc: 1234,
                    base: TraceMemBase::Addr(0x6000_0000),
                    offsets: vec![0],
                },
            ]
        );
    }

    #[test]
    fn parses_native_event_trace_specs() {
        assert_eq!(
            parse_trace_native_event_specs_raw(
                "0x716f0534:TextureGroup::uploadTexture; 0x716eb818"
            ),
            vec![
                TraceNativeEventSpec {
                    pc: 0x716f_0534,
                    name: "TextureGroup::uploadTexture".to_string(),
                },
                TraceNativeEventSpec {
                    pc: 0x716e_b818,
                    name: "0x716eb818".to_string(),
                },
            ]
        );
        assert_eq!(
            parse_trace_mem32_specs_raw("0x716eb818:r0+0x24,+0x28"),
            vec![TraceMem32Spec {
                pc: 0x716e_b818,
                base: TraceMemBase::Reg(0),
                offsets: vec![0x24, 0x28],
            }]
        );
        assert_eq!(
            parse_trace_cxx_string_specs_raw("0x716f2038:r2+0x4,96"),
            vec![TraceCxxStringSpec {
                pc: 0x716f_2038,
                base: TraceMemBase::Reg(2),
                offset: 0x04,
                max_len: 96,
            }]
        );
        assert_eq!(
            parse_trace_bytes_specs_raw(
                "0x716f2038:*r2+0x4,32;0x716f2040:r3,16;0x716f2044:*r0+0x4,+0x18,64"
            ),
            vec![
                TraceBytesSpec {
                    pc: 0x716f_2038,
                    source: TraceBytesSource::Deref32 {
                        base: TraceMemBase::Reg(2),
                        offset: 0x04,
                    },
                    max_len: 32,
                },
                TraceBytesSpec {
                    pc: 0x716f_2040,
                    source: TraceBytesSource::Direct {
                        base: TraceMemBase::Reg(3),
                        offset: 0,
                    },
                    max_len: 16,
                },
                TraceBytesSpec {
                    pc: 0x716f_2044,
                    source: TraceBytesSource::Deref32Chain {
                        base: TraceMemBase::Reg(0),
                        offsets: vec![0x04, 0x18],
                    },
                    max_len: 64,
                },
            ]
        );
    }

    #[test]
    fn parses_multi_hle_trace_filter() {
        let filter = parse_trace_hle_filter_raw("glDraw, eglSwap").unwrap();
        assert!(filter.matches("glDrawElements"));
        assert!(filter.matches("eglSwapBuffers"));
        assert!(!filter.matches("glBindTexture"));
    }

    #[test]
    fn parses_exact_hle_trace_filter() {
        let filter = parse_trace_hle_filter_raw("=read, =open").unwrap();
        assert!(filter.matches("read"));
        assert!(filter.matches("open"));
        assert!(!filter.matches("pthread_create"));
        assert!(!filter.matches("fread"));
    }

    #[test]
    fn parses_nonzero_usize_env_values() {
        assert_eq!(parse_nonzero_usize("250000000"), Some(250_000_000));
        assert_eq!(parse_nonzero_usize("0"), None);
        assert_eq!(parse_nonzero_usize("not-a-number"), None);
    }

    #[test]
    fn parses_native_cpu_backend_names() {
        assert_eq!(
            NativeCpuBackendKind::parse("aemu"),
            Some(NativeCpuBackendKind::AemuInterpreter)
        );
        assert_eq!(NativeCpuBackendKind::parse("unknown"), None);
    }

    #[test]
    fn write_guest_words_preserves_arm_alignment_after_byte_allocations() {
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
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

        runtime.write_guest_bytes(&[0xaa]).unwrap();
        let words = runtime.write_guest_words(&[0xe12f_ff1e]).unwrap();

        assert_eq!(words % 4, 0);
        assert_eq!(runtime.link.memory.load32(words).unwrap(), 0xe12f_ff1e);
    }

    #[test]
    fn builds_minecraft_resource_bridge_from_target_symbols() {
        let report = minecraft_resource_bridge_test_report();

        assert_eq!(
            build_minecraft_resource_bridge(&report),
            Some(MinecraftResourceBridge {
                render_resource_gate_pc: 0x70ec_c8f2,
                on_resources_loaded: 0x70bb_eb1d,
            })
        );
    }

    #[test]
    fn does_not_install_minecraft_resource_bridge_by_default() {
        let report = minecraft_resource_bridge_test_report();
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_1000,
            stack_size: 0x1000,
            tls_base: 0x5000_2000,
            tls_size: 0x1000,
            heap_base: 0x5000_3000,
            heap_size: 0x1000,
        };
        let runtime = NativeRuntime::new(report, config).unwrap();

        assert_eq!(runtime.minecraft_resource_bridge, None);
    }

    fn minecraft_resource_bridge_test_report() -> NativeLinkReport {
        NativeLinkReport {
            apk_path: PathBuf::from("mcpe.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: vec![LoadedNativeObject {
                entry_name: "lib/armeabi-v7a/libminecraftpe.so".to_string(),
                library_name: MCPE_LIBRARY.to_string(),
                load_bias: 0x7050_0000,
                memory_base: 0x7050_0000,
                memory_size: 0x200_0000,
                entry: 0,
                needed: Vec::new(),
                imports: Vec::new(),
                defined_symbols: vec![
                    NativeSymbol {
                        name: MCPE_GAME_RENDERER_RENDER.to_string(),
                        address: 0x70ec_c755,
                        library_name: MCPE_LIBRARY.to_string(),
                    },
                    NativeSymbol {
                        name: MCPE_ON_RESOURCES_LOADED.to_string(),
                        address: 0x70bb_eb1d,
                        library_name: MCPE_LIBRARY.to_string(),
                    },
                ],
                relocations: Vec::new(),
                relocation_count: 0,
                init: None,
                init_array: None,
                arm_exidx: None,
            }],
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        }
    }

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
                args: [0x5000_0100, 0, 0, 0],
            }
        );
        assert_eq!(runtime.cpu.reg(0), 5);
        assert_eq!(runtime.cpu.pc(), 0x1234);
    }

    #[test]
    fn run_function_can_stop_on_named_hle_call() {
        let hle_address = 0x6f00_0000;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(hle_address, 0x1000).unwrap();
        memory.store32(hle_address, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![HleSymbol {
                name: "eglSwapBuffers".to_string(),
                address: hle_address,
                kind: HleSymbolKind::Egl,
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
            runtime
                .run_function_with_args_until_hle(hle_address, &[1, 4], 8, Some("eglSwapBuffers"))
                .unwrap(),
            NativeRuntimeFunctionExit::HleCall {
                name: "eglSwapBuffers".to_string(),
                address: hle_address,
                args: [1, 4, 0, 0],
                step: 0,
            }
        );
    }

    #[test]
    fn continue_until_hle_resumes_after_stopped_hle_call() {
        let swap_address = 0x6f00_0000;
        let flush_address = 0x6f00_0004;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(swap_address, 0x1000).unwrap();
        memory.store32(swap_address, HLE_TRAP_ARM_INSTR).unwrap();
        memory.store32(flush_address, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![
                HleSymbol {
                    name: "eglSwapBuffers".to_string(),
                    address: swap_address,
                    kind: HleSymbolKind::Egl,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::Implemented,
                },
                HleSymbol {
                    name: "glFlush".to_string(),
                    address: flush_address,
                    kind: HleSymbolKind::Gles,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::Implemented,
                },
            ],
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
        runtime.cpu.set_pc(swap_address);
        runtime.cpu.set_reg(14, flush_address);

        assert_eq!(
            runtime
                .continue_until_hle(8, Some("eglSwapBuffers"))
                .unwrap(),
            NativeRuntimeFunctionExit::HleCall {
                name: "eglSwapBuffers".to_string(),
                address: swap_address,
                args: [0, 0, 0, 0],
                step: 0,
            }
        );
        assert_eq!(runtime.cpu.pc(), flush_address);

        assert_eq!(
            runtime.continue_until_hle(8, Some("glFlush")).unwrap(),
            NativeRuntimeFunctionExit::HleCall {
                name: "glFlush".to_string(),
                address: flush_address,
                args: [1, 0, 0, 0],
                step: 0,
            }
        );
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
                entry_name: "lib/armeabi-v7a/libgame.so".to_string(),
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
                arm_exidx: None,
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

    #[test]
    fn native_activity_jni_table_preserves_refs_and_exposes_java_vm() {
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
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
            heap_size: 0x4000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        let harness = runtime.prepare_native_activity().unwrap();
        let env_vtable = runtime.link.memory.load32(harness.jni_env).unwrap();

        let new_global_ref = runtime.link.memory.load32(env_vtable + 0x54).unwrap();
        runtime
            .run_function_with_args(
                new_global_ref,
                &[harness.jni_env, harness.activity_class],
                16,
            )
            .unwrap();
        assert_eq!(runtime.cpu.reg(0), harness.activity_class);

        let method_name = runtime.write_guest_c_string("method").unwrap();
        let method_sig = runtime
            .write_guest_c_string("()Ljava/lang/String;")
            .unwrap();
        let get_method_id = runtime.link.memory.load32(env_vtable + 0x84).unwrap();
        runtime
            .run_function_with_args(
                get_method_id,
                &[
                    harness.jni_env,
                    harness.activity_class,
                    method_name,
                    method_sig,
                ],
                16,
            )
            .unwrap();
        let method_id = runtime.cpu.reg(0);
        assert_ne!(method_id, 0);

        let locale_name = runtime.write_guest_c_string("getLocale").unwrap();
        let get_locale_id = runtime.link.memory.load32(env_vtable + 0x84).unwrap();
        runtime
            .run_function_with_args(
                get_locale_id,
                &[
                    harness.jni_env,
                    harness.activity_class,
                    locale_name,
                    method_sig,
                ],
                16,
            )
            .unwrap();
        let get_locale_id = runtime.cpu.reg(0);

        let call_object_method = runtime.link.memory.load32(env_vtable + 0x88).unwrap();
        runtime
            .run_function_with_args(
                call_object_method,
                &[harness.jni_env, harness.activity_class, get_locale_id],
                16,
            )
            .unwrap();
        let locale_string = runtime.cpu.reg(0);
        assert_ne!(locale_string, 0);

        let get_string_utf_chars = runtime.link.memory.load32(env_vtable + 0x2a4).unwrap();
        runtime
            .run_function_with_args(
                get_string_utf_chars,
                &[harness.jni_env, locale_string, 0],
                16,
            )
            .unwrap();
        assert_eq!(
            runtime.load_guest_c_string(runtime.cpu.reg(0), 16).unwrap(),
            "en_US"
        );

        let width_name = runtime.write_guest_c_string("getScreenWidth").unwrap();
        let int_sig = runtime.write_guest_c_string("()I").unwrap();
        runtime
            .run_function_with_args(
                get_method_id,
                &[harness.jni_env, harness.activity_class, width_name, int_sig],
                16,
            )
            .unwrap();
        let width_id = runtime.cpu.reg(0);
        let call_int_method = runtime.link.memory.load32(env_vtable + 0xc4).unwrap();
        runtime
            .run_function_with_args(
                call_int_method,
                &[harness.jni_env, harness.activity_class, width_id],
                16,
            )
            .unwrap();
        assert_eq!(runtime.cpu.reg(0), 854);

        let java_vm_out = runtime.alloc_guest_zeroed(4, 4).unwrap();
        let get_java_vm = runtime.link.memory.load32(env_vtable + 0x36c).unwrap();
        runtime
            .run_function_with_args(get_java_vm, &[harness.jni_env, java_vm_out], 16)
            .unwrap();
        assert_eq!(runtime.cpu.reg(0), 0);
        assert_eq!(
            runtime.link.memory.load32(java_vm_out).unwrap(),
            harness.java_vm
        );
    }

    #[test]
    fn native_activity_queues_lifecycle_alooper_sources() {
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
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
            heap_size: 0x4000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        let harness = runtime.prepare_native_activity().unwrap();
        let app = runtime.prepare_android_app(harness).unwrap();
        let out = runtime.alloc_guest_zeroed(12, 4).unwrap();

        runtime.cpu.set_isa(Isa::Arm);
        runtime.cpu.set_reg(14, 0x6000_0000);
        runtime.cpu.set_reg(0, 0);
        runtime.cpu.set_reg(1, out);
        runtime.cpu.set_reg(2, out + 4);
        runtime.cpu.set_reg(3, out + 8);
        runtime
            .hle
            .dispatch(
                "ALooper_pollAll",
                &mut runtime.cpu,
                &mut runtime.link.memory,
            )
            .unwrap();

        let source = runtime.link.memory.load32(out + 8).unwrap();
        assert_eq!(runtime.cpu.reg(0), ANDROID_POLL_SOURCE_MAIN);
        assert_eq!(runtime.link.memory.load32(source).unwrap(), 1);
        assert_eq!(runtime.link.memory.load32(source + 4).unwrap(), app);
        assert_ne!(runtime.link.memory.load32(source + 8).unwrap(), 0);
        assert_eq!(
            runtime.link.memory.load32(source + 12).unwrap(),
            APP_CMD_START
        );

        runtime
            .hle
            .dispatch(
                "ALooper_pollAll",
                &mut runtime.cpu,
                &mut runtime.link.memory,
            )
            .unwrap();
        let source = runtime.link.memory.load32(out + 8).unwrap();
        assert_eq!(
            runtime.link.memory.load32(source + 12).unwrap(),
            APP_CMD_RESUME
        );
    }

    #[test]
    fn services_pthread_created_guest_thread_entry() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x4000_0000, 0x1000).unwrap();
        // Thumb: movs r1,#42; str r1,[r0]; bx lr.
        memory.store16(0x4000_0100, 0x212a).unwrap();
        memory.store16(0x4000_0102, 0x6001).unwrap();
        memory.store16(0x4000_0104, 0x4770).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_0000,
            stack_size: 0x0010_0000,
            tls_base: 0x5010_0000,
            tls_size: 0x1000,
            heap_base: 0x5020_0000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        runtime.cpu.set_isa(Isa::Arm);
        runtime.cpu.set_reg(14, 0);
        runtime.cpu.set_reg(0, 0x4000_0300);
        runtime.cpu.set_reg(2, 0x4000_0101);
        runtime.cpu.set_reg(3, 0x4000_0200);

        runtime
            .hle
            .dispatch("pthread_create", &mut runtime.cpu, &mut runtime.link.memory)
            .unwrap();
        assert_eq!(runtime.link.memory.load32(0x4000_0200).unwrap(), 0);

        runtime.service_guest_threads(8).unwrap();

        assert_eq!(runtime.link.memory.load32(0x4000_0200).unwrap(), 42);
        assert!(runtime.guest_threads.is_empty());
        assert_eq!(runtime.cpu.pc(), 0);
    }

    #[test]
    fn blocks_guest_thread_on_pthread_cond_wait_until_signal() {
        let wait_addr = 0x6f00_0000;
        let signal_addr = 0x6f00_0004;
        let cond = 0x6000_0100;
        let mutex = 0x6000_0200;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x6f00_0000, 0x1000).unwrap();
        memory.store32(wait_addr, HLE_TRAP_ARM_INSTR).unwrap();
        memory.store32(signal_addr, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![
                HleSymbol {
                    name: "pthread_cond_wait".to_string(),
                    address: wait_addr,
                    kind: HleSymbolKind::Libc,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::ReturnZero,
                },
                HleSymbol {
                    name: "pthread_cond_signal".to_string(),
                    address: signal_addr,
                    kind: HleSymbolKind::Libc,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::ReturnZero,
                },
            ],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_0000,
            stack_size: 0x0010_0000,
            tls_base: 0x5010_0000,
            tls_size: 0x1000,
            heap_base: 0x5020_0000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_pc(wait_addr);
        cpu.set_reg(0, cond);
        cpu.set_reg(1, mutex);
        cpu.set_reg(14, THREAD_RETURN_SENTINEL);
        runtime.guest_threads.push_back(GuestThread {
            id: 7,
            cpu,
            wait: GuestThreadWait::Runnable,
            trace_tail: VecDeque::with_capacity(GUEST_THREAD_TRACE_LEN),
            trace_pc_count: 0,
            trace_mem32_count: 0,
            trace_mem32_deref_count: 0,
            trace_cxx_string_count: 0,
        });

        runtime.service_guest_threads(4).unwrap();

        assert_eq!(runtime.guest_threads.len(), 1);
        assert_eq!(
            runtime.guest_threads[0].wait,
            GuestThreadWait::Condvar { cond, mutex }
        );
        assert_eq!(runtime.guest_threads[0].cpu.pc(), THREAD_RETURN_SENTINEL);

        runtime.cpu.set_isa(Isa::Arm);
        runtime.cpu.set_pc(signal_addr);
        runtime.cpu.set_reg(0, cond);
        runtime.cpu.set_reg(14, CALL_RETURN_SENTINEL);
        let step = runtime.step().unwrap();
        let NativeRuntimeStep::HleCall { name, args, .. } = step else {
            panic!("expected pthread_cond_signal HLE call");
        };
        runtime.handle_thread_sync_hle(1, &name, args).unwrap();

        assert_eq!(runtime.guest_threads[0].wait, GuestThreadWait::Runnable);
        runtime.service_guest_threads(4).unwrap();
        assert!(runtime.guest_threads.is_empty());
    }

    #[test]
    fn blocks_main_thread_on_pthread_mutex_lock_until_guest_unlocks() {
        let lock_addr = 0x6f00_0000;
        let unlock_addr = 0x6f00_0004;
        let mutex = 0x6000_0200;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x6f00_0000, 0x1000).unwrap();
        memory.store32(lock_addr, HLE_TRAP_ARM_INSTR).unwrap();
        memory.store32(unlock_addr, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![
                HleSymbol {
                    name: "pthread_mutex_lock".to_string(),
                    address: lock_addr,
                    kind: HleSymbolKind::Libc,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::ReturnZero,
                },
                HleSymbol {
                    name: "pthread_mutex_unlock".to_string(),
                    address: unlock_addr,
                    kind: HleSymbolKind::Libc,
                    shape: HleSymbolShape::Function,
                    behavior: HleCallBehavior::ReturnZero,
                },
            ],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_0000,
            stack_size: 0x0010_0000,
            tls_base: 0x5010_0000,
            tls_size: 0x1000,
            heap_base: 0x5020_0000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        runtime.guest_mutexes.push(GuestMutex {
            addr: mutex,
            owner: Some(7),
            recursion: 1,
        });
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_pc(unlock_addr);
        cpu.set_reg(0, mutex);
        cpu.set_reg(14, THREAD_RETURN_SENTINEL);
        runtime.guest_threads.push_back(GuestThread {
            id: 7,
            cpu,
            wait: GuestThreadWait::Runnable,
            trace_tail: VecDeque::with_capacity(GUEST_THREAD_TRACE_LEN),
            trace_pc_count: 0,
            trace_mem32_count: 0,
            trace_mem32_deref_count: 0,
            trace_cxx_string_count: 0,
        });

        runtime
            .run_function_with_args(lock_addr, &[mutex], 8)
            .unwrap();

        assert!(runtime.guest_threads.is_empty());
        assert_eq!(runtime.cpu.pc(), CALL_RETURN_SENTINEL);
        assert_eq!(
            runtime
                .guest_mutexes
                .iter()
                .find(|guest_mutex| guest_mutex.addr == mutex)
                .unwrap()
                .owner,
            Some(1)
        );
    }

    #[test]
    fn pthread_mutex_trylock_returns_busy_when_owned_by_another_thread() {
        let trylock_addr = 0x6f00_0000;
        let mutex = 0x6000_0200;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x6f00_0000, 0x1000).unwrap();
        memory.store32(trylock_addr, HLE_TRAP_ARM_INSTR).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![HleSymbol {
                name: "pthread_mutex_trylock".to_string(),
                address: trylock_addr,
                kind: HleSymbolKind::Libc,
                shape: HleSymbolShape::Function,
                behavior: HleCallBehavior::ReturnZero,
            }],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_0000,
            stack_size: 0x0010_0000,
            tls_base: 0x5010_0000,
            tls_size: 0x1000,
            heap_base: 0x5020_0000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        runtime.guest_mutexes.push(GuestMutex {
            addr: mutex,
            owner: Some(7),
            recursion: 1,
        });

        runtime
            .run_function_with_args(trylock_addr, &[mutex], 4)
            .unwrap();

        assert_eq!(runtime.cpu.reg(0), 16);
        assert_eq!(
            runtime
                .guest_mutexes
                .iter()
                .find(|guest_mutex| guest_mutex.addr == mutex)
                .unwrap()
                .owner,
            Some(7)
        );
    }

    #[test]
    fn pthread_once_runs_guest_init_routine_only_once() {
        let once_addr = 0x6f00_0000;
        let init_addr = 0x6f00_0100;
        let once_control = 0x6f00_0200;
        let counter = 0x6f00_0204;
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x6f00_0000, 0x1000).unwrap();
        memory.store32(once_addr, HLE_TRAP_ARM_INSTR).unwrap();
        memory.store32(init_addr, 0xe59f_000c).unwrap(); // ldr r0, [pc, #12]
        memory.store32(init_addr + 4, 0xe590_1000).unwrap(); // ldr r1, [r0]
        memory.store32(init_addr + 8, 0xe281_1001).unwrap(); // add r1, r1, #1
        memory.store32(init_addr + 12, 0xe580_1000).unwrap(); // str r1, [r0]
        memory.store32(init_addr + 16, 0xe12f_ff1e).unwrap(); // bx lr
        memory.store32(init_addr + 20, counter).unwrap();

        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: vec![HleSymbol {
                name: "pthread_once".to_string(),
                address: once_addr,
                kind: HleSymbolKind::Libc,
                shape: HleSymbolShape::Function,
                behavior: HleCallBehavior::ReturnZero,
            }],
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let config = NativeRuntimeConfig {
            stack_base: 0x5000_0000,
            stack_size: 0x0010_0000,
            tls_base: 0x5010_0000,
            tls_size: 0x1000,
            heap_base: 0x5020_0000,
            heap_size: 0x1000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();

        runtime
            .run_function_with_args(once_addr, &[once_control, init_addr], 16)
            .unwrap();
        runtime
            .run_function_with_args(once_addr, &[once_control, init_addr], 16)
            .unwrap();

        assert_eq!(runtime.link.memory.load32(once_control).unwrap(), 1);
        assert_eq!(runtime.link.memory.load32(counter).unwrap(), 1);
        assert_eq!(runtime.cpu.reg(0), 0);
    }

    #[test]
    fn default_runtime_regions_do_not_overlap() {
        assert_eq!(DEFAULT_HEAP_SIZE, 0x0800_0000);
        assert_eq!(DEFAULT_STACK_SIZE, 0x0200_0000);

        let heap_end = DEFAULT_HEAP_BASE
            .checked_add(DEFAULT_HEAP_SIZE as u32)
            .unwrap();
        assert!(heap_end <= DEFAULT_STACK_BASE);

        let stack_end = DEFAULT_STACK_BASE
            .checked_add(DEFAULT_STACK_SIZE as u32)
            .unwrap();
        assert!(stack_end <= DEFAULT_TLS_BASE);

        let tls_end = DEFAULT_TLS_BASE
            .checked_add(DEFAULT_TLS_SIZE as u32)
            .unwrap();
        assert!(
            tls_end <= DEFAULT_HLE_BASE,
            "TLS must stay below linked HLE trap symbols"
        );
    }

    #[test]
    fn runtime_hle_dynamic_cast_uses_guest_typeinfo() {
        let dynamic_cast = 0x7000_0101;
        let class_type_info_vtable = 0x4000_1000;
        let si_class_type_info_vtable = 0x4000_1100;
        let vmi_class_type_info_vtable = 0x4000_1200;
        let base_type = 0x4000_2000;
        let derived_type = 0x4000_2020;
        let object = 0x4000_3000;
        let object_vptr = 0x4000_3080;

        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x4000_0000, 0x4000).unwrap();
        memory
            .store32(base_type, class_type_info_vtable + 8)
            .unwrap();
        memory
            .store32(derived_type, si_class_type_info_vtable + 8)
            .unwrap();
        memory.store32(derived_type + 8, base_type).unwrap();
        memory.store32(object, object_vptr).unwrap();
        memory.store32(object_vptr - 8, 0).unwrap();
        memory.store32(object_vptr - 4, derived_type).unwrap();

        let symbols = vec![
            NativeSymbol {
                name: "__dynamic_cast".to_string(),
                address: dynamic_cast,
                library_name: "libgnustl_shared.so".to_string(),
            },
            NativeSymbol {
                name: "_ZTVN10__cxxabiv117__class_type_infoE".to_string(),
                address: class_type_info_vtable,
                library_name: "libgnustl_shared.so".to_string(),
            },
            NativeSymbol {
                name: "_ZTVN10__cxxabiv120__si_class_type_infoE".to_string(),
                address: si_class_type_info_vtable,
                library_name: "libgnustl_shared.so".to_string(),
            },
            NativeSymbol {
                name: "_ZTVN10__cxxabiv121__vmi_class_type_infoE".to_string(),
                address: vmi_class_type_info_vtable,
                library_name: "libgnustl_shared.so".to_string(),
            },
        ];
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory,
            objects: Vec::new(),
            global_symbols: symbols,
            hle_symbols: Vec::new(),
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

        runtime
            .run_function_with_args(
                dynamic_cast,
                &[object, base_type, derived_type, 0xffff_ffff],
                4,
            )
            .unwrap();

        assert_eq!(runtime.cpu.reg(0), object);
        assert_eq!(runtime.cpu.pc(), CALL_RETURN_SENTINEL);
    }

    #[test]
    fn jni_get_image_data_returns_android_argb_int_array_from_apk() {
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: Vec::new(),
            global_symbols: Vec::new(),
            hle_symbols: Vec::new(),
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
            heap_size: 0x10000,
        };
        let mut runtime = NativeRuntime::new(report, config).unwrap();
        runtime.hle.set_apk_bytes(stored_zip_with_one_file(
            "assets/images/font/default8.png",
            one_by_one_rgba_png(),
        ));
        runtime.jni_methods.push(JniMethod {
            id: 0x1234,
            name: "_getImageData".to_string(),
            sig: "(Ljava/lang/String;)[I".to_string(),
            is_static: false,
        });
        let path = runtime
            .write_guest_c_string("images/font/default8.png")
            .unwrap();
        runtime.cpu.set_reg(2, 0x1234);
        runtime.cpu.set_reg(3, path);

        let array = runtime.call_jni_object_method().unwrap();
        assert_ne!(array, 0);
        runtime.cpu.set_reg(1, array);
        assert_eq!(runtime.jni_array_len().unwrap(), 3);
        runtime.cpu.set_reg(1, array);
        runtime.cpu.set_reg(2, 0);
        let data = runtime.jni_int_array_elements().unwrap();

        assert_eq!(runtime.link.memory.load32(data).unwrap(), 1);
        assert_eq!(runtime.link.memory.load32(data + 4).unwrap(), 1);
        assert_eq!(runtime.link.memory.load32(data + 8).unwrap(), 0x4411_2233);

        let vararg_path = runtime
            .write_guest_c_string("images/font/default8.png")
            .unwrap();
        let vararg = runtime.alloc_guest_zeroed(4, 4).unwrap();
        runtime.store_runtime32(vararg, vararg_path).unwrap();
        runtime.cpu.set_reg(2, 0x1234);
        runtime.cpu.set_reg(3, vararg);
        assert_ne!(runtime.call_jni_object_method().unwrap(), 0);
    }

    #[test]
    fn finds_duplicate_symbol_in_requested_library() {
        let object = |library_name: &str, address: u32| LoadedNativeObject {
            entry_name: format!("lib/armeabi-v7a/{library_name}"),
            library_name: library_name.to_string(),
            load_bias: address & 0xfff0_0000,
            memory_base: address & 0xfff0_0000,
            memory_size: 0x1000,
            entry: address & 0xfff0_0000,
            needed: Vec::new(),
            imports: Vec::new(),
            defined_symbols: vec![NativeSymbol {
                name: "JNI_OnLoad".to_string(),
                address,
                library_name: library_name.to_string(),
            }],
            relocations: Vec::new(),
            relocation_count: 0,
            init: None,
            init_array: None,
            arm_exidx: None,
        };
        let report = NativeLinkReport {
            apk_path: PathBuf::from("test.apk"),
            abi: "armeabi-v7a".to_string(),
            memory: MappedMemory::new(),
            objects: vec![
                object("libfmod.so", 0x700c_cb68),
                object("libminecraftpe.so", 0x7128_d499),
            ],
            global_symbols: vec![NativeSymbol {
                name: "JNI_OnLoad".to_string(),
                address: 0x700c_cb68,
                library_name: "libfmod.so".to_string(),
            }],
            hle_symbols: Vec::new(),
            resolved_imports: Vec::new(),
            unresolved_imports: Vec::new(),
            relocation_errors: Vec::new(),
        };
        let runtime = NativeRuntime {
            link: report,
            cpu: Cpu::new(),
            hle: HleRuntime::new(0, 0x6000_0000, 0x1000),
            cpu_backend: NativeCpuBackendKind::AemuInterpreter,
            runtime_hle_traps: Vec::new(),
            jni_methods: Vec::new(),
            jni_java_vm: 0,
            stack_base: 0x5000_1000,
            stack_size: 0x1000,
            next_thread_stack_top: 0x5000_1000,
            guest_threads: VecDeque::new(),
            guest_mutexes: Vec::new(),
            main_wait: GuestThreadWait::Runnable,
            minecraft_resource_bridge: None,
            minecraft_resource_bridge_active: false,
            trace_native_event_count: 0,
        };

        assert_eq!(runtime.symbol_address("JNI_OnLoad"), Some(0x700c_cb68));
        assert_eq!(
            runtime.symbol_address_in_library("libminecraftpe.so", "JNI_OnLoad"),
            Some(0x7128_d499)
        );
    }

    fn one_by_one_rgba_png() -> &'static [u8] {
        &[
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x78,
            0x9c, 0x63, 0x10, 0x54, 0x32, 0x76, 0x01, 0x00, 0x01, 0x59, 0x00, 0xab, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0x00, 0x00, 0x00, 0x00,
        ]
    }

    fn stored_zip_with_one_file(name: &str, contents: &[u8]) -> Vec<u8> {
        let name_bytes = name.as_bytes();
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0x0403_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, contents.len() as u32);
        push_u32(&mut bytes, contents.len() as u32);
        push_u16(&mut bytes, name_bytes.len() as u16);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(name_bytes);
        bytes.extend_from_slice(contents);

        let central_offset = bytes.len() as u32;
        push_u32(&mut bytes, 0x0201_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, contents.len() as u32);
        push_u32(&mut bytes, contents.len() as u32);
        push_u16(&mut bytes, name_bytes.len() as u16);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        bytes.extend_from_slice(name_bytes);

        let central_size = bytes.len() as u32 - central_offset;
        push_u32(&mut bytes, 0x0605_4b50);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 1);
        push_u32(&mut bytes, central_size);
        push_u32(&mut bytes, central_offset);
        push_u16(&mut bytes, 0);
        bytes
    }

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}
