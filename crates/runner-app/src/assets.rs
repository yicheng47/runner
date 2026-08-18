use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const BRAND_MARK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32"><svg x="3" y="3" width="9" height="9" viewBox="0 0 24 24"><polyline points="9 18 15 12 9 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity=".4"/></svg><svg x="9" y="9" width="14" height="14" viewBox="0 0 24 24"><polyline points="9 18 15 12 9 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg><svg x="3" y="20" width="9" height="9" viewBox="0 0 24 24"><polyline points="9 18 15 12 9 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity=".4"/></svg></svg>"#;
const PANEL_LEFT_FILLED: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 64 50" fill="none"><rect x="1.5" y="1.5" width="57" height="43" rx="7.5" stroke="currentColor" stroke-width="3"/><path d="M9 3 H19 V43 H9 Q3 43 3 37 V9 Q3 3 9 3 Z" fill="currentColor"/></svg>"#;
const PANEL_LEFT_HOLLOW: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 64 50" fill="none"><rect x="1.5" y="1.5" width="57" height="43" rx="7.5" stroke="currentColor" stroke-width="3"/><rect x="19" y="3" width="3" height="40" rx="1.5" fill="currentColor"/></svg>"#;
const SETTINGS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.74v-.51a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"#;
const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>"#;
const ARROW_LEFT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>"#;

const ASSETS: &[(&str, &[u8])] = &[
    ("brand-mark.svg", BRAND_MARK),
    ("panel-left-filled.svg", PANEL_LEFT_FILLED),
    ("panel-left-hollow.svg", PANEL_LEFT_HOLLOW),
    ("settings.svg", SETTINGS),
    ("close.svg", CLOSE),
    ("arrow-left.svg", ARROW_LEFT),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(asset_path, _)| *asset_path == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(asset_path, _)| asset_path.starts_with(path))
            .map(|(asset_path, _)| SharedString::from(*asset_path))
            .collect())
    }
}
