#[cfg(target_os = "macos")]
use gpui::Window;

#[cfg(target_os = "macos")]
pub fn sync_traffic_lights(window: &Window, zoom: f32) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
    use raw_window_handle::RawWindowHandle;

    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let view_ptr = handle.ns_view.as_ptr().cast::<NSView>();
    let Some(view) = (unsafe { Retained::retain(view_ptr) }) else {
        return;
    };
    let Some(window) = view.window() else {
        return;
    };
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(minimize) = window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let Some(maximize) = window.standardWindowButton(NSWindowButton::ZoomButton) else {
        return;
    };
    let Some(button_group) = (unsafe { close.superview() }) else {
        return;
    };
    let Some(titlebar_container) = (unsafe { button_group.superview() }) else {
        return;
    };

    let titlebar_height = 44. * zoom as f64;
    let close_frame = close.frame();
    let button_height = close_frame.size.height;
    let spacing = minimize.frame().origin.x - close_frame.origin.x;
    let mut titlebar_frame = titlebar_container.frame();
    titlebar_frame.size.height = titlebar_height;
    titlebar_frame.origin.y = window.frame().size.height - titlebar_height;
    titlebar_container.setFrame(titlebar_frame);

    for (index, button) in [close, minimize, maximize].into_iter().enumerate() {
        let mut frame = button.frame();
        frame.origin.x = 16. + index as f64 * spacing;
        frame.origin.y = (titlebar_height - button_height) / 2.;
        button.setFrameOrigin(frame.origin);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn sync_traffic_lights(_: &gpui::Window, _: f32) {}
