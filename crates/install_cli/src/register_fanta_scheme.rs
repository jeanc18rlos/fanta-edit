use client::FANTA_URL_SCHEME;
use gpui::{AsyncApp, actions};

actions!(
    cli,
    [
        /// Registers the fanta:// URL scheme handler.
        RegisterFantaScheme
    ]
);

pub async fn register_fanta_scheme(cx: &AsyncApp) -> anyhow::Result<()> {
    cx.update(|cx| cx.register_url_scheme(FANTA_URL_SCHEME)).await
}
