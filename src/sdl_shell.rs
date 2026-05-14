use crate::hle_imports::{GlesActive, GlesClientAttribPayload, GlesEvent};
use crate::host::{
    HostBackend, HostConfig, HostError, HostEvent, HostKey, HostResult, PointerPhase,
};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::video::GLProfile;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::thread;
use std::time::{Duration, Instant};

const GL_FALSE: u8 = 0;
const GL_TRUE: u8 = 1;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const GL_STATIC_DRAW: u32 = 0x88e4;
const GL_TEXTURE0: u32 = 0x84c0;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
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
type GlDepthFunc = unsafe extern "C" fn(u32);
type GlDepthMask = unsafe extern "C" fn(u8);
type GlDepthRangef = unsafe extern "C" fn(f32, f32);
type GlColorMask = unsafe extern "C" fn(u8, u8, u8, u8);
type GlScissor = unsafe extern "C" fn(i32, i32, i32, i32);
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
    depth_func: GlDepthFunc,
    depth_mask: GlDepthMask,
    depth_rangef: GlDepthRangef,
    color_mask: GlColorMask,
    scissor: GlScissor,
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
    textures: HashMap<u32, u32>,
    shaders: HashMap<u32, u32>,
    programs: HashMap<u32, ReplayProgram>,
    enabled_vertex_attribs: HashMap<u32, bool>,
    client_side_vertex_attribs: HashMap<u32, bool>,
    client_attrib_buffers: HashMap<u32, u32>,
    vertex_attribs: HashMap<u32, ReplayVertexAttrib>,
    current_program: u32,
    bound_array_buffer: u32,
    bound_element_array_buffer: u32,
    stats: SdlGlesReplayStats,
}

#[derive(Debug)]
struct ReplayProgram {
    host: u32,
    uniforms: HashMap<u32, i32>,
}

#[derive(Debug, Clone, Copy)]
struct ReplayVertexAttrib {
    size: i32,
    ty: u32,
    normalized: bool,
    stride: i32,
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

        Ok(Self {
            _sdl: sdl,
            _video: video,
            window,
            _gl_context: gl_context,
            event_pump,
            gl,
            replay: SdlGlesReplay::default(),
        })
    }

    pub fn replay_stats(&self) -> SdlGlesReplayStats {
        self.replay.stats
    }

    pub fn capture_framebuffer_rgb(&self) -> HostResult<SdlFramebufferCapture> {
        let (width, height, rgba) = self.read_framebuffer_rgba()?;
        let width_usize = width as usize;
        let height_usize = height as usize;
        let mut rgb = Vec::with_capacity(width_usize * height_usize * 3);
        for y in (0..height_usize).rev() {
            let row = y * width_usize * 4;
            for x in 0..width_usize {
                let pixel = row + x * 4;
                rgb.extend_from_slice(&rgba[pixel..pixel + 3]);
            }
        }
        Ok(SdlFramebufferCapture { width, height, rgb })
    }
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
            depth_func: load_required_gl(video, "glDepthFunc")?,
            depth_mask: load_required_gl(video, "glDepthMask")?,
            depth_rangef: load_required_gl(video, "glDepthRangef")?,
            color_mask: load_required_gl(video, "glColorMask")?,
            scissor: load_required_gl(video, "glScissor")?,
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
            GlesEvent::ActiveTexture { texture } => unsafe {
                (self.gl.active_texture)(*texture);
            },
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
                }
            }
            GlesEvent::BindTexture { target, texture } => {
                let host = self.host_texture(*texture);
                unsafe {
                    (self.gl.bind_texture)(*target, host);
                }
            }
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
                let data = payload
                    .as_ref()
                    .map_or(ptr::null(), |payload| payload.as_ptr().cast::<c_void>());
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
                if let Some(payload) = payload {
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
            GlesEvent::Enable { cap } => unsafe {
                (self.gl.enable)(*cap);
            },
            GlesEvent::Disable { cap } => unsafe {
                (self.gl.disable)(*cap);
            },
            GlesEvent::BlendFunc { sfactor, dfactor } => unsafe {
                (self.gl.blend_func)(*sfactor, *dfactor);
            },
            GlesEvent::BlendFuncSeparate {
                src_rgb,
                dst_rgb,
                src_alpha,
                dst_alpha,
            } => unsafe {
                (self.gl.blend_func_separate)(*src_rgb, *dst_rgb, *src_alpha, *dst_alpha);
            },
            GlesEvent::DepthFunc { func } => unsafe {
                (self.gl.depth_func)(*func);
            },
            GlesEvent::DepthMask { enabled } => unsafe {
                (self.gl.depth_mask)(gl_bool(*enabled));
            },
            GlesEvent::DepthRangef { near, far } => unsafe {
                (self.gl.depth_rangef)(f32::from_bits(*near), f32::from_bits(*far));
            },
            GlesEvent::ColorMask {
                red,
                green,
                blue,
                alpha,
            } => unsafe {
                (self.gl.color_mask)(
                    gl_bool(*red),
                    gl_bool(*green),
                    gl_bool(*blue),
                    gl_bool(*alpha),
                );
            },
            GlesEvent::Scissor {
                x,
                y,
                width,
                height,
            } => unsafe {
                (self.gl.scissor)(*x, *y, *width, *height);
            },
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
            GlesEvent::Clear { mask } => unsafe {
                (self.gl.clear)(*mask);
            },
            GlesEvent::Viewport {
                x,
                y,
                width,
                height,
            } => unsafe {
                (self.gl.viewport)(*x, *y, *width, *height);
            },
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
        for uniform in uniforms {
            if let Some(location) = self.lookup_host_uniform(host_program, &uniform.name) {
                host_uniforms.insert(uniform.location, location);
            }
        }
        if let Some(program) = self.replay.programs.get_mut(&guest_program) {
            program.uniforms = host_uniforms;
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
        for (index, event) in events.iter().enumerate() {
            self.replay_gles_event(event)?;
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
