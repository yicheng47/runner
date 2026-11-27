use super::*;

pub(super) enum Read {
    Text(
        Arc<Document>,
        Option<Rendered>,
        Option<DecodeOutcome>,
        Stamp,
        Option<u64>,
    ),
    Image(DecodedImage, Stamp, u64),
    ImageTooLarge {
        width: u32,
        height: u32,
        size: u64,
        stamp: Stamp,
        hash: u64,
    },
    ImageFailed {
        stamp: Stamp,
        hash: u64,
    },
    Unchanged(Option<Stamp>),
    Binary(u64, Option<Stamp>),
    TooLarge(u64),
    Missing,
}

fn stamp_filter(since: Option<Stamp>) -> Option<FileStamp> {
    since.and_then(|(size, modified)| {
        modified.map(|modified| FileStamp {
            size,
            modified: Some(modified),
        })
    })
}

pub(super) fn read_raster_file(
    client: &DaemonClient,
    root: &Path,
    relative: &str,
    format: gpui::ImageFormat,
    since: Option<Stamp>,
    prev_hash: Option<u64>,
) -> Result<Read, ClientError> {
    let content = if let Some((prev_size, prev_modified)) = since {
        match client.files_read(root, relative, false, stamp_filter(since))? {
            FileContent::Unchanged => return Ok(Read::Unchanged(None)),
            FileContent::Binary {
                size,
                modified: Some(modified),
                ..
            } if prev_size == size && prev_modified == Some(modified) => {
                return Ok(Read::Unchanged(None));
            }
            FileContent::Binary { .. } => client.files_read(root, relative, true, None)?,
            FileContent::Text {
                size,
                modified: Some(modified),
                ..
            } if prev_size == size && prev_modified == Some(modified) => {
                return Ok(Read::Unchanged(None));
            }
            other => other,
        }
    } else {
        client.files_read(root, relative, true, None)?
    };

    Ok(match content {
        FileContent::Unchanged => Read::Unchanged(None),
        FileContent::Binary {
            size,
            bytes: Some(bytes),
            modified,
        } => {
            let hash = image::bytes_hash(&bytes.0);
            let stamp = (size, modified);
            if prev_hash == Some(hash) {
                return Ok(Read::Unchanged(Some(stamp)));
            }
            match image::decode_raster_image(format, &bytes.0, size) {
                DecodeOutcome::Ready(decoded) => Read::Image(decoded, stamp, hash),
                DecodeOutcome::TooManyPixels {
                    width,
                    height,
                    size,
                } => Read::ImageTooLarge {
                    width,
                    height,
                    size,
                    stamp,
                    hash,
                },
                DecodeOutcome::Failed => Read::ImageFailed { stamp, hash },
            }
        }
        FileContent::Binary { size, modified, .. } => {
            Read::Binary(size, modified.map(|modified| (size, Some(modified))))
        }
        FileContent::Text {
            bytes,
            size,
            modified,
        } => {
            let hash = image::bytes_hash(&bytes.0);
            let stamp = (size, modified);
            if let Some(detected) = image::detect_raster_magic(&bytes.0) {
                if prev_hash == Some(hash) {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                match image::decode_raster_image(detected, &bytes.0, size) {
                    DecodeOutcome::Ready(decoded) => Read::Image(decoded, stamp, hash),
                    DecodeOutcome::TooManyPixels {
                        width,
                        height,
                        size,
                    } => Read::ImageTooLarge {
                        width,
                        height,
                        size,
                        stamp,
                        hash,
                    },
                    DecodeOutcome::Failed if std::str::from_utf8(&bytes.0).is_err() => {
                        Read::ImageFailed { stamp, hash }
                    }
                    DecodeOutcome::Failed => {
                        let document = Arc::new(Document::decode(&bytes.0));
                        Read::Text(document, None, None, stamp, None)
                    }
                }
            } else if std::str::from_utf8(&bytes.0).is_err() {
                if prev_hash == Some(hash) {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                Read::ImageFailed { stamp, hash }
            } else {
                let document = Arc::new(Document::decode(&bytes.0));
                Read::Text(document, None, None, stamp, None)
            }
        }
        FileContent::TooLarge { size } => Read::TooLarge(size),
        FileContent::Missing => Read::Missing,
    })
}

pub(super) fn read_text_or_svg_file(
    client: &DaemonClient,
    root: &Path,
    relative: &str,
    markdown: bool,
    svg: bool,
    since: Option<Stamp>,
    prev_hash: Option<u64>,
) -> Result<Read, ClientError> {
    let known_binary = matches!(
        crate::file_links::classify_file_link(Path::new(relative)),
        crate::file_links::FileLinkKind::Binary(_)
    );
    Ok(
        match client.files_read(root, relative, false, stamp_filter(since))? {
            FileContent::Unchanged => Read::Unchanged(None),
            FileContent::Text { size, modified, .. } if known_binary => {
                Read::Binary(size, modified.map(|modified| (size, Some(modified))))
            }
            FileContent::Binary { size, modified, .. } => {
                Read::Binary(size, modified.map(|modified| (size, Some(modified))))
            }
            FileContent::TooLarge { size } if known_binary => Read::Binary(size, None),
            FileContent::Text { size, modified, .. }
                if since == Some((size, modified)) && (!svg || modified.is_some()) =>
            {
                Read::Unchanged(None)
            }
            FileContent::Text {
                bytes,
                size,
                modified,
            } => {
                let stamp = (size, modified);
                let svg_hash = svg.then(|| image::bytes_hash(&bytes.0));
                if svg && prev_hash.is_some() && prev_hash == svg_hash {
                    return Ok(Read::Unchanged(Some(stamp)));
                }
                let document = Arc::new(Document::decode(&bytes.0));
                let rendered = markdown.then(|| {
                    let page = Page::parse(document.text(), relative);
                    let images = page
                        .images()
                        .into_iter()
                        .filter_map(|image| {
                            let decoded = read_image(client, root, &image)?;
                            Some((image, decoded))
                        })
                        .collect();
                    (Arc::new(page), images)
                });
                let svg_decoded = svg.then(|| image::decode_svg_image(&bytes.0, size));
                Read::Text(document, rendered, svg_decoded, stamp, svg_hash)
            }
            FileContent::TooLarge { size } => Read::TooLarge(size),
            FileContent::Missing => Read::Missing,
        },
    )
}

fn read_image(client: &DaemonClient, root: &Path, path: &str) -> Option<Arc<gpui::Image>> {
    let format = markdown::image_format(path)?;
    let bytes = match client.files_read(root, path, true, None).ok()? {
        FileContent::Text { bytes, .. } => bytes.0,
        FileContent::Binary {
            bytes: Some(bytes), ..
        } => bytes.0,
        _ => return None,
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

impl UnifiedViewer {
    // ── File Reading & Live Reloads ────────────────────────────────────

    pub(super) fn show_line(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Content::Text(view) = &doc.content else {
            return;
        };
        if let Some(line) = doc.line.take() {
            doc.pending_code_line = None;
            view.update(cx, |view, cx| view.go_to_line(line, cx));
        } else if let Some(line) = doc.pending_code_line.take() {
            view.update(cx, |view, cx| view.scroll_to_line(line, cx));
        }
        cx.notify();
    }

    pub(super) fn note_ignored_tabs(&mut self) {
        let Some(status) = self.status.as_ref() else {
            return;
        };
        for (key, doc) in &mut self.docs {
            if !key.is_outside() && status.status.is_ignored(&key.path) {
                doc.ignored = true;
            }
        }
    }

    pub(super) fn refresh_ignored(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(&key) else {
            return;
        };
        if (doc.ignored || key.is_outside())
            && !doc.reading
            && !matches!(doc.content, Content::Idle)
        {
            let stamp = doc.stamp;
            let prev_hash = (!matches!(doc.content, Content::Deleted(_)))
                .then_some(doc.image_hash)
                .flatten();
            self.read_file_since(key, stamp, prev_hash, cx);
        }
    }

    pub(super) fn refresh_ignored_tabs(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(IGNORED_REFRESH).await;
            let refreshed = this.update(cx, |this, cx| {
                let keys: Vec<TabKey> = this.tabs.tabs().iter().map(|t| t.key.clone()).collect();
                for key in keys {
                    this.refresh_ignored(key, cx);
                }
            });
            if refreshed.is_err() {
                break;
            }
        })
    }

    pub(super) fn read_again_after_resolve(&mut self, cx: &mut Context<Self>) {
        let waiting: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(_, doc)| doc.reading)
            .map(|(k, _)| k.clone())
            .collect();
        let waiting_diffs: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(_, doc)| {
                doc.diff_reading
                    || (doc.mode == DocMode::Diff
                        && matches!(doc.diff, FileTabDiff::Idle | FileTabDiff::Loading))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in waiting {
            self.read_file(key, cx);
        }
        for key in waiting_diffs {
            self.load_tab_diff(key, cx);
        }
    }

    pub(super) fn read_file(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let prev_hash = self.docs.get(&key).and_then(|doc| {
            (!matches!(doc.content, Content::Deleted(_)))
                .then_some(doc.image_hash)
                .flatten()
        });
        self.read_file_since(key, None, prev_hash, cx);
    }

    fn read_target(&self, key: &TabKey) -> Option<(PathBuf, String)> {
        if let Some(outside) = key.outside_path() {
            let parent = outside.parent()?.to_path_buf();
            let name = outside.file_name()?.to_str()?.to_owned();
            return Some((parent, name));
        }
        Some((self.root_path()?.to_path_buf(), key.path.clone()))
    }

    fn read_file_since(
        &mut self,
        key: TabKey,
        since: Option<Stamp>,
        prev_hash: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let Some((root, relative)) = self.read_target(&key) else {
            return;
        };
        self.next_read += 1;
        let read = self.next_read;
        if let Some(doc) = self.docs.get_mut(&key) {
            doc.read = read;
            doc.reading = true;
            if matches!(doc.content, Content::Idle) {
                doc.content = Content::Loading;
            }
        }
        let markdown = is_markdown(&key.path);
        let svg = is_svg(&key.path);
        let raster_format = raster_image_format(&key.path);
        self.request(
            cx,
            move |client| {
                if let Some(format) = raster_format {
                    return read_raster_file(client, &root, &relative, format, since, prev_hash);
                }
                read_text_or_svg_file(client, &root, &relative, markdown, svg, since, prev_hash)
            },
            move |this, result, cx| this.apply_read(key, read, result, cx),
        );
    }

    pub(super) fn apply_read(
        &mut self,
        key: TabKey,
        read: u64,
        result: Result<Read, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let open_line = self.open_line_handler(key.clone(), cx);
        let open_link = self.open_link_handler(cx);
        let active = self.tabs.active().is_some_and(|tab| tab.key == key);
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        if doc.read != read {
            return;
        }
        doc.reading = false;
        if let Ok(Read::Unchanged(new_stamp)) = &result {
            if let Some(stamp) = new_stamp {
                doc.stamp = Some(*stamp);
            }
            return;
        }
        let same = match (&result, &doc.content) {
            (Ok(Read::Missing), Content::Deleted(_)) => true,
            (Ok(Read::Binary(new, _)), Content::Binary { size })
            | (Ok(Read::TooLarge(new)), Content::TooLarge { size }) => new == size,
            _ => false,
        };
        if same {
            if let Ok(Read::Binary(_, stamp)) = &result {
                doc.stamp = *stamp;
            }
            return;
        }
        doc.stamp = match &result {
            Ok(Read::Text(_, _, _, stamp, _))
            | Ok(Read::Image(_, stamp, _))
            | Ok(Read::ImageTooLarge { stamp, .. })
            | Ok(Read::ImageFailed { stamp, .. }) => Some(*stamp),
            Ok(Read::Binary(_, stamp)) => *stamp,
            _ => None,
        };
        doc.image_hash = match &result {
            Ok(Read::Text(_, _, _, _, svg_hash)) => *svg_hash,
            Ok(Read::Image(_, _, hash))
            | Ok(Read::ImageTooLarge { hash, .. })
            | Ok(Read::ImageFailed { hash, .. }) => Some(*hash),
            _ => None,
        };
        let previous = std::mem::replace(&mut doc.content, Content::Loading);
        let (kept_text, kept_image) = match previous {
            Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => {
                (Some(view), None)
            }
            Content::Image {
                canvas,
                width,
                height,
                size,
            }
            | Content::Deleted(Some(DeletedContent::Image {
                canvas,
                width,
                height,
                size,
            })) => (None, Some((canvas, width, height, size))),
            _ => (None, None),
        };
        if let Ok(Read::Text(_, Some((page, images)), _, _, _)) = &result {
            let (page, images) = (page.clone(), images.clone());
            match &doc.rendered {
                Some(view) => view.update(cx, |view, cx| {
                    view.set_page(page, images, cx);
                    view.set_deleted(false, cx);
                }),
                None => {
                    let relative = if key.is_outside() {
                        key.name().to_owned()
                    } else {
                        key.path.clone()
                    };
                    doc.rendered = Some(cx.new(|cx| {
                        let mut view = MarkdownView::new(page, relative, images, cx);
                        view.set_open_link(open_link);
                        view
                    }));
                }
            }
        }
        if let (Ok(Read::Missing), Some(view)) = (&result, &doc.rendered) {
            view.update(cx, |view, cx| view.set_deleted(true, cx));
        }
        if let (Ok(Read::Missing), Some(canvas)) = (&result, &doc.svg_canvas) {
            canvas.update(cx, |canvas, cx| canvas.set_deleted(true, cx));
        }
        doc.working_tree_missing = match &result {
            Ok(
                Read::Text(..)
                | Read::Image(..)
                | Read::ImageTooLarge { .. }
                | Read::ImageFailed { .. }
                | Read::Binary(..)
                | Read::TooLarge(_),
            ) => Some(false),
            Ok(Read::Missing) => Some(true),
            _ => doc.working_tree_missing,
        };
        let drop_kept_image = |kept: Option<(Entity<ImageCanvas>, u32, u32, u64)>, cx: &mut App| {
            if let Some((canvas, ..)) = kept {
                canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(None, cx));
            }
        };
        let clear_svg_canvas = |doc: &mut OpenDocument, cx: &mut App| {
            if let Some(canvas) = doc.svg_canvas.take() {
                canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(None, cx));
            }
            doc.svg_too_large = None;
            doc.svg_meta = None;
        };
        doc.content = match result {
            Ok(Read::Text(document, _, svg_decoded, _, _)) => {
                drop_kept_image(kept_image, cx);
                match svg_decoded {
                    Some(DecodeOutcome::Ready(decoded)) => {
                        doc.svg_too_large = None;
                        doc.svg_meta = Some((decoded.width, decoded.height, decoded.size));
                        match &doc.svg_canvas {
                            Some(canvas) => canvas.update(cx, |canvas, cx| {
                                canvas.set_single_image(decoded, cx);
                                canvas.set_deleted(false, cx);
                            }),
                            None => {
                                doc.svg_canvas =
                                    Some(cx.new(|cx| ImageCanvas::new_single(decoded, cx)));
                            }
                        }
                        if std::mem::take(&mut doc.svg_invalid) {
                            doc.source = false;
                        }
                    }
                    Some(DecodeOutcome::TooManyPixels {
                        width,
                        height,
                        size,
                    }) => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_too_large = Some((width, height, size));
                        doc.svg_meta = Some((width, height, size));
                        if std::mem::take(&mut doc.svg_invalid) {
                            doc.source = false;
                        }
                    }
                    Some(DecodeOutcome::Failed) => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_invalid = true;
                        doc.source = true;
                    }
                    None => {
                        clear_svg_canvas(doc, cx);
                        doc.svg_invalid = false;
                    }
                }
                match kept_text {
                    Some(view) => {
                        view.update(cx, |view, cx| {
                            view.set_document(document, cx);
                            view.set_deleted(false, cx);
                        });
                        Content::Text(view)
                    }
                    None => Content::Text(cx.new(|cx| {
                        let mut view = CodeView::new(
                            document,
                            super::highlight::grammar_for_path(&key.path),
                            cx,
                        );
                        view.set_open_line(open_line);
                        view
                    })),
                }
            }
            Ok(Read::Image(decoded, ..)) => {
                clear_svg_canvas(doc, cx);
                let (width, height, size) = (decoded.width, decoded.height, decoded.size);
                let canvas = match kept_image {
                    Some((canvas, ..)) => {
                        canvas.update(cx, |canvas, cx| {
                            canvas.set_single_image(decoded, cx);
                            canvas.set_deleted(false, cx);
                        });
                        canvas
                    }
                    None => cx.new(|cx| ImageCanvas::new_single(decoded, cx)),
                };
                Content::Image {
                    canvas,
                    width,
                    height,
                    size,
                }
            }
            Ok(Read::ImageTooLarge {
                width,
                height,
                size,
                ..
            }) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::ImageTooLarge {
                    width,
                    height,
                    size,
                }
            }
            Ok(Read::ImageFailed { .. }) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::ImageFailed
            }
            Ok(Read::Unchanged(_)) => unreachable!(),
            Ok(Read::Binary(size, _)) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::Binary { size }
            }
            Ok(Read::TooLarge(size)) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::TooLarge { size }
            }
            Ok(Read::Missing) => {
                let kept = if let Some(view) = kept_text {
                    view.update(cx, |view, cx| view.set_deleted(true, cx));
                    Some(DeletedContent::Text(view))
                } else if let Some((canvas, width, height, size)) = kept_image {
                    canvas.update(cx, |canvas, cx| canvas.set_deleted(true, cx));
                    Some(DeletedContent::Image {
                        canvas,
                        width,
                        height,
                        size,
                    })
                } else {
                    None
                };
                if !doc.has_changes_target {
                    doc.mode = DocMode::Code;
                    doc.diff = FileTabDiff::Idle;
                    doc.diff_reading = false;
                }
                if active {
                    self.open_in_open = false;
                }
                Content::Deleted(kept)
            }
            Err(error) => {
                drop_kept_image(kept_image, cx);
                clear_svg_canvas(doc, cx);
                Content::Failed(error.to_string().into())
            }
        };
        self.show_line(&key, cx);
        if active {
            self.search(cx);
        }
        cx.notify();
    }

    pub(crate) fn reload_changed(
        &mut self,
        changed: Option<&FilesChanged>,
        cx: &mut Context<Self>,
    ) {
        let rescan = changed.is_none_or(|changed| changed.paths.is_empty());
        let keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .map(|t| t.key.clone())
            .filter(|key| {
                if key.is_outside() {
                    return changed.is_none();
                }
                rescan
                    || changed.is_some_and(|changed| {
                        changed.paths.iter().any(|changed| {
                            changed == &key.path
                                || key
                                    .path
                                    .strip_prefix(changed.as_str())
                                    .is_some_and(|rest| rest.starts_with('/'))
                        })
                    })
            })
            .collect();
        for key in keys {
            let (read_code, read_diff, load_hunks, reprobe_exists) =
                self.docs
                    .get(&key)
                    .map_or((false, false, false, false), |doc| {
                        let read_code = !matches!(doc.content, Content::Idle);
                        if key.is_outside() {
                            return (read_code, false, false, false);
                        }
                        let read_diff = doc.mode == DocMode::Diff
                            || (key.target == SelectedTarget::Uncommitted
                                && matches!(doc.diff, FileTabDiff::Loaded(_)));
                        let load_hunks = !read_diff
                            && doc.mode == DocMode::Code
                            && key.target == SelectedTarget::Uncommitted
                            && self.has_uncommitted_diff(&key.path) == Some(true);
                        let reprobe_exists = !read_code
                            && key.target != SelectedTarget::Uncommitted
                            && doc.working_tree_missing.is_some();
                        (read_code, read_diff, load_hunks, reprobe_exists)
                    });
            if read_code {
                self.read_file(key.clone(), cx);
            }
            if read_diff {
                self.load_tab_diff(key.clone(), cx);
            } else if load_hunks {
                self.load_code_hunks(key.clone(), cx);
            }
            if reprobe_exists {
                self.reprobe_working_tree_exists(key, cx);
            }
        }
    }
}
