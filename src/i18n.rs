//! Text for users, from `i18n/<language>/rusticlone.ftl` in the language asked for, else English, message by
//! message. The [`t!`] macro and `#[derive(Fluent)]` check at build time that each message is in the English
//! one. The process's language is the user's; a page's is its visitor's ([`Language::accepted`]).

use std::{
    cell::Cell,
    sync::{Arc, LazyLock},
};

use fluent::{FluentArgs, FluentResource, FluentValue};
use i18n_embed::{LanguageLoader as _, fluent::FluentLanguageLoader, unic_langid::LanguageIdentifier};
pub(crate) use rusticlone_macros::{Fluent, t};

mod catalogs {
    rusticlone_macros::catalogs!();
}
use catalogs::{Catalogs, FALLBACK};

fn fallback() -> LanguageIdentifier {
    FALLBACK.parse().unwrap_or_default()
}

/// The process's text, in the language the user prefers.
static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    #[expect(clippy::disallowed_methods, reason = "the language is the system's choice, not a setting")]
    let asked = requested(|key| std::env::var(key).ok());
    // macOS sets LANG only in a terminal, and Windows never: their settings list the languages instead.
    loader(&asked.unwrap_or_else(|| sys_locale::get_locales().filter_map(|locale| locale.parse().ok()).collect()))
});

/// A loader for each catalog, for the pages.
#[cfg(feature = "server")]
static LOADERS: LazyLock<Vec<(LanguageIdentifier, FluentLanguageLoader)>> = LazyLock::new(|| {
    let languages = LOADER.available_languages(&Catalogs).unwrap_or_default();
    languages.into_iter().map(|language| (language.clone(), loader(&[language]))).collect()
});

/// The first of `languages` that has a catalog, with English for the messages it lacks.
fn loader(languages: &[LanguageIdentifier]) -> FluentLanguageLoader {
    let loader = FluentLanguageLoader::new(env!("CARGO_PKG_NAME"), fallback());
    // English is embedded, so only a broken build fails to load it.
    i18n_embed::select(&loader, &Catalogs, languages).ok();
    // Fluent wraps each argument in Unicode isolation marks, which terminals and notifiers print. Set once
    // the languages are loaded, as it applies to their bundles.
    loader.set_use_isolating(false);
    // The terms the code knows: `{ -name }`, the command's name, `{ -app }`, the app's, `{ -env }`, its
    // variables' prefix, and `{ -description }`, Cargo's description of it.
    if let Ok(terms) = FluentResource::try_new(format!(
        "-name = {}\n-app = {}\n-env = {}\n-description = {}\n",
        crate::NAME,
        crate::APP_NAME,
        crate::ENV_PREFIX,
        env!("CARGO_PKG_DESCRIPTION")
    )) {
        let terms = Arc::new(terms);
        loader.with_bundles_mut(|bundle| bundle.add_resource_overriding(Arc::clone(&terms)));
    }
    loader
}

/// The language that text is shown in.
#[derive(Clone, Copy)]
pub(crate) struct Language(&'static FluentLanguageLoader);

thread_local! {
    /// Set while [`Language::scope`] runs.
    static SCOPED: Cell<Option<Language>> = const { Cell::new(None) };
}

#[cfg(feature = "server")]
tokio::task_local! {
    /// Set while [`Language::scope_task`] runs: a request's.
    static TASK: Language;
}

impl Language {
    /// The one that an `Accept-Language` header prefers among the catalogs, else English.
    #[cfg(feature = "server")]
    pub(crate) fn accepted(header: &str) -> Self {
        let available: Vec<LanguageIdentifier> = LOADERS.iter().map(|(language, _)| language.clone()).collect();
        let fallback = fallback();
        let asked = fluent_langneg::accepted_languages::parse(header);
        let strategy = fluent_langneg::NegotiationStrategy::Lookup;
        let chosen = fluent_langneg::negotiate_languages(&asked, &available, Some(&fallback), strategy);
        let found =
            |language: &LanguageIdentifier| LOADERS.iter().find(|(l, _)| l == language).map(|(_, loader)| loader);
        Self(chosen.first().and_then(|language| found(language)).or_else(|| found(&fallback)).unwrap_or(&LOADER))
    }

    /// The language every message is in, for what is the same for everyone, such as the files packages install.
    #[cfg(all(feature = "server", any(target_os = "linux", test)))]
    pub(crate) fn english() -> Self {
        Self::accepted("")
    }

    /// [`Self::scope`]'s, else a request's, else the process's.
    pub(crate) fn current() -> Self {
        #[cfg(feature = "server")]
        let task = TASK.try_with(|language| *language).ok();
        #[cfg(not(feature = "server"))]
        let task = None;
        SCOPED.get().or(task).unwrap_or(Self(&LOADER))
    }

    /// Runs `f` with text in this language: on a thread that a request hands its work to.
    #[cfg(feature = "server")]
    pub(crate) fn scope<T>(self, f: impl FnOnce() -> T) -> T {
        let before = SCOPED.replace(Some(self));
        let out = f();
        SCOPED.set(before);
        out
    }

    /// Runs `future` with text in this language, on its task.
    #[cfg(feature = "server")]
    pub(crate) async fn scope_task<F: Future>(self, future: F) -> F::Output {
        TASK.scope(self, future).await
    }
}

/// Its tag, `en`, for `<html lang>` and `Content-Language`.
impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.current_language().fmt(f)
    }
}

/// The languages asked for, as gettext reads them: `LANGUAGE`'s list, else the first of `LC_ALL`, `LC_MESSAGES`
/// and `LANG` that is set, `it_IT.UTF-8`. `C` and `POSIX` are English; none set leaves it to the system.
fn requested(var: impl Fn(&str) -> Option<String>) -> Option<Vec<LanguageIdentifier>> {
    let set = |key: &str| var(key).filter(|v| !v.is_empty());
    let asked = set("LANGUAGE").or_else(|| ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(set))?;
    let languages = (asked.split(':')).filter_map(|language| language.split(['.', '@']).next());
    let languages = languages.filter(|language| !matches!(*language, "" | "C" | "POSIX"));
    Some(languages.filter_map(|language| language.replace('_', "-").parse().ok()).collect())
}

/// Message `id` with `args`, which the macros check.
pub(crate) fn text<const N: usize>(id: &str, args: [(&'static str, FluentValue<'static>); N]) -> String {
    Language::current().0.get_args_fluent(id, (N > 0).then(|| args.into_iter().collect::<FluentArgs<'_>>()).as_ref())
}

/// Message `id` in each translation whose own catalog has it, by language: `[("it", "…")]`.
#[cfg(all(feature = "server", any(target_os = "linux", test)))]
pub(crate) fn translations(id: &str) -> Vec<(String, String)> {
    let fallback = fallback();
    let own = |language: &LanguageIdentifier, loader: &FluentLanguageLoader| {
        loader.with_message_iter(language, |mut messages| Iterator::any(&mut messages, |m| m.id.name == id))
    };
    (LOADERS.iter())
        .filter(|(language, loader)| *language != fallback && own(language, loader))
        .map(|(language, loader)| (language.to_string(), Language(loader).scope(|| text(id, []))))
        .collect()
}

/// Message `id`, if the catalog has it: for ids made at run time, which tests check.
pub(crate) fn find<const N: usize>(id: &str, args: [(&'static str, FluentValue<'static>); N]) -> Option<String> {
    Language::current().0.has(id).then(|| text(id, args))
}

/// The English catalog's ids.
#[cfg(test)]
pub(crate) fn ids() -> Vec<String> {
    LOADER.with_message_iter(&fallback(), |messages| messages.map(|m| m.id.name.to_owned()).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn language_follows_gettext() {
        let asked = |vars: &[(&str, &str)]| {
            let found = |key: &str| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| (*v).to_owned());
            super::requested(found).map(|asked| asked.iter().map(ToString::to_string).collect::<Vec<_>>())
        };
        assert_eq!(asked(&[("LANG", "it_IT.UTF-8"), ("LC_MESSAGES", "de_DE")]).unwrap(), ["de-DE"]);
        assert_eq!(asked(&[("LANGUAGE", "fr:it"), ("LC_ALL", "de_DE.UTF-8")]).unwrap(), ["fr", "it"]);
        assert_eq!(asked(&[("LANGUAGE", ""), ("LANG", "pt_BR@euro")]).unwrap(), ["pt-BR"]);
        assert!(asked(&[("LANG", "C.UTF-8")]).unwrap().is_empty() && asked(&[]).is_none());
    }

    /// A page is in the language its visitor prefers among the catalogs, else English.
    #[cfg(feature = "server")]
    #[test]
    fn a_page_is_in_the_accepted_language() {
        for header in ["", "*", "en-GB,en;q=0.8", "xx-YY, zz;q=0.5"] {
            assert_eq!(super::Language::accepted(header).to_string(), super::FALLBACK, "{header}");
        }
        let italian = super::Language::accepted("it");
        assert_eq!(italian.scope(|| super::Language::current().to_string()), italian.to_string());
    }

    /// Every term a message names exists: an unknown one would show as `{-term}`.
    #[test]
    fn every_term_resolves() {
        let unresolved: Vec<String> =
            super::ids().into_iter().filter(|id| super::LOADER.get(id).contains("{-")).collect();
        assert!(unresolved.is_empty(), "unknown terms in {unresolved:?}");
    }
}

/// A message's variable, as `(&&Arg(&value)).value()` picks it: a number, which plurals select on, else its
/// text. The macros write this.
pub(crate) struct Arg<'a, T: ?Sized>(pub(crate) &'a T);

pub(crate) trait Number {
    fn value(&self) -> FluentValue<'static>;
}

impl<T: Numeric> Number for &Arg<'_, T> {
    fn value(&self) -> FluentValue<'static> {
        self.0.number()
    }
}

/// The types that plurals can select on.
pub(crate) trait Numeric {
    fn number(&self) -> FluentValue<'static>;
}

impl<T: Numeric + ?Sized> Numeric for &T {
    fn number(&self) -> FluentValue<'static> {
        (**self).number()
    }
}

macro_rules! numeric {
    ($($n:ty),*) => {$(
        impl Numeric for $n {
            fn number(&self) -> FluentValue<'static> {
                FluentValue::from(*self)
            }
        }
    )*};
}
numeric!(u8, u16, u32, u64, usize, i32, i64, f64);

pub(crate) trait Text {
    fn value(&self) -> FluentValue<'static>;
}

impl<T: std::fmt::Display + ?Sized> Text for Arg<'_, T> {
    fn value(&self) -> FluentValue<'static> {
        FluentValue::from(self.0.to_string())
    }
}
