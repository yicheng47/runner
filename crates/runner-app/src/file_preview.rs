use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, img, px, AnyElement, ClipboardItem, Context, Entity, FocusHandle, Image, ImageFormat,
    KeyDownEvent, ListAlignment, ListOffset, ListState, Window,
};
use runner_app::workspace_files::{FilePreview, PreviewContent};

use crate::surfaces::mission_markdown::{
    render_plain_line, selected_plain_text, FeedMarkdown, FeedSelection, FeedSelectionHandler,
    FeedSelectionPhase,
};
use crate::{theme, Copy};

const IMAGE_WIDTH: f32 = 560.;

struct MarkdownSegment {
    source: String,
    feed: FeedMarkdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PreviewRow {
    Markdown { segment: usize, row: usize },
    Text { line: usize },
    MarkdownImage { image: usize },
    PreviewImage,
    Unsupported,
}

struct LocalMarkdownImage {
    source: PathBuf,
    alt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TextLine {
    text: String,
    source_range: std::ops::Range<usize>,
}

struct PreviewModel {
    rows: Vec<PreviewRow>,
    markdown: Vec<MarkdownSegment>,
    text: Vec<TextLine>,
    images: Vec<LocalMarkdownImage>,
    unsupported: Option<String>,
}

impl PreviewModel {
    fn new(preview: &FilePreview) -> Self {
        match &preview.content {
            PreviewContent::Markdown(source) => Self::from_markdown(source),
            PreviewContent::Text(source) => {
                let text = text_lines(source);
                let rows = (0..text.len())
                    .map(|line| PreviewRow::Text { line })
                    .collect();
                Self {
                    rows,
                    markdown: Vec::new(),
                    text,
                    images: Vec::new(),
                    unsupported: None,
                }
            }
            PreviewContent::Image => Self {
                rows: vec![PreviewRow::PreviewImage],
                markdown: Vec::new(),
                text: Vec::new(),
                images: Vec::new(),
                unsupported: None,
            },
            PreviewContent::Unsupported(message) => Self {
                rows: vec![PreviewRow::Unsupported],
                markdown: Vec::new(),
                text: Vec::new(),
                images: Vec::new(),
                unsupported: Some(message.clone()),
            },
        }
    }

    fn from_markdown(source: &str) -> Self {
        let mut markdown = Vec::<MarkdownSegment>::new();
        let mut images = Vec::<LocalMarkdownImage>::new();
        let mut rows = Vec::<PreviewRow>::new();
        let mut current = String::new();

        let flush_markdown = |current: &mut String,
                              markdown: &mut Vec<MarkdownSegment>,
                              rows: &mut Vec<PreviewRow>| {
            if current.is_empty() {
                return;
            }
            let source = std::mem::take(current);
            let segment = markdown.len();
            let feed = FeedMarkdown::new(&source);
            rows.extend((0..feed.row_count()).map(|row| PreviewRow::Markdown { segment, row }));
            markdown.push(MarkdownSegment { source, feed });
        };

        for line in source.split_inclusive('\n') {
            let without_newline = line.strip_suffix('\n').unwrap_or(line);
            if let Some((alt, path)) = standalone_relative_image(without_newline) {
                if current.ends_with('\n') {
                    current.pop();
                }
                flush_markdown(&mut current, &mut markdown, &mut rows);
                let image = images.len();
                images.push(LocalMarkdownImage { source: path, alt });
                rows.push(PreviewRow::MarkdownImage { image });
            } else {
                current.push_str(line);
            }
        }
        flush_markdown(&mut current, &mut markdown, &mut rows);

        // Empty markdown still gets a scrollable row, matching FeedMarkdown's empty state.
        if rows.is_empty() {
            markdown.push(MarkdownSegment {
                source: source.to_owned(),
                feed: FeedMarkdown::new(source),
            });
            rows.push(PreviewRow::Markdown { segment: 0, row: 0 });
        }
        Self {
            rows,
            markdown,
            text: Vec::new(),
            images,
            unsupported: None,
        }
    }

    fn row_count(&self) -> usize {
        self.rows.len().max(1)
    }
}

fn text_lines(source: &str) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = source.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch != '\n' && ch != '\r' {
            continue;
        }
        lines.push(TextLine {
            text: source[start..index].to_owned(),
            source_range: start..index,
        });
        start = if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            let (newline_index, _) = chars.next().expect("peeked line feed");
            newline_index + 1
        } else {
            index + ch.len_utf8()
        };
    }
    if start < source.len() || lines.is_empty() || start == source.len() {
        lines.push(TextLine {
            text: source[start..].to_owned(),
            source_range: start..source.len(),
        });
    }
    lines
}

fn clamp_scroll_to_rows(mut offset: ListOffset, row_count: usize) -> ListOffset {
    offset.item_ix = offset.item_ix.min(row_count.saturating_sub(1));
    offset
}

fn clamped_text_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn local_text_selection(
    selection: Option<&FeedSelection>,
    line: usize,
    text: &str,
    event_id: &str,
) -> Option<FeedSelection> {
    let selection = selection.filter(|selection| selection.event_id == "preview-text")?;
    let (start, end) = if selection.anchor <= selection.head {
        (selection.anchor, selection.head)
    } else {
        (selection.head, selection.anchor)
    };
    if line < start.0 || line > end.0 {
        return None;
    }
    let offset_for = |point: (usize, usize)| {
        if point.0 < line {
            0
        } else if point.0 > line {
            text.len()
        } else {
            clamped_text_offset(text, point.1)
        }
    };
    Some(FeedSelection {
        event_id: event_id.to_owned(),
        anchor: (0, offset_for(selection.anchor)),
        head: (0, offset_for(selection.head)),
    })
}

fn selected_original_text(
    source: &str,
    lines: &[TextLine],
    selection: &FeedSelection,
) -> Option<String> {
    if selection.event_id != "preview-text" {
        return None;
    }
    let (start, end) = if selection.anchor <= selection.head {
        (selection.anchor, selection.head)
    } else {
        (selection.head, selection.anchor)
    };
    if start == end {
        return None;
    }
    let start_line = lines.get(start.0)?;
    let end_line = lines.get(end.0)?;
    let start = start_line.source_range.start + clamped_text_offset(&start_line.text, start.1);
    let end = end_line.source_range.start + clamped_text_offset(&end_line.text, end.1);
    source.get(start..end).map(str::to_owned)
}

fn standalone_relative_image(line: &str) -> Option<(String, PathBuf)> {
    let line = line.trim();
    let content = line.strip_prefix("![")?;
    let alt_end = content.find("](")?;
    let alt = content[..alt_end].to_owned();
    let destination = content[alt_end + 2..].strip_suffix(')')?.trim();
    if destination.is_empty()
        || destination.starts_with('/')
        || destination.starts_with("//")
        || destination.starts_with('#')
        || destination.contains("://")
        || destination.starts_with("data:")
        || destination.contains('?')
        || destination.contains('#')
    {
        return None;
    }
    let path = Path::new(destination);
    if path.is_absolute() {
        return None;
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    ) {
        return None;
    }
    Some((alt, path.to_path_buf()))
}

fn image_format(path: &Path) -> Option<ImageFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "gif" => Some(ImageFormat::Gif),
        "webp" => Some(ImageFormat::Webp),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        _ => None,
    }
}

fn read_local_image(root: &Path, path: &Path) -> Result<Image, String> {
    let format = image_format(path).ok_or_else(|| "Unsupported image format".to_owned())?;
    let bytes = runner_app::workspace_files::read_image_bytes(root, path)?;
    Ok(Image::from_bytes(format, bytes))
}

fn zoom_factor(current: f32, zoom_in: bool) -> f32 {
    let next = if zoom_in {
        current * 1.25
    } else {
        current / 1.25
    };
    next.clamp(0.2, 4.)
}

fn zoomed_image_width(viewport_width: f32, zoom_factor: f32) -> f32 {
    viewport_width.max(1.) * zoom_factor.clamp(0.2, 4.)
}

pub(crate) struct FilePreviewView {
    root: PathBuf,
    preview: FilePreview,
    model: PreviewModel,
    list: ListState,
    selection: Option<FeedSelection>,
    selecting: bool,
    focus: FocusHandle,
    image_loads: HashMap<usize, Result<Image, String>>,
    image_loading: HashSet<usize>,
    image_zoom: f32,
    image_fit: bool,
    viewport_width: f32,
    image_generation: u64,
}

impl FilePreviewView {
    pub(crate) fn new(root: PathBuf, preview: FilePreview, cx: &mut Context<Self>) -> Self {
        let model = PreviewModel::new(&preview);
        let list = ListState::new(model.row_count(), ListAlignment::Top, px(160.));
        Self {
            root,
            preview,
            model,
            list,
            selection: None,
            selecting: false,
            focus: cx.focus_handle(),
            image_loads: HashMap::new(),
            image_loading: HashSet::new(),
            image_zoom: 1.,
            image_fit: true,
            viewport_width: 0.,
            image_generation: 0,
        }
    }

    pub(crate) fn replace(&mut self, preview: FilePreview, cx: &mut Context<Self>) {
        let old_count = self.model.row_count();
        let same_path = self.preview.path == preview.path;
        let mut scroll = self.list.logical_scroll_top();
        let model = PreviewModel::new(&preview);
        scroll = clamp_scroll_to_rows(scroll, model.row_count());
        self.list.splice(0..old_count, model.row_count());
        self.list.scroll_to(scroll);
        self.preview = preview;
        self.model = model;
        self.selection = None;
        self.selecting = false;
        self.image_loads.clear();
        self.image_loading.clear();
        self.image_generation = self.image_generation.wrapping_add(1);
        if !same_path {
            self.image_zoom = 1.;
            self.image_fit = true;
        }
        cx.notify();
    }

    fn handle_selection(
        &mut self,
        phase: FeedSelectionPhase,
        event_id: &str,
        position: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match phase {
            FeedSelectionPhase::Begin => {
                self.selection = Some(FeedSelection {
                    event_id: event_id.to_owned(),
                    anchor: position,
                    head: position,
                });
                self.selecting = true;
                window.focus(&self.focus, cx);
            }
            FeedSelectionPhase::Extend if self.selecting => {
                if let Some(selection) = self.selection.as_mut() {
                    if selection.event_id == event_id {
                        selection.head = position;
                    }
                }
            }
            FeedSelectionPhase::End => {
                if self.selecting {
                    if let Some(selection) = self.selection.as_mut() {
                        if selection.event_id == event_id {
                            selection.head = position;
                        }
                    }
                }
                self.selecting = false;
            }
            FeedSelectionPhase::Extend => {}
        }
        cx.notify();
    }

    fn selection_handler(&self, cx: &Context<Self>) -> FeedSelectionHandler {
        let weak = cx.entity().downgrade();
        FeedSelectionHandler::new(move |phase, event_id, position, window, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.handle_selection(phase, event_id, position, window, cx);
            });
        })
    }

    fn text_selection_handler(&self, line: usize, cx: &Context<Self>) -> FeedSelectionHandler {
        let weak = cx.entity().downgrade();
        FeedSelectionHandler::new(move |phase, _, position, window, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.handle_selection(phase, "preview-text", (line, position.1), window, cx);
            });
        })
    }

    fn start_image_load(&mut self, key: usize, path: PathBuf, cx: &mut Context<Self>) {
        if self.image_loading.contains(&key) || self.image_loads.contains_key(&key) {
            return;
        }
        self.image_loading.insert(key);
        let root = self.root.clone();
        let generation = self.image_generation;
        cx.spawn(async move |weak, cx| {
            let task = cx
                .background_executor()
                .spawn(async move { read_local_image(&root, &path) });
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.image_generation != generation {
                    return;
                }
                this.image_loading.remove(&key);
                this.image_loads.insert(key, result);
                cx.notify();
            });
        })
        .detach();
    }

    fn image_for_row(&mut self, row: &PreviewRow, cx: &mut Context<Self>) -> Option<Image> {
        let (key, path) = match row {
            PreviewRow::PreviewImage => (0, self.preview.path.clone()),
            PreviewRow::MarkdownImage { image } => {
                let source = self.model.images.get(*image)?.source.clone();
                let parent = self.preview.path.parent().unwrap_or_else(|| Path::new(""));
                (image + 1, parent.join(source))
            }
            _ => return None,
        };
        self.start_image_load(key, path, cx);
        self.image_loads
            .get(&key)
            .and_then(|result| result.as_ref().ok())
            .cloned()
    }

    fn image_error_for_row(&self, row: &PreviewRow) -> Option<&str> {
        let key = match row {
            PreviewRow::PreviewImage => 0,
            PreviewRow::MarkdownImage { image } => image + 1,
            _ => return None,
        };
        self.image_loads
            .get(&key)
            .and_then(|result| result.as_ref().err())
            .map(String::as_str)
    }

    fn copy_selection(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection.as_ref() else {
            return;
        };
        let copied = if selection.event_id == "preview-text" {
            match &self.preview.content {
                PreviewContent::Text(source) => {
                    selected_original_text(source, &self.model.text, selection)
                }
                _ => None,
            }
        } else if let Some(index) = selection.event_id.strip_prefix("preview-markdown-") {
            index
                .parse::<usize>()
                .ok()
                .and_then(|segment| self.model.markdown.get(segment))
                .and_then(|segment| selected_plain_text(&segment.source, selection))
        } else {
            None
        };
        let Some(text) = copied else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.stop_propagation();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.selection.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn set_zoom(&mut self, zoom_factor: f32, cx: &mut Context<Self>) {
        self.image_fit = false;
        self.image_zoom = zoom_factor.clamp(0.2, 4.);
        cx.notify();
    }

    fn render_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.model.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        match row {
            PreviewRow::Markdown { segment, row } => {
                let event_id = format!("preview-markdown-{segment}");
                let mut selection_color = theme::accent();
                selection_color.a = 0.28;
                let handler = self.selection_handler(cx);
                let Some(markdown) = self.model.markdown.get(segment) else {
                    return div().into_any_element();
                };
                let content = markdown.feed.render_row(
                    row,
                    &event_id,
                    cx.entity_id(),
                    self.selection.as_ref(),
                    selection_color,
                    &handler,
                    cx,
                );
                // A list row is a layout root; constrain the reused flex-based renderer.
                div()
                    .w_full()
                    .min_w(px(0.))
                    .child(content)
                    .into_any_element()
            }
            PreviewRow::Text { line } => {
                let source = self
                    .model
                    .text
                    .get(line)
                    .map_or("", |line| line.text.as_str());
                let event_id = format!("preview-text-line-{line}");
                let mut selection_color = theme::accent();
                selection_color.a = 0.28;
                let handler = self.text_selection_handler(line, cx);
                let local_selection =
                    local_text_selection(self.selection.as_ref(), line, source, &event_id);
                let content = render_plain_line(
                    format!("{event_id}-plain"),
                    &event_id,
                    0,
                    source.to_owned(),
                    cx.entity_id(),
                    local_selection.as_ref(),
                    selection_color,
                    Some(&handler),
                );
                div()
                    .w_full()
                    .min_w(px(0.))
                    .flex()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_ui())
                    .line_height(gpui::rems(20. / 16.))
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .text_right()
                            .text_color(theme::faint())
                            .text_size(theme::text_ui())
                            .child(format!("{}", line + 1)),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!(
                                "preview-text-scroll-{line}"
                            )))
                            .min_w(px(0.))
                            .flex_1()
                            .overflow_x_scroll()
                            .child(content),
                    )
                    .into_any_element()
            }
            image_row @ (PreviewRow::MarkdownImage { .. } | PreviewRow::PreviewImage) => {
                let image = self.image_for_row(&image_row, cx);
                if let Some(image) = image {
                    let zoom = self.image_zoom;
                    let fit = self.image_fit;
                    let alt = match &image_row {
                        PreviewRow::MarkdownImage { image } => {
                            self.model.images.get(*image).map(|image| image.alt.clone())
                        }
                        _ => None,
                    };
                    let viewport_width = self.viewport_width;
                    let fit_width = if viewport_width > 1. {
                        viewport_width
                    } else {
                        IMAGE_WIDTH
                    };
                    let image_width = zoomed_image_width(fit_width, zoom);
                    div()
                        .id("preview-image-viewport")
                        .debug_selector(|| "PREVIEW_IMAGE_VIEWPORT".into())
                        .w_full()
                        .min_w(px(0.))
                        .overflow_x_scroll()
                        .py_2()
                        .child(
                            div()
                                .when(fit, |content| content.w_full())
                                .when(!fit, |content| content.w(px(image_width)))
                                .flex()
                                .flex_col()
                                .items_center()
                                .child(
                                    img(Arc::new(image))
                                        .when(fit, |image| image.w_full().max_w_full())
                                        .when(!fit, |image| image.w(px(image_width))),
                                )
                                .children(alt.map(|alt| {
                                    div()
                                        .pt_1()
                                        .text_size(theme::text_ui())
                                        .text_color(theme::muted())
                                        .child(alt)
                                })),
                        )
                        .into_any_element()
                } else if self.image_error_for_row(&image_row).is_some() {
                    div()
                        .px_3()
                        .py_2()
                        .text_color(theme::muted())
                        .child(crate::i18n::text("panel.preview.image_load_failed"))
                        .into_any_element()
                } else {
                    div()
                        .px_3()
                        .py_2()
                        .text_color(theme::faint())
                        .child(crate::i18n::text("panel.preview.loading_image"))
                        .into_any_element()
                }
            }
            PreviewRow::Unsupported => div()
                .w_full()
                .px_4()
                .py_3()
                .text_color(theme::muted())
                .child(crate::i18n::text("panel.preview.unsupported"))
                .when_some(self.model.unsupported.as_ref(), |element, message| {
                    element.child(
                        div()
                            .pt_1()
                            .text_color(theme::faint())
                            .child(message.clone()),
                    )
                })
                .into_any_element(),
        }
    }

    fn render_image_toolbar(&self, entity: Entity<Self>) -> AnyElement {
        div()
            .w_full()
            .flex_none()
            .px_2()
            .py_1()
            .gap_1()
            .flex()
            .justify_end()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .id("preview-zoom-out")
                    .cursor_pointer()
                    .px_2()
                    .text_color(theme::muted())
                    .child(crate::i18n::text("panel.preview.zoom_out"))
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_zoom(zoom_factor(this.image_zoom, false), cx)
                            });
                        }
                    }),
            )
            .child(
                div()
                    .id("preview-zoom-in")
                    .cursor_pointer()
                    .px_2()
                    .text_color(theme::muted())
                    .child(crate::i18n::text("panel.preview.zoom_in"))
                    .on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.set_zoom(zoom_factor(this.image_zoom, true), cx)
                            });
                        }
                    }),
            )
            .child(
                div()
                    .id("preview-fit")
                    .cursor_pointer()
                    .px_2()
                    .text_color(theme::muted())
                    .child(crate::i18n::text("panel.preview.fit"))
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.image_fit = true;
                            this.image_zoom = 1.;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }
}

impl Render for FilePreviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Row layout already holds ListState's mutable borrow, so sample outside it.
        self.viewport_width = f32::from(self.list.viewport_bounds().size.width);
        let entity = cx.entity();
        let image_preview = matches!(&self.preview.content, PreviewContent::Image);
        let markdown_preview = matches!(&self.preview.content, PreviewContent::Markdown(_));
        let list = self.list.clone();
        div()
            .id("file-preview")
            .key_context("FilePreview")
            .track_focus(&self.focus)
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .on_action({
                let entity = entity.clone();
                move |action: &Copy, window, cx| {
                    entity.update(cx, |this, cx| this.copy_selection(action, window, cx));
                }
            })
            .on_key_down({
                let entity = entity.clone();
                move |event: &KeyDownEvent, window, cx| {
                    entity.update(cx, |this, cx| this.on_key_down(event, window, cx));
                }
            })
            .children(image_preview.then(|| self.render_image_toolbar(entity.clone())))
            .child(
                div()
                    .id("file-preview-content")
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .when(markdown_preview, |viewport| viewport.px_4())
                    .child(
                        gpui::list(list, move |index, _window, cx| {
                            entity.update(cx, |this, cx| this.render_row(index, cx))
                        })
                        .min_h(px(0.))
                        .min_w(px(0.))
                        .flex_1()
                        .w_full(),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size, AppContext, TestAppContext, VisualTestContext};
    use std::fs;

    struct PreviewVisualHost {
        preview: Entity<FilePreviewView>,
        width: f32,
    }

    impl Render for PreviewVisualHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("preview-visual-host")
                .debug_selector(|| "PREVIEW_HOST".into())
                .w(px(self.width))
                .h(px(320.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .min_h(px(0.))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(self.preview.clone().into_any_element()),
                )
        }
    }

    fn preview(content: PreviewContent) -> FilePreview {
        FilePreview {
            path: PathBuf::from("/workspace/readme.md"),
            bytes: 0,
            content,
        }
    }

    #[test]
    fn markdown_preview_list_and_rendered_content_have_visible_height() {
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, cx| {
            let preview = cx.new(|cx| {
                FilePreviewView::new(
                    PathBuf::from("/workspace"),
                    FilePreview {
                        path: PathBuf::from("/workspace/readme.md"),
                        bytes: 18,
                        content: PreviewContent::Markdown(
                            "# Preview title\n\nVisible paragraph content.".into(),
                        ),
                    },
                    cx,
                )
            });
            PreviewVisualHost {
                preview,
                width: 480.,
            }
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(640.), px(480.)));
        visual.run_until_parked();

        let (viewport_height, markdown_height) = host
            .read_with(&visual, |host, cx| {
                host.preview.read_with(cx, |preview, _| {
                    (
                        f32::from(preview.list.viewport_bounds().size.height),
                        preview
                            .list
                            .bounds_for_item(0)
                            .map(|bounds| f32::from(bounds.size.height)),
                    )
                })
            })
            .unwrap();
        assert!(
            viewport_height > 100.,
            "list viewport height {viewport_height}"
        );
        assert!(
            markdown_height.is_some_and(|height| height > 0.),
            "rendered markdown row bounds {markdown_height:?}"
        );
        let markdown_block = visual
            .debug_bounds("MISSION_MARKDOWN_BLOCK_0")
            .expect("rendered Markdown block should be in the fixed preview host");
        assert!(
            markdown_block.size.height > px(0.),
            "Markdown block bounds {markdown_block:?}"
        );
    }

    #[test]
    fn plain_text_fences_are_literal_content_with_compact_rows() {
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, cx| {
            let preview = cx.new(|cx| {
                FilePreviewView::new(
                    PathBuf::from("/workspace"),
                    preview(PreviewContent::Text("```\n# not a heading\n````".into())),
                    cx,
                )
            });
            PreviewVisualHost {
                preview,
                width: 480.,
            }
        });
        let visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        host.read_with(&visual, |host, cx| {
            let preview = host.preview.read(cx);
            assert_eq!(preview.model.text[0].text, "```");
            assert_eq!(preview.model.text[1].text, "# not a heading");
            let row = preview.list.bounds_for_item(0).unwrap();
            assert!(
                row.size.height > px(0.) && row.size.height <= px(24.),
                "{row:?}"
            );
        })
        .unwrap();
    }

    #[test]
    fn markdown_preview_has_balanced_side_margins_at_narrow_and_wide_widths() {
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, cx| {
            let preview = cx.new(|cx| FilePreviewView::new(
                PathBuf::from("/workspace"), preview(PreviewContent::Markdown(
                    "# Readable document\n\nA paragraph with **formatting** and enough words to wrap in a narrow panel.\n\n| A | B |\n| --- | --- |\n| 1 | 2 |".into())), cx));
            PreviewVisualHost { preview, width: 480. }
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1000.), px(600.)));
        for width in [240., 480., 720.] {
            host.update(&mut visual, |host, _, cx| {
                host.width = width;
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            let host_bounds = visual.debug_bounds("PREVIEW_HOST").unwrap();
            let heading = visual.debug_bounds("MISSION_MARKDOWN_BLOCK_0").unwrap();
            assert_eq!(
                heading.left(),
                host_bounds.left() + px(16.),
                "width={width}"
            );
            assert_eq!(
                heading.right(),
                host_bounds.right() - px(16.),
                "width={width}"
            );
        }
    }

    #[test]
    fn raster_preview_loads_without_borrowing_the_list_during_row_layout() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image.png");
        fs::write(&path, include_bytes!("../../../assets/icon.png")).unwrap();
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, cx| {
            let preview = cx.new(|cx| {
                FilePreviewView::new(
                    temp.path().to_path_buf(),
                    FilePreview {
                        path: path.clone(),
                        bytes: 1,
                        content: PreviewContent::Image,
                    },
                    cx,
                )
            });
            PreviewVisualHost {
                preview,
                width: 480.,
            }
        });
        let visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        host.read_with(&visual, |host, cx| {
            let preview = host.preview.read(cx);
            assert!(matches!(preview.image_loads.get(&0), Some(Ok(_))));
            assert!(preview.list.viewport_bounds().size.height > px(100.));
        })
        .unwrap();
    }

    #[test]
    fn replacing_preview_clamps_scroll_anchor_after_row_count_shrinks() {
        let offset = ListOffset {
            item_ix: 40,
            offset_in_item: px(12.),
        };
        let restored = clamp_scroll_to_rows(offset, 3);
        assert_eq!(restored.item_ix, 2);
        assert_eq!(restored.offset_in_item, px(12.));
    }

    #[test]
    fn zoom_steps_are_relative_to_the_current_fit_width() {
        for viewport_width in [240., 720.] {
            let enlarged_factor = zoom_factor(1., true);
            let reduced_factor = zoom_factor(1., false);
            assert!(zoomed_image_width(viewport_width, enlarged_factor) > viewport_width);
            assert!(zoomed_image_width(viewport_width, reduced_factor) < viewport_width);
        }
        assert_eq!(zoom_factor(1., true), 1.25);
        assert_eq!(zoom_factor(1., false), 0.8);
    }

    #[test]
    fn zoomed_image_width_is_clamped_to_a_reasonable_scale_without_clipping() {
        assert_eq!(zoomed_image_width(240., 20.), 240. * 4.);
        assert_eq!(zoomed_image_width(720., 0.01), 720. * 0.2);
    }

    #[test]
    fn text_preview_rows_keep_line_numbers_and_empty_documents_visible() {
        let model = PreviewModel::new(&preview(PreviewContent::Text("one\ntwo\n".into())));
        assert_eq!(
            model
                .text
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", ""]
        );
        assert_eq!(model.row_count(), 3);

        let empty = PreviewModel::new(&preview(PreviewContent::Text(String::new())));
        assert_eq!(
            empty
                .text
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            [""]
        );
        assert_eq!(empty.row_count(), 1);
    }

    #[test]
    fn text_multiline_selection_copies_exact_utf8_and_trailing_line_breaks() {
        let source = "alpha\n世界é\r\nlast\n";
        let lines = text_lines(source);
        let selection = FeedSelection {
            event_id: "preview-text".into(),
            anchor: (0, 2),
            head: (3, 0),
        };
        assert_eq!(
            selected_original_text(source, &lines, &selection).as_deref(),
            Some("pha\n世界é\r\nlast\n")
        );
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1].text, "世界é");
    }

    #[test]
    fn text_selection_maps_global_line_positions_to_each_rendered_row() {
        let selection = FeedSelection {
            event_id: "preview-text".into(),
            anchor: (0, 1),
            head: (2, 2),
        };
        let middle = local_text_selection(Some(&selection), 1, "middle", "line-1").unwrap();
        assert_eq!(middle.event_id, "line-1");
        assert_eq!(middle.anchor, (0, 0));
        assert_eq!(middle.head, (0, "middle".len()));

        let last = local_text_selection(Some(&selection), 2, "ending", "line-2").unwrap();
        assert_eq!(last.anchor, (0, 0));
        assert_eq!(last.head, (0, 2));
    }

    #[test]
    fn markdown_preview_virtualizes_blocks_and_extracts_only_local_image_rows() {
        let source = format!(
            "{}\n\n![diagram](assets/diagram.png)\n\nlast",
            (0..30)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        );
        let model = PreviewModel::new(&preview(PreviewContent::Markdown(source)));
        assert_eq!(model.images.len(), 1);
        assert!(model.images[0].source.ends_with("assets/diagram.png"));
        assert!(model
            .rows
            .iter()
            .any(|row| matches!(row, PreviewRow::MarkdownImage { .. })));
        assert!(
            model
                .rows
                .iter()
                .filter(|row| matches!(row, PreviewRow::Markdown { .. }))
                .count()
                > 1
        );
    }

    #[test]
    fn markdown_image_references_reject_remote_absolute_and_non_image_destinations() {
        assert!(standalone_relative_image("![a](https://example.com/a.png)").is_none());
        assert!(standalone_relative_image("![a](/tmp/a.png)").is_none());
        assert!(standalone_relative_image("![a](data:image/png;base64,abc)").is_none());
        assert!(standalone_relative_image("![a](notes.txt)").is_none());
        assert_eq!(
            standalone_relative_image("  ![a](images/a.png)  ")
                .unwrap()
                .1,
            PathBuf::from("images/a.png")
        );
    }

    #[test]
    fn markdown_relative_images_resolve_from_the_preview_directory_within_root() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("runner-preview-{}-{nonce}", std::process::id()));
        let docs = root.join("docs");
        let assets = root.join("assets");
        fs::create_dir_all(&docs).unwrap();
        fs::create_dir_all(&assets).unwrap();
        let preview_path = docs.join("readme.md");
        fs::write(&preview_path, "").unwrap();
        let image_path = assets.join("diagram.png");
        fs::write(&image_path, b"test image bytes").unwrap();

        let (_, relative) = standalone_relative_image("![diagram](../assets/diagram.png)").unwrap();
        let resolved = runner_app::workspace_files::resolve_path(
            &root,
            &preview_path.parent().unwrap().join(relative),
        )
        .unwrap();
        assert_eq!(resolved, image_path.canonicalize().unwrap());

        let outside = root
            .parent()
            .unwrap()
            .join(format!("runner-preview-outside-{nonce}.png"));
        fs::write(&outside, b"outside").unwrap();
        let escaping = preview_path
            .parent()
            .unwrap()
            .join("../../runner-preview-outside-")
            .with_file_name(format!("runner-preview-outside-{nonce}.png"));
        assert!(runner_app::workspace_files::resolve_path(&root, &escaping).is_err());

        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_preview_has_one_readonly_fallback_row() {
        let model = PreviewModel::new(&preview(PreviewContent::Unsupported("binary data".into())));
        assert_eq!(model.rows.as_slice(), &[PreviewRow::Unsupported]);
        assert_eq!(model.unsupported.as_deref(), Some("binary data"));
    }
}
