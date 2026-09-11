//! blitz_traits::shell compatible shape.

use cursor_icon::CursorIcon;

pub struct ClipboardError;

pub trait ShellProvider: Send + Sync + 'static {
    fn request_redraw(&self) {}
    fn set_cursor(&self, icon: Option<CursorIcon>) {
        let _ = icon;
    }
    fn set_window_title(&self, title: String) {
        let _ = title;
    }
    fn set_ime_enabled(&self, is_enabled: bool) {
        let _ = is_enabled;
    }
    fn set_ime_cursor_area(&self, x: f32, y: f32, width: f32, height: f32) {
        let _ = x;
        let _ = y;
        let _ = width;
        let _ = height;
    }
    fn get_clipboard_text(&self) -> Result<String, ClipboardError> {
        Err(ClipboardError)
    }
    fn set_clipboard_text(&self, text: String) -> Result<(), ClipboardError> {
        let _ = text;
        Err(ClipboardError)
    }
    fn open_file_dialog(
        &self,
        multiple: bool,
        filter: Option<FileDialogFilter>,
    ) -> Vec<std::path::PathBuf> {
        let _ = multiple;
        let _ = filter;
        vec![]
    }
    fn request_window_close(&self) {}
    fn set_window_minimized(&self, minimized: bool) {
        let _ = minimized;
    }
    fn set_window_maximized(&self, maximized: bool) {
        let _ = maximized;
    }
    fn is_window_maximized(&self) -> bool {
        false
    }
    fn set_window_decorations(&self, decorations: bool) {
        let _ = decorations;
    }
    fn drag_window(&self) {}
}

pub struct DummyShellProvider;
impl ShellProvider for DummyShellProvider {}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    pub color_scheme: ColorScheme,
    pub window_size: (u32, u32),
    pub hidpi_scale: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            window_size: (0, 0),
            hidpi_scale: 1.0,
            zoom: 1.0,
            color_scheme: ColorScheme::Light,
        }
    }
}

impl Viewport {
    pub fn new(
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
        color_scheme: ColorScheme,
    ) -> Self {
        Self {
            window_size: (physical_width, physical_height),
            hidpi_scale: scale_factor,
            zoom: 1.0,
            color_scheme,
        }
    }
    pub fn scale(&self) -> f32 {
        self.hidpi_scale * self.zoom
    }
    pub fn scale_f64(&self) -> f64 {
        self.scale() as f64
    }
    pub fn set_hidpi_scale(&mut self, scale: f32) {
        self.hidpi_scale = scale;
    }
    pub fn zoom(&self) -> f32 {
        self.zoom
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
    }
    pub fn zoom_by(&mut self, zoom: f32) {
        self.zoom += zoom;
    }
    pub fn zoom_mut(&mut self) -> &mut f32 {
        &mut self.zoom
    }
}

pub struct FileDialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}
