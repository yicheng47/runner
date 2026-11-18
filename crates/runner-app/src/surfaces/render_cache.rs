use std::rc::Rc;

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, StyleRefinement, Subscription, WeakEntity, Window,
};

use crate::NativeRoot;

type Renderer<T> = Rc<dyn Fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement>;

pub(crate) struct ShellComposition {
    root: WeakEntity<NativeRoot>,
}

impl ShellComposition {
    pub(crate) fn new(root: WeakEntity<NativeRoot>) -> Self {
        Self { root }
    }
}

impl Render for ShellComposition {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _turn = crate::stall_watchdog::enter_turn("root.render");
        self.root
            .update(cx, |root, cx| {
                if !cx.has_active_drag() {
                    root.pane_drop = None;
                }
                root.render_app_shell(window, cx)
            })
            .expect("window root is alive")
    }
}

pub(crate) struct CachedRegion<T: 'static> {
    root: WeakEntity<T>,
    renderer: Renderer<T>,
    _subscription: Subscription,
}

impl<T: 'static> Render for CachedRegion<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self
            .root
            .update(cx, |root, cx| (self.renderer)(root, window, cx))
            .expect("window root is alive");
        // A cached view lays its element out as a layout root, where a flex
        // root shrinks to its content; a block root stretches to the region.
        div().size_full().child(content)
    }
}

impl NativeRoot {
    pub(crate) fn cached_region(
        &mut self,
        key: impl Into<String>,
        style: StyleRefinement,
        renderer: impl Fn(&mut Self, &mut Window, &mut Context<Self>) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        cached_region(
            cx.entity(),
            &mut self.render_regions,
            key,
            style,
            renderer,
            cx,
        )
    }
}

impl super::MissionWorkspace {
    pub(super) fn cached_region(
        &mut self,
        key: impl Into<String>,
        style: StyleRefinement,
        renderer: impl Fn(&mut Self, &mut Window, &mut Context<Self>) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        cached_region(
            cx.entity(),
            &mut self.render_regions,
            key,
            style,
            renderer,
            cx,
        )
    }
}

fn cached_region<T: 'static>(
    root: Entity<T>,
    regions: &mut std::collections::HashMap<String, Entity<CachedRegion<T>>>,
    key: impl Into<String>,
    style: StyleRefinement,
    renderer: impl Fn(&mut T, &mut Window, &mut Context<T>) -> AnyElement + 'static,
    cx: &mut Context<T>,
) -> AnyElement {
    let key = key.into();
    let renderer: Renderer<T> = Rc::new(renderer);
    let region = regions.entry(key).or_insert_with(|| {
        let renderer = renderer.clone();
        cx.new(|cx| CachedRegion {
            root: root.downgrade(),
            renderer,
            _subscription: cx.observe(&root, |_, _, cx| cx.notify()),
        })
    });
    region.update(cx, |region, _| region.renderer = renderer);
    region.clone().cached(style).into_any_element()
}
