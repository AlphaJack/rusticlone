//! The first collection: profiles from a few answers, which the web UI asks and the CLI prints as an
//! example.

mod answers;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;
mod choice;
mod field;
mod profiles;
#[cfg(feature = "server")]
mod save;
#[cfg(test)]
mod tests;

#[cfg(feature = "server")]
use std::path::PathBuf;

#[cfg(feature = "server")]
pub(crate) use save::*;
pub(crate) use {answers::*, choice::*, field::*};

use crate::settings;

#[cfg(any(feature = "server", test))]
#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "setup-error")]
pub(crate) enum Error {
    Name,
    /// rustic reads it from wherever it runs, and backs up nothing if nothing is there, without failing.
    Relative {
        path: String,
    },
    NoSources,
    NoPrimary,
    #[fluent(transparent)]
    Schedule(#[from] settings::Error),
    NoCustom,
    #[fluent(transparent)]
    Notify(#[from] crate::notification::Error),
    /// The field, by its [`Field::label`].
    #[fluent(field = Field::text_of(field))]
    Incomplete {
        role: Role,
        field: &'static str,
    },
    /// The field, by its [`Field::label`], and the values it takes.
    #[fluent(field = Field::text_of(field))]
    NotAChoice {
        role: Role,
        field: &'static str,
        choices: String,
    },
    #[cfg(feature = "server")]
    NoDir,
    #[cfg(feature = "server")]
    #[fluent(path = path.display())]
    Exists {
        path: PathBuf,
    },
    /// rustic would back up the repository into itself and grow it at every run.
    #[cfg(feature = "server")]
    Inside,
    /// Neither empty nor a repository: rustic would mix its files in with others.
    #[cfg(feature = "server")]
    NotEmpty {
        folder: String,
    },
}
