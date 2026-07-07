//! Optional registration of local font directories for high-fidelity imports.
//!
//! These fonts are not vendored or required at runtime. A fidelity run can set
//! `FANTA_FONT_DIRS=/path/to/fonts[:/another/path]` and every supported font
//! file in those directories is registered under its embedded family name.

use std::{env, fs, path::Path};

use skia_safe::{FontMgr, textlayout::TypefaceFontProvider};

const ENV_FONT_DIRS: &str = "FANTA_FONT_DIRS";

pub(super) fn register_external_font_dirs(
    provider: &mut TypefaceFontProvider,
    mgr: &FontMgr,
) -> usize {
    let Some(raw_dirs) = env::var_os(ENV_FONT_DIRS) else {
        return 0;
    };
    if raw_dirs.is_empty() {
        return 0;
    }

    let mut registered = 0;
    for dir in env::split_paths(&raw_dirs) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && is_supported_font(path))
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let Some(face) = mgr.new_from_data(&bytes, None) else {
                continue;
            };
            provider.register_typeface(face, None);
            registered += 1;
        }
    }
    registered
}

fn is_supported_font(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ttf" | "otf" | "ttc" | "otc")
    )
}
