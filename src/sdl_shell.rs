use crate::gles_trace::{
    TextureUploadMatch, texture_payload_stats, texture_payload_to_rgb, texture_upload_matches,
};
use crate::hle_imports::{GlesActive, GlesClientAttribPayload, GlesEvent};
use crate::host::{
    HostBackend, HostConfig, HostError, HostEvent, HostKey, HostResult, PointerPhase,
};
use crate::png_util::encode_rgb_png;
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::video::GLProfile;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

const GL_FALSE: u8 = 0;
const GL_TRUE: u8 = 1;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const GL_STATIC_DRAW: u32 = 0x88e4;
const GL_TEXTURE0: u32 = 0x84c0;
const GL_TEXTURE_2D: u32 = 0x0de1;
const GL_BLEND: u32 = 0x0be2;
const GL_CULL_FACE: u32 = 0x0b44;
const GL_DEPTH_TEST: u32 = 0x0b71;
const GL_SCISSOR_TEST: u32 = 0x0c11;
const GL_STENCIL_TEST: u32 = 0x0b90;
const GL_LESS: u32 = 0x0201;
const GL_ALWAYS: u32 = 0x0207;
const GL_KEEP: u32 = 0x1e00;
const GL_FRONT: u32 = 0x0404;
const GL_BACK: u32 = 0x0405;
const GL_FRONT_AND_BACK: u32 = 0x0408;
const GL_ONE: u32 = 1;
const GL_ZERO: u32 = 0;
const GL_RGBA: u32 = 0x1908;
const GL_BYTE: u32 = 0x1400;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_SHORT: u32 = 0x1402;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_INT: u32 = 0x1404;
const GL_UNSIGNED_INT: u32 = 0x1405;
const GL_FLOAT: u32 = 0x1406;
const GL_FIXED: u32 = 0x140c;
const GL_NO_ERROR: u32 = 0;
const GL_COMPILE_STATUS: u32 = 0x8b81;
const GL_LINK_STATUS: u32 = 0x8b82;
const GL_INFO_LOG_LENGTH: u32 = 0x8b84;
const GL_UNPACK_ALIGNMENT: u32 = 0x0cf5;

pub struct Sdl2Host {
    _sdl: sdl2::Sdl,
    _video: sdl2::VideoSubsystem,
    window: sdl2::video::Window,
    _gl_context: sdl2::video::GLContext,
    event_pump: sdl2::EventPump,
    gl: SdlGl,
    replay: SdlGlesReplay,
}

pub struct SdlFramebufferCapture {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SdlGlesReplayStats {
    pub draw_arrays: usize,
    pub draw_elements: usize,
    pub skipped_client_attrib_draws: usize,
    pub skipped_missing_index_draws: usize,
    pub readback_width: u32,
    pub readback_height: u32,
    pub readback_nonzero_rgb_pixels: usize,
    pub readback_nonzero_alpha_pixels: usize,
    pub gl_error_count: usize,
    pub first_gl_error_event_index: usize,
    pub first_gl_error_event_kind: Option<&'static str>,
    pub first_gl_error_code: u32,
}

type GlClear = unsafe extern "C" fn(u32);
type GlClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
type GlClearDepthf = unsafe extern "C" fn(f32);
type GlViewport = unsafe extern "C" fn(i32, i32, i32, i32);
type GlCreateShader = unsafe extern "C" fn(u32) -> u32;
type GlShaderSource = unsafe extern "C" fn(u32, i32, *const *const c_char, *const i32);
type GlCompileShader = unsafe extern "C" fn(u32);
type GlGetShaderiv = unsafe extern "C" fn(u32, u32, *mut i32);
type GlGetShaderInfoLog = unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char);
type GlCreateProgram = unsafe extern "C" fn() -> u32;
type GlAttachShader = unsafe extern "C" fn(u32, u32);
type GlBindAttribLocation = unsafe extern "C" fn(u32, u32, *const c_char);
type GlLinkProgram = unsafe extern "C" fn(u32);
type GlGetProgramiv = unsafe extern "C" fn(u32, u32, *mut i32);
type GlGetProgramInfoLog = unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char);
type GlUseProgram = unsafe extern "C" fn(u32);
type GlGetUniformLocation = unsafe extern "C" fn(u32, *const c_char) -> i32;
type GlGenBuffers = unsafe extern "C" fn(i32, *mut u32);
type GlBindBuffer = unsafe extern "C" fn(u32, u32);
type GlBufferData = unsafe extern "C" fn(u32, isize, *const c_void, u32);
type GlBufferSubData = unsafe extern "C" fn(u32, isize, isize, *const c_void);
type GlGenTextures = unsafe extern "C" fn(i32, *mut u32);
type GlBindTexture = unsafe extern "C" fn(u32, u32);
type GlGenFramebuffers = unsafe extern "C" fn(i32, *mut u32);
type GlGenRenderbuffers = unsafe extern "C" fn(i32, *mut u32);
type GlBindFramebuffer = unsafe extern "C" fn(u32, u32);
type GlBindRenderbuffer = unsafe extern "C" fn(u32, u32);
type GlFramebufferTexture2D = unsafe extern "C" fn(u32, u32, u32, u32, i32);
type GlFramebufferRenderbuffer = unsafe extern "C" fn(u32, u32, u32, u32);
type GlRenderbufferStorage = unsafe extern "C" fn(u32, u32, i32, i32);
type GlTexParameteri = unsafe extern "C" fn(u32, u32, i32);
type GlTexImage2D = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type GlTexSubImage2D = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type GlActiveTexture = unsafe extern "C" fn(u32);
type GlUniform1i = unsafe extern "C" fn(i32, i32);
type GlUniform1fv = unsafe extern "C" fn(i32, i32, *const f32);
type GlUniform2fv = unsafe extern "C" fn(i32, i32, *const f32);
type GlUniform3fv = unsafe extern "C" fn(i32, i32, *const f32);
type GlUniform4fv = unsafe extern "C" fn(i32, i32, *const f32);
type GlUniform1iv = unsafe extern "C" fn(i32, i32, *const i32);
type GlUniform2iv = unsafe extern "C" fn(i32, i32, *const i32);
type GlUniform3iv = unsafe extern "C" fn(i32, i32, *const i32);
type GlUniform4iv = unsafe extern "C" fn(i32, i32, *const i32);
type GlUniformMatrix2fv = unsafe extern "C" fn(i32, i32, u8, *const f32);
type GlUniformMatrix3fv = unsafe extern "C" fn(i32, i32, u8, *const f32);
type GlUniformMatrix4fv = unsafe extern "C" fn(i32, i32, u8, *const f32);
type GlVertexAttribPointer = unsafe extern "C" fn(u32, i32, u32, u8, i32, *const c_void);
type GlEnableVertexAttribArray = unsafe extern "C" fn(u32);
type GlEnable = unsafe extern "C" fn(u32);
type GlDisable = unsafe extern "C" fn(u32);
type GlBlendFunc = unsafe extern "C" fn(u32, u32);
type GlBlendFuncSeparate = unsafe extern "C" fn(u32, u32, u32, u32);
type GlStencilFuncSeparate = unsafe extern "C" fn(u32, u32, i32, u32);
type GlStencilOpSeparate = unsafe extern "C" fn(u32, u32, u32, u32);
type GlStencilMask = unsafe extern "C" fn(u32);
type GlCullFace = unsafe extern "C" fn(u32);
type GlPolygonOffset = unsafe extern "C" fn(f32, f32);
type GlDepthFunc = unsafe extern "C" fn(u32);
type GlDepthMask = unsafe extern "C" fn(u8);
type GlDepthRangef = unsafe extern "C" fn(f32, f32);
type GlColorMask = unsafe extern "C" fn(u8, u8, u8, u8);
type GlScissor = unsafe extern "C" fn(i32, i32, i32, i32);
type GlClearStencil = unsafe extern "C" fn(i32);
type GlDrawArrays = unsafe extern "C" fn(u32, i32, i32);
type GlDrawElements = unsafe extern "C" fn(u32, i32, u32, *const c_void);
type GlFlush = unsafe extern "C" fn();
type GlPixelStorei = unsafe extern "C" fn(u32, i32);
type GlReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
type GlGetError = unsafe extern "C" fn() -> u32;

struct SdlGl {
    clear: GlClear,
    clear_color: GlClearColor,
    clear_depthf: Option<GlClearDepthf>,
    viewport: GlViewport,
    create_shader: GlCreateShader,
    shader_source: GlShaderSource,
    compile_shader: GlCompileShader,
    get_shader_iv: GlGetShaderiv,
    get_shader_info_log: GlGetShaderInfoLog,
    create_program: GlCreateProgram,
    attach_shader: GlAttachShader,
    bind_attrib_location: GlBindAttribLocation,
    link_program: GlLinkProgram,
    get_program_iv: GlGetProgramiv,
    get_program_info_log: GlGetProgramInfoLog,
    use_program: GlUseProgram,
    get_uniform_location: GlGetUniformLocation,
    gen_buffers: GlGenBuffers,
    bind_buffer: GlBindBuffer,
    buffer_data: GlBufferData,
    buffer_sub_data: GlBufferSubData,
    gen_textures: GlGenTextures,
    bind_texture: GlBindTexture,
    gen_framebuffers: GlGenFramebuffers,
    gen_renderbuffers: GlGenRenderbuffers,
    bind_framebuffer: GlBindFramebuffer,
    bind_renderbuffer: GlBindRenderbuffer,
    framebuffer_texture_2d: GlFramebufferTexture2D,
    framebuffer_renderbuffer: GlFramebufferRenderbuffer,
    renderbuffer_storage: GlRenderbufferStorage,
    tex_parameteri: GlTexParameteri,
    tex_image_2d: GlTexImage2D,
    tex_sub_image_2d: GlTexSubImage2D,
    active_texture: GlActiveTexture,
    uniform_1i: GlUniform1i,
    uniform_1fv: GlUniform1fv,
    uniform_2fv: GlUniform2fv,
    uniform_3fv: GlUniform3fv,
    uniform_4fv: GlUniform4fv,
    uniform_1iv: GlUniform1iv,
    uniform_2iv: GlUniform2iv,
    uniform_3iv: GlUniform3iv,
    uniform_4iv: GlUniform4iv,
    uniform_matrix_2fv: GlUniformMatrix2fv,
    uniform_matrix_3fv: GlUniformMatrix3fv,
    uniform_matrix_4fv: GlUniformMatrix4fv,
    vertex_attrib_pointer: GlVertexAttribPointer,
    enable_vertex_attrib_array: GlEnableVertexAttribArray,
    enable: GlEnable,
    disable: GlDisable,
    blend_func: GlBlendFunc,
    blend_func_separate: GlBlendFuncSeparate,
    stencil_func_separate: GlStencilFuncSeparate,
    stencil_op_separate: GlStencilOpSeparate,
    stencil_mask: GlStencilMask,
    cull_face: GlCullFace,
    polygon_offset: GlPolygonOffset,
    depth_func: GlDepthFunc,
    depth_mask: GlDepthMask,
    depth_rangef: GlDepthRangef,
    color_mask: GlColorMask,
    scissor: GlScissor,
    clear_stencil: GlClearStencil,
    draw_arrays: GlDrawArrays,
    draw_elements: GlDrawElements,
    flush: GlFlush,
    pixel_storei: GlPixelStorei,
    read_pixels: GlReadPixels,
    get_error: GlGetError,
}

#[derive(Debug, Default)]
struct SdlGlesReplay {
    buffers: HashMap<u32, u32>,
    buffer_data: HashMap<u32, Vec<u8>>,
    textures: HashMap<u32, u32>,
    texture_info: HashMap<u32, ReplayTextureInfo>,
    framebuffers: HashMap<u32, u32>,
    renderbuffers: HashMap<u32, u32>,
    shaders: HashMap<u32, u32>,
    programs: HashMap<u32, ReplayProgram>,
    enabled_vertex_attribs: HashMap<u32, bool>,
    client_side_vertex_attribs: HashMap<u32, bool>,
    client_attrib_buffers: HashMap<u32, u32>,
    vertex_attribs: HashMap<u32, ReplayVertexAttrib>,
    current_program: u32,
    active_texture: u32,
    bound_textures: HashMap<(u32, u32), u32>,
    bound_array_buffer: u32,
    bound_element_array_buffer: u32,
    bound_framebuffer: u32,
    bound_renderbuffer: u32,
    viewport: (i32, i32, i32, i32),
    texture_upload_dump_index: usize,
    enabled_caps: HashMap<u32, bool>,
    blend_func: (u32, u32, u32, u32),
    depth_func: u32,
    depth_mask: bool,
    color_mask: (bool, bool, bool, bool),
    scissor: (i32, i32, i32, i32),
    stencil_front_func: (u32, i32, u32),
    stencil_back_func: (u32, i32, u32),
    stencil_front_op: (u32, u32, u32),
    stencil_back_op: (u32, u32, u32),
    stencil_mask: u32,
    stats: SdlGlesReplayStats,
}

#[derive(Debug)]
struct ReplayProgram {
    host: u32,
    uniforms: HashMap<u32, i32>,
    uniform_names: HashMap<u32, String>,
    attributes: Vec<GlesActive>,
}

#[derive(Debug, Clone, Copy)]
struct ReplayVertexAttrib {
    size: i32,
    ty: u32,
    normalized: bool,
    stride: i32,
    pointer: u32,
    buffer: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct ReplayTextureInfo {
    width: i32,
    height: i32,
    format: u32,
    ty: u32,
    last_upload_width: i32,
    last_upload_height: i32,
    last_payload_len: usize,
    last_nonzero_rgb_pixels: Option<usize>,
    last_nonzero_alpha_pixels: Option<usize>,
}

struct SdlDrawChangeTrace {
    limit: usize,
    dump_limit: usize,
    skip: usize,
    include_unchanged: bool,
    logged: usize,
    dumped: usize,
    submitted_draws: usize,
    previous_default_framebuffer: Option<Vec<u8>>,
    dump_dir: Option<PathBuf>,
    dump_matcher: Option<String>,
}

impl SdlDrawChangeTrace {
    fn from_env(host: &Sdl2Host) -> HostResult<Self> {
        let limit = env_usize("AEMU_TRACE_SDL_DRAW_CHANGES").unwrap_or(0);
        let dump_dir = std::env::var_os("AEMU_DUMP_SDL_DRAW_CHANGES_DIR").map(PathBuf::from);
        let dump_limit = env_usize("AEMU_DUMP_SDL_DRAW_CHANGES_LIMIT")
            .unwrap_or_else(|| if dump_dir.is_some() { limit.max(32) } else { 0 });
        if let Some(dir) = &dump_dir {
            fs::create_dir_all(dir).map_err(|err| HostError::new(err.to_string()))?;
        }
        let previous_default_framebuffer = if limit == 0 && dump_limit == 0 {
            None
        } else {
            Some(host.read_framebuffer_rgba()?.2)
        };
        Ok(Self {
            limit,
            dump_limit,
            skip: env_usize("AEMU_TRACE_SDL_DRAW_CHANGES_SKIP").unwrap_or(0),
            include_unchanged: std::env::var_os("AEMU_TRACE_SDL_DRAW_CHANGES_ALL").is_some(),
            logged: 0,
            dumped: 0,
            submitted_draws: host.replay.stats.draw_arrays + host.replay.stats.draw_elements,
            previous_default_framebuffer,
            dump_dir,
            dump_matcher: std::env::var("AEMU_DUMP_SDL_DRAW_CHANGES_MATCH").ok(),
        })
    }

    fn after_event(
        &mut self,
        host: &Sdl2Host,
        event_index: usize,
        event: &GlesEvent,
        before_draw_arrays: usize,
        before_draw_elements: usize,
    ) -> HostResult<()> {
        if self.limit == 0 && self.dump_limit == 0 {
            return Ok(());
        }
        let before_draws = before_draw_arrays + before_draw_elements;
        let after_draws = host.replay.stats.draw_arrays + host.replay.stats.draw_elements;
        if after_draws == before_draws {
            return Ok(());
        }
        self.submitted_draws += after_draws - before_draws;
        if self.submitted_draws <= self.skip {
            return Ok(());
        }

        let draw_count = match event {
            GlesEvent::DrawArrays { count, .. } | GlesEvent::DrawElements { count, .. } => *count,
            _ => 0,
        };
        let bound_texture_2d = host
            .replay
            .bound_textures
            .get(&(host.replay.active_texture, GL_TEXTURE_2D))
            .copied()
            .unwrap_or(0);
        let (viewport_x, viewport_y, viewport_width, viewport_height) = host.replay.viewport;
        let details = host.describe_draw_event(event);

        if host.replay.bound_framebuffer != 0 {
            if self.include_unchanged && self.logged < self.limit {
                self.logged += 1;
                eprintln!(
                    "SDL draw-change event={event_index} draw={} kind={} count={draw_count} fb={} program={} active_texture=0x{:04x} tex2d={} viewport={viewport_x},{viewport_y},{viewport_width},{viewport_height} skipped_default_readback=bound-fbo {details}",
                    self.submitted_draws,
                    event.kind(),
                    host.replay.bound_framebuffer,
                    host.replay.current_program,
                    host.replay.active_texture,
                    bound_texture_2d,
                );
            }
            return Ok(());
        }

        let (width, height, pixels) = host.read_framebuffer_rgba()?;
        let (changed_pixels, changed_bytes) = self
            .previous_default_framebuffer
            .as_deref()
            .map_or((0, 0), |previous| framebuffer_delta(previous, &pixels));
        let should_record = changed_pixels != 0 || self.include_unchanged;
        let should_dump = should_record
            && self.dumped < self.dump_limit
            && self.dump_dir.is_some()
            && self.dump_matcher.as_deref().map_or(true, |matcher| {
                sdl_draw_change_matches(
                    matcher,
                    DrawChangeMatch {
                        event_index,
                        draw: self.submitted_draws,
                        kind: event.kind(),
                        program: host.replay.current_program,
                        texture: bound_texture_2d,
                    },
                )
            });
        if should_dump {
            let texture_info = host.replay.texture_info.get(&bound_texture_2d).copied();
            self.dump_draw_png(
                event_index,
                event,
                draw_count,
                host.replay.current_program,
                host.replay.active_texture,
                bound_texture_2d,
                host.replay.viewport,
                changed_pixels,
                changed_bytes,
                width,
                height,
                texture_info,
                &pixels,
            )?;
        }
        self.previous_default_framebuffer = Some(pixels);
        if changed_pixels == 0 && !self.include_unchanged {
            return Ok(());
        }
        if self.logged >= self.limit {
            return Ok(());
        }
        self.logged += 1;
        eprintln!(
            "SDL draw-change event={event_index} draw={} kind={} count={draw_count} fb=0 program={} active_texture=0x{:04x} tex2d={} viewport={viewport_x},{viewport_y},{viewport_width},{viewport_height} changed_pixels={changed_pixels} changed_bytes={changed_bytes} {details}",
            self.submitted_draws,
            event.kind(),
            host.replay.current_program,
            host.replay.active_texture,
            bound_texture_2d,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn dump_draw_png(
        &mut self,
        event_index: usize,
        event: &GlesEvent,
        draw_count: i32,
        program: u32,
        active_texture: u32,
        texture: u32,
        viewport: (i32, i32, i32, i32),
        changed_pixels: usize,
        changed_bytes: usize,
        width: u32,
        height: u32,
        texture_info: Option<ReplayTextureInfo>,
        rgba: &[u8],
    ) -> HostResult<()> {
        let Some(dir) = self.dump_dir.as_deref() else {
            return Ok(());
        };
        let rgb = framebuffer_rgba_to_top_down_rgb(width, height, rgba)?;
        let png = encode_rgb_png(width, height, &rgb).map_err(HostError::new)?;
        let stem = format!(
            "{:04}-event{:05}-draw{:05}-{}-program{}-tex{}-{}x{}",
            self.dumped,
            event_index,
            self.submitted_draws,
            event.kind(),
            program,
            texture,
            width,
            height
        );
        let png_path = dir.join(format!("{stem}.png"));
        fs::write(&png_path, png).map_err(|err| HostError::new(err.to_string()))?;
        append_draw_dump_manifest(
            dir,
            &png_path,
            self.dumped,
            event_index,
            self.submitted_draws,
            event.kind(),
            draw_count,
            program,
            active_texture,
            texture,
            viewport,
            changed_pixels,
            changed_bytes,
            width,
            height,
            texture_info,
        )?;
        self.dumped += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct DrawChangeMatch<'a> {
    event_index: usize,
    draw: usize,
    kind: &'a str,
    program: u32,
    texture: u32,
}

fn sdl_draw_change_matches(matcher: &str, draw: DrawChangeMatch<'_>) -> bool {
    matcher
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.eq_ignore_ascii_case("all")
                || token.eq_ignore_ascii_case(draw.kind)
                || token.eq_ignore_ascii_case(&draw.event_index.to_string())
                || token.eq_ignore_ascii_case(&format!("event{}", draw.event_index))
                || token.eq_ignore_ascii_case(&format!("draw{}", draw.draw))
                || token.eq_ignore_ascii_case(&format!("program{}", draw.program))
                || token.eq_ignore_ascii_case(&format!("prog{}", draw.program))
                || token.eq_ignore_ascii_case(&format!("tex{}", draw.texture))
                || token.eq_ignore_ascii_case(&format!("0x{:x}", draw.texture))
        })
}

#[allow(clippy::too_many_arguments)]
fn append_draw_dump_manifest(
    dir: &Path,
    png_path: &Path,
    index: usize,
    event_index: usize,
    draw: usize,
    kind: &str,
    count: i32,
    program: u32,
    active_texture: u32,
    texture: u32,
    viewport: (i32, i32, i32, i32),
    changed_pixels: usize,
    changed_bytes: usize,
    width: u32,
    height: u32,
    texture_info: Option<ReplayTextureInfo>,
) -> HostResult<()> {
    use std::io::Write;

    let mut row = serde_json::json!({
        "index": index,
        "event_index": event_index,
        "draw": draw,
        "kind": kind,
        "count": count,
        "program": program,
        "active_texture": active_texture,
        "texture": texture,
        "viewport": [viewport.0, viewport.1, viewport.2, viewport.3],
        "changed_pixels": changed_pixels,
        "changed_bytes": changed_bytes,
        "width": width,
        "height": height,
        "png": png_path.file_name().and_then(|name| name.to_str()).unwrap_or(""),
    });
    if let Some(info) = texture_info {
        row["texture_width"] = serde_json::json!(info.width);
        row["texture_height"] = serde_json::json!(info.height);
        row["texture_format"] = serde_json::json!(info.format);
        row["texture_type"] = serde_json::json!(info.ty);
        row["texture_last_upload_width"] = serde_json::json!(info.last_upload_width);
        row["texture_last_upload_height"] = serde_json::json!(info.last_upload_height);
        row["texture_last_payload_len"] = serde_json::json!(info.last_payload_len);
        row["texture_last_nonzero_rgb_pixels"] = serde_json::json!(info.last_nonzero_rgb_pixels);
        row["texture_last_nonzero_alpha_pixels"] =
            serde_json::json!(info.last_nonzero_alpha_pixels);
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("draw_manifest.jsonl"))
        .map_err(|err| HostError::new(err.to_string()))?;
    writeln!(file, "{row}").map_err(|err| HostError::new(err.to_string()))
}

impl Sdl2Host {
    pub fn new(config: &HostConfig) -> HostResult<Self> {
        let sdl = sdl2::init().map_err(HostError::new)?;
        let video = sdl.video().map_err(HostError::new)?;

        let gl_attr = video.gl_attr();
        gl_attr.set_context_profile(GLProfile::GLES);
        gl_attr.set_context_version(2, 0);
        gl_attr.set_double_buffer(true);
        gl_attr.set_depth_size(24);
        gl_attr.set_stencil_size(8);

        let window = video
            .window(&config.title, config.width, config.height)
            .position_centered()
            .resizable()
            .opengl()
            .build()
            .map_err(|err| HostError::new(err.to_string()))?;
        let gl_context = window.gl_create_context().map_err(HostError::new)?;
        window
            .gl_make_current(&gl_context)
            .map_err(HostError::new)?;
        video.gl_set_swap_interval(1).map_err(HostError::new)?;
        let gl = SdlGl::load(&video)?;
        let event_pump = sdl.event_pump().map_err(HostError::new)?;
        let mut replay = SdlGlesReplay {
            active_texture: GL_TEXTURE0,
            viewport: (0, 0, config.width as i32, config.height as i32),
            blend_func: (GL_ONE, GL_ZERO, GL_ONE, GL_ZERO),
            depth_func: GL_LESS,
            depth_mask: true,
            color_mask: (true, true, true, true),
            scissor: (0, 0, config.width as i32, config.height as i32),
            stencil_front_func: (GL_ALWAYS, 0, u32::MAX),
            stencil_back_func: (GL_ALWAYS, 0, u32::MAX),
            stencil_front_op: (GL_KEEP, GL_KEEP, GL_KEEP),
            stencil_back_op: (GL_KEEP, GL_KEEP, GL_KEEP),
            stencil_mask: u32::MAX,
            ..SdlGlesReplay::default()
        };
        replay
            .bound_textures
            .insert((GL_TEXTURE0, GL_TEXTURE_2D), 0);

        Ok(Self {
            _sdl: sdl,
            _video: video,
            window,
            _gl_context: gl_context,
            event_pump,
            gl,
            replay,
        })
    }

    pub fn replay_stats(&self) -> SdlGlesReplayStats {
        self.replay.stats
    }

    pub fn capture_framebuffer_rgb(&self) -> HostResult<SdlFramebufferCapture> {
        let (width, height, rgba) = self.read_framebuffer_rgba()?;
        let rgb = framebuffer_rgba_to_top_down_rgb(width, height, &rgba)?;
        Ok(SdlFramebufferCapture { width, height, rgb })
    }
}

fn framebuffer_rgba_to_top_down_rgb(width: u32, height: u32, rgba: &[u8]) -> HostResult<Vec<u8>> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    let expected = width_usize
        .checked_mul(height_usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            HostError::new(format!(
                "framebuffer dimensions too large: {width}x{height}"
            ))
        })?;
    if rgba.len() < expected {
        return Err(HostError::new(format!(
            "framebuffer payload too short: {} bytes for {width}x{height}",
            rgba.len()
        )));
    }
    let mut rgb = Vec::with_capacity(width_usize * height_usize * 3);
    for y in (0..height_usize).rev() {
        let row = y * width_usize * 4;
        for x in 0..width_usize {
            let pixel = row + x * 4;
            rgb.extend_from_slice(&rgba[pixel..pixel + 3]);
        }
    }
    Ok(rgb)
}

impl SdlGl {
    fn load(video: &sdl2::VideoSubsystem) -> HostResult<Self> {
        Ok(Self {
            clear: load_required_gl(video, "glClear")?,
            clear_color: load_required_gl(video, "glClearColor")?,
            clear_depthf: load_optional_gl(video, "glClearDepthf"),
            viewport: load_required_gl(video, "glViewport")?,
            create_shader: load_required_gl(video, "glCreateShader")?,
            shader_source: load_required_gl(video, "glShaderSource")?,
            compile_shader: load_required_gl(video, "glCompileShader")?,
            get_shader_iv: load_required_gl(video, "glGetShaderiv")?,
            get_shader_info_log: load_required_gl(video, "glGetShaderInfoLog")?,
            create_program: load_required_gl(video, "glCreateProgram")?,
            attach_shader: load_required_gl(video, "glAttachShader")?,
            bind_attrib_location: load_required_gl(video, "glBindAttribLocation")?,
            link_program: load_required_gl(video, "glLinkProgram")?,
            get_program_iv: load_required_gl(video, "glGetProgramiv")?,
            get_program_info_log: load_required_gl(video, "glGetProgramInfoLog")?,
            use_program: load_required_gl(video, "glUseProgram")?,
            get_uniform_location: load_required_gl(video, "glGetUniformLocation")?,
            gen_buffers: load_required_gl(video, "glGenBuffers")?,
            bind_buffer: load_required_gl(video, "glBindBuffer")?,
            buffer_data: load_required_gl(video, "glBufferData")?,
            buffer_sub_data: load_required_gl(video, "glBufferSubData")?,
            gen_textures: load_required_gl(video, "glGenTextures")?,
            bind_texture: load_required_gl(video, "glBindTexture")?,
            gen_framebuffers: load_required_gl(video, "glGenFramebuffers")?,
            gen_renderbuffers: load_required_gl(video, "glGenRenderbuffers")?,
            bind_framebuffer: load_required_gl(video, "glBindFramebuffer")?,
            bind_renderbuffer: load_required_gl(video, "glBindRenderbuffer")?,
            framebuffer_texture_2d: load_required_gl(video, "glFramebufferTexture2D")?,
            framebuffer_renderbuffer: load_required_gl(video, "glFramebufferRenderbuffer")?,
            renderbuffer_storage: load_required_gl(video, "glRenderbufferStorage")?,
            tex_parameteri: load_required_gl(video, "glTexParameteri")?,
            tex_image_2d: load_required_gl(video, "glTexImage2D")?,
            tex_sub_image_2d: load_required_gl(video, "glTexSubImage2D")?,
            active_texture: load_required_gl(video, "glActiveTexture")?,
            uniform_1i: load_required_gl(video, "glUniform1i")?,
            uniform_1fv: load_required_gl(video, "glUniform1fv")?,
            uniform_2fv: load_required_gl(video, "glUniform2fv")?,
            uniform_3fv: load_required_gl(video, "glUniform3fv")?,
            uniform_4fv: load_required_gl(video, "glUniform4fv")?,
            uniform_1iv: load_required_gl(video, "glUniform1iv")?,
            uniform_2iv: load_required_gl(video, "glUniform2iv")?,
            uniform_3iv: load_required_gl(video, "glUniform3iv")?,
            uniform_4iv: load_required_gl(video, "glUniform4iv")?,
            uniform_matrix_2fv: load_required_gl(video, "glUniformMatrix2fv")?,
            uniform_matrix_3fv: load_required_gl(video, "glUniformMatrix3fv")?,
            uniform_matrix_4fv: load_required_gl(video, "glUniformMatrix4fv")?,
            vertex_attrib_pointer: load_required_gl(video, "glVertexAttribPointer")?,
            enable_vertex_attrib_array: load_required_gl(video, "glEnableVertexAttribArray")?,
            enable: load_required_gl(video, "glEnable")?,
            disable: load_required_gl(video, "glDisable")?,
            blend_func: load_required_gl(video, "glBlendFunc")?,
            blend_func_separate: load_required_gl(video, "glBlendFuncSeparate")?,
            stencil_func_separate: load_required_gl(video, "glStencilFuncSeparate")?,
            stencil_op_separate: load_required_gl(video, "glStencilOpSeparate")?,
            stencil_mask: load_required_gl(video, "glStencilMask")?,
            cull_face: load_required_gl(video, "glCullFace")?,
            polygon_offset: load_required_gl(video, "glPolygonOffset")?,
            depth_func: load_required_gl(video, "glDepthFunc")?,
            depth_mask: load_required_gl(video, "glDepthMask")?,
            depth_rangef: load_required_gl(video, "glDepthRangef")?,
            color_mask: load_required_gl(video, "glColorMask")?,
            scissor: load_required_gl(video, "glScissor")?,
            clear_stencil: load_required_gl(video, "glClearStencil")?,
            draw_arrays: load_required_gl(video, "glDrawArrays")?,
            draw_elements: load_required_gl(video, "glDrawElements")?,
            flush: load_required_gl(video, "glFlush")?,
            pixel_storei: load_required_gl(video, "glPixelStorei")?,
            read_pixels: load_required_gl(video, "glReadPixels")?,
            get_error: load_required_gl(video, "glGetError")?,
        })
    }
}

fn load_required_gl<T: Copy>(video: &sdl2::VideoSubsystem, name: &str) -> HostResult<T> {
    load_optional_gl(video, name).ok_or_else(|| HostError::new(format!("missing GL symbol {name}")))
}

fn load_optional_gl<T: Copy>(video: &sdl2::VideoSubsystem, name: &str) -> Option<T> {
    let ptr = video.gl_get_proc_address(name) as *const ();
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute_copy(&ptr) })
    }
}

impl Sdl2Host {
    fn replay_gles_event(&mut self, event: &GlesEvent) -> HostResult<()> {
        match event {
            GlesEvent::CreateShader {
                shader,
                shader_type,
            } => {
                let host = unsafe { (self.gl.create_shader)(*shader_type) };
                self.replay.shaders.insert(*shader, host);
            }
            GlesEvent::ShaderSource { shader, source } => {
                if let Some(host) = self.replay.shaders.get(shader).copied() {
                    self.compile_replay_shader(*shader, host, source)?;
                }
            }
            GlesEvent::CreateProgram { program } => {
                let host = unsafe { (self.gl.create_program)() };
                self.replay.programs.insert(
                    *program,
                    ReplayProgram {
                        host,
                        uniforms: HashMap::new(),
                        uniform_names: HashMap::new(),
                        attributes: Vec::new(),
                    },
                );
            }
            GlesEvent::AttachShader { program, shader } => {
                let Some(host_program) = self.host_program(*program) else {
                    return Ok(());
                };
                let Some(host_shader) = self.replay.shaders.get(shader).copied() else {
                    return Ok(());
                };
                unsafe {
                    (self.gl.attach_shader)(host_program, host_shader);
                }
            }
            GlesEvent::LinkProgram {
                program,
                uniforms,
                attributes,
            } => self.link_replay_program(*program, uniforms, attributes)?,
            GlesEvent::ActiveTexture { texture } => {
                self.replay.active_texture = *texture;
                unsafe {
                    (self.gl.active_texture)(*texture);
                }
            }
            GlesEvent::BindBuffer { target, buffer } => {
                let host = self.host_buffer(*buffer);
                unsafe {
                    (self.gl.bind_buffer)(*target, host);
                }
                match *target {
                    GL_ARRAY_BUFFER => self.replay.bound_array_buffer = *buffer,
                    GL_ELEMENT_ARRAY_BUFFER => self.replay.bound_element_array_buffer = *buffer,
                    _ => {}
                }
            }
            GlesEvent::BufferData {
                target,
                size,
                usage,
                payload,
                ..
            } => {
                let size = gl_size(*size, "glBufferData size")?;
                let data = payload_ptr(payload.as_deref(), size);
                self.rebind_guest_buffer(*target);
                unsafe {
                    (self.gl.buffer_data)(*target, size, data, *usage);
                }
                if let Some(guest) = self.bound_guest_buffer(*target) {
                    let mut stored = vec![0_u8; size as usize];
                    if let Some(payload) = payload.as_deref() {
                        let copy_len = stored.len().min(payload.len());
                        stored[..copy_len].copy_from_slice(&payload[..copy_len]);
                    }
                    self.replay.buffer_data.insert(guest, stored);
                }
            }
            GlesEvent::BufferSubData {
                target,
                offset,
                size,
                payload,
                ..
            } => {
                let offset = gl_size(*offset, "glBufferSubData offset")?;
                let size = gl_size(*size, "glBufferSubData size")?;
                if let Some(payload) = payload_bytes(payload.as_deref(), size) {
                    self.rebind_guest_buffer(*target);
                    unsafe {
                        (self.gl.buffer_sub_data)(*target, offset, size, payload.as_ptr().cast());
                    }
                    if let Some(guest) = self.bound_guest_buffer(*target) {
                        update_buffer_data(
                            &mut self.replay.buffer_data,
                            guest,
                            offset as usize,
                            size as usize,
                            payload,
                        );
                    }
                }
            }
            GlesEvent::BindTexture { target, texture } => {
                let host = self.host_texture(*texture);
                self.replay
                    .bound_textures
                    .insert((self.replay.active_texture, *target), *texture);
                unsafe {
                    (self.gl.bind_texture)(*target, host);
                }
            }
            GlesEvent::BindFramebuffer {
                target,
                framebuffer,
            } => {
                let host = self.host_framebuffer(*framebuffer);
                self.replay.bound_framebuffer = *framebuffer;
                unsafe {
                    (self.gl.bind_framebuffer)(*target, host);
                }
            }
            GlesEvent::BindRenderbuffer {
                target,
                renderbuffer,
            } => {
                let host = self.host_renderbuffer(*renderbuffer);
                self.replay.bound_renderbuffer = *renderbuffer;
                unsafe {
                    (self.gl.bind_renderbuffer)(*target, host);
                }
            }
            GlesEvent::FramebufferTexture2D {
                target,
                attachment,
                textarget,
                texture,
                level,
            } => {
                let host = self.host_texture(*texture);
                unsafe {
                    (self.gl.framebuffer_texture_2d)(
                        *target,
                        *attachment,
                        *textarget,
                        host,
                        *level,
                    );
                }
            }
            GlesEvent::FramebufferRenderbuffer {
                target,
                attachment,
                renderbuffertarget,
                renderbuffer,
            } => {
                let host = self.host_renderbuffer(*renderbuffer);
                unsafe {
                    (self.gl.framebuffer_renderbuffer)(
                        *target,
                        *attachment,
                        *renderbuffertarget,
                        host,
                    );
                }
            }
            GlesEvent::RenderbufferStorage {
                target,
                internal_format,
                width,
                height,
            } => unsafe {
                (self.gl.renderbuffer_storage)(*target, *internal_format, *width, *height);
            },
            GlesEvent::TexParameteri {
                target,
                name,
                value,
            } => unsafe {
                (self.gl.tex_parameteri)(*target, *name, *value as i32);
            },
            GlesEvent::TexImage2D {
                target,
                level,
                internal_format,
                width,
                height,
                border,
                format,
                ty,
                payload,
                ..
            } => {
                let texture = self.bound_guest_texture(*target).unwrap_or(0);
                self.maybe_dump_texture_upload(
                    "teximage2d",
                    texture,
                    *width,
                    *height,
                    *format,
                    *ty,
                    payload.as_deref(),
                );
                let data = payload
                    .as_ref()
                    .map_or(ptr::null(), |payload| payload.as_ptr().cast::<c_void>());
                self.track_texture_image(
                    *target,
                    *level,
                    *width,
                    *height,
                    *format,
                    *ty,
                    payload.as_deref(),
                );
                unsafe {
                    (self.gl.tex_image_2d)(
                        *target,
                        *level,
                        *internal_format,
                        *width,
                        *height,
                        *border,
                        *format,
                        *ty,
                        data,
                    );
                }
            }
            GlesEvent::TexSubImage2D {
                target,
                level,
                xoffset,
                yoffset,
                width,
                height,
                format,
                ty,
                payload,
                ..
            } => {
                let texture = self.bound_guest_texture(*target).unwrap_or(0);
                self.maybe_dump_texture_upload(
                    "texsubimage2d",
                    texture,
                    *width,
                    *height,
                    *format,
                    *ty,
                    payload.as_deref(),
                );
                if let Some(payload) = payload {
                    self.track_texture_sub_image(
                        *target,
                        *level,
                        *width,
                        *height,
                        *format,
                        *ty,
                        Some(payload),
                    );
                    unsafe {
                        (self.gl.tex_sub_image_2d)(
                            *target,
                            *level,
                            *xoffset,
                            *yoffset,
                            *width,
                            *height,
                            *format,
                            *ty,
                            payload.as_ptr().cast(),
                        );
                    }
                }
            }
            GlesEvent::UseProgram { program } => {
                let host = self.host_program(*program).unwrap_or(0);
                unsafe {
                    (self.gl.use_program)(host);
                }
                self.replay.current_program = *program;
            }
            GlesEvent::Uniform1i { location, value } => {
                let location = self.host_uniform_location(*location);
                if location >= 0 {
                    unsafe {
                        (self.gl.uniform_1i)(location, *value);
                    }
                }
            }
            GlesEvent::UniformVector {
                components,
                integer,
                location,
                count,
                payload,
                ..
            } => self.replay_uniform_vector(*components, *integer, *location, *count, payload)?,
            GlesEvent::UniformMatrix {
                columns,
                location,
                count,
                transpose,
                payload,
                ..
            } => self.replay_uniform_matrix(*columns, *location, *count, *transpose, payload)?,
            GlesEvent::VertexAttribPointer {
                index,
                size,
                ty,
                normalized,
                stride,
                pointer,
            } => {
                let has_array_buffer = self.replay.bound_array_buffer != 0;
                self.replay.vertex_attribs.insert(
                    *index,
                    ReplayVertexAttrib {
                        size: *size,
                        ty: *ty,
                        normalized: *normalized,
                        stride: *stride,
                        pointer: *pointer,
                        buffer: self.replay.bound_array_buffer,
                    },
                );
                self.replay
                    .client_side_vertex_attribs
                    .insert(*index, !has_array_buffer);
                if has_array_buffer {
                    self.rebind_guest_buffer(GL_ARRAY_BUFFER);
                    unsafe {
                        (self.gl.vertex_attrib_pointer)(
                            *index,
                            *size,
                            *ty,
                            gl_bool(*normalized),
                            *stride,
                            offset_ptr(*pointer),
                        );
                    }
                }
            }
            GlesEvent::EnableVertexAttribArray { index } => {
                self.replay.enabled_vertex_attribs.insert(*index, true);
                unsafe {
                    (self.gl.enable_vertex_attrib_array)(*index);
                }
            }
            GlesEvent::Enable { cap } => {
                let enabled = !(*cap == GL_STENCIL_TEST && env_flag("AEMU_SDL_DISABLE_STENCIL"));
                self.replay.enabled_caps.insert(*cap, enabled);
                unsafe {
                    if enabled {
                        (self.gl.enable)(*cap);
                    } else {
                        (self.gl.disable)(*cap);
                    }
                }
            }
            GlesEvent::Disable { cap } => {
                self.replay.enabled_caps.insert(*cap, false);
                unsafe {
                    (self.gl.disable)(*cap);
                }
            }
            GlesEvent::BlendFunc { sfactor, dfactor } => {
                self.replay.blend_func = (*sfactor, *dfactor, *sfactor, *dfactor);
                unsafe {
                    (self.gl.blend_func)(*sfactor, *dfactor);
                }
            }
            GlesEvent::BlendFuncSeparate {
                src_rgb,
                dst_rgb,
                src_alpha,
                dst_alpha,
            } => {
                self.replay.blend_func = (*src_rgb, *dst_rgb, *src_alpha, *dst_alpha);
                unsafe {
                    (self.gl.blend_func_separate)(*src_rgb, *dst_rgb, *src_alpha, *dst_alpha);
                }
            }
            GlesEvent::StencilFuncSeparate {
                face,
                func,
                reference,
                mask,
            } => {
                self.set_stencil_func(*face, *func, *reference, *mask);
                unsafe {
                    (self.gl.stencil_func_separate)(*face, *func, *reference, *mask);
                }
            }
            GlesEvent::StencilOpSeparate {
                face,
                sfail,
                dpfail,
                dppass,
            } => {
                self.set_stencil_op(*face, *sfail, *dpfail, *dppass);
                unsafe {
                    (self.gl.stencil_op_separate)(*face, *sfail, *dpfail, *dppass);
                }
            }
            GlesEvent::StencilMask { mask } => {
                self.replay.stencil_mask = *mask;
                unsafe {
                    (self.gl.stencil_mask)(*mask);
                }
            }
            GlesEvent::CullFace { mode } => unsafe {
                (self.gl.cull_face)(*mode);
            },
            GlesEvent::PolygonOffset { factor, units } => unsafe {
                (self.gl.polygon_offset)(f32::from_bits(*factor), f32::from_bits(*units));
            },
            GlesEvent::DepthFunc { func } => {
                self.replay.depth_func = *func;
                unsafe {
                    (self.gl.depth_func)(*func);
                }
            }
            GlesEvent::DepthMask { enabled } => {
                self.replay.depth_mask = *enabled;
                unsafe {
                    (self.gl.depth_mask)(gl_bool(*enabled));
                }
            }
            GlesEvent::DepthRangef { near, far } => unsafe {
                (self.gl.depth_rangef)(f32::from_bits(*near), f32::from_bits(*far));
            },
            GlesEvent::ColorMask {
                red,
                green,
                blue,
                alpha,
            } => {
                self.replay.color_mask = (*red, *green, *blue, *alpha);
                unsafe {
                    (self.gl.color_mask)(
                        gl_bool(*red),
                        gl_bool(*green),
                        gl_bool(*blue),
                        gl_bool(*alpha),
                    );
                }
            }
            GlesEvent::Scissor {
                x,
                y,
                width,
                height,
            } => {
                self.replay.scissor = (*x, *y, *width, *height);
                unsafe {
                    (self.gl.scissor)(*x, *y, *width, *height);
                }
            }
            GlesEvent::ClearColor {
                red,
                green,
                blue,
                alpha,
            } => unsafe {
                (self.gl.clear_color)(
                    f32::from_bits(*red),
                    f32::from_bits(*green),
                    f32::from_bits(*blue),
                    f32::from_bits(*alpha),
                );
            },
            GlesEvent::ClearDepthf { depth } => {
                if let Some(clear_depthf) = self.gl.clear_depthf {
                    unsafe {
                        clear_depthf(f32::from_bits(*depth));
                    }
                }
            }
            GlesEvent::ClearStencil { value } => unsafe {
                (self.gl.clear_stencil)(*value);
            },
            GlesEvent::Clear { mask } => unsafe {
                (self.gl.clear)(*mask);
            },
            GlesEvent::Viewport {
                x,
                y,
                width,
                height,
            } => {
                self.replay.viewport = (*x, *y, *width, *height);
                unsafe {
                    (self.gl.viewport)(*x, *y, *width, *height);
                }
            }
            GlesEvent::DrawArrays {
                mode,
                first,
                count,
                client_attribs,
            } => {
                if !self.prepare_client_attribs(client_attribs)? {
                    self.replay.stats.skipped_client_attrib_draws += 1;
                    return Ok(());
                }
                unsafe {
                    (self.gl.draw_arrays)(*mode, *first, *count);
                }
                self.replay.stats.draw_arrays += 1;
            }
            GlesEvent::DrawElements {
                mode,
                count,
                ty,
                indices,
                index_payload,
                client_attribs,
            } => {
                if !self.prepare_client_attribs(client_attribs)? {
                    self.replay.stats.skipped_client_attrib_draws += 1;
                    return Ok(());
                }
                self.rebind_guest_buffer(GL_ELEMENT_ARRAY_BUFFER);
                let Some(indices) = self.draw_indices_ptr(*indices, index_payload.as_deref())
                else {
                    self.replay.stats.skipped_missing_index_draws += 1;
                    return Ok(());
                };
                unsafe {
                    (self.gl.draw_elements)(*mode, *count, *ty, indices);
                }
                self.replay.stats.draw_elements += 1;
            }
            GlesEvent::Flush => unsafe {
                (self.gl.flush)();
            },
            GlesEvent::SwapBuffers { .. } => {
                self.capture_framebuffer_readback()?;
                self.swap_buffers()?;
            }
        }
        Ok(())
    }

    fn compile_replay_shader(
        &self,
        guest_shader: u32,
        host_shader: u32,
        source: &str,
    ) -> HostResult<()> {
        let source = CString::new(source)
            .map_err(|_| HostError::new(format!("shader {guest_shader} source contains NUL")))?;
        let ptr = source.as_ptr();
        let len = source.as_bytes().len() as i32;
        unsafe {
            (self.gl.shader_source)(host_shader, 1, &ptr, &len);
            (self.gl.compile_shader)(host_shader);
        }
        let mut status = 0;
        unsafe {
            (self.gl.get_shader_iv)(host_shader, GL_COMPILE_STATUS, &mut status);
        }
        if status == 0 {
            return Err(HostError::new(format!(
                "shader {guest_shader} compile failed: {}",
                self.shader_info_log(host_shader)
            )));
        }
        Ok(())
    }

    fn link_replay_program(
        &mut self,
        guest_program: u32,
        uniforms: &[GlesActive],
        attributes: &[GlesActive],
    ) -> HostResult<()> {
        let Some(host_program) = self.host_program(guest_program) else {
            return Ok(());
        };
        for attribute in attributes {
            if let Ok(name) = CString::new(attribute.name.as_str()) {
                unsafe {
                    (self.gl.bind_attrib_location)(host_program, attribute.location, name.as_ptr());
                }
            }
        }
        unsafe {
            (self.gl.link_program)(host_program);
        }
        let mut status = 0;
        unsafe {
            (self.gl.get_program_iv)(host_program, GL_LINK_STATUS, &mut status);
        }
        if status == 0 {
            return Err(HostError::new(format!(
                "program {guest_program} link failed: {}",
                self.program_info_log(host_program)
            )));
        }

        let mut host_uniforms = HashMap::new();
        let mut uniform_names = HashMap::new();
        for uniform in uniforms {
            if let Some(location) = self.lookup_host_uniform(host_program, &uniform.name) {
                host_uniforms.insert(uniform.location, location);
                uniform_names.insert(uniform.location, uniform.name.clone());
            }
        }
        if let Some(program) = self.replay.programs.get_mut(&guest_program) {
            program.uniforms = host_uniforms;
            program.uniform_names = uniform_names;
            program.attributes = attributes.to_vec();
        }
        Ok(())
    }

    fn replay_uniform_vector(
        &self,
        components: u8,
        integer: bool,
        guest_location: i32,
        count: i32,
        payload: &Option<Vec<u8>>,
    ) -> HostResult<()> {
        let location = self.host_uniform_location(guest_location);
        if location < 0 || count <= 0 {
            return Ok(());
        }
        let width = usize::from(components)
            .checked_mul(4)
            .ok_or_else(|| HostError::new("uniform vector width overflow"))?;
        let needed = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(width))
            .ok_or_else(|| HostError::new("uniform vector payload size overflow"))?;
        let Some(payload) = payload.as_ref().filter(|payload| payload.len() >= needed) else {
            return Ok(());
        };
        unsafe {
            if integer {
                let values = payload.as_ptr().cast::<i32>();
                match components {
                    1 => (self.gl.uniform_1iv)(location, count, values),
                    2 => (self.gl.uniform_2iv)(location, count, values),
                    3 => (self.gl.uniform_3iv)(location, count, values),
                    4 => (self.gl.uniform_4iv)(location, count, values),
                    _ => {}
                }
            } else {
                let values = payload.as_ptr().cast::<f32>();
                match components {
                    1 => (self.gl.uniform_1fv)(location, count, values),
                    2 => (self.gl.uniform_2fv)(location, count, values),
                    3 => (self.gl.uniform_3fv)(location, count, values),
                    4 => (self.gl.uniform_4fv)(location, count, values),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn replay_uniform_matrix(
        &self,
        columns: u8,
        guest_location: i32,
        count: i32,
        transpose: bool,
        payload: &Option<Vec<u8>>,
    ) -> HostResult<()> {
        let location = self.host_uniform_location(guest_location);
        if location < 0 || count <= 0 {
            return Ok(());
        }
        let columns = usize::from(columns);
        let width = columns
            .checked_mul(columns)
            .and_then(|items| items.checked_mul(4))
            .ok_or_else(|| HostError::new("uniform matrix width overflow"))?;
        let needed = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(width))
            .ok_or_else(|| HostError::new("uniform matrix payload size overflow"))?;
        let Some(payload) = payload.as_ref().filter(|payload| payload.len() >= needed) else {
            return Ok(());
        };
        let values = payload.as_ptr().cast::<f32>();
        unsafe {
            match columns {
                2 => (self.gl.uniform_matrix_2fv)(location, count, gl_bool(transpose), values),
                3 => (self.gl.uniform_matrix_3fv)(location, count, gl_bool(transpose), values),
                4 => (self.gl.uniform_matrix_4fv)(location, count, gl_bool(transpose), values),
                _ => {}
            }
        }
        Ok(())
    }

    fn host_buffer(&mut self, guest: u32) -> u32 {
        if guest == 0 {
            return 0;
        }
        if let Some(host) = self.replay.buffers.get(&guest).copied() {
            return host;
        }
        let mut host = 0;
        unsafe {
            (self.gl.gen_buffers)(1, &mut host);
        }
        self.replay.buffers.insert(guest, host);
        host
    }

    fn rebind_guest_buffer(&mut self, target: u32) {
        let guest = match target {
            GL_ARRAY_BUFFER => self.replay.bound_array_buffer,
            GL_ELEMENT_ARRAY_BUFFER => self.replay.bound_element_array_buffer,
            _ => return,
        };
        let host = self.host_buffer(guest);
        unsafe {
            (self.gl.bind_buffer)(target, host);
        }
    }

    fn bound_guest_buffer(&self, target: u32) -> Option<u32> {
        let guest = match target {
            GL_ARRAY_BUFFER => self.replay.bound_array_buffer,
            GL_ELEMENT_ARRAY_BUFFER => self.replay.bound_element_array_buffer,
            _ => return None,
        };
        (guest != 0).then_some(guest)
    }

    fn prepare_client_attribs(
        &mut self,
        client_attribs: &[GlesClientAttribPayload],
    ) -> HostResult<bool> {
        if !self.has_unbacked_client_attrib() {
            return Ok(true);
        }
        for (index, enabled) in self.replay.enabled_vertex_attribs.clone() {
            if !enabled
                || !self
                    .replay
                    .client_side_vertex_attribs
                    .get(&index)
                    .copied()
                    .unwrap_or(false)
            {
                continue;
            }
            let Some(attrib) = self.replay.vertex_attribs.get(&index).copied() else {
                return Ok(false);
            };
            let Some(payload) = client_attribs
                .iter()
                .find(|payload| payload.index == index)
                .and_then(|payload| payload.payload.as_deref())
            else {
                return Ok(false);
            };
            let buffer = self.client_attrib_buffer(index);
            unsafe {
                (self.gl.bind_buffer)(GL_ARRAY_BUFFER, buffer);
                (self.gl.buffer_data)(
                    GL_ARRAY_BUFFER,
                    isize::try_from(payload.len()).map_err(|_| {
                        HostError::new(format!(
                            "client vertex attrib {index} payload too large: {}",
                            payload.len()
                        ))
                    })?,
                    payload.as_ptr().cast(),
                    GL_STATIC_DRAW,
                );
                (self.gl.vertex_attrib_pointer)(
                    index,
                    attrib.size,
                    attrib.ty,
                    gl_bool(attrib.normalized),
                    attrib.stride,
                    ptr::null(),
                );
            }
        }
        Ok(true)
    }

    fn client_attrib_buffer(&mut self, index: u32) -> u32 {
        if let Some(buffer) = self.replay.client_attrib_buffers.get(&index).copied() {
            return buffer;
        }
        let mut buffer = 0;
        unsafe {
            (self.gl.gen_buffers)(1, &mut buffer);
        }
        self.replay.client_attrib_buffers.insert(index, buffer);
        buffer
    }

    fn host_texture(&mut self, guest: u32) -> u32 {
        if guest == 0 {
            return 0;
        }
        if let Some(host) = self.replay.textures.get(&guest).copied() {
            return host;
        }
        let mut host = 0;
        unsafe {
            (self.gl.gen_textures)(1, &mut host);
        }
        self.replay.textures.insert(guest, host);
        host
    }

    fn bound_guest_texture(&self, target: u32) -> Option<u32> {
        self.replay
            .bound_textures
            .get(&(self.replay.active_texture, target))
            .copied()
            .filter(|texture| *texture != 0)
    }

    fn track_texture_image(
        &mut self,
        target: u32,
        level: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        payload: Option<&[u8]>,
    ) {
        if level != 0 {
            return;
        }
        let Some(texture) = self.bound_guest_texture(target) else {
            return;
        };
        let stats =
            payload.and_then(|payload| texture_payload_stats(width, height, format, ty, payload));
        self.replay.texture_info.insert(
            texture,
            ReplayTextureInfo {
                width,
                height,
                format,
                ty,
                last_upload_width: width,
                last_upload_height: height,
                last_payload_len: payload.map_or(0, <[u8]>::len),
                last_nonzero_rgb_pixels: stats.map(|stats| stats.nonzero_rgb_pixels),
                last_nonzero_alpha_pixels: stats.map(|stats| stats.nonzero_alpha_pixels),
            },
        );
    }

    fn track_texture_sub_image(
        &mut self,
        target: u32,
        level: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        payload: Option<&[u8]>,
    ) {
        if level != 0 {
            return;
        }
        let Some(texture) = self.bound_guest_texture(target) else {
            return;
        };
        let stats =
            payload.and_then(|payload| texture_payload_stats(width, height, format, ty, payload));
        let entry = self.replay.texture_info.entry(texture).or_default();
        if entry.width == 0 || entry.height == 0 {
            entry.width = width;
            entry.height = height;
            entry.format = format;
            entry.ty = ty;
        }
        entry.last_upload_width = width;
        entry.last_upload_height = height;
        entry.last_payload_len = payload.map_or(0, <[u8]>::len);
        entry.last_nonzero_rgb_pixels = stats.map(|stats| stats.nonzero_rgb_pixels);
        entry.last_nonzero_alpha_pixels = stats.map(|stats| stats.nonzero_alpha_pixels);
    }

    fn maybe_dump_texture_upload(
        &mut self,
        kind: &str,
        texture: u32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        payload: Option<&[u8]>,
    ) {
        let Some(payload) = payload else {
            return;
        };
        let Some(dir) = std::env::var_os("AEMU_DUMP_SDL_TEXTURE_UPLOADS_DIR") else {
            return;
        };
        let Some(matcher) = std::env::var_os("AEMU_DUMP_SDL_TEXTURE_UPLOADS_MATCH") else {
            return;
        };
        let matcher = matcher.to_string_lossy();
        if !texture_upload_matches(
            &matcher,
            TextureUploadMatch {
                kind: Some(kind),
                texture,
                width,
                height,
                format,
                ty,
            },
        ) {
            return;
        }
        let limit = env_usize("AEMU_DUMP_SDL_TEXTURE_UPLOADS_LIMIT").unwrap_or(usize::MAX);
        if self.replay.texture_upload_dump_index >= limit {
            return;
        }
        let Some(rgb) = texture_payload_to_rgb(width, height, format, ty, payload) else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        if let Err(err) = fs::create_dir_all(&dir) {
            eprintln!("SDL texture-upload dump create dir failed: {err}");
            return;
        }
        let index = self.replay.texture_upload_dump_index;
        self.replay.texture_upload_dump_index += 1;
        let stem =
            format!("{index:04}-{kind}-tex{texture}-{width}x{height}-fmt{format:04x}-ty{ty:04x}");
        let raw_path = dir.join(format!("{stem}.raw"));
        if let Err(err) = fs::write(&raw_path, payload) {
            eprintln!("SDL texture-upload raw dump failed {:?}: {err}", raw_path);
        }
        let png = match encode_rgb_png(width as u32, height as u32, &rgb) {
            Ok(png) => png,
            Err(err) => {
                eprintln!("SDL texture-upload png encode failed {stem}: {err}");
                return;
            }
        };
        let png_path = dir.join(format!("{stem}.png"));
        if let Err(err) = fs::write(&png_path, png) {
            eprintln!("SDL texture-upload png dump failed {:?}: {err}", png_path);
        } else {
            eprintln!(
                "SDL texture-upload dumped {:?} tex={} {}x{} fmt=0x{format:04x} type=0x{ty:04x} bytes={}",
                png_path,
                texture,
                width,
                height,
                payload.len()
            );
        }
    }

    fn host_framebuffer(&mut self, guest: u32) -> u32 {
        if guest == 0 {
            return 0;
        }
        if let Some(host) = self.replay.framebuffers.get(&guest).copied() {
            return host;
        }
        let mut host = 0;
        unsafe {
            (self.gl.gen_framebuffers)(1, &mut host);
        }
        self.replay.framebuffers.insert(guest, host);
        host
    }

    fn host_renderbuffer(&mut self, guest: u32) -> u32 {
        if guest == 0 {
            return 0;
        }
        if let Some(host) = self.replay.renderbuffers.get(&guest).copied() {
            return host;
        }
        let mut host = 0;
        unsafe {
            (self.gl.gen_renderbuffers)(1, &mut host);
        }
        self.replay.renderbuffers.insert(guest, host);
        host
    }

    fn host_program(&self, guest: u32) -> Option<u32> {
        if guest == 0 {
            Some(0)
        } else {
            self.replay.programs.get(&guest).map(|program| program.host)
        }
    }

    fn host_uniform_location(&self, guest_location: i32) -> i32 {
        if guest_location < 0 {
            return -1;
        }
        let guest_location = guest_location as u32;
        let Some(program) = self.replay.programs.get(&self.replay.current_program) else {
            return -1;
        };
        program.uniforms.get(&guest_location).copied().unwrap_or(-1)
    }

    fn set_stencil_func(&mut self, face: u32, func: u32, reference: i32, mask: u32) {
        match face {
            GL_FRONT => self.replay.stencil_front_func = (func, reference, mask),
            GL_BACK => self.replay.stencil_back_func = (func, reference, mask),
            GL_FRONT_AND_BACK => {
                self.replay.stencil_front_func = (func, reference, mask);
                self.replay.stencil_back_func = (func, reference, mask);
            }
            _ => {}
        }
    }

    fn set_stencil_op(&mut self, face: u32, sfail: u32, dpfail: u32, dppass: u32) {
        match face {
            GL_FRONT => self.replay.stencil_front_op = (sfail, dpfail, dppass),
            GL_BACK => self.replay.stencil_back_op = (sfail, dpfail, dppass),
            GL_FRONT_AND_BACK => {
                self.replay.stencil_front_op = (sfail, dpfail, dppass);
                self.replay.stencil_back_op = (sfail, dpfail, dppass);
            }
            _ => {}
        }
    }

    fn describe_draw_event(&self, event: &GlesEvent) -> String {
        format!(
            "program=\"{}\" state=\"{}\" texture=\"{}\" geometry=\"{}\"",
            self.describe_current_program(),
            self.describe_draw_state(),
            self.describe_bound_texture(),
            self.describe_draw_geometry(event)
        )
    }

    fn describe_current_program(&self) -> String {
        let Some(program) = self.replay.programs.get(&self.replay.current_program) else {
            return "none".to_string();
        };
        let mut attrs = program
            .attributes
            .iter()
            .map(|attr| format!("{}:{}:0x{:04x}", attr.location, attr.name, attr.ty))
            .collect::<Vec<_>>();
        attrs.sort();
        let mut uniforms = program
            .uniform_names
            .iter()
            .map(|(location, name)| format!("{location}:{name}"))
            .collect::<Vec<_>>();
        uniforms.sort();
        format!(
            "attrs=[{}] uniforms=[{}]",
            attrs.join(","),
            uniforms.join(",")
        )
    }

    fn describe_draw_state(&self) -> String {
        let caps = [
            GL_BLEND,
            GL_SCISSOR_TEST,
            GL_DEPTH_TEST,
            GL_STENCIL_TEST,
            GL_CULL_FACE,
        ]
        .into_iter()
        .filter(|cap| self.cap_enabled(*cap))
        .map(gl_cap_name)
        .collect::<Vec<_>>();
        let caps = if caps.is_empty() {
            "none".to_string()
        } else {
            caps.join("|")
        };
        let (src_rgb, dst_rgb, src_alpha, dst_alpha) = self.replay.blend_func;
        let (red, green, blue, alpha) = self.replay.color_mask;
        let (sx, sy, sw, sh) = self.replay.scissor;
        let (front_func, front_ref, front_mask) = self.replay.stencil_front_func;
        let (back_func, back_ref, back_mask) = self.replay.stencil_back_func;
        let (front_sfail, front_dpfail, front_dppass) = self.replay.stencil_front_op;
        let (back_sfail, back_dpfail, back_dppass) = self.replay.stencil_back_op;
        format!(
            "caps={caps} blend=0x{src_rgb:04x}/0x{dst_rgb:04x}/0x{src_alpha:04x}/0x{dst_alpha:04x} depth_func=0x{:04x} depth_mask={} color_mask={}{}{}{} scissor={sx},{sy},{sw},{sh} stencil_front=0x{front_func:04x}/{front_ref}/0x{front_mask:08x}/0x{front_sfail:04x},0x{front_dpfail:04x},0x{front_dppass:04x} stencil_back=0x{back_func:04x}/{back_ref}/0x{back_mask:08x}/0x{back_sfail:04x},0x{back_dpfail:04x},0x{back_dppass:04x} stencil_mask=0x{:08x}",
            self.replay.depth_func,
            bool_digit(self.replay.depth_mask),
            bool_digit(red),
            bool_digit(green),
            bool_digit(blue),
            bool_digit(alpha),
            self.replay.stencil_mask
        )
    }

    fn describe_bound_texture(&self) -> String {
        let texture = self
            .replay
            .bound_textures
            .get(&(self.replay.active_texture, GL_TEXTURE_2D))
            .copied()
            .unwrap_or(0);
        let Some(info) = self.replay.texture_info.get(&texture) else {
            return format!("tex2d={texture} info=none");
        };
        let rgb = info
            .last_nonzero_rgb_pixels
            .map_or_else(|| "?".to_string(), |value| value.to_string());
        let alpha = info
            .last_nonzero_alpha_pixels
            .map_or_else(|| "?".to_string(), |value| value.to_string());
        format!(
            "tex2d={texture} size={}x{} fmt=0x{:04x} type=0x{:04x} last_upload={}x{} len={} nz_rgb={} nz_alpha={}",
            info.width,
            info.height,
            info.format,
            info.ty,
            info.last_upload_width,
            info.last_upload_height,
            info.last_payload_len,
            rgb,
            alpha
        )
    }

    fn describe_draw_geometry(&self, event: &GlesEvent) -> String {
        let vertex_indices = self.draw_vertex_indices(event);
        let index_text = if vertex_indices.is_empty() {
            "indices=none".to_string()
        } else {
            let min = vertex_indices.iter().copied().min().unwrap_or(0);
            let max = vertex_indices.iter().copied().max().unwrap_or(0);
            format!("indices={} min={} max={}", vertex_indices.len(), min, max)
        };
        let mut attribs = self
            .replay
            .enabled_vertex_attribs
            .iter()
            .filter_map(|(index, enabled)| enabled.then_some(*index))
            .collect::<Vec<_>>();
        attribs.sort_unstable();
        let attrib_text = attribs
            .into_iter()
            .take(8)
            .map(|index| self.describe_vertex_attrib(index, event, &vertex_indices))
            .collect::<Vec<_>>()
            .join(";");
        if attrib_text.is_empty() {
            format!("{index_text} attrs=none")
        } else {
            format!("{index_text} attrs={attrib_text}")
        }
    }

    fn draw_vertex_indices(&self, event: &GlesEvent) -> Vec<u32> {
        match event {
            GlesEvent::DrawArrays { first, count, .. } => {
                let Some(count) = usize::try_from((*count).max(0)).ok() else {
                    return Vec::new();
                };
                let first = (*first).max(0) as u32;
                (0..count.min(2048))
                    .map(|idx| first.wrapping_add(idx as u32))
                    .collect()
            }
            GlesEvent::DrawElements {
                count,
                ty,
                indices,
                index_payload,
                ..
            } => {
                let Some(index_size) = gl_index_size(*ty) else {
                    return Vec::new();
                };
                let Some(count) = usize::try_from((*count).max(0)).ok() else {
                    return Vec::new();
                };
                let from_bound_buffer = self.replay.bound_element_array_buffer != 0;
                let source = if from_bound_buffer {
                    self.replay
                        .buffer_data
                        .get(&self.replay.bound_element_array_buffer)
                        .and_then(|buffer| buffer.get((*indices as usize)..))
                } else {
                    index_payload.as_deref()
                };
                let Some(source) = source else {
                    return Vec::new();
                };
                let count = count.min(2048).min(source.len() / index_size);
                (0..count)
                    .filter_map(|idx| read_index(source, idx * index_size, *ty))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn describe_vertex_attrib(
        &self,
        index: u32,
        event: &GlesEvent,
        vertex_indices: &[u32],
    ) -> String {
        let Some(attrib) = self.replay.vertex_attribs.get(&index).copied() else {
            return format!("{index}:missing");
        };
        let component_size = gl_component_size(attrib.ty).unwrap_or(0);
        if attrib.size <= 0 || component_size == 0 {
            return format!(
                "{index}:ptr={} type=0x{:04x} size={} norm={} unsupported",
                attrib.pointer,
                attrib.ty,
                attrib.size,
                bool_digit(attrib.normalized)
            );
        }
        let stride = if attrib.stride > 0 {
            attrib.stride as usize
        } else {
            attrib.size as usize * component_size
        };
        let (source_name, base, source) = if attrib.buffer == 0 {
            let payload = event_client_attrib_payload(event, index);
            ("client".to_string(), 0usize, payload)
        } else {
            (
                format!("buf{}", attrib.buffer),
                attrib.pointer as usize,
                self.replay
                    .buffer_data
                    .get(&attrib.buffer)
                    .map(Vec::as_slice),
            )
        };
        let Some(source) = source else {
            return format!(
                "{index}:{} ptr={} type=0x{:04x} size={} norm={} stride={} missing",
                source_name,
                attrib.pointer,
                attrib.ty,
                attrib.size,
                bool_digit(attrib.normalized),
                stride
            );
        };
        let mut mins = vec![f32::INFINITY; attrib.size as usize];
        let mut maxs = vec![f32::NEG_INFINITY; attrib.size as usize];
        let mut seen = 0usize;
        let mut oob = false;
        let fallback_indices = [0_u32];
        let indices = if vertex_indices.is_empty() {
            &fallback_indices[..]
        } else {
            vertex_indices
        };
        for vertex in indices.iter().copied().take(2048) {
            let vertex_base = base.saturating_add(vertex as usize * stride);
            for component in 0..attrib.size as usize {
                let offset = vertex_base.saturating_add(component * component_size);
                let Some(value) =
                    read_attrib_component(source, offset, attrib.ty, attrib.normalized)
                else {
                    oob = true;
                    continue;
                };
                mins[component] = mins[component].min(value);
                maxs[component] = maxs[component].max(value);
            }
            seen += 1;
        }
        if seen == 0 || mins.iter().any(|value| !value.is_finite()) {
            return format!(
                "{index}:{} ptr={} type=0x{:04x} size={} norm={} stride={} empty{}",
                source_name,
                attrib.pointer,
                attrib.ty,
                attrib.size,
                bool_digit(attrib.normalized),
                stride,
                if oob { " oob" } else { "" }
            );
        }
        format!(
            "{index}:{} ptr={} type=0x{:04x} size={} norm={} stride={} min={} max={}{}",
            source_name,
            attrib.pointer,
            attrib.ty,
            attrib.size,
            bool_digit(attrib.normalized),
            stride,
            format_f32_list(&mins),
            format_f32_list(&maxs),
            if oob { " oob" } else { "" }
        )
    }

    fn cap_enabled(&self, cap: u32) -> bool {
        self.replay.enabled_caps.get(&cap).copied().unwrap_or(false)
    }

    fn lookup_host_uniform(&self, host_program: u32, guest_name: &str) -> Option<i32> {
        for name in uniform_lookup_names(guest_name) {
            let Ok(name) = CString::new(name) else {
                continue;
            };
            let location = unsafe { (self.gl.get_uniform_location)(host_program, name.as_ptr()) };
            if location >= 0 {
                return Some(location);
            }
        }
        None
    }

    fn capture_framebuffer_readback(&mut self) -> HostResult<()> {
        let (width, height, pixels) = self.read_framebuffer_rgba()?;
        let mut nonzero_rgb = 0usize;
        let mut nonzero_alpha = 0usize;
        for pixel in pixels.chunks_exact(4) {
            if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                nonzero_rgb += 1;
            }
            if pixel[3] != 0 {
                nonzero_alpha += 1;
            }
        }
        self.replay.stats.readback_width = width;
        self.replay.stats.readback_height = height;
        self.replay.stats.readback_nonzero_rgb_pixels = nonzero_rgb;
        self.replay.stats.readback_nonzero_alpha_pixels = nonzero_alpha;
        Ok(())
    }

    fn read_framebuffer_rgba(&self) -> HostResult<(u32, u32, Vec<u8>)> {
        let (width, height) = self.window.drawable_size();
        let Some(byte_len) = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return Err(HostError::new(format!(
                "framebuffer readback dimensions too large: {width}x{height}"
            )));
        };
        let mut pixels = vec![0_u8; byte_len];
        unsafe {
            (self.gl.read_pixels)(
                0,
                0,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr().cast(),
            );
        }
        Ok((width, height, pixels))
    }

    fn record_gl_errors(&mut self, event_index: usize, event_kind: &'static str) {
        loop {
            let error = unsafe { (self.gl.get_error)() };
            if error == GL_NO_ERROR {
                break;
            }
            if self.replay.stats.gl_error_count == 0 {
                self.replay.stats.first_gl_error_event_index = event_index;
                self.replay.stats.first_gl_error_event_kind = Some(event_kind);
                self.replay.stats.first_gl_error_code = error;
            }
            self.replay.stats.gl_error_count += 1;
        }
    }

    fn shader_info_log(&self, shader: u32) -> String {
        let mut len = 0;
        unsafe {
            (self.gl.get_shader_iv)(shader, GL_INFO_LOG_LENGTH, &mut len);
        }
        let mut log = vec![0_u8; usize::try_from(len.max(1)).unwrap_or(1)];
        let mut written = 0;
        unsafe {
            (self.gl.get_shader_info_log)(
                shader,
                len,
                &mut written,
                log.as_mut_ptr().cast::<c_char>(),
            );
        }
        gl_log_string(&log, written)
    }

    fn program_info_log(&self, program: u32) -> String {
        let mut len = 0;
        unsafe {
            (self.gl.get_program_iv)(program, GL_INFO_LOG_LENGTH, &mut len);
        }
        let mut log = vec![0_u8; usize::try_from(len.max(1)).unwrap_or(1)];
        let mut written = 0;
        unsafe {
            (self.gl.get_program_info_log)(
                program,
                len,
                &mut written,
                log.as_mut_ptr().cast::<c_char>(),
            );
        }
        gl_log_string(&log, written)
    }

    fn has_unbacked_client_attrib(&self) -> bool {
        self.replay
            .enabled_vertex_attribs
            .iter()
            .any(|(index, enabled)| {
                *enabled
                    && self
                        .replay
                        .client_side_vertex_attribs
                        .get(index)
                        .copied()
                        .unwrap_or(false)
            })
    }

    fn draw_indices_ptr(&self, indices: u32, payload: Option<&[u8]>) -> Option<*const c_void> {
        if self.replay.bound_element_array_buffer != 0 {
            return Some(offset_ptr(indices));
        }
        payload.map(|payload| payload.as_ptr().cast())
    }
}

fn gl_size(value: u32, label: &str) -> HostResult<isize> {
    isize::try_from(value).map_err(|_| HostError::new(format!("{label} too large: {value}")))
}

fn payload_ptr(payload: Option<&[u8]>, size: isize) -> *const c_void {
    payload_bytes(payload, size).map_or(ptr::null(), |payload| payload.as_ptr().cast())
}

fn payload_bytes(payload: Option<&[u8]>, size: isize) -> Option<&[u8]> {
    let size = usize::try_from(size).ok()?;
    if size == 0 {
        Some(&[])
    } else {
        payload.filter(|payload| payload.len() >= size)
    }
}

fn offset_ptr(offset: u32) -> *const c_void {
    offset as usize as *const c_void
}

fn gl_bool(value: bool) -> u8 {
    if value { GL_TRUE } else { GL_FALSE }
}

fn uniform_lookup_names(name: &str) -> [&str; 2] {
    [name, name.strip_suffix("[0]").unwrap_or(name)]
}

fn gl_log_string(log: &[u8], written: i32) -> String {
    let len = usize::try_from(written).unwrap_or(0).min(log.len());
    let log = &log[..len];
    String::from_utf8_lossy(log)
        .trim_end_matches('\0')
        .to_string()
}

impl HostBackend for Sdl2Host {
    fn poll_events(&mut self) -> HostResult<Vec<HostEvent>> {
        let mut events = Vec::new();
        let (width, height) = self.window.drawable_size();

        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => events.push(HostEvent::Quit),
                Event::KeyDown {
                    keycode: Some(key),
                    repeat,
                    ..
                } => events.push(HostEvent::Key {
                    key: map_key(key),
                    pressed: true,
                    repeat,
                }),
                Event::KeyUp {
                    keycode: Some(key), ..
                } => events.push(HostEvent::Key {
                    key: map_key(key),
                    pressed: false,
                    repeat: false,
                }),
                Event::Window {
                    win_event:
                        WindowEvent::Resized(width, height) | WindowEvent::SizeChanged(width, height),
                    ..
                } => events.push(HostEvent::Resize {
                    width: width.max(0) as u32,
                    height: height.max(0) as u32,
                }),
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => events.push(mouse_pointer_event(mouse_btn, PointerPhase::Down, x, y)),
                Event::MouseButtonUp {
                    mouse_btn, x, y, ..
                } => events.push(mouse_pointer_event(mouse_btn, PointerPhase::Up, x, y)),
                Event::MouseMotion {
                    x, y, mousestate, ..
                } if mousestate.left() => {
                    events.push(HostEvent::Pointer {
                        id: 0,
                        phase: PointerPhase::Move,
                        x: x as f32,
                        y: y as f32,
                        pressure: 1.0,
                    });
                }
                Event::FingerDown {
                    finger_id,
                    x,
                    y,
                    pressure,
                    ..
                } => events.push(touch_pointer_event(
                    finger_id,
                    PointerPhase::Down,
                    x,
                    y,
                    pressure,
                    width,
                    height,
                )),
                Event::FingerUp {
                    finger_id,
                    x,
                    y,
                    pressure,
                    ..
                } => events.push(touch_pointer_event(
                    finger_id,
                    PointerPhase::Up,
                    x,
                    y,
                    pressure,
                    width,
                    height,
                )),
                Event::FingerMotion {
                    finger_id,
                    x,
                    y,
                    pressure,
                    ..
                } => events.push(touch_pointer_event(
                    finger_id,
                    PointerPhase::Move,
                    x,
                    y,
                    pressure,
                    width,
                    height,
                )),
                _ => {}
            }
        }

        Ok(events)
    }

    fn replay_gles_events(&mut self, events: &[GlesEvent]) -> HostResult<()> {
        unsafe {
            (self.gl.active_texture)(GL_TEXTURE0);
            (self.gl.pixel_storei)(GL_UNPACK_ALIGNMENT, 1);
        }
        self.record_gl_errors(usize::MAX, "replay-init");
        let mut draw_trace = SdlDrawChangeTrace::from_env(self)?;
        for (index, event) in events.iter().enumerate() {
            let before_draw_arrays = self.replay.stats.draw_arrays;
            let before_draw_elements = self.replay.stats.draw_elements;
            self.replay_gles_event(event)?;
            draw_trace.after_event(self, index, event, before_draw_arrays, before_draw_elements)?;
            self.record_gl_errors(index, event.kind());
        }
        Ok(())
    }

    fn swap_buffers(&mut self) -> HostResult<()> {
        self.window.gl_swap_window();
        Ok(())
    }
}

pub fn run_debug_shell(config: HostConfig, max_frames: Option<u64>) -> HostResult<()> {
    let mut host = Sdl2Host::new(&config)?;
    let mut frames = 0_u64;
    let frame_time = Duration::from_millis(16);

    loop {
        let start = Instant::now();
        for event in host.poll_events()? {
            match event {
                HostEvent::Quit
                | HostEvent::Key {
                    key: HostKey::Escape,
                    pressed: true,
                    ..
                } => return Ok(()),
                _ => {}
            }
        }

        host.swap_buffers()?;
        frames += 1;
        if max_frames.is_some_and(|limit| frames >= limit) {
            return Ok(());
        }

        if let Some(remaining) = frame_time.checked_sub(start.elapsed()) {
            thread::sleep(remaining);
        }
    }
}

fn update_buffer_data(
    buffers: &mut HashMap<u32, Vec<u8>>,
    guest: u32,
    offset: usize,
    size: usize,
    payload: &[u8],
) {
    let Some(end) = offset.checked_add(size) else {
        return;
    };
    let entry = buffers.entry(guest).or_default();
    if entry.len() < end {
        entry.resize(end, 0);
    }
    let copy_len = size.min(payload.len());
    entry[offset..offset + copy_len].copy_from_slice(&payload[..copy_len]);
}

fn gl_index_size(ty: u32) -> Option<usize> {
    match ty {
        GL_UNSIGNED_BYTE => Some(1),
        GL_UNSIGNED_SHORT => Some(2),
        GL_UNSIGNED_INT => Some(4),
        _ => None,
    }
}

fn read_index(bytes: &[u8], offset: usize, ty: u32) -> Option<u32> {
    match ty {
        GL_UNSIGNED_BYTE => bytes.get(offset).copied().map(u32::from),
        GL_UNSIGNED_SHORT => bytes
            .get(offset..offset + 2)
            .map(|bytes| u32::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
        GL_UNSIGNED_INT => bytes
            .get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => None,
    }
}

fn gl_component_size(ty: u32) -> Option<usize> {
    match ty {
        GL_BYTE | GL_UNSIGNED_BYTE => Some(1),
        GL_SHORT | GL_UNSIGNED_SHORT => Some(2),
        GL_INT | GL_UNSIGNED_INT | GL_FLOAT | GL_FIXED => Some(4),
        _ => None,
    }
}

fn read_attrib_component(bytes: &[u8], offset: usize, ty: u32, normalized: bool) -> Option<f32> {
    match ty {
        GL_BYTE => {
            let value = *bytes.get(offset)? as i8;
            Some(if normalized {
                (f32::from(value) / 127.0).max(-1.0)
            } else {
                f32::from(value)
            })
        }
        GL_UNSIGNED_BYTE => {
            let value = *bytes.get(offset)?;
            Some(if normalized {
                f32::from(value) / 255.0
            } else {
                f32::from(value)
            })
        }
        GL_SHORT => {
            let bytes = bytes.get(offset..offset + 2)?;
            let value = i16::from_le_bytes([bytes[0], bytes[1]]);
            Some(if normalized {
                (f32::from(value) / 32767.0).max(-1.0)
            } else {
                f32::from(value)
            })
        }
        GL_UNSIGNED_SHORT => {
            let bytes = bytes.get(offset..offset + 2)?;
            let value = u16::from_le_bytes([bytes[0], bytes[1]]);
            Some(if normalized {
                f32::from(value) / 65535.0
            } else {
                f32::from(value)
            })
        }
        GL_INT => {
            let bytes = bytes.get(offset..offset + 4)?;
            let value = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Some(value as f32)
        }
        GL_UNSIGNED_INT => {
            let bytes = bytes.get(offset..offset + 4)?;
            let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Some(value as f32)
        }
        GL_FLOAT => {
            let bytes = bytes.get(offset..offset + 4)?;
            Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        GL_FIXED => {
            let bytes = bytes.get(offset..offset + 4)?;
            let value = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Some(value as f32 / 65536.0)
        }
        _ => None,
    }
}

fn event_client_attrib_payload(event: &GlesEvent, index: u32) -> Option<&[u8]> {
    let client_attribs = match event {
        GlesEvent::DrawArrays { client_attribs, .. }
        | GlesEvent::DrawElements { client_attribs, .. } => client_attribs,
        _ => return None,
    };
    client_attribs
        .iter()
        .find(|payload| payload.index == index)
        .and_then(|payload| payload.payload.as_deref())
}

fn format_f32_list(values: &[f32]) -> String {
    let items = values
        .iter()
        .map(|value| {
            if value.abs() >= 1000.0 || value.fract().abs() < 0.0005 {
                format!("{value:.0}")
            } else {
                format!("{value:.3}")
            }
        })
        .collect::<Vec<_>>();
    format!("[{}]", items.join(","))
}

fn bool_digit(value: bool) -> char {
    if value { '1' } else { '0' }
}

fn gl_cap_name(cap: u32) -> &'static str {
    match cap {
        GL_BLEND => "blend",
        GL_SCISSOR_TEST => "scissor",
        GL_DEPTH_TEST => "depth",
        GL_STENCIL_TEST => "stencil",
        GL_CULL_FACE => "cull",
        _ => "unknown",
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn framebuffer_delta(previous: &[u8], current: &[u8]) -> (usize, usize) {
    let mut changed_bytes = 0usize;
    let mut changed_pixels = 0usize;
    for (previous, current) in previous.chunks_exact(4).zip(current.chunks_exact(4)) {
        let mut pixel_changed = false;
        for (old, new) in previous.iter().zip(current) {
            if old != new {
                changed_bytes += 1;
                pixel_changed = true;
            }
        }
        if pixel_changed {
            changed_pixels += 1;
        }
    }
    (changed_pixels, changed_bytes)
}

fn map_key(key: Keycode) -> HostKey {
    match key {
        Keycode::Escape => HostKey::Escape,
        Keycode::Return => HostKey::Enter,
        Keycode::Space => HostKey::Space,
        Keycode::Backspace => HostKey::Backspace,
        Keycode::Tab => HostKey::Tab,
        Keycode::Up => HostKey::ArrowUp,
        Keycode::Down => HostKey::ArrowDown,
        Keycode::Left => HostKey::ArrowLeft,
        Keycode::Right => HostKey::ArrowRight,
        _ => HostKey::Other(format!("{key:?}")),
    }
}

fn mouse_pointer_event(button: MouseButton, phase: PointerPhase, x: i32, y: i32) -> HostEvent {
    HostEvent::Pointer {
        id: mouse_button_id(button),
        phase,
        x: x as f32,
        y: y as f32,
        pressure: 1.0,
    }
}

fn mouse_button_id(button: MouseButton) -> i64 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
        MouseButton::Unknown => -1,
    }
}

fn touch_pointer_event(
    finger_id: i64,
    phase: PointerPhase,
    x: f32,
    y: f32,
    pressure: f32,
    width: u32,
    height: u32,
) -> HostEvent {
    HostEvent::Pointer {
        id: finger_id,
        phase,
        x: x * width as f32,
        y: y * height as f32,
        pressure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_change_matcher_accepts_event_draw_program_and_texture_tokens() {
        let draw = DrawChangeMatch {
            event_index: 1671,
            draw: 42,
            kind: "DrawElements",
            program: 86,
            texture: 325,
        };

        assert!(sdl_draw_change_matches("DrawElements", draw));
        assert!(sdl_draw_change_matches("event1671", draw));
        assert!(sdl_draw_change_matches("draw42", draw));
        assert!(sdl_draw_change_matches("program86", draw));
        assert!(sdl_draw_change_matches("prog86", draw));
        assert!(sdl_draw_change_matches("tex325", draw));
        assert!(sdl_draw_change_matches("0x145", draw));
        assert!(!sdl_draw_change_matches("event1670,program79,tex324", draw));
    }

    #[test]
    fn framebuffer_rgb_conversion_flips_gl_bottom_origin() {
        let rgba = [
            0x10, 0, 0, 0xff, 0x20, 0, 0, 0xff, 0, 0x30, 0, 0xff, 0, 0x40, 0, 0xff,
        ];

        let rgb = framebuffer_rgba_to_top_down_rgb(2, 2, &rgba).unwrap();

        assert_eq!(rgb, vec![0, 0x30, 0, 0, 0x40, 0, 0x10, 0, 0, 0x20, 0, 0]);
    }
}
