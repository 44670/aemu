#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
use std::collections::HashMap;

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
use crate::hle_imports::{GlesActive, GlesClientAttribPayload, GlesEvent};
use crate::host::GraphicsApi;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
use crate::host::{HostBackend, HostError, HostEvent, HostResult};

pub const DEFAULT_CANVAS_ID: &str = "aemu-canvas";

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_ARRAY_BUFFER: u32 = 0x8892;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_STATIC_DRAW: u32 = 0x88e4;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_TEXTURE0: u32 = 0x84c0;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_RGBA: u32 = 0x1908;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_UNSIGNED_BYTE: u32 = 0x1401;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_NO_ERROR: u32 = 0;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_COMPILE_STATUS: u32 = 0x8b81;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_LINK_STATUS: u32 = 0x8b82;
#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
const GL_UNPACK_ALIGNMENT: u32 = 0x0cf5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebGlTarget {
    pub canvas_id: String,
    pub api: GraphicsApi,
}

impl WebGlTarget {
    pub fn new(canvas_id: impl Into<String>, api: GraphicsApi) -> Self {
        Self {
            canvas_id: canvas_id.into(),
            api,
        }
    }

    pub fn default_webgl1() -> Self {
        Self::new(DEFAULT_CANVAS_ID, GraphicsApi::WebGl1)
    }

    pub fn context_name(&self) -> &'static str {
        webgl_context_name(self.api)
    }
}

pub fn webgl_context_name(api: GraphicsApi) -> &'static str {
    match api {
        GraphicsApi::Gles2 | GraphicsApi::WebGl1 => "webgl",
        GraphicsApi::WebGl2 => "webgl2",
    }
}

pub fn browser_backend_note(api: GraphicsApi) -> &'static str {
    match api {
        GraphicsApi::Gles2 | GraphicsApi::WebGl1 => {
            "GLES2 guest calls should target a WebGL 1 context first"
        }
        GraphicsApi::WebGl2 => "WebGL 2 is reserved for target-proven GLES3/WebGL2 needs",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WebGlReplayStats {
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

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy)]
struct WebGlReplayVertexAttrib {
    size: i32,
    ty: u32,
    normalized: bool,
    stride: i32,
}

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
mod browser {
    use super::*;
    use js_sys::{Array, Float32Array, Function, Int32Array, Reflect, Uint8Array};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    pub struct BrowserWebGlHost {
        context: JsValue,
        width: u32,
        height: u32,
        buffers: HashMap<u32, JsValue>,
        textures: HashMap<u32, JsValue>,
        framebuffers: HashMap<u32, JsValue>,
        renderbuffers: HashMap<u32, JsValue>,
        shaders: HashMap<u32, JsValue>,
        programs: HashMap<u32, WebGlReplayProgram>,
        enabled_vertex_attribs: HashMap<u32, bool>,
        client_side_vertex_attribs: HashMap<u32, bool>,
        client_attrib_buffers: HashMap<u32, JsValue>,
        client_index_buffer: Option<JsValue>,
        vertex_attribs: HashMap<u32, WebGlReplayVertexAttrib>,
        current_program: u32,
        bound_array_buffer: u32,
        bound_element_array_buffer: u32,
        bound_renderbuffer: u32,
        stats: WebGlReplayStats,
    }

    struct WebGlReplayProgram {
        host: JsValue,
        uniforms: HashMap<u32, JsValue>,
    }

    impl BrowserWebGlHost {
        pub fn from_canvas_id(canvas_id: &str, api: GraphicsApi) -> HostResult<Self> {
            let window =
                web_sys::window().ok_or_else(|| HostError::new("browser window unavailable"))?;
            let document = window
                .document()
                .ok_or_else(|| HostError::new("browser document unavailable"))?;
            let canvas = document
                .get_element_by_id(canvas_id)
                .ok_or_else(|| HostError::new(format!("missing canvas #{canvas_id}")))?
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .map_err(|_| HostError::new(format!("#{canvas_id} is not a canvas element")))?;
            let context = canvas
                .get_context(webgl_context_name(api))
                .map_err(js_to_host_error)?
                .ok_or_else(|| {
                    HostError::new(format!("{} context unavailable", webgl_context_name(api)))
                })?;
            Self::from_context(context.into(), canvas.width(), canvas.height())
        }

        pub fn from_context(context: JsValue, width: u32, height: u32) -> HostResult<Self> {
            let host = Self {
                context,
                width,
                height,
                buffers: HashMap::new(),
                textures: HashMap::new(),
                framebuffers: HashMap::new(),
                renderbuffers: HashMap::new(),
                shaders: HashMap::new(),
                programs: HashMap::new(),
                enabled_vertex_attribs: HashMap::new(),
                client_side_vertex_attribs: HashMap::new(),
                client_attrib_buffers: HashMap::new(),
                client_index_buffer: None,
                vertex_attribs: HashMap::new(),
                current_program: 0,
                bound_array_buffer: 0,
                bound_element_array_buffer: 0,
                bound_renderbuffer: 0,
                stats: WebGlReplayStats::default(),
            };
            host.call("activeTexture", &[number(GL_TEXTURE0)])?;
            host.call("pixelStorei", &[number(GL_UNPACK_ALIGNMENT), number(1)])?;
            Ok(host)
        }

        pub fn replay_stats(&self) -> WebGlReplayStats {
            self.stats
        }

        fn replay_gles_event(&mut self, event: &GlesEvent) -> HostResult<()> {
            match event {
                GlesEvent::CreateShader {
                    shader,
                    shader_type,
                } => {
                    let host = self.call("createShader", &[number(*shader_type)])?;
                    if !host.is_null() && !host.is_undefined() {
                        self.shaders.insert(*shader, host);
                    }
                }
                GlesEvent::DeleteShader { shader } => {
                    if let Some(host) = self.shaders.remove(shader) {
                        self.call("deleteShader", &[host])?;
                    }
                }
                GlesEvent::ShaderSource { shader, source } => {
                    if let Some(host) = self.shaders.get(shader).cloned() {
                        self.compile_replay_shader(*shader, &host, source)?;
                    }
                }
                GlesEvent::DeleteTextures { textures } => {
                    for guest in textures {
                        if let Some(host) = self.textures.remove(guest) {
                            self.call("deleteTexture", &[host])?;
                        }
                    }
                }
                GlesEvent::DeleteFramebuffers { framebuffers } => {
                    for guest in framebuffers {
                        if let Some(host) = self.framebuffers.remove(guest) {
                            self.call("deleteFramebuffer", &[host])?;
                        }
                    }
                }
                GlesEvent::DeleteRenderbuffers { renderbuffers } => {
                    for guest in renderbuffers {
                        if let Some(host) = self.renderbuffers.remove(guest) {
                            self.call("deleteRenderbuffer", &[host])?;
                        }
                        if self.bound_renderbuffer == *guest {
                            self.bound_renderbuffer = 0;
                        }
                    }
                }
                GlesEvent::CreateProgram { program } => {
                    let host = self.call("createProgram", &[])?;
                    if !host.is_null() && !host.is_undefined() {
                        self.programs.insert(
                            *program,
                            WebGlReplayProgram {
                                host,
                                uniforms: HashMap::new(),
                            },
                        );
                    }
                }
                GlesEvent::DeleteProgram { program } => {
                    if let Some(state) = self.programs.remove(program) {
                        self.call("deleteProgram", &[state.host])?;
                    }
                    if self.current_program == *program {
                        self.current_program = 0;
                    }
                }
                GlesEvent::AttachShader { program, shader } => {
                    let Some(host_program) = self.host_program(*program) else {
                        return Ok(());
                    };
                    let Some(host_shader) = self.shaders.get(shader).cloned() else {
                        return Ok(());
                    };
                    self.call("attachShader", &[host_program, host_shader])?;
                }
                GlesEvent::LinkProgram {
                    program,
                    uniforms,
                    attributes,
                } => self.link_replay_program(*program, uniforms, attributes)?,
                GlesEvent::ActiveTexture { texture } => {
                    self.call("activeTexture", &[number(*texture)])?;
                }
                GlesEvent::BindBuffer { target, buffer } => {
                    let host = self.host_buffer(*buffer)?;
                    self.call("bindBuffer", &[number(*target), host])?;
                    match *target {
                        GL_ARRAY_BUFFER => self.bound_array_buffer = *buffer,
                        GL_ELEMENT_ARRAY_BUFFER => self.bound_element_array_buffer = *buffer,
                        _ => {}
                    }
                }
                GlesEvent::DeleteBuffers { buffers } => {
                    for guest in buffers {
                        if let Some(host) = self.buffers.remove(guest) {
                            self.call("deleteBuffer", &[host])?;
                        }
                        if self.bound_array_buffer == *guest {
                            self.bound_array_buffer = 0;
                        }
                        if self.bound_element_array_buffer == *guest {
                            self.bound_element_array_buffer = 0;
                        }
                    }
                }
                GlesEvent::BufferData {
                    target,
                    size,
                    usage,
                    payload,
                    ..
                } => {
                    self.rebind_guest_buffer(*target)?;
                    if let Some(payload) = payload_bytes(payload.as_deref(), *size) {
                        let data = Uint8Array::from(payload);
                        self.call(
                            "bufferData",
                            &[number(*target), data.into(), number(*usage)],
                        )?;
                    } else {
                        self.call(
                            "bufferData",
                            &[number(*target), number(*size), number(*usage)],
                        )?;
                    }
                }
                GlesEvent::BufferSubData {
                    target,
                    offset,
                    size,
                    payload,
                    ..
                } => {
                    if let Some(payload) = payload_bytes(payload.as_deref(), *size) {
                        self.rebind_guest_buffer(*target)?;
                        let data = Uint8Array::from(payload);
                        self.call(
                            "bufferSubData",
                            &[number(*target), number(*offset), data.into()],
                        )?;
                    }
                }
                GlesEvent::BindTexture { target, texture } => {
                    let host = self.host_texture(*texture)?;
                    self.call("bindTexture", &[number(*target), host])?;
                }
                GlesEvent::BindFramebuffer {
                    target,
                    framebuffer,
                } => {
                    let host = self.host_framebuffer(*framebuffer)?;
                    self.call("bindFramebuffer", &[number(*target), host])?;
                }
                GlesEvent::BindRenderbuffer {
                    target,
                    renderbuffer,
                } => {
                    let host = self.host_renderbuffer(*renderbuffer)?;
                    self.bound_renderbuffer = *renderbuffer;
                    self.call("bindRenderbuffer", &[number(*target), host])?;
                }
                GlesEvent::FramebufferTexture2D {
                    target,
                    attachment,
                    textarget,
                    texture,
                    level,
                } => {
                    let host = self.host_texture(*texture)?;
                    self.call(
                        "framebufferTexture2D",
                        &[
                            number(*target),
                            number(*attachment),
                            number(*textarget),
                            host,
                            i32_number(*level),
                        ],
                    )?;
                }
                GlesEvent::FramebufferRenderbuffer {
                    target,
                    attachment,
                    renderbuffertarget,
                    renderbuffer,
                } => {
                    let host = self.host_renderbuffer(*renderbuffer)?;
                    self.call(
                        "framebufferRenderbuffer",
                        &[
                            number(*target),
                            number(*attachment),
                            number(*renderbuffertarget),
                            host,
                        ],
                    )?;
                }
                GlesEvent::RenderbufferStorage {
                    target,
                    internal_format,
                    width,
                    height,
                } => {
                    self.call(
                        "renderbufferStorage",
                        &[
                            number(*target),
                            number(*internal_format),
                            i32_number(*width),
                            i32_number(*height),
                        ],
                    )?;
                }
                GlesEvent::TexParameteri {
                    target,
                    name,
                    value,
                } => {
                    self.call(
                        "texParameteri",
                        &[number(*target), number(*name), number(*value)],
                    )?;
                }
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
                        .map(|payload| Uint8Array::from(payload.as_slice()).into())
                        .unwrap_or(JsValue::NULL);
                    self.call(
                        "texImage2D",
                        &[
                            number(*target),
                            i32_number(*level),
                            i32_number(*internal_format),
                            i32_number(*width),
                            i32_number(*height),
                            i32_number(*border),
                            number(*format),
                            number(*ty),
                            data,
                        ],
                    )?;
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
                        let data = Uint8Array::from(payload.as_slice());
                        self.call(
                            "texSubImage2D",
                            &[
                                number(*target),
                                i32_number(*level),
                                i32_number(*xoffset),
                                i32_number(*yoffset),
                                i32_number(*width),
                                i32_number(*height),
                                number(*format),
                                number(*ty),
                                data.into(),
                            ],
                        )?;
                    }
                }
                GlesEvent::UseProgram { program } => {
                    let host = self.host_program(*program).unwrap_or(JsValue::NULL);
                    self.call("useProgram", &[host])?;
                    self.current_program = *program;
                }
                GlesEvent::Uniform1i { location, value } => {
                    if let Some(location) = self.host_uniform_location(*location) {
                        self.call("uniform1i", &[location, i32_number(*value)])?;
                    }
                }
                GlesEvent::UniformVector {
                    components,
                    integer,
                    location,
                    count,
                    payload,
                    ..
                } => {
                    self.replay_uniform_vector(*components, *integer, *location, *count, payload)?
                }
                GlesEvent::UniformMatrix {
                    columns,
                    location,
                    count,
                    transpose,
                    payload,
                    ..
                } => {
                    self.replay_uniform_matrix(*columns, *location, *count, *transpose, payload)?
                }
                GlesEvent::VertexAttribPointer {
                    index,
                    size,
                    ty,
                    normalized,
                    stride,
                    pointer,
                } => {
                    let has_array_buffer = self.bound_array_buffer != 0;
                    self.vertex_attribs.insert(
                        *index,
                        WebGlReplayVertexAttrib {
                            size: *size,
                            ty: *ty,
                            normalized: *normalized,
                            stride: *stride,
                        },
                    );
                    self.client_side_vertex_attribs
                        .insert(*index, !has_array_buffer);
                    if has_array_buffer {
                        self.rebind_guest_buffer(GL_ARRAY_BUFFER)?;
                        self.call(
                            "vertexAttribPointer",
                            &[
                                number(*index),
                                i32_number(*size),
                                number(*ty),
                                JsValue::from_bool(*normalized),
                                i32_number(*stride),
                                number(*pointer),
                            ],
                        )?;
                    }
                }
                GlesEvent::EnableVertexAttribArray { index } => {
                    self.enabled_vertex_attribs.insert(*index, true);
                    self.call("enableVertexAttribArray", &[number(*index)])?;
                }
                GlesEvent::Enable { cap } => {
                    self.call("enable", &[number(*cap)])?;
                }
                GlesEvent::Disable { cap } => {
                    self.call("disable", &[number(*cap)])?;
                }
                GlesEvent::BlendFunc { sfactor, dfactor } => {
                    self.call("blendFunc", &[number(*sfactor), number(*dfactor)])?;
                }
                GlesEvent::BlendFuncSeparate {
                    src_rgb,
                    dst_rgb,
                    src_alpha,
                    dst_alpha,
                } => {
                    self.call(
                        "blendFuncSeparate",
                        &[
                            number(*src_rgb),
                            number(*dst_rgb),
                            number(*src_alpha),
                            number(*dst_alpha),
                        ],
                    )?;
                }
                GlesEvent::StencilFuncSeparate {
                    face,
                    func,
                    reference,
                    mask,
                } => {
                    self.call(
                        "stencilFuncSeparate",
                        &[
                            number(*face),
                            number(*func),
                            i32_number(*reference),
                            number(*mask),
                        ],
                    )?;
                }
                GlesEvent::StencilOpSeparate {
                    face,
                    sfail,
                    dpfail,
                    dppass,
                } => {
                    self.call(
                        "stencilOpSeparate",
                        &[
                            number(*face),
                            number(*sfail),
                            number(*dpfail),
                            number(*dppass),
                        ],
                    )?;
                }
                GlesEvent::StencilMask { mask } => {
                    self.call("stencilMask", &[number(*mask)])?;
                }
                GlesEvent::CullFace { mode } => {
                    self.call("cullFace", &[number(*mode)])?;
                }
                GlesEvent::PolygonOffset { factor, units } => {
                    self.call(
                        "polygonOffset",
                        &[float_bits_number(*factor), float_bits_number(*units)],
                    )?;
                }
                GlesEvent::DepthFunc { func } => {
                    self.call("depthFunc", &[number(*func)])?;
                }
                GlesEvent::DepthMask { enabled } => {
                    self.call("depthMask", &[JsValue::from_bool(*enabled)])?;
                }
                GlesEvent::DepthRangef { near, far } => {
                    self.call(
                        "depthRange",
                        &[float_bits_number(*near), float_bits_number(*far)],
                    )?;
                }
                GlesEvent::ColorMask {
                    red,
                    green,
                    blue,
                    alpha,
                } => {
                    self.call(
                        "colorMask",
                        &[
                            JsValue::from_bool(*red),
                            JsValue::from_bool(*green),
                            JsValue::from_bool(*blue),
                            JsValue::from_bool(*alpha),
                        ],
                    )?;
                }
                GlesEvent::Scissor {
                    x,
                    y,
                    width,
                    height,
                } => {
                    self.call(
                        "scissor",
                        &[
                            i32_number(*x),
                            i32_number(*y),
                            i32_number(*width),
                            i32_number(*height),
                        ],
                    )?;
                }
                GlesEvent::ClearColor {
                    red,
                    green,
                    blue,
                    alpha,
                } => {
                    self.call(
                        "clearColor",
                        &[
                            float_bits_number(*red),
                            float_bits_number(*green),
                            float_bits_number(*blue),
                            float_bits_number(*alpha),
                        ],
                    )?;
                }
                GlesEvent::ClearDepthf { depth } => {
                    self.call("clearDepth", &[float_bits_number(*depth)])?;
                }
                GlesEvent::ClearStencil { value } => {
                    self.call("clearStencil", &[i32_number(*value)])?;
                }
                GlesEvent::Clear { mask } => {
                    self.call("clear", &[number(*mask)])?;
                }
                GlesEvent::Viewport {
                    x,
                    y,
                    width,
                    height,
                } => {
                    self.call(
                        "viewport",
                        &[
                            i32_number(*x),
                            i32_number(*y),
                            i32_number(*width),
                            i32_number(*height),
                        ],
                    )?;
                }
                GlesEvent::DrawArrays {
                    mode,
                    first,
                    count,
                    client_attribs,
                } => {
                    if !self.prepare_client_attribs(client_attribs)? {
                        self.stats.skipped_client_attrib_draws += 1;
                        return Ok(());
                    }
                    self.call(
                        "drawArrays",
                        &[number(*mode), i32_number(*first), i32_number(*count)],
                    )?;
                    self.stats.draw_arrays += 1;
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
                        self.stats.skipped_client_attrib_draws += 1;
                        return Ok(());
                    }
                    let Some(offset) =
                        self.prepare_draw_indices(*indices, index_payload.as_deref())?
                    else {
                        self.stats.skipped_missing_index_draws += 1;
                        return Ok(());
                    };
                    self.call(
                        "drawElements",
                        &[
                            number(*mode),
                            i32_number(*count),
                            number(*ty),
                            number(offset),
                        ],
                    )?;
                    self.stats.draw_elements += 1;
                }
                GlesEvent::Flush => {
                    self.call("flush", &[])?;
                }
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
            host_shader: &JsValue,
            source: &str,
        ) -> HostResult<()> {
            self.call(
                "shaderSource",
                &[host_shader.clone(), JsValue::from_str(source)],
            )?;
            self.call("compileShader", &[host_shader.clone()])?;
            let status = self
                .call(
                    "getShaderParameter",
                    &[host_shader.clone(), number(GL_COMPILE_STATUS)],
                )?
                .as_bool()
                .unwrap_or(false);
            if !status {
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
                self.call(
                    "bindAttribLocation",
                    &[
                        host_program.clone(),
                        number(attribute.location),
                        JsValue::from_str(&attribute.name),
                    ],
                )?;
            }
            self.call("linkProgram", &[host_program.clone()])?;
            let status = self
                .call(
                    "getProgramParameter",
                    &[host_program.clone(), number(GL_LINK_STATUS)],
                )?
                .as_bool()
                .unwrap_or(false);
            if !status {
                return Err(HostError::new(format!(
                    "program {guest_program} link failed: {}",
                    self.program_info_log(&host_program)
                )));
            }

            let mut host_uniforms = HashMap::new();
            for uniform in uniforms {
                if let Some(location) = self.lookup_host_uniform(&host_program, &uniform.name)? {
                    host_uniforms.insert(uniform.location, location);
                }
            }
            if let Some(program) = self.programs.get_mut(&guest_program) {
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
            let Some(location) = self.host_uniform_location(guest_location) else {
                return Ok(());
            };
            if count <= 0 {
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
            let method = match (integer, components) {
                (false, 1) => "uniform1fv",
                (false, 2) => "uniform2fv",
                (false, 3) => "uniform3fv",
                (false, 4) => "uniform4fv",
                (true, 1) => "uniform1iv",
                (true, 2) => "uniform2iv",
                (true, 3) => "uniform3iv",
                (true, 4) => "uniform4iv",
                _ => return Ok(()),
            };
            if integer {
                let values = i32_values(&payload[..needed]);
                let array = Int32Array::from(values.as_slice());
                self.call(method, &[location, array.into()])?;
            } else {
                let values = f32_values(&payload[..needed]);
                let array = Float32Array::from(values.as_slice());
                self.call(method, &[location, array.into()])?;
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
            let Some(location) = self.host_uniform_location(guest_location) else {
                return Ok(());
            };
            if count <= 0 {
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
            let method = match columns {
                2 => "uniformMatrix2fv",
                3 => "uniformMatrix3fv",
                4 => "uniformMatrix4fv",
                _ => return Ok(()),
            };
            let values = f32_values(&payload[..needed]);
            let array = Float32Array::from(values.as_slice());
            self.call(
                method,
                &[location, JsValue::from_bool(transpose), array.into()],
            )?;
            Ok(())
        }

        fn prepare_client_attribs(
            &mut self,
            client_attribs: &[GlesClientAttribPayload],
        ) -> HostResult<bool> {
            // Draw events contain only client arrays consumed by the linked
            // program. Ignore stale enabled arrays at inactive locations.
            for client_attrib in client_attribs {
                let index = client_attrib.index;
                if !self
                    .enabled_vertex_attribs
                    .get(&index)
                    .copied()
                    .unwrap_or(false)
                    || !self
                        .client_side_vertex_attribs
                        .get(&index)
                        .copied()
                        .unwrap_or(false)
                {
                    return Ok(false);
                }
                let Some(attrib) = self.vertex_attribs.get(&index).copied() else {
                    return Ok(false);
                };
                let Some(payload) = client_attrib.payload.as_deref() else {
                    return Ok(false);
                };
                let buffer = self.client_attrib_buffer(index)?;
                let data = Uint8Array::from(payload);
                self.call("bindBuffer", &[number(GL_ARRAY_BUFFER), buffer])?;
                self.call(
                    "bufferData",
                    &[number(GL_ARRAY_BUFFER), data.into(), number(GL_STATIC_DRAW)],
                )?;
                self.call(
                    "vertexAttribPointer",
                    &[
                        number(index),
                        i32_number(attrib.size),
                        number(attrib.ty),
                        JsValue::from_bool(attrib.normalized),
                        i32_number(attrib.stride),
                        number(0),
                    ],
                )?;
            }
            Ok(true)
        }

        fn prepare_draw_indices(
            &mut self,
            indices: u32,
            payload: Option<&[u8]>,
        ) -> HostResult<Option<u32>> {
            if self.bound_element_array_buffer != 0 {
                self.rebind_guest_buffer(GL_ELEMENT_ARRAY_BUFFER)?;
                return Ok(Some(indices));
            }
            let Some(payload) = payload else {
                return Ok(None);
            };
            let buffer = self.client_index_buffer()?;
            let data = Uint8Array::from(payload);
            self.call("bindBuffer", &[number(GL_ELEMENT_ARRAY_BUFFER), buffer])?;
            self.call(
                "bufferData",
                &[
                    number(GL_ELEMENT_ARRAY_BUFFER),
                    data.into(),
                    number(GL_STATIC_DRAW),
                ],
            )?;
            Ok(Some(0))
        }

        fn host_buffer(&mut self, guest: u32) -> HostResult<JsValue> {
            if guest == 0 {
                return Ok(JsValue::NULL);
            }
            if let Some(host) = self.buffers.get(&guest).cloned() {
                return Ok(host);
            }
            let host = self.call("createBuffer", &[])?;
            if host.is_null() || host.is_undefined() {
                return Err(HostError::new(format!(
                    "WebGL createBuffer failed for guest buffer {guest}"
                )));
            }
            self.buffers.insert(guest, host.clone());
            Ok(host)
        }

        fn host_texture(&mut self, guest: u32) -> HostResult<JsValue> {
            if guest == 0 {
                return Ok(JsValue::NULL);
            }
            if let Some(host) = self.textures.get(&guest).cloned() {
                return Ok(host);
            }
            let host = self.call("createTexture", &[])?;
            if host.is_null() || host.is_undefined() {
                return Err(HostError::new(format!(
                    "WebGL createTexture failed for guest texture {guest}"
                )));
            }
            self.textures.insert(guest, host.clone());
            Ok(host)
        }

        fn host_framebuffer(&mut self, guest: u32) -> HostResult<JsValue> {
            if guest == 0 {
                return Ok(JsValue::NULL);
            }
            if let Some(host) = self.framebuffers.get(&guest).cloned() {
                return Ok(host);
            }
            let host = self.call("createFramebuffer", &[])?;
            if host.is_null() || host.is_undefined() {
                return Err(HostError::new(format!(
                    "WebGL createFramebuffer failed for guest framebuffer {guest}"
                )));
            }
            self.framebuffers.insert(guest, host.clone());
            Ok(host)
        }

        fn host_renderbuffer(&mut self, guest: u32) -> HostResult<JsValue> {
            if guest == 0 {
                return Ok(JsValue::NULL);
            }
            if let Some(host) = self.renderbuffers.get(&guest).cloned() {
                return Ok(host);
            }
            let host = self.call("createRenderbuffer", &[])?;
            if host.is_null() || host.is_undefined() {
                return Err(HostError::new(format!(
                    "WebGL createRenderbuffer failed for guest renderbuffer {guest}"
                )));
            }
            self.renderbuffers.insert(guest, host.clone());
            Ok(host)
        }

        fn host_program(&self, guest: u32) -> Option<JsValue> {
            if guest == 0 {
                Some(JsValue::NULL)
            } else {
                self.programs
                    .get(&guest)
                    .map(|program| program.host.clone())
            }
        }

        fn host_uniform_location(&self, guest_location: i32) -> Option<JsValue> {
            if guest_location < 0 {
                return None;
            }
            let guest_location = guest_location as u32;
            self.programs
                .get(&self.current_program)
                .and_then(|program| program.uniforms.get(&guest_location).cloned())
        }

        fn lookup_host_uniform(
            &self,
            host_program: &JsValue,
            guest_name: &str,
        ) -> HostResult<Option<JsValue>> {
            for name in uniform_lookup_names(guest_name) {
                let location = self.call(
                    "getUniformLocation",
                    &[host_program.clone(), JsValue::from_str(name)],
                )?;
                if !location.is_null() && !location.is_undefined() {
                    return Ok(Some(location));
                }
            }
            Ok(None)
        }

        fn rebind_guest_buffer(&mut self, target: u32) -> HostResult<()> {
            let guest = match target {
                GL_ARRAY_BUFFER => self.bound_array_buffer,
                GL_ELEMENT_ARRAY_BUFFER => self.bound_element_array_buffer,
                _ => return Ok(()),
            };
            let host = self.host_buffer(guest)?;
            self.call("bindBuffer", &[number(target), host])?;
            Ok(())
        }

        fn client_attrib_buffer(&mut self, index: u32) -> HostResult<JsValue> {
            if let Some(buffer) = self.client_attrib_buffers.get(&index).cloned() {
                return Ok(buffer);
            }
            let buffer = self.call("createBuffer", &[])?;
            if buffer.is_null() || buffer.is_undefined() {
                return Err(HostError::new(format!(
                    "WebGL createBuffer failed for client attrib {index}"
                )));
            }
            self.client_attrib_buffers.insert(index, buffer.clone());
            Ok(buffer)
        }

        fn client_index_buffer(&mut self) -> HostResult<JsValue> {
            if let Some(buffer) = self.client_index_buffer.clone() {
                return Ok(buffer);
            }
            let buffer = self.call("createBuffer", &[])?;
            if buffer.is_null() || buffer.is_undefined() {
                return Err(HostError::new(
                    "WebGL createBuffer failed for client indices",
                ));
            }
            self.client_index_buffer = Some(buffer.clone());
            Ok(buffer)
        }

        fn capture_framebuffer_readback(&mut self) -> HostResult<()> {
            let Some(byte_len) = usize::try_from(self.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(self.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4))
            else {
                return Err(HostError::new(format!(
                    "framebuffer readback dimensions too large: {}x{}",
                    self.width, self.height
                )));
            };
            let array = Uint8Array::new_with_length(
                u32::try_from(byte_len)
                    .map_err(|_| HostError::new("framebuffer readback too large"))?,
            );
            self.call(
                "readPixels",
                &[
                    i32_number(0),
                    i32_number(0),
                    i32_number(self.width as i32),
                    i32_number(self.height as i32),
                    number(GL_RGBA),
                    number(GL_UNSIGNED_BYTE),
                    array.clone().into(),
                ],
            )?;
            let mut pixels = vec![0_u8; byte_len];
            array.copy_to(&mut pixels);
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
            self.stats.readback_width = self.width;
            self.stats.readback_height = self.height;
            self.stats.readback_nonzero_rgb_pixels = nonzero_rgb;
            self.stats.readback_nonzero_alpha_pixels = nonzero_alpha;
            Ok(())
        }

        fn record_gl_errors(&mut self, event_index: usize, event_kind: &'static str) {
            loop {
                let Ok(error) = self.call("getError", &[]) else {
                    return;
                };
                let Some(error) = error.as_f64().map(|value| value as u32) else {
                    return;
                };
                if error == GL_NO_ERROR {
                    break;
                }
                if self.stats.gl_error_count == 0 {
                    self.stats.first_gl_error_event_index = event_index;
                    self.stats.first_gl_error_event_kind = Some(event_kind);
                    self.stats.first_gl_error_code = error;
                }
                self.stats.gl_error_count += 1;
            }
        }

        fn shader_info_log(&self, shader: &JsValue) -> String {
            self.call("getShaderInfoLog", &[shader.clone()])
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default()
        }

        fn program_info_log(&self, program: &JsValue) -> String {
            self.call("getProgramInfoLog", &[program.clone()])
                .ok()
                .and_then(|value| value.as_string())
                .unwrap_or_default()
        }

        fn call(&self, name: &str, args: &[JsValue]) -> HostResult<JsValue> {
            call_method(&self.context, name, args).map_err(js_to_host_error)
        }
    }

    impl HostBackend for BrowserWebGlHost {
        fn poll_events(&mut self) -> HostResult<Vec<HostEvent>> {
            Ok(Vec::new())
        }

        fn replay_gles_events(&mut self, events: &[GlesEvent]) -> HostResult<()> {
            self.record_gl_errors(usize::MAX, "replay-init");
            for (index, event) in events.iter().enumerate() {
                self.replay_gles_event(event)?;
                self.record_gl_errors(index, event.kind());
            }
            Ok(())
        }

        fn swap_buffers(&mut self) -> HostResult<()> {
            Ok(())
        }
    }

    fn call_method(context: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
        let method = Reflect::get(context, &JsValue::from_str(name))?;
        let function = method.dyn_into::<Function>()?;
        let array = Array::new();
        for arg in args {
            array.push(arg);
        }
        function.apply(context, &array)
    }

    fn js_to_host_error(value: JsValue) -> HostError {
        HostError::new(
            value
                .as_string()
                .unwrap_or_else(|| format!("JavaScript error: {value:?}")),
        )
    }

    fn number(value: u32) -> JsValue {
        JsValue::from_f64(f64::from(value))
    }

    fn i32_number(value: i32) -> JsValue {
        JsValue::from_f64(f64::from(value))
    }

    fn float_bits_number(value: u32) -> JsValue {
        JsValue::from_f64(f64::from(f32::from_bits(value)))
    }

    fn payload_bytes(payload: Option<&[u8]>, size: u32) -> Option<&[u8]> {
        let size = usize::try_from(size).ok()?;
        if size == 0 {
            Some(&[])
        } else {
            payload.filter(|payload| payload.len() >= size)
        }
    }

    fn f32_values(payload: &[u8]) -> Vec<f32> {
        payload
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    fn i32_values(payload: &[u8]) -> Vec<i32> {
        payload
            .chunks_exact(4)
            .map(|chunk| i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    fn uniform_lookup_names(name: &str) -> [&str; 2] {
        [name, name.strip_suffix("[0]").unwrap_or(name)]
    }
}

#[cfg(all(feature = "webgl", target_arch = "wasm32"))]
pub use browser::BrowserWebGlHost;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gles2_uses_webgl1_context_name() {
        assert_eq!(webgl_context_name(GraphicsApi::Gles2), "webgl");
        assert_eq!(webgl_context_name(GraphicsApi::WebGl1), "webgl");
        assert_eq!(webgl_context_name(GraphicsApi::WebGl2), "webgl2");
    }

    #[test]
    fn replay_stats_start_empty() {
        assert_eq!(WebGlReplayStats::default().draw_elements, 0);
        assert_eq!(WebGlReplayStats::default().first_gl_error_event_kind, None);
    }
}
