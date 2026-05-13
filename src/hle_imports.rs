use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

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
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: u32 = 0x2601;
const GL_REPEAT: u32 = 0x2901;
const GL_MAX_TEXTURE_IMAGE_UNITS: u32 = 0x8872;
const GL_MAX_VERTEX_ATTRIBS: u32 = 0x8869;
const GL_FLOAT: u32 = 0x1406;
const GL_INT: u32 = 0x1404;
const GL_COMPILE_STATUS: u32 = 0x8b81;
const GL_LINK_STATUS: u32 = 0x8b82;
const GL_INFO_LOG_LENGTH: u32 = 0x8b84;
const GL_ACTIVE_UNIFORMS: u32 = 0x8b86;
const GL_ACTIVE_UNIFORM_MAX_LENGTH: u32 = 0x8b87;
const GL_ACTIVE_ATTRIBUTES: u32 = 0x8b89;
const GL_ACTIVE_ATTRIBUTE_MAX_LENGTH: u32 = 0x8b8a;
const GL_FLOAT_VEC2: u32 = 0x8b50;
const GL_FLOAT_VEC3: u32 = 0x8b51;
const GL_FLOAT_VEC4: u32 = 0x8b52;
const GL_INT_VEC2: u32 = 0x8b53;
const GL_INT_VEC3: u32 = 0x8b54;
const GL_INT_VEC4: u32 = 0x8b55;
const GL_BOOL: u32 = 0x8b56;
const GL_BOOL_VEC2: u32 = 0x8b57;
const GL_BOOL_VEC3: u32 = 0x8b58;
const GL_BOOL_VEC4: u32 = 0x8b59;
const GL_FLOAT_MAT2: u32 = 0x8b5a;
const GL_FLOAT_MAT3: u32 = 0x8b5b;
const GL_FLOAT_MAT4: u32 = 0x8b5c;
const GL_SAMPLER_2D: u32 = 0x8b5e;
const GL_SAMPLER_CUBE: u32 = 0x8b60;
const GL_SHADING_LANGUAGE_VERSION: u32 = 0x8b8c;
const GL_LOW_FLOAT: u32 = 0x8df0;
const GL_MEDIUM_FLOAT: u32 = 0x8df1;
const GL_HIGH_FLOAT: u32 = 0x8df2;
const GL_LOW_INT: u32 = 0x8df3;
const GL_MEDIUM_INT: u32 = 0x8df4;
const GL_HIGH_INT: u32 = 0x8df5;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8cd5;
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
static HLE_STRING_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
const FAKE_TIME_BASE_SECS: u64 = 1_600_000_000;
const FAKE_TIME_STEP_NANOS: u64 = 16_666_667;
const GLES_EVENT_LIMIT: usize = 4096;

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
    gl_shaders: Vec<GlShader>,
    gl_programs: Vec<GlProgram>,
    gles_events: VecDeque<GlesEvent>,
    next_fd: u32,
    files: Vec<FakeFile>,
    virtual_files: Vec<VirtualFile>,
    current_pthread: u32,
    created_pthreads: VecDeque<CreatedPthread>,
    next_pthread_key: u32,
    pthread_specific: Vec<PthreadSpecific>,
    native_activity: Option<u32>,
    alooper_events: VecDeque<u32>,
    unwind_tables: Vec<HleUnwindTable>,
    random_state: u32,
    clock_ns: u64,
    fake_geometry: Option<u32>,
    fake_texture_pair: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedPthread {
    pub id: u32,
    pub start: u32,
    pub arg: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HleUnwindTable {
    pub memory_base: u32,
    pub memory_end: u32,
    pub exidx_addr: u32,
    pub exidx_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlesEvent {
    ClearColor {
        red: u32,
        green: u32,
        blue: u32,
        alpha: u32,
    },
    ClearDepthf {
        depth: u32,
    },
    Clear {
        mask: u32,
    },
    Viewport {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    DrawArrays {
        mode: u32,
        first: i32,
        count: i32,
    },
    DrawElements {
        mode: u32,
        count: i32,
        ty: u32,
        indices: u32,
    },
    SwapBuffers {
        display: u32,
        surface: u32,
    },
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
struct GlShader {
    name: u32,
    shader_type: u32,
    source: String,
}

#[derive(Debug, Clone)]
struct GlProgram {
    name: u32,
    shaders: Vec<u32>,
    uniforms: Vec<GlActive>,
    attributes: Vec<GlActive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GlActive {
    name: String,
    size: u32,
    ty: u32,
    location: u32,
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
    thread: u32,
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
            gl_shaders: Vec::new(),
            gl_programs: Vec::new(),
            gles_events: VecDeque::new(),
            next_fd: FIRST_FAKE_FD,
            files: Vec::new(),
            virtual_files: Vec::new(),
            current_pthread: 1,
            created_pthreads: VecDeque::new(),
            next_pthread_key: 0,
            pthread_specific: Vec::new(),
            native_activity: None,
            alooper_events: VecDeque::new(),
            unwind_tables: Vec::new(),
            random_state: 0x1234_5678,
            clock_ns: 0,
            fake_geometry: None,
            fake_texture_pair: None,
        }
    }

    pub fn set_apk_path(&mut self, apk_path: PathBuf) {
        self.apk_path = Some(apk_path);
    }

    pub fn set_unwind_tables(&mut self, unwind_tables: Vec<HleUnwindTable>) {
        self.unwind_tables = unwind_tables;
    }

    pub(crate) fn has_created_pthreads(&self) -> bool {
        !self.created_pthreads.is_empty()
    }

    pub(crate) fn take_created_pthreads(&mut self) -> Vec<CreatedPthread> {
        self.created_pthreads.drain(..).collect()
    }

    pub fn take_gles_events(&mut self) -> Vec<GlesEvent> {
        self.gles_events.drain(..).collect()
    }

    pub(crate) fn current_pthread(&self) -> u32 {
        self.current_pthread
    }

    pub(crate) fn set_current_pthread(&mut self, thread: u32) {
        self.current_pthread = thread;
    }

    pub fn set_native_activity(&mut self, activity: u32) {
        self.native_activity = Some(activity);
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
            "strstr" => self.strstr(cpu, memory),
            "strspn" => self.strspn(cpu, memory),
            "strcspn" => self.strcspn(cpu, memory),
            "strpbrk" => self.strpbrk(cpu, memory),
            "strdup" => self.strdup(cpu, memory),
            "strcasecmp" => self.strcasecmp(cpu, memory),
            "strncasecmp" => self.strncasecmp(cpu, memory),
            "strtod" => self.strtod(cpu, memory),
            "strtof" => self.strtof(cpu, memory),
            "atof" => self.atof(cpu, memory),
            "strtol" => self.strtol(cpu, memory),
            "strtoul" => self.strtoul(cpu, memory),
            "strtoull" => self.strtoull(cpu, memory),
            "atoi" => self.atoi(cpu, memory),
            "sscanf" => self.sscanf(cpu, memory),
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
            "__gnu_Unwind_Find_exidx" => self.gnu_unwind_find_exidx(cpu, memory),
            "__aeabi_idiv" => self.aeabi_idiv(cpu),
            "__aeabi_uidiv" => self.aeabi_uidiv(cpu),
            "__aeabi_idivmod" => self.aeabi_idivmod(cpu),
            "__aeabi_uidivmod" => self.aeabi_uidivmod(cpu),
            "__divsi3" => self.aeabi_idiv(cpu),
            "__udivsi3" => self.aeabi_uidiv(cpu),
            "__modsi3" => self.modsi3(cpu),
            "__umodsi3" => self.umodsi3(cpu),
            name if descriptor.kind == HleSymbolKind::Libm => self.libm(name, cpu),
            "getauxval" => Ok(self.return32(cpu, self.getauxval(cpu.reg(0)))),
            "gettimeofday" => self.gettimeofday(cpu, memory),
            "clock_gettime" => self.clock_gettime(cpu, memory),
            "time" => self.time(cpu, memory),
            "sysconf" => self.sysconf(cpu, memory),
            "pthread_self" => Ok(self.return32(cpu, self.current_pthread)),
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
            "fputs" => self.fputs_call(cpu, memory),
            "fputc" => self.fputc_call(cpu, memory),
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
            "_ZSt11_Hash_bytesPKvjj" => self.libstdcxx_hash_bytes(cpu, memory),
            "_ZSt15_Fnv_hash_bytesPKvjj" => self.libstdcxx_fnv_hash_bytes(cpu, memory),
            "_ZN8WebTokenC1ERKS_" | "_ZN8WebTokenC2ERKS_" => {
                self.minecraft_webtoken_copy_ctor(cpu, memory)
            }
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation" => {
                self.minecraft_texture_group_get_texture_pair(cpu, memory)
            }
            "_ZNK3mce12TextureGroup8isLoadedERK16ResourceLocation" => Ok(self.return32(cpu, 1)),
            "_ZN13GeometryGroup11getGeometryERKSs" | "_ZN13GeometryGroup14tryGetGeometryERKSs" => {
                self.minecraft_geometry_group_get_geometry(cpu, memory)
            }
            "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E"
            | "_ZN9UIControl18_resolvePostCreateEv" => self.minecraft_ui_control_resolve_noop(cpu),
            "_ZN14GamePadManager16getGamePadsInUseEv"
            | "_ZN14GamePadManager20getConnectedGamePadsEv" => {
                self.minecraft_empty_vector_return(cpu, memory)
            }
            "_ZN13GamePadMapper4tickER15InputEventQueue"
            | "_ZN13GamePadMapper8tickTurnER15InputEventQueue" => Ok(self.return32(cpu, 0)),
            "_ZNK7GamePad11isConnectedEv" | "_ZNK7GamePad7isInUseEv" => Ok(self.return32(cpu, 0)),
            "_ZN6Screen15controllerEventEv"
            | "_ZN6Screen27_processControllerDirectionEi"
            | "_ZN11MenuGamePad12getDirectionEi"
            | "_ZN11MenuGamePad4getXEi"
            | "_ZN11MenuGamePad4getYEi"
            | "_ZN11MenuGamePad9isTouchedEi" => Ok(self.return32(cpu, 0)),
            "_ZN14KeyboardMapper21clearInputDeviceQueueEv"
            | "_ZN14KeyboardMapper4tickER15InputEventQueue"
            | "_ZN11MouseMapper21clearInputDeviceQueueEv"
            | "_ZN11MouseMapper4tickER15InputEventQueue"
            | "_ZN11TouchMapper21clearInputDeviceQueueEv"
            | "_ZN11TouchMapper4tickER15InputEventQueue"
            | "_ZN19TestAutoInputMapper21clearInputDeviceQueueEv"
            | "_ZN19TestAutoInputMapper4tickER15InputEventQueue"
            | "_ZN18DeviceButtonMapper4tickER15InputEventQueue"
            | "_ZN22GazeGestureVoiceMapper21clearInputDeviceQueueEv"
            | "_ZN22GazeGestureVoiceMapper4tickER15InputEventQueue"
            | "_ZN11MouseDevice12isButtonDownEi"
            | "_ZN11MouseDevice14getButtonStateEi"
            | "_ZN11MouseDevice14getEventButtonEv"
            | "_ZN11MouseDevice16wasFirstMovementEv"
            | "_ZN11MouseDevice19getEventButtonStateEv"
            | "_ZN11MouseDevice4getXEv"
            | "_ZN11MouseDevice4getYEv"
            | "_ZN11MouseDevice4nextEv"
            | "_ZN11MouseDevice5getDXEv"
            | "_ZN11MouseDevice5getDYEv"
            | "_ZN11MouseDevice5resetEv"
            | "_ZN11MouseDevice6reset2Ev"
            | "_ZN11MouseDevice6rewindEv"
            | "_ZN11MouseDevice8getEventEv"
            | "_ZN10Multitouch10isReleasedEi"
            | "_ZN10Multitouch11isEdgeTouchEi"
            | "_ZN10Multitouch13isPointerDownEi"
            | "_ZN10Multitouch15resetThisUpdateEv"
            | "_ZN10Multitouch19isPressedThisUpdateEi"
            | "_ZN10Multitouch20isReleasedThisUpdateEi"
            | "_ZN10Multitouch4nextEv"
            | "_ZN10Multitouch5resetEv"
            | "_ZN10Multitouch6commitEv"
            | "_ZN10Multitouch9isPressedEi" => Ok(self.return32(cpu, 0)),
            "_ZN10Multitouch19getActivePointerIdsEPPKi"
            | "_ZN10Multitouch29getActivePointerIdsThisUpdateEPPKi" => {
                self.minecraft_empty_pointer_ids_return(cpu, memory)
            }
            "_ZN10Multitouch25getFirstActivePointerIdExEv"
            | "_ZN10Multitouch35getFirstActivePointerIdExThisUpdateEv" => {
                Ok(self.return32(cpu, u32::MAX))
            }
            "_ZN3mce11MathUtility21interpolateTransformsERN3glm6detail7tmat4x4IfEERKS4_S7_f" => {
                self.minecraft_interpolate_transforms(cpu, memory)
            }
            "_ZN3mce16RenderContextOGL17unbindAllTexturesEv" => {
                self.minecraft_ogl_unbind_all_textures(cpu, memory)
            }
            "_ZN12ProfilerLite4tickEbb" | "_ZN12ProfilerLite9_endScopeENS_5ScopeEdd" => {
                Ok(self.return32(cpu, 0))
            }
            "_ZN18MinecraftTelemetry4tickEv"
            | "_ZN18MinecraftTelemetry15forceSendEventsEv"
            | "_ZN6Social11Multiplayer18needToHandleInviteEv"
            | "_ZN6Social11Multiplayer4tickEb"
            | "_ZN6Social11Multiplayer22tickMultiplayerManagerEv"
            | "_ZN6Social11UserManager12silentSigninESt8functionIFvNS_12SignInResultEEE"
            | "_ZN6Social11UserManager21registerSignInHandlerESt8functionIFvvEE"
            | "_ZN6Social11UserManager22registerSignOutHandlerESt8functionIFvvEE"
            | "_ZN6Social11UserManager4tickEv"
            | "_ZNK6Social11UserManager10isSignedInEv"
            | "_ZN9RealmsAPI6updateEv" => Ok(self.return32(cpu, 0)),
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

    fn gnu_unwind_find_exidx<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let return_address = cpu.reg(0) & !1;
        let count_ptr = cpu.reg(1);
        let table = self
            .unwind_tables
            .iter()
            .find(|table| return_address >= table.memory_base && return_address < table.memory_end);
        let (addr, count) = table.map_or((0, 0), |table| (table.exidx_addr, table.exidx_count));
        if count_ptr != 0 {
            store32(memory, count_ptr, count)?;
        }
        Ok(self.return32(cpu, addr))
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

    fn strstr<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let haystack_ptr = cpu.reg(0);
        let needle_ptr = cpu.reg(1);
        let haystack_len = strlen(memory, haystack_ptr)?;
        let needle_len = strlen(memory, needle_ptr)?;
        if needle_len == 0 {
            return Ok(self.return32(cpu, haystack_ptr));
        }
        if needle_len > haystack_len {
            return Ok(self.return32(cpu, 0));
        }
        let haystack = load_bytes(memory, haystack_ptr, haystack_len)?;
        let needle = load_bytes(memory, needle_ptr, needle_len)?;
        let found = haystack
            .windows(needle.len())
            .position(|window| window == needle.as_slice())
            .map_or(0, |idx| haystack_ptr.wrapping_add(idx as u32));
        Ok(self.return32(cpu, found))
    }

    fn strspn<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let accept = c_string_byte_set(memory, cpu.reg(1))?;
        let mut off = 0u32;
        loop {
            let byte = load8(memory, ptr.wrapping_add(off))?;
            if byte == 0 || !accept[byte as usize] {
                return Ok(self.return32(cpu, off));
            }
            off = off.wrapping_add(1);
        }
    }

    fn strcspn<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let reject = c_string_byte_set(memory, cpu.reg(1))?;
        let mut off = 0u32;
        loop {
            let byte = load8(memory, ptr.wrapping_add(off))?;
            if byte == 0 || reject[byte as usize] {
                return Ok(self.return32(cpu, off));
            }
            off = off.wrapping_add(1);
        }
    }

    fn strpbrk<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let accept = c_string_byte_set(memory, cpu.reg(1))?;
        let mut off = 0u32;
        loop {
            let byte = load8(memory, ptr.wrapping_add(off))?;
            if byte == 0 {
                return Ok(self.return32(cpu, 0));
            }
            if accept[byte as usize] {
                return Ok(self.return32(cpu, ptr.wrapping_add(off)));
            }
            off = off.wrapping_add(1);
        }
    }

    fn strdup<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let src = cpu.reg(0);
        let len = strlen(memory, src)?;
        let dst = self.alloc_guest(len.wrapping_add(1), 1)?;
        for idx in 0..=len {
            let byte = load8(memory, src.wrapping_add(idx))?;
            store8(memory, dst.wrapping_add(idx), byte)?;
        }
        Ok(self.return32(cpu, dst))
    }

    fn strcasecmp<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let result = ascii_strcasecmp(memory, cpu.reg(0), cpu.reg(1), u32::MAX)?;
        Ok(self.return32(cpu, i32_to_u32(result)))
    }

    fn strncasecmp<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let result = ascii_strcasecmp(memory, cpu.reg(0), cpu.reg(1), cpu.reg(2))?;
        Ok(self.return32(cpu, i32_to_u32(result)))
    }

    fn strtod<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let endptr = cpu.reg(1);
        let parsed = parse_c_float(memory, ptr)?;
        if endptr != 0 {
            store32(memory, endptr, ptr.wrapping_add(parsed.consumed))?;
        }
        Ok(self.return_f64(cpu, parsed.value))
    }

    fn strtof<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let endptr = cpu.reg(1);
        let parsed = parse_c_float(memory, ptr)?;
        if endptr != 0 {
            store32(memory, endptr, ptr.wrapping_add(parsed.consumed))?;
        }
        Ok(self.return_f32(cpu, parsed.value as f32))
    }

    fn atof<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let parsed = parse_c_float(memory, cpu.reg(0))?;
        Ok(self.return_f64(cpu, parsed.value))
    }

    fn strtol<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let endptr = cpu.reg(1);
        let parsed = parse_c_integer(memory, ptr, cpu.reg(2))?;
        if endptr != 0 {
            store32(memory, endptr, ptr.wrapping_add(parsed.consumed))?;
        }
        Ok(self.return32(cpu, parsed.as_i32() as u32))
    }

    fn strtoul<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let endptr = cpu.reg(1);
        let parsed = parse_c_integer(memory, ptr, cpu.reg(2))?;
        if endptr != 0 {
            store32(memory, endptr, ptr.wrapping_add(parsed.consumed))?;
        }
        Ok(self.return32(cpu, parsed.as_u32()))
    }

    fn strtoull<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let endptr = cpu.reg(1);
        let parsed = parse_c_integer(memory, ptr, cpu.reg(2))?;
        if endptr != 0 {
            store32(memory, endptr, ptr.wrapping_add(parsed.consumed))?;
        }
        Ok(self.return_u64(cpu, parsed.as_u64()))
    }

    fn atoi<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let parsed = parse_c_integer(memory, cpu.reg(0), 10)?;
        Ok(self.return32(cpu, parsed.as_i32() as u32))
    }

    fn sscanf<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let input = load_c_string_bytes(memory, cpu.reg(0), 4096)?;
        let format = load_c_string_bytes(memory, cpu.reg(1), 512)?;
        let assigned = scan_input(memory, cpu, &input, &format)?;
        Ok(self.return32(cpu, assigned))
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

    fn modsi3(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0) as i32;
        let rhs = cpu.reg(1) as i32;
        let result = if rhs == 0 { 0 } else { lhs.wrapping_rem(rhs) };
        Ok(self.return32(cpu, result as u32))
    }

    fn umodsi3(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let lhs = cpu.reg(0);
        let rhs = cpu.reg(1);
        let result = if rhs == 0 { 0 } else { lhs % rhs };
        Ok(self.return32(cpu, result))
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
        let start = cpu.reg(2);
        let arg = cpu.reg(3);
        let id = self.alloc_fd();
        if thread_out != 0 {
            store32(memory, thread_out, id)?;
        }

        // Android's native_app_glue waits for the created thread to mark
        // android_app.running before ANativeActivity_onCreate returns. Other
        // thread arguments may be game worker objects, so only touch app-like
        // structs that point back to the registered ANativeActivity.
        if arg != 0 && self.is_native_app_thread_arg(memory, arg) {
            store32(memory, arg.wrapping_add(0x6c), 1)?;
        } else if start != 0 {
            self.created_pthreads
                .push_back(CreatedPthread { id, start, arg });
        }
        Ok(self.return32(cpu, 0))
    }

    fn is_native_app_thread_arg<M: Memory>(&self, memory: &mut M, arg: u32) -> bool {
        let Some(activity) = self.native_activity else {
            return false;
        };
        load32(memory, arg.wrapping_add(0x0c)).is_ok_and(|candidate| candidate == activity)
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
            .find(|entry| entry.thread == self.current_pthread && entry.key == key)
            .map(|entry| entry.value)
            .unwrap_or(0)
    }

    fn pthread_setspecific(&mut self, cpu: &mut Cpu) -> Result<(), HleError> {
        let key = cpu.reg(0);
        let value = cpu.reg(1);
        if let Some(entry) = self
            .pthread_specific
            .iter_mut()
            .find(|entry| entry.thread == self.current_pthread && entry.key == key)
        {
            entry.value = value;
        } else if value != 0 {
            self.pthread_specific.push(PthreadSpecific {
                thread: self.current_pthread,
                key,
                value,
            });
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
        trace_hle_write("write", memory, buf, count);
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
        trace_hle_write("fwrite", memory, ptr, total);
        let written = self.write_fake_fd(memory, fd, ptr, total)?;
        Ok(self.return32(cpu, written / size))
    }

    fn fputs_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let stream = cpu.reg(1);
        if ptr == 0 || stream == 0 {
            return Ok(self.return32(cpu, u32::MAX));
        }
        let len = strlen(memory, ptr)?;
        trace_hle_write("fputs", memory, ptr, len);
        let Ok(fd) = self.fake_file_fd(memory, stream) else {
            return Ok(self.return32(cpu, 0));
        };
        self.write_fake_fd(memory, fd, ptr, len)?;
        Ok(self.return32(cpu, 0))
    }

    fn fputc_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ch = cpu.reg(0) as u8;
        let stream = cpu.reg(1);
        if stream == 0 {
            return Ok(self.return32(cpu, u32::MAX));
        }
        if std::env::var_os("AEMU_TRACE_HLE_FILE").is_some() {
            let printable = if ch.is_ascii_graphic() || ch == b' ' {
                char::from(ch).to_string()
            } else {
                format!("\\x{ch:02x}")
            };
            eprintln!("HLE file fputc {printable}");
        }
        let Ok(fd) = self.fake_file_fd(memory, stream) else {
            return Ok(self.return32(cpu, u32::from(ch)));
        };
        let scratch = self.alloc(1, 1)?;
        store8(memory, scratch, ch)?;
        self.write_fake_fd(memory, fd, scratch, 1)?;
        Ok(self.return32(cpu, u32::from(ch)))
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

    fn libstdcxx_hash_bytes<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let value = libstdcxx_hash_bytes(memory, cpu.reg(0), cpu.reg(1), cpu.reg(2))?;
        Ok(self.return32(cpu, value))
    }

    fn libstdcxx_fnv_hash_bytes<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let value = libstdcxx_fnv_hash_bytes(memory, cpu.reg(0), cpu.reg(1), cpu.reg(2))?;
        Ok(self.return32(cpu, value))
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
        let lhs_ptr = cpu.reg(0);
        let rhs_ptr = cpu.reg(1);
        let lhs = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let rhs_len = strlen(memory, cpu.reg(1))?;
        let rhs = load_bytes(memory, cpu.reg(1), rhs_len)?;
        let result = compare_bytes(&lhs, &rhs);
        trace_hle_string_compare("_ZNKSs7compareEPKc", lhs_ptr, rhs_ptr, &lhs, &rhs, result);
        Ok(self.return32(cpu, i32_to_u32(result)))
    }

    fn libstdcxx_string_compare_string<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let lhs_ptr = cpu.reg(0);
        let rhs_ptr = cpu.reg(1);
        let lhs = load_cxx_string_bytes(memory, cpu.reg(0))?;
        let rhs = load_cxx_string_bytes(memory, cpu.reg(1))?;
        let result = compare_bytes(&lhs, &rhs);
        trace_hle_string_compare("_ZNKSs7compareERKSs", lhs_ptr, rhs_ptr, &lhs, &rhs, result);
        Ok(self.return32(cpu, i32_to_u32(result)))
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

    fn minecraft_empty_vector_return<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let out = cpu.reg(0);
        store32(memory, out, 0)?;
        store32(memory, out.wrapping_add(4), 0)?;
        store32(memory, out.wrapping_add(8), 0)?;
        Ok(self.return32(cpu, out))
    }

    fn minecraft_empty_pointer_ids_return<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let out = cpu.reg(0);
        if out != 0 {
            store32(memory, out, 0)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn minecraft_interpolate_transforms<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let out = cpu.reg(0);
        let from = cpu.reg(1);
        let to = cpu.reg(2);
        let t = f32::from_bits(cpu.reg(3));

        for idx in 0..16 {
            let offset = idx * 4;
            let a = f32::from_bits(load32(memory, from.wrapping_add(offset))?);
            let b = f32::from_bits(load32(memory, to.wrapping_add(offset))?);
            let value = a + (b - a) * t;
            store32(memory, out.wrapping_add(offset), value.to_bits())?;
        }

        Ok(self.return32(cpu, out))
    }

    fn minecraft_ogl_unbind_all_textures<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let context = cpu.reg(0);
        for slot in 0..8 {
            store32(memory, context.wrapping_add(0x7c + slot * 4), 0)?;
        }
        store32(memory, context.wrapping_add(0x100), 0x84c7)?;
        Ok(self.return32(cpu, context))
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
            "eglSwapBuffers" => {
                self.push_gles_event(GlesEvent::SwapBuffers {
                    display: cpu.reg(0),
                    surface: cpu.reg(1),
                });
                Ok(self.return32(cpu, 1))
            }
            "eglBindAPI" | "eglMakeCurrent" | "eglSwapInterval" | "eglDestroySurface"
            | "eglDestroyContext" | "eglTerminate" | "eglReleaseThread" | "eglSurfaceAttrib"
            | "eglWaitGL" | "eglWaitNative" => Ok(self.return32(cpu, 1)),
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
            "glCreateProgram" => {
                let value = self.alloc_gl_name();
                self.gl_programs.push(GlProgram {
                    name: value,
                    shaders: Vec::new(),
                    uniforms: Vec::new(),
                    attributes: Vec::new(),
                });
                Ok(self.return32(cpu, value))
            }
            "glCreateShader" => {
                let value = self.alloc_gl_name();
                self.gl_shaders.push(GlShader {
                    name: value,
                    shader_type: cpu.reg(0),
                    source: String::new(),
                });
                Ok(self.return32(cpu, value))
            }
            "glShaderSource" => self.gl_shader_source(cpu, memory),
            "glAttachShader" => {
                let program = cpu.reg(0);
                let shader = cpu.reg(1);
                if let Some(program) = self
                    .gl_programs
                    .iter_mut()
                    .find(|item| item.name == program)
                {
                    if !program.shaders.contains(&shader) {
                        program.shaders.push(shader);
                    }
                }
                Ok(self.return32(cpu, 0))
            }
            "glLinkProgram" => {
                self.gl_link_program(cpu.reg(0));
                Ok(self.return32(cpu, 0))
            }
            "glGenBuffers" | "glGenFramebuffers" | "glGenRenderbuffers" | "glGenTextures" => {
                self.gl_gen_names(cpu, memory)
            }
            "glClearColor" => {
                self.push_gles_event(GlesEvent::ClearColor {
                    red: cpu.reg(0),
                    green: cpu.reg(1),
                    blue: cpu.reg(2),
                    alpha: cpu.reg(3),
                });
                Ok(self.return32(cpu, 0))
            }
            "glClearDepthf" => {
                self.push_gles_event(GlesEvent::ClearDepthf { depth: cpu.reg(0) });
                Ok(self.return32(cpu, 0))
            }
            "glClear" => {
                self.push_gles_event(GlesEvent::Clear { mask: cpu.reg(0) });
                Ok(self.return32(cpu, 0))
            }
            "glViewport" => {
                self.push_gles_event(GlesEvent::Viewport {
                    x: cpu.reg(0) as i32,
                    y: cpu.reg(1) as i32,
                    width: cpu.reg(2) as i32,
                    height: cpu.reg(3) as i32,
                });
                Ok(self.return32(cpu, 0))
            }
            "glDrawArrays" => {
                self.push_gles_event(GlesEvent::DrawArrays {
                    mode: cpu.reg(0),
                    first: cpu.reg(1) as i32,
                    count: cpu.reg(2) as i32,
                });
                Ok(self.return32(cpu, 0))
            }
            "glDrawElements" => {
                self.push_gles_event(GlesEvent::DrawElements {
                    mode: cpu.reg(0),
                    count: cpu.reg(1) as i32,
                    ty: cpu.reg(2),
                    indices: cpu.reg(3),
                });
                Ok(self.return32(cpu, 0))
            }
            "glCheckFramebufferStatus" => Ok(self.return32(cpu, GL_FRAMEBUFFER_COMPLETE)),
            "glGetString" => {
                let Some(value) = gl_query_string(cpu.reg(0)) else {
                    return Ok(self.return32(cpu, 0));
                };
                let ptr = self.alloc_c_string(memory, value)?;
                Ok(self.return32(cpu, ptr))
            }
            "glGetError" => Ok(self.return32(cpu, 0)),
            "glGetProgramiv" => {
                let value = self.gl_program_iv(cpu.reg(0), cpu.reg(1));
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
            "glGetShaderPrecisionFormat" => {
                let (range_min, range_max, precision) = gl_shader_precision(cpu.reg(1));
                let range_ptr = cpu.reg(2);
                let precision_ptr = cpu.reg(3);
                if range_ptr != 0 {
                    store32(memory, range_ptr, range_min)?;
                    store32(memory, range_ptr.wrapping_add(4), range_max)?;
                }
                if precision_ptr != 0 {
                    store32(memory, precision_ptr, precision)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetTexParameteriv" => {
                let value = gl_tex_parameter_iv(cpu.reg(1));
                if cpu.reg(2) != 0 {
                    store32(memory, cpu.reg(2), value)?;
                }
                Ok(self.return32(cpu, 0))
            }
            "glGetActiveUniform" => self.gl_get_active(cpu, memory, true),
            "glGetActiveAttrib" => self.gl_get_active(cpu, memory, false),
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
            "glGetAttribLocation" => self.gl_get_location(cpu, memory, false),
            "glGetUniformLocation" => self.gl_get_location(cpu, memory, true),
            "glIsTexture" => Ok(self.return32(cpu, u32::from(cpu.reg(0) != 0))),
            _ => Ok(self.return32(cpu, 0)),
        }
    }

    fn alloc_gl_name(&mut self) -> u32 {
        let value = self.next_gl_name;
        self.next_gl_name = self.next_gl_name.wrapping_add(1).max(1);
        value
    }

    fn push_gles_event(&mut self, event: GlesEvent) {
        if self.gles_events.len() == GLES_EVENT_LIMIT {
            self.gles_events.pop_front();
        }
        self.gles_events.push_back(event);
    }

    fn gl_gen_names<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let count = cpu.reg(0);
        let out = cpu.reg(1);
        if out != 0 {
            for idx in 0..count {
                let value = self.alloc_gl_name();
                store32(memory, out.wrapping_add(idx.wrapping_mul(4)), value)?;
            }
        }
        Ok(self.return32(cpu, 0))
    }

    fn gl_shader_source<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        let shader_name = cpu.reg(0);
        let count = cpu.reg(1);
        let strings = cpu.reg(2);
        let lengths = cpu.reg(3);
        let mut source = String::new();
        for idx in 0..count {
            let string_ptr = load32(memory, strings.wrapping_add(idx.wrapping_mul(4)))?;
            if string_ptr == 0 {
                continue;
            }
            let bytes = if lengths != 0 {
                let raw_len = load32(memory, lengths.wrapping_add(idx.wrapping_mul(4)))?;
                if (raw_len as i32) >= 0 {
                    load_bytes(memory, string_ptr, raw_len)?
                } else {
                    load_c_string(memory, string_ptr, 64 * 1024)?.into_bytes()
                }
            } else {
                load_c_string(memory, string_ptr, 64 * 1024)?.into_bytes()
            };
            source.push_str(&String::from_utf8_lossy(&bytes));
        }
        if let Some(shader) = self
            .gl_shaders
            .iter_mut()
            .find(|shader| shader.name == shader_name)
        {
            shader.source = source;
        }
        Ok(self.return32(cpu, 0))
    }

    fn gl_link_program(&mut self, program_name: u32) {
        let Some(shader_names) = self
            .gl_programs
            .iter()
            .find(|program| program.name == program_name)
            .map(|program| program.shaders.clone())
        else {
            return;
        };
        let mut sources = Vec::new();
        for shader_name in shader_names {
            if let Some(shader) = self
                .gl_shaders
                .iter()
                .find(|shader| shader.name == shader_name)
            {
                sources.push((shader.shader_type, shader.source.as_str()));
            }
        }
        let uniforms = reflect_glsl_uniforms(&sources);
        let attributes = reflect_glsl_attributes(&sources);
        if let Some(program) = self
            .gl_programs
            .iter_mut()
            .find(|program| program.name == program_name)
        {
            program.uniforms = uniforms;
            program.attributes = attributes;
        }
    }

    fn gl_program_iv(&self, program_name: u32, name: u32) -> u32 {
        let program = self
            .gl_programs
            .iter()
            .find(|program| program.name == program_name);
        match name {
            GL_LINK_STATUS => 1,
            GL_INFO_LOG_LENGTH => 0,
            GL_ACTIVE_UNIFORMS => program.map_or(0, |program| program.uniforms.len() as u32),
            GL_ACTIVE_UNIFORM_MAX_LENGTH => program
                .and_then(|program| active_max_name_len(&program.uniforms))
                .unwrap_or(0),
            GL_ACTIVE_ATTRIBUTES => program.map_or(0, |program| program.attributes.len() as u32),
            GL_ACTIVE_ATTRIBUTE_MAX_LENGTH => program
                .and_then(|program| active_max_name_len(&program.attributes))
                .unwrap_or(0),
            _ => 0,
        }
    }

    fn gl_get_active<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
        uniform: bool,
    ) -> Result<(), HleError> {
        let program_name = cpu.reg(0);
        let index = cpu.reg(1) as usize;
        let buf_size = cpu.reg(2);
        let length_ptr = cpu.reg(3);
        let size_ptr = load32(memory, cpu.reg(13)).unwrap_or(0);
        let type_ptr = load32(memory, cpu.reg(13).wrapping_add(4)).unwrap_or(0);
        let name_ptr = load32(memory, cpu.reg(13).wrapping_add(8)).unwrap_or(0);
        let active = self
            .gl_programs
            .iter()
            .find(|program| program.name == program_name)
            .and_then(|program| {
                if uniform {
                    program.uniforms.get(index)
                } else {
                    program.attributes.get(index)
                }
            });
        if let Some(active) = active {
            let written = write_gl_name(memory, name_ptr, buf_size, &active.name)?;
            if length_ptr != 0 {
                store32(memory, length_ptr, written)?;
            }
            if size_ptr != 0 {
                store32(memory, size_ptr, active.size)?;
            }
            if type_ptr != 0 {
                store32(memory, type_ptr, active.ty)?;
            }
        } else {
            if length_ptr != 0 {
                store32(memory, length_ptr, 0)?;
            }
            if size_ptr != 0 {
                store32(memory, size_ptr, 0)?;
            }
            if type_ptr != 0 {
                store32(memory, type_ptr, 0)?;
            }
            if name_ptr != 0 && buf_size != 0 {
                store8(memory, name_ptr, 0)?;
            }
        }
        Ok(self.return32(cpu, 0))
    }

    fn gl_get_location<M: Memory>(
        &mut self,
        cpu: &mut Cpu,
        memory: &mut M,
        uniform: bool,
    ) -> Result<(), HleError> {
        let program_name = cpu.reg(0);
        let name = load_c_string(memory, cpu.reg(1), 1024).unwrap_or_default();
        let location = self
            .gl_programs
            .iter()
            .find(|program| program.name == program_name)
            .and_then(|program| {
                let active = if uniform {
                    &program.uniforms
                } else {
                    &program.attributes
                };
                active
                    .iter()
                    .find(|item| active_name_matches(&item.name, &name))
            })
            .map_or(u32::MAX, |item| item.location);
        Ok(self.return32(cpu, location))
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

    fn return_u64(&mut self, cpu: &mut Cpu, value: u64) {
        cpu.set_reg(1, (value >> 32) as u32);
        self.return32(cpu, value as u32);
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
                | "strstr"
                | "strspn"
                | "strcspn"
                | "strpbrk"
                | "strdup"
                | "strcasecmp"
                | "strncasecmp"
                | "strtod"
                | "strtof"
                | "atof"
                | "strtol"
                | "strtoul"
                | "strtoull"
                | "atoi"
                | "sscanf"
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
                | "__divsi3"
                | "__udivsi3"
                | "__modsi3"
                | "__umodsi3"
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
            | "_ZNK3mce12TextureGroup8isLoadedERK16ResourceLocation"
            | "_ZN13GeometryGroup11getGeometryERKSs"
            | "_ZN13GeometryGroup14tryGetGeometryERKSs"
            | "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E"
            | "_ZN9UIControl18_resolvePostCreateEv"
            | "_ZN14GamePadManager16getGamePadsInUseEv"
            | "_ZN14GamePadManager20getConnectedGamePadsEv"
            | "_ZN13GamePadMapper4tickER15InputEventQueue"
            | "_ZN13GamePadMapper8tickTurnER15InputEventQueue"
            | "_ZNK7GamePad11isConnectedEv"
            | "_ZNK7GamePad7isInUseEv"
            | "_ZN6Screen15controllerEventEv"
            | "_ZN6Screen27_processControllerDirectionEi"
            | "_ZN11MenuGamePad12getDirectionEi"
            | "_ZN11MenuGamePad4getXEi"
            | "_ZN11MenuGamePad4getYEi"
            | "_ZN11MenuGamePad9isTouchedEi"
            | "_ZN14KeyboardMapper21clearInputDeviceQueueEv"
            | "_ZN14KeyboardMapper4tickER15InputEventQueue"
            | "_ZN11MouseMapper21clearInputDeviceQueueEv"
            | "_ZN11MouseMapper4tickER15InputEventQueue"
            | "_ZN11TouchMapper21clearInputDeviceQueueEv"
            | "_ZN11TouchMapper4tickER15InputEventQueue"
            | "_ZN19TestAutoInputMapper21clearInputDeviceQueueEv"
            | "_ZN19TestAutoInputMapper4tickER15InputEventQueue"
            | "_ZN18DeviceButtonMapper4tickER15InputEventQueue"
            | "_ZN22GazeGestureVoiceMapper21clearInputDeviceQueueEv"
            | "_ZN22GazeGestureVoiceMapper4tickER15InputEventQueue"
            | "_ZN11MouseDevice12isButtonDownEi"
            | "_ZN11MouseDevice14getButtonStateEi"
            | "_ZN11MouseDevice14getEventButtonEv"
            | "_ZN11MouseDevice16wasFirstMovementEv"
            | "_ZN11MouseDevice19getEventButtonStateEv"
            | "_ZN11MouseDevice4getXEv"
            | "_ZN11MouseDevice4getYEv"
            | "_ZN11MouseDevice4nextEv"
            | "_ZN11MouseDevice5getDXEv"
            | "_ZN11MouseDevice5getDYEv"
            | "_ZN11MouseDevice5resetEv"
            | "_ZN11MouseDevice6reset2Ev"
            | "_ZN11MouseDevice6rewindEv"
            | "_ZN11MouseDevice8getEventEv"
            | "_ZN10Multitouch10isReleasedEi"
            | "_ZN10Multitouch11isEdgeTouchEi"
            | "_ZN10Multitouch13isPointerDownEi"
            | "_ZN10Multitouch15resetThisUpdateEv"
            | "_ZN10Multitouch19getActivePointerIdsEPPKi"
            | "_ZN10Multitouch19isPressedThisUpdateEi"
            | "_ZN10Multitouch20isReleasedThisUpdateEi"
            | "_ZN10Multitouch25getFirstActivePointerIdExEv"
            | "_ZN10Multitouch29getActivePointerIdsThisUpdateEPPKi"
            | "_ZN10Multitouch35getFirstActivePointerIdExThisUpdateEv"
            | "_ZN10Multitouch4nextEv"
            | "_ZN10Multitouch5resetEv"
            | "_ZN10Multitouch6commitEv"
            | "_ZN10Multitouch9isPressedEi"
            | "_ZN3mce11MathUtility21interpolateTransformsERN3glm6detail7tmat4x4IfEERKS4_S7_f"
            | "_ZN3mce16RenderContextOGL17unbindAllTexturesEv"
            | "_ZN12ProfilerLite4tickEbb"
            | "_ZN12ProfilerLite9_endScopeENS_5ScopeEdd"
            | "_ZN18MinecraftTelemetry4tickEv"
            | "_ZN18MinecraftTelemetry15forceSendEventsEv"
            | "_ZN6Social11Multiplayer18needToHandleInviteEv"
            | "_ZN6Social11Multiplayer4tickEb"
            | "_ZN6Social11Multiplayer22tickMultiplayerManagerEv"
            | "_ZN6Social11UserManager12silentSigninESt8functionIFvNS_12SignInResultEEE"
            | "_ZN6Social11UserManager21registerSignInHandlerESt8functionIFvvEE"
            | "_ZN6Social11UserManager22registerSignOutHandlerESt8functionIFvvEE"
            | "_ZN6Social11UserManager4tickEv"
            | "_ZNK6Social11UserManager10isSignedInEv"
            | "_ZN9RealmsAPI6updateEv"
    )
}

fn is_libc_symbol(name: &str) -> bool {
    matches!(
        name,
        "__assert2"
            | "__divsi3"
            | "__errno"
            | "__gnu_Unwind_Find_exidx"
            | "__google_potentially_blocking_region_begin"
            | "__google_potentially_blocking_region_end"
            | "__modsi3"
            | "__pthread_cleanup_pop"
            | "__pthread_cleanup_push"
            | "__sF"
            | "__stack_chk_fail"
            | "__stack_chk_guard"
            | "__udivsi3"
            | "__umodsi3"
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
            | "_ZSt11_Hash_bytesPKvjj"
            | "_ZSt15_Fnv_hash_bytesPKvjj"
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

fn load_c_string_bytes<M: Memory>(
    memory: &mut M,
    ptr: u32,
    max_len: u32,
) -> Result<Vec<u8>, HleError> {
    let mut bytes = Vec::new();
    for idx in 0..max_len {
        let byte = load8(memory, ptr.wrapping_add(idx))?;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    Ok(bytes)
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

fn c_string_byte_set<M: Memory>(memory: &mut M, ptr: u32) -> Result<[bool; 256], HleError> {
    let mut set = [false; 256];
    let mut off = 0u32;
    loop {
        let byte = load8(memory, ptr.wrapping_add(off))?;
        if byte == 0 {
            return Ok(set);
        }
        set[byte as usize] = true;
        off = off.wrapping_add(1);
    }
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

fn ascii_strcasecmp<M: Memory>(
    memory: &mut M,
    a: u32,
    b: u32,
    max_len: u32,
) -> Result<i32, HleError> {
    for idx in 0..max_len {
        let av = load8(memory, a.wrapping_add(idx))?;
        let bv = load8(memory, b.wrapping_add(idx))?;
        let al = av.to_ascii_lowercase();
        let bl = bv.to_ascii_lowercase();
        if al != bl || av == 0 || bv == 0 {
            return Ok(i32::from(al) - i32::from(bl));
        }
    }
    Ok(0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ParsedFloat {
    value: f64,
    consumed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedInteger {
    negative: bool,
    magnitude: u128,
    consumed: u32,
}

impl ParsedInteger {
    fn as_i32(self) -> i32 {
        if self.negative {
            let magnitude = self.magnitude.min(2_147_483_648);
            if magnitude == 2_147_483_648 {
                i32::MIN
            } else {
                -(magnitude as i32)
            }
        } else {
            self.magnitude.min(i32::MAX as u128) as i32
        }
    }

    fn as_i64(self) -> i64 {
        if self.negative {
            let magnitude = self.magnitude.min(9_223_372_036_854_775_808);
            if magnitude == 9_223_372_036_854_775_808 {
                i64::MIN
            } else {
                -(magnitude as i64)
            }
        } else {
            self.magnitude.min(i64::MAX as u128) as i64
        }
    }

    fn as_u32(self) -> u32 {
        if self.magnitude > u32::MAX as u128 {
            return u32::MAX;
        }
        let magnitude = self.magnitude as u32;
        if self.negative {
            0u32.wrapping_sub(magnitude)
        } else {
            magnitude
        }
    }

    fn as_u64(self) -> u64 {
        if self.magnitude > u64::MAX as u128 {
            return u64::MAX;
        }
        let magnitude = self.magnitude as u64;
        if self.negative {
            0u64.wrapping_sub(magnitude)
        } else {
            magnitude
        }
    }
}

fn parse_c_float<M: Memory>(memory: &mut M, ptr: u32) -> Result<ParsedFloat, HleError> {
    let bytes = load_c_string_bytes(memory, ptr, 4096)?;
    Ok(parse_c_float_bytes(&bytes))
}

fn parse_c_float_bytes(bytes: &[u8]) -> ParsedFloat {
    let mut idx = skip_ascii_space(bytes, 0);
    let start = idx;
    let mut sign = 1.0;
    if bytes
        .get(idx)
        .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        if bytes[idx] == b'-' {
            sign = -1.0;
        }
        idx += 1;
    }

    if ascii_starts_with_ignore_case(&bytes[idx..], b"infinity") {
        return ParsedFloat {
            value: sign * f64::INFINITY,
            consumed: (idx + 8) as u32,
        };
    }
    if ascii_starts_with_ignore_case(&bytes[idx..], b"inf") {
        return ParsedFloat {
            value: sign * f64::INFINITY,
            consumed: (idx + 3) as u32,
        };
    }
    if ascii_starts_with_ignore_case(&bytes[idx..], b"nan") {
        return ParsedFloat {
            value: f64::NAN,
            consumed: (idx + 3) as u32,
        };
    }

    let number_start = start;
    let mut digits = 0usize;
    while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
        idx += 1;
        digits += 1;
    }
    if bytes.get(idx) == Some(&b'.') {
        idx += 1;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return ParsedFloat {
            value: 0.0,
            consumed: 0,
        };
    }
    if bytes
        .get(idx)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        let exp_marker = idx;
        idx += 1;
        if bytes
            .get(idx)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            idx += 1;
        }
        let exp_digits_start = idx;
        while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
            idx += 1;
        }
        if idx == exp_digits_start {
            idx = exp_marker;
        }
    }

    let value = std::str::from_utf8(&bytes[number_start..idx])
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(0.0);
    ParsedFloat {
        value,
        consumed: idx as u32,
    }
}

fn parse_c_integer<M: Memory>(
    memory: &mut M,
    ptr: u32,
    base: u32,
) -> Result<ParsedInteger, HleError> {
    let bytes = load_c_string_bytes(memory, ptr, 4096)?;
    Ok(parse_c_integer_bytes(&bytes, base))
}

fn parse_c_integer_bytes(bytes: &[u8], base: u32) -> ParsedInteger {
    let mut idx = skip_ascii_space(bytes, 0);
    let mut negative = false;
    if bytes
        .get(idx)
        .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        negative = bytes[idx] == b'-';
        idx += 1;
    }

    let mut radix = base;
    if radix != 0 && !(2..=36).contains(&radix) {
        return ParsedInteger {
            negative,
            magnitude: 0,
            consumed: 0,
        };
    }

    if radix == 0 {
        if bytes.get(idx) == Some(&b'0')
            && matches!(bytes.get(idx + 1), Some(b'x' | b'X'))
            && bytes
                .get(idx + 2)
                .and_then(|byte| ascii_digit_value(*byte))
                .is_some_and(|digit| digit < 16)
        {
            radix = 16;
            idx += 2;
        } else if bytes.get(idx) == Some(&b'0') {
            radix = 8;
        } else {
            radix = 10;
        }
    } else if radix == 16
        && bytes.get(idx) == Some(&b'0')
        && matches!(bytes.get(idx + 1), Some(b'x' | b'X'))
        && bytes
            .get(idx + 2)
            .and_then(|byte| ascii_digit_value(*byte))
            .is_some_and(|digit| digit < 16)
    {
        idx += 2;
    }

    let digits_start = idx;
    let mut magnitude = 0u128;
    while let Some(digit) = bytes
        .get(idx)
        .and_then(|byte| ascii_digit_value(*byte))
        .filter(|digit| *digit < radix)
    {
        magnitude = magnitude
            .saturating_mul(radix as u128)
            .saturating_add(digit as u128);
        idx += 1;
    }
    if idx == digits_start {
        return ParsedInteger {
            negative,
            magnitude: 0,
            consumed: 0,
        };
    }
    ParsedInteger {
        negative,
        magnitude,
        consumed: idx as u32,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanfLength {
    None,
    Char,
    Short,
    Long,
    LongLong,
}

fn scan_input<M: Memory>(
    memory: &mut M,
    cpu: &Cpu,
    input: &[u8],
    format: &[u8],
) -> Result<u32, HleError> {
    let mut fmt_idx = 0usize;
    let mut input_idx = 0usize;
    let mut arg_idx = 0usize;
    let mut assigned = 0u32;

    while fmt_idx < format.len() {
        let fmt = format[fmt_idx];
        if fmt.is_ascii_whitespace() {
            fmt_idx = skip_ascii_space(format, fmt_idx);
            input_idx = skip_ascii_space(input, input_idx);
            continue;
        }
        if fmt != b'%' {
            if input.get(input_idx) != Some(&fmt) {
                break;
            }
            fmt_idx += 1;
            input_idx += 1;
            continue;
        }

        fmt_idx += 1;
        if format.get(fmt_idx) == Some(&b'%') {
            if input.get(input_idx) != Some(&b'%') {
                break;
            }
            fmt_idx += 1;
            input_idx += 1;
            continue;
        }

        let suppress = format.get(fmt_idx) == Some(&b'*');
        if suppress {
            fmt_idx += 1;
        }
        let (width, next_fmt) = scan_decimal_width(format, fmt_idx);
        fmt_idx = next_fmt;
        let (length, next_fmt) = scan_length_modifier(format, fmt_idx);
        fmt_idx = next_fmt;
        let Some(specifier) = format.get(fmt_idx).copied() else {
            break;
        };
        fmt_idx += 1;

        if !matches!(specifier, b'c' | b'[' | b'n') {
            input_idx = skip_ascii_space(input, input_idx);
        }

        match specifier {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' => {
                let base = match specifier {
                    b'i' => 0,
                    b'o' => 8,
                    b'x' | b'X' | b'p' => 16,
                    _ => 10,
                };
                let scan = limited_scan_slice(input, input_idx, width);
                let parsed = parse_c_integer_bytes(scan, base);
                if parsed.consumed == 0 {
                    break;
                }
                input_idx += parsed.consumed as usize;
                if !suppress {
                    let ptr = scan_vararg(cpu, memory, arg_idx)?;
                    store_scan_integer(memory, ptr, length, specifier, parsed)?;
                    assigned = assigned.wrapping_add(1);
                    arg_idx += 1;
                }
            }
            b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                let scan = limited_scan_slice(input, input_idx, width);
                let parsed = parse_c_float_bytes(scan);
                if parsed.consumed == 0 {
                    break;
                }
                input_idx += parsed.consumed as usize;
                if !suppress {
                    let ptr = scan_vararg(cpu, memory, arg_idx)?;
                    match length {
                        ScanfLength::Long | ScanfLength::LongLong => {
                            store_f64(memory, ptr, parsed.value)?;
                        }
                        _ => store32(memory, ptr, (parsed.value as f32).to_bits())?,
                    }
                    assigned = assigned.wrapping_add(1);
                    arg_idx += 1;
                }
            }
            b's' => {
                let start = input_idx;
                let max_len = width.unwrap_or(usize::MAX);
                while input_idx < input.len()
                    && input_idx - start < max_len
                    && !input[input_idx].is_ascii_whitespace()
                {
                    input_idx += 1;
                }
                if input_idx == start {
                    break;
                }
                if !suppress {
                    let ptr = scan_vararg(cpu, memory, arg_idx)?;
                    for (idx, byte) in input[start..input_idx].iter().copied().enumerate() {
                        store8(memory, ptr.wrapping_add(idx as u32), byte)?;
                    }
                    store8(memory, ptr.wrapping_add((input_idx - start) as u32), 0)?;
                    assigned = assigned.wrapping_add(1);
                    arg_idx += 1;
                }
            }
            b'c' => {
                let count = width.unwrap_or(1);
                if input.len().saturating_sub(input_idx) < count {
                    break;
                }
                if !suppress {
                    let ptr = scan_vararg(cpu, memory, arg_idx)?;
                    for idx in 0..count {
                        store8(memory, ptr.wrapping_add(idx as u32), input[input_idx + idx])?;
                    }
                    assigned = assigned.wrapping_add(1);
                    arg_idx += 1;
                }
                input_idx += count;
            }
            b'n' => {
                if !suppress {
                    let ptr = scan_vararg(cpu, memory, arg_idx)?;
                    store_scan_count(memory, ptr, length, input_idx as u32)?;
                    arg_idx += 1;
                }
            }
            _ => break,
        }
    }

    Ok(assigned)
}

fn scan_decimal_width(bytes: &[u8], mut idx: usize) -> (Option<usize>, usize) {
    let start = idx;
    let mut value = 0usize;
    while let Some(byte) = bytes.get(idx).filter(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(usize::from(*byte - b'0'));
        idx += 1;
    }
    if idx == start {
        (None, idx)
    } else {
        (Some(value), idx)
    }
}

fn scan_length_modifier(bytes: &[u8], idx: usize) -> (ScanfLength, usize) {
    match bytes.get(idx).copied() {
        Some(b'h') if bytes.get(idx + 1) == Some(&b'h') => (ScanfLength::Char, idx + 2),
        Some(b'h') => (ScanfLength::Short, idx + 1),
        Some(b'l') if bytes.get(idx + 1) == Some(&b'l') => (ScanfLength::LongLong, idx + 2),
        Some(b'l') | Some(b'L') => (ScanfLength::Long, idx + 1),
        _ => (ScanfLength::None, idx),
    }
}

fn limited_scan_slice(bytes: &[u8], start: usize, width: Option<usize>) -> &[u8] {
    let end = width
        .and_then(|width| start.checked_add(width))
        .map_or(bytes.len(), |end| end.min(bytes.len()));
    &bytes[start..end]
}

fn scan_vararg<M: Memory>(cpu: &Cpu, memory: &mut M, idx: usize) -> Result<u32, HleError> {
    match idx {
        0 => Ok(cpu.reg(2)),
        1 => Ok(cpu.reg(3)),
        _ => load32(memory, cpu.reg(13).wrapping_add(((idx - 2) * 4) as u32)),
    }
}

fn store_scan_integer<M: Memory>(
    memory: &mut M,
    ptr: u32,
    length: ScanfLength,
    specifier: u8,
    parsed: ParsedInteger,
) -> Result<(), HleError> {
    let unsigned = matches!(specifier, b'u' | b'o' | b'x' | b'X' | b'p');
    match length {
        ScanfLength::Char => store8(
            memory,
            ptr,
            if unsigned {
                parsed.as_u32() as u8
            } else {
                parsed.as_i32() as u8
            },
        ),
        ScanfLength::Short => store16(
            memory,
            ptr,
            if unsigned {
                parsed.as_u32() as u16
            } else {
                parsed.as_i32() as u16
            },
        ),
        ScanfLength::LongLong => {
            let value = if unsigned {
                parsed.as_u64()
            } else {
                parsed.as_i64() as u64
            };
            store64(memory, ptr, value)
        }
        ScanfLength::None | ScanfLength::Long => store32(
            memory,
            ptr,
            if unsigned {
                parsed.as_u32()
            } else {
                parsed.as_i32() as u32
            },
        ),
    }
}

fn store_scan_count<M: Memory>(
    memory: &mut M,
    ptr: u32,
    length: ScanfLength,
    count: u32,
) -> Result<(), HleError> {
    match length {
        ScanfLength::Char => store8(memory, ptr, count as u8),
        ScanfLength::Short => store16(memory, ptr, count as u16),
        ScanfLength::LongLong => store64(memory, ptr, u64::from(count)),
        ScanfLength::None | ScanfLength::Long => store32(memory, ptr, count),
    }
}

fn skip_ascii_space(bytes: &[u8], mut idx: usize) -> usize {
    while bytes.get(idx).is_some_and(u8::is_ascii_whitespace) {
        idx += 1;
    }
    idx
}

fn ascii_digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn ascii_starts_with_ignore_case(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.len() >= needle.len()
        && bytes
            .iter()
            .zip(needle.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn trace_hle_string_compare(
    name: &str,
    lhs_ptr: u32,
    rhs_ptr: u32,
    lhs: &[u8],
    rhs: &[u8],
    result: i32,
) {
    if std::env::var_os("AEMU_TRACE_HLE_STRING").is_none() {
        return;
    }
    if let Some(needle) = std::env::var("AEMU_TRACE_HLE_STRING_CONTAINS")
        .ok()
        .filter(|needle| !needle.is_empty())
    {
        let needle = needle.as_bytes();
        if !bytes_contain(lhs, needle) && !bytes_contain(rhs, needle) {
            return;
        }
    }
    let count = HLE_STRING_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
    let limit = std::env::var("AEMU_TRACE_HLE_STRING_LIMIT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok());
    if limit.is_some_and(|limit| count >= limit) {
        return;
    }
    eprintln!(
        "HLE_STRING_COMPARE name={name} lhs_ptr={lhs_ptr:#010x} rhs_ptr={rhs_ptr:#010x} lhs={} rhs={} result={result}",
        format_trace_bytes(lhs),
        format_trace_bytes(rhs),
    );
}

fn format_trace_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    out.push('"');
    for byte in bytes.iter().copied().take(96) {
        for escaped in std::ascii::escape_default(byte) {
            out.push(char::from(escaped));
        }
    }
    if bytes.len() > 96 {
        out.push_str("...");
    }
    out.push('"');
    out
}

fn bytes_contain(bytes: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    bytes.windows(needle.len()).any(|window| window == needle)
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

fn libstdcxx_hash_bytes<M: Memory>(
    memory: &mut M,
    ptr: u32,
    len: u32,
    seed: u32,
) -> Result<u32, HleError> {
    const MURMUR_M: u32 = 0x5bd1_e995;

    let mut hash = seed ^ len;
    let mut offset = 0;
    while len.wrapping_sub(offset) > 3 {
        let mut k = u32::from(load8(memory, ptr.wrapping_add(offset))?)
            | (u32::from(load8(memory, ptr.wrapping_add(offset + 1))?) << 8)
            | (u32::from(load8(memory, ptr.wrapping_add(offset + 2))?) << 16)
            | (u32::from(load8(memory, ptr.wrapping_add(offset + 3))?) << 24);
        hash = hash.wrapping_mul(MURMUR_M);
        k = k.wrapping_mul(MURMUR_M);
        k ^= k >> 24;
        k = k.wrapping_mul(MURMUR_M);
        hash ^= k;
        offset += 4;
    }

    match len & 3 {
        3 => {
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset + 2))?) << 16;
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset + 1))?) << 8;
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset))?);
            hash = hash.wrapping_mul(MURMUR_M);
        }
        2 => {
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset + 1))?) << 8;
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset))?);
            hash = hash.wrapping_mul(MURMUR_M);
        }
        1 => {
            hash ^= u32::from(load8(memory, ptr.wrapping_add(offset))?);
            hash = hash.wrapping_mul(MURMUR_M);
        }
        _ => {}
    }

    hash ^= hash >> 13;
    hash = hash.wrapping_mul(MURMUR_M);
    hash ^= hash >> 15;
    Ok(hash)
}

fn libstdcxx_fnv_hash_bytes<M: Memory>(
    memory: &mut M,
    ptr: u32,
    len: u32,
    seed: u32,
) -> Result<u32, HleError> {
    let mut hash = seed;
    for offset in 0..len {
        hash ^= u32::from(load8(memory, ptr.wrapping_add(offset))?);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    Ok(hash)
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

fn trace_hle_write<M: Memory>(name: &str, memory: &mut M, ptr: u32, len: u32) {
    if std::env::var_os("AEMU_TRACE_HLE_FILE").is_none() {
        return;
    }
    let trace_len = len.min(256);
    match load_bytes(memory, ptr, trace_len) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            eprintln!("HLE file {name} len={len} text={text:?}");
        }
        Err(err) => eprintln!("HLE file {name} len={len} text=<{err}>"),
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

fn store64<M: Memory>(memory: &mut M, addr: u32, value: u64) -> Result<(), HleError> {
    store32(memory, addr, value as u32)?;
    store32(memory, addr.wrapping_add(4), (value >> 32) as u32)
}

fn store_f64<M: Memory>(memory: &mut M, addr: u32, value: f64) -> Result<(), HleError> {
    store64(memory, addr, value.to_bits())
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

fn active_max_name_len(active: &[GlActive]) -> Option<u32> {
    active.iter().map(|item| item.name.len() as u32 + 1).max()
}

fn write_gl_name<M: Memory>(
    memory: &mut M,
    ptr: u32,
    buf_size: u32,
    name: &str,
) -> Result<u32, HleError> {
    if ptr == 0 || buf_size == 0 {
        return Ok(0);
    }
    let max_bytes = buf_size.saturating_sub(1) as usize;
    let bytes = name.as_bytes();
    let write_len = bytes.len().min(max_bytes);
    for (idx, byte) in bytes.iter().copied().take(write_len).enumerate() {
        store8(memory, ptr.wrapping_add(idx as u32), byte)?;
    }
    store8(memory, ptr.wrapping_add(write_len as u32), 0)?;
    Ok(write_len as u32)
}

fn active_name_matches(active: &str, query: &str) -> bool {
    active == query || active.strip_suffix("[0]").is_some_and(|base| base == query)
}

fn reflect_glsl_uniforms(sources: &[(u32, &str)]) -> Vec<GlActive> {
    let mut active = Vec::new();
    for (_, source) in sources {
        let source = glsl_es2_visible_source(source);
        reflect_glsl_declarations(&source, "uniform", &mut active);
    }
    active
}

fn reflect_glsl_attributes(sources: &[(u32, &str)]) -> Vec<GlActive> {
    let mut active = Vec::new();
    for (_, source) in sources {
        let source = glsl_es2_visible_source(source);
        reflect_glsl_declarations(&source, "attribute", &mut active);
    }
    active
}

fn reflect_glsl_declarations(source: &str, keyword: &str, active: &mut Vec<GlActive>) {
    let tokens = glsl_tokens(source);
    let mut idx = 0usize;
    while idx < tokens.len() {
        if tokens[idx] != keyword {
            idx += 1;
            continue;
        }
        idx += 1;
        idx = skip_glsl_qualifiers(&tokens, idx);
        let Some(ty_token) = tokens.get(idx) else {
            break;
        };
        let Some(ty) = glsl_type_to_gl(ty_token) else {
            idx = skip_glsl_declaration(&tokens, idx);
            continue;
        };
        idx += 1;
        loop {
            idx = skip_glsl_qualifiers(&tokens, idx);
            let Some(token) = tokens.get(idx) else {
                return;
            };
            if token == ";" {
                idx += 1;
                break;
            }
            if token == "," {
                idx += 1;
                continue;
            }
            if !is_glsl_identifier(token) {
                idx += 1;
                continue;
            }
            let base_name = token.clone();
            idx += 1;
            let mut size = 1u32;
            let mut name = base_name.clone();
            if tokens.get(idx).is_some_and(|token| token == "[") {
                if let Some(size_token) = tokens.get(idx + 1) {
                    size = size_token.parse::<u32>().unwrap_or(1).max(1);
                }
                name = format!("{base_name}[0]");
                while idx < tokens.len() && tokens[idx] != "]" {
                    idx += 1;
                }
                if idx < tokens.len() {
                    idx += 1;
                }
            }
            if glsl_token_occurrences(&tokens, &base_name) > 1 {
                push_gl_active(active, name, size, ty);
            }
            while idx < tokens.len() && tokens[idx] != "," && tokens[idx] != ";" {
                idx += 1;
            }
        }
    }
}

fn glsl_token_occurrences(tokens: &[String], name: &str) -> usize {
    tokens.iter().filter(|token| token.as_str() == name).count()
}

fn push_gl_active(active: &mut Vec<GlActive>, name: String, size: u32, ty: u32) {
    if active
        .iter()
        .any(|item| active_name_matches(&item.name, &name))
    {
        return;
    }
    active.push(GlActive {
        name,
        size,
        ty,
        location: active.len() as u32,
    });
}

fn skip_glsl_qualifiers(tokens: &[String], mut idx: usize) -> usize {
    loop {
        let Some(token) = tokens.get(idx) else {
            return idx;
        };
        if token == "layout" && tokens.get(idx + 1).is_some_and(|token| token == "(") {
            idx += 2;
            while idx < tokens.len() && tokens[idx] != ")" {
                idx += 1;
            }
            if idx < tokens.len() {
                idx += 1;
            }
            continue;
        }
        if !is_glsl_qualifier(token) {
            return idx;
        }
        idx += 1;
    }
}

fn skip_glsl_declaration(tokens: &[String], mut idx: usize) -> usize {
    while idx < tokens.len() && tokens[idx] != ";" {
        idx += 1;
    }
    idx.saturating_add(1)
}

fn is_glsl_qualifier(token: &str) -> bool {
    matches!(
        token,
        "const"
            | "centroid"
            | "flat"
            | "smooth"
            | "invariant"
            | "lowp"
            | "mediump"
            | "highp"
            | "readonly"
            | "writeonly"
            | "coherent"
            | "volatile"
            | "restrict"
    )
}

fn is_glsl_identifier(token: &str) -> bool {
    token
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
}

fn glsl_type_to_gl(token: &str) -> Option<u32> {
    match token {
        "float" => Some(GL_FLOAT),
        "int" => Some(GL_INT),
        "bool" => Some(GL_BOOL),
        "vec2" => Some(GL_FLOAT_VEC2),
        "vec3" | "POS3" => Some(GL_FLOAT_VEC3),
        "vec4" | "POS4" => Some(GL_FLOAT_VEC4),
        "ivec2" => Some(GL_INT_VEC2),
        "ivec3" => Some(GL_INT_VEC3),
        "ivec4" => Some(GL_INT_VEC4),
        "bvec2" => Some(GL_BOOL_VEC2),
        "bvec3" => Some(GL_BOOL_VEC3),
        "bvec4" => Some(GL_BOOL_VEC4),
        "mat2" => Some(GL_FLOAT_MAT2),
        "mat3" => Some(GL_FLOAT_MAT3),
        "mat4" | "MAT4" => Some(GL_FLOAT_MAT4),
        "sampler2D" => Some(GL_SAMPLER_2D),
        "samplerCube" => Some(GL_SAMPLER_CUBE),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct GlslPreprocFrame {
    parent_active: bool,
    condition_value: bool,
    known: bool,
    active: bool,
}

fn glsl_es2_visible_source(source: &str) -> String {
    let source = strip_glsl_comments(source);
    let mut out = String::new();
    let mut frames: Vec<GlslPreprocFrame> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(expr) = trimmed.strip_prefix("#ifdef") {
            let parent_active = frames.last().map_or(true, |frame| frame.active);
            let condition_value = glsl_unknown_preproc_condition_value(expr);
            frames.push(GlslPreprocFrame {
                parent_active,
                condition_value,
                known: false,
                active: parent_active && condition_value,
            });
            continue;
        }
        if let Some(expr) = trimmed.strip_prefix("#ifndef") {
            let parent_active = frames.last().map_or(true, |frame| frame.active);
            let condition_value = !glsl_unknown_preproc_condition_value(expr);
            frames.push(GlslPreprocFrame {
                parent_active,
                condition_value,
                known: false,
                active: parent_active && condition_value,
            });
            continue;
        }
        if let Some(expr) = trimmed.strip_prefix("#if") {
            let parent_active = frames.last().map_or(true, |frame| frame.active);
            let (known, condition_value) = glsl_es2_preproc_condition_value(expr);
            frames.push(GlslPreprocFrame {
                parent_active,
                condition_value,
                known,
                active: parent_active && condition_value,
            });
            continue;
        }
        if trimmed.starts_with("#else") {
            if let Some(frame) = frames.last_mut() {
                frame.active = if frame.known {
                    frame.parent_active && !frame.condition_value
                } else {
                    false
                };
            }
            continue;
        }
        if trimmed.starts_with("#endif") {
            frames.pop();
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if frames.last().map_or(true, |frame| frame.active) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn glsl_unknown_preproc_condition_value(_expr: &str) -> bool {
    true
}

fn glsl_es2_preproc_condition_value(expr: &str) -> (bool, bool) {
    if !expr.contains("__VERSION__") {
        return (false, true);
    }
    let compact: String = expr.chars().filter(|ch| !ch.is_whitespace()).collect();
    let value = compact.contains("__VERSION__<300")
        || compact.contains("__VERSION__<=100")
        || compact.contains("__VERSION__==100");
    (true, value)
}

fn strip_glsl_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut idx = 0usize;
    let mut in_block = false;
    while idx < bytes.len() {
        if in_block {
            if bytes[idx] == b'*' && bytes.get(idx + 1) == Some(&b'/') {
                in_block = false;
                idx += 2;
            } else {
                if bytes[idx] == b'\n' {
                    out.push('\n');
                }
                idx += 1;
            }
            continue;
        }
        if bytes[idx] == b'/' && bytes.get(idx + 1) == Some(&b'/') {
            idx += 2;
            while idx < bytes.len() && bytes[idx] != b'\n' {
                idx += 1;
            }
            if idx < bytes.len() {
                out.push('\n');
                idx += 1;
            }
            continue;
        }
        if bytes[idx] == b'/' && bytes.get(idx + 1) == Some(&b'*') {
            in_block = true;
            idx += 2;
            continue;
        }
        out.push(bytes[idx] as char);
        idx += 1;
    }
    out
}

fn glsl_tokens(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let byte = bytes[idx];
        if byte.is_ascii_whitespace() {
            idx += 1;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = idx;
            idx += 1;
            while idx < bytes.len() && (bytes[idx].is_ascii_alphanumeric() || bytes[idx] == b'_') {
                idx += 1;
            }
            tokens.push(String::from_utf8_lossy(&bytes[start..idx]).into_owned());
            continue;
        }
        if byte.is_ascii_digit() {
            let start = idx;
            idx += 1;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }
            tokens.push(String::from_utf8_lossy(&bytes[start..idx]).into_owned());
            continue;
        }
        if matches!(byte, b';' | b',' | b'[' | b']' | b'(' | b')') {
            tokens.push((byte as char).to_string());
        }
        idx += 1;
    }
    tokens
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

fn gl_shader_precision(precision_type: u32) -> (u32, u32, u32) {
    match precision_type {
        GL_LOW_FLOAT | GL_MEDIUM_FLOAT | GL_HIGH_FLOAT => (127, 127, 23),
        GL_LOW_INT | GL_MEDIUM_INT | GL_HIGH_INT => (31, 30, 0),
        _ => (0, 0, 0),
    }
}

fn gl_tex_parameter_iv(name: u32) -> u32 {
    match name {
        GL_TEXTURE_MIN_FILTER | GL_TEXTURE_MAG_FILTER => GL_LINEAR,
        GL_TEXTURE_WRAP_S | GL_TEXTURE_WRAP_T => GL_REPEAT,
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
            "_ZSt11_Hash_bytesPKvjj",
            "_ZN8WebTokenC2ERKS_",
            "_ZN3mce12TextureGroup14getTexturePairERK16ResourceLocation",
            "_ZN13GeometryGroup11getGeometryERKSs",
            "_ZN13GeometryGroup14tryGetGeometryERKSs",
            "_ZN9UIControl20_resolveControlNamesERKSt10shared_ptrIS_E",
            "_ZN9UIControl18_resolvePostCreateEv",
            "_ZN14GamePadManager16getGamePadsInUseEv",
            "_ZN14GamePadManager20getConnectedGamePadsEv",
            "_ZN13GamePadMapper4tickER15InputEventQueue",
            "_ZN13GamePadMapper8tickTurnER15InputEventQueue",
            "_ZNK7GamePad11isConnectedEv",
            "_ZNK7GamePad7isInUseEv",
            "_ZN6Screen15controllerEventEv",
            "_ZN6Screen27_processControllerDirectionEi",
            "_ZN11MenuGamePad12getDirectionEi",
            "_ZN11MenuGamePad4getXEi",
            "_ZN11MenuGamePad4getYEi",
            "_ZN11MenuGamePad9isTouchedEi",
            "_ZN14KeyboardMapper21clearInputDeviceQueueEv",
            "_ZN14KeyboardMapper4tickER15InputEventQueue",
            "_ZN11MouseMapper21clearInputDeviceQueueEv",
            "_ZN11MouseMapper4tickER15InputEventQueue",
            "_ZN11TouchMapper21clearInputDeviceQueueEv",
            "_ZN11TouchMapper4tickER15InputEventQueue",
            "_ZN19TestAutoInputMapper21clearInputDeviceQueueEv",
            "_ZN19TestAutoInputMapper4tickER15InputEventQueue",
            "_ZN18DeviceButtonMapper4tickER15InputEventQueue",
            "_ZN22GazeGestureVoiceMapper21clearInputDeviceQueueEv",
            "_ZN22GazeGestureVoiceMapper4tickER15InputEventQueue",
            "_ZN11MouseDevice12isButtonDownEi",
            "_ZN11MouseDevice14getButtonStateEi",
            "_ZN11MouseDevice14getEventButtonEv",
            "_ZN11MouseDevice16wasFirstMovementEv",
            "_ZN11MouseDevice19getEventButtonStateEv",
            "_ZN11MouseDevice4getXEv",
            "_ZN11MouseDevice4getYEv",
            "_ZN11MouseDevice4nextEv",
            "_ZN11MouseDevice5getDXEv",
            "_ZN11MouseDevice5getDYEv",
            "_ZN11MouseDevice5resetEv",
            "_ZN11MouseDevice6reset2Ev",
            "_ZN11MouseDevice6rewindEv",
            "_ZN11MouseDevice8getEventEv",
            "_ZN10Multitouch10isReleasedEi",
            "_ZN10Multitouch11isEdgeTouchEi",
            "_ZN10Multitouch13isPointerDownEi",
            "_ZN10Multitouch15resetThisUpdateEv",
            "_ZN10Multitouch19getActivePointerIdsEPPKi",
            "_ZN10Multitouch19isPressedThisUpdateEi",
            "_ZN10Multitouch20isReleasedThisUpdateEi",
            "_ZN10Multitouch25getFirstActivePointerIdExEv",
            "_ZN10Multitouch29getActivePointerIdsThisUpdateEPPKi",
            "_ZN10Multitouch35getFirstActivePointerIdExThisUpdateEv",
            "_ZN10Multitouch4nextEv",
            "_ZN10Multitouch5resetEv",
            "_ZN10Multitouch6commitEv",
            "_ZN10Multitouch9isPressedEi",
            "_ZN3mce11MathUtility21interpolateTransformsERN3glm6detail7tmat4x4IfEERKS4_S7_f",
            "_ZN3mce16RenderContextOGL17unbindAllTexturesEv",
            "_ZN12ProfilerLite4tickEbb",
            "_ZN12ProfilerLite9_endScopeENS_5ScopeEdd",
            "_ZN18MinecraftTelemetry4tickEv",
            "_ZN18MinecraftTelemetry15forceSendEventsEv",
            "_ZN6Social11Multiplayer18needToHandleInviteEv",
            "_ZN6Social11Multiplayer4tickEb",
            "_ZN6Social11Multiplayer22tickMultiplayerManagerEv",
            "_ZN6Social11UserManager12silentSigninESt8functionIFvNS_12SignInResultEEE",
            "_ZN6Social11UserManager21registerSignInHandlerESt8functionIFvvEE",
            "_ZN6Social11UserManager22registerSignOutHandlerESt8functionIFvvEE",
            "_ZN6Social11UserManager4tickEv",
            "_ZNK6Social11UserManager10isSignedInEv",
            "_ZN9RealmsAPI6updateEv",
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
    fn dispatches_gles_shader_reflection_facade_outputs() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x9000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(13, 0x1800);
        let mut hle = HleRuntime::new(0, 0x3000, 0x5000);
        const GL_FRAGMENT_SHADER: u32 = 0x8b30;
        const GL_VERTEX_SHADER: u32 = 0x8b31;

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, GL_VERTEX_SHADER);
        hle.dispatch("glCreateShader", &mut cpu, &mut memory)
            .unwrap();
        let vertex_shader = cpu.reg(0);
        let vertex_source = "\
            uniform MAT4 WORLDVIEWPROJ;\n\
            uniform vec4 LIGHTING;\n\
            attribute mediump vec4 POSITION;\n\
            attribute vec4 COLOR;\n\
            varying vec4 color;\n\
            void main() { gl_Position = WORLDVIEWPROJ * POSITION; color = COLOR; }\n";
        let vertex_source_ptr = hle.alloc_c_string(&mut memory, vertex_source).unwrap();
        memory.store32(0x1100, vertex_source_ptr).unwrap();
        memory.store32(0x1104, vertex_source.len() as u32).unwrap();
        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, vertex_shader);
        cpu.set_reg(1, 1);
        cpu.set_reg(2, 0x1100);
        cpu.set_reg(3, 0x1104);
        hle.dispatch("glShaderSource", &mut cpu, &mut memory)
            .unwrap();

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, GL_FRAGMENT_SHADER);
        hle.dispatch("glCreateShader", &mut cpu, &mut memory)
            .unwrap();
        let fragment_shader = cpu.reg(0);
        let fragment_source = "\
            #if __VERSION__ >= 300\n\
            uniform highp vec3 TEXTURE_DIMENSIONS;\n\
            #else\n\
            uniform sampler2D TEXTURE_0;\n\
            #endif\n\
            void main() { gl_FragColor = texture2D(TEXTURE_0, vec2(0.0)); }\n";
        let fragment_source_ptr = hle.alloc_c_string(&mut memory, fragment_source).unwrap();
        memory.store32(0x1110, fragment_source_ptr).unwrap();
        memory
            .store32(0x1114, fragment_source.len() as u32)
            .unwrap();
        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, fragment_shader);
        cpu.set_reg(1, 1);
        cpu.set_reg(2, 0x1110);
        cpu.set_reg(3, 0x1114);
        hle.dispatch("glShaderSource", &mut cpu, &mut memory)
            .unwrap();

        cpu.set_reg(14, 0x2010);
        hle.dispatch("glCreateProgram", &mut cpu, &mut memory)
            .unwrap();
        let program = cpu.reg(0);
        cpu.set_reg(14, 0x2014);
        cpu.set_reg(0, program);
        cpu.set_reg(1, vertex_shader);
        hle.dispatch("glAttachShader", &mut cpu, &mut memory)
            .unwrap();
        cpu.set_reg(14, 0x2018);
        cpu.set_reg(0, program);
        cpu.set_reg(1, fragment_shader);
        hle.dispatch("glAttachShader", &mut cpu, &mut memory)
            .unwrap();
        cpu.set_reg(14, 0x201c);
        cpu.set_reg(0, program);
        hle.dispatch("glLinkProgram", &mut cpu, &mut memory)
            .unwrap();

        cpu.set_reg(14, 0x2020);
        cpu.set_reg(0, program);
        cpu.set_reg(1, GL_ACTIVE_UNIFORMS);
        cpu.set_reg(2, 0x1120);
        hle.dispatch("glGetProgramiv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1120).unwrap(), 2);
        cpu.set_reg(14, 0x2024);
        cpu.set_reg(0, program);
        cpu.set_reg(1, GL_ACTIVE_ATTRIBUTES);
        cpu.set_reg(2, 0x1124);
        hle.dispatch("glGetProgramiv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1124).unwrap(), 2);

        memory.store32(0x1800, 0x1134).unwrap();
        memory.store32(0x1804, 0x1138).unwrap();
        memory.store32(0x1808, 0x1140).unwrap();
        cpu.set_reg(14, 0x2028);
        cpu.set_reg(0, program);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 64);
        cpu.set_reg(3, 0x1130);
        hle.dispatch("glGetActiveUniform", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1130).unwrap(), 13);
        assert_eq!(memory.load32(0x1134).unwrap(), 1);
        assert_eq!(memory.load32(0x1138).unwrap(), GL_FLOAT_MAT4);
        assert_eq!(
            load_c_string(&mut memory, 0x1140, 64).unwrap(),
            "WORLDVIEWPROJ"
        );

        cpu.set_reg(14, 0x202c);
        cpu.set_reg(0, program);
        cpu.set_reg(1, 1);
        cpu.set_reg(2, 64);
        cpu.set_reg(3, 0x1130);
        hle.dispatch("glGetActiveUniform", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1138).unwrap(), GL_SAMPLER_2D);
        assert_eq!(load_c_string(&mut memory, 0x1140, 64).unwrap(), "TEXTURE_0");

        cpu.set_reg(14, 0x2030);
        cpu.set_reg(0, program);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 64);
        cpu.set_reg(3, 0x1130);
        hle.dispatch("glGetActiveAttrib", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1138).unwrap(), GL_FLOAT_VEC4);
        assert_eq!(load_c_string(&mut memory, 0x1140, 64).unwrap(), "POSITION");

        let texture_name = hle.alloc_c_string(&mut memory, "TEXTURE_0").unwrap();
        cpu.set_reg(14, 0x2034);
        cpu.set_reg(0, program);
        cpu.set_reg(1, texture_name);
        hle.dispatch("glGetUniformLocation", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);

        let color_name = hle.alloc_c_string(&mut memory, "COLOR").unwrap();
        cpu.set_reg(14, 0x2038);
        cpu.set_reg(0, program);
        cpu.set_reg(1, color_name);
        hle.dispatch("glGetAttribLocation", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 1);
    }

    #[test]
    fn dispatches_gles_precision_and_texture_parameter_queries() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x3000, 0x1000);

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(1, GL_HIGH_FLOAT);
        cpu.set_reg(2, 0x1100);
        cpu.set_reg(3, 0x1108);
        hle.dispatch("glGetShaderPrecisionFormat", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1100).unwrap(), 127);
        assert_eq!(memory.load32(0x1104).unwrap(), 127);
        assert_eq!(memory.load32(0x1108).unwrap(), 23);
        assert_eq!(cpu.pc(), 0x2000);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(1, GL_TEXTURE_MIN_FILTER);
        cpu.set_reg(2, 0x1110);
        hle.dispatch("glGetTexParameteriv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1110).unwrap(), GL_LINEAR);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(1, GL_TEXTURE_WRAP_S);
        cpu.set_reg(2, 0x1114);
        hle.dispatch("glGetTexParameteriv", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(memory.load32(0x1114).unwrap(), GL_REPEAT);
    }

    #[test]
    fn dispatches_gles_object_name_facade_outputs() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x2000, 0x1000);

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, 3);
        cpu.set_reg(1, 0x1100);
        hle.dispatch("glGenTextures", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(memory.load32(0x1100).unwrap(), 1);
        assert_eq!(memory.load32(0x1104).unwrap(), 2);
        assert_eq!(memory.load32(0x1108).unwrap(), 3);

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, 1);
        cpu.set_reg(1, 0x1110);
        hle.dispatch("glGenBuffers", &mut cpu, &mut memory).unwrap();
        assert_eq!(memory.load32(0x1110).unwrap(), 4);

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, memory.load32(0x1100).unwrap());
        hle.dispatch("glIsTexture", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);

        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, 0);
        hle.dispatch("glIsTexture", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(14, 0x2010);
        hle.dispatch("glCheckFramebufferStatus", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), GL_FRAMEBUFFER_COMPLETE);
    }

    #[test]
    fn records_gles_frame_events_for_host_replay() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x2000, 0x1000);

        cpu.set_reg(14, 0x2000);
        cpu.set_reg(0, 0.25f32.to_bits());
        cpu.set_reg(1, 0.5f32.to_bits());
        cpu.set_reg(2, 0.75f32.to_bits());
        cpu.set_reg(3, 1.0f32.to_bits());
        hle.dispatch("glClearColor", &mut cpu, &mut memory).unwrap();

        cpu.set_reg(14, 0x2004);
        cpu.set_reg(0, 0x0000_4100);
        hle.dispatch("glClear", &mut cpu, &mut memory).unwrap();

        cpu.set_reg(14, 0x2008);
        cpu.set_reg(0, 0);
        cpu.set_reg(1, 0);
        cpu.set_reg(2, 854);
        cpu.set_reg(3, 480);
        hle.dispatch("glViewport", &mut cpu, &mut memory).unwrap();

        cpu.set_reg(14, 0x200c);
        cpu.set_reg(0, 4);
        cpu.set_reg(1, 6);
        cpu.set_reg(2, 0x1403);
        cpu.set_reg(3, 0x6000_1000);
        hle.dispatch("glDrawElements", &mut cpu, &mut memory)
            .unwrap();

        cpu.set_reg(14, 0x2010);
        cpu.set_reg(0, EGL_DISPLAY_HANDLE);
        cpu.set_reg(1, EGL_SURFACE_HANDLE);
        hle.dispatch("eglSwapBuffers", &mut cpu, &mut memory)
            .unwrap();

        assert_eq!(
            hle.take_gles_events(),
            vec![
                GlesEvent::ClearColor {
                    red: 0.25f32.to_bits(),
                    green: 0.5f32.to_bits(),
                    blue: 0.75f32.to_bits(),
                    alpha: 1.0f32.to_bits(),
                },
                GlesEvent::Clear { mask: 0x0000_4100 },
                GlesEvent::Viewport {
                    x: 0,
                    y: 0,
                    width: 854,
                    height: 480,
                },
                GlesEvent::DrawElements {
                    mode: 4,
                    count: 6,
                    ty: 0x1403,
                    indices: 0x6000_1000,
                },
                GlesEvent::SwapBuffers {
                    display: EGL_DISPLAY_HANDLE,
                    surface: EGL_SURFACE_HANDLE,
                },
            ]
        );
        assert!(hle.take_gles_events().is_empty());
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

        hle.dispatch("pthread_self", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);
        hle.set_current_pthread(7);
        hle.dispatch("pthread_self", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 7);

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

        hle.set_current_pthread(1);
        cpu.set_reg(0, key);
        hle.dispatch("pthread_getspecific", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        hle.set_current_pthread(7);

        cpu.set_reg(0, key);
        hle.dispatch("pthread_key_delete", &mut cpu, &mut memory)
            .unwrap();
        cpu.set_reg(0, key);
        hle.dispatch("pthread_getspecific", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
    }

    #[test]
    fn pthread_create_marks_only_native_app_thread_arg_running() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);
        hle.set_native_activity(0x1100);

        let app = 0x1200;
        memory.store32(app + 0x0c, 0x1100).unwrap();
        cpu.set_reg(0, 0x1300);
        cpu.set_reg(2, 0x2201);
        cpu.set_reg(3, app);
        hle.dispatch("pthread_create", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_ne!(memory.load32(0x1300).unwrap(), 0);
        assert_eq!(memory.load32(app + 0x6c).unwrap(), 1);
        assert!(hle.take_created_pthreads().is_empty());

        let worker_arg = 0x1400;
        memory.store32(worker_arg + 0x0c, 0xfeed_beef).unwrap();
        memory.store32(worker_arg + 0x6c, 0x55aa_aa55).unwrap();
        cpu.set_reg(0, 0x1310);
        cpu.set_reg(2, 0x3301);
        cpu.set_reg(3, worker_arg);
        hle.dispatch("pthread_create", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        let worker_id = memory.load32(0x1310).unwrap();
        assert_ne!(worker_id, 0);
        assert_eq!(memory.load32(worker_arg + 0x6c).unwrap(), 0x55aa_aa55);
        assert_eq!(
            hle.take_created_pthreads(),
            vec![CreatedPthread {
                id: worker_id,
                start: 0x3301,
                arg: worker_arg,
            }]
        );
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
    fn dispatches_numeric_and_extended_string_hle_calls() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        memory.load_bytes(0x1100, b"58 ], trailing\0").unwrap();
        memory.load_bytes(0x1120, b"  -0.25e2,\0").unwrap();
        memory.load_bytes(0x1140, b"x\0").unwrap();
        memory.load_bytes(0x1160, b"-14px\0").unwrap();
        memory.load_bytes(0x1180, b"0xff]\0").unwrap();
        memory
            .load_bytes(0x11a0, b"18446744073709551615!\0")
            .unwrap();
        memory.load_bytes(0x11c0, b"AlphaBeta\0").unwrap();
        memory.load_bytes(0x11e0, b"alphabet\0").unwrap();
        memory.load_bytes(0x1200, b"0123456789\0").unwrap();
        memory.load_bytes(0x1220, b"pB\0").unwrap();
        memory.load_bytes(0x1240, b"%lf\0").unwrap();
        memory.load_bytes(0x1260, b"%d %x\0").unwrap();
        memory.load_bytes(0x1280, b"-14 0xff\0").unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x2800, 0x600);

        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 0x1300);
        hle.dispatch("strtod", &mut cpu, &mut memory).unwrap();
        let double_bits = (u64::from(cpu.reg(1)) << 32) | u64::from(cpu.reg(0));
        assert_eq!(f64::from_bits(double_bits), 58.0);
        assert_eq!(memory.load32(0x1300).unwrap(), 0x1102);

        cpu.set_reg(0, 0x1120);
        cpu.set_reg(1, 0x1304);
        hle.dispatch("strtof", &mut cpu, &mut memory).unwrap();
        assert_eq!(f32::from_bits(cpu.reg(0)), -25.0);
        assert_eq!(memory.load32(0x1304).unwrap(), 0x1129);

        cpu.set_reg(0, 0x1140);
        cpu.set_reg(1, 0x1308);
        hle.dispatch("strtod", &mut cpu, &mut memory).unwrap();
        assert_eq!(memory.load32(0x1308).unwrap(), 0x1140);

        cpu.set_reg(0, 0x1160);
        cpu.set_reg(1, 0x130c);
        cpu.set_reg(2, 10);
        hle.dispatch("strtol", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0) as i32, -14);
        assert_eq!(memory.load32(0x130c).unwrap(), 0x1163);

        cpu.set_reg(0, 0x1180);
        cpu.set_reg(1, 0x1310);
        cpu.set_reg(2, 0);
        hle.dispatch("strtoul", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 255);
        assert_eq!(memory.load32(0x1310).unwrap(), 0x1184);

        cpu.set_reg(0, 0x11a0);
        cpu.set_reg(1, 0x1314);
        cpu.set_reg(2, 10);
        hle.dispatch("strtoull", &mut cpu, &mut memory).unwrap();
        assert_eq!(
            (u64::from(cpu.reg(1)) << 32) | u64::from(cpu.reg(0)),
            u64::MAX
        );
        assert_eq!(memory.load32(0x1314).unwrap(), 0x11b4);

        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 0x1240);
        cpu.set_reg(2, 0x1320);
        hle.dispatch("sscanf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(
            f64::from_bits(
                u64::from(memory.load32(0x1320).unwrap())
                    | (u64::from(memory.load32(0x1324).unwrap()) << 32)
            ),
            58.0
        );

        cpu.set_reg(0, 0x1280);
        cpu.set_reg(1, 0x1260);
        cpu.set_reg(2, 0x1328);
        cpu.set_reg(3, 0x132c);
        hle.dispatch("sscanf", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 2);
        assert_eq!(memory.load32(0x1328).unwrap() as i32, -14);
        assert_eq!(memory.load32(0x132c).unwrap(), 255);

        cpu.set_reg(0, 0x11c0);
        cpu.set_reg(1, 0x11e0);
        hle.dispatch("strcasecmp", &mut cpu, &mut memory).unwrap();
        assert!(cpu.reg(0) as i32 > 0);

        cpu.set_reg(0, 0x1200);
        cpu.set_reg(1, 0x1160);
        hle.dispatch("strspn", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0);

        cpu.set_reg(0, 0x11c0);
        cpu.set_reg(1, 0x1220);
        hle.dispatch("strpbrk", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0x11c2);

        cpu.set_reg(0, 0x11c0);
        cpu.set_reg(1, 0x11c5);
        hle.dispatch("strstr", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 0x11c5);

        cpu.set_reg(0, 0x11c0);
        hle.dispatch("strdup", &mut cpu, &mut memory).unwrap();
        let dup = cpu.reg(0);
        assert_ne!(dup, 0);
        assert_eq!(load_c_string(&mut memory, dup, 16).unwrap(), "AlphaBeta");
    }

    #[test]
    fn dispatches_compiler_integer_runtime_helpers() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2000);
        let mut hle = HleRuntime::new(0x1000, 0x1800, 0x400);

        cpu.set_reg(0, 43);
        cpu.set_reg(1, 11);
        hle.dispatch("__umodsi3", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 10);

        cpu.set_reg(0, 43);
        cpu.set_reg(1, 11);
        hle.dispatch("__udivsi3", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0), 3);

        cpu.set_reg(0, (-43i32) as u32);
        cpu.set_reg(1, 11);
        hle.dispatch("__modsi3", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0) as i32, -10);

        cpu.set_reg(0, (-43i32) as u32);
        cpu.set_reg(1, 11);
        hle.dispatch("__divsi3", &mut cpu, &mut memory).unwrap();
        assert_eq!(cpu.reg(0) as i32, -3);
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
    fn dispatches_libstdcxx_hash_bytes() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();
        memory.load_bytes(0x1100, b"MinecraftPE").unwrap();
        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Thumb);
        let mut hle = HleRuntime::new(0, 0x2000, 0x1000);

        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 11);
        cpu.set_reg(2, 0x1234_5678);
        hle.dispatch("_ZSt11_Hash_bytesPKvjj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0xf711_c23b);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);

        cpu.set_reg(14, 0x2005);
        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 11);
        cpu.set_reg(2, 0x1234_5678);
        hle.dispatch("_ZSt15_Fnv_hash_bytesPKvjj", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0xee96_2070);
        assert_eq!(cpu.pc(), 0x2004);
        assert_eq!(cpu.isa(), Isa::Thumb);
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
    fn dispatches_minecraft_texture_group_is_loaded_facade() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, 0x1100);
        cpu.set_reg(1, 0x1200);
        let mut hle = HleRuntime::new(0, 0x1800, 0x800);

        let descriptor =
            describe_hle_import("_ZNK3mce12TextureGroup8isLoadedERK16ResourceLocation").unwrap();
        assert_eq!(descriptor.kind, HleSymbolKind::Target);
        assert_eq!(descriptor.behavior, HleCallBehavior::Implemented);

        hle.dispatch(
            "_ZNK3mce12TextureGroup8isLoadedERK16ResourceLocation",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(cpu.reg(0), 1);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);
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
    fn dispatches_profiler_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        let mut hle = HleRuntime::new(0, 0x1800, 0x800);

        hle.dispatch(
            "_ZN12ProfilerLite9_endScopeENS_5ScopeEdd",
            &mut cpu,
            &mut memory,
        )
        .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);

        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2101);
        cpu.set_reg(0, 1);
        cpu.set_reg(1, 1);
        hle.dispatch("_ZN12ProfilerLite4tickEbb", &mut cpu, &mut memory)
            .unwrap();
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.pc(), 0x2100);
        assert_eq!(cpu.isa(), Isa::Thumb);
    }

    #[test]
    fn keeps_worker_pool_coroutines_native() {
        assert!(describe_hle_import("_ZN10WorkerPool17processCoroutinesEd").is_none());
    }

    #[test]
    fn dispatches_no_input_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x2800, 0x800);

        for (idx, name) in [
            "_ZN14KeyboardMapper21clearInputDeviceQueueEv",
            "_ZN14KeyboardMapper4tickER15InputEventQueue",
            "_ZN11MouseMapper21clearInputDeviceQueueEv",
            "_ZN11MouseMapper4tickER15InputEventQueue",
            "_ZN11TouchMapper21clearInputDeviceQueueEv",
            "_ZN11TouchMapper4tickER15InputEventQueue",
            "_ZN19TestAutoInputMapper21clearInputDeviceQueueEv",
            "_ZN19TestAutoInputMapper4tickER15InputEventQueue",
            "_ZN18DeviceButtonMapper4tickER15InputEventQueue",
            "_ZN22GazeGestureVoiceMapper21clearInputDeviceQueueEv",
            "_ZN22GazeGestureVoiceMapper4tickER15InputEventQueue",
            "_ZN11MouseDevice12isButtonDownEi",
            "_ZN11MouseDevice14getButtonStateEi",
            "_ZN11MouseDevice14getEventButtonEv",
            "_ZN11MouseDevice16wasFirstMovementEv",
            "_ZN11MouseDevice19getEventButtonStateEv",
            "_ZN11MouseDevice4getXEv",
            "_ZN11MouseDevice4getYEv",
            "_ZN11MouseDevice4nextEv",
            "_ZN11MouseDevice5getDXEv",
            "_ZN11MouseDevice5getDYEv",
            "_ZN11MouseDevice5resetEv",
            "_ZN11MouseDevice6reset2Ev",
            "_ZN11MouseDevice6rewindEv",
            "_ZN11MouseDevice8getEventEv",
            "_ZN10Multitouch10isReleasedEi",
            "_ZN10Multitouch11isEdgeTouchEi",
            "_ZN10Multitouch13isPointerDownEi",
            "_ZN10Multitouch15resetThisUpdateEv",
            "_ZN10Multitouch19isPressedThisUpdateEi",
            "_ZN10Multitouch20isReleasedThisUpdateEi",
            "_ZN10Multitouch4nextEv",
            "_ZN10Multitouch5resetEv",
            "_ZN10Multitouch6commitEv",
            "_ZN10Multitouch9isPressedEi",
        ]
        .into_iter()
        .enumerate()
        {
            cpu.set_isa(Isa::Arm);
            cpu.set_reg(14, 0x2201 + (idx as u32) * 4);
            cpu.set_reg(0, u32::MAX);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), 0);
            assert_eq!(cpu.pc(), 0x2200 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }

        for (idx, name) in [
            "_ZN10Multitouch19getActivePointerIdsEPPKi",
            "_ZN10Multitouch29getActivePointerIdsThisUpdateEPPKi",
        ]
        .into_iter()
        .enumerate()
        {
            let out = 0x1100 + (idx as u32) * 0x10;
            memory.store32(out, 0xaaaa_aaaa).unwrap();
            cpu.set_isa(Isa::Thumb);
            cpu.set_reg(14, 0x2601 + (idx as u32) * 4);
            cpu.set_reg(0, out);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), 0);
            assert_eq!(memory.load32(out).unwrap(), 0);
            assert_eq!(cpu.pc(), 0x2600 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }

        for (idx, name) in [
            "_ZN10Multitouch25getFirstActivePointerIdExEv",
            "_ZN10Multitouch35getFirstActivePointerIdExThisUpdateEv",
        ]
        .into_iter()
        .enumerate()
        {
            cpu.set_isa(Isa::Arm);
            cpu.set_reg(14, 0x2701 + (idx as u32) * 4);
            cpu.set_reg(0, 0);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), u32::MAX);
            assert_eq!(cpu.pc(), 0x2700 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }
    }

    #[test]
    fn dispatches_minecraft_transform_interpolation() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x4000).unwrap();

        for idx in 0..16 {
            let offset = (idx as u32) * 4;
            memory
                .store32(0x1100 + offset, (idx as f32).to_bits())
                .unwrap();
            memory
                .store32(0x1200 + offset, (100.0 + idx as f32).to_bits())
                .unwrap();
        }

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, 0x1300);
        cpu.set_reg(1, 0x1100);
        cpu.set_reg(2, 0x1200);
        cpu.set_reg(3, 0.25f32.to_bits());
        let mut hle = HleRuntime::new(0, 0x3000, 0x800);

        hle.dispatch(
            "_ZN3mce11MathUtility21interpolateTransformsERN3glm6detail7tmat4x4IfEERKS4_S7_f",
            &mut cpu,
            &mut memory,
        )
        .unwrap();

        assert_eq!(cpu.reg(0), 0x1300);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);
        for idx in 0..16 {
            let offset = (idx as u32) * 4;
            let value = f32::from_bits(memory.load32(0x1300 + offset).unwrap());
            assert_eq!(value, 25.0 + idx as f32);
        }
    }

    #[test]
    fn dispatches_minecraft_ogl_unbind_all_textures() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();

        for slot in 0..8 {
            memory
                .store32(0x1100 + 0x7c + slot * 4, 0xffff_ffff)
                .unwrap();
        }
        memory.store32(0x1100 + 0x100, 0x84c0).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        cpu.set_reg(14, 0x2001);
        cpu.set_reg(0, 0x1100);
        let mut hle = HleRuntime::new(0, 0x1800, 0x800);

        hle.dispatch(
            "_ZN3mce16RenderContextOGL17unbindAllTexturesEv",
            &mut cpu,
            &mut memory,
        )
        .unwrap();

        assert_eq!(cpu.reg(0), 0x1100);
        assert_eq!(cpu.pc(), 0x2000);
        assert_eq!(cpu.isa(), Isa::Thumb);
        for slot in 0..8 {
            assert_eq!(memory.load32(0x1100 + 0x7c + slot * 4).unwrap(), 0);
        }
        assert_eq!(memory.load32(0x1100 + 0x100).unwrap(), 0x84c7);
    }

    #[test]
    fn dispatches_no_gamepad_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x2000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Thumb);
        let mut hle = HleRuntime::new(0, 0x2800, 0x800);

        for (idx, name) in [
            "_ZN14GamePadManager16getGamePadsInUseEv",
            "_ZN14GamePadManager20getConnectedGamePadsEv",
        ]
        .into_iter()
        .enumerate()
        {
            let out = 0x1100 + (idx as u32) * 0x20;
            memory.store32(out, 0xaaaa_aaaa).unwrap();
            memory.store32(out + 4, 0xbbbb_bbbb).unwrap();
            memory.store32(out + 8, 0xcccc_cccc).unwrap();
            cpu.set_isa(Isa::Thumb);
            cpu.set_reg(14, 0x2401 + (idx as u32) * 4);
            cpu.set_reg(0, out);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), out);
            assert_eq!(memory.load32(out).unwrap(), 0);
            assert_eq!(memory.load32(out + 4).unwrap(), 0);
            assert_eq!(memory.load32(out + 8).unwrap(), 0);
            assert_eq!(cpu.pc(), 0x2400 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }

        for (idx, name) in [
            "_ZN13GamePadMapper4tickER15InputEventQueue",
            "_ZN13GamePadMapper8tickTurnER15InputEventQueue",
            "_ZNK7GamePad11isConnectedEv",
            "_ZNK7GamePad7isInUseEv",
            "_ZN6Screen15controllerEventEv",
            "_ZN6Screen27_processControllerDirectionEi",
            "_ZN11MenuGamePad12getDirectionEi",
            "_ZN11MenuGamePad4getXEi",
            "_ZN11MenuGamePad4getYEi",
            "_ZN11MenuGamePad9isTouchedEi",
        ]
        .into_iter()
        .enumerate()
        {
            cpu.set_isa(Isa::Arm);
            cpu.set_reg(14, 0x2501 + (idx as u32) * 4);
            cpu.set_reg(0, u32::MAX);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), 0);
            assert_eq!(cpu.pc(), 0x2500 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }
    }

    #[test]
    fn dispatches_no_network_social_tick_facades() {
        let mut memory = MappedMemory::new();
        memory.map_zeroed(0x1000, 0x1000).unwrap();

        let mut cpu = Cpu::new();
        cpu.set_isa(Isa::Arm);
        let mut hle = HleRuntime::new(0, 0x1800, 0x800);

        for (idx, name) in [
            "_ZN18MinecraftTelemetry4tickEv",
            "_ZN18MinecraftTelemetry15forceSendEventsEv",
            "_ZN6Social11Multiplayer18needToHandleInviteEv",
            "_ZN6Social11Multiplayer4tickEb",
            "_ZN6Social11Multiplayer22tickMultiplayerManagerEv",
            "_ZN6Social11UserManager12silentSigninESt8functionIFvNS_12SignInResultEEE",
            "_ZN6Social11UserManager21registerSignInHandlerESt8functionIFvvEE",
            "_ZN6Social11UserManager22registerSignOutHandlerESt8functionIFvvEE",
            "_ZN6Social11UserManager4tickEv",
            "_ZNK6Social11UserManager10isSignedInEv",
            "_ZN9RealmsAPI6updateEv",
        ]
        .into_iter()
        .enumerate()
        {
            cpu.set_isa(Isa::Arm);
            cpu.set_reg(14, 0x2201 + (idx as u32) * 4);
            cpu.set_reg(0, u32::MAX);
            hle.dispatch(name, &mut cpu, &mut memory).unwrap();
            assert_eq!(cpu.reg(0), 0);
            assert_eq!(cpu.pc(), 0x2200 + (idx as u32) * 4);
            assert_eq!(cpu.isa(), Isa::Thumb);
        }
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
