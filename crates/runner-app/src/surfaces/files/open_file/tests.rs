use std::path::Path;
use std::sync::{Arc, Mutex};

use ::image::codecs::png::PngEncoder;
use ::image::{ExtendedColorType, ImageEncoder};
use runner_core::companion::TerminalBytes;
use runner_core::protocol::files::{FileContent, FileStamp};
use runner_core::protocol::{
    ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
};

use super::{format_size, image, read_raster_file, read_text_or_svg_file, Read};

fn test_png(rgba: [u8; 4]) -> Vec<u8> {
    let mut buf = Vec::new();
    let pixels = vec![rgba[0], rgba[1], rgba[2], rgba[3]]
        .into_iter()
        .cycle()
        .take(8 * 8 * 4)
        .collect::<Vec<_>>();
    PngEncoder::new(&mut buf)
        .write_image(&pixels, 8, 8, ExtendedColorType::Rgba8)
        .unwrap();
    buf
}

struct StubSub;
impl EventSubscription for StubSub {
    fn recv(
        &mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        runner_core::protocol::ClientEvent,
                        runner_core::protocol::EventError,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(runner_core::protocol::EventError::Closed) })
    }
}

struct FakeFilesTransport {
    state: Mutex<(Vec<u8>, Option<chrono::DateTime<chrono::Utc>>, bool)>,
    requests: Mutex<Vec<(Option<bool>, Option<FileStamp>)>>,
    bytes_transferred: Mutex<usize>,
}

impl Transport for FakeFilesTransport {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        match request {
            Request::files_read {
                want_bytes,
                if_unchanged,
                ..
            } => {
                self.requests
                    .lock()
                    .unwrap()
                    .push((want_bytes, if_unchanged));
                let (bytes, modified, is_text) = self.state.lock().unwrap().clone();
                let size = bytes.len() as u64;
                if let Some(expected) = if_unchanged
                    && expected.modified.is_some()
                    && expected.size == size
                    && expected.modified == modified
                {
                    return Ok(Response::files_read(Ok(FileContent::Unchanged)));
                }
                if is_text {
                    *self.bytes_transferred.lock().unwrap() += bytes.len();
                    return Ok(Response::files_read(Ok(FileContent::Text {
                        bytes: TerminalBytes(bytes),
                        size,
                        modified,
                    })));
                }
                let send = want_bytes.unwrap_or(false);
                if send {
                    *self.bytes_transferred.lock().unwrap() += bytes.len();
                }
                Ok(Response::files_read(Ok(FileContent::Binary {
                    size,
                    bytes: send.then_some(TerminalBytes(bytes)),
                    modified,
                })))
            }
            other => Err(ClientError::msg(format!("unexpected request: {other:?}"))),
        }
    }

    fn subscribe(&self) -> Box<dyn EventSubscription> {
        Box::new(StubSub)
    }
}

#[test]
fn sizes_read_as_people_write_them() {
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1_258_291), "1.2 MB");
    assert_eq!(format_size(224_395_264), "214 MB");
    assert_eq!(format_size(12 * 1024), "12 KB");
    assert_eq!(format_size(10 * 1024 * 1024), "10 MB");
}

#[test]
fn ignored_image_poll_skips_bytes_until_stamp_changes_and_handles_missing_modified() {
    let png_a = test_png([255, 0, 0, 255]);
    let png_b = test_png([0, 255, 0, 255]);
    assert_eq!(png_a.len(), png_b.len());
    let size = png_a.len() as u64;
    let t1: chrono::DateTime<chrono::Utc> = "2026-11-25T00:00:01Z".parse().unwrap();
    let t2: chrono::DateTime<chrono::Utc> = "2026-11-25T00:00:02Z".parse().unwrap();
    let t3: chrono::DateTime<chrono::Utc> = "2026-11-25T00:00:03Z".parse().unwrap();

    let transport = Arc::new(FakeFilesTransport {
        state: Mutex::new((png_a.clone(), Some(t1), false)),
        requests: Mutex::new(Vec::new()),
        bytes_transferred: Mutex::new(0),
    });
    let client = DaemonClient::new(transport.clone());

    // 1. Unchanged ignored PNG: metadata-only probe transfers 0 bytes and returns Unchanged(None).
    let hash_a = image::bytes_hash(&png_a);
    let unchanged = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some((size, Some(t1))),
        Some(hash_a),
    )
    .unwrap();
    assert!(matches!(unchanged, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);

    // 2. Touching an ignored PNG (new mtime, identical bytes): poll 1 fetches once and
    //    returns the persisted stamp; poll 2 with that stamp transfers 0 bytes.
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    *transport.state.lock().unwrap() = (png_a.clone(), Some(t2), false);
    let touched = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some((size, Some(t1))),
        Some(hash_a),
    )
    .unwrap();
    let persisted_stamp = match touched {
        Read::Unchanged(Some(stamp)) => stamp,
        _ => panic!("touched PNG must persist updated stamp without replacing canvas"),
    };
    assert_eq!(persisted_stamp, (size, Some(t2)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), png_a.len());

    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    let after_touch = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some(persisted_stamp),
        Some(hash_a),
    )
    .unwrap();
    assert!(matches!(after_touch, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);

    // 3. Same-size regeneration with newer modified timestamp: probes without bytes, then fetches bytes.
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    *transport.state.lock().unwrap() = (png_b.clone(), Some(t3), false);
    let reloaded = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some(persisted_stamp),
        Some(hash_a),
    )
    .unwrap();
    assert!(
        matches!(reloaded, Read::Image(decoded, (s, Some(m)), _) if s == size && m == t3 && decoded.width == 8)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), png_b.len());

    // 4. Daemon without `modified`: falls back to byte comparison and still reloads same-size changes.
    transport.requests.lock().unwrap().clear();
    let hash_b = image::bytes_hash(&png_b);
    *transport.state.lock().unwrap() = (png_b.clone(), None, false);
    let legacy_same = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some((size, None)),
        Some(hash_b),
    )
    .unwrap();
    assert!(matches!(legacy_same, Read::Unchanged(Some((s, None))) if s == size));

    *transport.state.lock().unwrap() = (png_a.clone(), None, false);
    let legacy_changed = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/shot.png",
        gpui::ImageFormat::Png,
        Some((size, None)),
        Some(hash_b),
    )
    .unwrap();
    assert!(matches!(legacy_changed, Read::Image(_, (s, None), h) if s == size && h == hash_a));

    // 5. Unchanged ignored SVG transfers 0 bytes via conditional probe; touched SVG persists stamp.
    let svg_bytes =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#0f0"/></svg>"##
            .to_vec();
    let svg_size = svg_bytes.len() as u64;
    let svg_hash = image::bytes_hash(&svg_bytes);
    *transport.state.lock().unwrap() = (svg_bytes.clone(), Some(t1), true);
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;

    let svg_unchanged = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/icon.svg",
        false,
        true,
        Some((svg_size, Some(t1))),
        Some(svg_hash),
    )
    .unwrap();
    assert!(matches!(svg_unchanged, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);

    *transport.state.lock().unwrap() = (svg_bytes.clone(), Some(t2), true);
    let svg_touched = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/icon.svg",
        false,
        true,
        Some((svg_size, Some(t1))),
        Some(svg_hash),
    )
    .unwrap();
    let svg_persisted = match svg_touched {
        Read::Unchanged(Some(stamp)) => stamp,
        _ => panic!("touched SVG must persist new stamp without replacing canvas"),
    };
    assert_eq!(svg_persisted, (svg_size, Some(t2)));
    assert_eq!(
        *transport.bytes_transferred.lock().unwrap(),
        svg_bytes.len()
    );

    *transport.bytes_transferred.lock().unwrap() = 0;
    let svg_after_touch = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/icon.svg",
        false,
        true,
        Some(svg_persisted),
        Some(svg_hash),
    )
    .unwrap();
    assert!(matches!(svg_after_touch, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);

    // 6. Missing `modified` on Text-classified SVG or raster still compares bytes and
    //    reloads equal-length edits instead of short-circuiting on `(size, None)`.
    let svg_v2 =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#f00"/></svg>"##
            .to_vec();
    assert_eq!(svg_v2.len(), svg_bytes.len());
    *transport.state.lock().unwrap() = (svg_bytes.clone(), None, true);
    let svg_no_mtime_same = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/icon.svg",
        false,
        true,
        Some((svg_size, None)),
        Some(svg_hash),
    )
    .unwrap();
    assert!(matches!(
        svg_no_mtime_same,
        Read::Unchanged(Some((s, None))) if s == svg_size
    ));

    *transport.state.lock().unwrap() = (svg_v2.clone(), None, true);
    let svg_no_mtime_changed = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/icon.svg",
        false,
        true,
        Some((svg_size, None)),
        Some(svg_hash),
    )
    .unwrap();
    assert!(matches!(
        svg_no_mtime_changed,
        Read::Text(_, None, Some(super::DecodeOutcome::Ready(_)), (s, None), Some(h))
            if s == svg_size && h == image::bytes_hash(&svg_v2)
    ));

    let make_text_jpg = |rgb: [u8; 3]| {
        use ::image::codecs::jpeg::JpegEncoder;
        let mut jpg = Vec::new();
        let pixels = vec![rgb[0], rgb[1], rgb[2]]
            .into_iter()
            .cycle()
            .take(8 * 8 * 3)
            .collect::<Vec<_>>();
        JpegEncoder::new(&mut jpg)
            .write_image(&pixels, 8, 8, ExtendedColorType::Rgb8)
            .unwrap();
        jpg
    };
    let mut jpg_a = make_text_jpg([255, 0, 0]);
    let mut jpg_b = make_text_jpg([0, 0, 255]);
    let max_len = jpg_a.len().max(jpg_b.len());
    jpg_a.resize(max_len, 0xff);
    jpg_b.resize(max_len, 0xff);
    let jpg_size = max_len as u64;
    let jpg_a_hash = image::bytes_hash(&jpg_a);
    *transport.state.lock().unwrap() = (jpg_b.clone(), None, true);
    let jpg_no_mtime_changed = read_raster_file(
        &client,
        Path::new("/repo"),
        "build/photo.jpg",
        gpui::ImageFormat::Jpeg,
        Some((jpg_size, None)),
        Some(jpg_a_hash),
    )
    .unwrap();
    assert!(matches!(
        jpg_no_mtime_changed,
        Read::Image(decoded, (s, None), h)
            if s == jpg_size && h == image::bytes_hash(&jpg_b) && decoded.width == 8
    ));

    // 7. Ordinary ignored Markdown without `modified` preserves the `(size, None)` fast path
    //    so it does not reparse or refetch embedded local images on every poll.
    let md_bytes = b"# Doc\n\n![shot](shot.png)\n".to_vec();
    let md_size = md_bytes.len() as u64;
    *transport.state.lock().unwrap() = (md_bytes, None, true);
    transport.requests.lock().unwrap().clear();
    let md_unchanged = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/README.md",
        true,
        false,
        Some((md_size, None)),
        None,
    )
    .unwrap();
    assert!(matches!(md_unchanged, Read::Unchanged(None)));
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        1,
        "unchanged Markdown must not refetch embedded local images"
    );
}

#[test]
fn nul_free_pdf_read_preserves_and_updates_stamp_for_conditional_refresh() {
    let pdf_v1 = b"%PDF-1.4\n1 0 obj\n(v1)\nendobj\n%%EOF\n".to_vec();
    let pdf_v2 = b"%PDF-1.4\n1 0 obj\n(v2)\nendobj\n%%EOF\n".to_vec();
    assert_eq!(pdf_v1.len(), pdf_v2.len());
    let size = pdf_v1.len() as u64;
    let t1: chrono::DateTime<chrono::Utc> = "2026-11-27T00:00:01Z".parse().unwrap();
    let t2: chrono::DateTime<chrono::Utc> = "2026-11-27T00:00:02Z".parse().unwrap();

    let transport = Arc::new(FakeFilesTransport {
        state: Mutex::new((pdf_v1, Some(t1), true)),
        requests: Mutex::new(Vec::new()),
        bytes_transferred: Mutex::new(0),
    });
    let client = DaemonClient::new(transport.clone());

    // 1. Initial read of a NUL-free PDF classifies as Binary and retains its stamp.
    let initial = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/report.pdf",
        false,
        false,
        None,
        None,
    )
    .unwrap();
    let initial_stamp = match initial {
        Read::Binary(got_size, Some(stamp)) => {
            assert_eq!(got_size, size);
            stamp
        }
        _ => panic!("expected Read::Binary with daemon stamp"),
    };
    assert_eq!(initial_stamp, (size, Some(t1)));

    // 2. Unchanged conditional refresh sends if_unchanged and transfers 0 bytes.
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    let unchanged = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/report.pdf",
        false,
        false,
        Some(initial_stamp),
        None,
    )
    .unwrap();
    assert!(matches!(unchanged, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![(
            None,
            Some(FileStamp {
                size,
                modified: Some(t1),
            })
        )]
    );

    // 3. Same-size rewrite with newer modified time returns Read::Binary with the updated stamp.
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    *transport.state.lock().unwrap() = (pdf_v2, Some(t2), true);
    let rewritten = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/report.pdf",
        false,
        false,
        Some(initial_stamp),
        None,
    )
    .unwrap();
    let updated_stamp = match rewritten {
        Read::Binary(got_size, Some(stamp)) => {
            assert_eq!(got_size, size);
            stamp
        }
        _ => panic!("expected Read::Binary with updated stamp after same-size rewrite"),
    };
    assert_eq!(updated_stamp, (size, Some(t2)));

    // 4. Subsequent refresh using the updated stamp short-circuits with 0 bytes transferred.
    transport.requests.lock().unwrap().clear();
    *transport.bytes_transferred.lock().unwrap() = 0;
    let after_rewrite = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/report.pdf",
        false,
        false,
        Some(updated_stamp),
        None,
    )
    .unwrap();
    assert!(matches!(after_rewrite, Read::Unchanged(None)));
    assert_eq!(*transport.bytes_transferred.lock().unwrap(), 0);
}

#[test]
fn same_size_text_and_binary_transitions_without_mtime_are_not_treated_as_unchanged() {
    let text_bytes = b"abcd".to_vec();
    let binary_bytes = b"a\0cd".to_vec();
    assert_eq!(text_bytes.len(), binary_bytes.len());
    let size = text_bytes.len() as u64;

    let transport = Arc::new(FakeFilesTransport {
        state: Mutex::new((binary_bytes, None, false)),
        requests: Mutex::new(Vec::new()),
        bytes_transferred: Mutex::new(0),
    });
    let client = DaemonClient::new(transport.clone());

    // 1. Text tab with size-only stamp replaced by same-size binary without mtime:
    //    must return Read::Binary with stamp = None (never Unchanged).
    let text_to_binary = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/notes.dat",
        false,
        false,
        Some((size, None)),
        None,
    )
    .unwrap();
    let binary_stamp = match text_to_binary {
        Read::Binary(got_size, stamp) => {
            assert_eq!(got_size, size);
            stamp
        }
        _ => panic!("expected Read::Binary when text is replaced by same-size binary"),
    };
    assert_eq!(binary_stamp, None);

    // 2. Binary placeholder (no cached mtime stamp) replaced by same-size text without mtime:
    //    must return Read::Text (never Unchanged).
    *transport.state.lock().unwrap() = (text_bytes, None, true);
    let binary_to_text = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/notes.dat",
        false,
        false,
        binary_stamp,
        None,
    )
    .unwrap();
    assert!(matches!(
        binary_to_text,
        Read::Text(doc, None, None, (got_size, None), None)
            if got_size == size && doc.text() == "abcd"
    ));

    // 3. Known binary extension without mtime also omits a size-only stamp.
    let pdf_bytes = b"%PDF-1.4\n%%EOF\n".to_vec();
    let pdf_size = pdf_bytes.len() as u64;
    *transport.state.lock().unwrap() = (pdf_bytes, None, true);
    let pdf_no_mtime = read_text_or_svg_file(
        &client,
        Path::new("/repo"),
        "build/report.pdf",
        false,
        false,
        None,
        None,
    )
    .unwrap();
    assert!(matches!(pdf_no_mtime, Read::Binary(got_size, None) if got_size == pdf_size));
}
