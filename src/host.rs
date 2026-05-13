use std::error::Error;
use std::fmt;

use crate::hle_imports::GlesEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsApi {
    Gles2,
    WebGl1,
    WebGl2,
}

impl GraphicsApi {
    pub fn guest_name(self) -> &'static str {
        match self {
            Self::Gles2 => "OpenGL ES 2.0",
            Self::WebGl1 => "WebGL 1",
            Self::WebGl2 => "WebGL 2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub graphics_api: GraphicsApi,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "aemu".to_string(),
            width: 854,
            height: 480,
            graphics_api: GraphicsApi::Gles2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKey {
    Escape,
    Enter,
    Space,
    Backspace,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Up,
    Move,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Quit,
    Key {
        key: HostKey,
        pressed: bool,
        repeat: bool,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Pointer {
        id: i64,
        phase: PointerPhase,
        x: f32,
        y: f32,
        pressure: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for HostError {}

pub type HostResult<T> = Result<T, HostError>;

pub trait HostBackend {
    fn poll_events(&mut self) -> HostResult<Vec<HostEvent>>;
    fn replay_gles_events(&mut self, _events: &[GlesEvent]) -> HostResult<()> {
        Ok(())
    }
    fn swap_buffers(&mut self) -> HostResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_host_config_targets_mcpe_landscape_shape() {
        let config = HostConfig::default();

        assert_eq!(config.width, 854);
        assert_eq!(config.height, 480);
        assert_eq!(config.graphics_api, GraphicsApi::Gles2);
    }

    #[test]
    fn graphics_api_names_are_stable_for_logs() {
        assert_eq!(GraphicsApi::Gles2.guest_name(), "OpenGL ES 2.0");
        assert_eq!(GraphicsApi::WebGl1.guest_name(), "WebGL 1");
        assert_eq!(GraphicsApi::WebGl2.guest_name(), "WebGL 2");
    }
}
