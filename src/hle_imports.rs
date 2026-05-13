use std::fmt;

use crate::armv6::{Cpu, Memory};

pub const HLE_TRAP_ARM_INSTR: u32 = 0xe7f0_00f0;

const FAKE_FILE_SIZE: u32 = 0x40;
const FAKE_FILE_FD_OFFSET: u32 = 0x0e;
const FIRST_FAKE_FD: u32 = 3;

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
    next_gl_name: u32,
    next_fd: u32,
    random_state: u32,
}

#[derive(Debug, Clone, Copy)]
struct HleAllocation {
    ptr: u32,
    size: u32,
}

impl HleRuntime {
    pub fn new(errno_addr: u32, heap_base: u32, heap_size: u32) -> Self {
        Self {
            errno_addr,
            heap_next: align_up(heap_base, 8).unwrap_or(heap_base),
            heap_end: heap_base.saturating_add(heap_size),
            allocations: Vec::new(),
            next_gl_name: 1,
            next_fd: FIRST_FAKE_FD,
            random_state: 0x1234_5678,
        }
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
            "memset" | "__aeabi_memset" => self.memset(cpu, memory),
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
            "malloc" => self.malloc_call(cpu, memory),
            "calloc" => self.calloc(cpu, memory),
            "realloc" => self.realloc(cpu, memory),
            "free" => Ok(self.return32(cpu, 0)),
            "__errno" => Ok(self.return32(cpu, self.errno_addr)),
            "__aeabi_idiv" => self.aeabi_idiv(cpu),
            "__aeabi_uidiv" => self.aeabi_uidiv(cpu),
            "__aeabi_idivmod" => self.aeabi_idivmod(cpu),
            "__aeabi_uidivmod" => self.aeabi_uidivmod(cpu),
            name if descriptor.kind == HleSymbolKind::Libm => self.libm(name, cpu),
            "gettimeofday" => self.gettimeofday(cpu, memory),
            "clock_gettime" => self.clock_gettime(cpu, memory),
            "time" => self.time(cpu, memory),
            "pthread_self" => Ok(self.return32(cpu, 1)),
            "fopen" => self.fopen_call(cpu, memory),
            "fdopen" => self.fdopen_call(cpu, memory),
            "fclose" | "close" => Ok(self.return32(cpu, 0)),
            "open" => self.open_call(cpu, memory),
            "read" => self.read_call(cpu, memory),
            "fread" => self.fread_call(cpu, memory),
            "write" => Ok(self.return32(cpu, cpu.reg(2))),
            "fwrite" => Ok(self.return32(cpu, cpu.reg(2))),
            "__cxa_guard_acquire" => self.cxa_guard_acquire(cpu, memory),
            "__cxa_guard_release" => self.cxa_guard_release(cpu, memory),
            "__cxa_guard_abort" => Ok(self.return32(cpu, 0)),
            "_ZNSs14_M_replace_auxEjjjc" => self.libstdcxx_string_replace_aux(cpu, memory),
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
        let ptr = self.alloc(cpu.reg(0), 8)?;
        Ok(self.return32(cpu, ptr))
    }

    fn calloc<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let count = cpu.reg(0);
        let size = cpu.reg(1);
        let Some(total) = count.checked_mul(size) else {
            return Ok(self.return32(cpu, 0));
        };
        let ptr = self.alloc(total, 8)?;
        for idx in 0..total {
            store8(memory, ptr.wrapping_add(idx), 0)?;
        }
        Ok(self.return32(cpu, ptr))
    }

    fn realloc<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let size = cpu.reg(1);
        if ptr == 0 {
            let new_ptr = self.alloc(size, 8)?;
            Ok(self.return32(cpu, new_ptr))
        } else if size == 0 {
            Ok(self.return32(cpu, 0))
        } else {
            let new_ptr = self.alloc(size, 8)?;
            let old_size = self
                .allocations
                .iter()
                .rev()
                .find(|allocation| allocation.ptr == ptr)
                .map(|allocation| allocation.size)
                .unwrap_or(0);
            for idx in 0..old_size.min(size) {
                let byte = load8(memory, ptr.wrapping_add(idx))?;
                store8(memory, new_ptr.wrapping_add(idx), byte)?;
            }
            Ok(self.return32(cpu, new_ptr))
        }
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
        if tv != 0 {
            store32(memory, tv, 0)?;
            store32(memory, tv.wrapping_add(4), 0)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn clock_gettime<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ts = cpu.reg(1);
        if ts != 0 {
            store32(memory, ts, 0)?;
            store32(memory, ts.wrapping_add(4), 0)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn time<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let out = cpu.reg(0);
        if out != 0 {
            store32(memory, out, 0)?;
        }
        Ok(self.return32(cpu, 0))
    }

    fn fopen_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let path = load_c_string(memory, cpu.reg(0), 256)?;
        if is_random_device_path(&path) {
            let fd = self.alloc_fd();
            let ptr = self.alloc_fake_file(memory, fd)?;
            return Ok(self.return32(cpu, ptr));
        }
        self.set_errno(memory, 2)?;
        Ok(self.return32(cpu, 0))
    }

    fn fdopen_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fd = cpu.reg(0);
        if fd == u32::MAX {
            self.set_errno(memory, 9)?;
            return Ok(self.return32(cpu, 0));
        }
        let ptr = self.alloc_fake_file(memory, fd)?;
        Ok(self.return32(cpu, ptr))
    }

    fn open_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let path = load_c_string(memory, cpu.reg(0), 256)?;
        if is_random_device_path(&path) {
            let fd = self.alloc_fd();
            return Ok(self.return32(cpu, fd));
        }
        self.set_errno(memory, 2)?;
        Ok(self.return32(cpu, u32::MAX))
    }

    fn read_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let fd = cpu.reg(0);
        let buf = cpu.reg(1);
        let count = cpu.reg(2);
        if fd < FIRST_FAKE_FD || buf == 0 {
            self.set_errno(memory, 9)?;
            return Ok(self.return32(cpu, u32::MAX));
        }
        self.fill_random(memory, buf, count)?;
        Ok(self.return32(cpu, count))
    }

    fn fread_call<M: Memory>(&mut self, cpu: &mut Cpu, memory: &mut M) -> Result<(), HleError> {
        let ptr = cpu.reg(0);
        let size = cpu.reg(1);
        let count = cpu.reg(2);
        let stream = cpu.reg(3);
        if ptr == 0 || stream == 0 || self.fake_file_fd(memory, stream).is_err() {
            return Ok(self.return32(cpu, 0));
        }
        let Some(total) = size.checked_mul(count) else {
            return Ok(self.return32(cpu, 0));
        };
        self.fill_random(memory, ptr, total)?;
        Ok(self.return32(cpu, count))
    }

    fn alloc_fd(&mut self) -> u32 {
        let fd = self.next_fd;
        self.next_fd = self.next_fd.wrapping_add(1).max(FIRST_FAKE_FD);
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

    fn egl<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "eglGetDisplay" | "eglCreateContext" | "eglCreateWindowSurface" => {
                Ok(self.return32(cpu, 1))
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
                let num_config_ptr = load32(memory, cpu.reg(13)).unwrap_or(0);
                if num_config_ptr != 0 {
                    store32(memory, num_config_ptr, 1)?;
                }
                Ok(self.return32(cpu, 1))
            }
            "eglGetError" => Ok(self.return32(cpu, 0x3000)),
            "eglGetProcAddress" => Ok(self.return32(cpu, 0)),
            _ => Ok(self.return32(cpu, 1)),
        }
    }

    fn gles<M: Memory>(
        &mut self,
        name: &str,
        cpu: &mut Cpu,
        _memory: &mut M,
    ) -> Result<(), HleError> {
        match name {
            "glCreateProgram" | "glCreateShader" => {
                let value = self.next_gl_name;
                self.next_gl_name = self.next_gl_name.wrapping_add(1).max(1);
                Ok(self.return32(cpu, value))
            }
            "glGetError" => Ok(self.return32(cpu, 0)),
            "glGetAttribLocation" | "glGetUniformLocation" => Ok(self.return32(cpu, 0)),
            "glIsTexture" => Ok(self.return32(cpu, 0)),
            _ => Ok(self.return32(cpu, 0)),
        }
    }

    fn alloc(&mut self, size: u32, align: u32) -> Result<u32, HleError> {
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
        self.allocations.push(HleAllocation { ptr: start, size });
        if std::env::var_os("AEMU_TRACE_HLE_ALLOC").is_some() {
            eprintln!("HLE alloc size={size:#x} align={align:#x} -> {start:#010x}");
        }
        Ok(start)
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
                | "malloc"
                | "calloc"
                | "realloc"
                | "free"
                | "__errno"
                | "__aeabi_idiv"
                | "__aeabi_uidiv"
                | "__aeabi_idivmod"
                | "__aeabi_uidivmod"
                | "gettimeofday"
                | "clock_gettime"
                | "time"
                | "pthread_self"
                | "fopen"
                | "fdopen"
                | "fclose"
                | "open"
                | "close"
                | "read"
                | "fread"
                | "write"
                | "fwrite"
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
    matches!(name, "_ZNSs14_M_replace_auxEjjjc")
}

fn init_ctype<M: Memory>(memory: &mut M, address: u32, upper: bool) -> Result<(), HleError> {
    for value in 0..=255u32 {
        let flags = ctype_flags(value as u8, upper);
        store16(memory, address.wrapping_add((value + 1) * 2), flags)?;
    }
    Ok(())
}

fn init_case_table<M: Memory>(memory: &mut M, address: u32, upper: bool) -> Result<(), HleError> {
    for value in 0..=255u32 {
        let byte = value as u8;
        let mapped = if upper {
            byte.to_ascii_uppercase()
        } else {
            byte.to_ascii_lowercase()
        };
        store32(
            memory,
            address.wrapping_add((value + 1) * 4),
            u32::from(mapped),
        )?;
    }
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

fn is_random_device_path(path: &str) -> bool {
    matches!(path, "/dev/urandom" | "/dev/random")
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

#[cfg(test)]
mod tests {
    use crate::armv6::Isa;
    use crate::guest_memory::MappedMemory;

    use super::*;

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
            "_ZNSs14_M_replace_auxEjjjc",
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
}
