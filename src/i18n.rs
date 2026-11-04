//! Text for users, from `i18n/<language>/rusticlone.ftl` in the language asked for, else English, message by
//! message. The [`t!`] macro and `#[derive(Fluent)]` check at build time that each message is in the English
//! one. The process's language is the user's; a page's is its visitor's ([`Language::accepted`]).

#[cfg(all(feature = "desktop", any(target_os = "linux", test)))]
pub(crate) mod packages;

use std::{
    cell::Cell,
    sync::{Arc, LazyLock},
};

use fluent::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};
pub(crate) use rusticlone_macros::{Fluent, t};
use unic_langid::LanguageIdentifier;

mod catalogs {
    rusticlone_macros::catalogs!();
}
use catalogs::{Catalogs, FALLBACK};

fn fallback() -> LanguageIdentifier {
    FALLBACK.parse().unwrap_or_default()
}

/// The process's text, in the language the user prefers.
static LOADER: LazyLock<Loader> = LazyLock::new(|| {
    #[expect(clippy::disallowed_methods, reason = "the language is the system's choice, not a setting")]
    let asked = requested(|key| std::env::var(key).ok());
    // macOS sets LANG only in a terminal, and Windows never: their settings list the languages instead.
    Loader::new(&asked.unwrap_or_else(|| sys_locale::get_locales().filter_map(|locale| locale.parse().ok()).collect()))
});

/// A loader for each catalog, for the pages.
#[cfg(feature = "server")]
static LOADERS: LazyLock<Vec<(LanguageIdentifier, Loader)>> =
    LazyLock::new(|| available().into_iter().map(|language| (language.clone(), Loader::new(&[language]))).collect());

/// The languages that have a catalog: the folders of `i18n`.
fn available() -> Vec<LanguageIdentifier> {
    let mut languages: Vec<LanguageIdentifier> =
        Catalogs::iter().filter_map(|path| path.split('/').next()?.parse().ok()).collect();
    languages.dedup();
    languages
}

/// The catalog of `language`, if it has one.
fn catalog(language: &LanguageIdentifier) -> Option<Arc<FluentResource>> {
    let file = Catalogs::get(&format!("{language}/{}.ftl", env!("CARGO_PKG_NAME")))?;
    let source = String::from_utf8(file.data.into_owned()).ok()?;
    // The macros reject a catalog with mistakes at build time, so what parses is all there is.
    Some(Arc::new(FluentResource::try_new(source).unwrap_or_else(|(resource, _)| resource)))
}

/// A language's messages, with English's behind them.
struct Loader {
    /// A bundle for each language: the chosen one first, English last.
    bundles: Vec<FluentBundle<Arc<FluentResource>>>,
}

impl Loader {
    /// The first of `languages` that has a catalog, with English for the messages it lacks.
    fn new(languages: &[LanguageIdentifier]) -> Self {
        let fallback = fallback();
        let available = available();
        let strategy = fluent_langneg::NegotiationStrategy::Filtering;
        let chosen = fluent_langneg::negotiate_languages(languages, &available, Some(&fallback), strategy);
        let mut chain = vec![chosen.first().map_or_else(|| fallback.clone(), |language| (*language).clone())];
        if chain[0] != fallback {
            chain.push(fallback);
        }
        // The terms the code knows: `{ -name }`, the command's name, `{ -app }`, the app's, `{ -env }`, its
        // variables' prefix, and `{ -description }`, Cargo's description of it.
        let terms = FluentResource::try_new(format!(
            "-name = {}\n-app = {}\n-env = {}\n-description = {}\n",
            crate::NAME,
            crate::APP_NAME,
            crate::ENV_PREFIX,
            env!("CARGO_PKG_DESCRIPTION")
        ))
        .ok()
        .map(Arc::new);
        let bundles = chain.into_iter().filter_map(|language| {
            let resource = catalog(&language)?;
            let mut bundle = FluentBundle::new_concurrent(vec![language]);
            // Fluent wraps each argument in Unicode isolation marks, which terminals and notifiers print.
            bundle.set_use_isolating(false);
            bundle.add_resource_overriding(Arc::clone(&resource));
            if let Some(terms) = &terms {
                bundle.add_resource_overriding(Arc::clone(terms));
            }
            Some(bundle)
        });
        Self { bundles: bundles.collect() }
    }

    /// The language that its messages are in.
    fn language(&self) -> LanguageIdentifier {
        (self.bundles.first()).and_then(|bundle| bundle.locales.first().cloned()).unwrap_or_else(fallback)
    }

    /// Message `id` with `args`, from the first catalog that has it, else `id`.
    fn text(&self, id: &str, args: Option<&FluentArgs<'_>>) -> String {
        let format = |bundle: &FluentBundle<Arc<FluentResource>>| {
            let pattern = bundle.get_message(id)?.value()?;
            Some(bundle.format_pattern(pattern, args, &mut Vec::new()).into_owned())
        };
        self.bundles.iter().find_map(format).unwrap_or_else(|| id.to_owned())
    }

    fn has(&self, id: &str) -> bool {
        self.bundles.iter().any(|bundle| bundle.has_message(id))
    }
}

/// The language that text is shown in.
#[derive(Clone, Copy)]
pub(crate) struct Language(&'static Loader);

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
        self.0.language().fmt(f)
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
    Language::current().0.text(id, (N > 0).then(|| args.into_iter().collect::<FluentArgs<'_>>()).as_ref())
}

/// Message `id`, if the catalog has it: for ids made at run time, which tests check.
pub(crate) fn find<const N: usize>(id: &str, args: [(&'static str, FluentValue<'static>); N]) -> Option<String> {
    Language::current().0.has(id).then(|| text(id, args))
}

/// The English catalog's ids.
#[cfg(test)]
pub(crate) fn ids() -> Vec<String> {
    let message = |entry: &fluent_syntax::ast::Entry<&str>| {
        if let fluent_syntax::ast::Entry::Message(message) = entry { Some(message.id.name.to_owned()) } else { None }
    };
    catalog(&fallback()).map(|english| english.entries().filter_map(message).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    /// A message made at run time is found only if the catalog has it.
    #[test]
    fn only_a_message_of_the_catalog_is_found() {
        assert_eq!(super::find("field-region", []), Some("Region".to_owned()));
        assert_eq!(super::find("field-no-such-thing", []), None);
    }

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
            super::ids().into_iter().filter(|id| super::LOADER.text(id, None).contains("{-")).collect();
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
