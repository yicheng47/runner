use super::*;

impl UnifiedViewer {
    // ── Active Views & Markdown ────────────────────────────────────────

    pub(super) fn active_is_image(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        let Some(format) = markdown::image_format(&tab.key.path) else {
            return false;
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return false;
        };
        if format == gpui::ImageFormat::Svg {
            return true;
        }
        !matches!(
            doc.content,
            Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
        ) && !matches!(
            &doc.diff,
            FileTabDiff::Loaded(loaded) if matches!(loaded.kind, LoadedDiffKind::Unified(_))
        )
    }

    pub(super) fn active_image_canvas(&self) -> Option<&Entity<ImageCanvas>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        if doc.mode == DocMode::Diff {
            if !self.active_can_diff() {
                return None;
            }
            return match &doc.diff {
                FileTabDiff::Loaded(loaded) => match &loaded.kind {
                    LoadedDiffKind::Image { canvas, .. } => Some(canvas),
                    LoadedDiffKind::Svg {
                        canvas,
                        source: false,
                        ..
                    } => Some(canvas),
                    _ => None,
                },
                _ => None,
            };
        }
        if self.shows_svg_rendered(doc) {
            return doc.svg_canvas.as_ref();
        }
        match &doc.content {
            Content::Image { canvas, .. }
            | Content::Deleted(Some(DeletedContent::Image { canvas, .. })) => Some(canvas),
            _ => None,
        }
    }

    pub(super) fn set_svg_diff_source(
        &mut self,
        key: &TabKey,
        source: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(doc) = self.docs.get_mut(key)
            && let FileTabDiff::Loaded(loaded) = &mut doc.diff
            && let LoadedDiffKind::Svg {
                source: current, ..
            } = &mut loaded.kind
            && *current != source
        {
            *current = source;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub(super) fn active_view(&self) -> Option<&Entity<CodeView>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        if doc.mode == DocMode::Diff || self.shows_rendered(doc) {
            return None;
        }
        match &doc.content {
            Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => Some(view),
            _ => None,
        }
    }

    pub(super) fn shows_markdown_rendered(&self, doc: &OpenDocument) -> bool {
        doc.mode == DocMode::Code
            && doc.rendered.is_some()
            && !doc.source
            && matches!(
                doc.content,
                Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
            )
    }

    pub(super) fn shows_svg_rendered(&self, doc: &OpenDocument) -> bool {
        doc.mode == DocMode::Code
            && (doc.svg_canvas.is_some() || doc.svg_too_large.is_some())
            && !doc.source
            && matches!(
                doc.content,
                Content::Text(_) | Content::Deleted(Some(DeletedContent::Text(_)))
            )
    }

    fn shows_rendered(&self, doc: &OpenDocument) -> bool {
        self.shows_markdown_rendered(doc) || self.shows_svg_rendered(doc)
    }

    pub(super) fn active_rendered(&self) -> Option<&Entity<MarkdownView>> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        self.shows_markdown_rendered(doc)
            .then_some(doc.rendered.as_ref())
            .flatten()
    }

    pub(super) fn active_svg_rendered(&self) -> Option<bool> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        self.shows_svg_rendered(doc).then_some(true)
    }

    pub(super) fn active_markdown(&self) -> Option<bool> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        (doc.rendered.is_some() || doc.svg_canvas.is_some() || doc.svg_too_large.is_some())
            .then_some(doc.source)
    }

    pub(super) fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.active_markdown() else {
            return;
        };
        if self.active_deleted() {
            return;
        }
        if source {
            if let Some(view) = self.active_view().cloned() {
                view.update(cx, |view, cx| view.set_find(Arc::default(), None, cx));
            }
            self.find = None;
        }
        if let Some(doc) = self
            .tabs
            .active()
            .and_then(|tab| self.docs.get_mut(&tab.key))
        {
            doc.source = !source;
        }
        self.search(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn open_link_handler(&self, cx: &mut Context<Self>) -> markdown::OpenLink {
        let root = cx.weak_entity();
        Rc::new(move |target, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_link(target, cx));
        })
    }

    pub(super) fn open_link(&mut self, target: LinkTarget, cx: &mut Context<Self>) {
        if self.active_deleted() {
            return;
        }
        match target {
            LinkTarget::Url(url) => {
                #[cfg(test)]
                self.opened_urls.push(url);
                #[cfg(not(test))]
                cx.open_url(&url);
            }
            LinkTarget::File(path) => self.open_pinned_file(path, None, cx),
            LinkTarget::None => {}
        }
    }
}
