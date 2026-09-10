// FIXTURE — not compiled, not shipped. Exists to make the geometry gate fire.
//
// This is `apply_saved_geometry` as it actually was: two numbers out of the
// config, straight into `set_position`, and a comment asserting that the OS
// would refuse a position that no longer fits a display. It does not. This is
// the code that opened the window onto a monitor that had been unplugged.

fn apply_saved_geometry(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let config = UserConfig::load();

    if let (Some(w), Some(h)) = (config.window_w, config.window_h) {
        let _ = window.set_size(tauri::PhysicalSize::new(w, h));
    }

    // The claim this gate exists to disprove.
    if let (Some(x), Some(y)) = (config.window_x, config.window_y) {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}
