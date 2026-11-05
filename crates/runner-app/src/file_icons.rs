//! The Files tree's icons: themes in Zed's icon-theme format, bundled by
//! `script/file-icon-themes.py` in `assets/file-icons/` with only the SVGs
//! each theme maps.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::OnceLock;

use gpui::SharedString;
use serde::{Deserialize, Serialize};

const ASSET_PREFIX: &str = "file-icons/";

/// Unknown keys load as Catppuccin, which serde requires to be the last
/// variant.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSet {
    Material,
    Minimal,
    #[default]
    #[serde(other)]
    Catppuccin,
}

impl IconSet {
    pub(crate) const ALL: [Self; 3] = [Self::Catppuccin, Self::Material, Self::Minimal];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Catppuccin => "Catppuccin",
            Self::Material => "Material",
            Self::Minimal => "Minimal",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Catppuccin => "catppuccin",
            Self::Material => "material",
            Self::Minimal => "minimal",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "catppuccin" => Some(Self::Catppuccin),
            "material" => Some(Self::Material),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }

    fn compressed(self) -> &'static [u8] {
        match self {
            Self::Catppuccin => include_bytes!("../../../assets/file-icons/catppuccin.json.z"),
            Self::Material => include_bytes!("../../../assets/file-icons/material.json.z"),
            Self::Minimal => include_bytes!("../../../assets/file-icons/minimal.json.z"),
        }
    }

    fn bundle(self) -> &'static Bundle {
        static BUNDLES: [OnceLock<Bundle>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        BUNDLES[self as usize].get_or_init(|| {
            let mut json = Vec::new();
            flate2::read::ZlibDecoder::new(self.compressed())
                .read_to_end(&mut json)
                .expect("bundled icon theme inflates");
            serde_json::from_slice(&json).expect("bundled icon theme parses")
        })
    }
}

/// A theme in Zed's icon-theme format (`icon_themes/*.json`); fields the
/// tree does not use are ignored.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct IconTheme {
    #[serde(default)]
    directory_icons: DirectoryIcons,
    #[serde(default)]
    named_directory_icons: HashMap<String, DirectoryIcons>,
    #[serde(default)]
    file_stems: HashMap<String, String>,
    #[serde(default)]
    file_suffixes: HashMap<String, String>,
    #[serde(default)]
    file_icons: HashMap<String, IconDefinition>,
}

#[derive(Debug, Default, Deserialize)]
struct DirectoryIcons {
    collapsed: Option<String>,
    expanded: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IconDefinition {
    path: String,
}

impl IconTheme {
    /// The icon path for a file named `name`, matched as Zed does: the whole
    /// name, then each suffix after a dot from the longest, each against the
    /// theme's stems and then its suffixes; else the theme's default.
    pub(crate) fn file_icon(&self, name: &str) -> Option<&str> {
        let lookup = |key: &str| {
            self.file_stems
                .get(key)
                .or_else(|| self.file_suffixes.get(key))
                .and_then(|kind| self.file_icons.get(kind))
        };
        let mut candidate = name;
        let icon = lookup(candidate).or_else(|| loop {
            let (_, suffix) = candidate.split_once('.')?;
            if let Some(icon) = lookup(suffix) {
                break Some(icon);
            }
            candidate = suffix;
        });
        icon.or_else(|| self.file_icons.get("default"))
            .map(|icon| icon.path.as_str())
    }

    pub(crate) fn folder_icon(&self, name: &str, expanded: bool) -> Option<&str> {
        fn pick(icons: &DirectoryIcons, expanded: bool) -> Option<&str> {
            if expanded {
                icons.expanded.as_deref()
            } else {
                icons.collapsed.as_deref()
            }
        }
        self.named_directory_icons
            .get(name)
            .and_then(|icons| pick(icons, expanded))
            .or_else(|| pick(&self.directory_icons, expanded))
    }
}

#[derive(Deserialize)]
struct Bundle {
    theme: IconTheme,
    icons: HashMap<String, String>,
    /// Recolours the light icons for dark appearances.
    #[serde(default)]
    dark_colors: HashMap<String, String>,
    /// Dark icons the recolouring does not reproduce.
    #[serde(default)]
    dark_icons: HashMap<String, String>,
    /// Drawn tinted in the tree's text colour rather than in its own colours.
    #[serde(default)]
    monochrome: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileIcon {
    /// An embedded asset path served by [`asset`].
    pub(crate) path: SharedString,
    pub(crate) monochrome: bool,
}

fn icon(set: IconSet, dark: bool, theme_path: Option<&str>) -> Option<FileIcon> {
    let bundle = set.bundle();
    let theme_path = theme_path?;
    bundle.icons.contains_key(theme_path).then(|| FileIcon {
        path: format!(
            "{ASSET_PREFIX}{}/{}/{theme_path}",
            set.key(),
            if dark { "dark" } else { "light" }
        )
        .into(),
        monochrome: bundle.monochrome,
    })
}

pub(crate) fn file_icon(set: IconSet, dark: bool, name: &str) -> Option<FileIcon> {
    icon(set, dark, set.bundle().theme.file_icon(name))
}

pub(crate) fn folder_icon(
    set: IconSet,
    dark: bool,
    name: &str,
    expanded: bool,
) -> Option<FileIcon> {
    icon(set, dark, set.bundle().theme.folder_icon(name, expanded))
}

/// Serves `file-icons/<set>/<light|dark>/<theme path>` for the app's asset source.
pub(crate) fn asset(path: &str) -> Option<Cow<'static, [u8]>> {
    let rest = path.strip_prefix(ASSET_PREFIX)?;
    let (set, rest) = rest.split_once('/')?;
    let (appearance, theme_path) = rest.split_once('/')?;
    let bundle = IconSet::from_key(set)?.bundle();
    let svg = bundle.icons.get(theme_path)?;
    if appearance != "dark" || bundle.dark_colors.is_empty() {
        return Some(Cow::Borrowed(svg.as_bytes()));
    }
    if let Some(svg) = bundle.dark_icons.get(theme_path) {
        return Some(Cow::Borrowed(svg.as_bytes()));
    }
    Some(Cow::Owned(recolour(svg, &bundle.dark_colors).into_bytes()))
}

fn recolour(svg: &str, colors: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(index) = rest.find('#') {
        out.push_str(&rest[..index]);
        let hex = rest.get(index..index + 7);
        match hex.and_then(|hex| colors.get(&hex.to_ascii_lowercase())) {
            Some(color) => {
                out.push_str(color);
                rest = &rest[index + 7..];
            }
            None => {
                out.push('#');
                rest = &rest[index + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(json: serde_json::Value) -> IconTheme {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn files_match_the_whole_name_then_each_suffix_then_the_default() {
        let theme = theme(serde_json::json!({
            "file_stems": {"Makefile": "make", "Cargo.lock": "lock"},
            "file_suffixes": {"rs": "rust", "config.js": "config", "js": "javascript", "md": "markdown"},
            "file_icons": {
                "make": {"path": "make.svg"},
                "lock": {"path": "lock.svg"},
                "rust": {"path": "rust.svg"},
                "config": {"path": "config.svg"},
                "javascript": {"path": "js.svg"},
                "markdown": {"path": "md.svg"},
                "default": {"path": "file.svg"},
            },
        }));
        assert_eq!(theme.file_icon("Makefile"), Some("make.svg"));
        assert_eq!(theme.file_icon("Cargo.lock"), Some("lock.svg"));
        assert_eq!(theme.file_icon("lib.rs"), Some("rust.svg"));
        assert_eq!(theme.file_icon("eslint.config.js"), Some("config.svg"));
        assert_eq!(theme.file_icon("app.js"), Some("js.svg"));
        assert_eq!(theme.file_icon(".notes.md"), Some("md.svg"));
        assert_eq!(theme.file_icon("LICENSE"), Some("file.svg"));
        assert_eq!(theme.file_icon("archive.tar.gz"), Some("file.svg"));
    }

    #[test]
    fn folders_use_a_named_icon_or_the_theme_folder() {
        let theme = theme(serde_json::json!({
            "directory_icons": {"collapsed": "folder.svg", "expanded": "folder-open.svg"},
            "named_directory_icons": {"src": {"collapsed": "src.svg", "expanded": "src-open.svg"}},
        }));
        assert_eq!(theme.folder_icon("src", false), Some("src.svg"));
        assert_eq!(theme.folder_icon("src", true), Some("src-open.svg"));
        assert_eq!(theme.folder_icon("docs", false), Some("folder.svg"));
        assert_eq!(theme.folder_icon("docs", true), Some("folder-open.svg"));
    }

    #[test]
    fn bundled_themes_load_and_resolve_common_files() {
        for set in [IconSet::Catppuccin, IconSet::Material, IconSet::Minimal] {
            for dark in [false, true] {
                for name in ["main.rs", "README.md", "Cargo.toml", "no-extension"] {
                    let icon =
                        file_icon(set, dark, name).unwrap_or_else(|| panic!("{set:?} {name}"));
                    assert_eq!(icon.monochrome, set == IconSet::Minimal);
                    let svg = asset(&icon.path).unwrap();
                    assert!(std::str::from_utf8(&svg).unwrap().starts_with("<svg"));
                }
                for expanded in [false, true] {
                    let icon = folder_icon(set, dark, "src", expanded).unwrap();
                    assert!(asset(&icon.path).is_some());
                }
            }
        }
        let rust = file_icon(IconSet::Catppuccin, false, "lib.rs").unwrap();
        assert_ne!(
            rust,
            file_icon(IconSet::Catppuccin, false, "Cargo.toml").unwrap()
        );
    }

    #[test]
    fn catppuccin_follows_latte_in_light_and_mocha_in_dark() {
        let light = file_icon(IconSet::Catppuccin, false, "lib.rs").unwrap();
        let dark = file_icon(IconSet::Catppuccin, true, "lib.rs").unwrap();
        let light = String::from_utf8(asset(&light.path).unwrap().into_owned()).unwrap();
        let dark = String::from_utf8(asset(&dark.path).unwrap().into_owned()).unwrap();
        // Latte's and Mocha's peach.
        assert!(light.contains("#fe640b"), "{light}");
        assert!(dark.contains("#fab387"), "{dark}");
        assert!(light.contains(r#"width="48""#));
    }

    #[test]
    fn each_bundle_carries_its_licences() {
        for (set, licences) in [
            (
                IconSet::Catppuccin,
                vec![include_str!(
                    "../../../assets/file-icons/LICENSE-catppuccin"
                )],
            ),
            (
                IconSet::Material,
                vec![
                    include_str!("../../../assets/file-icons/LICENSE-material"),
                    include_str!("../../../assets/file-icons/LICENSE-material-icons"),
                ],
            ),
            (
                IconSet::Minimal,
                vec![include_str!("../../../assets/file-icons/LICENSE-minimal")],
            ),
        ] {
            let mut json = Vec::new();
            flate2::read::ZlibDecoder::new(set.compressed())
                .read_to_end(&mut json)
                .unwrap();
            let bundle: serde_json::Value = serde_json::from_slice(&json).unwrap();
            assert_eq!(bundle["licenses"], serde_json::json!(licences), "{set:?}");
        }
    }

    #[test]
    fn unknown_assets_are_not_served() {
        assert!(asset("file-icons/catppuccin/light/./icons/latte/nope.svg").is_none());
        assert!(asset("file-icons/other/light/x.svg").is_none());
        assert!(asset("folder.svg").is_none());
    }

    #[test]
    fn recolouring_replaces_mapped_hex_colours_only() {
        let colors = HashMap::from([("#aabbcc".to_owned(), "#112233".to_owned())]);
        assert_eq!(
            recolour(r##"<g fill="#AABBCC" stroke="#ddeeff"/>#"##, &colors),
            r##"<g fill="#112233" stroke="#ddeeff"/>#"##
        );
    }
}
