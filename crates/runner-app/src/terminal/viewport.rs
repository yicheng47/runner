use gpui::{
    prelude::*, AnyElement, App, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, StyleRefinement, Subscription, Window,
};

use super::element::TerminalElement;

struct TerminalViewport {
    element: TerminalElement,
    _subscriptions: Vec<Subscription>,
}

impl TerminalViewport {
    fn subscriptions(element: &TerminalElement, cx: &mut Context<Self>) -> Vec<Subscription> {
        let mut subscriptions = vec![
            cx.observe(&element.interaction, |_, _, cx| cx.notify()),
            cx.observe(&element.input, |_, _, cx| cx.notify()),
        ];
        if let Some(find) = &element.find {
            subscriptions.push(cx.observe(find, |_, _, cx| cx.notify()));
        }
        if let Some(updates) = cx
            .try_global::<crate::app_store::GlobalAppStore>()
            .map(|store| store.0.read(cx).terminal_updates.clone())
        {
            subscriptions.push(cx.subscribe(&updates, |this, _, id: &String, cx| {
                if id == this.element.session.session_id() {
                    cx.notify();
                }
            }));
        }
        subscriptions
    }
}

impl Render for TerminalViewport {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.element.clone()
    }
}

pub(crate) struct TerminalViewportElement {
    element: TerminalElement,
}

impl TerminalViewportElement {
    pub(crate) fn new(element: TerminalElement) -> Self {
        Self { element }
    }
}

impl Element for TerminalViewportElement {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(format!("terminal-viewport-{}", self.element.session.session_id()).into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let view =
            window.with_element_state::<Entity<TerminalViewport>, _>(id.unwrap(), |view, _| {
                let view = view.unwrap_or_else(|| {
                    let element = self.element.clone();
                    cx.new(|cx| {
                        let subscriptions = TerminalViewport::subscriptions(&element, cx);
                        TerminalViewport {
                            element,
                            _subscriptions: subscriptions,
                        }
                    })
                });
                view.update(cx, |view, cx| {
                    let old = &view.element;
                    let new = &self.element;
                    let changed = !std::sync::Arc::ptr_eq(&old.session, &new.session)
                        || old.interaction != new.interaction
                        || old.input != new.input
                        || old.focus_handle != new.focus_handle
                        || old.interactive != new.interactive
                        || old.scrollable != new.scrollable
                        || old.resize_owner != new.resize_owner
                        || old.style != new.style
                        || old.find != new.find;
                    if old.interaction != new.interaction
                        || old.input != new.input
                        || old.find != new.find
                    {
                        view._subscriptions = TerminalViewport::subscriptions(new, cx);
                    }
                    view.element = new.clone();
                    if changed {
                        cx.notify();
                    }
                });
                (view.clone(), view)
            });
        let mut child = view
            .cached(StyleRefinement::default().size_full())
            .into_any_element();
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

impl IntoElement for TerminalViewportElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}
