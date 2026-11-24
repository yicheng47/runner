/// Where a link in the page leads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkTarget {
    /// Opens in the browser.
    Url(String),
    /// A file in the root, by its path from the root: opens in a new tab.
    File(String),
    /// Nothing to open: an anchor, another scheme, or a path that leaves
    /// the root.
    None,
}

/// Where `href`, written in the Markdown file at `from` (a path from the
/// root), leads. A relative path resolves against the file's folder and a
/// path starting with `/` against the root, as on a forge; neither may leave
/// the root.
pub(crate) fn route(href: &str, from: &str) -> LinkTarget {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return LinkTarget::None;
    }
    if let Some((scheme, _)) = href.split_once(':') {
        let is_scheme = scheme.len() > 1
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme || scheme.len() == 1 {
            return match scheme.to_ascii_lowercase().as_str() {
                "http" | "https" | "mailto" => LinkTarget::Url(href.to_owned()),
                _ => LinkTarget::None,
            };
        }
    }
    let path = href.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() || path.contains('\\') {
        return LinkTarget::None;
    }
    let path = percent_decode(path);
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        let folder = from.rsplit_once('/').map_or("", |(folder, _)| folder);
        folder.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return LinkTarget::None;
                }
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        LinkTarget::None
    } else {
        LinkTarget::File(parts.join("/"))
    }
}

/// `%20` and its kind back to the bytes they stand for; anything that is
/// not a valid escape stays as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, ..) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// The image format a local image's extension names; any other file is
/// not read as an image.
pub(crate) fn image_format(path: &str) -> Option<gpui::ImageFormat> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => gpui::ImageFormat::Png,
        "jpg" | "jpeg" => gpui::ImageFormat::Jpeg,
        "gif" => gpui::ImageFormat::Gif,
        "webp" => gpui::ImageFormat::Webp,
        "svg" => gpui::ImageFormat::Svg,
        "bmp" => gpui::ImageFormat::Bmp,
        _ => return None,
    })
}
