// This crate was essentially pulled out verbatim from main `zed` crate to avoid having to run RustEmbed macro whenever zed has to be rebuilt. It saves a second or two on an incremental build.

use anyhow::{Context as _, anyhow};
use gpui::{App, AssetSource, Result, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../assets"]
#[include = "sounds/**/*"]
#[include = "prompts/**/*"]
#[include = "*.md"]
#[exclude = "*.DS_Store"]
struct EditorAssets;

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if path == "licenses.md" {
            if let Some(file) = EditorAssets::get(path) {
                return Ok(Some(file.data));
            }
        }
        if let Some(bytes) = zed_ui_assets::Assets.load(path)? {
            return Ok(Some(bytes));
        }
        EditorAssets::get(path)
            .map(|file| Some(file.data))
            .with_context(|| format!("loading asset at path {path:?}"))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = zed_ui_assets::Assets.list(path)?;
        for editor_path in EditorAssets::iter().filter_map(|p| {
            if p.starts_with(path) {
                Some(p.into())
            } else {
                None
            }
        }) {
            if !paths.contains(&editor_path) {
                paths.push(editor_path);
            }
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_license_attribution_takes_precedence_when_present() -> Result<()> {
        let loaded = Assets
            .load("licenses.md")?
            .context("license asset is missing")?;
        let shared = zed_ui_assets::Assets
            .load("licenses.md")?
            .context("shared license asset is missing")?;
        if let Some(editor_licenses) = EditorAssets::get("licenses.md") {
            assert_eq!(loaded.as_ref(), editor_licenses.data.as_ref());
        } else {
            assert_eq!(loaded.as_ref(), shared.as_ref());
        }
        Ok(())
    }
}

impl Assets {
    /// Populate the [`TextSystem`] of the given [`AppContext`] with all `.ttf` fonts in the `fonts` directory.
    pub fn load_fonts(&self, cx: &App) -> anyhow::Result<()> {
        let font_paths = self.list("fonts")?;
        let mut embedded_fonts = Vec::new();
        for font_path in font_paths {
            if font_path.ends_with(".ttf") {
                let font_bytes = cx
                    .asset_source()
                    .load(&font_path)?
                    .ok_or_else(|| anyhow!("bundled font missing: {font_path}"))?;
                embedded_fonts.push(font_bytes);
            }
        }

        cx.text_system().add_fonts(embedded_fonts)
    }

    pub fn load_test_fonts(&self, cx: &App) {
        cx.text_system()
            .add_fonts(vec![
                self.load("fonts/lilex/Lilex-Regular.ttf").unwrap().unwrap(),
            ])
            .unwrap()
    }
}
