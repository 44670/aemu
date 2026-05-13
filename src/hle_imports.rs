use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;

use crate::armv6::{Cpu, Memory};
use crate::zip_probe::read_zip_entry;

pub const HLE_TRAP_ARM_INSTR: u32 = 0xe7f0_00f0;

const FAKE_FILE_SIZE: u32 = 0x40;
const FAKE_FILE_FD_OFFSET: u32 = 0x0e;
const FIRST_FAKE_FD: u32 = 3;
const AT_HWCAP: u32 = 16;
const AT_HWCAP2: u32 = 26;
const SC_ARG_MAX: u32 = 0;
const SC_CHILD_MAX: u32 = 1;
const SC_CLK_TCK: u32 = 2;
const SC_NGROUPS_MAX: u32 = 3;
const SC_OPEN_MAX: u32 = 4;
const SC_JOB_CONTROL: u32 = 5;
const SC_SAVED_IDS: u32 = 6;
const SC_VERSION: u32 = 7;
const SC_PAGESIZE: u32 = 8;
const SC_NPROCESSORS_CONF: u32 = 9;
const SC_NPROCESSORS_ONLN: u32 = 10;
const SC_PHYS_PAGES: u32 = 11;
const SC_AVPHYS_PAGES: u32 = 12;
const SC_THREAD_KEYS_MAX: u32 = 38;
const SC_THREAD_STACK_MIN: u32 = 39;
const SC_THREAD_THREADS_MAX: u32 = 40;
const SC_THREADS: u32 = 42;
const SC_THREAD_SAFE_FUNCTIONS: u32 = 49;
const HWCAP_SWP: u32 = 1 << 0;
const HWCAP_HALF: u32 = 1 << 1;
const HWCAP_THUMB: u32 = 1 << 2;
const HWCAP_FAST_MULT: u32 = 1 << 4;
const HWCAP_VFP: u32 = 1 << 6;
const HWCAP_EDSP: u32 = 1 << 7;
const HWCAP_NEON: u32 = 1 << 12;
const HWCAP_VFPV3: u32 = 1 << 13;
const HWCAP_TLS: u32 = 1 << 15;
const HWCAP_VFPD32: u32 = 1 << 19;
const ARMV7_NEON_HWCAP: u32 = HWCAP_SWP
    | HWCAP_HALF
    | HWCAP_THUMB
    | HWCAP_FAST_MULT
    | HWCAP_VFP
    | HWCAP_EDSP
    | HWCAP_NEON
    | HWCAP_VFPV3
    | HWCAP_TLS
    | HWCAP_VFPD32;
const EGL_DISPLAY_HANDLE: u32 = 1;
const EGL_CONFIG_HANDLE: u32 = 2;
const EGL_CONTEXT_HANDLE: u32 = 3;
const EGL_SURFACE_HANDLE: u32 = 4;
const EGL_SUCCESS: u32 = 0x3000;
const EGL_BUFFER_SIZE: u32 = 0x3020;
const EGL_ALPHA_SIZE: u32 = 0x3021;
const EGL_BLUE_SIZE: u32 = 0x3022;
const EGL_GREEN_SIZE: u32 = 0x3023;
const EGL_RED_SIZE: u32 = 0x3024;
const EGL_DEPTH_SIZE: u32 = 0x3025;
const EGL_STENCIL_SIZE: u32 = 0x3026;
const EGL_CONFIG_CAVEAT: u32 = 0x3027;
const EGL_CONFIG_ID: u32 = 0x3028;
const EGL_LEVEL: u32 = 0x3029;
const EGL_MAX_PBUFFER_HEIGHT: u32 = 0x302a;
const EGL_MAX_PBUFFER_PIXELS: u32 = 0x302b;
const EGL_MAX_PBUFFER_WIDTH: u32 = 0x302c;
const EGL_NATIVE_RENDERABLE: u32 = 0x302d;
const EGL_NATIVE_VISUAL_ID: u32 = 0x302e;
const EGL_NATIVE_VISUAL_TYPE: u32 = 0x302f;
const EGL_SAMPLES: u32 = 0x3031;
const EGL_SAMPLE_BUFFERS: u32 = 0x3032;
const EGL_SURFACE_TYPE: u32 = 0x3033;
const EGL_TRANSPARENT_TYPE: u32 = 0x3034;
const EGL_NONE: u32 = 0x3038;
const EGL_BIND_TO_TEXTURE_RGB: u32 = 0x3039;
const EGL_BIND_TO_TEXTURE_RGBA: u32 = 0x303a;
const EGL_MIN_SWAP_INTERVAL: u32 = 0x303b;
const EGL_MAX_SWAP_INTERVAL: u32 = 0x303c;
const EGL_LUMINANCE_SIZE: u32 = 0x303d;
const EGL_ALPHA_MASK_SIZE: u32 = 0x303e;
const EGL_COLOR_BUFFER_TYPE: u32 = 0x303f;
const EGL_RENDERABLE_TYPE: u32 = 0x3040;
const EGL_CONFORMANT: u32 = 0x3042;
const EGL_VENDOR: u32 = 0x3053;
const EGL_VERSION: u32 = 0x3054;
const EGL_EXTENSIONS: u32 = 0x3055;
const EGL_HEIGHT: u32 = 0x3056;
const EGL_WIDTH: u32 = 0x3057;
const EGL_CLIENT_APIS: u32 = 0x308d;
const EGL_RGB_BUFFER: u32 = 0x308e;
const EGL_WINDOW_BIT: u32 = 0x0004;
const EGL_PBUFFER_BIT: u32 = 0x0001;
const EGL_OPENGL_ES_BIT: u32 = 0x0001;
const EGL_OPENGL_ES2_BIT: u32 = 0x0004;
const ANDROID_WINDOW_FORMAT_RGBA_8888: u32 = 1;
const ACONFIGURATION_SIZE: u32 = 8;
const AASSET_HANDLE_SIZE: u32 = 0x10;
const FAKE_GEOMETRY_SIZE: u32 = 0x20;
const FAKE_TEXTURE_PAIR_SIZE: u32 = 0x44;
const FAKE_TEXTURE_SIDE: u32 = 256;
const FAKE_TEXTURE_BYTES: u32 = FAKE_TEXTURE_SIDE * FAKE_TEXTURE_SIDE * 4;
const EGL_DEFAULT_SURFACE_WIDTH: u32 = 854;
const EGL_DEFAULT_SURFACE_HEIGHT: u32 = 480;
const GL_VENDOR: u32 = 0x1f00;
const GL_RENDERER: u32 = 0x1f01;
const GL_VERSION: u32 = 0x1f02;
const GL_EXTENSIONS: u32 = 0x1f03;
const GL_MAX_TEXTURE_SIZE: u32 = 0x0d33;
const GL_MAX_TEXTURE_IMAGE_UNITS: u32 = 0x8872;
const GL_MAX_VERTEX_ATTRIBS: u32 = 0x8869;
const GL_COMPILE_STATUS: u32 = 0x8b81;
const GL_LINK_STATUS: u32 = 0x8b82;
const GL_INFO_LOG_LENGTH: u32 = 0x8b84;
const GL_ACTIVE_UNIFORMS: u32 = 0x8b86;
const GL_ACTIVE_UNIFORM_MAX_LENGTH: u32 = 0x8b87;
const GL_ACTIVE_ATTRIBUTES: u32 = 0x8b89;
const GL_ACTIVE_ATTRIBUTE_MAX_LENGTH: u32 = 0x8b8a;
const GL_SHADING_LANGUAGE_VERSION: u32 = 0x8b8c;
const WCTYPE_ALNUM: u32 = 1 << 0;
const WCTYPE_ALPHA: u32 = 1 << 1;
const WCTYPE_BLANK: u32 = 1 << 2;
const WCTYPE_CNTRL: u32 = 1 << 3;
const WCTYPE_DIGIT: u32 = 1 << 4;
const WCTYPE_GRAPH: u32 = 1 << 5;
const WCTYPE_LOWER: u32 = 1 << 6;
const WCTYPE_PRINT: u32 = 1 << 7;
const WCTYPE_PUNCT: u32 = 1 << 8;
const WCTYPE_SPACE: u32 = 1 << 9;
const WCTYPE_UPPER: u32 = 1 << 10;
const WCTYPE_XDIGIT: u32 = 1 << 11;
const CXX_STRING_REP_HEADER_SIZE: u32 = 12;
const CXX_STRING_NPOS: u32 = u32::MAX;
const CXX_STRING_MAX_SIZE: u32 = 0x3fff_fffc;
const FAKE_TIME_BASE_SECS: u64 = 1_600_000_000;
const FAKE_TIME_STEP_NANOS: u64 = 16_666_667;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HleSymbolKind {
    Libc,
    Libm,
    Libdl,
    Liblog,
    Android,
    Egl,
    Gles,
    OpenSl,
    Zlib,
    CxxAbi,
    CxxStd,
    Target,
}

impl fmt::Display for HleSymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Libc => write!(f, "libc"),
            Self::Libm => write!(f, "libm"),
            Self::Libdl => write!(f, "libdl"),
            Self::Liblog => write!(f, "liblog"),
            Self::Android => write!(f, "android"),
            Self::Egl => write!(f, "EGL"),
            Self::Gles => write!(f, "GLES"),
            Self::OpenSl => write!(f, "OpenSL"),
            Self::Zlib => write!(f, "zlib"),
            Self::CxxAbi => write!(f, "cxxabi"),
            Self::CxxStd => write!(f, "c++std"),
            Self::Target => write!(f, "target"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HleImportDescriptor {
    pub kind: HleSymbolKind,
    pub shape: HleSymbolShape,
    pub behavior: HleCallBehavior,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleSymbolShape {
    Function,
    Data { size: u32, init: HleDataInit },
}

impl HleSymbolShape {
    pub fn size(self) -> u32 {
        match self {
            Self::Function => 4,
            Self::Data { size, .. } => size,
        }
    }
}

impl fmt::Display for HleSymbolShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function => write!(f, "fn"),
            Self::Data { size, .. } => write!(f, "data:{size:#x}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleDataInit {
    Zero,
    StackGuard,
    Ctype,
    ToLower,
    ToUpper,
    CxxStringEmptyRep,
    CxxStringTerminal,
    CxxStringMaxSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleCallBehavior {
    Implemented,
    ReturnZero,
    ReturnOne,
    ReturnMinusOneErrno,
    ReturnNull,
    Abort,
}

impl fmt::Display for HleCallBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Implemented => write!(f, "implemented"),
            Self::ReturnZero => write!(f, "stub:0"),
            Self::ReturnOne => write!(f, "stub:1"),
            Self::ReturnMinusOneErrno => write!(f, "stub:-1"),
            Self::ReturnNull => write!(f, "stub:null"),
            Self::Abort => write!(f, "abort"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HleError {
    UnknownSymbol(String),
    Memory(String),
    HeapExhausted { requested: u32 },
    Abort(String),
}

impl fmt::Display for HleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSymbol(name) => write!(f, "unknown HLE symbol: {name}"),
            Self::Memory(err) => write!(f, "{err}"),
            Self::HeapExhausted { requested } => {
                write!(f, "HLE heap exhausted while allocating {requested} bytes")
            }
            Self::Abort(name) => write!(f, "HLE abort requested by {name}"),
        }
    }
}

impl std::error::Error for HleError {}

#[derive(Debug, Clone)]
pub struct HleRuntime {
    errno_addr: u32,
    heap_next: u32,
    heap_end: u32,
    allocations: Vec<HleAllocation>,
    freed: Vec<HleAllocation>,
    apk_path: Option<PathBuf>,
    assets: Vec<AndroidAsset>,
    next_gl_name: u32,
    next_fd: u32,
    files: Vec<FakeFile>,
    virtual_files: Vec<VirtualFile>,
    next_pthread_key: u32,
    pthread_specific: Vec<PthreadSpecific>,
    alooper_events: VecDeque<u32>,
    random_state: u32,
    clock_ns: u64,
    fake_geometry: Option<u32>,
    fake_texture_pair: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct HleAllocation {
    ptr: u32,
    size: u32,
    freeable: bool,
}

#[derive(Debug, Clone)]
struct AndroidAsset {
    handle: u32,
    buffer: u32,
    len: u32,
    closed: bool,
}

#[derive(Debug, Clone)]
struct FakeFile {
    fd: u32,
    kind: FakeFileKind,
    offset: u32,
}

#[derive(Debug, Clone)]
enum FakeFileKind {
    Random,
    Virtual { path: String },
}

#[derive(Debug, Clone)]
struct VirtualFile {
    path: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct PthreadSpecific {
    key: u32,
    value: u32,
}

#[derive(Debug, Clone, Copy)]
struct FakeTime {
    monotonic_secs: u64,
    wall_secs: u64,
    nsecs: u32,
    usecs: u32,
}

impl HleRuntime {
    pub fn new(errno_addr: u32, heap_base: u32, heap_size: u32) -> Self {
        Self {
            errno_addr,
            heap_next: align_up(heap_base, 8).unwrap_or(heap_base),
            heap_end: heap_base.saturating_add(heap_size),
            allocations: Vec::new(),
            freed: Vec::new(),
            apk_path: None,
            assets: Vec::new(),
            next_gl_name: 1,
            next_fd: FIRST_FAKE_FD,
            files: Vec::new(),
            virtual_files: Vec::new(),
            next_pthread_key: 0,
            pthread_specific: Vec::new(),
            alooper_events: VecDeque::new(),
            random_state: 0x1234_5678,
            clock_ns: 0,
            fake_geometry: None,
            fake_texture_pair: None,
        }
    }

    pub fn set_apk_path(&mut self, apk_path: PathBuf) {
        self.apk_path = Some(apk_path);
    }

    pub fn queue_alooper_event(&mut self, source: u32) {
        self.alooper_events.push_back(source);
    }

    pub fn dispatch<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let descriptor =
            describe_hle_import(name).ok_or_else(|| HleError::UnknownSymbol(name.to_string()))?;
        if descriptor.shape != HleSymbolShape::Function {
            return Err(HleError::UnknownSymbol(name.to_string()));
        }

        match name {
            "memcpy" | "__aeabi_memcpy" => self.memcpy(cpu, memory),
            "memmove" | "__aeabi_memmove" => self.memmove(cpu, memory),
            "memset" => self.memset(cpu, memory),
            "__aeabi_memset" => self.aeabi_memset(cpu, memory),
            "memcmp" => self.memcmp(cpu, memory),
            "memchr" => self.memchr(cpu, memory),
            "strlen" => self.strlen(cpu, memory),
            "strcmp" => self.strcmp(cpu, memory),
            "strncmp" => self.strncmp(cpu, memory),
            "strcpy" => self.strcpy(cpu, memory),
            "strncpy" => self.strncpy(cpu, memory),
            "strcat" => self.strcat(cpu, memory),
            "strchr" => self.strchr(cpu, memory),
            "strrchr" => self.strrchr(cpu, memory),
            "isalnum" => Ok(self.return32(cpu, u32::from(ascii_isalnum(cpu.reg(0))))),
            "isspace" => Ok(self.return32(cpu, u32::from(ascii_isspace(cpu.reg(0))))),
            "isupper" => Ok(self.return32(cpu, u32::from(ascii_isupper(cpu.reg(0))))),
            "isxdigit" => Ok(self.return32(cpu, u32::from(ascii_isxdigit(cpu.reg(0))))),
            "tolower" => Ok(self.return32(cpu, ascii_tolower(cpu.reg(0)))),
            "btowc" => Ok(self.return32(cpu, btowc_ascii(cpu.reg(0)))),
            "wctob" => Ok(self.return32(cpu, wctob_ascii(cpu.reg(0)))),
            "towlower" => Ok(self.return32(cpu, ascii_tolower(cpu.reg(0)))),
            "towupper" => Ok(self.return32(cpu, ascii_toupper(cpu.reg(0)))),
            "iswspace" => Ok(self.return32(cpu, u32::from(ascii_isspace(cpu.reg(0))))),
            "wctype" => self.wctype(cpu, memory),
            "iswctype" => Ok(self.return32(cpu, u32::from(ascii_iswctype(cpu.reg(0), cpu.reg(1))))),
            "mbrtowc" => self.mbrtowc(cpu, memory),
            "mbstowcs" => self.mbstowcs(cpu, memory),
            "wcstombs" => self.wcstombs(cpu, memory),
            "wcrtomb" => self.wcrtomb(cpu, memory),
            "wcslen" => self.wcslen(cpu, memory),
            "malloc" => self.malloc_call(cpu, memory),
            "calloc" => self.calloc(cpu, memory),
            "realloc" => self.realloc(cpu, memory),
            "free" => self.free_call(cpu),
            "__errno" => Ok(self.return32(cpu, self.errno_addr)),
            "__aeabi_idiv" => self.aeabi_idiv(cpu),
            "__aeabi_uidiv" => self.aeabi_uidiv(cpu),
            "__aeabi_idivmod" => self.aeabi_idivmod(cpu),
            "__aeabi_uidivmod" => self.aeabi_uidivmod(cpu),
            name if descriptor.kind == HleSymbolKind::Libm => self.libm(name, cpu),
            "getauxval" => Ok(self.return32(cpu, self.getauxval(cpu.reg(0)))),
            "gettimeofday" => self.gettimeofday(cpu, memory),
            "clock_gettime" => self.clock_gettime(cpu, memory),
            "time" => self.time(cpu, memory),
            "sysconf" => self.sysconf(cpu, memory),
            "pthread_self" => Ok(self.return32(cpu, 1)),
            "pthread_equal" => Ok(self.return32(cpu, u32::from(cpu.reg(0) == cpu.reg(1)))),
            "pthread_key_create" => self.pthread_key_create(cpu, memory),
            "pthread_key_delete" => self.pthread_key_delete(cpu),
            "pthread_getspecific" => Ok(self.return32(cpu, self.pthread_getspecific(cpu.reg(0)))),
            "pthread_setspecific" => self.pthread_setspecific(cpu),
            "ALooper_pollAll" | "ALooper_pollOnce" => self.alooper_poll(cpu, memory),
            "ALooper_prepare" | "ALooper_forThread" | "ALooper_acquire" => {
                Ok(self.return32(cpu, 1))
            }
            "ALooper_addFd" => Ok(self.return32(cpu, 1)),
            "ALooper_removeFd" | "ALooper_wake" | "ALooper_release" => Ok(self.return32(cpu, 0)),
            "fopen" => self.fopen_call(cpu, memory),
            "fdopen" => self.fdopen_call(cpu, memory),
            "fclose" => self.fclose_call(cpu, memory),
            "close" => self.close_call(cpu),
            "open" => self.open_call(cpu, memory),
            "pipe" => self.pipe_call(cpu, memory),
            "read" => self.read_call(cpu, memory),
            "fread" => self.fread_call(cpu, memory),
            "write" => self.write_call(cpu, memory),
            "fwrite" => self.fwrite_call(cpu, memory),
            "pthread_create" => self.pthread_create(cpu, memory),
            "__cxa_guard_acquire" => self.cxa_guard_acquire(cpu, memory),
            "__cxa_guard_release" => self.cxa_guard_release(cpu, memory),
            "__cxa_guard_abort" => Ok(self.return32(cpu, 0)),
            "_ZNSs14_M_replace_auxEjjjc" => self.libstdcxx_string_replace_aux(cpu, memory),
            "_ZNSsC1Ev" | "_ZNSsC2Ev" => self.libstdcxx_string_default_ctor(cpu, memory),
            "_ZNSsC1ERKSs" | "_ZNSsC2ERKSs" => self.libstdcxx_string_copy_ctor(cpu, memory),
            "_ZNSsC1EPKcRKSaIcE" | "_ZNSsC2EPKcRKSaIcE" => {
                self.libstdcxx_string_cstr_ctor(cpu, memory)
            }
            "_ZNSsC1EPKcjRKSaIcE" | "_ZNSsC2EPKcjRKSaIcE" => {
                self.libstdcxx_string_ptr_len_ctor(cpu, memory)
            }
            "_ZNSsC1EjcRKSaIcE" | "_ZNSsC2EjcRKSaIcE" => {
                self.libstdcxx_string_fill_ctor(cpu, memory)
            }
            "_ZNSsC1ERKSsjj" | "_ZNSsC2ERKSsjj" => self.libstdcxx_string_substr_ctor(cpu, memory),
            "_ZNSsD1Ev" | "_ZNSsD2Ev" => Ok(self.return32(cpu, 0)),
            "_ZNSs4_Rep10_M_destroyERKSaIcE" => Ok(self.return32(cpu, 0)),
            "_ZNSs4_Rep9_S_createEjjRKSaIcE" => self.libstdcxx_string_rep_create(cpu, memory),
            "_ZNSs12_S_constructEjcRKSaIcE" => self.libstdcxx_string_construct_fill(cpu, memory),
            "_ZNSs4swapERSs" => self.libstdcxx_string_swap(cpu, memory),
            "_ZNKSs7compareEPKc" => self.libstdcxx_string_compare_cstr(cpu, memory),
            "_ZNKSs7compareERKSs" => self.libstdcxx_string_compare_string(cpu, memory),
            "_ZNKSs4findEPKcjj" => self.libstdcxx_string_find_cstr_len(cpu, memory),
            "_ZNKSs4findEcj" => self.libstdcxx_string_find_char(cpu, memory),
            "_ZNKSs5rfindEPKcjj" => self.libstdcxx_string_rfind_cstr_len(cpu, memory),
            "_ZNKSs5rfindEcj" => self.libstdcxx_string_rfind_char(cpu, memory),
            "_ZNKSs12find_last_ofEPKcjj" => self.libstdcxx_string_find_last_of(cpu, memory),
            "_ZNKSs13find_first_ofEPKcjj" => self.libstdcxx_string_find_first_of(cpu, memory),
            "_ZNKSs16find_last_not_ofEPKcjj" => self.libstdcxx_string_find_last_not_of(cpu, memory),
            "_ZNKSs17find_first_not_ofEPKcjj" => {
                self.libstdcxx_string_find_first_not_of(cpu, memory)
            }
            "_ZNSs6appendEPKcj" => self.libstdcxx_string_append_cstr_len(cpu, memory),
            "_ZNSs6appendERKSs" => self.libstdcxx_string_append_string(cpu, memory),
            "_ZNSs6appendEjc" => self.libstdcxx_string_append_fill(cpu, memory),
            "_ZNSs6assignEPKcj" => self.libstdcxx_string_assign_cstr_len(cpu, memory),
            "_ZNSs6assignERKSs" | "_ZNSsaSERKSs" => {
                self.libstdcxx_string_assign_string(cpu, memory)
            }
            "_ZNSsaSEPKc" => self.libstdcxx_string_assign_cstr(cpu, memory),
            "_ZNSs6resizeEjc" => self.libstdcxx_string_resize_fill(cpu, memory),
            "_ZNSs7reserveEj" => self.libstdcxx_string_reserve(cpu, memory),
            "_ZNSs9_M_mutateEjjj" => self.libstdcxx_string_mutate(cpu, memory),
            "_ZNSs12_M_leak_hardEv" => self.libstdcxx_string_leak_hard(cpu, memory),
            "_ZNSs15_M_replace_safeEjjPKcj" | "_ZNSs7replaceEjjPKcj" => {
                self.libstdcxx_string_replace_safe(cpu, memory)
            }
            "_ZNSs6insertEjPKcj" => self.libstdcxx_string_insert_cstr_len(cpu, memory),
            "_ZNSs5eraseEN9__gnu_cxx17__normal_iteratorIPcSsEES2_" => {
                self.libstdcxx_string_erase_range(cpu, memory)
            }
            "_ZN8WebTokenC1ERKS_" | "_ZN8WebTokenC2ERKS_" => {
                self.minecraft_webtoken_copy_ctor(cpu, memory)
            }
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation" => {
                self.minecraft_texture_group_get_texture_pair(cpu, memory)
            }
            "_ZN13GeometryGroup11getGeometryERKSs" | "_ZN13GeometryGroup14tryGetGeometryERKSs" => {
                self.minecraft_geometry_group_get_geometry(cpu, memory)
            }
            "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E"
            | "_ZN9UIControl18_resolvePostCreateEv" => self.minecraft_ui_control_resolve_noop(cpu),
            name if name.starts_with("AConfiguration_") => {
                self.android_configuration(name, cpu, memory)
            }
            name if name.starts_with("AAsset") => self.android_asset(name, cpu, memory),
            name if name.starts_with("gl") => self.gles(name, cpu, memory),
            name if name.starts_with("egl") => self.egl(name, cpu, memory),
            _ => self.dispatch_stub(name, descriptor.behavior, cpu, memory),
        }
    }

    fn dispatch_stub<M: Memory>(
        &mut self,
        name: &str,
        behavior: HleCallBehavior,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match behavior {
            HleCallBehavior::Implemented | HleCallBehavior::ReturnZero => Ok(self.return32(cpu, 0)),
            HleCallBehavior::ReturnOne => Ok(self.return32(cpu, 1)),
            HleCallBehavior::ReturnMinusOneErrno => {
                self.set_errno(memory, 38)?;
                Ok(self.return32(cpu, u32::MAX))
            }
            HleCallBehavior::ReturnNull => Ok(self.return32(cpu, 0)),
            HleCallBehavior::Abort => Err(HleError::Abort(name.to_string())),
        }
    }

    fn getauxval(&self, key: u32) -> u32 {
        match key {
            AT_HWCAP => ARMV7_NEON_HWCAP,
            AT_HWCAP2 => 0,
            _ => 0,
        }
    }

    fn sysconf<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let value = match cpu.reg(0) {
            SC_ARG_MAX => Some(131_072),
            SC_CHILD_MAX => Some(999),
            SC_CLK_TCK => Some(100),
            SC_NGROUPS_MAX => Some(32),
            SC_OPEN_MAX => Some(1024),
            SC_JOB_CONTROL | SC_SAVED_IDS => Some(1),
            SC_VERSION | SC_THREADS | SC_THREAD_SAFE_FUNCTIONS => Some(200_809),
            SC_PAGESIZE => Some(4096),
            SC_NPROCESSORS_CONF | SC_NPROCESSORS_ONLN => Some(1),
            SC_PHYS_PAGES => Some(256 * 1024),
            SC_AVPHYS_PAGES => Some(128 * 1024),
            SC_THREAD_KEYS_MAX => Some(128),
            SC_THREAD_STACK_MIN => Some(16 * 1024),
            SC_THREAD_THREADS_MAX => Some(64),
            _ => None,
        };
        if let Some(value) = value {
            Ok(self.return32(cpu, value))
        } else {
            self.set_errno(memory, 22)?;
            Ok(self.return32(cpu, u32::MAX))
        }
    }

    fn memcpy<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let len = cpu.reg(2);
        for idx in 0..len {
            let byte = load8(memory, src.wrapping_add(idx))?;
            store8(memory, dst.wrapping_add(idx), byte)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn memmove<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let len = cpu.reg(2);
        let mut tmp = Vec::with_capacity(len as usize);
        for idx in 0..len {
            tmp.push(load8(memory, src.wrapping_add(idx))?);
        }
        for (idx, byte) in tmp.into_iter().enumerate() {
            store8(memory, dst.wrapping_add(idx as u32), byte)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn memset<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let value = cpu.reg(1) as u8;
        let len = cpu.reg(2);
        for idx in 0..len {
            store8(memory, dst.wrapping_add(idx), value)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn aeabi_memset<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let len = cpu.reg(1);
        let value = cpu.reg(2) as u8;
        for idx in 0..len {
            store8(memory, dst.wrapping_add(idx), value)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn memcmp<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let a = cpu.reg(0);
        let b = cpu.reg(1);
        let len = cpu.reg(2);
        for idx in 0..len {
            let av = load8(memory, a.wrapping_add(idx))?;
            let bv = load8(memory, b.wrapping_add(idx))?;
            if av != bv {
                return Ok(self.return32(cpu, i32_to_u32(i32::from(av) - i32::from(bv))));
            }
        }
        Ok(self.return32(cpu, 0))
    }

    fn memchr<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let value = cpu.reg(1) as u8;
        let len = cpu.reg(2);
        for idx in 0..len {
            if load8(memory, ptr.wrapping_add(idx))? == value {
                return Ok(self.return32(cpu, ptr.wrapping_add(idx)));
            }
        }
        Ok(self.return32(cpu, 0))
    }

    fn strlen<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let len = strlen(memory, cpu.reg(0))?;
        Ok(self.return32(cpu, len))
    }

    fn strcmp<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let result = strcmp(memory, cpu.reg(0), cpu.reg(1), u32::MAX)?;
        Ok(self.return32(cpu, i32_to_u32(result)))
    }

    fn strncmp<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let result = strcmp(memory, cpu.reg(0), cpu.reg(1), cpu.reg(2))?;
        Ok(self.return32(cpu, i32_to_u32(result)))
    }

    fn strcpy<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let mut idx = 0;
        loop {
            let byte = load8(memory, src.wrapping_add(idx))?;
            store8(memory, dst.wrapping_add(idx), byte)?;
            idx += 1;
            if byte == 0 {
                break;
            }
        }
        Ok(self.return32(cpu, dst))
    }

    fn strncpy<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let len = cpu.reg(2);
        let mut nul_seen = false;
        for idx in 0..len {
            let byte = if nul_seen {
                0
            } else {
                let byte = load8(memory, src.wrapping_add(idx))?;
                nul_seen = byte == 0;
                byte
            };
            store8(memory, dst.wrapping_add(idx), byte)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn strcat<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let mut off = strlen(memory, dst)?;
        let mut idx = 0;
        loop {
            let byte = load8(memory, src.wrapping_add(idx))?;
            store8(memory, dst.wrapping_add(off), byte)?;
            off += 1;
            idx += 1;
            if byte == 0 {
                break;
            }
        }
        Ok(self.return32(cpu, dst))
    }

    fn strchr<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let needle = cpu.reg(1) as u8;
        let mut off = 0;
        loop {
            let byte = load8(memory, ptr.wrapping_add(off))?;
            if byte == needle {
                return Ok(self.return32(cpu, ptr.wrapping_add(off)));
            }
            if byte == 0 {
                return Ok(self.return32(cpu, 0));
            }
            off += 1;
        }
    }

    fn strrchr<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let needle = cpu.reg(1) as u8;
        let mut found = 0;
        let mut off = 0;
        loop {
            let byte = load8(memory, ptr.wrapping_add(off))?;
            if byte == needle {
                found = ptr.wrapping_add(off);
            }
            if byte == 0 {
                break;
            }
            off += 1;
        }
        Ok(self.return32(cpu, found))
    }

    fn wctype<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let name = load_c_string(memory, cpu.reg(0), 32)?;
        Ok(self.return32(cpu, ascii_wctype_descriptor(&name).unwrap_or(0)))
    }

    fn mbrtowc<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let out = cpu.reg(0);
        let src = cpu.reg(1);
        let len = cpu.reg(2);
        if src == 0 {
            return Ok(self.return32(cpu, 0));
        }
        if len == 0 {
            return Ok(self.return32(cpu, u32::MAX - 1));
        }
        let byte = load8(memory, src)?;
        if out != 0 {
            store32(memory, out, u32::from(byte))?;
        }
        Ok(self.return32(cpu, if byte == 0 { 0 } else { 1 }))
    }

    fn mbstowcs<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let max = cpu.reg(2);
        let mut count = 0u32;
        loop {
            let byte = load8(memory, src.wrapping_add(count))?;
            if byte == 0 {
                if dst != 0 && count < max {
                    store32(memory, dst.wrapping_add(count * 4), 0)?;
                }
                return Ok(self.return32(cpu, count));
            }
            if dst != 0 && count < max {
                store32(memory, dst.wrapping_add(count * 4), u32::from(byte))?;
            }
            count = count.wrapping_add(1);
            if dst != 0 && count >= max {
                return Ok(self.return32(cpu, count));
            }
        }
    }

    fn wcstombs<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let src = cpu.reg(1);
        let max = cpu.reg(2);
        let mut count = 0u32;
        loop {
            let value = load32(memory, src.wrapping_add(count * 4))?;
            if value == 0 {
                if dst != 0 && count < max {
                    store8(memory, dst.wrapping_add(count), 0)?;
                }
                return Ok(self.return32(cpu, count));
            }
            if dst != 0 && count < max {
                store8(memory, dst.wrapping_add(count), wctob_ascii(value) as u8)?;
            }
            count = count.wrapping_add(1);
            if dst != 0 && count >= max {
                return Ok(self.return32(cpu, count));
            }
        }
    }

    fn wcrtomb<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let dst = cpu.reg(0);
        let value = cpu.reg(1);
        if dst == 0 {
            return Ok(self.return32(cpu, 1));
        }
        let byte = wctob_ascii(value);
        if byte == u32::MAX {
            self.set_errno(memory, 84)?;
            return Ok(self.return32(cpu, u32::MAX));
        }
        store8(memory, dst, byte as u8)?;
        Ok(self.return32(cpu, 1))
    }

    fn wcslen<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let mut len = 0u32;
        while load32(memory, ptr.wrapping_add(len * 4))? != 0 {
            len = len.wrapping_add(1);
        }
        Ok(self.return32(cpu, len))
    }

    fn malloc_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        if std::env::var_os("AEMU_TRACE_HLE_ALLOC").is_some() {
            let saved_lr = load32(memory, cpu.reg(13).wrapping_add(4)).unwrap_or(0);
            eprintln!(
                "HLE malloc request size={:#x} r4={:#x} lr={:#010x} caller_lr={saved_lr:#010x}",
                cpu.reg(0),
                cpu.reg(4),
                cpu.reg(14)
            );
        }
        let ptr = self.alloc_guest(cpu.reg(0), 8)?;
        Ok(self.return32(cpu, ptr))
    }

    fn calloc<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let count = cpu.reg(0);
        let size = cpu.reg(1);
        let Some(total) = count.checked_mul(size) else {
            return Ok(self.return32(cpu, 0));
        };
        let ptr = self.alloc_guest(total, 8)?;
        for idx in 0..total {
            store8(memory, ptr.wrapping_add(idx), 0)?;
        }
        Ok(self.return32(cpu, ptr))
    }

    fn realloc<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let size = cpu.reg(1);
        if ptr == 0 {
            let new_ptr = self.alloc_guest(size, 8)?;
            Ok(self.return32(cpu, new_ptr))
        } else if size == 0 {
            self.free_ptr(ptr);
            Ok(self.return32(cpu, 0))
        } else {
            if let Some(old_size) = self.allocation_size(ptr) {
                if size <= old_size {
                    return Ok(self.return32(cpu, ptr));
                }
            }
            let new_ptr = self.alloc_guest(size, 8)?;
            let old_size = self.allocation_size(ptr).unwrap_or(0);
            for idx in 0..old_size.min(size) {
                let byte = load8(memory, ptr.wrapping_add(idx))?;
                store8(memory, new_ptr.wrapping_add(idx), byte)?;
            }
            self.free_ptr(ptr);
            Ok(self.return32(cpu, new_ptr))
        }
    }

    fn free_call(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        self.free_ptr(cpu.reg(0));
        Ok(self.return32(cpu, 0))
    }

    fn aeabi_idiv(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0) as i32;
        let rhs = cpu.reg(1) as i32;
        let result = if rhs == 0 { 0 } else { lhs.wrapping_div(rhs) };
        Ok(self.return32(cpu, result as u32))
    }

    fn aeabi_uidiv(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0);
        let rhs = cpu.reg(1);
        let result = if rhs == 0 { 0 } else { lhs / rhs };
        Ok(self.return32(cpu, result))
    }

    fn aeabi_idivmod(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0) as i32;
        let rhs = cpu.reg(1) as i32;
        let (q, r) = if rhs == 0 {
            (0, 0)
        } else {
            (lhs.wrapping_div(rhs), lhs.wrapping_rem(rhs))
        };
        cpu.set_reg(1, r as u32);
        Ok(self.return32(cpu, q as u32))
    }

    fn aeabi_uidivmod(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0);
        let rhs = cpu.reg(1);
        let (q, r) = if rhs == 0 {
            (0, 0)
        } else {
            (lhs / rhs, lhs % rhs)
        };
        cpu.set_reg(1, r);
        Ok(self.return32(cpu, q))
    }

    fn gettimeofday<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let tv = cpu.reg(0);
        let now = self.advance_clock();
        if tv != 0 {
            store32(memory, tv, now.wall_secs as u32)?;
            store32(memory, tv.wrapping_add(4), now.usecs)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn clock_gettime<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ts = cpu.reg(1);
        let now = self.advance_clock();
        if ts != 0 {
            store32(memory, ts, now.monotonic_secs as u32)?;
            store32(memory, ts.wrapping_add(4), now.nsecs)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn time<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let out = cpu.reg(0);
        let now = self.advance_clock();
        if out != 0 {
            store32(memory, out, now.wall_secs as u32)?;
        }
        Ok(self.return32(cpu, now.wall_secs as u32))
    }

    fn advance_clock(&mut self) -> FakeTime {
        self.clock_ns = self.clock_ns.saturating_add(FAKE_TIME_STEP_NANOS);
        let monotonic_secs = self.clock_ns / 1_000_000_000;
        let nsecs = (self.clock_ns % 1_000_000_000) as u32;
        FakeTime {
            monotonic_secs,
            wall_secs: FAKE_TIME_BASE_SECS + monotonic_secs,
            nsecs,
            usecs: nsecs / 1_000,
        }
    }

    fn fopen_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let path = load_c_string(memory, cpu.reg(0), 256)?;
        let mode = load_c_string(memory, cpu.reg(1), 16).unwrap_or_else(|_| String::new());
        if is_random_device_path(&path) {
            let ptr = self.open_fake_stream(memory, FakeFileKind::Random)?;
            trace_hle_file(format_args!(
                "fopen path={path:?} mode={mode:?} -> stream {ptr:#010x}"
            ));
            return Ok(self.return32(cpu, ptr));
        }
        if is_virtual_storage_path(&path) {
            let wants_create = mode.contains('w') || mode.contains('a');
            if wants_create {
                self.create_virtual_file(&path, mode.contains('a'));
            }
            if wants_create || self.virtual_file_index(&path).is_some() {
                let ptr =
                    self.open_fake_stream(memory, FakeFileKind::Virtual { path: path.clone() })?;
                trace_hle_file(format_args!(
                    "fopen path={path:?} mode={mode:?} -> stream {ptr:#010x}"
                ));
                return Ok(self.return32(cpu, ptr));
            }
        }
        self.set_errno(memory, 2)?;
        trace_hle_file(format_args!("fopen path={path:?} mode={mode:?} -> null"));
        Ok(self.return32(cpu, 0))
    }

    fn fdopen_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fd = cpu.reg(0);
        if fd == u32::MAX || self.fake_file_index(fd).is_none() {
            self.set_errno(memory, 9)?;
            return Ok(self.return32(cpu, 0));
        }
        let ptr = self.alloc_fake_file(memory, fd)?;
        Ok(self.return32(cpu, ptr))
    }

    fn fclose_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let stream = cpu.reg(0);
        if stream != 0 {
            if let Ok(fd) = self.fake_file_fd(memory, stream) {
                self.close_fd(fd);
            }
        }
        Ok(self.return32(cpu, 0))
    }

    fn close_call(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        self.close_fd(cpu.reg(0));
        Ok(self.return32(cpu, 0))
    }

    fn open_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let path = load_c_string(memory, cpu.reg(0), 256)?;
        let flags = cpu.reg(1);
        let mode = cpu.reg(2);
        if is_random_device_path(&path) {
            let fd = self.open_fake_fd(FakeFileKind::Random);
            trace_hle_file(format_args!(
                "open path={path:?} flags={flags:#x} mode={mode:#x} -> fd {fd}"
            ));
            return Ok(self.return32(cpu, fd));
        }
        if is_virtual_storage_path(&path) {
            let wants_create = flags & 0x40 != 0 || flags & 0x200 != 0 || flags & 0x400 != 0;
            if wants_create {
                self.create_virtual_file(&path, false);
            }
            if wants_create || self.virtual_file_index(&path).is_some() {
                let fd = self.open_fake_fd(FakeFileKind::Virtual { path: path.clone() });
                trace_hle_file(format_args!(
                    "open path={path:?} flags={flags:#x} mode={mode:#x} -> fd {fd}"
                ));
                return Ok(self.return32(cpu, fd));
            }
        }
        self.set_errno(memory, 2)?;
        trace_hle_file(format_args!(
            "open path={path:?} flags={flags:#x} mode={mode:#x} -> -1"
        ));
        Ok(self.return32(cpu, u32::MAX))
    }

    fn pipe_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fds = cpu.reg(0);
        if fds == 0 {
            self.set_errno(memory, 14)?;
            return Ok(self.return32(cpu, u32::MAX));
        }
        let read_fd = self.alloc_fd();
        let write_fd = self.alloc_fd();
        store32(memory, fds, read_fd)?;
        store32(memory, fds.wrapping_add(4), write_fd)?;
        Ok(self.return32(cpu, 0))
    }

    fn pthread_create<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let thread_out = cpu.reg(0);
        let arg = cpu.reg(3);
        if thread_out != 0 {
            store32(memory, thread_out, self.alloc_fd())?;
        }

        // Android's native_app_glue waits for the created thread to mark
        // android_app.running at offset 0x6c before ANativeActivity_onCreate
        // returns. The launch probe drives android_main explicitly afterward.
        if arg != 0 {
            store32(memory, arg.wrapping_add(0x6c), 1)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn pthread_key_create<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let key_out = cpu.reg(0);
        if key_out == 0 {
            return Ok(self.return32(cpu, 22));
        }
        let key = self.next_pthread_key;
        self.next_pthread_key = self.next_pthread_key.wrapping_add(1);
        store32(memory, key_out, key)?;
        Ok(self.return32(cpu, 0))
    }

    fn pthread_key_delete(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let key = cpu.reg(0);
        self.pthread_specific.retain(|entry| entry.key != key);
        Ok(self.return32(cpu, 0))
    }

    fn pthread_getspecific(&self, key: u32) -> u32 {
        self.pthread_specific
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value)
            .unwrap_or(0)
    }

    fn pthread_setspecific(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let key = cpu.reg(0);
        let value = cpu.reg(1);
        if let Some(entry) = self
            .pthread_specific
            .iter_mut()
            .find(|entry| entry.key == key)
        {
            entry.value = value;
        } else if value != 0 {
            self.pthread_specific.push(PthreadSpecific { key, value });
        }
        Ok(self.return32(cpu, 0))
    }

    fn alooper_poll<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let out_fd = cpu.reg(1);
        let out_events = cpu.reg(2);
        let out_data = cpu.reg(3);
        let source = self.alooper_events.pop_front();
        if out_fd != 0 {
            store32(memory, out_fd, u32::MAX)?;
        }
        if out_events != 0 {
            store32(memory, out_events, 0)?;
        }
        if out_data != 0 {
            store32(memory, out_data, source.unwrap_or(0))?;
        }
        if std::env::var_os("AEMU_TRACE_ALOOPER").is_some() {
            if let Some(source) = source {
                let id = load32(memory, source).unwrap_or(u32::MAX);
                let app = load32(memory, source.wrapping_add(4)).unwrap_or(0);
                let process = load32(memory, source.wrapping_add(8)).unwrap_or(0);
                let command = load32(memory, source.wrapping_add(12)).unwrap_or(u32::MAX);
                eprintln!(
                    "ALOOPER source={source:#010x} id={id} app={app:#010x} process={process:#010x} command={command}"
                );
            } else {
                eprintln!("ALOOPER no-event");
            }
        }
        if source.is_some() {
            Ok(self.return32(cpu, 1))
        } else {
            Ok(self.return32(cpu, u32::MAX))
        }
    }

    fn read_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fd = cpu.reg(0);
        let buf = cpu.reg(1);
        let count = cpu.reg(2);
        if fd < FIRST_FAKE_FD || buf == 0 {
            self.set_errno(memory, 9)?;
            return Ok(self.return32(cpu, u32::MAX));
        }
        let read = self.read_fake_fd(memory, fd, buf, count)?;
        Ok(self.return32(cpu, read))
    }

    fn fread_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let size = cpu.reg(1);
        let count = cpu.reg(2);
        let stream = cpu.reg(3);
        if ptr == 0 || stream == 0 || size == 0 {
            return Ok(self.return32(cpu, 0));
        }
        let Some(total) = size.checked_mul(count) else {
            return Ok(self.return32(cpu, 0));
        };
        let Ok(fd) = self.fake_file_fd(memory, stream) else {
            return Ok(self.return32(cpu, 0));
        };
        let read = self.read_fake_fd(memory, fd, ptr, total)?;
        Ok(self.return32(cpu, read / size))
    }

    fn write_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fd = cpu.reg(0);
        let buf = cpu.reg(1);
        let count = cpu.reg(2);
        if fd < FIRST_FAKE_FD || buf == 0 {
            self.set_errno(memory, 9)?;
            return Ok(self.return32(cpu, u32::MAX));
        }
        let written = self.write_fake_fd(memory, fd, buf, count)?;
        Ok(self.return32(cpu, written))
    }

    fn fwrite_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let size = cpu.reg(1);
        let count = cpu.reg(2);
        let stream = cpu.reg(3);
        if ptr == 0 || stream == 0 || size == 0 {
            return Ok(self.return32(cpu, 0));
        }
        let Some(total) = size.checked_mul(count) else {
            return Ok(self.return32(cpu, 0));
        };
        let Ok(fd) = self.fake_file_fd(memory, stream) else {
            return Ok(self.return32(cpu, 0));
        };
        let written = self.write_fake_fd(memory, fd, ptr, total)?;
        Ok(self.return32(cpu, written / size))
    }

    fn alloc_fd(&mut self) -> u32 {
        let fd = self.next_fd;
        self.next_fd = self.next_fd.wrapping_add(1).max(FIRST_FAKE_FD);
        fd
    }

    fn open_fake_stream<M: Memory>(
        &mut self,
        memory: &mut M,
        kind: FakeFileKind,
    ) -> Result<u32, HleError> {
        let fd = self.open_fake_fd(kind);
        self.alloc_fake_file(memory, fd)
    }

    fn open_fake_fd(&mut self, kind: FakeFileKind) -> u32 {
        let fd = self.alloc_fd();
        self.files.push(FakeFile {
            fd,
            kind,
            offset: 0,
        });
        fd
    }

    fn alloc_fake_file<M: Memory>(&mut self, memory: &mut M, fd: u32) -> Result<u32, HleError> {
        let ptr = self.alloc(FAKE_FILE_SIZE, 8)?;
        for offset in 0..FAKE_FILE_SIZE {
            store8(memory, ptr.wrapping_add(offset), 0)?;
        }
        store16(memory, ptr.wrapping_add(FAKE_FILE_FD_OFFSET), fd as u16)?;
        Ok(ptr)
    }

    fn fake_file_fd<M: Memory>(&mut self, memory: &mut M, stream: u32) -> Result<u32, HleError> {
        Ok(u32::from(load16(
            memory,
            stream.wrapping_add(FAKE_FILE_FD_OFFSET),
        )?))
    }

    fn fake_file_index(&self, fd: u32) -> Option<usize> {
        self.files.iter().position(|file| file.fd == fd)
    }

    fn virtual_file_index(&self, path: &str) -> Option<usize> {
        self.virtual_files.iter().position(|file| file.path == path)
    }

    fn create_virtual_file(&mut self, path: &str, append: bool) {
        if let Some(idx) = self.virtual_file_index(path) {
            if !append {
                self.virtual_files[idx].data.clear();
            }
        } else {
            self.virtual_files.push(VirtualFile {
                path: path.to_string(),
                data: Vec::new(),
            });
        }
    }

    fn close_fd(&mut self, fd: u32) {
        self.files.retain(|file| file.fd != fd);
    }

    fn read_fake_fd<M: Memory>(
        &mut self,
        memory: &mut M,
        fd: u32,
        buf: u32,
        count: u32,
    ) -> Result<u32, HleError> {
        let Some(file_idx) = self.fake_file_index(fd) else {
            return Ok(0);
        };
        match self.files[file_idx].kind.clone() {
            FakeFileKind::Random => {
                self.fill_random(memory, buf, count)?;
                self.files[file_idx].offset = self.files[file_idx].offset.wrapping_add(count);
                Ok(count)
            }
            FakeFileKind::Virtual { path } => {
                let Some(virtual_idx) = self.virtual_file_index(&path) else {
                    return Ok(0);
                };
                let offset = self.files[file_idx].offset as usize;
                let data = &self.virtual_files[virtual_idx].data;
                let available = data.len().saturating_sub(offset);
                let read = available.min(count as usize);
                for idx in 0..read {
                    store8(memory, buf.wrapping_add(idx as u32), data[offset + idx])?;
                }
                self.files[file_idx].offset = self.files[file_idx].offset.wrapping_add(read as u32);
                Ok(read as u32)
            }
        }
    }

    fn write_fake_fd<M: Memory>(
        &mut self,
        memory: &mut M,
        fd: u32,
        buf: u32,
        count: u32,
    ) -> Result<u32, HleError> {
        let Some(file_idx) = self.fake_file_index(fd) else {
            return Ok(count);
        };
        match self.files[file_idx].kind.clone() {
            FakeFileKind::Random => Ok(count),
            FakeFileKind::Virtual { path } => {
                let virtual_idx = if let Some(idx) = self.virtual_file_index(&path) {
                    idx
                } else {
                    self.virtual_files.push(VirtualFile {
                        path: path.clone(),
                        data: Vec::new(),
                    });
                    self.virtual_files.len() - 1
                };
                let offset = self.files[file_idx].offset as usize;
                let end = offset.saturating_add(count as usize);
                if self.virtual_files[virtual_idx].data.len() < end {
                    self.virtual_files[virtual_idx].data.resize(end, 0);
                }
                for idx in 0..count {
                    self.virtual_files[virtual_idx].data[offset + idx as usize] =
                        load8(memory, buf.wrapping_add(idx))?;
                }
                self.files[file_idx].offset = self.files[file_idx].offset.wrapping_add(count);
                Ok(count)
            }
        }
    }

    fn libstdcxx_string_replace_aux<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let pos = cpu.reg(1);
        let erase_len = cpu.reg(2);
        let insert_len = cpu.reg(3);
        let ch = load8(memory, cpu.reg(13))?;
        let data = load32(memory, string)?;
        let old_len = load32(memory, data.wrapping_sub(12))?;
        let old_capacity = load32(memory, data.wrapping_sub(8))?;
        let old_refcount = load32(memory, data.wrapping_sub(4))? as i32;

        let pos = pos.min(old_len);
        let erase_len = erase_len.min(old_len.saturating_sub(pos));
        let suffix_pos = pos.wrapping_add(erase_len);
        let new_len = old_len
            .checked_sub(erase_len)
            .and_then(|len| len.checked_add(insert_len))
            .ok_or(HleError::HeapExhausted {
                requested: insert_len,
            })?;

        let mut bytes = Vec::with_capacity(new_len as usize);
        for idx in 0..pos {
            bytes.push(load8(memory, data.wrapping_add(idx))?);
        }
        bytes.extend(std::iter::repeat(ch).take(insert_len as usize));
        for idx in suffix_pos..old_len {
            bytes.push(load8(memory, data.wrapping_add(idx))?);
        }

        let reuse = old_capacity >= new_len && old_refcount <= 0;
        let out_data = if reuse {
            data
        } else {
            let doubled = old_capacity.saturating_mul(2);
            let capacity = new_len.max(doubled).max(15);
            let allocation = self.alloc(
                capacity.checked_add(13).ok_or(HleError::HeapExhausted {
                    requested: capacity,
                })?,
                4,
            )?;
            store32(memory, allocation, new_len)?;
            store32(memory, allocation.wrapping_add(4), capacity)?;
            store32(memory, allocation.wrapping_add(8), 0)?;
            let out_data = allocation.wrapping_add(12);
            store32(memory, string, out_data)?;
            out_data
        };

        for (idx, byte) in bytes.into_iter().enumerate() {
            store8(memory, out_data.wrapping_add(idx as u32), byte)?;
        }
        store8(memory, out_data.wrapping_add(new_len), 0)?;
        store32(memory, out_data.wrapping_sub(12), new_len)?;
        if reuse {
            store32(memory, out_data.wrapping_sub(4), 0)?;
        }

        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_default_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        self.store_cxx_string_bytes(memory, string, &[], 0)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_copy_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let bytes = load_cxx_string_bytes(memory, cpu.reg(1))?;
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_cstr_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let ptr = cpu.reg(1);
        let len = if ptr == 0 { 0 } else { strlen(memory, ptr)? };
        let bytes = load_bytes(memory, ptr, len)?;
        self.store_cxx_string_bytes(memory, string, &bytes, len)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_ptr_len_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let ptr = cpu.reg(1);
        let len = cpu.reg(2);
        let bytes = load_bytes(memory, ptr, len)?;
        self.store_cxx_string_bytes(memory, string, &bytes, len)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_fill_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let len = cpu.reg(1);
        let ch = cpu.reg(2) as u8;
        let bytes = vec![ch; len as usize];
        self.store_cxx_string_bytes(memory, string, &bytes, len)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_substr_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let source = load_cxx_string_bytes(memory, cpu.reg(1))?;
        let pos = (cpu.reg(2) as usize).min(source.len());
        let requested = cpu.reg(3);
        let available = source.len().saturating_sub(pos);
        let len = if requested == CXX_STRING_NPOS {
            available
        } else {
            (requested as usize).min(available)
        };
        self.store_cxx_string_bytes(memory, string, &source[pos..pos + len], len as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_rep_create<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let capacity = cpu.reg(0).max(1);
        let rep = self.alloc_cxx_string_rep(memory, &[], capacity)?;
        Ok(self.return32(cpu, rep))
    }

    fn libstdcxx_string_construct_fill<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let len = cpu.reg(0);
        let ch = cpu.reg(1) as u8;
        let bytes = vec![ch; len as usize];
        let data = self.alloc_cxx_string_data(memory, &bytes, len)?;
        Ok(self.return32(cpu, data))
    }

    fn libstdcxx_string_swap<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let lhs = cpu.reg(0);
        let rhs = cpu.reg(1);
        let lhs_data = load32(memory, lhs)?;
        let rhs_data = load32(memory, rhs)?;
        store32(memory, lhs, rhs_data)?;
        store32(memory, rhs, lhs_data)?;
        Ok(self.return32(cpu, lhs))
    }

    fn libstdcxx_string_compare_cstr<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let lhs = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let rhs_len = strlen(memory, cpu.reg(1))?;
        let rhs = load_bytes(memory, cpu.reg(1), rhs_len)?;
        Ok(self.return32(cpu, i32_to_u32(compare_bytes(&lhs, &rhs))))
    }

    fn libstdcxx_string_compare_string<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let lhs = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let rhs = load_cxx_string_bytes(memory, cpu.reg(1))?;
        Ok(self.return32(cpu, i32_to_u32(compare_bytes(&lhs, &rhs))))
    }

    fn libstdcxx_string_find_cstr_len<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needle = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, find_subslice(&haystack, &needle, cpu.reg(2))))
    }

    fn libstdcxx_string_find_char<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        Ok(self.return32(cpu, find_byte(&haystack, cpu.reg(1) as u8, cpu.reg(2))))
    }

    fn libstdcxx_string_rfind_cstr_len<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needle = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, rfind_subslice(&haystack, &needle, cpu.reg(2))))
    }

    fn libstdcxx_string_rfind_char<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        Ok(self.return32(cpu, rfind_byte(&haystack, cpu.reg(1) as u8, cpu.reg(2))))
    }

    fn libstdcxx_string_find_last_of<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needles = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, find_last_of(&haystack, &needles, cpu.reg(2), true)))
    }

    fn libstdcxx_string_find_first_of<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needles = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, find_first_of(&haystack, &needles, cpu.reg(2), true)))
    }

    fn libstdcxx_string_find_last_not_of<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needles = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, find_last_of(&haystack, &needles, cpu.reg(2), false)))
    }

    fn libstdcxx_string_find_first_not_of<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let haystack = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let needles = load_bytes(memory, cpu.reg(1), cpu.reg(3))?;
        Ok(self.return32(cpu, find_first_of(&haystack, &needles, cpu.reg(2), false)))
    }

    fn libstdcxx_string_append_cstr_len<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        bytes.extend(load_bytes(memory, cpu.reg(1), cpu.reg(2))?);
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_append_string<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        bytes.extend(load_cxx_string_bytes(memory, cpu.reg(1))?);
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_append_fill<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        let count = cpu.reg(1);
        bytes.extend(std::iter::repeat(cpu.reg(2) as u8).take(count as usize));
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_assign_cstr_len<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let bytes = load_bytes(memory, cpu.reg(1), cpu.reg(2))?;
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_assign_string<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let bytes = load_cxx_string_bytes(memory, cpu.reg(1))?;
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_assign_cstr<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let len = strlen(memory, cpu.reg(1))?;
        let bytes = load_bytes(memory, cpu.reg(1), len)?;
        self.store_cxx_string_bytes(memory, string, &bytes, len)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_resize_fill<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let new_len = cpu.reg(1) as usize;
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        bytes.resize(new_len, cpu.reg(2) as u8);
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_reserve<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let requested = cpu.reg(1);
        let bytes = load_cxx_string_bytes(memory, string)?;
        if cxx_string_capacity(memory, string)? < requested {
            self.store_cxx_string_bytes(memory, string, &bytes, requested)?;
        }
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_mutate<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let pos = cpu.reg(1) as usize;
        let erase_len = cpu.reg(2) as usize;
        let insert_len = cpu.reg(3) as usize;
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        let pos = pos.min(bytes.len());
        let end = pos.saturating_add(erase_len).min(bytes.len());
        bytes.splice(pos..end, std::iter::repeat(0).take(insert_len));
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_leak_hard<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let data = load32(memory, string)?;
        if data != 0 {
            store32(memory, data.wrapping_sub(4), u32::MAX)?;
        }
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_replace_safe<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let pos = cpu.reg(1) as usize;
        let erase_len = cpu.reg(2) as usize;
        let replacement_len = load32(memory, cpu.reg(13))?;
        let replacement = load_bytes(memory, cpu.reg(3), replacement_len)?;
        self.replace_cxx_string_range(memory, string, pos, erase_len, &replacement)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_insert_cstr_len<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let pos = cpu.reg(1) as usize;
        let replacement = load_bytes(memory, cpu.reg(2), cpu.reg(3))?;
        self.replace_cxx_string_range(memory, string, pos, 0, &replacement)?;
        Ok(self.return32(cpu, string))
    }

    fn libstdcxx_string_erase_range<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let string = cpu.reg(0);
        let data = load32(memory, string)?;
        let len = cxx_string_len_from_data(memory, data)? as usize;
        let first = cpu.reg(1).saturating_sub(data).min(len as u32) as usize;
        let last = cpu.reg(2).saturating_sub(data).min(len as u32) as usize;
        let erase_len = last.saturating_sub(first);
        let new_data = self.replace_cxx_string_range(memory, string, first, erase_len, &[])?;
        Ok(self.return32(cpu, new_data.wrapping_add(first as u32)))
    }

    fn minecraft_webtoken_copy_ctor<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let dest = cpu.reg(0);
        let source = cpu.reg(1);

        if source == 0 {
            self.store_empty_webtoken(memory, dest)?;
        } else {
            let issuer = load_cxx_string_bytes(memory, source)?;
            self.store_cxx_string_bytes(memory, dest, &issuer, 0)?;
            self.store_json_value_copy(memory, dest.wrapping_add(0x08), source.wrapping_add(0x08))?;
            let subject = load_cxx_string_bytes(memory, source.wrapping_add(0x18))?;
            self.store_cxx_string_bytes(memory, dest.wrapping_add(0x18), &subject, 0)?;
            self.store_json_value_copy(memory, dest.wrapping_add(0x20), source.wrapping_add(0x20))?;
            let signature = load_cxx_string_bytes(memory, source.wrapping_add(0x30))?;
            self.store_cxx_string_bytes(memory, dest.wrapping_add(0x30), &signature, 0)?;
        }

        Ok(self.return32(cpu, dest))
    }

    fn minecraft_texture_group_get_texture_pair<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let pair = match self.fake_texture_pair {
            Some(pair) => pair,
            None => {
                let pair = self.alloc(FAKE_TEXTURE_PAIR_SIZE, 4)?;
                for offset in 0..FAKE_TEXTURE_PAIR_SIZE {
                    store8(memory, pair.wrapping_add(offset), 0)?;
                }
                store32(memory, pair.wrapping_add(0x38), FAKE_TEXTURE_SIDE)?;
                store32(memory, pair.wrapping_add(0x3c), FAKE_TEXTURE_SIDE)?;
                self.store_cxx_string_bytes(
                    memory,
                    pair.wrapping_add(0x40),
                    &[],
                    FAKE_TEXTURE_BYTES,
                )?;
                self.fake_texture_pair = Some(pair);
                pair
            }
        };
        Ok(self.return32(cpu, pair))
    }

    fn minecraft_geometry_group_get_geometry<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let out = cpu.reg(0);
        let geometry = match self.fake_geometry {
            Some(geometry) => geometry,
            None => {
                let geometry = self.alloc(FAKE_GEOMETRY_SIZE, 4)?;
                for offset in 0..FAKE_GEOMETRY_SIZE {
                    store8(memory, geometry.wrapping_add(offset), 0)?;
                }
                self.fake_geometry = Some(geometry);
                geometry
            }
        };

        store32(memory, out, 0)?;
        store32(memory, out.wrapping_add(4), geometry)?;
        Ok(self.return32(cpu, out))
    }

    fn minecraft_ui_control_resolve_noop(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        Ok(self.return32(cpu, 0))
    }

    fn store_empty_webtoken<M: Memory>(
        &mut self,
        memory: &mut M,
        dest: u32,
    ) -> Result<(), HleError> {
        self.store_cxx_string_bytes(memory, dest, &[], 0)?;
        store_json_null(memory, dest.wrapping_add(0x08))?;
        self.store_cxx_string_bytes(memory, dest.wrapping_add(0x18), &[], 0)?;
        store_json_null(memory, dest.wrapping_add(0x20))?;
        self.store_cxx_string_bytes(memory, dest.wrapping_add(0x30), &[], 0)?;
        Ok(())
    }

    fn store_json_value_copy<M: Memory>(
        &mut self,
        memory: &mut M,
        dest: u32,
        source: u32,
    ) -> Result<(), HleError> {
        if source < 0x1000 {
            return store_json_null(memory, dest);
        }

        let value_type = load8(memory, source.wrapping_add(0x08))?;
        match value_type {
            0 | 1 | 2 | 3 | 5 => {
                for offset in 0..16 {
                    let byte = load8(memory, source.wrapping_add(offset))?;
                    store8(memory, dest.wrapping_add(offset), byte)?;
                }
                Ok(())
            }
            4 => {
                let ptr = load32(memory, source)?;
                let bytes = if ptr == 0 {
                    Vec::new()
                } else {
                    let len = strlen(memory, ptr)?;
                    load_bytes(memory, ptr, len)?
                };
                let allocation = self.alloc((bytes.len() as u32).saturating_add(1), 1)?;
                for (idx, byte) in bytes.iter().copied().enumerate() {
                    store8(memory, allocation.wrapping_add(idx as u32), byte)?;
                }
                store8(memory, allocation.wrapping_add(bytes.len() as u32), 0)?;
                store32(memory, dest, allocation)?;
                store32(memory, dest.wrapping_add(4), 0)?;
                store16(memory, dest.wrapping_add(8), u16::from(value_type) | 0x100)?;
                store16(memory, dest.wrapping_add(10), 0)?;
                store32(memory, dest.wrapping_add(12), 0)?;
                Ok(())
            }
            _ => store_json_null(memory, dest),
        }
    }

    fn replace_cxx_string_range<M: Memory>(
        &mut self,
        memory: &mut M,
        string: u32,
        pos: usize,
        erase_len: usize,
        replacement: &[u8],
    ) -> Result<u32, HleError> {
        let mut bytes = load_cxx_string_bytes(memory, string)?;
        let pos = pos.min(bytes.len());
        let end = pos.saturating_add(erase_len).min(bytes.len());
        bytes.splice(pos..end, replacement.iter().copied());
        self.store_cxx_string_bytes(memory, string, &bytes, bytes.len() as u32)
    }

    fn store_cxx_string_bytes<M: Memory>(
        &mut self,
        memory: &mut M,
        string: u32,
        bytes: &[u8],
        min_capacity: u32,
    ) -> Result<u32, HleError> {
        let data = self.alloc_cxx_string_data(memory, bytes, min_capacity)?;
        store32(memory, string, data)?;
        Ok(data)
    }

    fn alloc_cxx_string_data<M: Memory>(
        &mut self,
        memory: &mut M,
        bytes: &[u8],
        min_capacity: u32,
    ) -> Result<u32, HleError> {
        Ok(self
            .alloc_cxx_string_rep(memory, bytes, min_capacity)?
            .wrapping_add(CXX_STRING_REP_HEADER_SIZE))
    }

    fn alloc_cxx_string_rep<M: Memory>(
        &mut self,
        memory: &mut M,
        bytes: &[u8],
        min_capacity: u32,
    ) -> Result<u32, HleError> {
        let len = bytes.len() as u32;
        let capacity = min_capacity.max(len);
        let allocation = self.alloc(
            capacity.checked_add(CXX_STRING_REP_HEADER_SIZE + 1).ok_or(
                HleError::HeapExhausted {
                    requested: capacity,
                },
            )?,
            4,
        )?;
        store32(memory, allocation, len)?;
        store32(memory, allocation.wrapping_add(4), capacity)?;
        store32(memory, allocation.wrapping_add(8), 0)?;
        let data = allocation.wrapping_add(CXX_STRING_REP_HEADER_SIZE);
        for (idx, byte) in bytes.iter().copied().enumerate() {
            store8(memory, data.wrapping_add(idx as u32), byte)?;
        }
        store8(memory, data.wrapping_add(len), 0)?;
        Ok(allocation)
    }

    fn fill_random<M: Memory>(
        &mut self,
        memory: &mut M,
        ptr: u32,
        len: u32,
    ) -> Result<(), HleError> {
        for idx in 0..len {
            self.random_state = self
                .random_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            store8(
                memory,
                ptr.wrapping_add(idx),
                (self.random_state >> 24) as u8,
            )?;
        }
        Ok(())
    }

    fn cxa_guard_acquire<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let guard = cpu.reg(0);
        let initialized = guard != 0 && load8(memory, guard)? != 0;
        Ok(self.return32(cpu, u32::from(!initialized)))
    }

    fn cxa_guard_release<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let guard = cpu.reg(0);
        if guard != 0 {
            store8(memory, guard, 1)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn libm(&mut self, name: &str, cpu: &mut Cpu) -> Result<(), HleError> {
        match name {
            "sinf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).sin())),
            "cosf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).cos())),
            "tanf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).tan())),
            "sqrtf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).sqrt())),
            "floorf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).floor())),
            "ceilf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).ceil())),
            "fabsf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).abs())),
            "roundf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).round())),
            "truncf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).trunc())),
            "powf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).powf(f32_arg(cpu, 1)))),
            "fmaxf" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).max(f32_arg(cpu, 1)))),
            "exp2f" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).exp2())),
            "nearbyintf" | "rint" => Ok(self.return_f32(cpu, f32_arg(cpu, 0).round())),
            "sin" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).sin())),
            "cos" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).cos())),
            "tan" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).tan())),
            "sqrt" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).sqrt())),
            "floor" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).floor())),
            "ceil" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).ceil())),
            "fabs" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).abs())),
            "pow" => Ok(self.return_f64(cpu, f64_arg(cpu, 0).powf(f64_arg(cpu, 2)))),
            _ => Ok(self.return32(cpu, 0)),
        }
    }

    fn android_configuration<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "AConfiguration_new" => {
                let ptr = self.alloc(ACONFIGURATION_SIZE, 4)?;
                for offset in 0..ACONFIGURATION_SIZE {
                    store8(memory, ptr.wrapping_add(offset), 0)?;
                }
                Ok(self.return32(cpu, ptr))
            }
            "AConfiguration_getLanguage" => {
                write_android_locale_code(memory, cpu.reg(1), b"en")?;
                Ok(self.return32(cpu, 0))
            }
            "AConfiguration_getCountry" => {
                write_android_locale_code(memory, cpu.reg(1), b"US")?;
                Ok(self.return32(cpu, 0))
            }
            "AConfiguration_fromAssetManager" | "AConfiguration_delete" => {
                Ok(self.return32(cpu, 0))
            }
            _ => self.dispatch_stub(name, HleCallBehavior::ReturnZero, cpu, memory),
        }
    }

    fn android_asset<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "AAssetManager_open" => {
                let path = load_c_string(memory, cpu.reg(1), 1024)?;
                let Some(apk_path) = self.apk_path.as_ref() else {
                    trace_android_asset(format_args!(
                        "AAssetManager_open path={path:?} failed: no APK path"
                    ));
                    return Ok(self.return32(cpu, 0));
                };
                let mut last_err = None;
                let mut loaded = None;
                for entry_name in android_asset_entry_candidates(&path) {
                    match read_zip_entry(apk_path, &entry_name) {
                        Ok(bytes) => {
                            loaded = Some((entry_name, bytes));
                            break;
                        }
                        Err(err) => last_err = Some(err.to_string()),
                    }
                }
                let Some((entry_name, bytes)) = loaded else {
                    trace_android_asset(format_args!(
                        "AAssetManager_open path={path:?} failed: {}",
                        last_err.unwrap_or_else(|| "empty asset path".to_string())
                    ));
                    return Ok(self.return32(cpu, 0));
                };
                let len = u32::try_from(bytes.len()).map_err(|_| HleError::HeapExhausted {
                    requested: u32::MAX,
                })?;
                let buffer = self.alloc(len.max(1), 1)?;
                for (idx, byte) in bytes.into_iter().enumerate() {
                    store8(memory, buffer.wrapping_add(idx as u32), byte)?;
                }
                let handle = self.alloc(AASSET_HANDLE_SIZE, 4)?;
                for offset in 0..AASSET_HANDLE_SIZE {
                    store8(memory, handle.wrapping_add(offset), 0)?;
                }
                store32(memory, handle, buffer)?;
                store32(memory, handle.wrapping_add(4), len)?;
                self.assets.push(AndroidAsset {
                    handle,
                    buffer,
                    len,
                    closed: false,
                });
                trace_android_asset(format_args!(
                    "AAssetManager_open path={path:?} entry={entry_name:?} len={len} handle={handle:#010x} buffer={buffer:#010x}"
                ));
                Ok(self.return32(cpu, handle))
            }
            "AAsset_getLength" => {
                let len = self
                    .assets
                    .iter()
                    .find(|asset| asset.handle == cpu.reg(0) && !asset.closed)
                    .map_or(0, |asset| asset.len);
                Ok(self.return32(cpu, len))
            }
            "AAsset_getBuffer" => {
                let buffer = self
                    .assets
                    .iter()
                    .find(|asset| asset.handle == cpu.reg(0) && !asset.closed)
                    .map_or(0, |asset| asset.buffer);
                Ok(self.return32(cpu, buffer))
            }
            "AAsset_close" => {
                if let Some(asset) = self
                    .assets
                    .iter_mut()
                    .find(|asset| asset.handle == cpu.reg(0))
                {
                    asset.closed = true;
                }
                Ok(self.return32(cpu, 0))
            }
            _ => self.dispatch_stub(name, HleCallBehavior::ReturnZero, cpu, memory),
        }
    }

    fn egl<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "eglGetDisplay" => Ok(self.return32(cpu, EGL_DISPLAY_HANDLE)),
            "eglCreateContext" => Ok(self.return32(cpu, EGL_CONTEXT_HANDLE)),
            "eglCreateWindowSurface" | "eglCreatePbufferSurface" => {
                Ok(self.return32(cpu, EGL_SURFACE_HANDLE))
            }
            "eglInitialize" => {
                if cpu.reg(1) != 0 {
                    store32(memory, cpu.reg(1), 1)?;
                }
                if cpu.reg(2) != 0 {
                    store32(memory, cpu.reg(2), 4)?;
                }
                Ok(self.return32(cpu, 1))
            }
            "eglChooseConfig" => {
                let configs = cpu.reg(2);
                let config_size = cpu.reg(3);
                let num_config_ptr = load32(memory, cpu.reg(13)).unwrap_or(0);
                if configs != 0 && config_size != 0 {
                    store32(memory, configs, EGL_CONFIG_HANDLE)?;
                }
                if num_config_ptr != 0 {
                    store32(memory, num_config_ptr, 1)?;
                }
                Ok(self.return32(cpu, 1))
            }
            "eglGetConfigAttrib" => {
                let attr = cpu.reg(2);
                let value_ptr = cpu.reg(3);
                if value_ptr != 0 {
                    store32(memory, value_ptr, egl_config_attrib(attr))?;
                }
                Ok(self.return32(cpu, 1))
            }
            "eglQuerySurface" => {
                let attr = cpu.reg(2);
                let value_ptr = cpu.reg(3);
                if value_ptr != 0 {
                    store32(memory, value_ptr, egl_surface_attrib(attr))?;
                }
                Ok(self.return32(cpu, 1))
            }
            "eglQueryString" => {
                let Some(value) = egl_query_string(cpu.reg(1)) else {
                    return Ok(self.return32(cpu, 0));
                };
                let ptr = self.alloc_c_string(memory, value)?;
                Ok(self.return32(cpu, ptr))
            }
            "eglGetCurrentDisplay" => Ok(self.return32(cpu, EGL_DISPLAY_HANDLE)),
            "eglGetCurrentContext" => Ok(self.return32(cpu, EGL_CONTEXT_HANDLE)),
            "eglGetCurrentSurface" => Ok(self.return32(cpu, EGL_SURFACE_HANDLE)),
            "eglGetError" => Ok(self.return32(cpu, EGL_SUCCESS)),
            "eglGetProcAddress" => Ok(self.return32(cpu, 0)),
            "eglBindAPI" | "eglMakeCurrent" | "eglSwapBuffers" | "eglSwapInterval"
            | "eglDestroySurface" | "eglDestroyContext" | "eglTerminate" | "eglReleaseThread"
            | "eglSurfaceAttrib" | "eglWaitGL" | "eglWaitNative" => Ok(self.return32(cpu, 1)),
            _ => Ok(self.return32(cpu, 1)),
        }
    }

    fn gles<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "glCreateProgram" | "glCreateShader" => {
                let value = self.next_gl_name;
                self.next_gl_name = self.next_gl_name.wrapping_add(1).max(1);
                Ok(self.return32(cpu, value))
            }
            "glGetString" => {
                let Some(value) = gl_query_string(cpu.reg(0)) else {
                    return Ok(self.return32(cpu, 0));
                };
                let ptr = self.alloc_c_string(memory, value)?;
                Ok(self.return32(cpu, ptr))
            }
            "glGetError" => Ok(self.return32(cpu, 0)),
            "glGetProgramiv" => {
                let value = gl_program_iv(cpu.reg(1));
                if cpu.reg(2) != 0 {
                    store32(memory, cpu.reg(2), value)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetShaderiv" => {
                let value = gl_shader_iv(cpu.reg(1));
                if cpu.reg(2) != 0 {
                    store32(memory, cpu.reg(2), value)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetIntegerv" => {
                let value = gl_integer(cpu.reg(0));
                if cpu.reg(1) != 0 {
                    store32(memory, cpu.reg(1), value)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetActiveUniform" | "glGetActiveAttrib" => {
                let length_ptr = cpu.reg(3);
                let size_ptr = load32(memory, cpu.reg(13)).unwrap_or(0);
                let type_ptr = load32(memory, cpu.reg(13).wrapping_add(4)).unwrap_or(0);
                let name_ptr = load32(memory, cpu.reg(13).wrapping_add(8)).unwrap_or(0);
                if length_ptr != 0 {
                    store32(memory, length_ptr, 0)?;
                }
                if size_ptr != 0 {
                    store32(memory, size_ptr, 0)?;
                }
                if type_ptr != 0 {
                    store32(memory, type_ptr, 0)?;
                }
                if name_ptr != 0 && cpu.reg(2) != 0 {
                    store8(memory, name_ptr, 0)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetProgramInfoLog" | "glGetShaderInfoLog" => {
                let length_ptr = cpu.reg(2);
                let info_log_ptr = cpu.reg(3);
                if length_ptr != 0 {
                    store32(memory, length_ptr, 0)?;
                }
                if info_log_ptr != 0 {
                    store8(memory, info_log_ptr, 0)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetAttribLocation" | "glGetUniformLocation" => Ok(self.return32(cpu, 0)),
            "glIsTexture" => Ok(self.return32(cpu, 0)),
            _ => Ok(self.return32(cpu, 0)),
        }
    }

    pub(crate) fn alloc(&mut self, size: u32, align: u32) -> Result<u32, HleError> {
        self.alloc_bump(size, align, false)
    }

    fn alloc_guest(&mut self, size: u32, align: u32) -> Result<u32, HleError> {
        let size = size.max(1);
        if let Some(ptr) = self.alloc_freed(size, align)? {
            return Ok(ptr);
        }
        self.alloc_bump(size, align, true)
    }

    fn alloc_bump(&mut self, size: u32, align: u32, freeable: bool) -> Result<u32, HleError> {
        let size = size.max(1);
        let start =
            align_up(self.heap_next, align).ok_or(HleError::HeapExhausted { requested: size })?;
        let end = start
            .checked_add(size)
            .ok_or(HleError::HeapExhausted { requested: size })?;
        if end > self.heap_end {
            return Err(HleError::HeapExhausted { requested: size });
        }
        self.heap_next = end;
        self.allocations.push(HleAllocation {
            ptr: start,
            size,
            freeable,
        });
        if std::env::var_os("AEMU_TRACE_HLE_ALLOC").is_some() {
            eprintln!("HLE alloc size={size:#x} align={align:#x} -> {start:#010x}");
        }
        Ok(start)
    }

    fn alloc_freed(&mut self, size: u32, align: u32) -> Result<Option<u32>, HleError> {
        let Some((idx, start, end)) =
            self.freed.iter().enumerate().find_map(|(idx, allocation)| {
                let start = align_up(allocation.ptr, align)?;
                let end = start.checked_add(size)?;
                let block_end = allocation.ptr.checked_add(allocation.size)?;
                (end <= block_end).then_some((idx, start, end))
            })
        else {
            return Ok(None);
        };
        let allocation = self.freed.remove(idx);
        let block_end = allocation
            .ptr
            .checked_add(allocation.size)
            .ok_or(HleError::HeapExhausted { requested: size })?;
        if allocation.ptr < start {
            self.insert_free_block(HleAllocation {
                ptr: allocation.ptr,
                size: start - allocation.ptr,
                freeable: true,
            });
        }
        if end < block_end {
            self.insert_free_block(HleAllocation {
                ptr: end,
                size: block_end - end,
                freeable: true,
            });
        }
        self.allocations.push(HleAllocation {
            ptr: start,
            size,
            freeable: true,
        });
        if std::env::var_os("AEMU_TRACE_HLE_ALLOC").is_some() {
            eprintln!("HLE alloc reused size={size:#x} align={align:#x} -> {start:#010x}");
        }
        Ok(Some(start))
    }

    fn free_ptr(&mut self, ptr: u32) {
        if ptr == 0 {
            return;
        }
        if let Some(idx) = self
            .allocations
            .iter()
            .rposition(|allocation| allocation.ptr == ptr && allocation.freeable)
        {
            let allocation = self.allocations.remove(idx);
            self.insert_free_block(allocation);
        }
    }

    fn allocation_size(&self, ptr: u32) -> Option<u32> {
        self.allocations
            .iter()
            .rev()
            .find(|allocation| allocation.ptr == ptr && allocation.freeable)
            .map(|allocation| allocation.size)
    }

    fn insert_free_block(&mut self, allocation: HleAllocation) {
        if allocation.size == 0 {
            return;
        }
        self.freed.push(allocation);
        self.freed.sort_by_key(|allocation| allocation.ptr);

        let mut coalesced: Vec<HleAllocation> = Vec::with_capacity(self.freed.len());
        for block in self.freed.drain(..) {
            if let Some(last) = coalesced.last_mut() {
                let last_end = last.ptr.saturating_add(last.size);
                if last_end >= block.ptr {
                    let block_end = block.ptr.saturating_add(block.size);
                    last.size = block_end.saturating_sub(last.ptr).max(last.size);
                    continue;
                }
            }
            coalesced.push(block);
        }
        self.freed = coalesced;
    }

    fn alloc_c_string<M: Memory>(&mut self, memory: &mut M, value: &str) -> Result<u32, HleError> {
        let ptr = self.alloc(value.len() as u32 + 1, 1)?;
        for (idx, byte) in value.bytes().enumerate() {
            store8(memory, ptr.wrapping_add(idx as u32), byte)?;
        }
        store8(memory, ptr.wrapping_add(value.len() as u32), 0)?;
        Ok(ptr)
    }

    fn set_errno<M: Memory>(&mut self, memory: &mut M, errno: u32) -> Result<(), HleError> {
        if self.errno_addr != 0 {
            store32(memory, self.errno_addr, errno)?;
        }
        Ok(())
    }

    fn return32(&mut self, cpu: &mut Cpu, value: u32) {
        cpu.set_reg(0, value);
        cpu.branch_exchange(cpu.reg(14));
    }

    fn return_f32(&mut self, cpu: &mut Cpu, value: f32) {
        self.return32(cpu, value.to_bits());
    }

    fn return_f64(&mut self, cpu: &mut Cpu, value: f64) {
        let bits = value.to_bits();
        cpu.set_reg(1, (bits >> 32) as u32);
        self.return32(cpu, bits as u32);
    }
}

pub fn describe_hle_import(name: &str) -> Option<HleImportDescriptor> {
    let kind = classify_hle_symbol(name)?;
    Some(HleImportDescriptor {
        kind,
        shape: hle_shape(name),
        behavior: hle_behavior(name, kind),
    })
}

pub fn initialize_hle_symbol<M: Memory>(
    memory: &mut M,
    descriptor: HleImportDescriptor,
    address: u32,
) -> Result<(), HleError> {
    match descriptor.shape {
        HleSymbolShape::Function => store32(memory, address, HLE_TRAP_ARM_INSTR),
        HleSymbolShape::Data { size, init } => {
            for idx in 0..size {
                store8(memory, address.wrapping_add(idx), 0)?;
            }
            match init {
                HleDataInit::Zero => {}
                HleDataInit::StackGuard => store32(memory, address, 0x00c0_ffee)?,
                HleDataInit::Ctype => init_ctype(memory, address, false)?,
                HleDataInit::ToLower => init_case_table(memory, address, false)?,
                HleDataInit::ToUpper => init_case_table(memory, address, true)?,
                HleDataInit::CxxStringEmptyRep => init_cxx_string_empty_rep(memory, address)?,
                HleDataInit::CxxStringTerminal => store8(memory, address, 0)?,
                HleDataInit::CxxStringMaxSize => store32(memory, address, CXX_STRING_MAX_SIZE)?,
            }
            Ok(())
        }
    }
}

fn classify_hle_symbol(name: &str) -> Option<HleSymbolKind> {
    if name.starts_with("gl") && name.as_bytes().get(2).is_some_and(u8::is_ascii_uppercase) {
        return Some(HleSymbolKind::Gles);
    }
    if name.starts_with("egl") && name.as_bytes().get(3).is_some_and(u8::is_ascii_uppercase) {
        return Some(HleSymbolKind::Egl);
    }
    if name.starts_with("sl") && name.as_bytes().get(2).is_some_and(u8::is_ascii_uppercase) {
        return Some(HleSymbolKind::OpenSl);
    }
    if name.starts_with("ANative")
        || name.starts_with("AInput")
        || name.starts_with("AKey")
        || name.starts_with("AMotion")
        || name.starts_with("AAsset")
        || name.starts_with("ALooper")
        || name.starts_with("AConfiguration")
        || name.starts_with("android_")
    {
        return Some(HleSymbolKind::Android);
    }
    if name.starts_with("__android_log_") {
        return Some(HleSymbolKind::Liblog);
    }
    if matches!(name, "dlopen" | "dlsym" | "dlclose" | "dlerror") {
        return Some(HleSymbolKind::Libdl);
    }
    if is_libm_symbol(name) {
        return Some(HleSymbolKind::Libm);
    }
    if is_zlib_symbol(name) {
        return Some(HleSymbolKind::Zlib);
    }
    if is_cxxabi_symbol(name) {
        return Some(HleSymbolKind::CxxAbi);
    }
    if is_libstdcxx_symbol(name) {
        return Some(HleSymbolKind::CxxStd);
    }
    if is_target_symbol(name) {
        return Some(HleSymbolKind::Target);
    }
    if is_libc_symbol(name) {
        return Some(HleSymbolKind::Libc);
    }
    None
}

fn hle_shape(name: &str) -> HleSymbolShape {
    match name {
        "__stack_chk_guard" => HleSymbolShape::Data {
            size: 4,
            init: HleDataInit::StackGuard,
        },
        "__sF" => HleSymbolShape::Data {
            size: 0x300,
            init: HleDataInit::Zero,
        },
        "_ctype_" => HleSymbolShape::Data {
            size: 0x200,
            init: HleDataInit::Ctype,
        },
        "_tolower_tab_" => HleSymbolShape::Data {
            size: 0x400,
            init: HleDataInit::ToLower,
        },
        "_toupper_tab_" => HleSymbolShape::Data {
            size: 0x400,
            init: HleDataInit::ToUpper,
        },
        "_ZNSs4_Rep20_S_empty_rep_storageE" => HleSymbolShape::Data {
            size: CXX_STRING_REP_HEADER_SIZE + 4,
            init: HleDataInit::CxxStringEmptyRep,
        },
        "_ZNSs4_Rep11_S_terminalE" => HleSymbolShape::Data {
            size: 1,
            init: HleDataInit::CxxStringTerminal,
        },
        "_ZNSs4_Rep11_S_max_sizeE" => HleSymbolShape::Data {
            size: 4,
            init: HleDataInit::CxxStringMaxSize,
        },
        _ => HleSymbolShape::Function,
    }
}

fn hle_behavior(name: &str, kind: HleSymbolKind) -> HleCallBehavior {
    if hle_shape(name) != HleSymbolShape::Function {
        return HleCallBehavior::ReturnZero;
    }
    if matches!(name, "abort" | "exit" | "__stack_chk_fail" | "__assert2") {
        return HleCallBehavior::Abort;
    }
    if kind == HleSymbolKind::Libm
        || kind == HleSymbolKind::Gles
        || kind == HleSymbolKind::Egl
        || kind == HleSymbolKind::CxxStd
        || kind == HleSymbolKind::Target
        || matches!(
            name,
            "memcpy"
                | "__aeabi_memcpy"
                | "memmove"
                | "__aeabi_memmove"
                | "memset"
                | "__aeabi_memset"
                | "memcmp"
                | "memchr"
                | "strlen"
                | "strcmp"
                | "strncmp"
                | "strcpy"
                | "strncpy"
                | "strcat"
                | "strchr"
                | "strrchr"
                | "isalnum"
                | "isspace"
                | "isupper"
                | "isxdigit"
                | "tolower"
                | "btowc"
                | "wctob"
                | "towlower"
                | "towupper"
                | "iswspace"
                | "wctype"
                | "iswctype"
                | "mbrtowc"
                | "mbstowcs"
                | "wcstombs"
                | "wcrtomb"
                | "wcslen"
                | "malloc"
                | "calloc"
                | "realloc"
                | "free"
                | "__errno"
                | "__aeabi_idiv"
                | "__aeabi_uidiv"
                | "__aeabi_idivmod"
                | "__aeabi_uidivmod"
                | "getauxval"
                | "gettimeofday"
                | "clock_gettime"
                | "time"
                | "sysconf"
                | "pthread_self"
                | "pthread_equal"
                | "pthread_getspecific"
                | "pthread_key_create"
                | "pthread_key_delete"
                | "pthread_setspecific"
                | "ALooper_pollAll"
                | "ALooper_pollOnce"
                | "ALooper_prepare"
                | "ALooper_forThread"
                | "ALooper_acquire"
                | "ALooper_addFd"
                | "ALooper_removeFd"
                | "ALooper_wake"
                | "ALooper_release"
                | "AAssetManager_open"
                | "AAsset_getLength"
                | "AAsset_getBuffer"
                | "AAsset_close"
                | "AConfiguration_new"
                | "AConfiguration_fromAssetManager"
                | "AConfiguration_getLanguage"
                | "AConfiguration_getCountry"
                | "AConfiguration_delete"
                | "fopen"
                | "fdopen"
                | "fclose"
                | "open"
                | "close"
                | "pipe"
                | "read"
                | "fread"
                | "write"
                | "fwrite"
                | "pthread_create"
                | "__cxa_guard_acquire"
                | "__cxa_guard_release"
                | "__cxa_guard_abort"
                | "_ZNSs14_M_replace_auxEjjjc"
        )
    {
        return HleCallBehavior::Implemented;
    }
    if is_negative_stub(name) {
        return HleCallBehavior::ReturnMinusOneErrno;
    }
    if is_null_stub(name) {
        return HleCallBehavior::ReturnNull;
    }
    if kind == HleSymbolKind::Egl {
        return HleCallBehavior::ReturnOne;
    }
    HleCallBehavior::ReturnZero
}

fn is_negative_stub(name: &str) -> bool {
    matches!(
        name,
        "accept"
            | "bind"
            | "chmod"
            | "close"
            | "closedir"
            | "connect"
            | "epoll_create"
            | "epoll_ctl"
            | "epoll_wait"
            | "fcntl"
            | "fdatasync"
            | "fsync"
            | "fstat"
            | "getaddrinfo"
            | "getnameinfo"
            | "getpeername"
            | "getsockname"
            | "getsockopt"
            | "ioctl"
            | "listen"
            | "lseek"
            | "mkdir"
            | "open"
            | "opendir"
            | "pipe"
            | "poll"
            | "pread"
            | "read"
            | "recv"
            | "recvfrom"
            | "recvmsg"
            | "remove"
            | "rename"
            | "rmdir"
            | "select"
            | "send"
            | "sendmsg"
            | "sendto"
            | "setsockopt"
            | "shutdown"
            | "socket"
            | "stat"
            | "unlink"
            | "utime"
            | "write"
            | "writev"
    )
}

fn is_null_stub(name: &str) -> bool {
    matches!(
        name,
        "fopen"
            | "fdopen"
            | "fgets"
            | "getenv"
            | "gethostbyname"
            | "readdir"
            | "strerror"
            | "dlopen"
            | "dlsym"
            | "dlerror"
    )
}

fn is_target_symbol(name: &str) -> bool {
    matches!(
        name,
        "_ZN8WebTokenC1ERKS_"
            | "_ZN8WebTokenC2ERKS_"
            | "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation"
            | "_ZN13GeometryGroup11getGeometryERKSs"
            | "_ZN13GeometryGroup14tryGetGeometryERKSs"
            | "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E"
            | "_ZN9UIControl18_resolvePostCreateEv"
    )
}

fn is_libc_symbol(name: &str) -> bool {
    matches!(
        name,
        "__assert2"
            | "__errno"
            | "__gnu_Unwind_Find_exidx"
            | "__google_potentially_blocking_region_begin"
            | "__google_potentially_blocking_region_end"
            | "__pthread_cleanup_pop"
            | "__pthread_cleanup_push"
            | "__sF"
            | "__stack_chk_fail"
            | "__stack_chk_guard"
            | "_ctype_"
            | "_tolower_tab_"
            | "_toupper_tab_"
            | "abort"
            | "accept"
            | "access"
            | "atoi"
            | "atof"
            | "bind"
            | "bsd_signal"
            | "btowc"
            | "calloc"
            | "chmod"
            | "clock"
            | "clock_gettime"
            | "close"
            | "closedir"
            | "connect"
            | "ctime"
            | "difftime"
            | "epoll_create"
            | "epoll_ctl"
            | "epoll_wait"
            | "exit"
            | "fclose"
            | "fcntl"
            | "fdatasync"
            | "fdopen"
            | "feof"
            | "ferror"
            | "fflush"
            | "fgetc"
            | "fgets"
            | "fopen"
            | "fprintf"
            | "fputc"
            | "fputs"
            | "fread"
            | "free"
            | "freeaddrinfo"
            | "fscanf"
            | "fseek"
            | "fseeko"
            | "fstat"
            | "fsync"
            | "ftell"
            | "ftello"
            | "fwrite"
            | "gai_strerror"
            | "getc"
            | "geteuid"
            | "getaddrinfo"
            | "getauxval"
            | "getenv"
            | "gethostbyname"
            | "gethostname"
            | "getnameinfo"
            | "getpeername"
            | "getpid"
            | "getsockname"
            | "getsockopt"
            | "gettimeofday"
            | "getuid"
            | "getwc"
            | "gmtime"
            | "gmtime_r"
            | "if_indextoname"
            | "if_nametoindex"
            | "inet_addr"
            | "inet_ntoa"
            | "inet_ntop"
            | "inet_pton"
            | "ioctl"
            | "isalnum"
            | "isspace"
            | "isupper"
            | "iswctype"
            | "iswspace"
            | "isxdigit"
            | "listen"
            | "localtime"
            | "localtime_r"
            | "lrand48"
            | "lseek"
            | "malloc"
            | "mbrtowc"
            | "mbstowcs"
            | "memcmp"
            | "memchr"
            | "memcpy"
            | "memmem"
            | "memmove"
            | "memset"
            | "mkdir"
            | "mktime"
            | "mmap"
            | "munmap"
            | "nanosleep"
            | "open"
            | "opendir"
            | "perror"
            | "pipe"
            | "poll"
            | "pread"
            | "printf"
            | "pthread_attr_destroy"
            | "pthread_attr_getdetachstate"
            | "pthread_attr_init"
            | "pthread_attr_setdetachstate"
            | "pthread_attr_setschedparam"
            | "pthread_attr_setstacksize"
            | "pthread_cond_broadcast"
            | "pthread_cond_destroy"
            | "pthread_cond_init"
            | "pthread_cond_signal"
            | "pthread_cond_timedwait"
            | "pthread_cond_wait"
            | "pthread_create"
            | "pthread_detach"
            | "pthread_equal"
            | "pthread_getspecific"
            | "pthread_join"
            | "pthread_key_create"
            | "pthread_key_delete"
            | "pthread_mutex_destroy"
            | "pthread_mutex_init"
            | "pthread_mutex_lock"
            | "pthread_mutex_trylock"
            | "pthread_mutex_unlock"
            | "pthread_mutexattr_destroy"
            | "pthread_mutexattr_init"
            | "pthread_mutexattr_settype"
            | "pthread_once"
            | "pthread_self"
            | "pthread_setname_np"
            | "pthread_setspecific"
            | "putc"
            | "putchar"
            | "puts"
            | "putwc"
            | "qsort"
            | "raise"
            | "read"
            | "readdir"
            | "realloc"
            | "recv"
            | "recvfrom"
            | "recvmsg"
            | "remove"
            | "rename"
            | "rmdir"
            | "sched_yield"
            | "select"
            | "sem_destroy"
            | "sem_init"
            | "sem_post"
            | "sem_wait"
            | "send"
            | "sendmsg"
            | "sendto"
            | "setenv"
            | "setlocale"
            | "setpriority"
            | "setsockopt"
            | "setvbuf"
            | "shutdown"
            | "sigaction"
            | "siglongjmp"
            | "sigprocmask"
            | "sigsetjmp"
            | "sleep"
            | "snprintf"
            | "socket"
            | "sprintf"
            | "srand"
            | "sscanf"
            | "stat"
            | "strcasecmp"
            | "strcat"
            | "strchr"
            | "strcmp"
            | "strcoll"
            | "strcpy"
            | "strcspn"
            | "strdup"
            | "strerror"
            | "strftime"
            | "strlen"
            | "strncasecmp"
            | "strncat"
            | "strncmp"
            | "strncpy"
            | "strpbrk"
            | "strptime"
            | "strrchr"
            | "strspn"
            | "strstr"
            | "strtod"
            | "strtof"
            | "strtol"
            | "strtoul"
            | "strtoull"
            | "strxfrm"
            | "syscall"
            | "sysconf"
            | "time"
            | "tolower"
            | "towlower"
            | "towupper"
            | "ungetc"
            | "ungetwc"
            | "unlink"
            | "unsetenv"
            | "usleep"
            | "utime"
            | "vfprintf"
            | "vsnprintf"
            | "vsprintf"
            | "wcrtomb"
            | "wcscoll"
            | "wcsftime"
            | "wcslen"
            | "wcstombs"
            | "wcsxfrm"
            | "wctob"
            | "wctype"
            | "wmemcmp"
            | "wmemchr"
            | "wmemcpy"
            | "wmemmove"
            | "wmemset"
            | "write"
            | "writev"
    ) || name.starts_with("__aeabi_")
}

fn is_libm_symbol(name: &str) -> bool {
    matches!(
        name,
        "acos"
            | "acosf"
            | "asin"
            | "asinf"
            | "atan"
            | "atan2"
            | "atan2f"
            | "atanf"
            | "ceil"
            | "ceilf"
            | "cos"
            | "cosf"
            | "cosh"
            | "exp"
            | "exp2f"
            | "expf"
            | "fabs"
            | "fabsf"
            | "floor"
            | "floorf"
            | "fmaxf"
            | "fmod"
            | "fmodf"
            | "frexp"
            | "ldexp"
            | "log"
            | "log10"
            | "log10f"
            | "logf"
            | "modf"
            | "nearbyintf"
            | "pow"
            | "powf"
            | "rint"
            | "roundf"
            | "sin"
            | "sinf"
            | "sinh"
            | "sqrt"
            | "sqrtf"
            | "tan"
            | "tanf"
            | "tanh"
            | "truncf"
    )
}

fn is_zlib_symbol(name: &str) -> bool {
    matches!(
        name,
        "adler32"
            | "compress"
            | "compress2"
            | "crc32"
            | "deflate"
            | "deflateEnd"
            | "deflateInit_"
            | "deflateInit2_"
            | "inflate"
            | "inflateEnd"
            | "inflateInit_"
            | "inflateInit2_"
            | "uncompress"
            | "zlibVersion"
    )
}

fn is_cxxabi_symbol(name: &str) -> bool {
    name.starts_with("__cxa_")
        || name.starts_with("__gxx_personality")
        || matches!(
            name,
            "_Unwind_Resume"
                | "_Unwind_DeleteException"
                | "_Unwind_GetRegionStart"
                | "_Unwind_GetLanguageSpecificData"
        )
}

fn is_libstdcxx_symbol(name: &str) -> bool {
    matches!(
        name,
        "_ZNSs4_Rep11_S_max_sizeE"
            | "_ZNSs4_Rep11_S_terminalE"
            | "_ZNSs4_Rep20_S_empty_rep_storageE"
            | "_ZNSs4_Rep10_M_destroyERKSaIcE"
            | "_ZNSs4_Rep9_S_createEjjRKSaIcE"
            | "_ZNSs12_S_constructEjcRKSaIcE"
            | "_ZNSs14_M_replace_auxEjjjc"
            | "_ZNSs15_M_replace_safeEjjPKcj"
            | "_ZNSs12_M_leak_hardEv"
            | "_ZNSs4swapERSs"
            | "_ZNSs5eraseEN9__gnu_cxx17__normal_iteratorIPcSsEES2_"
            | "_ZNSs6appendEPKcj"
            | "_ZNSs6appendERKSs"
            | "_ZNSs6appendEjc"
            | "_ZNSs6assignEPKcj"
            | "_ZNSs6assignERKSs"
            | "_ZNSs6insertEjPKcj"
            | "_ZNSs6resizeEjc"
            | "_ZNSs7replaceEjjPKcj"
            | "_ZNSs7reserveEj"
            | "_ZNSs9_M_mutateEjjj"
            | "_ZNSsaSEPKc"
            | "_ZNSsaSERKSs"
            | "_ZNSsC1EPKcRKSaIcE"
            | "_ZNSsC2EPKcRKSaIcE"
            | "_ZNSsC1EPKcjRKSaIcE"
            | "_ZNSsC2EPKcjRKSaIcE"
            | "_ZNSsC1ERKSs"
            | "_ZNSsC2ERKSs"
            | "_ZNSsC1ERKSsjj"
            | "_ZNSsC2ERKSsjj"
            | "_ZNSsC1EjcRKSaIcE"
            | "_ZNSsC2EjcRKSaIcE"
            | "_ZNSsC1Ev"
            | "_ZNSsC2Ev"
            | "_ZNSsD1Ev"
            | "_ZNSsD2Ev"
            | "_ZNKSs4findEPKcjj"
            | "_ZNKSs4findEcj"
            | "_ZNKSs5rfindEPKcjj"
            | "_ZNKSs5rfindEcj"
            | "_ZNKSs7compareEPKc"
            | "_ZNKSs7compareERKSs"
            | "_ZNKSs12find_last_ofEPKcjj"
            | "_ZNKSs13find_first_ofEPKcjj"
            | "_ZNKSs16find_last_not_ofEPKcjj"
            | "_ZNKSs17find_first_not_ofEPKcjj"
    )
}

fn init_ctype<M: Memory>(memory: &mut M, address: u32, upper: bool) -> Result<(), HleError> {
    let table = address.wrapping_add(4);
    store32(memory, address, table)?;
    for value in 0..=255u32 {
        let flags = ctype_flags(value as u8, upper);
        store8(memory, table.wrapping_add(value + 1), flags as u8)?;
    }
    Ok(())
}

fn init_case_table<M: Memory>(memory: &mut M, address: u32, upper: bool) -> Result<(), HleError> {
    let table = address.wrapping_add(4);
    store32(memory, address, table)?;
    for value in 0..=255u32 {
        let byte = value as u8;
        let mapped = if upper {
            byte.to_ascii_uppercase()
        } else {
            byte.to_ascii_lowercase()
        };
        store16(
            memory,
            table.wrapping_add((value + 1) * 2),
            u16::from(mapped),
        )?;
    }
    Ok(())
}

fn init_cxx_string_empty_rep<M: Memory>(memory: &mut M, address: u32) -> Result<(), HleError> {
    store32(memory, address, 0)?;
    store32(memory, address.wrapping_add(4), 0)?;
    store32(memory, address.wrapping_add(8), 0)?;
    store8(memory, address.wrapping_add(CXX_STRING_REP_HEADER_SIZE), 0)?;
    Ok(())
}

fn ctype_flags(value: u8, _upper: bool) -> u16 {
    let mut flags = 0u16;
    if value.is_ascii_uppercase() {
        flags |= 0x0001;
    }
    if value.is_ascii_lowercase() {
        flags |= 0x0002;
    }
    if value.is_ascii_digit() {
        flags |= 0x0004;
    }
    if value.is_ascii_whitespace() {
        flags |= 0x0008;
    }
    if value.is_ascii_punctuation() {
        flags |= 0x0010;
    }
    if value.is_ascii_control() {
        flags |= 0x0020;
    }
    if value == b' ' {
        flags |= 0x0040;
    }
    if value.is_ascii_hexdigit() {
        flags |= 0x0080;
    }
    flags
}

fn ascii_byte(value: u32) -> Option<u8> {
    (value <= 0xff).then_some(value as u8)
}

fn ascii_isalnum(value: u32) -> bool {
    ascii_byte(value).is_some_and(|byte| byte.is_ascii_alphanumeric())
}

fn ascii_isspace(value: u32) -> bool {
    ascii_byte(value).is_some_and(|byte| byte.is_ascii_whitespace())
}

fn ascii_isupper(value: u32) -> bool {
    ascii_byte(value).is_some_and(|byte| byte.is_ascii_uppercase())
}

fn ascii_isxdigit(value: u32) -> bool {
    ascii_byte(value).is_some_and(|byte| byte.is_ascii_hexdigit())
}

fn ascii_tolower(value: u32) -> u32 {
    ascii_byte(value)
        .map(|byte| u32::from(byte.to_ascii_lowercase()))
        .unwrap_or(value)
}

fn ascii_toupper(value: u32) -> u32 {
    ascii_byte(value)
        .map(|byte| u32::from(byte.to_ascii_uppercase()))
        .unwrap_or(value)
}

fn btowc_ascii(value: u32) -> u32 {
    if value == u32::MAX {
        u32::MAX
    } else {
        u32::from(value as u8)
    }
}

fn wctob_ascii(value: u32) -> u32 {
    ascii_byte(value).map(u32::from).unwrap_or(u32::MAX)
}

fn ascii_wctype_descriptor(name: &str) -> Option<u32> {
    match name {
        "alnum" => Some(WCTYPE_ALNUM),
        "alpha" => Some(WCTYPE_ALPHA),
        "blank" => Some(WCTYPE_BLANK),
        "cntrl" => Some(WCTYPE_CNTRL),
        "digit" => Some(WCTYPE_DIGIT),
        "graph" => Some(WCTYPE_GRAPH),
        "lower" => Some(WCTYPE_LOWER),
        "print" => Some(WCTYPE_PRINT),
        "punct" => Some(WCTYPE_PUNCT),
        "space" => Some(WCTYPE_SPACE),
        "upper" => Some(WCTYPE_UPPER),
        "xdigit" => Some(WCTYPE_XDIGIT),
        _ => None,
    }
}

fn ascii_iswctype(value: u32, descriptor: u32) -> bool {
    let Some(byte) = ascii_byte(value) else {
        return false;
    };
    match descriptor {
        WCTYPE_ALNUM => byte.is_ascii_alphanumeric(),
        WCTYPE_ALPHA => byte.is_ascii_alphabetic(),
        WCTYPE_BLANK => matches!(byte, b' ' | b'\t'),
        WCTYPE_CNTRL => byte.is_ascii_control(),
        WCTYPE_DIGIT => byte.is_ascii_digit(),
        WCTYPE_GRAPH => byte.is_ascii_graphic(),
        WCTYPE_LOWER => byte.is_ascii_lowercase(),
        WCTYPE_PRINT => byte.is_ascii_graphic() || byte == b' ',
        WCTYPE_PUNCT => byte.is_ascii_punctuation(),
        WCTYPE_SPACE => byte.is_ascii_whitespace(),
        WCTYPE_UPPER => byte.is_ascii_uppercase(),
        WCTYPE_XDIGIT => byte.is_ascii_hexdigit(),
        _ => false,
    }
}

fn strlen<M: Memory>(memory: &mut M, ptr: u32) -> Result<u32, HleError> {
    let mut len = 0u32;
    loop {
        if load8(memory, ptr.wrapping_add(len))? == 0 {
            return Ok(len);
        }
        len = len.wrapping_add(1);
    }
}

fn load_c_string<M: Memory>(memory: &mut M, ptr: u32, max_len: u32) -> Result<String, HleError> {
    let mut bytes = Vec::new();
    for idx in 0..max_len {
        let byte = load8(memory, ptr.wrapping_add(idx))?;
        if byte == 0 {
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        bytes.push(byte);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn load_bytes<M: Memory>(memory: &mut M, ptr: u32, len: u32) -> Result<Vec<u8>, HleError> {
    let mut bytes = Vec::with_capacity(len as usize);
    for idx in 0..len {
        bytes.push(load8(memory, ptr.wrapping_add(idx))?);
    }
    Ok(bytes)
}

fn load_cxx_string_bytes<M: Memory>(memory: &mut M, string: u32) -> Result<Vec<u8>, HleError> {
    if string == 0 {
        return Ok(Vec::new());
    }
    let data = load32(memory, string)?;
    let len = cxx_string_len_from_data(memory, data)?;
    load_bytes(memory, data, len)
}

fn cxx_string_len_from_data<M: Memory>(memory: &mut M, data: u32) -> Result<u32, HleError> {
    if data == 0 {
        Ok(0)
    } else {
        load32(memory, data.wrapping_sub(CXX_STRING_REP_HEADER_SIZE))
    }
}

fn cxx_string_capacity<M: Memory>(memory: &mut M, string: u32) -> Result<u32, HleError> {
    if string == 0 {
        return Ok(0);
    }
    let data = load32(memory, string)?;
    if data == 0 {
        Ok(0)
    } else {
        load32(memory, data.wrapping_sub(8))
    }
}

fn compare_bytes(lhs: &[u8], rhs: &[u8]) -> i32 {
    for (left, right) in lhs.iter().copied().zip(rhs.iter().copied()) {
        if left != right {
            return i32::from(left) - i32::from(right);
        }
    }
    lhs.len().cmp(&rhs.len()) as i32
}

fn find_subslice(haystack: &[u8], needle: &[u8], pos: u32) -> u32 {
    let pos = pos as usize;
    if needle.is_empty() {
        return if pos <= haystack.len() {
            pos as u32
        } else {
            CXX_STRING_NPOS
        };
    }
    if pos > haystack.len() || needle.len() > haystack.len().saturating_sub(pos) {
        return CXX_STRING_NPOS;
    }
    haystack[pos..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|idx| (pos + idx) as u32)
        .unwrap_or(CXX_STRING_NPOS)
}

fn find_byte(haystack: &[u8], needle: u8, pos: u32) -> u32 {
    haystack
        .get(pos as usize..)
        .and_then(|tail| tail.iter().position(|&byte| byte == needle))
        .map(|idx| pos.wrapping_add(idx as u32))
        .unwrap_or(CXX_STRING_NPOS)
}

fn rfind_subslice(haystack: &[u8], needle: &[u8], pos: u32) -> u32 {
    if needle.is_empty() {
        return (pos as usize).min(haystack.len()) as u32;
    }
    if needle.len() > haystack.len() {
        return CXX_STRING_NPOS;
    }
    let start = (pos as usize).min(haystack.len() - needle.len());
    haystack[..=start]
        .windows(needle.len())
        .rposition(|window| window == needle)
        .map(|idx| idx as u32)
        .unwrap_or(CXX_STRING_NPOS)
}

fn rfind_byte(haystack: &[u8], needle: u8, pos: u32) -> u32 {
    if haystack.is_empty() {
        return CXX_STRING_NPOS;
    }
    let start = (pos as usize).min(haystack.len() - 1);
    haystack[..=start]
        .iter()
        .rposition(|&byte| byte == needle)
        .map(|idx| idx as u32)
        .unwrap_or(CXX_STRING_NPOS)
}

fn find_first_of(haystack: &[u8], needles: &[u8], pos: u32, want_match: bool) -> u32 {
    haystack
        .get(pos as usize..)
        .and_then(|tail| {
            tail.iter()
                .position(|byte| needles.contains(byte) == want_match)
        })
        .map(|idx| pos.wrapping_add(idx as u32))
        .unwrap_or(CXX_STRING_NPOS)
}

fn find_last_of(haystack: &[u8], needles: &[u8], pos: u32, want_match: bool) -> u32 {
    if haystack.is_empty() {
        return CXX_STRING_NPOS;
    }
    let start = (pos as usize).min(haystack.len() - 1);
    haystack[..=start]
        .iter()
        .rposition(|byte| needles.contains(byte) == want_match)
        .map(|idx| idx as u32)
        .unwrap_or(CXX_STRING_NPOS)
}

fn android_asset_entry_candidates(path: &str) -> Vec<String> {
    let mut clean = path.trim_start_matches('/');
    while let Some(stripped) = clean.strip_prefix("./") {
        clean = stripped.trim_start_matches('/');
    }
    if clean.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    push_unique_string(&mut candidates, clean.to_string());
    if let Some(stripped) = clean.strip_prefix("assets/") {
        if !stripped.is_empty() {
            push_unique_string(&mut candidates, stripped.to_string());
        }
    } else {
        push_unique_string(&mut candidates, format!("assets/{clean}"));
    }
    candidates
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn trace_android_asset(args: fmt::Arguments<'_>) {
    if std::env::var_os("AEMU_TRACE_HLE_ASSET").is_some() {
        eprintln!("HLE asset {args}");
    }
}

fn trace_hle_file(args: fmt::Arguments<'_>) {
    if std::env::var_os("AEMU_TRACE_HLE_FILE").is_some() {
        eprintln!("HLE file {args}");
    }
}

fn is_random_device_path(path: &str) -> bool {
    matches!(path, "/dev/urandom" | "/dev/random")
}

fn is_virtual_storage_path(path: &str) -> bool {
    path.starts_with("/sdcard/") || path.starts_with("/storage/") || path.starts_with("/data/data/")
}

fn strcmp<M: Memory>(memory: &mut M, a: u32, b: u32, max_len: u32) -> Result<i32, HleError> {
    for idx in 0..max_len {
        let av = load8(memory, a.wrapping_add(idx))?;
        let bv = load8(memory, b.wrapping_add(idx))?;
        if av != bv || av == 0 || bv == 0 {
            return Ok(i32::from(av) - i32::from(bv));
        }
    }
    Ok(0)
}

fn load8<M: Memory>(memory: &mut M, addr: u32) -> Result<u8, HleError> {
    memory
        .load8(addr)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn load16<M: Memory>(memory: &mut M, addr: u32) -> Result<u16, HleError> {
    memory
        .load16(addr)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn load32<M: Memory>(memory: &mut M, addr: u32) -> Result<u32, HleError> {
    memory
        .load32(addr)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn store8<M: Memory>(memory: &mut M, addr: u32, value: u8) -> Result<(), HleError> {
    memory
        .store8(addr, value)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn store16<M: Memory>(memory: &mut M, addr: u32, value: u16) -> Result<(), HleError> {
    memory
        .store16(addr, value)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn store32<M: Memory>(memory: &mut M, addr: u32, value: u32) -> Result<(), HleError> {
    memory
        .store32(addr, value)
        .map_err(|err| HleError::Memory(err.to_string()))
}

fn store_json_null<M: Memory>(memory: &mut M, addr: u32) -> Result<(), HleError> {
    for offset in 0..16 {
        store8(memory, addr.wrapping_add(offset), 0)?;
    }
    Ok(())
}

fn write_android_locale_code<M: Memory>(
    memory: &mut M,
    ptr: u32,
    code: &[u8; 2],
) -> Result<(), HleError> {
    if ptr != 0 {
        store8(memory, ptr, code[0])?;
        store8(memory, ptr.wrapping_add(1), code[1])?;
        store8(memory, ptr.wrapping_add(2), 0)?;
    }
    Ok(())
}

fn f32_arg(cpu: &Cpu, reg: usize) -> f32 {
    f32::from_bits(cpu.reg(reg))
}

fn f64_arg(cpu: &Cpu, reg: usize) -> f64 {
    let lo = u64::from(cpu.reg(reg));
    let hi = u64::from(cpu.reg(reg + 1));
    f64::from_bits(lo | (hi << 32))
}

fn i32_to_u32(value: i32) -> u32 {
    value as u32
}

fn align_up(value: u32, align: u32) -> Option<u32> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

fn egl_query_string(name: u32) -> Option<&'static str> {
    match name {
        EGL_VENDOR => Some("AEMU"),
        EGL_VERSION => Some("1.4 AEMU EGL"),
        EGL_EXTENSIONS => Some("EGL_KHR_create_context EGL_KHR_surfaceless_context"),
        EGL_CLIENT_APIS => Some("OpenGL ES"),
        _ => None,
    }
}

fn egl_config_attrib(attr: u32) -> u32 {
    match attr {
        EGL_BUFFER_SIZE => 32,
        EGL_RED_SIZE | EGL_GREEN_SIZE | EGL_BLUE_SIZE | EGL_ALPHA_SIZE => 8,
        EGL_DEPTH_SIZE => 24,
        EGL_STENCIL_SIZE => 8,
        EGL_CONFIG_CAVEAT => EGL_NONE,
        EGL_CONFIG_ID => EGL_CONFIG_HANDLE,
        EGL_LEVEL => 0,
        EGL_MAX_PBUFFER_HEIGHT => EGL_DEFAULT_SURFACE_HEIGHT,
        EGL_MAX_PBUFFER_PIXELS => EGL_DEFAULT_SURFACE_WIDTH * EGL_DEFAULT_SURFACE_HEIGHT,
        EGL_MAX_PBUFFER_WIDTH => EGL_DEFAULT_SURFACE_WIDTH,
        EGL_NATIVE_RENDERABLE => 0,
        EGL_NATIVE_VISUAL_ID => ANDROID_WINDOW_FORMAT_RGBA_8888,
        EGL_NATIVE_VISUAL_TYPE => EGL_NONE,
        EGL_SAMPLES | EGL_SAMPLE_BUFFERS => 0,
        EGL_SURFACE_TYPE => EGL_WINDOW_BIT | EGL_PBUFFER_BIT,
        EGL_TRANSPARENT_TYPE => EGL_NONE,
        EGL_BIND_TO_TEXTURE_RGB | EGL_BIND_TO_TEXTURE_RGBA => 0,
        EGL_MIN_SWAP_INTERVAL | EGL_MAX_SWAP_INTERVAL => 1,
        EGL_LUMINANCE_SIZE | EGL_ALPHA_MASK_SIZE => 0,
        EGL_COLOR_BUFFER_TYPE => EGL_RGB_BUFFER,
        EGL_RENDERABLE_TYPE | EGL_CONFORMANT => EGL_OPENGL_ES_BIT | EGL_OPENGL_ES2_BIT,
        _ => 0,
    }
}

fn egl_surface_attrib(attr: u32) -> u32 {
    match attr {
        EGL_WIDTH => EGL_DEFAULT_SURFACE_WIDTH,
        EGL_HEIGHT => EGL_DEFAULT_SURFACE_HEIGHT,
        EGL_CONFIG_ID => EGL_CONFIG_HANDLE,
        EGL_RENDERABLE_TYPE => EGL_OPENGL_ES_BIT | EGL_OPENGL_ES2_BIT,
        _ => 0,
    }
}

fn gl_query_string(name: u32) -> Option<&'static str> {
    match name {
        GL_VENDOR => Some("AEMU"),
        GL_RENDERER => Some("AEMU WebGL1 GLES2 HLE"),
        GL_VERSION => Some("OpenGL ES 2.0 AEMU"),
        GL_EXTENSIONS => Some(
            "GL_OES_rgb8_rgba8 GL_OES_depth24 GL_OES_packed_depth_stencil GL_EXT_texture_format_BGRA8888",
        ),
        GL_SHADING_LANGUAGE_VERSION => Some("OpenGL ES GLSL ES 1.00 AEMU"),
        _ => None,
    }
}

fn gl_program_iv(name: u32) -> u32 {
    match name {
        GL_LINK_STATUS => 1,
        GL_INFO_LOG_LENGTH
        | GL_ACTIVE_UNIFORMS
        | GL_ACTIVE_UNIFORM_MAX_LENGTH
        | GL_ACTIVE_ATTRIBUTES
        | GL_ACTIVE_ATTRIBUTE_MAX_LENGTH => 0,
        _ => 0,
    }
}

fn gl_shader_iv(name: u32) -> u32 {
    match name {
        GL_COMPILE_STATUS => 1,
        GL_INFO_LOG_LENGTH => 0,
        _ => 0,
    }
}

fn gl_integer(name: u32) -> u32 {
    match name {
        GL_MAX_TEXTURE_SIZE => 4096,
        GL_MAX_TEXTURE_IMAGE_UNITS => 8,
        GL_MAX_VERTEX_ATTRIBS => 16,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::armv6::Isa;
    use crate::guest_memory::MappedMemory;

    use super::*;

    fn load_test_cxx_string(memory: &mut MappedMemory, string: u32) -> Vec<u8> {
        let data = memory.load32(string).unwrap();
        let len = memory.load32(data - CXX_STRING_REP_HEADER_SIZE).unwrap();
        memory.load_bytes_for_test(data, len as usize).to_vec()
    }

    #[test]
    fn describes_current_minecraft_system_imports() {
        for name in [
            "socket",
            "getaddrinfo",
            "pthread_cond_timedwait",
            "AKeyEvent_getAction",
            "AMotionEvent_getX",
            "__stack_chk_guard",
            "__sF",
            "_ctype_",
            "roundf",
            "__gnu_Unwind_Find_exidx",
            "_ZNSs4_Rep20_S_empty_rep_storageE",
            "_ZNSsC1ERKSs",
            "_ZNSs14_M_replace_auxEjjjc",
            "_ZN8WebTokenC2ERKS_",
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation",
            "_ZN13GeometryGroup11getGeometryERKSs",
            "_ZN13GeometryGroup14tryGetGeometryERKSs",
            "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E",
            "_ZN9UIControl18_resolvePostCreateEv",
        ] {
            assert!(describe_hle_import(name).is_some(), "{name}");
        }
    }

    #[test]
    fn initializes_function_and_data_symbols() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let gl = describe_hle_import("glCreateShader").unwrap();
        initialize_hle_symbol(&mut memory, gl, 0x1000).unwrap();
        assert_eq!(memory.load32(0x1000).unwrap(), HLE_TRAP_ARM_INSTR);

        let guard = describe_hle_import("__stack_chk_guard").unwrap();
        initialize_hle_symbol(&mut memory, guard, 0x1100).unwrap();
        assert_eq!(memory.load32(0x1100).unwrap(), 0x00c0_ffee);

        let empty_rep = describe_hle_import("_ZNSs4_Rep20_S_empty_rep_storageE").unwrap();
        initialize_hle_symbol(&mut memory, empty_rep, 0x1200).unwrap();
        assert_eq!(memory.load32(0x1200).unwrap(), 0);
        assert_eq!(memory.load8(0x120c).unwrap(), 0);
    }

    #[test]
    fn initializes_ctype_as_pointer_backed_byte_table() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let ctype = describe_hle_import("_ctype_").unwrap();
        initialize_hle_symbol(&mut memory, ctype, 0x1200).unwrap();

        let table = memory.load32(0x1200).unwrap();
        assert_eq!(table, 0x1204);
        assert_eq!(
            memory.load8(table + u32::from(b'A') + 1).unwrap(),
            ctype_flags(b'A', false) as u8
        );
        assert_eq!(
            memory.load8(table + u32::from(b'a') + 1).unwrap(),
            ctype_flags(b'a', false) as u8
        );
        assert_eq!(
            memory.load8(table + u32::from(b'0') + 1).unwrap(),
            ctype_flags(b'0', false) as u8
        );
        assert_eq!(
            memory.load8(table + u32::from(b' ') + 1).unwrap(),
            ctype_flags(b' ', false) as u8
        );
    }

    #[test]
    fn initializes_case_maps_as_pointer_backed_halfword_tables() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let tolower = describe_hle_import("_tolower_tab_").unwrap();
        initialize_hle_symbol(&mut memory, tolower, 0x1200).unwrap();
        let lower_table = memory.load32(0x1200).unwrap();
        assert_eq!(lower_table, 0x1204);
        assert_eq!(
            memory
                .load16(lower_table + (u32::from(b'A') + 1) * 2)
                .unwrap(),
            u16::from(b'a')
        );
        assert_eq!(
            memory
                .load16(lower_table + (u32::from(b'z') + 1) * 2)
                .unwrap(),
            u16::from(b'z')
        );

        let toupper = describe_hle_import("_toupper_tab_").unwrap();
        initialize_hle_symbol(&mut memory, toupper, 0x1600).unwrap();
        let upper_table = memory.load32(0x1600).unwrap();
        assert_eq!(upper_table, 0x1604);
        assert_eq!(
            memory
                .load16(upper_table + (u32::from(b'a') + 1) * 2)
                .unwrap(),
            u16::from(b'A')
        );
        assert_eq!(
            memory
                .load16(upper_table + (u32::from(b'Z') + 1) * 2)
                .unwrap(),
            u16::from(b'Z')
        );
    }

    #[test]
    fn dispatches_getauxval_with_armv7_neon_hwcap() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        assert_eq!(
            describe_hle_import("getauxval").unwrap().behavior,
            HleCallBehavior::Implemented
        );

        cpu.set_reg(0, AT_HWCAP);
        hle.dispatch("getauxval", &mut cpu, &mut memory).unwrap();
        assert_ne!(cpu.reg(0) & HWCAP_NEON, 0);
        assert_ne!(cpu.reg(0) & HWCAP_VFPV3, 0);
        assert_ne!(cpu.reg(0) & HWCAP_VFPD32, 0);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(0, AT_HWCAP2);
        hle.dispatch("getauxval", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn dispatches_egl_facade_outputs() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x3000, 0x1000);

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, EGL_VENDOR);
        hle.dispatch("eglQueryString", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_c_string(&mut memory, cpu.reg(0), 32).unwrap(), "AEMU");
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 0x1104);
        hle.dispatch("eglInitialize", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(memory.load32(0x1100).unwrap(), 1);
        assert_eq!(memory.load32(0x1104).unwrap(), 4);

        memory.store32(0x1200, 0x1124).unwrap();
        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0x1120);
        cpu.set_reg(3, 1);
        cpu.set_reg(13, 0x1200);
        hle.dispatch("eglChooseConfig", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(memory.load32(0x1120).unwrap(), EGL_CONFIG_HANDLE);
        assert_eq!(memory.load32(0x1124).unwrap(), 1);

        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, EGL_CONFIG_HANDLE);
        cpu.set_reg(2, EGL_NATIVE_VISUAL_ID);
        cpu.set_reg(3, 0x1130);
        hle.dispatch("eglGetConfigAttrib", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(
            memory.load32(0x1130).unwrap(),
            ANDROID_WINDOW_FORMAT_RGBA_8888
        );

        cpu.set_reg(14, 0x2010);
        cpu.set_reg(2, EGL_RENDERABLE_TYPE);
        cpu.set_reg(3, 0x1134);
        hle.dispatch("eglGetConfigAttrib", &mut cpu, &mut memory)
            .unwrap();
        assert_ne!(memory.load32(0x1134).unwrap() & EGL_OPENGL_ES2_BIT, 0);

        cpu.set_reg(14, 0x2014);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, EGL_SURFACE_HANDLE);
        cpu.set_reg(2, EGL_WIDTH);
        cpu.set_reg(3, 0x1140);
        hle.dispatch("eglQuerySurface", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1140).unwrap(), EGL_DEFAULT_SURFACE_WIDTH);

        cpu.set_reg(14, 0x2018);
        cpu.set_reg(2, EGL_HEIGHT);
        cpu.set_reg(3, 0x1144);
        hle.dispatch("eglQuerySurface", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1144).unwrap(), EGL_DEFAULT_SURFACE_HEIGHT);
    }

    #[test]
    fn dispatches_gles_string_facade_outputs() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, GL_VERSION);
        let mut hle = HleRuntime::new(0, 0x3000, 0x1000);

        hle.dispatch("glGetString", &mut cpu, &mut memory).unwrap();
        assert_ne!(cpu.reg(0), 0);
        assert_eq!(
            load_c_string(&mut memory, cpu.reg(0), 64).unwrap(),
            "OpenGL ES 2.0 AEMU"
        );
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, GL_SHADING_LANGUAGE_VERSION);
        hle.dispatch("glGetString", &mut cpu, &mut memory).unwrap();
        assert!(
            load_c_string(&mut memory, cpu.reg(0), 64)
                .unwrap()
                .contains("GLSL ES 1.00")
        );
        assert_eq!(cpu.pc(), 0x2004);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, 0xffff);
        hle.dispatch("glGetString", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2008);
    }

    #[test]
    fn dispatches_gles_shader_query_facade_outputs() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x5000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(13, 0x1800);
        let mut hle = HleRuntime::new(0, 0x3000, 0x2000);

        memory.store32(0x1100, 0xcccc_cccc).unwrap();
        cpu.set_reg(14, 0x2000);
        cpu.set_reg(1, GL_ACTIVE_UNIFORMS);
        cpu.set_reg(2, 0x1100);
        hle.dispatch("glGetProgramiv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1100).unwrap(), 0);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(1, GL_LINK_STATUS);
        cpu.set_reg(2, 0x1104);
        hle.dispatch("glGetProgramiv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1104).unwrap(), 1);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(1, GL_COMPILE_STATUS);
        cpu.set_reg(2, 0x1108);
        hle.dispatch("glGetShaderiv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1108).unwrap(), 1);

        memory.store32(0x1800, 0x1110).unwrap();
        memory.store32(0x1804, 0x1114).unwrap();
        memory.store32(0x1808, 0x1118).unwrap();
        memory.store8(0x1118, b'x').unwrap();
        cpu.set_reg(14, 0x200c);
        cpu.set_reg(2, 4);
        cpu.set_reg(3, 0x110c);
        hle.dispatch("glGetActiveUniform", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x110c).unwrap(), 0);
        assert_eq!(memory.load32(0x1110).unwrap(), 0);
        assert_eq!(memory.load32(0x1114).unwrap(), 0);
        assert_eq!(memory.load8(0x1118).unwrap(), 0);

        memory.store32(0x1120, 0xcccc_cccc).unwrap();
        memory.store8(0x1124, b'x').unwrap();
        cpu.set_reg(14, 0x2010);
        cpu.set_reg(2, 0x1120);
        cpu.set_reg(3, 0x1124);
        hle.dispatch("glGetProgramInfoLog", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1120).unwrap(), 0);
        assert_eq!(memory.load8(0x1124).unwrap(), 0);
    }

    #[test]
    fn dispatches_android_configuration_locale_facade() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x3000, 0x1000);

        cpu.set_reg(14, 0x2000);
        hle.dispatch("AConfiguration_new", &mut cpu, &mut memory)
            .unwrap();
        assert_ne!(cpu.reg(0), 0);
        let config = cpu.reg(0);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, config);
        cpu.set_reg(1, 0x1100);
        hle.dispatch("AConfiguration_getLanguage", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_c_string(&mut memory, 0x1100, 4).unwrap(), "en");
        assert_eq!(cpu.pc(), 0x2004);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, config);
        cpu.set_reg(1, 0x1110);
        hle.dispatch("AConfiguration_getCountry", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_c_string(&mut memory, 0x1110, 4).unwrap(), "US");
        assert_eq!(cpu.pc(), 0x2008);

        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, config);
        hle.dispatch("AConfiguration_fromAssetManager", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.pc(), 0x200c);

        cpu.set_reg(14, 0x2010);
        cpu.set_reg(0, config);
        hle.dispatch("AConfiguration_delete", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.pc(), 0x2010);
    }

    #[test]
    fn dispatches_android_asset_manager_reads_apk_entries() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("aemu-assets-{stamp}.apk"));
        fs::write(
            &path,
            stored_zip_with_one_file("assets/loc/languages.json", br#"{"en_US":"English"}"#),
        )
        .unwrap();

        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();
        memory.load_bytes(0x1100, b"loc/languages.json\0").unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x3000, 0x1000);
        hle.set_apk_path(path.clone());

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, 0x1200);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 3);
        hle.dispatch("AAssetManager_open", &mut cpu, &mut memory)
            .unwrap();
        let asset = cpu.reg(0);
        assert_ne!(asset, 0);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, asset);
        hle.dispatch("AAsset_getLength", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 19);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, asset);
        hle.dispatch("AAsset_getBuffer", &mut cpu, &mut memory)
            .unwrap();
        let buffer = cpu.reg(0);
        let mut loaded = Vec::new();
        for idx in 0..19 {
            loaded.push(memory.load8(buffer + idx).unwrap());
        }
        assert_eq!(loaded, br#"{"en_US":"English"}"#);

        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, asset);
        hle.dispatch("AAsset_close", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.pc(), 0x200c);

        cpu.set_reg(14, 0x2010);
        cpu.set_reg(0, asset);
        hle.dispatch("AAsset_getBuffer", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn dispatches_sysconf_for_android_runtime_values() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        assert_eq!(
            describe_hle_import("sysconf").unwrap().behavior,
            HleCallBehavior::Implemented
        );

        cpu.set_reg(0, SC_PAGESIZE);
        hle.dispatch("sysconf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 4096);

        cpu.set_reg(0, SC_NPROCESSORS_ONLN);
        hle.dispatch("sysconf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(0, SC_CLK_TCK);
        hle.dispatch("sysconf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 100);

        cpu.set_reg(0, 0xffff);
        hle.dispatch("sysconf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), u32::MAX);
        assert_eq!(memory.load32(0x1000).unwrap(), 22);
    }

    #[test]
    fn dispatches_advancing_time_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0x1100);
        hle.dispatch("clock_gettime", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1100).unwrap(), 0);
        assert_eq!(memory.load32(0x1104).unwrap(), FAKE_TIME_STEP_NANOS as u32);

        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0x1110);
        hle.dispatch("clock_gettime", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(
            memory.load32(0x1114).unwrap(),
            (FAKE_TIME_STEP_NANOS * 2) as u32
        );

        cpu.set_reg(0, 0x1120);
        hle.dispatch("gettimeofday", &mut cpu, &mut memory).unwrap();
        assert_eq!(memory.load32(0x1120).unwrap(), FAKE_TIME_BASE_SECS as u32);
        assert_eq!(
            memory.load32(0x1124).unwrap(),
            ((FAKE_TIME_STEP_NANOS * 3) / 1_000) as u32
        );

        cpu.set_reg(0, 0x1130);
        hle.dispatch("time", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), FAKE_TIME_BASE_SECS as u32);
        assert_eq!(memory.load32(0x1130).unwrap(), FAKE_TIME_BASE_SECS as u32);
    }

    #[test]
    fn dispatches_pthread_identity_and_specific_data() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        assert_eq!(
            describe_hle_import("pthread_equal").unwrap().behavior,
            HleCallBehavior::Implemented
        );

        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0);
        hle.dispatch("pthread_equal", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(0, 1);
        cpu.set_reg(1, 2);
        hle.dispatch("pthread_equal", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(0, 0x1100);
        hle.dispatch("pthread_key_create", &mut cpu, &mut memory)
            .unwrap();
        let key = memory.load32(0x1100).unwrap();
        assert_eq!(key, 0);
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(0, key);
        cpu.set_reg(1, 0xfeed_beef);
        hle.dispatch("pthread_setspecific", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(0, key);
        hle.dispatch("pthread_getspecific", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0xfeed_beef);

        cpu.set_reg(0, key);
        hle.dispatch("pthread_key_delete", &mut cpu, &mut memory)
            .unwrap();
        cpu.set_reg(0, key);
        hle.dispatch("pthread_getspecific", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn dispatches_alooper_poll_as_no_event() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        assert_eq!(
            describe_hle_import("ALooper_pollAll").unwrap().behavior,
            HleCallBehavior::Implemented
        );

        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 0x1104);
        cpu.set_reg(3, 0x1108);
        hle.dispatch("ALooper_pollAll", &mut cpu, &mut memory)
            .unwrap();

        assert_eq!(cpu.reg(0), u32::MAX);
        assert_eq!(memory.load32(0x1100).unwrap(), u32::MAX);
        assert_eq!(memory.load32(0x1104).unwrap(), 0);
        assert_eq!(memory.load32(0x1108).unwrap(), 0);
    }

    #[test]
    fn dispatches_queued_alooper_event_source() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        hle.queue_alooper_event(0x1234_5678);

        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 0x1104);
        cpu.set_reg(3, 0x1108);
        hle.dispatch("ALooper_pollAll", &mut cpu, &mut memory)
            .unwrap();

        assert_eq!(cpu.reg(0), 1);
        assert_eq!(memory.load32(0x1100).unwrap(), u32::MAX);
        assert_eq!(memory.load32(0x1104).unwrap(), 0);
        assert_eq!(memory.load32(0x1108).unwrap(), 0x1234_5678);

        hle.dispatch("ALooper_pollAll", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), u32::MAX);
        assert_eq!(memory.load32(0x1108).unwrap(), 0);
    }

    #[test]
    fn dispatches_memory_and_string_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        memory.load_bytes(0x1100, b"abc\0").unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        cpu.set_reg(0, 0x1100);
        hle.dispatch("strlen", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 3);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(0, 0x1200);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 4);
        hle.dispatch("memcpy", &mut cpu, &mut memory).unwrap();
        assert_eq!(memory.load_bytes_for_test(0x1200, 4), b"abc\0");

        cpu.set_reg(0, 0x1204);
        cpu.set_reg(1, 0xee);
        cpu.set_reg(2, 3);
        hle.dispatch("memset", &mut cpu, &mut memory).unwrap();
        assert_eq!(memory.load_bytes_for_test(0x1204, 3), &[0xee; 3]);

        cpu.set_reg(0, 0x1208);
        cpu.set_reg(1, 3);
        cpu.set_reg(2, 0x44);
        hle.dispatch("__aeabi_memset", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load_bytes_for_test(0x1208, 3), &[0x44; 3]);

        cpu.set_reg(0, 4);
        hle.dispatch("malloc", &mut cpu, &mut memory).unwrap();
        let old_ptr = cpu.reg(0);
        memory.load_bytes(old_ptr, b"rust").unwrap();

        cpu.set_reg(0, old_ptr);
        cpu.set_reg(1, 8);
        hle.dispatch("realloc", &mut cpu, &mut memory).unwrap();
        let new_ptr = cpu.reg(0);
        assert_ne!(new_ptr, old_ptr);
        assert_eq!(memory.load_bytes_for_test(new_ptr, 4), b"rust");

        cpu.set_reg(0, 4);
        hle.dispatch("malloc", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), old_ptr);

        cpu.set_reg(0, new_ptr);
        hle.dispatch("free", &mut cpu, &mut memory).unwrap();
        cpu.set_reg(0, 8);
        hle.dispatch("malloc", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), new_ptr);
    }

    #[test]
    fn guest_free_does_not_recycle_pinned_runtime_allocations() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        let pinned = hle.alloc(4, 4).unwrap();
        cpu.set_reg(0, pinned);
        hle.dispatch("free", &mut cpu, &mut memory).unwrap();

        cpu.set_reg(0, 4);
        hle.dispatch("malloc", &mut cpu, &mut memory).unwrap();
        assert_ne!(cpu.reg(0), pinned);
    }

    #[test]
    fn dispatches_ascii_wide_char_locale_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        memory.load_bytes(0x1100, b"space\0").unwrap();
        memory.load_bytes(0x1120, b"Az\0").unwrap();
        memory.store32(0x1200, u32::from(b'A')).unwrap();
        memory.store32(0x1204, u32::from(b'z')).unwrap();
        memory.store32(0x1208, 0).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x800);

        cpu.set_reg(0, u32::from(b'A'));
        hle.dispatch("btowc", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), u32::from(b'A'));

        cpu.set_reg(0, u32::from(b'Z'));
        hle.dispatch("wctob", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), u32::from(b'Z'));

        cpu.set_reg(0, u32::from(b'Q'));
        hle.dispatch("tolower", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), u32::from(b'q'));

        cpu.set_reg(0, 0x1100);
        hle.dispatch("wctype", &mut cpu, &mut memory).unwrap();
        let space_class = cpu.reg(0);
        assert_ne!(space_class, 0);

        cpu.set_reg(0, u32::from(b' '));
        cpu.set_reg(1, space_class);
        hle.dispatch("iswctype", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(0, 0x1300);
        cpu.set_reg(1, 0x1120);
        cpu.set_reg(2, 2);
        cpu.set_reg(3, 0);
        hle.dispatch("mbrtowc", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(memory.load32(0x1300).unwrap(), u32::from(b'A'));

        cpu.set_reg(0, 0x1300);
        cpu.set_reg(1, 0x1120);
        cpu.set_reg(2, 4);
        hle.dispatch("mbstowcs", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 2);
        assert_eq!(memory.load32(0x1300).unwrap(), u32::from(b'A'));
        assert_eq!(memory.load32(0x1304).unwrap(), u32::from(b'z'));
        assert_eq!(memory.load32(0x1308).unwrap(), 0);

        cpu.set_reg(0, 0x1400);
        cpu.set_reg(1, 0x1200);
        cpu.set_reg(2, 4);
        hle.dispatch("wcstombs", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 2);
        assert_eq!(memory.load_bytes_for_test(0x1400, 3), b"Az\0");

        cpu.set_reg(0, 0x1410);
        cpu.set_reg(1, u32::from(b'!'));
        cpu.set_reg(2, 0);
        hle.dispatch("wcrtomb", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(memory.load8(0x1410).unwrap(), b'!');

        cpu.set_reg(0, 0x1200);
        hle.dispatch("wcslen", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 2);
    }

    #[test]
    fn dispatches_libstdcxx_string_replace_aux() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();

        let string = 0x1100;
        let empty_rep = 0x1200;
        let empty_data = empty_rep + 12;
        memory.store32(empty_rep, 0).unwrap();
        memory.store32(empty_rep + 4, 0).unwrap();
        memory.store32(empty_rep + 8, 0).unwrap();
        memory.store32(string, empty_data).unwrap();
        memory.store8(0x1300, b'4').unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(13, 0x1300);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, string);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0);
        cpu.set_reg(3, 1);
        let mut hle = HleRuntime::new(0x1000, 0x2000, 0x1000);

        hle.dispatch("_ZNSs14_M_replace_auxEjjjc", &mut cpu, &mut memory)
            .unwrap();
        let data = memory.load32(string).unwrap();
        assert_ne!(data, empty_data);
        assert_eq!(memory.load32(data - 12).unwrap(), 1);
        assert_eq!(memory.load32(data - 8).unwrap(), 15);
        assert_eq!(memory.load_bytes_for_test(data, 2), b"4\0");
        assert_eq!(cpu.reg(0), string);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);

        memory.store8(0x1300, b'9').unwrap();
        cpu.set_reg(14, 0x3001);
        cpu.set_reg(0, string);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 0);
        cpu.set_reg(3, 1);
        hle.dispatch("_ZNSs14_M_replace_auxEjjjc", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(string).unwrap(), data);
        assert_eq!(memory.load32(data - 12).unwrap(), 2);
        assert_eq!(memory.load_bytes_for_test(data, 3), b"94\0");
    }

    #[test]
    fn dispatches_libstdcxx_string_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x8000).unwrap();
        memory.load_bytes(0x1100, b"stone\0").unwrap();
        memory.load_bytes(0x1110, b"craft\0").unwrap();
        memory.load_bytes(0x1120, b"pick\0").unwrap();
        memory.store32(0x1300, 4).unwrap();

        let first = 0x1200;
        let second = 0x1204;
        let third = 0x1208;
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(13, 0x1300);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x3000, 0x4000);

        cpu.set_reg(0, first);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 0);
        hle.dispatch("_ZNSsC1EPKcRKSaIcE", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, first), b"stone");

        cpu.set_reg(0, second);
        cpu.set_reg(1, first);
        hle.dispatch("_ZNSsC1ERKSs", &mut cpu, &mut memory).unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, second), b"stone");

        cpu.set_reg(0, third);
        cpu.set_reg(1, 0);
        hle.dispatch("_ZNSsC1ERKSs", &mut cpu, &mut memory).unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, third), b"");

        cpu.set_reg(0, second);
        cpu.set_reg(1, 0x1110);
        cpu.set_reg(2, 5);
        hle.dispatch("_ZNSs6appendEPKcj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, second), b"stonecraft");

        cpu.set_reg(0, second);
        cpu.set_reg(1, 0x1110);
        cpu.set_reg(2, 0);
        cpu.set_reg(3, 5);
        hle.dispatch("_ZNKSs4findEPKcjj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 5);

        cpu.set_reg(0, third);
        cpu.set_reg(1, second);
        cpu.set_reg(2, 5);
        cpu.set_reg(3, 5);
        hle.dispatch("_ZNSsC1ERKSsjj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, third), b"craft");

        cpu.set_reg(0, second);
        cpu.set_reg(1, 5);
        cpu.set_reg(2, 5);
        cpu.set_reg(3, 0x1120);
        hle.dispatch("_ZNSs15_M_replace_safeEjjPKcj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, second), b"stonepick");

        let data = memory.load32(second).unwrap();
        cpu.set_reg(0, second);
        cpu.set_reg(1, data + 5);
        cpu.set_reg(2, data + 9);
        hle.dispatch(
            "_ZNSs5eraseEN9__gnu_cxx17__normal_iteratorIPcSsEES2_",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(load_test_cxx_string(&mut memory, second), b"stone");
    }

    #[test]
    fn dispatches_minecraft_webtoken_copy_ctor_for_null_source() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x5000).unwrap();

        let dest = 0x1200;
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, dest);
        cpu.set_reg(1, 0);
        let mut hle = HleRuntime::new(0x1000, 0x3000, 0x2000);

        hle.dispatch("_ZN8WebTokenC2ERKS_", &mut cpu, &mut memory)
            .unwrap();

        assert_eq!(load_test_cxx_string(&mut memory, dest), b"");
        assert_eq!(load_test_cxx_string(&mut memory, dest + 0x18), b"");
        assert_eq!(load_test_cxx_string(&mut memory, dest + 0x30), b"");
        assert_eq!(memory.load_bytes_for_test(dest + 0x08, 16), &[0; 16]);
        assert_eq!(memory.load_bytes_for_test(dest + 0x20, 16), &[0; 16]);
        assert_eq!(cpu.reg(0), dest);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);
    }

    #[test]
    fn dispatches_minecraft_texture_group_pair_facade() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x50000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x3000, 0x48000);

        hle.dispatch(
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation",
            &mut cpu,
            &mut memory,
        )
        .unwrap();

        let pair = cpu.reg(0);
        assert_ne!(pair, 0);
        assert_eq!(memory.load32(pair + 0x38).unwrap(), FAKE_TEXTURE_SIDE);
        assert_eq!(memory.load32(pair + 0x3c).unwrap(), FAKE_TEXTURE_SIDE);
        let pixels = memory.load32(pair + 0x40).unwrap();
        assert_ne!(pixels, 0);
        assert_eq!(
            memory.load32(pixels - CXX_STRING_REP_HEADER_SIZE).unwrap(),
            0
        );
        assert_eq!(memory.load32(pixels - 8).unwrap(), FAKE_TEXTURE_BYTES);
        assert_eq!(memory.load8(pixels).unwrap(), 0);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        hle.dispatch(
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(cpu.reg(0), pair);
        assert_eq!(cpu.pc(), 0x2004);
    }

    #[test]
    fn dispatches_minecraft_geometry_group_facade() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x5000).unwrap();

        let out = 0x1200;
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, out);
        cpu.set_reg(1, 0x1300);
        cpu.set_reg(2, 0x1400);
        let mut hle = HleRuntime::new(0x1000, 0x3000, 0x2000);

        hle.dispatch(
            "_ZN13GeometryGroup11getGeometryERKSs",
            &mut cpu,
            &mut memory,
        )
        .unwrap();

        let geometry = memory.load32(out + 4).unwrap();
        assert_eq!(memory.load32(out).unwrap(), 0);
        assert_ne!(geometry, 0);
        assert_eq!(memory.load32(geometry + 0x14).unwrap(), 0);
        assert_eq!(cpu.reg(0), out);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, out + 8);
        hle.dispatch(
            "_ZN13GeometryGroup14tryGetGeometryERKSs",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(memory.load32(out + 12).unwrap(), geometry);
        assert_eq!(cpu.pc(), 0x2004);
    }

    #[test]
    fn dispatches_minecraft_ui_control_resolve_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x800);

        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0x1200);
        hle.dispatch(
            "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, 0);
        hle.dispatch("_ZN9UIControl18_resolvePostCreateEv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2004);
    }

    #[test]
    fn dispatches_random_device_stdio_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        memory.load_bytes(0x1100, b"/dev/urandom\0").unwrap();
        memory.load_bytes(0x1120, b"rb\0").unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x800);

        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 0x1120);
        hle.dispatch("fopen", &mut cpu, &mut memory).unwrap();
        let file = cpu.reg(0);
        assert_ne!(file, 0);
        let fd = memory
            .load16(file.wrapping_add(FAKE_FILE_FD_OFFSET))
            .unwrap();
        assert_eq!(fd, FIRST_FAKE_FD as u16);

        cpu.set_reg(0, u32::from(fd));
        cpu.set_reg(1, 0x1200);
        cpu.set_reg(2, 4);
        hle.dispatch("read", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 4);
        assert_ne!(memory.load_bytes_for_test(0x1200, 4), [0, 0, 0, 0]);

        cpu.set_reg(0, 0x1210);
        cpu.set_reg(1, 2);
        cpu.set_reg(2, 3);
        cpu.set_reg(3, file);
        hle.dispatch("fread", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 3);
        assert_ne!(memory.load_bytes_for_test(0x1210, 6), [0, 0, 0, 0, 0, 0]);

        memory
            .load_bytes(
                0x1140,
                b"/sdcard/games/com.mojang/minecraftpe/resource_packs.txt\0",
            )
            .unwrap();
        memory.load_bytes(0x1180, b"w\0").unwrap();
        memory.load_bytes(0x1188, b"r+\0").unwrap();
        memory.load_bytes(0x1220, b"pack").unwrap();

        cpu.set_reg(0, 0x1140);
        cpu.set_reg(1, 0x1180);
        hle.dispatch("fopen", &mut cpu, &mut memory).unwrap();
        let writable = cpu.reg(0);
        assert_ne!(writable, 0);

        cpu.set_reg(0, 0x1220);
        cpu.set_reg(1, 1);
        cpu.set_reg(2, 4);
        cpu.set_reg(3, writable);
        hle.dispatch("fwrite", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 4);

        cpu.set_reg(0, writable);
        hle.dispatch("fclose", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(0, 0x1140);
        cpu.set_reg(1, 0x1188);
        hle.dispatch("fopen", &mut cpu, &mut memory).unwrap();
        let readable = cpu.reg(0);
        assert_ne!(readable, 0);

        cpu.set_reg(0, 0x1230);
        cpu.set_reg(1, 1);
        cpu.set_reg(2, 4);
        cpu.set_reg(3, readable);
        hle.dispatch("fread", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 4);
        assert_eq!(memory.load_bytes_for_test(0x1230, 4), b"pack");
    }

    #[test]
    fn dispatches_cxa_guard_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        cpu.set_reg(0, 0x1100);
        hle.dispatch("__cxa_guard_acquire", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(0, 0x1100);
        hle.dispatch("__cxa_guard_release", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load8(0x1100).unwrap(), 1);

        cpu.set_reg(0, 0x1100);
        hle.dispatch("__cxa_guard_acquire", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    trait TestBytes {
        fn load_bytes_for_test(&mut self, addr: u32, len: usize) -> Vec<u8>;
    }

    impl TestBytes for MappedMemory {
        fn load_bytes_for_test(&mut self, addr: u32, len: usize) -> Vec<u8> {
            (0..len)
                .map(|idx| self.load8(addr.wrapping_add(idx as u32)).unwrap())
                .collect()
        }
    }

    fn stored_zip_with_one_file(name: &str, contents: &[u8]) -> Vec<u8> {
        let name = name.as_bytes();
        let mut bytes = Vec::new();
        let local_offset = bytes.len() as u32;
        push_u32(&mut bytes, 0x0403_4b50);
        push_u16(&mut bytes, 20);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, contents.len() as u32);
        push_u32(&mut bytes, contents.len() as u32);
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        bytes.extend_from_slice(name);
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
        push_u16(&mut bytes, name.len() as u16);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, local_offset);
        bytes.extend_from_slice(name);

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

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
}
