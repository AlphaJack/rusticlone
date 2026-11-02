//! What the files that packages install say: in English, with each translation that has it.

use i18n_embed::{LanguageLoader as _, fluent::FluentLanguageLoader, unic_langid::LanguageIdentifier};

use super::{LOADER, LOADERS, Language};

impl Language {
    /// The language every message is in, for what is the same for everyone, such as the files packages install.
    pub(crate) fn english() -> Self {
        Self::accepted("")
    }
}

/// Message `id` in each translation whose own catalog has it, by language: `[("it", "…")]`.
pub(crate) fn translations(id: &str) -> Vec<(String, String)> {
    let fallback = LOADER.fallback_language();
    let own = |language: &LanguageIdentifier, loader: &FluentLanguageLoader| {
        loader.with_message_iter(language, |mut messages| Iterator::any(&mut messages, |m| m.id.name == id))
    };
    (LOADERS.iter())
        .filter(|(language, loader)| language != fallback && own(language, loader))
        .map(|(language, loader)| (language.to_string(), loader.get(id)))
        .collect()
}
