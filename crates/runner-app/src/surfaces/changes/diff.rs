//! Virtualized Unified and Split diff view with two-version background
//! highlighting, gutter-free text selection/copy, hunk navigation, and
//! end-of-diff viewed box.

mod element;
mod shape;
#[cfg(test)]
mod tests;
mod view;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, fill, point, px, relative, size, svg, App, Bounds, ClipboardItem, ContentMask, Context,
    CursorStyle, DispatchPhase, Element, Entity, FocusHandle, Font, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollEvent, ShapedLine, SharedString, Style,
    TextAlign, TextRun, Window,
};
use runner_app::ui::{
    app_scrollbar_gutter, Scrollbar, ScrollbarKind, ScrollbarLanes, ScrollbarMetrics,
};
use runner_core::protocol::changes::{DiffHunk, DiffOp};

use crate::app_settings::DiffViewMode;
use crate::surfaces::files::code::{word_at, CodeView};
use crate::surfaces::files::document::Document;
use crate::surfaces::files::highlight::{self, Grammar, Highlights};
use crate::surfaces::files::syntax::SyntaxTheme;
use crate::theme;
use crate::{Copy, SelectAll};

use element::UnifiedDiffElement;
use shape::*;
#[cfg(test)]
pub(crate) use view::build_split_rows;
pub(crate) use view::{render_viewed_checkbox, UnifiedDiffView};

const GUTTER_PAD_LEFT: f32 = 8.;
const COL_GAP: f32 = 8.;
const SIGN_WIDTH: f32 = 10.;
const TEXT_GAP: f32 = 8.;
const TEXT_PAD_RIGHT: f32 = 24.;
const TOP_PAD: f32 = 6.;
const BOTTOM_PAD_NO_FOOTER: f32 = 8.;
const FOOTER_HEIGHT: f32 = 40.;

pub(crate) type DiffCallback = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiffPos {
    pub(crate) row: usize,
    pub(crate) offset: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DiffSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffRow {
    Hunk { header: SharedString },
    Context { old_line: usize, new_line: usize },
    Removed { old_line: usize },
    Added { new_line: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitCell {
    Empty,
    Context { line: usize },
    Removed { line: usize },
    Added { line: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SplitRow {
    Hunk { header: SharedString },
    Pair { left: SplitCell, right: SplitCell },
}

#[derive(Clone, Copy, Debug)]
struct Metrics {
    mode: DiffViewMode,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    char_width: Pixels,
    old_num_right: Pixels,
    new_num_right: Pixels,
    sign_left: Pixels,
    text_left: Pixels,
    left_col_width: Pixels,
    right_col_left: Pixels,
    right_col_width: Pixels,
    right_num_right: Pixels,
    right_sign_left: Pixels,
    right_text_left: Pixels,
    lanes: ScrollbarLanes,
}

impl Metrics {
    /// What the scrollbars' lanes leave of the view.
    fn viewport(&self) -> Bounds<Pixels> {
        Bounds::new(
            self.bounds.origin,
            size(
                (self.bounds.size.width - self.lanes.right).max(px(0.)),
                (self.bounds.size.height - self.lanes.bottom).max(px(0.)),
            ),
        )
    }

    fn left_text_width(&self) -> Pixels {
        (self.left_col_width - self.text_left).max(px(0.))
    }

    fn right_text_width(&self) -> Pixels {
        (self.right_col_width - self.lanes.right - self.right_text_left).max(px(0.))
    }

    fn scroll_viewport_width(&self) -> Pixels {
        match self.mode {
            DiffViewMode::Unified => (self.viewport().size.width - self.text_left).max(px(0.)),
            DiffViewMode::Split => self.left_text_width().min(self.right_text_width()),
        }
    }

    fn shape_viewport_width(&self) -> Pixels {
        match self.mode {
            DiffViewMode::Unified => (self.viewport().size.width - self.text_left).max(px(0.)),
            DiffViewMode::Split => self.left_text_width().max(self.right_text_width()),
        }
    }

    fn side_at_x(&self, x: Pixels) -> DiffSide {
        if x < self.bounds.left() + self.right_col_left {
            DiffSide::Left
        } else {
            DiffSide::Right
        }
    }

    fn side_text_left_abs(&self, side: DiffSide) -> Pixels {
        match (self.mode, side) {
            (DiffViewMode::Unified, _) | (DiffViewMode::Split, DiffSide::Left) => {
                self.bounds.left() + self.text_left
            }
            (DiffViewMode::Split, DiffSide::Right) => {
                self.bounds.left() + self.right_col_left + self.right_text_left
            }
        }
    }

    fn side_right_abs(&self, side: DiffSide) -> Pixels {
        match (self.mode, side) {
            (DiffViewMode::Split, DiffSide::Left) => self.bounds.left() + self.left_col_width,
            _ => self.viewport().right(),
        }
    }
}
