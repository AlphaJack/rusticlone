//! Without the desktop, no tray icon.

use std::sync::Arc;

use anyhow::Result;

use super::App;

pub(super) struct Tray(std::convert::Infallible);

impl Tray {
    pub(super) fn follow(self, _: Arc<App>, _: String, _: String) -> std::future::Ready<()> {
        match self.0 {}
    }
}

/// Runs `serve` without one.
pub(super) fn beside(_: bool, serve: impl FnOnce(Option<Tray>) -> Result<()> + Send + 'static) -> Result<()> {
    serve(None)
}
