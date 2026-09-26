use super::*;

#[test]
fn viewport_default_values() {
    let viewport = Viewport::default();
    assert_eq!(viewport.window_size, (0, 0));
    assert_eq!(viewport.hidpi_scale, 1.0);
    assert_eq!(viewport.zoom, 1.0);
    assert_eq!(viewport.color_scheme, ColorScheme::Light);
}

#[test]
fn color_scheme_default_is_light() {
    assert_eq!(ColorScheme::default(), ColorScheme::Light);
    assert_ne!(ColorScheme::Light, ColorScheme::Dark);
}

#[test]
fn viewport_new_preserves_args_with_unit_zoom() {
    let viewport = Viewport::new(800, 600, 2.0, ColorScheme::Dark);
    assert_eq!(viewport.window_size, (800, 600));
    assert_eq!(viewport.hidpi_scale, 2.0);
    assert_eq!(viewport.zoom, 1.0);
    assert_eq!(viewport.color_scheme, ColorScheme::Dark);
}

#[test]
fn scale_multiplies_hidpi_and_zoom() {
    let mut viewport = Viewport::new(800, 600, 2.0, ColorScheme::Light);
    viewport.set_zoom(1.5);
    assert_eq!(viewport.scale(), 3.0);
    assert_eq!(viewport.scale_f64(), 3.0_f64);
}

#[test]
fn set_hidpi_scale_updates_scale() {
    let mut viewport = Viewport::default();
    viewport.set_hidpi_scale(2.0);
    assert_eq!(viewport.scale(), 2.0);
}

#[test]
fn zoom_set_and_by() {
    let mut viewport = Viewport::default();
    viewport.set_zoom(2.0);
    assert_eq!(viewport.zoom(), 2.0);
    viewport.zoom_by(0.5);
    assert_eq!(viewport.zoom(), 2.5);
    *viewport.zoom_mut() = 1.0;
    assert_eq!(viewport.zoom(), 1.0);
}

#[test]
fn dummy_shell_provider_defaults() {
    let provider = DummyShellProvider;
    provider.request_redraw();
    provider.set_cursor(None);
    provider.set_cursor(Some(CursorIcon::Default));
    provider.set_window_title("title".to_string());
    provider.set_ime_enabled(true);
    provider.set_ime_cursor_area(0.0, 0.0, 10.0, 10.0);
    assert!(provider.get_clipboard_text().is_err());
    assert!(provider.set_clipboard_text("x".to_string()).is_err());
    assert!(provider.open_file_dialog(false, None).is_empty());
    provider.request_window_close();
    provider.set_window_minimized(true);
    provider.set_window_maximized(true);
    assert!(!provider.is_window_maximized());
    provider.set_window_decorations(true);
    provider.drag_window();
}

#[test]
fn file_dialog_filter_constructs() {
    let filter = FileDialogFilter {
        name: "images".to_string(),
        extensions: vec!["png".to_string()],
    };
    assert_eq!(filter.name, "images");
    assert_eq!(filter.extensions, vec!["png".to_string()]);
}
