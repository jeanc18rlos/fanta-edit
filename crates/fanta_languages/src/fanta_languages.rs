//! Language registration for Fanta.
//!
//! Fanta is a design canvas, not an editor: the only source it ever shows is
//! the read-only FNX behind a design plus the odd JSON file. So this crate
//! registers syntax highlighting for exactly those languages and nothing else —
//! no LSP adapters, no toolchain listers, no task context providers — which is
//! what keeps the app from trying to download Node and start a TypeScript
//! server when a design is opened.

use std::sync::Arc;

use language::{LanguageRegistry, LoadedLanguage};

/// Languages Fanta registers. FNX reuses the TSX grammar and queries (see
/// `grammars::load_queries`), so the whole set needs only two parser crates.
// `regex` is here because the buffer search bar highlights its own query
// field with it; without it search logs an error on every window.
const LANGUAGES: &[&str] = &["fnx", "tsx", "typescript", "json", "jsonc", "regex"];

pub fn init(languages: Arc<LanguageRegistry>) {
    languages.register_native_grammars(grammars::fanta_native_grammars());

    for name in LANGUAGES {
        let name = *name;
        let config = grammars::load_config(name);
        languages.register_language(
            config.name.clone(),
            config.grammar.clone(),
            config.matcher.clone(),
            config.hidden,
            None,
            Arc::new(move || {
                Ok(LoadedLanguage {
                    config: config.clone(),
                    queries: grammars::load_queries(name),
                    context_provider: None,
                    toolchain_provider: None,
                    manifest_name: None,
                })
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[gpui::test]
    async fn fnx_uses_the_tsx_grammar_and_no_language_servers(cx: &mut gpui::TestAppContext) {
        let languages = Arc::new(LanguageRegistry::new(cx.executor()));
        init(languages.clone());

        let available = languages
            .language_for_file_path(Path::new("/designs/Page.fnx"))
            .expect("the .fnx suffix resolves to a registered language");
        assert_eq!(available.name().as_ref(), "FNX");

        let fnx = languages
            .language_for_name("FNX")
            .await
            .expect("FNX loads with its grammar");
        assert_eq!(fnx.config().grammar.as_deref(), Some("tsx"));
        assert!(
            fnx.grammar().is_some(),
            "the TSX grammar should have resolved for FNX"
        );
        assert!(
            fnx.grammar()
                .and_then(|grammar| grammar.highlights_config.as_ref())
                .is_some(),
            "FNX should inherit the TSX highlight queries"
        );

        assert!(
            languages.lsp_adapters(&fnx.name()).is_empty(),
            "FNX must not have a language server adapter"
        );
        assert!(
            languages.all_lsp_adapters().is_empty(),
            "no language server adapters should be registered at all"
        );
    }

    #[gpui::test]
    async fn json_is_registered_for_highlighting(cx: &mut gpui::TestAppContext) {
        let languages = Arc::new(LanguageRegistry::new(cx.executor()));
        init(languages.clone());

        let json = languages
            .language_for_name("JSON")
            .await
            .expect("JSON loads with its grammar");
        assert!(json.grammar().is_some());
        assert!(languages.lsp_adapters(&json.name()).is_empty());
    }

    /// The buffer search bar highlights its own query field with the regex
    /// grammar and logs an error at startup if it is missing.
    #[gpui::test]
    async fn regex_is_registered_for_the_search_bar(cx: &mut gpui::TestAppContext) {
        let languages = Arc::new(LanguageRegistry::new(cx.executor()));
        init(languages.clone());

        let regex = languages
            .language_for_name("Regex")
            .await
            .expect("the search bar's regex language must be registered");
        assert!(
            regex.grammar().is_some(),
            "the regex grammar should have resolved"
        );
    }
}
