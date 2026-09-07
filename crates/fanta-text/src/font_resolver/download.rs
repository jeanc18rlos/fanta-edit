//! Real, offline-safe font download from a free font bank (step 3).
//!
//! This is what lets a document family that exists on a free bank — "Roboto",
//! "Open Sans", "Lato", "Work Sans", "Noto Sans", … — render in its *actual*
//! font instead of a generic substitute, the way Figma/OpenPencil do.
//!
//! ## Source: the Google Fonts repository (OFL / Apache / UFL)
//!
//! We fetch from `github.com/google/fonts` via the raw CDN
//! (`raw.githubusercontent.com`). It is a flat, license-partitioned tree:
//! `ofl/<slug>/`, `apache/<slug>/`, or `ufl/<slug>/`, where `<slug>` is the
//! family name lowercased with every non-alphanumeric character stripped
//! ("Open Sans" → `opensans`, "PT Sans" → `ptsans`, "IBM Plex Sans" →
//! `ibmplexsans`).
//!
//! ## Why a `METADATA.pb` lookup, not filename guessing
//!
//! The *old* downloader guessed the face filename (`Roboto[wght].ttf`,
//! `Roboto-Regular.ttf`, …) and **always missed**: Roboto actually ships as
//! `Roboto[wdth,wght].ttf` (a `wdth,wght` variable file), Open Sans as
//! `OpenSans[wdth,wght].ttf`, Work Sans as `WorkSans[wght].ttf`, Lato as a pile
//! of static `Lato-*.ttf` weights — there is no single guessable pattern. So we
//! never actually downloaded anything; every family fell straight through to the
//! bundled/generic substitute.
//!
//! Each family directory carries a `METADATA.pb` (a *text*-format protobuf) that
//! authoritatively lists every face: `fonts { filename: "…" style: "normal"
//! weight: N … }`. We fetch that small text file, parse the blocks, and pick the
//! upright face to download — a variable file if the family ships one (Skia
//! instances it per weight), else the `weight: 400 normal` static. This is the
//! same data Google's own font-file API is built on, fetched without an API key
//! or a rate-limited JSON endpoint.
//!
//! ## Caching
//!
//! Downloaded faces are cached on disk under `~/.cache/fanta/fonts` (the
//! platform cache dir) as `<slug>.ttf`, reused on subsequent runs without any
//! network. A family that isn't on the bank records a small `<slug>.miss` marker
//! so we don't re-probe the network for it every run. The in-process memo in
//! [`resolver`](super::resolver) additionally collapses repeat lookups within a
//! single run.
//!
//! ## Offline-safety
//!
//! Every network and filesystem operation is best-effort: no network, a 404, a
//! decode error, or a too-large body all yield `None`, and the resolver falls
//! through to the next step. Nothing here ever panics on a missing or hostile
//! network.

use std::io::{Cursor, Read as _};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Process-wide memo of download attempts: family name → resolved family on
/// success, or `None` recorded as a miss so we don't retry every layout. Keyed
/// by the requested name verbatim.
pub(super) fn download_cache() -> &'static Mutex<std::collections::HashMap<String, Option<String>>>
{
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Option<String>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// License-partition subtrees of `google/fonts`, tried in order. Most families
/// live under `ofl/`; the Apache-licensed and Ubuntu-licensed sets are smaller.
const TREES: &[&str] = &["ofl", "apache", "ufl"];

/// Raw CDN base for the `main` branch of `google/fonts`.
const RAW_BASE: &str = "https://raw.githubusercontent.com/google/fonts/main";

/// Adobe's legacy "Pro" family release zips. The Spectrum community file still
/// names these exact families; using them beats the newer Source Sans 3 /
/// Source Serif 4 fallback and gets closer to Figma's Adobe Fonts render.
const SOURCE_SANS_PRO_ZIP: &str = "https://github.com/adobe-fonts/source-sans/releases/download/2.040R-ro/1.090R-it/source-sans-pro-2.040R-ro-1.090R-it.zip";
const SOURCE_SERIF_PRO_ZIP: &str = "https://github.com/adobe-fonts/source-serif/releases/download/3.000R/source-serif-pro-3.000R.zip";

/// Per-request network timeout. Keeps a slow/hostile server from stalling
/// layout; on timeout the fetch yields `None` and the resolver falls through.
const HTTP_TIMEOUT: Duration = Duration::from_secs(12);

/// Hard cap on a single downloaded body, to bound memory on a hostile URL. A
/// large variable family with many subsets can exceed a few MiB, so this is
/// generous but finite.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The on-disk cache directory for downloaded fonts (`~/.cache/fanta/fonts` on
/// Linux, the platform equivalent elsewhere). Best-effort: if no cache dir can
/// be determined, downloads still work but aren't persisted across runs.
fn font_cache_dir() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("fanta").join("fonts");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The repo slug for a family name: lowercase, every non-alphanumeric character
/// removed. Matches the `google/fonts` directory convention ("Open Sans" →
/// `opensans`, "PT Sans" → `ptsans`, "IBM Plex Sans" → `ibmplexsans`).
fn family_slug(family: &str) -> String {
    family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Percent-encode the characters a `google/fonts` variable-font filename can
/// carry that a URL path must escape: `[` and `]`. (The axis-list comma is
/// accepted literally by the CDN, and every other character in a font filename
/// is already URL-safe.)
fn encode_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 6);
    for c in name.chars() {
        match c {
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            other => out.push(other),
        }
    }
    out
}

/// Fetch the TTF bytes for `family` from a free font bank, using the disk cache
/// when present. Offline-safe: returns `None` on any error.
///
/// On a cache miss it resolves the real face filename from the family's
/// `METADATA.pb` (across the `ofl`/`apache`/`ufl` trees), downloads it, and
/// persists it to `<slug>.ttf` for reuse. A confirmed not-on-the-bank result is
/// recorded as a `<slug>.miss` marker so the network isn't re-probed next run.
pub(super) fn fetch_google_font(family: &str) -> Option<Vec<u8>> {
    let slug = family_slug(family);
    if slug.is_empty() {
        return None;
    }

    // Disk cache: a previously-downloaded face, served without any network.
    if let Some(dir) = font_cache_dir() {
        let cached = dir.join(format!("{slug}.ttf"));
        if let Ok(bytes) = std::fs::read(&cached) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
        // A persisted miss marker: known not-on-the-bank, skip the network.
        if dir.join(format!("{slug}.miss")).exists() {
            return None;
        }
    }

    // Network path: discover the real filename, then download it.
    let bytes = download_family(&slug);

    // Persist the outcome (best-effort): the face on success, a miss marker
    // otherwise, so subsequent runs don't repeat the network probe.
    if let Some(dir) = font_cache_dir() {
        // A cache write is an optimization, not correctness: on failure the
        // next run just repeats the network probe. Log at debug so a broken
        // cache dir is diagnosable without being noisy.
        let (path, contents): (_, &[u8]) = match &bytes {
            Some(b) if !b.is_empty() => (dir.join(format!("{slug}.ttf")), b),
            _ => (dir.join(format!("{slug}.miss")), &[]),
        };
        if let Err(error) = std::fs::write(&path, contents) {
            tracing::debug!(target: "fanta-text.font_cache", ?path, %error, "font cache write failed");
        }
    }

    bytes
}

/// Resolve `slug`'s upright face filename from its `METADATA.pb` and download
/// the file, scanning the license trees in [`TREES`] order. `None` if the family
/// isn't found on any tree or the download fails.
fn download_family(slug: &str) -> Option<Vec<u8>> {
    if let Some(bytes) = download_adobe_legacy_source_pro(slug) {
        return Some(bytes);
    }

    for tree in TREES {
        let meta_url = format!("{RAW_BASE}/{tree}/{slug}/METADATA.pb");
        let Some(meta_bytes) = http_get(&meta_url) else {
            continue;
        };
        let meta_text = String::from_utf8_lossy(&meta_bytes);
        let Some(filename) = pick_upright_filename(&meta_text) else {
            continue;
        };
        let file_url = format!("{RAW_BASE}/{tree}/{slug}/{}", encode_filename(&filename));
        if let Some(bytes) = http_get(&file_url) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
    }
    None
}

fn download_adobe_legacy_source_pro(slug: &str) -> Option<Vec<u8>> {
    let spec = match slug {
        "sourcesanspro" => LegacyAdobeSourceSpec {
            url: SOURCE_SANS_PRO_ZIP,
            preferred_suffixes: &[
                "/TTF/SourceSansPro-Regular.ttf",
                "/VAR/SourceSansVariable-Roman.ttf",
            ],
        },
        "sourceserifpro" => LegacyAdobeSourceSpec {
            url: SOURCE_SERIF_PRO_ZIP,
            preferred_suffixes: &[
                "/TTF/SourceSerifPro-Regular.ttf",
                "/VAR/SourceSerifVariable-Roman.ttf",
            ],
        },
        _ => return None,
    };
    let zip_bytes = http_get(spec.url)?;
    extract_preferred_font_from_zip(&zip_bytes, spec.preferred_suffixes)
}

struct LegacyAdobeSourceSpec {
    url: &'static str,
    preferred_suffixes: &'static [&'static str],
}

fn extract_preferred_font_from_zip(
    zip_bytes: &[u8],
    preferred_suffixes: &[&str],
) -> Option<Vec<u8>> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;

    for suffix in preferred_suffixes {
        if let Some(bytes) = read_first_zip_entry_matching(&mut archive, |name| {
            name.ends_with(suffix.trim_start_matches('/')) || name.ends_with(suffix)
        }) {
            return Some(bytes);
        }
    }

    read_first_zip_entry_matching(&mut archive, |name| {
        let lower = name.to_ascii_lowercase();
        lower.ends_with(".ttf") && lower.contains("regular")
    })
}

fn read_first_zip_entry_matching(
    archive: &mut zip::ZipArchive<Cursor<&[u8]>>,
    mut predicate: impl FnMut(&str) -> bool,
) -> Option<Vec<u8>> {
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i).ok()?;
        let name = file.name().to_owned();
        if predicate(&name) {
            names.push(name);
        }
    }
    names.sort();
    let name = names.into_iter().next()?;
    let mut file = archive.by_name(&name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// One face entry parsed from a `METADATA.pb` `fonts { … }` block.
struct FontEntry {
    filename: String,
    /// `"normal"` (upright) or `"italic"`. Defaults to `"normal"` if absent.
    style: String,
    /// OS/2 weight (100–900). Defaults to 400 if absent.
    weight: u32,
}

/// Choose the best upright face filename to download from a `METADATA.pb` text.
///
/// Preference, in order:
///  1. an upright **variable** file (filename contains `[`, e.g.
///     `Roboto[wdth,wght].ttf`) — one download covers every weight, which Skia
///     instances per requested weight,
///  2. the upright **weight-400 (Regular)** static file,
///  3. the first upright file of any weight (so a family with no 400 — rare —
///     still resolves to *something* upright),
///  4. as a final fallback, any file at all (degenerate metadata).
fn pick_upright_filename(meta: &str) -> Option<String> {
    let entries = parse_font_entries(meta);
    if entries.is_empty() {
        return None;
    }
    let is_upright = |e: &&FontEntry| e.style.eq_ignore_ascii_case("normal");

    // 1. Upright variable file.
    if let Some(e) = entries
        .iter()
        .filter(is_upright)
        .find(|e| e.filename.contains('['))
    {
        return Some(e.filename.clone());
    }
    // 2. Upright Regular (weight 400).
    if let Some(e) = entries.iter().filter(is_upright).find(|e| e.weight == 400) {
        return Some(e.filename.clone());
    }
    // 3. Any upright face.
    if let Some(e) = entries.iter().find(is_upright) {
        return Some(e.filename.clone());
    }
    // 4. Anything.
    entries.first().map(|e| e.filename.clone())
}

/// Parse the `fonts { … }` blocks of a `METADATA.pb` text into [`FontEntry`]s.
///
/// `METADATA.pb` is protobuf *text* format: `key: value` lines, string values
/// double-quoted, blocks delimited by `{`/`}`. We only need three keys per
/// `fonts` block (`filename`, `style`, `weight`), so this is a small line
/// scanner rather than a full protobuf-text parser — robust to the subset of
/// the grammar these files actually use.
fn parse_font_entries(meta: &str) -> Vec<FontEntry> {
    let mut entries = Vec::new();
    let mut depth: i32 = 0;
    let mut in_fonts = false;
    let mut fonts_depth = 0;
    let mut cur_filename: Option<String> = None;
    let mut cur_style: Option<String> = None;
    let mut cur_weight: Option<u32> = None;

    for raw in meta.lines() {
        let line = raw.trim();

        // Enter a `fonts {` block. The brace may be on the same line.
        if !in_fonts && line.starts_with("fonts") && line.contains('{') {
            in_fonts = true;
            fonts_depth = depth;
            depth += 1;
            cur_filename = None;
            cur_style = None;
            cur_weight = None;
            continue;
        }

        if in_fonts {
            // Track nesting so a stray nested `{ }` doesn't end the block early.
            let opens = line.matches('{').count() as i32;
            let closes = line.matches('}').count() as i32;

            if let Some(v) = parse_string_field(line, "filename") {
                cur_filename = Some(v);
            } else if let Some(v) = parse_string_field(line, "style") {
                cur_style = Some(v);
            } else if let Some(v) = parse_uint_field(line, "weight") {
                cur_weight = Some(v);
            }

            depth += opens - closes;
            // The block closes when depth drops back to where `fonts {` opened.
            if depth <= fonts_depth {
                if let Some(filename) = cur_filename.take() {
                    entries.push(FontEntry {
                        filename,
                        style: cur_style.take().unwrap_or_else(|| "normal".to_string()),
                        weight: cur_weight.take().unwrap_or(400),
                    });
                }
                in_fonts = false;
            }
            continue;
        }

        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth < 0 {
            depth = 0;
        }
    }

    entries
}

/// Parse a `key: "value"` line, returning the unquoted value if `key` matches.
fn parse_string_field(line: &str, key: &str) -> Option<String> {
    let rest = field_value(line, key)?;
    let trimmed = rest.trim();
    let inner = trimmed.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// Parse a `key: <uint>` line, returning the integer if `key` matches.
fn parse_uint_field(line: &str, key: &str) -> Option<u32> {
    let rest = field_value(line, key)?;
    rest.trim().parse().ok()
}

/// The portion of `line` after `key:`, if the line is exactly that field. Guards
/// against a prefix collision (e.g. `filename` vs a hypothetical `filenamex`) by
/// requiring the next character after the (whitespace-trimmed) key to be `:`.
fn field_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    Some(rest)
}

/// One blocking HTTP GET with a short timeout, returning the body bytes on a 200
/// response. Any error (no network, non-200, too large, timeout) yields `None`
/// so callers fall through. Capped at [`MAX_BYTES`] to bound memory.
fn http_get(url: &str) -> Option<Vec<u8>> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let mut resp = agent.get(url).call().ok()?;
    if resp.status() != 200 {
        return None;
    }
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn slug_strips_spaces_and_punctuation_lowercased() {
        assert_eq!(family_slug("Roboto"), "roboto");
        assert_eq!(family_slug("Open Sans"), "opensans");
        assert_eq!(family_slug("PT Sans"), "ptsans");
        assert_eq!(family_slug("IBM Plex Sans"), "ibmplexsans");
        assert_eq!(family_slug("Source Sans 3"), "sourcesans3");
        assert_eq!(family_slug("Source Sans Pro"), "sourcesanspro");
        assert_eq!(family_slug("  "), "");
    }

    #[test]
    fn extracts_preferred_font_from_adobe_zip() {
        let mut buf = Cursor::new(Vec::<u8>::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            zw.start_file("pkg/TTF/Other-Regular.ttf", opts).unwrap();
            zw.write_all(b"other").unwrap();
            zw.start_file("pkg/TTF/SourceSansPro-Regular.ttf", opts)
                .unwrap();
            zw.write_all(b"source-sans-pro").unwrap();
            zw.finish().unwrap();
        }

        let bytes =
            extract_preferred_font_from_zip(buf.get_ref(), &["/TTF/SourceSansPro-Regular.ttf"])
                .unwrap();
        assert_eq!(bytes, b"source-sans-pro");
    }

    #[test]
    fn encode_filename_escapes_only_brackets() {
        assert_eq!(encode_filename("Lato-Regular.ttf"), "Lato-Regular.ttf");
        assert_eq!(
            encode_filename("Roboto[wdth,wght].ttf"),
            "Roboto%5Bwdth,wght%5D.ttf"
        );
        assert_eq!(
            encode_filename("WorkSans[wght].ttf"),
            "WorkSans%5Bwght%5D.ttf"
        );
    }

    #[test]
    fn picks_upright_variable_file_first() {
        // A Roboto-shaped metadata: one normal + one italic variable file.
        let meta = r#"
name: "Roboto"
fonts {
  name: "Roboto"
  style: "normal"
  weight: 400
  filename: "Roboto[wdth,wght].ttf"
}
fonts {
  name: "Roboto"
  style: "italic"
  weight: 400
  filename: "Roboto-Italic[wdth,wght].ttf"
}
"#;
        assert_eq!(
            pick_upright_filename(meta).as_deref(),
            Some("Roboto[wdth,wght].ttf")
        );
    }

    #[test]
    fn picks_regular_static_when_no_variable() {
        // A Lato-shaped metadata: many static weights, no variable file. Must
        // pick the upright weight-400 face, not Thin/Bold/an italic.
        let meta = r#"
name: "Lato"
fonts {
  style: "normal"
  weight: 100
  filename: "Lato-Thin.ttf"
}
fonts {
  style: "italic"
  weight: 400
  filename: "Lato-Italic.ttf"
}
fonts {
  style: "normal"
  weight: 700
  filename: "Lato-Bold.ttf"
}
fonts {
  style: "normal"
  weight: 400
  filename: "Lato-Regular.ttf"
}
"#;
        assert_eq!(
            pick_upright_filename(meta).as_deref(),
            Some("Lato-Regular.ttf")
        );
    }

    #[test]
    fn picks_upright_any_weight_when_no_400() {
        let meta = r#"
fonts {
  style: "normal"
  weight: 500
  filename: "Foo-Medium.ttf"
}
"#;
        assert_eq!(
            pick_upright_filename(meta).as_deref(),
            Some("Foo-Medium.ttf")
        );
    }

    #[test]
    fn empty_or_garbage_metadata_yields_none() {
        assert_eq!(pick_upright_filename(""), None);
        assert_eq!(pick_upright_filename("name: \"NoFonts\""), None);
    }

    #[test]
    fn field_parsers_guard_prefix_collisions() {
        // `filename:` parses; a hypothetical `filenamex:` must NOT match `filename`.
        assert_eq!(
            parse_string_field("  filename: \"x.ttf\"", "filename").as_deref(),
            Some("x.ttf")
        );
        assert_eq!(
            parse_string_field("  filenamex: \"x.ttf\"", "filename"),
            None
        );
        assert_eq!(parse_uint_field("  weight: 700", "weight"), Some(700));
        assert_eq!(parse_uint_field("  weightx: 700", "weight"), None);
    }

    #[test]
    fn weight_and_style_default_when_absent() {
        // A degenerate block with only a filename still produces an entry,
        // defaulting style→normal weight→400, so it's selectable.
        let meta = r#"
fonts {
  filename: "Solo.ttf"
}
"#;
        let entries = parse_font_entries(meta);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "Solo.ttf");
        assert_eq!(entries[0].style, "normal");
        assert_eq!(entries[0].weight, 400);
        assert_eq!(pick_upright_filename(meta).as_deref(), Some("Solo.ttf"));
    }
}
