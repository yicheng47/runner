use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use gpui::{App, AppContext as _, Context, Entity, SharedString, WeakEntity};
use runner_core::protocol::files::{BINARY_SNIFF_BYTES, MAX_READ_BYTES};
use runner_core::protocol::runtime_paths::find_executable;
use runner_terminal::terminal::{FileLinkContext, LinkTarget};

use crate::app_settings::{AppSettings, FileLinkEditor, FileLinkOpenIn};
use crate::app_store::global_app_store;
use crate::surfaces::files::{FilesView, ShowPreview};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileLinkTarget {
    pub(crate) path: PathBuf,
    pub(crate) line: Option<u32>,
    pub(crate) column: Option<u32>,
}

impl FileLinkTarget {
    fn locator(&self) -> String {
        let mut locator = self.path.to_string_lossy().into_owned();
        if let Some(line) = self.line {
            locator.push_str(&format!(":{line}"));
            if let Some(column) = self.column {
                locator.push_str(&format!(":{column}"));
            }
        }
        locator
    }
}

/// The side panel's availability for opening a file link in Runner's preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLinkPanelState {
    Shown,
    Collapsed,
    PanelOff,
    FilesOff,
}

impl FileLinkPanelState {
    pub(crate) fn from_settings(settings: &AppSettings, open: bool) -> Self {
        if !settings.show_side_panel {
            Self::PanelOff
        } else if !settings.files_view {
            Self::FilesOff
        } else if open {
            Self::Shown
        } else {
            Self::Collapsed
        }
    }

    pub(crate) fn can_preview(self) -> bool {
        matches!(self, Self::Shown | Self::Collapsed)
    }
}

/// Known binary file categories shared by click routing and the viewer's
/// cannot-preview placeholder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryFileKind {
    Pdf,
    Word,
    Document,
    Excel,
    Spreadsheet,
    PowerPoint,
    Presentation,
    Zip,
    Archive,
    DiskImage,
    Installer,
    Audio,
    Video,
    Font,
    Database,
    Executable,
    Image,
    Ebook,
}

impl BinaryFileKind {
    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::Pdf => "file_kind.pdf",
            Self::Word => "file_kind.word",
            Self::Document => "file_kind.document",
            Self::Excel => "file_kind.excel",
            Self::Spreadsheet => "file_kind.spreadsheet",
            Self::PowerPoint => "file_kind.powerpoint",
            Self::Presentation => "file_kind.presentation",
            Self::Zip => "file_kind.zip",
            Self::Archive => "file_kind.archive",
            Self::DiskImage => "file_kind.disk_image",
            Self::Installer => "file_kind.installer",
            Self::Audio => "file_kind.audio",
            Self::Video => "file_kind.video",
            Self::Font => "file_kind.font",
            Self::Database => "file_kind.database",
            Self::Executable => "file_kind.executable",
            Self::Image => "file_kind.image",
            Self::Ebook => "file_kind.ebook",
        }
    }

    pub(crate) fn is_executable_or_installer(self) -> bool {
        matches!(self, Self::Executable | Self::Installer)
    }
}

pub(crate) fn binary_kind_label(path: &Path) -> &'static str {
    let key = binary_kind_for_path(path).map_or("file_kind.binary", BinaryFileKind::label_key);
    crate::i18n::text(key)
}

/// What kind of file a link's path names, from its name and extension alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLinkKind {
    Text,
    Image,
    Html,
    Binary(BinaryFileKind),
    Executable(BinaryFileKind),
    Unknown,
}

/// Content facts gathered off the UI thread for an unknown-extension link.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FileLinkContent {
    #[default]
    Unknown,
    Binary,
    Executable,
}

/// Where a ⌘-clicked (Ctrl-clicked on Windows and Linux) file link opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLinkDecision {
    Preview,
    Editor,
    DefaultApp,
    Browser,
}

pub(crate) fn binary_kind_for_path(path: &Path) -> Option<BinaryFileKind> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_ascii_lowercase();
    Some(match ext.as_str() {
        "pdf" => BinaryFileKind::Pdf,
        "doc" | "docx" => BinaryFileKind::Word,
        "odt" | "pages" => BinaryFileKind::Document,
        "xls" | "xlsx" => BinaryFileKind::Excel,
        "ods" | "numbers" => BinaryFileKind::Spreadsheet,
        "ppt" | "pptx" => BinaryFileKind::PowerPoint,
        "odp" | "key" => BinaryFileKind::Presentation,
        "zip" => BinaryFileKind::Zip,
        "tar" | "gz" | "tgz" | "bz2" | "tbz2" | "xz" | "txz" | "zst" | "7z" | "rar" | "war"
        | "ear" | "apk" | "ipa" | "whl" | "cab" => BinaryFileKind::Archive,
        "dmg" | "iso" | "img" => BinaryFileKind::DiskImage,
        "msi" | "msix" | "msp" | "pkg" | "appimage" | "deb" | "rpm" => BinaryFileKind::Installer,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "oga" | "opus" | "m4a" | "wma" | "aiff"
        | "aif" | "mid" | "midi" => BinaryFileKind::Audio,
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "wmv" | "flv" | "m4v" | "mpeg" | "mpg" | "3gp" => {
            BinaryFileKind::Video
        }
        "ttf" | "otf" | "woff" | "woff2" | "eot" => BinaryFileKind::Font,
        "sqlite" | "sqlite3" | "db" | "mdb" => BinaryFileKind::Database,
        "exe" | "com" | "scr" | "pif" | "cpl" | "dll" | "so" | "dylib" | "a" | "o" | "obj"
        | "wasm" | "pyc" | "pyo" | "class" | "beam" | "rlib" | "pdb" | "jar" | "app"
        | "command" | "lnk" | "hta" | "reg" => BinaryFileKind::Executable,
        "heic" | "heif" | "avif" | "tiff" | "tif" | "ico" | "icns" | "psd" | "ai" | "eps"
        | "raw" | "cr2" | "nef" | "dng" => BinaryFileKind::Image,
        "epub" | "mobi" => BinaryFileKind::Ebook,
        _ => return None,
    })
}

pub(crate) fn classify_file_link(path: &Path) -> FileLinkKind {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if let Some(ext) = ext.as_deref() {
        if matches!(ext, "html" | "htm") {
            return FileLinkKind::Html;
        }
        if matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp") {
            return FileLinkKind::Image;
        }
    }
    if let Some(kind) = binary_kind_for_path(path) {
        return if kind.is_executable_or_installer() {
            FileLinkKind::Executable(kind)
        } else {
            FileLinkKind::Binary(kind)
        };
    }
    let path_str = path.to_string_lossy();
    if crate::surfaces::files::highlight::grammar_for_path(&path_str).is_some() {
        return FileLinkKind::Text;
    }
    if let Some(ext) = ext.as_deref() {
        if matches!(
            ext,
            "txt"
                | "text"
                | "log"
                | "csv"
                | "tsv"
                | "xml"
                | "xhtml"
                | "ini"
                | "cfg"
                | "conf"
                | "env"
                | "properties"
                | "diff"
                | "patch"
                | "rst"
                | "adoc"
                | "tex"
                | "bib"
                | "org"
                | "jsonl"
                | "ndjson"
                | "json5"
                | "graphql"
                | "gql"
                | "proto"
                | "prisma"
                | "tf"
                | "tfvars"
                | "hcl"
                | "nix"
                | "zig"
                | "dart"
                | "ex"
                | "exs"
                | "erl"
                | "hrl"
                | "hs"
                | "ml"
                | "mli"
                | "clj"
                | "cljs"
                | "scala"
                | "groovy"
                | "gradle"
                | "ps1"
                | "psm1"
                | "bat"
                | "cmd"
                | "vbs"
                | "vbe"
                | "wsf"
                | "wsh"
                | "fish"
                | "vim"
                | "el"
                | "lisp"
                | "scm"
                | "awk"
                | "sed"
                | "pl"
                | "pm"
                | "r"
                | "m"
                | "mm"
                | "vue"
                | "svelte"
                | "astro"
                | "mdx"
                | "scss"
                | "sass"
                | "less"
                | "styl"
                | "lock"
                | "sum"
                | "mod"
                | "cmake"
                | "just"
                | "dockerfile"
        ) {
            return FileLinkKind::Text;
        }
    }
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "readme"
                | "license"
                | "licence"
                | "changelog"
                | "authors"
                | "contributing"
                | "copying"
                | "notice"
                | "todo"
                | "dockerfile"
                | "containerfile"
                | "justfile"
                | "procfile"
                | "brewfile"
                | "vagrantfile"
                | "cmakelists.txt"
                | ".gitignore"
                | ".gitattributes"
                | ".gitmodules"
                | ".editorconfig"
                | ".dockerignore"
                | ".npmrc"
                | ".nvmrc"
                | ".env"
                | ".env.local"
                | ".env.example"
        ) {
            return FileLinkKind::Text;
        }
    }
    FileLinkKind::Unknown
}

/// The single pure decision for where a ⌘-clicked file link opens. Click
/// handling, hover tooltips, and the Settings → General info line all call it
/// so they cannot disagree.
pub(crate) fn decide_file_link(
    path: &Path,
    size: Option<u64>,
    content: FileLinkContent,
    local: bool,
    open_in: FileLinkOpenIn,
    panel: FileLinkPanelState,
) -> FileLinkDecision {
    if !local {
        return FileLinkDecision::Editor;
    }
    match classify_file_link(path) {
        FileLinkKind::Html => FileLinkDecision::Browser,
        FileLinkKind::Binary(_) => FileLinkDecision::DefaultApp,
        FileLinkKind::Executable(_) | FileLinkKind::Image => {
            if open_in == FileLinkOpenIn::Preview && panel.can_preview() {
                FileLinkDecision::Preview
            } else {
                FileLinkDecision::Editor
            }
        }
        FileLinkKind::Text => {
            if size.is_some_and(|bytes| bytes > MAX_READ_BYTES) {
                FileLinkDecision::Editor
            } else if open_in == FileLinkOpenIn::Preview && panel.can_preview() {
                FileLinkDecision::Preview
            } else {
                FileLinkDecision::Editor
            }
        }
        FileLinkKind::Unknown => match content {
            FileLinkContent::Binary => FileLinkDecision::DefaultApp,
            FileLinkContent::Executable => {
                if open_in == FileLinkOpenIn::Preview && panel.can_preview() {
                    FileLinkDecision::Preview
                } else {
                    FileLinkDecision::Editor
                }
            }
            FileLinkContent::Unknown => {
                if size.is_some_and(|bytes| bytes > MAX_READ_BYTES) {
                    FileLinkDecision::Editor
                } else if open_in == FileLinkOpenIn::Preview && panel.can_preview() {
                    FileLinkDecision::Preview
                } else {
                    FileLinkDecision::Editor
                }
            }
        },
    }
}

/// When Settings → General has **Runner preview** selected, returns why links
/// currently fall back to the Editor (panel off or Files off), decided through
/// [`decide_file_link`].
pub(crate) fn settings_panel_fallback(settings: &AppSettings) -> Option<FileLinkPanelState> {
    if settings.file_link_open_in != FileLinkOpenIn::Preview {
        return None;
    }
    let panel = FileLinkPanelState::from_settings(settings, true);
    let decision = decide_file_link(
        Path::new("file.txt"),
        None,
        FileLinkContent::Unknown,
        true,
        settings.file_link_open_in,
        panel,
    );
    (decision != FileLinkDecision::Preview).then_some(panel)
}

#[cfg(unix)]
fn open_regular_nonblocking(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    #[cfg(target_os = "macos")]
    const O_NONBLOCK: i32 = 0x0004;
    #[cfg(not(target_os = "macos"))]
    use libc::O_NONBLOCK;

    // Skip FIFOs, sockets, device nodes, and directories before opening so a
    // file URI pointing at a FIFO never strands a background worker.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    Some(file)
}

#[cfg(not(unix))]
fn open_regular_nonblocking(path: &Path) -> Option<std::fs::File> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::OpenOptions::new().read(true).open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    Some(file)
}

fn has_executable_magic(buf: &[u8]) -> bool {
    if buf.starts_with(b"\x7fELF") || buf.starts_with(b"MZ") {
        return true;
    }
    if buf.len() >= 4 {
        let magic = [buf[0], buf[1], buf[2], buf[3]];
        if matches!(
            magic,
            [0xFE, 0xED, 0xFA, 0xCE]
                | [0xCE, 0xFA, 0xED, 0xFE]
                | [0xFE, 0xED, 0xFA, 0xCF]
                | [0xCF, 0xFA, 0xED, 0xFE]
                | [0xCA, 0xFE, 0xBA, 0xBE]
                | [0xBE, 0xBA, 0xFE, 0xCA]
        ) {
            return true;
        }
    }
    false
}

/// Matches `runnerd`'s regular-file nonblocking open and NUL-byte binary sniff,
/// while keeping executable binaries on the safe inspection path. Run off the
/// UI thread.
pub(crate) fn sniff_file_content(path: &Path) -> FileLinkContent {
    use std::io::Read as _;
    let Some(file) = open_regular_nonblocking(path) else {
        return FileLinkContent::Unknown;
    };
    #[cfg(unix)]
    let is_exec_mode = {
        use std::os::unix::fs::PermissionsExt as _;
        file.metadata()
            .ok()
            .is_some_and(|meta| meta.permissions().mode() & 0o111 != 0)
    };
    #[cfg(not(unix))]
    let is_exec_mode = false;
    let mut buf = Vec::with_capacity(BINARY_SNIFF_BYTES);
    if file
        .take(BINARY_SNIFF_BYTES as u64)
        .read_to_end(&mut buf)
        .is_err()
    {
        return FileLinkContent::Unknown;
    }
    if !buf.contains(&0) {
        return FileLinkContent::Unknown;
    }
    if is_exec_mode || has_executable_magic(&buf) {
        FileLinkContent::Executable
    } else {
        FileLinkContent::Binary
    }
}

/// Files whose platform open verb can execute a program, script, or installer
/// instead of opening an inspection viewer when invoked automatically from a
/// local terminal click.
pub(crate) fn is_unsafe_os_open(path: &Path, content: FileLinkContent) -> bool {
    if matches!(content, FileLinkContent::Executable)
        || matches!(classify_file_link(path), FileLinkKind::Executable(_))
    {
        return true;
    }
    #[cfg(windows)]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|ext| {
                matches!(
                    ext.as_str(),
                    "bat" | "cmd" | "ps1" | "psm1" | "vbs" | "vbe" | "wsf" | "wsh"
                )
            })
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileLinkRouteState {
    pub(crate) local: bool,
    pub(crate) panel_open: bool,
}

type LinkView = Rc<dyn Fn(&mut App) -> Option<(Entity<FilesView>, ShowPreview)>>;
type ExistingLinkView = Rc<dyn Fn(&App) -> Option<Entity<FilesView>>>;
type LinkRouteState = Rc<dyn Fn(&App) -> FileLinkRouteState>;

/// Where a terminal's ⌘-clicked file links open: routed through
/// [`decide_file_link`] to the panel's preview, the chosen editor, the default
/// app, or the default browser.
#[derive(Clone)]
pub(crate) struct FileLinkRoute {
    view: LinkView,
    existing_view: ExistingLinkView,
    state: LinkRouteState,
}

impl FileLinkRoute {
    /// Route for a terminal on another computer: every file link stays on the
    /// chosen Editor (never local preview, browser, or default-app sniffing).
    pub(crate) fn remote() -> Self {
        Self {
            view: Rc::new(|_| None),
            existing_view: Rc::new(|_| None),
            state: Rc::new(|_| FileLinkRouteState {
                local: false,
                panel_open: false,
            }),
        }
    }

    /// `view` gives the panel's Files view while its preview can take links,
    /// `existing_view` reads an already-created Files view without fetching,
    /// `show` opens the panel on it, and `state` reports whether the terminal
    /// is local and whether its panel is open.
    pub(crate) fn new<T: 'static>(
        owner: WeakEntity<T>,
        view: impl Fn(&mut T, &mut Context<T>) -> Option<Entity<FilesView>> + 'static,
        existing_view: impl Fn(&T, &App) -> Option<Entity<FilesView>> + 'static,
        show: impl Fn(&mut T, &mut Context<T>) + 'static,
        state: impl Fn(&T, &App) -> FileLinkRouteState + 'static,
    ) -> Self {
        let show = Rc::new(show);
        let existing_owner = owner.clone();
        let state_owner = owner.clone();
        Self {
            view: Rc::new(move |cx| {
                let view = owner.update(cx, |this, cx| view(this, cx)).ok().flatten()?;
                let owner = owner.clone();
                let show = show.clone();
                let show: ShowPreview = Rc::new(move |cx| {
                    let _ = owner.update(cx, |owner, cx| show(owner, cx));
                });
                Some((view, show))
            }),
            existing_view: Rc::new(move |cx| {
                let owner = existing_owner.upgrade()?;
                existing_view(owner.read(cx), cx)
            }),
            state: Rc::new(move |cx| {
                state_owner
                    .upgrade()
                    .map(|owner| state(owner.read(cx), cx))
                    .unwrap_or(FileLinkRouteState {
                        local: false,
                        panel_open: false,
                    })
            }),
        }
    }

    fn route_inputs(&self, cx: &App) -> (bool, FileLinkOpenIn, FileLinkPanelState) {
        let settings = &global_app_store(cx).read(cx).settings;
        let state = (self.state)(cx);
        (
            state.local,
            settings.file_link_open_in,
            FileLinkPanelState::from_settings(settings, state.panel_open),
        )
    }

    pub(crate) fn is_local(&self, cx: &App) -> bool {
        (self.state)(cx).local
    }

    /// Reads the resolved root the Files view already holds, synchronously and
    /// without fetching.
    pub(crate) fn link_context(&self, cx: &App) -> FileLinkContext {
        let extra_root = (self.existing_view)(cx)
            .and_then(|view| view.read(cx).root_path().map(Path::to_path_buf));
        FileLinkContext { extra_root }
    }

    pub(crate) fn open(&self, target: FileLinkTarget, cx: &mut App) {
        let (local, open_in, panel) = self.route_inputs(cx);
        if !local {
            // Remote sessions keep the legacy editor path without local probes
            // or automatic local terminal safeguards.
            open_file_in_editor(target, cx);
            return;
        }
        let preliminary = decide_file_link(
            &target.path,
            None,
            FileLinkContent::Unknown,
            true,
            open_in,
            panel,
        );
        if preliminary != FileLinkDecision::Preview {
            open_local_non_preview(target, open_in, panel, cx);
            return;
        }
        match (self.view)(cx) {
            Some((view, show)) => {
                view.update(cx, |view, cx| view.open_terminal_link(target, show, cx));
            }
            None => open_local_non_preview(target, open_in, FileLinkPanelState::PanelOff, cx),
        }
    }

    /// Where a click on `path`, hovered since `since`, will open; `None` while
    /// the panel checks root or size off the UI thread, calling `redraw` once
    /// it may know.
    pub(crate) fn decision(
        &self,
        path: &Path,
        since: std::time::Instant,
        redraw: ShowPreview,
        cx: &mut App,
    ) -> Option<FileLinkDecision> {
        let (local, open_in, panel) = self.route_inputs(cx);
        let preliminary =
            decide_file_link(path, None, FileLinkContent::Unknown, local, open_in, panel);
        if preliminary != FileLinkDecision::Preview {
            return Some(preliminary);
        }
        match (self.view)(cx) {
            Some((view, _)) => {
                view.update(cx, |view, cx| view.link_decision(path, since, redraw, cx))
            }
            None => Some(FileLinkDecision::Editor),
        }
    }
}

/// How the effective editor destination reads in prose ("Zed", "VS Code",
/// "Cursor", or "default app").
pub(crate) fn effective_editor_destination(
    editor: FileLinkEditor,
    cli_found: Option<bool>,
) -> &'static str {
    if editor == FileLinkEditor::DefaultApp || cli_found == Some(false) {
        crate::i18n::text("ui.default_app_destination")
    } else {
        editor.label()
    }
}

/// The hover tooltip for a terminal link, driven by [`FileLinkDecision`].
/// Returns `None` when a guarded local executable/installer link has no safe
/// destination so the tooltip never promises an app that will not launch.
pub(crate) fn link_tooltip_content(
    target: &LinkTarget,
    modifier_held: bool,
    decision: FileLinkDecision,
    local: bool,
    cx: &App,
) -> Option<SharedString> {
    let store = global_app_store(cx);
    let store = store.read(cx);
    let editor = store.settings.file_link_editor;
    let cli_found = matches!(target, LinkTarget::File { .. })
        .then(|| {
            editor.cli().map(|command| {
                store
                    .file_link_environment
                    .1
                    .get(command)
                    .copied()
                    .unwrap_or(false)
            })
        })
        .flatten();
    link_tooltip(target, modifier_held, decision, local, editor, cli_found).map(Into::into)
}

/// Without the modifier the tooltip teaches the gesture; with it, it names
/// the action and the target so the user knows where focus is about to go.
fn link_tooltip(
    target: &LinkTarget,
    modifier_held: bool,
    decision: FileLinkDecision,
    local: bool,
    editor: FileLinkEditor,
    cli_found: Option<bool>,
) -> Option<String> {
    link_tooltip_for(
        target,
        modifier_held,
        decision,
        local,
        editor,
        cli_found,
        crate::platform_ui::PRIMARY_MODIFIER,
    )
}

fn link_tooltip_for(
    target: &LinkTarget,
    modifier_held: bool,
    decision: FileLinkDecision,
    local: bool,
    editor: FileLinkEditor,
    cli_found: Option<bool>,
    shortcut: &str,
) -> Option<String> {
    let (path, line) = match target {
        LinkTarget::Url(_) => {
            return Some(if modifier_held {
                crate::i18n::text("ui.open_in_browser").into()
            } else {
                crate::i18n::tf(
                    "format.click_to_open_in_browser",
                    &[("shortcut", shortcut.to_string())],
                )
            });
        }
        LinkTarget::File { path, line, .. } => (path, *line),
    };
    let name = || {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    Some(match decision {
        FileLinkDecision::Preview => match (modifier_held, line) {
            (false, _) => crate::i18n::tf(
                "format.click_to_open_in",
                &[
                    ("shortcut", shortcut.to_string()),
                    (
                        "destination",
                        crate::i18n::text("ui.side_panel_destination").to_string(),
                    ),
                ],
            ),
            (true, Some(line)) => crate::i18n::tf(
                "format.open_in_side_panel_at_line",
                &[("name", name()), ("line", line.to_string())],
            ),
            (true, None) => crate::i18n::tf("format.open_in_side_panel", &[("name", name())]),
        },
        FileLinkDecision::Browser => {
            if modifier_held {
                crate::i18n::tf("format.open_in_browser", &[("name", name())])
            } else {
                crate::i18n::tf(
                    "format.click_to_open_in_browser",
                    &[("shortcut", shortcut.to_string())],
                )
            }
        }
        FileLinkDecision::DefaultApp => {
            if modifier_held {
                crate::i18n::tf("format.open_in_default_app", &[("name", name())])
            } else {
                crate::i18n::tf(
                    "format.click_to_open_in",
                    &[
                        ("shortcut", shortcut.to_string()),
                        (
                            "destination",
                            crate::i18n::text("ui.default_app_destination").to_string(),
                        ),
                    ],
                )
            }
        }
        FileLinkDecision::Editor => {
            if local
                && is_unsafe_os_open(path, FileLinkContent::Unknown)
                && (editor == FileLinkEditor::DefaultApp || cli_found == Some(false))
            {
                return None;
            }
            if !modifier_held {
                let destination = effective_editor_destination(editor, cli_found);
                return Some(crate::i18n::tf(
                    "format.click_to_open_in",
                    &[
                        ("shortcut", shortcut.to_string()),
                        ("destination", destination.to_string()),
                    ],
                ));
            }
            let name = name();
            match (editor, cli_found, line) {
                (FileLinkEditor::DefaultApp, _, Some(line)) => crate::i18n::tf(
                    "format.open_in_default_app_line_is_lost",
                    &[("line", line.to_string())],
                ),
                (FileLinkEditor::DefaultApp, _, None) => {
                    crate::i18n::tf("format.open_in_default_app", &[("name", name)])
                }
                (editor, Some(false), _) => crate::i18n::tf(
                    "format.open_in_default_app_not_on_path",
                    &[("command", editor.cli().unwrap_or_default().to_string())],
                ),
                (editor, _, Some(line)) => crate::i18n::tf(
                    "format.open_in",
                    &[
                        ("editor", editor.label().to_string()),
                        ("name", name),
                        ("line", line.to_string()),
                    ],
                ),
                (editor, _, None) => crate::i18n::tf(
                    "format.open_file_in_editor",
                    &[("editor", editor.label().to_string()), ("name", name)],
                ),
            }
        }
    })
}

/// Opens a local file link that is not going to Runner's preview (Editor
/// chosen or panel/Files off), still routing by file type and sniffing
/// unknown-extension files off the UI thread.
pub(crate) fn open_local_non_preview(
    target: FileLinkTarget,
    open_in: FileLinkOpenIn,
    panel: FileLinkPanelState,
    cx: &mut App,
) {
    let preliminary = decide_file_link(
        &target.path,
        None,
        FileLinkContent::Unknown,
        true,
        open_in,
        panel,
    );
    if preliminary == FileLinkDecision::Editor
        && matches!(classify_file_link(&target.path), FileLinkKind::Unknown)
    {
        let path = target.path.clone();
        let sniff = cx.background_spawn(async move { sniff_file_content(&path) });
        cx.spawn(async move |cx| {
            let content = sniff.await;
            cx.update(|cx| {
                let open_in = global_app_store(cx).read(cx).settings.file_link_open_in;
                let decision = decide_file_link(&target.path, None, content, true, open_in, panel);
                open_local_terminal_decision(decision, target, content, cx);
            });
        })
        .detach();
        return;
    }
    open_local_terminal_decision(preliminary, target, FileLinkContent::Unknown, cx);
}

#[cfg(test)]
pub(crate) fn open_file_link(target: FileLinkTarget, cx: &mut App) {
    let open_in = global_app_store(cx).read(cx).settings.file_link_open_in;
    open_local_non_preview(target, open_in, FileLinkPanelState::PanelOff, cx);
}

/// Opens `target` in the editor saved by `file_link_editor` using the legacy
/// explicit/remote path (e.g. ⌘-click on a line number inside the file viewer,
/// or a remote/unrouted terminal).
pub(crate) fn open_file_in_editor(target: FileLinkTarget, cx: &mut App) {
    let editor = global_app_store(cx).read(cx).settings.file_link_editor;
    open_file_with(editor, target, cx);
}

/// Dispatches an automatic local terminal click's non-preview [`FileLinkDecision`].
pub(crate) fn open_local_terminal_decision(
    decision: FileLinkDecision,
    target: FileLinkTarget,
    content: FileLinkContent,
    cx: &mut App,
) {
    match decision {
        FileLinkDecision::Editor | FileLinkDecision::Preview => {
            let editor = global_app_store(cx).read(cx).settings.file_link_editor;
            open_file_with_mode(editor, target, content, LaunchMode::LocalAutomatic, cx);
        }
        FileLinkDecision::DefaultApp => open_in_default_app_local(target, content, cx),
        FileLinkDecision::Browser => open_in_browser(target, cx),
    }
}

/// Separates explicit viewer actions and remote terminal links (which keep
/// legacy launch/fallback behavior without local probes) from automatic local
/// terminal routing (which guards against invoking OS open on executables).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchMode {
    ExplicitOrRemote,
    LocalAutomatic,
}

/// Resolved external command plan, kept pure so tests can verify safety and
/// platform launcher selection below [`FileLinkLaunch`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LaunchPlan {
    Run {
        argv: Vec<String>,
        fallback: Option<Vec<String>>,
    },
    SkipUnsafeOpen,
}

pub(crate) fn plan_editor_launch(
    editor: FileLinkEditor,
    target: &FileLinkTarget,
    content: FileLinkContent,
    mode: LaunchMode,
) -> LaunchPlan {
    let unsafe_open =
        mode == LaunchMode::LocalAutomatic && is_unsafe_os_open(&target.path, content);
    match editor {
        FileLinkEditor::DefaultApp => {
            if unsafe_open {
                LaunchPlan::SkipUnsafeOpen
            } else {
                LaunchPlan::Run {
                    argv: open_argv(target),
                    fallback: None,
                }
            }
        }
        FileLinkEditor::Zed | FileLinkEditor::VsCode | FileLinkEditor::Cursor => LaunchPlan::Run {
            argv: editor_argv(editor, target),
            fallback: (!unsafe_open).then(|| open_argv(target)),
        },
    }
}

pub(crate) fn plan_default_app_launch(
    target: &FileLinkTarget,
    content: FileLinkContent,
    mode: LaunchMode,
) -> LaunchPlan {
    if mode == LaunchMode::LocalAutomatic && is_unsafe_os_open(&target.path, content) {
        LaunchPlan::SkipUnsafeOpen
    } else {
        LaunchPlan::Run {
            argv: open_argv(target),
            fallback: None,
        }
    }
}

/// Resolved browser command together with whether the launcher is a short-lived
/// helper (`wait_for_exit = true`) or a direct GUI browser process whose launch
/// succeeds as soon as it spawns (`wait_for_exit = false`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserLaunch {
    pub(crate) argv: Vec<String>,
    pub(crate) wait_for_exit: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileLinkLaunch {
    Editor(FileLinkEditor),
    DefaultApp,
    Browser,
}

#[cfg(test)]
thread_local! {
    /// What file-link launches would have invoked on this thread.
    pub(crate) static LAUNCHED: std::cell::RefCell<Vec<(FileLinkLaunch, FileLinkTarget)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Explicitly opens `target` in `editor` (viewer **Open in**, binary/large
/// placeholder buttons, viewer line ⌘-click, and remote terminals): preserves
/// legacy launch and fallback behavior without local probes or automatic
/// terminal blocking.
pub(crate) fn open_file_with(editor: FileLinkEditor, target: FileLinkTarget, cx: &mut App) {
    open_file_with_mode(
        editor,
        target,
        FileLinkContent::Unknown,
        LaunchMode::ExplicitOrRemote,
        cx,
    );
}

fn open_file_with_mode(
    editor: FileLinkEditor,
    target: FileLinkTarget,
    content: FileLinkContent,
    mode: LaunchMode,
    cx: &mut App,
) {
    if matches!(
        plan_editor_launch(editor, &target, content, mode),
        LaunchPlan::SkipUnsafeOpen
    ) {
        tracing::warn!(
            path = %target.path.display(),
            "refusing to invoke default OS launcher for automatic local executable or installer link"
        );
        return;
    }
    #[cfg(test)]
    if cfg!(test) {
        LAUNCHED.with(|launched| {
            launched
                .borrow_mut()
                .push((FileLinkLaunch::Editor(editor), target));
        });
        return;
    }
    let client = global_app_store(cx).read(cx).client.clone();
    cx.background_spawn(async move {
        let path_env = client.direct_chat_path().unwrap_or_default();
        if let Err(error) = execute_editor_launch(
            editor,
            &target,
            content,
            mode,
            &path_env,
            sniff_file_content,
            |argv, env| run_command_spec(argv, true, env),
        ) {
            tracing::warn!("file link editor launch failed: {error}");
        }
    })
    .detach();
}

fn open_in_default_app_local(target: FileLinkTarget, content: FileLinkContent, cx: &mut App) {
    if matches!(
        plan_default_app_launch(&target, content, LaunchMode::LocalAutomatic),
        LaunchPlan::SkipUnsafeOpen
    ) {
        tracing::warn!(
            path = %target.path.display(),
            "refusing to invoke default OS launcher for automatic local executable or installer link"
        );
        return;
    }
    #[cfg(test)]
    if cfg!(test) {
        LAUNCHED.with(|launched| {
            launched
                .borrow_mut()
                .push((FileLinkLaunch::DefaultApp, target));
        });
        return;
    }
    let client = global_app_store(cx).read(cx).client.clone();
    cx.background_spawn(async move {
        let path_env = client.direct_chat_path().unwrap_or_default();
        if let Err(error) = execute_default_app_launch(
            &target,
            content,
            LaunchMode::LocalAutomatic,
            &path_env,
            sniff_file_content,
            |argv, env| run_command_spec(argv, true, env),
        ) {
            tracing::warn!("file link default-app launch failed: {error}");
        }
    })
    .detach();
}

fn open_in_browser(target: FileLinkTarget, cx: &mut App) {
    #[cfg(test)]
    if cfg!(test) {
        LAUNCHED.with(|launched| {
            launched
                .borrow_mut()
                .push((FileLinkLaunch::Browser, target));
        });
        return;
    }
    let client = global_app_store(cx).read(cx).client.clone();
    cx.background_spawn(async move {
        let path_env = client.direct_chat_path().unwrap_or_default();
        let plan = plan_browser_launch(&target, &path_env);
        if let Err(error) = execute_browser_launch(plan, &path_env, |launch, env| {
            run_command_spec(&launch.argv, launch.wait_for_exit, env)
        }) {
            tracing::warn!("file link browser launch failed: {error}");
        }
    })
    .detach();
}

fn execute_editor_launch(
    editor: FileLinkEditor,
    target: &FileLinkTarget,
    content: FileLinkContent,
    mode: LaunchMode,
    path_env: &str,
    probe: impl FnOnce(&Path) -> FileLinkContent,
    mut runner: impl FnMut(&[String], &str) -> Result<(), String>,
) -> Result<(), String> {
    let resolved_content = if mode == LaunchMode::LocalAutomatic
        && content == FileLinkContent::Unknown
        && matches!(classify_file_link(&target.path), FileLinkKind::Unknown)
    {
        probe(&target.path)
    } else {
        content
    };
    match plan_editor_launch(editor, target, resolved_content, mode) {
        LaunchPlan::SkipUnsafeOpen => {
            tracing::warn!(
                path = %target.path.display(),
                "refusing to invoke default OS launcher for automatic local executable or installer link"
            );
            Ok(())
        }
        LaunchPlan::Run { argv, fallback } => {
            let Err(error) = runner(&argv, path_env) else {
                return Ok(());
            };
            let Some(fallback) = fallback else {
                return Err(error);
            };
            tracing::warn!("file link editor launch failed: {error}; falling back to open");
            runner(&fallback, path_env)
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
fn execute_default_app_launch(
    target: &FileLinkTarget,
    content: FileLinkContent,
    mode: LaunchMode,
    path_env: &str,
    probe: impl FnOnce(&Path) -> FileLinkContent,
    mut runner: impl FnMut(&[String], &str) -> Result<(), String>,
) -> Result<(), String> {
    let resolved_content = if mode == LaunchMode::LocalAutomatic
        && content == FileLinkContent::Unknown
        && matches!(classify_file_link(&target.path), FileLinkKind::Unknown)
    {
        probe(&target.path)
    } else {
        content
    };
    match plan_default_app_launch(target, resolved_content, mode) {
        LaunchPlan::SkipUnsafeOpen => {
            tracing::warn!(
                path = %target.path.display(),
                "refusing to invoke default OS launcher for automatic local executable or installer link"
            );
            Ok(())
        }
        LaunchPlan::Run { argv, .. } => runner(&argv, path_env),
    }
}

/// Executes an explicit browser launch plan and reports failures directly
/// without falling back to the file's `.html` open association.
fn execute_browser_launch(
    plan: Result<BrowserLaunch, String>,
    path_env: &str,
    runner: impl FnOnce(&BrowserLaunch, &str) -> Result<(), String>,
) -> Result<(), String> {
    let launch = plan?;
    runner(&launch, path_env)
}

fn path_to_file_url(path: &Path) -> Option<String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::fs::canonicalize(path)
            .ok()
            .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(path)))?
    };
    url::Url::from_file_path(&abs).ok().map(String::from)
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn plan_macos_browser_launch(
    file_url: &str,
    bundle_id: Option<&str>,
) -> Result<BrowserLaunch, String> {
    let Some(bundle_id) = bundle_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Err("no default web browser bundle registered".into());
    };
    Ok(BrowserLaunch {
        argv: vec![
            "/usr/bin/open".into(),
            "-b".into(),
            bundle_id.into(),
            file_url.into(),
        ],
        wait_for_exit: true,
    })
}

#[cfg(target_os = "linux")]
fn linux_xdg_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(data_home));
    } else if let Some(home) = runner_core::app_paths::home_dir() {
        dirs.push(home.join(".local").join("share"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty()) {
        dirs.extend(std::env::split_paths(&data_dirs));
    } else {
        dirs.push(PathBuf::from("/usr/local/share"));
        dirs.push(PathBuf::from("/usr/share"));
    }
    dirs
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn find_desktop_file_in_dirs(desktop_id: &str, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let id = desktop_id.trim();
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    for dir in data_dirs {
        let apps = dir.join("applications");
        let direct = apps.join(id);
        if direct.is_file() {
            return Some(direct);
        }
        // Freedesktop Desktop Entry Spec maps `-` to subdirectory separators
        // when locating desktop IDs nested under `applications/`.
        if let Some((prefix, rest)) = id.split_once('-') {
            let nested = apps.join(prefix).join(rest);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn parse_desktop_entry_exec(
    desktop_file: &Path,
    file_url: &str,
    local_path: &Path,
) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(desktop_file).ok()?;
    let mut in_entry = false;
    let mut exec_value = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "Exec"
        {
            exec_value = Some(value.trim());
            break;
        }
    }
    let tokens = shlex::split(exec_value?)?;
    if tokens.is_empty() || tokens[0].is_empty() {
        return None;
    }
    let local_str = local_path.to_string_lossy();
    let mut argv = Vec::with_capacity(tokens.len() + 1);
    argv.push(tokens[0].clone());
    let mut substituted = false;
    for token in tokens.into_iter().skip(1) {
        match token.as_str() {
            "%u" | "%U" => {
                argv.push(file_url.to_owned());
                substituted = true;
            }
            "%f" | "%F" => {
                argv.push(local_str.as_ref().to_owned());
                substituted = true;
            }
            // Drop desktop-entry metadata/deprecated field codes that take no URL.
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            other => {
                let mut expanded = other.replace("%%", "\0PERCENT\0");
                if expanded.contains("%u") || expanded.contains("%U") {
                    expanded = expanded.replace("%u", file_url).replace("%U", file_url);
                    substituted = true;
                }
                if expanded.contains("%f") || expanded.contains("%F") {
                    expanded = expanded
                        .replace("%f", local_str.as_ref())
                        .replace("%F", local_str.as_ref());
                    substituted = true;
                }
                argv.push(expanded.replace("\0PERCENT\0", "%"));
            }
        }
    }
    if !substituted {
        argv.push(file_url.to_owned());
    }
    Some(argv)
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn parse_browser_env(raw: &str, file_url: &str) -> Option<Vec<String>> {
    for entry in raw.split(':').map(str::trim).filter(|s| !s.is_empty()) {
        let Some(mut tokens) = shlex::split(entry) else {
            continue;
        };
        if tokens.is_empty() || tokens[0].is_empty() {
            continue;
        }
        let mut substituted = false;
        for arg in tokens.iter_mut().skip(1) {
            if arg == "%s" {
                *arg = file_url.to_owned();
                substituted = true;
            } else if arg.contains("%s") {
                *arg = arg.replace("%s", file_url);
                substituted = true;
            }
        }
        if !substituted {
            tokens.push(file_url.to_owned());
        }
        return Some(tokens);
    }
    None
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn plan_linux_browser_launch(
    file_url: &str,
    local_path: &Path,
    desktop_id: Option<&str>,
    has_gtk_launch: bool,
    has_gio: bool,
    data_dirs: &[PathBuf],
    browser_env: Option<&str>,
) -> Result<BrowserLaunch, String> {
    if let Some(id) = desktop_id.map(str::trim).filter(|id| !id.is_empty()) {
        if has_gtk_launch {
            return Ok(BrowserLaunch {
                argv: vec!["gtk-launch".into(), id.into(), file_url.into()],
                wait_for_exit: true,
            });
        }
        if let Some(desktop_file) = find_desktop_file_in_dirs(id, data_dirs) {
            if has_gio {
                return Ok(BrowserLaunch {
                    argv: vec![
                        "gio".into(),
                        "launch".into(),
                        desktop_file.to_string_lossy().into_owned(),
                        file_url.into(),
                    ],
                    wait_for_exit: true,
                });
            }
            if let Some(argv) = parse_desktop_entry_exec(&desktop_file, file_url, local_path) {
                return Ok(BrowserLaunch {
                    argv,
                    wait_for_exit: false,
                });
            }
            return Err(format!(
                "default web browser desktop entry {id} has no usable Exec command"
            ));
        }
        return Err(format!(
            "default web browser desktop entry {id} could not be launched without gtk-launch"
        ));
    }
    if let Some(argv) = browser_env.and_then(|raw| parse_browser_env(raw, file_url)) {
        return Ok(BrowserLaunch {
            argv,
            wait_for_exit: false,
        });
    }
    Err("no default web browser configured".into())
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
fn plan_windows_browser_launch(
    file_url: &str,
    command_argv: Option<Vec<String>>,
) -> Result<BrowserLaunch, String> {
    let Some(mut tokens) = command_argv.filter(|v| !v.is_empty() && !v[0].is_empty()) else {
        return Err("no default web browser command registered".into());
    };
    let mut substituted = false;
    for arg in tokens.iter_mut().skip(1) {
        if matches!(arg.as_str(), "%1" | "%L" | "%l" | "%*") {
            *arg = file_url.to_owned();
            substituted = true;
        } else if arg.contains("%1") {
            *arg = arg.replace("%1", file_url);
            substituted = true;
        } else if arg.contains("%L") {
            *arg = arg.replace("%L", file_url);
            substituted = true;
        }
    }
    if !substituted {
        tokens.push(file_url.to_owned());
    }
    Ok(BrowserLaunch {
        argv: tokens,
        wait_for_exit: false,
    })
}

#[cfg(target_os = "macos")]
fn default_macos_browser_bundle_id() -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSCopyDefaultHandlerForURLScheme(in_url_scheme: CFStringRef) -> CFStringRef;
    }

    for scheme in ["https", "http"] {
        let cf_scheme = CFString::new(scheme);
        let raw = unsafe { LSCopyDefaultHandlerForURLScheme(cf_scheme.as_concrete_TypeRef()) };
        if !raw.is_null() {
            let bundle_id = unsafe { CFString::wrap_under_create_rule(raw) }.to_string();
            if !bundle_id.is_empty() {
                return Some(bundle_id);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn default_linux_browser_desktop_id(path_env: &str) -> Option<String> {
    let query = |program: &str, args: &[&str]| -> Option<String> {
        let bin = find_executable(program, path_env)?;
        let output = Command::new(bin)
            .args(args)
            .env("PATH", path_env)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!value.is_empty() && !value.contains('\n')).then_some(value)
    };
    query("xdg-settings", &["get", "default-web-browser"])
        .or_else(|| query("xdg-mime", &["query", "default", "x-scheme-handler/https"]))
        .or_else(|| query("xdg-mime", &["query", "default", "x-scheme-handler/http"]))
}

#[cfg(windows)]
fn default_windows_browser_command_argv() -> Option<Vec<String>> {
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::UI::Shell::{
        AssocQueryStringW, CommandLineToArgvW, ASSOCF_IS_PROTOCOL, ASSOCSTR_COMMAND,
    };

    for scheme in [w!("https"), w!("http")] {
        let mut len = 0u32;
        let hr = unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_COMMAND,
                scheme,
                PCWSTR::null(),
                None,
                &mut len,
            )
        };
        if hr.is_err() || len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len as usize];
        let hr = unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                ASSOCSTR_COMMAND,
                scheme,
                PCWSTR::null(),
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
            )
        };
        if hr.is_err() {
            continue;
        }
        let mut argc = 0i32;
        let argv_ptr = unsafe { CommandLineToArgvW(PCWSTR(buf.as_ptr()), &mut argc) };
        if argv_ptr.is_null() {
            continue;
        }
        let mut args = Vec::new();
        if argc > 0 {
            let raw_args = unsafe { std::slice::from_raw_parts(argv_ptr, argc as usize) };
            args.reserve(raw_args.len());
            for &arg_ptr in raw_args {
                if let Ok(text) = unsafe { arg_ptr.to_string() } {
                    args.push(text);
                }
            }
        }
        unsafe {
            let _ = LocalFree(Some(HLOCAL(argv_ptr.cast())));
        }
        if !args.is_empty() && !args[0].is_empty() {
            return Some(args);
        }
    }
    None
}

fn plan_browser_launch(target: &FileLinkTarget, path_env: &str) -> Result<BrowserLaunch, String> {
    let file_url = path_to_file_url(&target.path)
        .ok_or_else(|| format!("cannot build file URL for {}", target.path.display()))?;
    #[cfg(target_os = "macos")]
    {
        let _ = path_env;
        plan_macos_browser_launch(&file_url, default_macos_browser_bundle_id().as_deref())
    }
    #[cfg(target_os = "linux")]
    {
        let desktop_id = default_linux_browser_desktop_id(path_env);
        let has_gtk_launch = find_executable("gtk-launch", path_env).is_some();
        let has_gio = find_executable("gio", path_env).is_some();
        let data_dirs = linux_xdg_data_dirs();
        let browser_env = std::env::var("BROWSER").ok();
        plan_linux_browser_launch(
            &file_url,
            &target.path,
            desktop_id.as_deref(),
            has_gtk_launch,
            has_gio,
            &data_dirs,
            browser_env.as_deref(),
        )
    }
    #[cfg(windows)]
    {
        let _ = path_env;
        plan_windows_browser_launch(&file_url, default_windows_browser_command_argv())
    }
}

fn editor_argv(editor: FileLinkEditor, target: &FileLinkTarget) -> Vec<String> {
    match editor {
        FileLinkEditor::Zed => vec!["zed".into(), target.locator()],
        FileLinkEditor::VsCode => vec!["code".into(), "--goto".into(), target.locator()],
        FileLinkEditor::Cursor => vec!["cursor".into(), "--goto".into(), target.locator()],
        FileLinkEditor::DefaultApp => open_argv(target),
    }
}

#[cfg(target_os = "macos")]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "/usr/bin/open".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

#[cfg(target_os = "linux")]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "xdg-open".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

#[cfg(windows)]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "rundll32.exe".into(),
        "url.dll,FileProtocolHandler".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

#[cfg(unix)]
fn reap_detached_child(mut child: std::process::Child) {
    // Reap on a dedicated OS thread rather than GPUI's background executor pool
    // so the caller returns immediately, no executor thread stays tied up while
    // a GUI browser runs, and the child never lingers as a zombie after exit.
    let _ = std::thread::Builder::new()
        .name("file-link-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

#[cfg(not(unix))]
fn reap_detached_child(_child: std::process::Child) {}

fn run_command_spec(argv: &[String], wait_for_exit: bool, path_env: &str) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("empty command")?;
    let program = if program.contains('/') || program.contains('\\') {
        PathBuf::from(program)
    } else {
        find_executable(program, path_env).ok_or_else(|| format!("{program} not found on PATH"))?
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !wait_for_exit {
        let child = cmd
            .spawn()
            .map_err(|error| format!("{}: {error}", argv.join(" ")))?;
        reap_detached_child(child);
        return Ok(());
    }
    let status = cmd
        .status()
        .map_err(|error| format!("{}: {error}", argv.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", argv.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(line: Option<u32>, column: Option<u32>) -> FileLinkTarget {
        FileLinkTarget {
            path: "/tmp/src/lib.rs".into(),
            line,
            column,
        }
    }

    fn file_target() -> LinkTarget {
        LinkTarget::File {
            path: "/repo/crates/runner-app/src/surfaces/panes.rs".into(),
            line: Some(1311),
            column: None,
        }
    }

    #[test]
    fn a_cmd_click_on_a_path_opens_it_with_the_side_panel_and_files_off() {
        use gpui::TestAppContext;
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.show_side_panel = false;
                    settings.files_view = false;
                    settings.file_link_editor = FileLinkEditor::Zed;
                    true
                },
                false,
                cx,
            );
        });
        LAUNCHED.with(|launched| launched.borrow_mut().clear());
        cx.update(|cx| open_file_link(target(Some(4), None), cx));
        assert_eq!(
            LAUNCHED.with(|launched| launched.borrow().clone()),
            [(
                FileLinkLaunch::Editor(FileLinkEditor::Zed),
                target(Some(4), None)
            )]
        );

        // Even with the side panel off, HTML routes to the browser and PDF to
        // the default app regardless of the chosen editor.
        LAUNCHED.with(|launched| launched.borrow_mut().clear());
        let html = FileLinkTarget {
            path: "/tmp/report.html".into(),
            line: None,
            column: None,
        };
        let pdf = FileLinkTarget {
            path: "/tmp/spec.pdf".into(),
            line: None,
            column: None,
        };
        cx.update(|cx| {
            open_file_link(html.clone(), cx);
            open_file_link(pdf.clone(), cx);
        });
        assert_eq!(
            LAUNCHED.with(|launched| launched.borrow().clone()),
            [
                (FileLinkLaunch::Browser, html),
                (FileLinkLaunch::DefaultApp, pdf),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_cmd_click_on_a_path_in_a_terminal_goes_through_its_surfaces_route() {
        use crate::terminal::control::tests::{attached, mirror_named};
        use crate::terminal::element::{TerminalElement, TerminalStyle};
        use gpui::{
            div, prelude::*, px, Modifiers, MouseButton, TestAppContext, VisualTestContext, Window,
        };
        use runner_core::protocol::terminal::TerminalController;

        struct Host {
            chat: crate::AttachedChat,
            routed: bool,
            asked: usize,
        }
        impl Host {
            fn view(&mut self, _: &mut Context<Self>) -> Option<Entity<FilesView>> {
                self.asked += 1;
                None
            }
            fn show(&mut self, _: &mut Context<Self>) {}
        }
        impl Render for Host {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let style = TerminalStyle {
                    palette: runner_terminal::palette::RUNNER,
                    font: crate::app_settings::AppSettings::default()
                        .terminal_font_family
                        .font(cx),
                    font_size: 14.,
                    app_zoom: 1.,
                    line_height_factor: crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
                    copy_on_select: false,
                };
                let route = self.routed.then(|| {
                    FileLinkRoute::new(
                        cx.weak_entity(),
                        Self::view,
                        |_, _| None,
                        Self::show,
                        |_, _| FileLinkRouteState {
                            local: true,
                            panel_open: true,
                        },
                    )
                });
                div().size_full().child(
                    div()
                        .debug_selector(|| "LINK_TERMINAL".into())
                        .w(px(800.))
                        .h(px(300.))
                        .child(
                            TerminalElement::new(
                                self.chat.terminal.clone(),
                                self.chat.terminal_interaction.clone(),
                                self.chat.terminal_input.clone(),
                                self.chat.terminal_focus.clone(),
                                true,
                                true,
                                style,
                            )
                            .file_links(route),
                        ),
                )
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().canonicalize().unwrap().join("cited.rs");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let mut cx = TestAppContext::single();
        crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let (_host, terminal, _tx, _ack) = mirror_named("links", TerminalController::Desktop);
        terminal.test_feed(1, format!("{}:2", file.display()).as_bytes());
        let window = cx.add_window(|_, cx| Host {
            chat: attached(terminal, cx, 100, 20),
            routed: true,
            asked: 0,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let bounds = visual.debug_bounds("LINK_TERMINAL").unwrap();
        let on_path = bounds.origin + gpui::point(px(30.), px(8.));
        let control = Modifiers {
            control: true,
            ..Default::default()
        };
        let expected = FileLinkTarget {
            path: file.clone(),
            line: Some(2),
            column: None,
        };
        // Hovering past the tooltip's delay asks the route from prepaint,
        // outside any update of its owner.
        visual.simulate_mouse_move(on_path, None, control);
        std::thread::sleep(std::time::Duration::from_millis(600));
        window.update(&mut visual, |_, _, cx| cx.notify()).unwrap();
        visual.run_until_parked();
        assert!(window.read_with(&visual, |host, _| host.asked).unwrap() >= 1);
        visual.simulate_mouse_move(gpui::point(px(0.), px(500.)), None, Modifiers::default());
        visual.run_until_parked();
        for routed in [true, false] {
            window
                .update(&mut visual, |host, _, _| host.asked = 0)
                .unwrap();
            window
                .update(&mut visual, |host, _, cx| {
                    host.routed = routed;
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            LAUNCHED.with(|launched| launched.borrow_mut().clear());
            visual.simulate_mouse_move(on_path, None, control);
            visual.simulate_mouse_down(on_path, MouseButton::Left, control);
            visual.simulate_mouse_up(on_path, MouseButton::Left, control);
            visual.run_until_parked();
            assert_eq!(
                window.read_with(&visual, |host, _| host.asked).unwrap(),
                usize::from(routed),
                "routed {routed}"
            );
            // The route had no preview to give, so the editor opens it.
            assert_eq!(
                LAUNCHED.with(|launched| launched.borrow().clone()),
                [(
                    FileLinkLaunch::Editor(FileLinkEditor::DefaultApp),
                    expected.clone()
                )],
                "routed {routed}"
            );
        }
    }

    #[test]
    fn link_context_reads_existing_files_root_without_fetching() {
        use gpui::TestAppContext;
        struct Host {
            asked: usize,
            existing: Option<Entity<FilesView>>,
        }
        impl Host {
            fn view(&mut self, _: &mut Context<Self>) -> Option<Entity<FilesView>> {
                self.asked += 1;
                None
            }
            fn existing_view(&self, _: &App) -> Option<Entity<FilesView>> {
                self.existing.clone()
            }
            fn show(&mut self, _: &mut Context<Self>) {}
        }

        let mut cx = TestAppContext::single();
        let host = cx.new(|_| Host {
            asked: 0,
            existing: None,
        });
        let route = FileLinkRoute::new(
            host.downgrade(),
            Host::view,
            Host::existing_view,
            Host::show,
            |_, _| FileLinkRouteState {
                local: true,
                panel_open: true,
            },
        );
        let context = cx.read(|cx| route.link_context(cx));
        assert_eq!(context, FileLinkContext::default());
        assert_eq!(
            host.read_with(&cx, |host, _| host.asked),
            0,
            "link_context must stay synchronous and never invoke the lazy view fetcher"
        );
    }

    #[test]
    fn decide_file_link_covers_routing_table_settings_panel_states_and_remote() {
        let small = Some(1024);
        let at_limit = Some(MAX_READ_BYTES);
        let over_limit = Some(MAX_READ_BYTES + 1);
        let unknown = FileLinkContent::Unknown;

        // Runner preview with panel shown or collapsed.
        for panel in [FileLinkPanelState::Shown, FileLinkPanelState::Collapsed] {
            assert_eq!(
                decide_file_link(
                    Path::new("src/lib.rs"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Preview
            );
            assert_eq!(
                decide_file_link(
                    Path::new("README.md"),
                    at_limit,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Preview
            );
            assert_eq!(
                decide_file_link(
                    Path::new("Makefile"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Preview
            );
            for image in [
                "a.png", "b.jpg", "c.JPEG", "d.gif", "e.webp", "f.svg", "g.bmp",
            ] {
                assert_eq!(
                    decide_file_link(
                        Path::new(image),
                        small,
                        unknown,
                        true,
                        FileLinkOpenIn::Preview,
                        panel
                    ),
                    FileLinkDecision::Preview,
                    "{image}"
                );
            }
            for html in ["target/report.html", "index.HTM"] {
                assert_eq!(
                    decide_file_link(
                        Path::new(html),
                        small,
                        unknown,
                        true,
                        FileLinkOpenIn::Preview,
                        panel
                    ),
                    FileLinkDecision::Browser,
                    "{html}"
                );
            }
            for binary in [
                "spec.pdf",
                "plan.docx",
                "sheet.xlsx",
                "deck.pptx",
                "bundle.zip",
                "backup.tar.gz",
                "song.mp3",
                "clip.mp4",
            ] {
                assert_eq!(
                    decide_file_link(
                        Path::new(binary),
                        small,
                        unknown,
                        true,
                        FileLinkOpenIn::Preview,
                        panel
                    ),
                    FileLinkDecision::DefaultApp,
                    "{binary}"
                );
            }
            for exec in ["setup.exe", "installer.msi", "app.pkg", "tool.jar"] {
                assert_eq!(
                    decide_file_link(
                        Path::new(exec),
                        small,
                        unknown,
                        true,
                        FileLinkOpenIn::Preview,
                        panel
                    ),
                    FileLinkDecision::Preview,
                    "{exec} stays on safe preview inspection path"
                );
            }
            assert_eq!(
                decide_file_link(
                    Path::new("logs/build.log"),
                    over_limit,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Preview
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    FileLinkContent::Binary,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::DefaultApp
            );
            assert_eq!(
                decide_file_link(
                    Path::new("tool.bin"),
                    small,
                    FileLinkContent::Executable,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Preview
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    over_limit,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
        }

        // Editor chosen: text, code, Markdown, images, executables and unknown text open in Editor;
        // HTML, known binary and sniffed unknown binary route to browser / default app.
        for panel in [
            FileLinkPanelState::Shown,
            FileLinkPanelState::Collapsed,
            FileLinkPanelState::PanelOff,
            FileLinkPanelState::FilesOff,
        ] {
            assert_eq!(
                decide_file_link(
                    Path::new("src/lib.rs"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("docs/crews.png"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("target/report.html"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Browser
            );
            assert_eq!(
                decide_file_link(
                    Path::new("spec.pdf"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::DefaultApp
            );
            for exec in ["setup.exe", "installer.msi"] {
                assert_eq!(
                    decide_file_link(
                        Path::new(exec),
                        small,
                        unknown,
                        true,
                        FileLinkOpenIn::Editor,
                        panel
                    ),
                    FileLinkDecision::Editor,
                    "{exec} must not route to DefaultApp"
                );
            }
            assert_eq!(
                decide_file_link(
                    Path::new("logs/build.log"),
                    over_limit,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    FileLinkContent::Binary,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::DefaultApp
            );
            assert_eq!(
                decide_file_link(
                    Path::new("tool.bin"),
                    small,
                    FileLinkContent::Executable,
                    true,
                    FileLinkOpenIn::Editor,
                    panel
                ),
                FileLinkDecision::Editor
            );
        }

        // Runner preview with the side panel or Files turned off falls back to Editor,
        // while HTML, known binary and sniffed unknown binary keep their own apps.
        for panel in [FileLinkPanelState::PanelOff, FileLinkPanelState::FilesOff] {
            assert_eq!(
                decide_file_link(
                    Path::new("src/lib.rs"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("docs/crews.png"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("dump.dat"),
                    small,
                    FileLinkContent::Binary,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::DefaultApp
            );
            assert_eq!(
                decide_file_link(
                    Path::new("setup.exe"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Editor
            );
            assert_eq!(
                decide_file_link(
                    Path::new("target/report.html"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::Browser
            );
            assert_eq!(
                decide_file_link(
                    Path::new("spec.pdf"),
                    small,
                    unknown,
                    true,
                    FileLinkOpenIn::Preview,
                    panel
                ),
                FileLinkDecision::DefaultApp
            );
        }

        // Remote sessions keep today's behaviour for every path and setting.
        for path in [
            "src/lib.rs",
            "docs/crews.png",
            "report.html",
            "spec.pdf",
            "dump.dat",
            "setup.exe",
        ] {
            for open_in in [FileLinkOpenIn::Preview, FileLinkOpenIn::Editor] {
                assert_eq!(
                    decide_file_link(
                        Path::new(path),
                        small,
                        unknown,
                        false,
                        open_in,
                        FileLinkPanelState::Shown
                    ),
                    FileLinkDecision::Editor,
                    "{path} {open_in:?}"
                );
            }
        }

        // Settings panel-off fallback uses the same decision function.
        assert_eq!(settings_panel_fallback(&AppSettings::default()), None);
        assert_eq!(
            settings_panel_fallback(&AppSettings {
                show_side_panel: false,
                ..AppSettings::default()
            }),
            Some(FileLinkPanelState::PanelOff)
        );
        assert_eq!(
            settings_panel_fallback(&AppSettings {
                files_view: false,
                ..AppSettings::default()
            }),
            Some(FileLinkPanelState::FilesOff)
        );
        assert_eq!(
            settings_panel_fallback(&AppSettings {
                file_link_open_in: FileLinkOpenIn::Editor,
                show_side_panel: false,
                ..AppSettings::default()
            }),
            None
        );
    }

    #[test]
    fn link_tooltip_teaches_the_gesture_then_names_the_action() {
        for modifier in ["⌘", "Ctrl"] {
            let url = LinkTarget::Url("https://example.com".into());
            assert_eq!(
                link_tooltip_for(
                    &url,
                    false,
                    FileLinkDecision::Browser,
                    true,
                    FileLinkEditor::Zed,
                    None,
                    modifier,
                )
                .as_deref(),
                Some(format!("{modifier}-click to open in browser").as_str())
            );
            assert_eq!(
                link_tooltip_for(
                    &url,
                    true,
                    FileLinkDecision::Browser,
                    true,
                    FileLinkEditor::Zed,
                    None,
                    modifier,
                )
                .as_deref(),
                Some("Open in browser")
            );

            let file = file_target();
            assert_eq!(
                link_tooltip_for(
                    &file,
                    false,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::Zed,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some(format!("{modifier}-click to open in Zed").as_str())
            );
            assert_eq!(
                link_tooltip_for(
                    &file,
                    true,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::Zed,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some("Open in Zed · panes.rs:1311")
            );
            assert_eq!(
                link_tooltip_for(
                    &file,
                    false,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::VsCode,
                    Some(false),
                    modifier,
                )
                .as_deref(),
                Some(format!("{modifier}-click to open in default app").as_str())
            );
            assert_eq!(
                link_tooltip_for(
                    &file,
                    true,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::VsCode,
                    Some(false),
                    modifier,
                )
                .as_deref(),
                Some("Open in default app · code not on PATH")
            );
            assert_eq!(
                link_tooltip_for(
                    &file,
                    true,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::DefaultApp,
                    None,
                    modifier,
                )
                .as_deref(),
                Some("Open in default app · line 1311 is lost")
            );

            let no_line = LinkTarget::File {
                path: "/repo/README.md".into(),
                line: None,
                column: None,
            };
            assert_eq!(
                link_tooltip_for(
                    &no_line,
                    true,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::DefaultApp,
                    None,
                    modifier,
                )
                .as_deref(),
                Some("Open in default app · README.md")
            );
            assert_eq!(
                link_tooltip_for(
                    &no_line,
                    true,
                    FileLinkDecision::Editor,
                    true,
                    FileLinkEditor::Cursor,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some("Open in Cursor · README.md")
            );

            // Browser and DefaultApp decisions ignore the chosen editor and its PATH state.
            let html = LinkTarget::File {
                path: "/tmp/report.html".into(),
                line: None,
                column: None,
            };
            assert_eq!(
                link_tooltip_for(
                    &html,
                    false,
                    FileLinkDecision::Browser,
                    true,
                    FileLinkEditor::Zed,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some(format!("{modifier}-click to open in browser").as_str())
            );
            assert_eq!(
                link_tooltip_for(
                    &html,
                    true,
                    FileLinkDecision::Browser,
                    true,
                    FileLinkEditor::Zed,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some("Open in browser · report.html")
            );

            let pdf = LinkTarget::File {
                path: "/tmp/spec.pdf".into(),
                line: None,
                column: None,
            };
            assert_eq!(
                link_tooltip_for(
                    &pdf,
                    false,
                    FileLinkDecision::DefaultApp,
                    true,
                    FileLinkEditor::VsCode,
                    Some(false),
                    modifier,
                )
                .as_deref(),
                Some(format!("{modifier}-click to open in default app").as_str())
            );
            assert_eq!(
                link_tooltip_for(
                    &pdf,
                    true,
                    FileLinkDecision::DefaultApp,
                    true,
                    FileLinkEditor::VsCode,
                    Some(false),
                    modifier,
                )
                .as_deref(),
                Some("Open in default app · spec.pdf")
            );

            // Guarded local executable/installer links suppress the tooltip when
            // no explicit editor CLI is available, rather than promising Default app.
            for guarded in ["/tmp/setup.exe", "/tmp/installer.msi"] {
                let target = LinkTarget::File {
                    path: guarded.into(),
                    line: None,
                    column: None,
                };
                for held in [false, true] {
                    assert_eq!(
                        link_tooltip_for(
                            &target,
                            held,
                            FileLinkDecision::Editor,
                            true,
                            FileLinkEditor::DefaultApp,
                            None,
                            modifier,
                        ),
                        None,
                        "local {guarded} with DefaultApp must not promise an app"
                    );
                    assert_eq!(
                        link_tooltip_for(
                            &target,
                            held,
                            FileLinkDecision::Editor,
                            true,
                            FileLinkEditor::VsCode,
                            Some(false),
                            modifier,
                        ),
                        None,
                        "local {guarded} with missing CLI must not promise default app"
                    );
                }
                // Available explicit editor and preview tab keep honest tooltips.
                assert_eq!(
                    link_tooltip_for(
                        &target,
                        false,
                        FileLinkDecision::Editor,
                        true,
                        FileLinkEditor::Zed,
                        Some(true),
                        modifier,
                    )
                    .as_deref(),
                    Some(format!("{modifier}-click to open in Zed").as_str())
                );
                assert_eq!(
                    link_tooltip_for(
                        &target,
                        false,
                        FileLinkDecision::Preview,
                        true,
                        FileLinkEditor::DefaultApp,
                        None,
                        modifier,
                    )
                    .as_deref(),
                    Some(format!("{modifier}-click to open in side panel").as_str())
                );
                // Remote links preserve legacy Editor tooltip even with DefaultApp.
                assert_eq!(
                    link_tooltip_for(
                        &target,
                        false,
                        FileLinkDecision::Editor,
                        false,
                        FileLinkEditor::DefaultApp,
                        None,
                        modifier,
                    )
                    .as_deref(),
                    Some(format!("{modifier}-click to open in default app").as_str())
                );
            }
        }
    }

    #[test]
    fn link_tooltip_names_the_side_panel_when_the_preview_takes_the_file() {
        for modifier in ["⌘", "Ctrl"] {
            let file = file_target();
            for editor in [FileLinkEditor::Zed, FileLinkEditor::DefaultApp] {
                assert_eq!(
                    link_tooltip_for(
                        &file,
                        false,
                        FileLinkDecision::Preview,
                        true,
                        editor,
                        Some(false),
                        modifier,
                    )
                    .as_deref(),
                    Some(format!("{modifier}-click to open in side panel").as_str())
                );
                assert_eq!(
                    link_tooltip_for(
                        &file,
                        true,
                        FileLinkDecision::Preview,
                        true,
                        editor,
                        Some(false),
                        modifier,
                    )
                    .as_deref(),
                    Some("Open in side panel · panes.rs:1311")
                );
            }
            let no_line = LinkTarget::File {
                path: "/repo/README.md".into(),
                line: None,
                column: None,
            };
            assert_eq!(
                link_tooltip_for(
                    &no_line,
                    true,
                    FileLinkDecision::Preview,
                    true,
                    FileLinkEditor::Zed,
                    Some(true),
                    modifier,
                )
                .as_deref(),
                Some("Open in side panel · README.md")
            );
            let url = LinkTarget::Url("https://example.com".into());
            assert_eq!(
                link_tooltip_for(
                    &url,
                    true,
                    FileLinkDecision::Preview,
                    true,
                    FileLinkEditor::Zed,
                    None,
                    modifier,
                )
                .as_deref(),
                Some("Open in browser")
            );
        }
    }

    #[test]
    fn editor_argv_passes_line_and_column_through_each_cli() {
        assert_eq!(
            editor_argv(FileLinkEditor::Zed, &target(Some(12), Some(5))),
            ["zed", "/tmp/src/lib.rs:12:5"]
        );
        assert_eq!(
            editor_argv(FileLinkEditor::VsCode, &target(Some(12), None)),
            ["code", "--goto", "/tmp/src/lib.rs:12"]
        );
        assert_eq!(
            editor_argv(FileLinkEditor::Cursor, &target(None, Some(3))),
            ["cursor", "--goto", "/tmp/src/lib.rs"]
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            editor_argv(FileLinkEditor::DefaultApp, &target(Some(1), None)),
            ["/usr/bin/open", "/tmp/src/lib.rs"]
        );
        #[cfg(windows)]
        assert_eq!(
            editor_argv(FileLinkEditor::DefaultApp, &target(Some(1), None)),
            [
                "rundll32.exe",
                "url.dll,FileProtocolHandler",
                "/tmp/src/lib.rs"
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn open_argv_passes_windows_paths_verbatim() {
        for path in [
            r"C:\Users\Jason Wang\项目\notes & %PATH% !.txt",
            r"\\server\shared files\notes.txt",
        ] {
            let target = FileLinkTarget {
                path: path.into(),
                line: Some(12),
                column: Some(5),
            };
            assert_eq!(
                open_argv(&target),
                ["rundll32.exe", "url.dll,FileProtocolHandler", path]
            );
        }
    }

    #[test]
    fn executable_and_installer_links_stay_on_safe_inspection_path() {
        use gpui::TestAppContext;

        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("setup.exe");
        let msi = temp.path().join("installer.msi");
        let elf = temp.path().join("tool.bin");
        let bat = temp.path().join("build.bat");
        std::fs::write(&exe, b"MZ\0\0inert-exe").unwrap();
        std::fs::write(&msi, b"\xD0\xCF\x11\xE0\0inert-msi").unwrap();
        std::fs::write(&elf, b"\x7fELF\0\x01\x02inert-elf").unwrap();
        std::fs::write(&bat, "@echo off\r\n").unwrap();

        assert_eq!(sniff_file_content(&elf), FileLinkContent::Executable);
        assert_eq!(binary_kind_label(&exe), "an executable");
        assert_eq!(binary_kind_label(&msi), "an installer");

        for (path, content) in [
            (&exe, FileLinkContent::Unknown),
            (&msi, FileLinkContent::Unknown),
            (&elf, FileLinkContent::Executable),
        ] {
            let t = FileLinkTarget {
                path: path.clone(),
                line: None,
                column: None,
            };
            // Automatic local terminal dispatch: explicit text editor opens
            // without falling back to the OS open verb, and DefaultApp skips.
            assert_eq!(
                plan_editor_launch(FileLinkEditor::Zed, &t, content, LaunchMode::LocalAutomatic),
                LaunchPlan::Run {
                    argv: vec!["zed".into(), path.to_string_lossy().into_owned()],
                    fallback: None,
                }
            );
            assert_eq!(
                plan_editor_launch(
                    FileLinkEditor::DefaultApp,
                    &t,
                    content,
                    LaunchMode::LocalAutomatic
                ),
                LaunchPlan::SkipUnsafeOpen
            );
            assert_eq!(
                plan_default_app_launch(&t, content, LaunchMode::LocalAutomatic),
                LaunchPlan::SkipUnsafeOpen
            );

            // Explicit viewer actions ("Open in default app" button / Open in menu)
            // keep their existing launch behavior below the viewer's cfg(test) stub.
            let mut explicit_probes = 0usize;
            let mut explicit_runs = Vec::new();
            execute_editor_launch(
                FileLinkEditor::DefaultApp,
                &t,
                FileLinkContent::Unknown,
                LaunchMode::ExplicitOrRemote,
                "",
                |_| {
                    explicit_probes += 1;
                    FileLinkContent::Executable
                },
                |argv, _| {
                    explicit_runs.push(argv.to_vec());
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(explicit_probes, 0);
            assert_eq!(explicit_runs, [open_argv(&t)]);

            // Automatic local terminal dispatch with a recording launcher skips
            // DefaultApp and never falls back to open_argv when Zed fails.
            let mut auto_runs = Vec::new();
            execute_editor_launch(
                FileLinkEditor::DefaultApp,
                &t,
                content,
                LaunchMode::LocalAutomatic,
                "",
                sniff_file_content,
                |argv, _| {
                    auto_runs.push(argv.to_vec());
                    Ok(())
                },
            )
            .unwrap();
            assert!(
                auto_runs.is_empty(),
                "automatic local DefaultApp must not invoke launcher for {}",
                path.display()
            );

            let mut failed_zed_runs = Vec::new();
            let err = execute_editor_launch(
                FileLinkEditor::Zed,
                &t,
                content,
                LaunchMode::LocalAutomatic,
                "",
                sniff_file_content,
                |argv, _| {
                    failed_zed_runs.push(argv.to_vec());
                    Err("zed missing".into())
                },
            );
            assert!(err.is_err());
            assert_eq!(
                failed_zed_runs,
                [editor_argv(FileLinkEditor::Zed, &t)],
                "automatic local Zed failure must not fall back to open_argv"
            );
        }

        // Explicit viewer Open in on .bat still runs its configured launcher.
        let bat_target = FileLinkTarget {
            path: bat,
            line: None,
            column: None,
        };
        let mut bat_runs = Vec::new();
        execute_editor_launch(
            FileLinkEditor::DefaultApp,
            &bat_target,
            FileLinkContent::Unknown,
            LaunchMode::ExplicitOrRemote,
            "",
            |_| unreachable!("explicit viewer action must not probe"),
            |argv, _| {
                bat_runs.push(argv.to_vec());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(bat_runs, [open_argv(&bat_target)]);

        // Remote unknown-suffix link matching a real local ELF binary keeps the
        // legacy editor + fallback path with zero local filesystem probes.
        let remote_same_name = FileLinkTarget {
            path: elf.clone(),
            line: Some(9),
            column: None,
        };
        let mut remote_probes = 0usize;
        let mut remote_runs = Vec::new();
        execute_editor_launch(
            FileLinkEditor::DefaultApp,
            &remote_same_name,
            FileLinkContent::Unknown,
            LaunchMode::ExplicitOrRemote,
            "",
            |_| {
                remote_probes += 1;
                FileLinkContent::Executable
            },
            |argv, _| {
                remote_runs.push(argv.to_vec());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            remote_probes, 0,
            "remote launch must never probe local disk"
        );
        assert_eq!(remote_runs, [open_argv(&remote_same_name)]);

        let mut remote_fallback_probes = 0usize;
        let mut remote_fallback_runs = Vec::new();
        execute_editor_launch(
            FileLinkEditor::Zed,
            &remote_same_name,
            FileLinkContent::Unknown,
            LaunchMode::ExplicitOrRemote,
            "",
            |_| {
                remote_fallback_probes += 1;
                FileLinkContent::Executable
            },
            |argv, _| {
                remote_fallback_runs.push(argv.to_vec());
                if argv[0] == "zed" {
                    Err("zed missing".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(remote_fallback_probes, 0);
        assert_eq!(
            remote_fallback_runs,
            [
                editor_argv(FileLinkEditor::Zed, &remote_same_name),
                open_argv(&remote_same_name),
            ]
        );

        // Dispatch-only regression with inert fixtures: automatic local
        // DefaultApp dispatches nothing, explicit open_file_with dispatches,
        // and automatic local Zed dispatches to Zed.
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.file_link_open_in = FileLinkOpenIn::Editor;
                    settings.file_link_editor = FileLinkEditor::DefaultApp;
                    true
                },
                false,
                cx,
            );
        });
        LAUNCHED.with(|l| l.borrow_mut().clear());
        cx.update(|cx| {
            open_file_link(
                FileLinkTarget {
                    path: exe.clone(),
                    line: None,
                    column: None,
                },
                cx,
            );
            open_file_link(
                FileLinkTarget {
                    path: msi.clone(),
                    line: None,
                    column: None,
                },
                cx,
            );
            open_file_link(
                FileLinkTarget {
                    path: elf.clone(),
                    line: None,
                    column: None,
                },
                cx,
            );
        });
        cx.run_until_parked();
        assert!(
            LAUNCHED.with(|l| l.borrow().is_empty()),
            "automatic local DefaultApp must never launch .exe, .msi, or ELF binaries"
        );

        // Explicit viewer action on the same .msi fixture still dispatches.
        let msi_target = FileLinkTarget {
            path: msi.clone(),
            line: None,
            column: None,
        };
        cx.update(|cx| open_file_with(FileLinkEditor::DefaultApp, msi_target.clone(), cx));
        assert_eq!(
            LAUNCHED.with(|l| l.borrow().clone()),
            [(
                FileLinkLaunch::Editor(FileLinkEditor::DefaultApp),
                msi_target.clone()
            )]
        );

        store.update(&mut cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.file_link_editor = FileLinkEditor::Zed;
                    true
                },
                false,
                cx,
            );
        });
        LAUNCHED.with(|l| l.borrow_mut().clear());
        let exe_target = FileLinkTarget {
            path: exe,
            line: None,
            column: None,
        };
        cx.update(|cx| {
            open_file_link(exe_target.clone(), cx);
            open_file_link(msi_target.clone(), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            LAUNCHED.with(|l| l.borrow().clone()),
            [
                (FileLinkLaunch::Editor(FileLinkEditor::Zed), exe_target),
                (FileLinkLaunch::Editor(FileLinkEditor::Zed), msi_target),
            ]
        );
    }

    #[test]
    fn browser_argv_builds_explicit_browser_commands_and_safe_file_urls() {
        let temp = tempfile::tempdir().unwrap();

        #[cfg(unix)]
        {
            assert_eq!(
                path_to_file_url(Path::new("/tmp/my report #1?x=2%3.html")).as_deref(),
                Some("file:///tmp/my%20report%20%231%3Fx=2%253.html")
            );
            assert_eq!(
                path_to_file_url(Path::new("/tmp/-flag.html")).as_deref(),
                Some("file:///tmp/-flag.html")
            );
            assert_eq!(
                path_to_file_url(Path::new("/tmp/中文.html")).as_deref(),
                Some("file:///tmp/%E4%B8%AD%E6%96%87.html")
            );
        }

        // macOS explicit bundle ID; missing handler errors rather than falling back to open.
        assert_eq!(
            plan_macos_browser_launch("file:///tmp/report.html", Some("com.apple.Safari")),
            Ok(BrowserLaunch {
                argv: vec![
                    "/usr/bin/open".into(),
                    "-b".into(),
                    "com.apple.Safari".into(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: true,
            })
        );
        assert!(plan_macos_browser_launch("file:///tmp/report.html", None).is_err());

        // Linux: gtk-launch, GIO desktop entry launch without gtk-launch, direct
        // desktop Exec= launch without gtk-launch/gio, and failure instead of xdg-open.
        let xdg_data = temp.path().join("xdg-data");
        let apps_dir = xdg_data.join("applications");
        std::fs::create_dir_all(&apps_dir).unwrap();
        let desktop_file = apps_dir.join("org.mozilla.firefox.desktop");
        std::fs::write(
            &desktop_file,
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=/usr/bin/firefox --new-tab %u\n",
        )
        .unwrap();
        let data_dirs = vec![xdg_data];
        let local_html = Path::new("/tmp/report.html");

        assert_eq!(
            plan_linux_browser_launch(
                "file:///tmp/report.html",
                local_html,
                Some("org.mozilla.firefox.desktop"),
                true,
                true,
                &data_dirs,
                Some("chromium"),
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    "gtk-launch".into(),
                    "org.mozilla.firefox.desktop".into(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: true,
            })
        );
        // Known desktop ID with NO gtk-launch and NO $BROWSER uses GIO launch (not xdg-open).
        assert_eq!(
            plan_linux_browser_launch(
                "file:///tmp/report.html",
                local_html,
                Some("org.mozilla.firefox.desktop"),
                false,
                true,
                &data_dirs,
                None,
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    "gio".into(),
                    "launch".into(),
                    desktop_file.to_string_lossy().into_owned(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: true,
            })
        );
        // Known desktop ID with neither gtk-launch nor gio spawns the desktop entry's Exec directly.
        assert_eq!(
            plan_linux_browser_launch(
                "file:///tmp/report.html",
                local_html,
                Some("org.mozilla.firefox.desktop"),
                false,
                false,
                &data_dirs,
                None,
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    "/usr/bin/firefox".into(),
                    "--new-tab".into(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: false,
            })
        );
        // Unresolvable known desktop ID fails cleanly instead of invoking xdg-open.
        assert!(plan_linux_browser_launch(
            "file:///tmp/report.html",
            local_html,
            Some("missing-browser.desktop"),
            false,
            false,
            &data_dirs,
            None,
        )
        .is_err());
        // $BROWSER when no desktop ID is set spawns directly without waiting for exit.
        assert_eq!(
            plan_linux_browser_launch(
                "file:///tmp/report.html",
                local_html,
                None,
                false,
                false,
                &data_dirs,
                Some("/usr/bin/brave --new-window %s:/usr/bin/firefox"),
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    "/usr/bin/brave".into(),
                    "--new-window".into(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: false,
            })
        );
        assert!(plan_linux_browser_launch(
            "file:///tmp/report.html",
            local_html,
            None,
            false,
            false,
            &data_dirs,
            None,
        )
        .is_err());

        // Windows browser command argv substitution (spawns directly without waiting for exit;
        // missing handler fails instead of falling back to rundll32).
        assert_eq!(
            plan_windows_browser_launch(
                "file:///C:/Users/Jason/report.html",
                Some(vec![
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                    "--single-argument".into(),
                    "%1".into(),
                ]),
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                    "--single-argument".into(),
                    "file:///C:/Users/Jason/report.html".into(),
                ],
                wait_for_exit: false,
            })
        );
        assert_eq!(
            plan_windows_browser_launch(
                "file:///C:/Users/Jason/report.html",
                Some(vec![
                    r"C:\Program Files\Mozilla Firefox\firefox.exe".into(),
                    "-osint".into(),
                    "-url".into(),
                    "%1".into(),
                ]),
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    r"C:\Program Files\Mozilla Firefox\firefox.exe".into(),
                    "-osint".into(),
                    "-url".into(),
                    "file:///C:/Users/Jason/report.html".into(),
                ],
                wait_for_exit: false,
            })
        );
        assert_eq!(
            plan_windows_browser_launch(
                "file:///C:/Users/Jason/report.html",
                Some(vec![r"C:\Browser\app.exe".into()]),
            ),
            Ok(BrowserLaunch {
                argv: vec![
                    r"C:\Browser\app.exe".into(),
                    "file:///C:/Users/Jason/report.html".into(),
                ],
                wait_for_exit: false,
            })
        );
        assert!(plan_windows_browser_launch("file:///C:/Users/Jason/report.html", None).is_err());

        // Browser launch failure reports the error and never falls back to open_argv.
        let mut browser_attempts = Vec::new();
        let failed = execute_browser_launch(
            Ok(BrowserLaunch {
                argv: vec![
                    "/usr/bin/open".into(),
                    "-b".into(),
                    "com.example.Browser".into(),
                    "file:///tmp/report.html".into(),
                ],
                wait_for_exit: true,
            }),
            "",
            |launch, _| {
                browser_attempts.push(launch.clone());
                Err("browser failed".into())
            },
        );
        assert_eq!(failed, Err("browser failed".into()));
        assert_eq!(
            browser_attempts.len(),
            1,
            "failed browser launch must not fall back to file association"
        );

        #[cfg(target_os = "macos")]
        {
            let html = temp.path().join("my report #1.html");
            std::fs::write(&html, "<h1>ok</h1>").unwrap();
            let launch = plan_browser_launch(
                &FileLinkTarget {
                    path: html,
                    line: None,
                    column: None,
                },
                "",
            )
            .expect("macOS should resolve a default web browser bundle");
            assert!(launch.wait_for_exit);
            assert_eq!(launch.argv[0], "/usr/bin/open");
            assert_eq!(
                launch.argv[1], "-b",
                "macOS must target the default browser bundle: {:?}",
                launch.argv
            );
            assert!(
                !launch.argv[2].is_empty(),
                "bundle id must be non-empty: {:?}",
                launch.argv
            );
            assert!(
                launch.argv[3].starts_with("file:///")
                    && launch.argv[3].ends_with("/my%20report%20%231.html"),
                "file URL must percent-encode spaces and #: {:?}",
                launch.argv
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn direct_browser_spawn_returns_promptly_and_reaps_child_on_exit() {
        use std::io::Write as _;
        use std::sync::mpsc;

        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("child.pid");
        let gate_fifo = temp.path().join("release.fifo");
        let status = Command::new("mkfifo").arg(&gate_fifo).status().unwrap();
        assert!(status.success(), "mkfifo failed");

        // Pre-open the FIFO read/write so opening never blocks and every exit
        // path (including a timeout/panic unwind) can release the child cleanly.
        let gate_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&gate_fifo)
            .unwrap();

        struct ReleaseOnDrop {
            gate_file: std::fs::File,
            released: bool,
            worker: Option<std::thread::JoinHandle<()>>,
        }
        impl ReleaseOnDrop {
            fn release(&mut self) {
                if !self.released {
                    self.released = true;
                    let _ = self.gate_file.write_all(b"go\n");
                }
            }
        }
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.release();
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
            }
        }

        let mut gate = ReleaseOnDrop {
            gate_file,
            released: false,
            worker: None,
        };

        // Pure POSIX shell builtins only (`exec`, `printf`, `read`), so `PATH=""`
        // cannot affect execution. Launch on a worker thread so a regression to
        // waiting for exit fails `recv_timeout` instead of hanging `cargo test`.
        let argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            r#"exec 3<"$2"; printf '%s\n' "$$" > "$1"; read _ <&3"#.into(),
            "sh".into(),
            pid_file.to_string_lossy().into_owned(),
            gate_fifo.to_string_lossy().into_owned(),
        ];
        let (tx, rx) = mpsc::channel();
        gate.worker = Some(std::thread::spawn(move || {
            let _ = tx.send(run_command_spec(&argv, false, ""));
        }));

        let launch_result = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("direct browser spawn blocked instead of returning before the gate opened");
        launch_result.unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let pid: u32 = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && text.ends_with('\n')
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for detached child to report its PID"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        let proc_state = |pid: u32| -> Option<String> {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let stat = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            (!stat.is_empty()).then_some(stat)
        };

        // The child must still be alive (non-zombie) after `run_command_spec` returned.
        let alive_state = proc_state(pid).expect("child should still be running before release");
        assert!(
            !alive_state.starts_with('Z'),
            "child should be blocked on gate FIFO, not zombie: {alive_state}"
        );

        // Release the gate FIFO so the child exits, then verify it is reaped
        // completely (absent from `ps`, not lingering as a `Z` defunct entry).
        gate.release();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match proc_state(pid) {
                None => break,
                Some(state) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "detached child {pid} was not reaped (ps stat={state:?})"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn unknown_extension_binary_routes_to_default_app_when_editor_is_selected_or_panel_off() {
        use gpui::TestAppContext;

        let temp = tempfile::tempdir().unwrap();
        let bin_unknown = temp.path().join("dump.dat");
        let text_unknown = temp.path().join("notes.dat");
        let elf_unknown = temp.path().join("tool.bin");
        std::fs::write(&bin_unknown, b"\x89BIN\0\x01\x02").unwrap();
        std::fs::write(&text_unknown, "plain text unknown ext\n").unwrap();
        std::fs::write(&elf_unknown, b"\x7fELF\0\x01\x02").unwrap();

        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());

        for (open_in, show_side_panel) in [
            (FileLinkOpenIn::Editor, true),
            (FileLinkOpenIn::Preview, false),
        ] {
            store.update(&mut cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        settings.file_link_open_in = open_in;
                        settings.file_link_editor = FileLinkEditor::Zed;
                        settings.show_side_panel = show_side_panel;
                        true
                    },
                    false,
                    cx,
                );
            });
            let panel = FileLinkPanelState::from_settings(
                &store.read_with(&cx, |s, _| s.settings.clone()),
                true,
            );
            // Hover tooltip before content sniff keeps the extension-based Editor hint.
            assert_eq!(
                decide_file_link(
                    &bin_unknown,
                    None,
                    FileLinkContent::Unknown,
                    true,
                    open_in,
                    panel
                ),
                FileLinkDecision::Editor
            );

            let bin_target = FileLinkTarget {
                path: bin_unknown.clone(),
                line: None,
                column: None,
            };
            let text_target = FileLinkTarget {
                path: text_unknown.clone(),
                line: None,
                column: None,
            };
            let elf_target = FileLinkTarget {
                path: elf_unknown.clone(),
                line: None,
                column: None,
            };

            LAUNCHED.with(|l| l.borrow_mut().clear());
            cx.update(|cx| open_local_non_preview(bin_target.clone(), open_in, panel, cx));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while LAUNCHED.with(|l| l.borrow().is_empty()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for binary sniff"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
                cx.run_until_parked();
            }
            assert_eq!(
                LAUNCHED.with(|l| l.borrow().clone()),
                [(FileLinkLaunch::DefaultApp, bin_target)]
            );

            LAUNCHED.with(|l| l.borrow_mut().clear());
            cx.update(|cx| open_local_non_preview(text_target.clone(), open_in, panel, cx));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while LAUNCHED.with(|l| l.borrow().is_empty()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for text sniff"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
                cx.run_until_parked();
            }
            assert_eq!(
                LAUNCHED.with(|l| l.borrow().clone()),
                [(FileLinkLaunch::Editor(FileLinkEditor::Zed), text_target)]
            );

            LAUNCHED.with(|l| l.borrow_mut().clear());
            cx.update(|cx| open_local_non_preview(elf_target.clone(), open_in, panel, cx));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while LAUNCHED.with(|l| l.borrow().is_empty()) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for elf sniff"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
                cx.run_until_parked();
            }
            assert_eq!(
                LAUNCHED.with(|l| l.borrow().clone()),
                [(FileLinkLaunch::Editor(FileLinkEditor::Zed), elf_target)]
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn sniff_file_content_skips_fifos_without_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let fifo = temp.path().join("pipe.dat");
        let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(status.success(), "mkfifo failed");
        assert_eq!(sniff_file_content(&fifo), FileLinkContent::Unknown);
    }
}
