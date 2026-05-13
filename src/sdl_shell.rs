use crate::hle_imports::GlesEvent;
use crate::host::{
    HostBackend, HostConfig, HostError, HostEvent, HostKey, HostResult, PointerPhase,
};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::video::GLProfile;
use std::thread;
use std::time::{Duration, Instant};

pub struct Sdl2Host {
    _sdl: sdl2::Sdl,
    _video: sdl2::VideoSubsystem,
    window: sdl2::video::Window,
    _gl_context: sdl2::video::GLContext,
    event_pump: sdl2::EventPump,
    gl: SdlGl,
}

type GlClear = unsafe extern "C" fn(u32);
type GlClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
type GlClearDepthf = unsafe extern "C" fn(f32);
type GlViewport = unsafe extern "C" fn(i32, i32, i32, i32);

struct SdlGl {
    clear: GlClear,
    clear_color: GlClearColor,
    clear_depthf: Option<GlClearDepthf>,
    viewport: GlViewport,
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
        })
    }
}

impl SdlGl {
    fn load(video: &sdl2::VideoSubsystem) -> HostResult<Self> {
        Ok(Self {
            clear: load_required_gl(video, "glClear")?,
            clear_color: load_required_gl(video, "glClearColor")?,
            clear_depthf: load_optional_gl(video, "glClearDepthf"),
            viewport: load_required_gl(video, "glViewport")?,
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
        for event in events {
            match event {
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
                GlesEvent::SwapBuffers { .. } => self.swap_buffers()?,
                _ => {}
            }
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
