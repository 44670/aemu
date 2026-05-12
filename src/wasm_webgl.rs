use crate::host::GraphicsApi;

pub const DEFAULT_CANVAS_ID: &str = "aemu-canvas";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gles2_uses_webgl1_context_name() {
        assert_eq!(webgl_context_name(GraphicsApi::Gles2), "webgl");
        assert_eq!(webgl_context_name(GraphicsApi::WebGl1), "webgl");
        assert_eq!(webgl_context_name(GraphicsApi::WebGl2), "webgl2");
    }
}
