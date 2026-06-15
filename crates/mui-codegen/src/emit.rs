//! Tiny source-emitting buffer: tracks indentation and hands out unique local
//! variable names so the generated Rust is readable and never shadows.

/// Accumulates lines of generated Rust at the current indentation level.
pub struct Emitter {
    buf: String,
    depth: usize,
    counter: usize,
    /// Window/screen size + `id:` widget sizes, so dimension props like
    /// `width: Window.width - 520` / `left_panel.width` resolve to constants.
    pub dims: mui_syntax::style::DimEnv,
    /// Live reactive signal snapshot (`is_macos = "1"`, `scene_h = 240`,
    /// …) so dimension ternaries like `height: ${scene_h}` resolve
    /// to a concrete f32 at codegen time. Same shape as
    /// `mui_runtime::build_reactive_env` produces.
    pub reactive_env: mui_syntax::style::ReactiveEnv,
}

impl Emitter {
    pub fn new() -> Self {
        Emitter {
            buf: String::new(),
            depth: 0,
            counter: 0,
            dims: mui_syntax::style::DimEnv::default(),
            reactive_env: mui_syntax::style::ReactiveEnv::default(),
        }
    }

    /// Write one line at the current indentation, followed by a newline.
    pub fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.buf.push_str("    ");
        }
        self.buf.push_str(text);
        self.buf.push('\n');
    }

    pub fn indent(&mut self) {
        self.depth += 1;
    }

    pub fn dedent(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// A fresh, collision-free local name like `stack_0`, `stack_1`, …
    pub fn fresh(&mut self, base: &str) -> String {
        let n = self.counter;
        self.counter += 1;
        format!("__{base}_{n}")
    }

    /// Consume the emitter and return the accumulated source.
    pub fn finish(self) -> String {
        self.buf
    }
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new()
    }
}
