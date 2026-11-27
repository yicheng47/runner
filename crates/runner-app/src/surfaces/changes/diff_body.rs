use super::*;

pub(crate) enum LoadedDiffKind {
    Unified(Entity<UnifiedDiffView>),
    Image {
        canvas: Entity<ImageCanvas>,
        one_sided: bool,
    },
    Svg {
        canvas: Entity<ImageCanvas>,
        unified: Entity<UnifiedDiffView>,
        source: bool,
        one_sided: bool,
    },
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) enum PreparedDiffBody {
    Unified {
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: Vec<DiffHunk>,
    },
    Image {
        sides: DiffSides,
    },
    Svg {
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: Vec<DiffHunk>,
        sides: DiffSides,
    },
    CrlfOnly,
    Binary {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    TooLarge {
        old_size: Option<u64>,
        new_size: Option<u64>,
    },
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Submodule {
        old_commit: Option<String>,
        new_commit: Option<String>,
    },
}

pub(crate) fn prepare_diff_body(diff: &ChangesDiff) -> PreparedDiffBody {
    let name = model::file_name(&diff.path).to_owned();
    let decode_raster_side = |caption: DiffCaptionKind,
                              format: gpui::ImageFormat,
                              bytes: &[u8],
                              fallback_size: Option<u64>| {
        let size = fallback_size.unwrap_or(bytes.len() as u64);
        let state = match image::decode_raster_image(format, bytes, size) {
            DecodeOutcome::Ready(decoded) => DiffSideState::Ready(decoded),
            DecodeOutcome::TooManyPixels {
                width,
                height,
                size,
            } => DiffSideState::TooManyPixels {
                width,
                height,
                size,
            },
            DecodeOutcome::Failed => DiffSideState::Failed,
        };
        DiffSide {
            caption,
            file_name: name.clone(),
            state,
        }
    };
    match &diff.body {
        DiffBody::Text { old, new, hunks } => {
            if markdown::image_format(&diff.path).is_some_and(|fmt| fmt != gpui::ImageFormat::Svg) {
                let old_present = diff.old_blob_id.is_some() || !old.0.is_empty();
                let new_present = diff.new_blob_id.is_some() || !new.0.is_empty();
                let old_magic = old_present
                    .then(|| image::detect_raster_magic(&old.0))
                    .flatten();
                let new_magic = new_present
                    .then(|| image::detect_raster_magic(&new.0))
                    .flatten();
                if old_magic.is_some() || new_magic.is_some() {
                    let decode_text_raster_side =
                        |caption: DiffCaptionKind,
                         bytes: &[u8],
                         detected: Option<gpui::ImageFormat>| {
                            if let Some(fmt) = detected {
                                decode_raster_side(caption, fmt, bytes, None)
                            } else {
                                DiffSide {
                                    caption,
                                    file_name: name.clone(),
                                    state: DiffSideState::Failed,
                                }
                            }
                        };
                    let sides = match (old_present, new_present) {
                        (false, true) => DiffSides::One(Box::new(decode_text_raster_side(
                            DiffCaptionKind::Added,
                            &new.0,
                            new_magic,
                        ))),
                        (true, false) => DiffSides::One(Box::new(decode_text_raster_side(
                            DiffCaptionKind::Deleted,
                            &old.0,
                            old_magic,
                        ))),
                        _ => DiffSides::Two {
                            before: Box::new(decode_text_raster_side(
                                DiffCaptionKind::Before,
                                &old.0,
                                old_magic,
                            )),
                            after: Box::new(decode_text_raster_side(
                                DiffCaptionKind::After,
                                &new.0,
                                new_magic,
                            )),
                        },
                    };
                    // Match file-tab behavior: when neither side is an actual image (nor hits
                    // pixel limits) and every present side is valid UTF-8 (e.g. plain text
                    // beginning with "BM..."), keep the normal text line diff.
                    let all_valid_utf8 = (!old_present || std::str::from_utf8(&old.0).is_ok())
                        && (!new_present || std::str::from_utf8(&new.0).is_ok());
                    if sides.has_drawable() || sides.has_too_many_pixels() || !all_valid_utf8 {
                        return PreparedDiffBody::Image { sides };
                    }
                }
            }
            if is_crlf_only_change(&old.0, &new.0, diff.mode_change.as_ref(), diff.conflicted) {
                PreparedDiffBody::CrlfOnly
            } else {
                let old_doc = Arc::new(Document::decode(&old.0));
                let new_doc = Arc::new(Document::decode(&new.0));
                if markdown::image_format(&diff.path) == Some(gpui::ImageFormat::Svg) {
                    let decode_svg_side = |caption: DiffCaptionKind, bytes: &[u8]| DiffSide {
                        caption,
                        file_name: name.clone(),
                        state: match image::decode_svg_image(bytes, bytes.len() as u64) {
                            DecodeOutcome::Ready(decoded) => DiffSideState::Ready(decoded),
                            DecodeOutcome::TooManyPixels {
                                width,
                                height,
                                size,
                            } => DiffSideState::TooManyPixels {
                                width,
                                height,
                                size,
                            },
                            DecodeOutcome::Failed => DiffSideState::Failed,
                        },
                    };
                    let sides = match (diff.old_blob_id.is_some(), diff.new_blob_id.is_some()) {
                        (false, true) => DiffSides::One(Box::new(decode_svg_side(
                            DiffCaptionKind::Added,
                            &new.0,
                        ))),
                        (true, false) => DiffSides::One(Box::new(decode_svg_side(
                            DiffCaptionKind::Deleted,
                            &old.0,
                        ))),
                        _ => DiffSides::Two {
                            before: Box::new(decode_svg_side(DiffCaptionKind::Before, &old.0)),
                            after: Box::new(decode_svg_side(DiffCaptionKind::After, &new.0)),
                        },
                    };
                    if sides.has_drawable() || sides.has_too_many_pixels() {
                        return PreparedDiffBody::Svg {
                            old_doc,
                            new_doc,
                            hunks: hunks.clone(),
                            sides,
                        };
                    }
                }
                PreparedDiffBody::Unified {
                    old_doc,
                    new_doc,
                    hunks: hunks.clone(),
                }
            }
        }
        DiffBody::Binary {
            old_size,
            new_size,
            old,
            new,
        } => {
            if (old.is_some() || new.is_some())
                && let Some(format) =
                    markdown::image_format(&diff.path).filter(|&fmt| fmt != gpui::ImageFormat::Svg)
            {
                let sides = match (old.as_ref(), new.as_ref()) {
                    (Some(old_b), Some(new_b)) => DiffSides::Two {
                        before: Box::new(decode_raster_side(
                            DiffCaptionKind::Before,
                            format,
                            &old_b.0,
                            *old_size,
                        )),
                        after: Box::new(decode_raster_side(
                            DiffCaptionKind::After,
                            format,
                            &new_b.0,
                            *new_size,
                        )),
                    },
                    (None, Some(new_b)) => DiffSides::One(Box::new(decode_raster_side(
                        DiffCaptionKind::Added,
                        format,
                        &new_b.0,
                        *new_size,
                    ))),
                    (Some(old_b), None) => DiffSides::One(Box::new(decode_raster_side(
                        DiffCaptionKind::Deleted,
                        format,
                        &old_b.0,
                        *old_size,
                    ))),
                    (None, None) => unreachable!(),
                };
                return PreparedDiffBody::Image { sides };
            }
            PreparedDiffBody::Binary {
                old_size: *old_size,
                new_size: *new_size,
            }
        }
        DiffBody::TooLarge { old_size, new_size } => PreparedDiffBody::TooLarge {
            old_size: *old_size,
            new_size: *new_size,
        },
        DiffBody::ModeOnly { old_mode, new_mode } => PreparedDiffBody::ModeOnly {
            old_mode: old_mode.clone(),
            new_mode: new_mode.clone(),
        },
        DiffBody::Submodule {
            old_commit,
            new_commit,
        } => PreparedDiffBody::Submodule {
            old_commit: old_commit.clone(),
            new_commit: new_commit.clone(),
        },
    }
}
