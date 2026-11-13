//! What the files that packages install say: in English, with each translation that has it.

use super::{FALLBACK, LOADERS, Language};

impl Language {
    /// The language every message is in, for what is the same for everyone, such as the files packages install.
    pub(crate) fn english() -> Self {
        Self::accepted("")
    }
}

/// Message `id` in each translation whose own catalog has it, by language: `[("it", "…")]`.
pub(crate) fn translations(id: &str) -> Vec<(String, String)> {
    // A loader's first bundle is its own language's catalog, English's behind it.
    let own = |loader: &super::Loader| loader.bundles.first().is_some_and(|bundle| bundle.has_message(id));
    (LOADERS.iter())
        .filter(|(language, loader)| !language.normalizing_eq(FALLBACK) && own(loader))
        .map(|(language, loader)| (language.to_string(), loader.text(id, None)))
        .collect()
}
