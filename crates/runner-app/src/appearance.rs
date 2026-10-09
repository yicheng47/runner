use gpui::{App, Global, WindowBackgroundAppearance};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowMaterial {
    Glass,
    Solid,
}

impl Default for WindowMaterial {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Glass
        } else {
            Self::Solid
        }
    }
}

impl WindowMaterial {
    pub fn effective(self, macos: bool, reduce_transparency: bool) -> Self {
        if macos && !reduce_transparency {
            self
        } else {
            Self::Solid
        }
    }

    pub fn background(self) -> WindowBackgroundAppearance {
        match self {
            Self::Glass => WindowBackgroundAppearance::Blurred,
            Self::Solid => WindowBackgroundAppearance::Opaque,
        }
    }
}

#[derive(Default)]
pub struct Appearance {
    pub reduce_transparency: bool,
    pub material: WindowMaterial,
}

impl Global for Appearance {}

pub fn effective(material: WindowMaterial, cx: &App) -> WindowMaterial {
    material.effective(
        cfg!(target_os = "macos"),
        cx.try_global::<Appearance>()
            .is_some_and(|state| state.reduce_transparency),
    )
}

pub fn configure(material: WindowMaterial, cx: &mut App) {
    let glass = effective(material, cx) == WindowMaterial::Glass;
    crate::theme::set_glass(glass && cx.has_global::<Appearance>());
    if cx.has_global::<Appearance>() {
        cx.global_mut::<Appearance>().material = material;
    }
}

pub fn install(cx: &mut App) {
    cx.set_global(Appearance::default());
    #[cfg(all(target_os = "macos", not(test)))]
    crate::material_macos::install(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_default_and_effective_material() {
        assert_eq!(
            WindowMaterial::default(),
            if cfg!(target_os = "macos") {
                WindowMaterial::Glass
            } else {
                WindowMaterial::Solid
            }
        );
        for material in [WindowMaterial::Glass, WindowMaterial::Solid] {
            assert_eq!(material.effective(false, false), WindowMaterial::Solid);
            assert_eq!(material.effective(false, true), WindowMaterial::Solid);
            assert_eq!(material.effective(true, true), WindowMaterial::Solid);
            assert_eq!(material.effective(true, false), material);
        }
    }
}
