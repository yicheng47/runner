//! Raw C ABI bindings to Runner's vendored static `libghostty-vt` (`vt-1.0.2`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::ptr;

    #[test]
    fn links_and_reports_pinned_version() {
        let version = unsafe { CStr::from_ptr(ghostty_version()) }
            .to_str()
            .expect("UTF-8 version string");
        assert_eq!(version, "1.0.2");

        let mut simd = false;
        let res =
            unsafe { ghostty_build_info(GHOSTTY_BUILD_INFO_SIMD, (&mut simd as *mut bool).cast()) };
        assert_eq!(res, GHOSTTY_SUCCESS);
        assert!(simd, "expected SIMD enabled in libghostty-vt build");
    }

    #[test]
    fn terminal_round_trip_and_render_state() {
        let mut term: GhosttyTerminal = ptr::null_mut();
        assert_eq!(
            unsafe { ghostty_terminal_new(ptr::null(), &mut term, 20, 4) },
            GHOSTTY_SUCCESS
        );
        assert!(!term.is_null());

        let text = b"hello \x1b[1mworld\x1b[0m";
        unsafe { ghostty_terminal_vt_write(term, text.as_ptr(), text.len()) };

        let mut cols: u16 = 0;
        let mut rows: u16 = 0;
        let mut cursor_x: u16 = 0;
        let mut cursor_y: u16 = 0;
        unsafe {
            assert_eq!(
                ghostty_terminal_get(
                    term,
                    GHOSTTY_TERMINAL_DATA_COLS,
                    (&mut cols as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_get(
                    term,
                    GHOSTTY_TERMINAL_DATA_ROWS,
                    (&mut rows as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_get(
                    term,
                    GHOSTTY_TERMINAL_DATA_CURSOR_X,
                    (&mut cursor_x as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_get(
                    term,
                    GHOSTTY_TERMINAL_DATA_CURSOR_Y,
                    (&mut cursor_y as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
        }
        assert_eq!((cols, rows, cursor_x, cursor_y), (20, 4, 11, 0));

        let mut render: GhosttyRenderState = ptr::null_mut();
        assert_eq!(
            unsafe { ghostty_render_state_new(ptr::null(), &mut render) },
            GHOSTTY_SUCCESS
        );
        assert_eq!(
            unsafe { ghostty_render_state_update(render, term) },
            GHOSTTY_SUCCESS
        );

        let mut render_cols: u16 = 0;
        let mut render_rows: u16 = 0;
        unsafe {
            assert_eq!(
                ghostty_render_state_get(
                    render,
                    GHOSTTY_RENDER_STATE_DATA_COLS,
                    (&mut render_cols as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_render_state_get(
                    render,
                    GHOSTTY_RENDER_STATE_DATA_ROWS,
                    (&mut render_rows as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            ghostty_render_state_free(render);
            ghostty_terminal_free(term);
        }
        assert_eq!((render_cols, render_rows), (20, 4));
    }
}
