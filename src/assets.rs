use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString, assets::Assets as ComponentAssets};
use rust_embed::RustEmbed;

pub const APP_LOGO: &str = "images/app.png";
pub const APP_LOGO_BYTES: &[u8] = include_bytes!("../assets/images/app.png");

// Add application images under assets/images; keys are relative to assets/.
#[derive(RustEmbed)]
#[folder = "assets/"]
#[include = "images/**/*.png"]
#[include = "images/**/*.jpg"]
#[include = "images/**/*.webp"]
#[include = "images/**/*.svg"]
#[include = "icons/**/*.svg"]
struct EmbeddedAssets;

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = EmbeddedAssets::get(path) {
            return Ok(Some(file.data));
        }
        ComponentAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = ComponentAssets.list(path)?;
        paths.extend(
            EmbeddedAssets::iter()
                .filter(|key| key.starts_with(path))
                .map(|key| SharedString::from(key.into_owned())),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_logo_and_preserves_component_icons() {
        let logo = AppAssets.load(APP_LOGO).unwrap().unwrap();
        let decoded = image::load_from_memory(&logo).unwrap();
        assert_eq!(decoded.width(), decoded.height());
        assert!(decoded.width() >= 256);
        assert!(AppAssets.load("icons/check.svg").unwrap().is_some());
        assert!(
            AppAssets
                .list("images/")
                .unwrap()
                .iter()
                .any(|p| p == APP_LOGO)
        );
    }
}
