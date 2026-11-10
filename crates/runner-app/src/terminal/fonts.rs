use gpui::{font, px, App, AppContext, Context, Entity, Global, Task, TextSystem};

pub(crate) const BUNDLED_FONT: &str = "JetBrainsMono Nerd Font Mono";

#[derive(Default)]
pub(crate) struct FontFamilies {
    pub(crate) all: Vec<String>,
    pub(crate) monospace: Vec<String>,
}

pub(crate) struct FontFamilyCache {
    pub(crate) families: Option<FontFamilies>,
    _task: Task<()>,
}

struct GlobalFontFamilyCache(Entity<FontFamilyCache>);
impl Global for GlobalFontFamilyCache {}

impl FontFamilyCache {
    pub(crate) fn global(cx: &mut App) -> Entity<Self> {
        if let Some(cache) = cx.try_global::<GlobalFontFamilyCache>() {
            return cache.0.clone();
        }
        let cache = cx.new(|cx: &mut Context<Self>| {
            let text_system = cx.text_system().clone();
            let load = cx.background_spawn(async move {
                let all = text_system.all_font_names();
                let monospace = all
                    .iter()
                    .filter(|name| name.as_str() != BUNDLED_FONT)
                    .filter(|name| is_monospace(&text_system, name))
                    .cloned()
                    .collect();
                FontFamilies { all, monospace }
            });
            Self {
                families: None,
                _task: cx.spawn(async move |cache, cx| {
                    let families = load.await;
                    let _ = cache.update(cx, |cache, cx| {
                        cache.families = Some(families);
                        cx.notify();
                        cx.refresh_windows();
                    });
                }),
            }
        });
        cx.set_global(GlobalFontFamilyCache(cache.clone()));
        cache
    }

    pub(crate) fn loaded(cx: &App) -> Option<&FontFamilies> {
        cx.try_global::<GlobalFontFamilyCache>()?
            .0
            .read(cx)
            .families
            .as_ref()
    }
}

/// GPUI loads the bundled faces from memory, but CoreText resolves a font's
/// fallbacks by family name, so the faces must also be registered with
/// CoreText before the bundled icons can fill in for another font.
#[cfg(target_os = "macos")]
pub(crate) fn register_bundled_font_for_fallback() {
    use core_foundation::base::TCFType;
    use core_foundation::data::{CFData, CFDataRef};
    use std::ffi::c_void;
    use std::sync::Arc;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGDataProviderCreateWithCFData(data: CFDataRef) -> *const c_void;
        fn CGDataProviderRelease(provider: *const c_void);
        fn CGFontCreateWithDataProvider(provider: *const c_void) -> *const c_void;
        fn CGFontRelease(font: *const c_void);
    }
    // Deprecated in favour of descriptor registration, which does not make
    // in-memory faces matchable by family name.
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerRegisterGraphicsFont(font: *const c_void, error: *mut c_void) -> bool;
    }

    for bytes in crate::assets::JETBRAINS_MONO_FONTS {
        let data = CFData::from_arc(Arc::new(bytes));
        unsafe {
            let provider = CGDataProviderCreateWithCFData(data.as_concrete_TypeRef());
            if provider.is_null() {
                continue;
            }
            let font = CGFontCreateWithDataProvider(provider);
            CGDataProviderRelease(provider);
            if font.is_null() {
                continue;
            }
            CTFontManagerRegisterGraphicsFont(font, std::ptr::null_mut());
            CGFontRelease(font);
        }
    }
}

fn is_monospace(text_system: &TextSystem, family: &str) -> bool {
    let id = text_system.resolve_font(&font(family.to_owned()));
    matching_advances(|ch| {
        text_system
            .advance(id, px(16.), ch)
            .ok()
            .map(|size| f32::from(size.width))
    })
}

fn matching_advances(advance: impl Fn(char) -> Option<f32>) -> bool {
    match ['i', 'W', 'm'].map(advance) {
        [Some(i), Some(w), Some(m)] => i > 0. && (i - w).abs() < 0.01 && (i - m).abs() < 0.01,
        _ => false,
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use font_kit::font::Font;
    use std::sync::Arc;

    #[test]
    fn monospace_filter_checks_real_glyph_advances() {
        let bundled =
            Font::from_bytes(Arc::new(crate::assets::JETBRAINS_MONO_FONTS[0].to_vec()), 0).unwrap();
        let inter = Font::from_bytes(Arc::new(crate::assets::INTER_FONTS[0].to_vec()), 0).unwrap();
        let accepts = |font: &Font| {
            matching_advances(|ch| {
                font.advance(font.glyph_for_char(ch)?)
                    .ok()
                    .map(|advance| advance.x())
            })
        };
        assert!(accepts(&bundled));
        assert!(!accepts(&inter));
    }

    #[test]
    fn installed_fonts_fall_back_to_bundled_icons() {
        use core_foundation::array::CFArray;
        use core_foundation::base::{CFRange, TCFType};
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::string::{CFString, CFStringRef};
        use core_text::font::CTFontRef;
        use core_text::font_descriptor::{
            kCTFontCascadeListAttribute, kCTFontFamilyNameAttribute, new_from_attributes,
            CTFontDescriptor,
        };

        #[link(name = "CoreText", kind = "framework")]
        unsafe extern "C" {
            fn CTFontCreateForString(
                font: CTFontRef,
                string: CFStringRef,
                range: CFRange,
            ) -> CTFontRef;
        }

        register_bundled_font_for_fallback();
        let family_key = unsafe { CFString::wrap_under_get_rule(kCTFontFamilyNameAttribute) };
        let cascade_key = unsafe { CFString::wrap_under_get_rule(kCTFontCascadeListAttribute) };
        let bundled = new_from_attributes(&CFDictionary::from_CFType_pairs(&[(
            family_key.clone(),
            CFString::new(BUNDLED_FONT).as_CFType(),
        )]));
        let menlo = new_from_attributes(&CFDictionary::from_CFType_pairs(&[
            (family_key, CFString::new("Menlo").as_CFType()),
            (
                cascade_key,
                CFArray::<CTFontDescriptor>::from_CFTypes(&[bundled]).as_CFType(),
            ),
        ]));
        let menlo = core_text::font::new_from_descriptor(&menlo, 16.);
        assert_eq!(menlo.family_name(), "Menlo");
        let icon = CFString::new("\u{f07c}");
        let drawn_with = unsafe {
            core_text::font::CTFont::wrap_under_create_rule(CTFontCreateForString(
                menlo.as_concrete_TypeRef(),
                icon.as_concrete_TypeRef(),
                CFRange::init(0, icon.char_len()),
            ))
        };
        assert_eq!(drawn_with.family_name(), BUNDLED_FONT);
    }
}
