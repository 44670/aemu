use std::collections::VecDeque;
use std::fmt;

use crate::armv6::{Cpu, Isa, Memory, Trap};
use crate::guest_memory::MappedMemoryError;
use crate::hle_imports::{CreatedPthread, HLE_TRAP_ARM_INSTR, HleError, HleRuntime};
use crate::native_loader::NativeLinkReport;

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
const GUEST_THREAD_STACK_SIZE: u32 = 0x0004_0000;
const GUEST_THREAD_STACK_ALIGN: u32 = 8;
const GUEST_THREAD_SLICE_STEPS: usize = 4096;
const GUEST_THREAD_SERVICE_INTERVAL: usize = 50_000;
const GUEST_THREAD_SERVICE_HLE: &[&str] = &[
    "pthread_create",
    "pthread_cond_signal",
    "pthread_cond_broadcast",
    "pthread_cond_wait",
    "pthread_cond_timedwait",
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
const ANDROID_APP_RUNNING_OFFSET: u32 = 0x6c;
const ANDROID_APP_PENDING_INPUT_QUEUE_OFFSET: u32 = 0x7c;
const ANDROID_APP_PENDING_WINDOW_OFFSET: u32 = 0x80;
const ANDROID_POLL_SOURCE_MAIN: u32 = 1;
const APP_CMD_INIT_WINDOW: u32 = 1;
const APP_CMD_GAINED_FOCUS: u32 = 6;
const APP_CMD_START: u32 = 10;
const APP_CMD_RESUME: u32 = 11;

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
    runtime_hle_traps: Vec<RuntimeHleTrap>,
    jni_methods: Vec<JniMethod>,
    jni_java_vm: u32,
    stack_base: u32,
    stack_size: u32,
    next_thread_stack_top: u32,
    guest_threads: VecDeque<GuestThread>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestThreadWait {
    Runnable,
    Condvar { cond: u32 },
}

impl GuestThreadWait {
    fn is_runnable(self) -> bool {
        matches!(self, Self::Runnable)
    }
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
        let mut hle = HleRuntime::new(errno_addr, config.heap_base, config.heap_size as u32);
        hle.set_apk_path(link.apk_path.clone());

        let runtime_hle_traps = collect_linked_runtime_hle_traps(&link);

        Ok(Self {
            link,
            cpu,
            hle,
            runtime_hle_traps,
            jni_methods: Vec::new(),
            jni_java_vm: 0,
            stack_base: config.stack_base,
            stack_size,
            next_thread_stack_top: config.stack_base,
            guest_threads: VecDeque::new(),
        })
    }

    pub fn step(&mut self) -> Result<NativeRuntimeStep, NativeRuntimeError> {
        let pc_before = self.cpu.pc();
        let isa_before = self.cpu.isa();
        let args_before = core::array::from_fn(|idx| self.cpu.reg(idx));
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

    pub fn run(&mut self, max_steps: usize) -> Result<(), NativeRuntimeError> {
        for _ in 0..max_steps {
            if let NativeRuntimeStep::HleCall { name, args, .. } = self.step()? {
                self.handle_thread_sync_hle(&name, args);
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
        self.store_runtime32(app.wrapping_add(ANDROID_APP_INPUT_QUEUE_OFFSET), 0)?;
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
        for (idx, &value) in args.iter().take(4).enumerate() {
            self.cpu.set_reg(idx, value);
        }
        self.cpu.branch_exchange(address);
        self.cpu.set_reg(14, CALL_RETURN_SENTINEL);
        let mut tail = VecDeque::with_capacity(RUN_FUNCTION_TRACE_LEN);
        let trace_ranges = parse_trace_pc_ranges();
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
                    self.handle_thread_sync_hle(&name, args);
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
                            "HLE function={:#010x} step={} pc={:#010x} name={} r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x}",
                            address,
                            step_idx,
                            hle_address,
                            name,
                            args[0],
                            args[1],
                            args[2],
                            args[3],
                        );
                    }
                    if should_service_threads {
                        self.service_guest_threads(GUEST_THREAD_SLICE_STEPS)?;
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

    fn handle_thread_sync_hle(&mut self, name: &str, args: [u32; 4]) {
        match name {
            "pthread_cond_signal" => self.wake_cond_threads(args[0], false),
            "pthread_cond_broadcast" => self.wake_cond_threads(args[0], true),
            _ => {}
        }
    }

    fn thread_wait_after_hle(&self, name: &str, args: [u32; 4]) -> Option<GuestThreadWait> {
        match name {
            "pthread_cond_wait" if args[0] != 0 => Some(GuestThreadWait::Condvar { cond: args[0] }),
            _ => None,
        }
    }

    fn wake_cond_threads(&mut self, cond: u32, broadcast: bool) {
        if cond == 0 {
            return;
        }
        for thread in &mut self.guest_threads {
            if thread.wait == (GuestThreadWait::Condvar { cond }) {
                thread.wait = GuestThreadWait::Runnable;
                if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                    eprintln!("THREAD wake id={} cond={cond:#010x}", thread.id);
                }
                if !broadcast {
                    break;
                }
            }
        }
    }

    fn run_guest_thread_slice(
        &mut self,
        thread: &mut GuestThread,
        max_steps: usize,
    ) -> Result<bool, NativeRuntimeError> {
        let main_cpu = std::mem::replace(&mut self.cpu, thread.cpu.clone());
        let previous_thread = self.hle.current_pthread();
        self.hle.set_current_pthread(thread.id);
        let mut result = Ok(false);
        for _ in 0..max_steps {
            if self.cpu.pc() == THREAD_RETURN_SENTINEL {
                result = Ok(true);
                break;
            }
            match self.step() {
                Ok(NativeRuntimeStep::GuestInstruction) => {}
                Ok(NativeRuntimeStep::HleCall { name, args, .. }) => {
                    self.handle_thread_sync_hle(&name, args);
                    if let Some(wait) = self.thread_wait_after_hle(&name, args) {
                        thread.wait = wait;
                        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                            eprintln!("THREAD wait id={} {wait:?}", thread.id);
                        }
                        break;
                    }
                }
                Err(err) => {
                    if matches!(
                        err,
                        NativeRuntimeError::Hle {
                            source: HleError::Abort(_),
                            ..
                        }
                    ) {
                        if std::env::var_os("AEMU_TRACE_THREADS").is_some() {
                            eprintln!(
                                "THREAD abort id={} pc={:#010x} {:?} r0={:#010x}",
                                thread.id,
                                self.cpu.pc(),
                                self.cpu.isa(),
                                self.cpu.reg(0),
                            );
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
        self.write_guest_bytes(&bytes)
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
        let value = match method.name.as_str() {
            "getLocale" => "en_US",
            "getDeviceModel" | "getDeviceModelName" => "AEMU",
            "getAndroidVersion" => "19",
            "getExternalStoragePath" => "/sdcard",
            "getPackageName" => "com.mojang.minecraftpe",
            "getFilesDir" | "getAbsolutePath" => "/data/data/com.mojang.minecraftpe/files",
            "getCacheDir" => "/data/data/com.mojang.minecraftpe/cache",
            "getUserInputString" => "",
            "getDeviceId" | "createUUID" => "00000000-0000-0000-0000-000000000000",
            _ if method.sig.ends_with("Ljava/lang/String;") => "",
            _ => return Ok(0),
        };
        self.write_guest_c_string(value)
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
    use crate::native_loader::{
        DEFAULT_HLE_BASE, HleSymbol, LoadedNativeObject, NativeLinkReport, NativeSymbol,
    };

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
                args: [0x5000_0100, 0, 0, 0],
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
        });

        runtime.service_guest_threads(4).unwrap();

        assert_eq!(runtime.guest_threads.len(), 1);
        assert_eq!(
            runtime.guest_threads[0].wait,
            GuestThreadWait::Condvar { cond }
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
        runtime.handle_thread_sync_hle(&name, args);

        assert_eq!(runtime.guest_threads[0].wait, GuestThreadWait::Runnable);
        runtime.service_guest_threads(4).unwrap();
        assert!(runtime.guest_threads.is_empty());
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
    fn finds_duplicate_symbol_in_requested_library() {
        let object = |library_name: &str, address: u32| LoadedNativeObject {
            entry_name: format!("lib/armeabi/{library_name}"),
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
            runtime_hle_traps: Vec::new(),
            jni_methods: Vec::new(),
            jni_java_vm: 0,
            stack_base: 0x5000_1000,
            stack_size: 0x1000,
            next_thread_stack_top: 0x5000_1000,
            guest_threads: VecDeque::new(),
        };

        assert_eq!(runtime.symbol_address("JNI_OnLoad"), Some(0x700c_cb68));
        assert_eq!(
            runtime.symbol_address_in_library("libminecraftpe.so", "JNI_OnLoad"),
            Some(0x7128_d499)
        );
    }
}
