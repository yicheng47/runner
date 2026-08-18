//! Tokyo Night constants for the native shell (grid colors come from
//! `palette`).

use gpui::{rgb, Hsla};

pub fn bg() -> Hsla {
    rgb(0x1a1b26).into()
}

pub fn text() -> Hsla {
    rgb(0xc0caf5).into()
}

pub fn muted() -> Hsla {
    rgb(0x565f89).into()
}

pub fn accent() -> Hsla {
    rgb(0x7aa2f7).into()
}

pub fn composer_bg() -> Hsla {
    rgb(0x16161e).into()
}

pub fn border() -> Hsla {
    rgb(0x292e42).into()
}
