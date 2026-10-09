use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

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

static THEME_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct ThemeGuard {
    previous: crate::theme::ThemeVariant,
    glass: bool,
    _lock: MutexGuard<'static, ()>,
}

impl ThemeGuard {
    pub(crate) fn new() -> Self {
        let lock = THEME_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        FILLS.with_borrow_mut(HashMap::clear);
        Self {
            previous: crate::theme::active_variant(),
            glass: crate::theme::is_glass(),
            _lock: lock,
        }
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        crate::theme::set_active_variant(self.previous);
        crate::theme::set_glass(self.glass);
        FILLS.with_borrow_mut(HashMap::clear);
    }
}
