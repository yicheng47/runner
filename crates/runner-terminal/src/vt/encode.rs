//! Safe wrappers around `libghostty-vt`'s key, mouse, paste, focus, color-scheme
//! and size-report encoders.

use std::marker::PhantomData;
use std::mem::size_of;
use std::ptr;

use ghostty_vt_sys as sys;

use super::terminal::{ColorScheme, Terminal};
use super::{check, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SizeReportStyle {
    Mode2048,
    Csi14T,
    Csi16T,
    Csi18T,
    Csi19T,
}

impl SizeReportStyle {
    fn to_raw(self) -> sys::GhosttySizeReportStyle {
        match self {
            Self::Mode2048 => sys::GHOSTTY_SIZE_REPORT_MODE_2048,
            Self::Csi14T => sys::GHOSTTY_SIZE_REPORT_CSI_14_T,
            Self::Csi16T => sys::GHOSTTY_SIZE_REPORT_CSI_16_T,
            Self::Csi18T => sys::GHOSTTY_SIZE_REPORT_CSI_18_T,
            Self::Csi19T => sys::GHOSTTY_SIZE_REPORT_CSI_19_T,
        }
    }
}

pub(crate) fn encode_color_scheme_report(scheme: ColorScheme) -> Result<Vec<u8>> {
    let mut buf = [0u8; 32];
    let mut written = 0usize;
    check(unsafe {
        sys::ghostty_color_scheme_report_encode(
            scheme.to_raw(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut written,
        )
    })?;
    Ok(buf[..written].to_vec())
}

pub(crate) fn encode_size_report(
    style: SizeReportStyle,
    rows: u16,
    columns: u16,
    cell_width: u32,
    cell_height: u32,
) -> Result<Vec<u8>> {
    let size = sys::GhosttySizeReportSize {
        rows,
        columns,
        cell_width,
        cell_height,
    };
    let mut buf = [0u8; 64];
    let mut written = 0usize;
    check(unsafe {
        sys::ghostty_size_report_encode(
            style.to_raw(),
            &size,
            buf.as_mut_ptr(),
            buf.len(),
            &mut written,
        )
    })?;
    Ok(buf[..written].to_vec())
}

pub(crate) fn encode_focus(focused: bool) -> Result<Vec<u8>> {
    let event = if focused {
        sys::GHOSTTY_FOCUS_GAINED
    } else {
        sys::GHOSTTY_FOCUS_LOST
    };
    let mut buf = [0u8; 16];
    let mut written = 0usize;
    check(unsafe { sys::ghostty_focus_encode(event, buf.as_mut_ptr(), buf.len(), &mut written) })?;
    Ok(buf[..written].to_vec())
}

pub(crate) fn paste_is_safe(text: &str) -> bool {
    unsafe { sys::ghostty_paste_is_safe(text.as_ptr().cast(), text.len()) }
}

pub(crate) fn encode_paste(text: &str, bracketed: bool) -> Result<Vec<u8>> {
    let mut input = text.as_bytes().to_vec();
    let mut needed = 0usize;
    let rc = unsafe {
        sys::ghostty_paste_encode(
            input.as_mut_ptr().cast(),
            input.len(),
            bracketed,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    match rc {
        sys::GHOSTTY_SUCCESS if needed == 0 => Ok(Vec::new()),
        sys::GHOSTTY_OUT_OF_SPACE => {
            let mut out = vec![0u8; needed];
            let mut written = 0usize;
            check(unsafe {
                sys::ghostty_paste_encode(
                    input.as_mut_ptr().cast(),
                    input.len(),
                    bracketed,
                    out.as_mut_ptr(),
                    out.len(),
                    &mut written,
                )
            })?;
            out.truncate(written);
            Ok(out)
        }
        other => Err(check(other).unwrap_err()),
    }
}

pub(crate) struct KeyEncoder {
    encoder: sys::GhosttyKeyEncoder,
    event: sys::GhosttyKeyEvent,
    _not_send_sync: PhantomData<*mut ()>,
}

impl KeyEncoder {
    pub(crate) fn new() -> Result<Self> {
        let mut encoder: sys::GhosttyKeyEncoder = ptr::null_mut();
        check(unsafe { sys::ghostty_key_encoder_new(ptr::null(), &mut encoder) })?;
        let mut event: sys::GhosttyKeyEvent = ptr::null_mut();
        if let Err(err) = check(unsafe { sys::ghostty_key_event_new(ptr::null(), &mut event) }) {
            unsafe { sys::ghostty_key_encoder_free(encoder) };
            return Err(err);
        }
        Ok(Self {
            encoder,
            event,
            _not_send_sync: PhantomData,
        })
    }

    pub(crate) fn sync_from_terminal(&mut self, term: &Terminal) -> Result<()> {
        check(unsafe { sys::ghostty_key_encoder_set_from_terminal(self.encoder, term.raw()) })
    }

    pub(crate) fn set_cursor_key_application(&mut self, enabled: bool) -> Result<()> {
        check(unsafe {
            sys::ghostty_key_encoder_set(
                self.encoder,
                sys::GHOSTTY_KEY_ENCODER_OPT_CURSOR_KEY_APPLICATION,
                (&raw const enabled).cast(),
            )
        })
    }

    pub(crate) fn set_keypad_key_application(&mut self, enabled: bool) -> Result<()> {
        check(unsafe {
            sys::ghostty_key_encoder_set(
                self.encoder,
                sys::GHOSTTY_KEY_ENCODER_OPT_KEYPAD_KEY_APPLICATION,
                (&raw const enabled).cast(),
            )
        })
    }

    pub(crate) fn set_alt_esc_prefix(&mut self, enabled: bool) -> Result<()> {
        check(unsafe {
            sys::ghostty_key_encoder_set(
                self.encoder,
                sys::GHOSTTY_KEY_ENCODER_OPT_ALT_ESC_PREFIX,
                (&raw const enabled).cast(),
            )
        })
    }

    pub(crate) fn set_kitty_flags(&mut self, flags: u8) -> Result<()> {
        check(unsafe {
            sys::ghostty_key_encoder_set(
                self.encoder,
                sys::GHOSTTY_KEY_ENCODER_OPT_KITTY_FLAGS,
                (&raw const flags).cast(),
            )
        })
    }

    pub(crate) fn set_macos_option_as_alt(&mut self, opt: sys::GhosttyOptionAsAlt) -> Result<()> {
        check(unsafe {
            sys::ghostty_key_encoder_set(
                self.encoder,
                sys::GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                (&raw const opt).cast(),
            )
        })
    }

    pub(crate) fn encode(
        &mut self,
        action: sys::GhosttyKeyAction,
        key: sys::GhosttyKey,
        mods: sys::GhosttyMods,
        consumed_mods: sys::GhosttyMods,
        utf8: &str,
        unshifted_codepoint: u32,
    ) -> Result<Vec<u8>> {
        // `ghostty_key_event_set_utf8` borrows the byte slice until replaced or freed;
        // always clear it before returning so `self.event` never retains a dangling pointer.
        struct ClearUtf8Guard(sys::GhosttyKeyEvent);
        impl Drop for ClearUtf8Guard {
            fn drop(&mut self) {
                unsafe {
                    sys::ghostty_key_event_set_utf8(
                        self.0,
                        sys::GhosttyString {
                            ptr: ptr::null(),
                            len: 0,
                        },
                    );
                }
            }
        }

        unsafe {
            check(sys::ghostty_key_event_set_action(self.event, action))?;
            check(sys::ghostty_key_event_set_key(self.event, key))?;
            check(sys::ghostty_key_event_set_mods(self.event, mods))?;
            check(sys::ghostty_key_event_set_consumed_mods(
                self.event,
                consumed_mods,
            ))?;
        }
        let _clear_utf8 = ClearUtf8Guard(self.event);
        unsafe {
            check(sys::ghostty_key_event_set_utf8(
                self.event,
                sys::GhosttyString {
                    ptr: if utf8.is_empty() {
                        ptr::null()
                    } else {
                        utf8.as_ptr()
                    },
                    len: utf8.len(),
                },
            ))?;
            check(sys::ghostty_key_event_set_unshifted_codepoint(
                self.event,
                unshifted_codepoint,
            ))?;
        }
        let mut stack = [0u8; 128];
        let mut written = 0usize;
        let rc = unsafe {
            sys::ghostty_key_encoder_encode(
                self.encoder,
                self.event,
                stack.as_mut_ptr(),
                stack.len(),
                &mut written,
            )
        };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(stack[..written].to_vec()),
            sys::GHOSTTY_OUT_OF_SPACE => {
                let mut heap = vec![0u8; written];
                check(unsafe {
                    sys::ghostty_key_encoder_encode(
                        self.encoder,
                        self.event,
                        heap.as_mut_ptr(),
                        heap.len(),
                        &mut written,
                    )
                })?;
                heap.truncate(written);
                Ok(heap)
            }
            other => Err(check(other).unwrap_err()),
        }
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        unsafe {
            sys::ghostty_key_event_free(self.event);
            sys::ghostty_key_encoder_free(self.encoder);
        }
    }
}

pub(crate) struct MouseEncoder {
    encoder: sys::GhosttyMouseEncoder,
    event: sys::GhosttyMouseEvent,
    _not_send_sync: PhantomData<*mut ()>,
}

impl MouseEncoder {
    pub(crate) fn new() -> Result<Self> {
        let mut encoder: sys::GhosttyMouseEncoder = ptr::null_mut();
        check(unsafe { sys::ghostty_mouse_encoder_new(ptr::null(), &mut encoder) })?;
        let mut event: sys::GhosttyMouseEvent = ptr::null_mut();
        if let Err(err) = check(unsafe { sys::ghostty_mouse_event_new(ptr::null(), &mut event) }) {
            unsafe { sys::ghostty_mouse_encoder_free(encoder) };
            return Err(err);
        }
        Ok(Self {
            encoder,
            event,
            _not_send_sync: PhantomData,
        })
    }

    pub(crate) fn sync_from_terminal(&mut self, term: &Terminal) -> Result<()> {
        check(unsafe { sys::ghostty_mouse_encoder_set_from_terminal(self.encoder, term.raw()) })
    }

    pub(crate) fn set_tracking(&mut self, tracking: sys::GhosttyMouseTrackingMode) -> Result<()> {
        check(unsafe {
            sys::ghostty_mouse_encoder_set(
                self.encoder,
                sys::GHOSTTY_MOUSE_ENCODER_OPT_EVENT,
                (&raw const tracking).cast(),
            )
        })
    }

    pub(crate) fn set_format(&mut self, format: sys::GhosttyMouseFormat) -> Result<()> {
        check(unsafe {
            sys::ghostty_mouse_encoder_set(
                self.encoder,
                sys::GHOSTTY_MOUSE_ENCODER_OPT_FORMAT,
                (&raw const format).cast(),
            )
        })
    }

    pub(crate) fn set_size(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<()> {
        let size = sys::GhosttyMouseEncoderSize {
            size: size_of::<sys::GhosttyMouseEncoderSize>(),
            screen_width,
            screen_height,
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        };
        check(unsafe {
            sys::ghostty_mouse_encoder_set(
                self.encoder,
                sys::GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                (&raw const size).cast(),
            )
        })
    }

    pub(crate) fn set_any_button_pressed(&mut self, pressed: bool) -> Result<()> {
        check(unsafe {
            sys::ghostty_mouse_encoder_set(
                self.encoder,
                sys::GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                (&raw const pressed).cast(),
            )
        })
    }

    pub(crate) fn encode(
        &mut self,
        action: sys::GhosttyMouseAction,
        button: Option<sys::GhosttyMouseButton>,
        mods: sys::GhosttyMods,
        x: f32,
        y: f32,
    ) -> Result<Vec<u8>> {
        unsafe {
            check(sys::ghostty_mouse_event_set_action(self.event, action))?;
            match button {
                Some(b) => check(sys::ghostty_mouse_event_set_button(self.event, b))?,
                None => check(sys::ghostty_mouse_event_clear_button(self.event))?,
            }
            check(sys::ghostty_mouse_event_set_mods(self.event, mods))?;
            check(sys::ghostty_mouse_event_set_position(
                self.event,
                sys::GhosttyMousePosition { x, y },
            ))?;
        }
        let mut buf = [0u8; 64];
        let mut written = 0usize;
        check(unsafe {
            sys::ghostty_mouse_encoder_encode(
                self.encoder,
                self.event,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            )
        })?;
        Ok(buf[..written].to_vec())
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        unsafe {
            sys::ghostty_mouse_event_free(self.event);
            sys::ghostty_mouse_encoder_free(self.encoder);
        }
    }
}
