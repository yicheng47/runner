use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{Fill, InteractiveElement, Styled, VisualTestContext};

thread_local! {
    static FILLS: RefCell<HashMap<&'static str, Option<Fill>>> = RefCell::default();
    static BORDERS: RefCell<HashMap<&'static str, Option<gpui::Hsla>>> = RefCell::default();
    static TEXTS: RefCell<HashMap<&'static str, Option<gpui::Hsla>>> = RefCell::default();
}

pub(crate) fn record_fill<T: Styled + InteractiveElement>(id: &'static str, mut element: T) -> T {
    let fill = element.style().background.clone();
    FILLS.with_borrow_mut(|fills| fills.insert(id, fill));
    element.debug_selector(|| id.into())
}

pub(crate) fn assert_fill(
    window: &mut VisualTestContext,
    id: &'static str,
    color: impl Into<Option<u32>>,
) {
    let bounds = window
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing surface {id}"));
    assert!(
        bounds.size.width > gpui::px(0.) && bounds.size.height > gpui::px(0.),
        "{id}: {bounds:?}"
    );
    let expected = color.into().map(|color| gpui::rgb(color).into());
    FILLS.with_borrow(|fills| assert_eq!(fills.get(id), Some(&expected), "{id}"));
}

pub(crate) fn assert_fill_color(
    window: &mut VisualTestContext,
    id: &'static str,
    color: Option<gpui::Hsla>,
) {
    assert!(window.debug_bounds(id).is_some(), "missing surface {id}");
    let expected = color.map(Fill::from);
    FILLS.with_borrow(|fills| assert_eq!(fills.get(id), Some(&expected), "{id}"));
}

pub(crate) fn record_border<T: Styled + InteractiveElement>(id: &'static str, mut element: T) -> T {
    BORDERS.with_borrow_mut(|borders| borders.insert(id, element.style().border_color));
    element.debug_selector(|| id.into())
}

/// Records an element's border, leaving the debug selector it already has.
pub(crate) fn note_border<T: Styled>(id: &'static str, mut element: T) -> T {
    BORDERS.with_borrow_mut(|borders| borders.insert(id, element.style().border_color));
    element
}

pub(crate) fn record_text<T: Styled + InteractiveElement>(id: &'static str, mut element: T) -> T {
    let color = element.style().text.color;
    TEXTS.with_borrow_mut(|texts| texts.insert(id, color));
    element.debug_selector(|| id.into())
}

pub(crate) fn assert_text(window: &mut VisualTestContext, id: &'static str, color: gpui::Hsla) {
    assert!(window.debug_bounds(id).is_some(), "missing surface {id}");
    TEXTS.with_borrow(|texts| assert_eq!(texts.get(id), Some(&Some(color)), "{id}"));
}

pub(crate) fn clear_border(id: &'static str) {
    BORDERS.with_borrow_mut(|borders| borders.remove(id));
}

pub(crate) fn recorded_border(id: &'static str) -> Option<gpui::Hsla> {
    BORDERS.with_borrow(|borders| borders.get(id).copied().flatten())
}

pub(crate) fn assert_border(window: &mut VisualTestContext, id: &'static str, color: gpui::Hsla) {
    assert!(window.debug_bounds(id).is_some(), "missing surface {id}");
    BORDERS.with_borrow(|borders| assert_eq!(borders.get(id), Some(&Some(color)), "{id}"));
}

pub(crate) struct ThemeGuard {
    _theme: crate::theme::testing::ThemeScope,
}

impl ThemeGuard {
    pub(crate) fn new() -> Self {
        FILLS.with_borrow_mut(HashMap::clear);
        BORDERS.with_borrow_mut(HashMap::clear);
        TEXTS.with_borrow_mut(HashMap::clear);
        Self {
            _theme: crate::theme::testing::scope(),
        }
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        FILLS.with_borrow_mut(HashMap::clear);
        BORDERS.with_borrow_mut(HashMap::clear);
        TEXTS.with_borrow_mut(HashMap::clear);
    }
}
