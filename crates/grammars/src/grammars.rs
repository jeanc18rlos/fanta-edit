use anyhow::Context as _;
use language_core::{LanguageConfig, LanguageQueries, QUERY_FILENAME_PREFIXES};
use rust_embed::RustEmbed;
use util::asset_str;

#[derive(RustEmbed)]
#[folder = "src/"]
#[exclude = "*.rs"]
struct GrammarDir;

/// Register all built-in native tree-sitter grammars with the provided registration function.
///
/// Each grammar is registered as a `(&str, tree_sitter_language::LanguageFn)` pair.
/// This must be called before loading language configs/queries.
#[cfg(feature = "load-grammars")]
pub fn native_grammars() -> Vec<(&'static str, tree_sitter::Language)> {
    vec![
        ("bash", tree_sitter_bash::LANGUAGE.into()),
        ("c", tree_sitter_c::LANGUAGE.into()),
        ("cpp", tree_sitter_cpp::LANGUAGE.into()),
        ("css", tree_sitter_css::LANGUAGE.into()),
        ("diff", tree_sitter_diff::LANGUAGE.into()),
        ("go", tree_sitter_go::LANGUAGE.into()),
        ("gomod", tree_sitter_go_mod::LANGUAGE.into()),
        ("gowork", tree_sitter_gowork::LANGUAGE.into()),
        ("jsdoc", tree_sitter_jsdoc::LANGUAGE.into()),
        ("json", tree_sitter_json::LANGUAGE.into()),
        ("jsonc", tree_sitter_json::LANGUAGE.into()),
        ("markdown", tree_sitter_md::LANGUAGE.into()),
        ("markdown-inline", tree_sitter_md::INLINE_LANGUAGE.into()),
        ("python", tree_sitter_python::LANGUAGE.into()),
        ("regex", tree_sitter_regex::LANGUAGE.into()),
        ("rust", tree_sitter_rust::LANGUAGE.into()),
        ("tsx", tree_sitter_typescript::LANGUAGE_TSX.into()),
        (
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ),
        ("yaml", tree_sitter_yaml::LANGUAGE.into()),
        ("gitcommit", tree_sitter_gitcommit::LANGUAGE.into()),
    ]
}

/// The grammars Fanta needs: TSX (which also backs the FNX design language),
/// TypeScript, and JSON (reused for JSONC).
///
/// This is the `native_grammars` subset that survives once the IDE language
/// support is dropped, so the binary does not link the other parser crates.
#[cfg(feature = "fanta-grammars")]
pub fn fanta_native_grammars() -> Vec<(&'static str, tree_sitter::Language)> {
    vec![
        ("json", tree_sitter_json::LANGUAGE.into()),
        ("regex", tree_sitter_regex::LANGUAGE.into()),
        ("jsonc", tree_sitter_json::LANGUAGE.into()),
        ("tsx", tree_sitter_typescript::LANGUAGE_TSX.into()),
        (
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ),
    ]
}

/// Load and parse the `config.toml` for a given language name.
pub fn load_config(name: &str) -> LanguageConfig {
    let config_toml = String::from_utf8(
        GrammarDir::get(&format!("{}/config.toml", name))
            .unwrap_or_else(|| panic!("missing config for language {:?}", name))
            .data
            .to_vec(),
    )
    .unwrap();

    let config: LanguageConfig = ::toml::from_str(&config_toml)
        .with_context(|| format!("failed to load config.toml for language {name:?}"))
        .unwrap();

    config
}

/// Load and parse the `config.toml` for a given language name, stripping fields
/// that require grammar support when grammars are not loaded.
pub fn load_config_for_feature(name: &str, grammars_loaded: bool) -> LanguageConfig {
    let config = load_config(name);

    if grammars_loaded {
        config
    } else {
        LanguageConfig {
            name: config.name,
            matcher: config.matcher,
            jsx_tag_auto_close: config.jsx_tag_auto_close,
            ..Default::default()
        }
    }
}

/// Get a raw embedded file by path (relative to `src/`).
///
/// Returns the file data as bytes, or `None` if the file does not exist.
pub fn get_file(path: &str) -> Option<rust_embed::EmbeddedFile> {
    GrammarDir::get(path)
}

/// Load all `.scm` query files for a given language name into a `LanguageQueries`.
///
/// Multiple `.scm` files with the same prefix (e.g. `highlights.scm` and
/// `highlights_extra.scm`) are concatenated together with their contents appended.
pub fn load_queries(name: &str) -> LanguageQueries {
    // FNX is deliberately a TSX-shaped design language. Keeping one query
    // source means highlighting, outlines, bracket matching, and indentation
    // stay in lockstep with the bundled TSX grammar instead of slowly
    // diverging as those queries evolve.
    let query_name = if name == "fnx" { "tsx" } else { name };
    let mut result = LanguageQueries::default();
    for path in GrammarDir::iter() {
        if let Some(remainder) = path
            .strip_prefix(query_name)
            .and_then(|path| path.strip_prefix('/'))
        {
            if !remainder.ends_with(".scm") {
                continue;
            }
            for (prefix, query) in QUERY_FILENAME_PREFIXES {
                if remainder.starts_with(prefix) {
                    let contents = asset_str::<GrammarDir>(path.as_ref());
                    match query(&mut result) {
                        None => *query(&mut result) = Some(contents),
                        Some(existing) => existing.to_mut().push_str(contents.as_ref()),
                    }
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnx_uses_the_tsx_grammar_and_queries() {
        let config = load_config("fnx");
        assert_eq!(config.name.as_ref(), "FNX");
        assert_eq!(config.grammar.as_deref(), Some("tsx"));
        assert_eq!(config.matcher.path_suffixes, ["fnx"]);
        assert_eq!(
            load_queries("fnx").highlights,
            load_queries("tsx").highlights
        );
    }

    #[test]
    #[cfg(any(feature = "load-grammars", feature = "fanta-grammars"))]
    fn generated_fnx_shape_is_valid_tsx() {
        let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).expect("set TSX grammar");
        let source = r##"import type {} from "../../fnx";
export default function Page() {
  return (
    <Frame background={{"kind": "solid", "color": fnxColor("#336699")}}>
      <Text content="Hello" x={12.0} y={24.0} />
    </Frame>
  );
}
"##;
        let tree = parser.parse(source, None).expect("parse FNX as TSX");
        assert!(!tree.root_node().has_error(), "{:#?}", tree.root_node());
    }
}
