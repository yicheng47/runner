use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{Fill, InteractiveElement, Styled, VisualTestContext};

thread_local! {
    static FILLS: RefCell<HashMap<&'static str, Option<Fill>>> = RefCell::default();
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

pub(crate) struct ThemeGuard {
    _theme: crate::theme::testing::ThemeScope,
}

impl ThemeGuard {
    pub(crate) fn new() -> Self {
        FILLS.with_borrow_mut(HashMap::clear);
        Self {
            _theme: crate::theme::testing::scope(),
        }
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        FILLS.with_borrow_mut(HashMap::clear);
    }
}
