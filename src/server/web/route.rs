//! The paths of the web UI, defined once: the router matches them, and pages and redirects link to them.

/// A page, action or asset, with the values in its path.
#[derive(Clone, Copy)]
pub(in crate::server) enum Route<'a> {
    Index,
    Health,
    Metrics,
    Status,
    Login,
    Logout,
    Events,
    Asset(&'a str),
    Setup,
    SetupNotifications,
    /// The free space of the local disk the first collection is saved to.
    SetupFree,
    /// Opens the folder of the saved profiles on this computer's desktop.
    ProfileFolder,
    /// Every profile in one file to keep away from this computer.
    Kit,
    /// The runs of every collection.
    Runs,
    Refresh,
    Collection(&'a str),
    CollectionRuns(&'a str),
    NewRun(&'a str),
    Run(&'a str, i64),
    Step(&'a str, i64, usize),
    CollectionRefresh(&'a str),
    /// A step's failure, by its [`crate::request::StepKey`].
    Failure(&'a str, &'a str),
    Notifications(&'a str),
    Repo(&'a str, &'a str),
    RepoRuns(&'a str, &'a str),
    RepoRefresh(&'a str, &'a str),
    Find(&'a str, &'a str),
    Snapshot(&'a str, &'a str, &'a str),
    Download(&'a str, &'a str, &'a str),
    Diff(&'a str, &'a str, &'a str),
}

/// A word of a path, or a value named as the handlers extract it.
#[derive(Clone)]
enum Part {
    Word(&'static str),
    Value(&'static str, String),
}

impl Route<'_> {
    fn parts(self) -> Vec<Part> {
        use Part::{Value, Word};
        let collection = |c: &str| vec![Word("collections"), Value("collection", c.to_owned())];
        let repo = |c, r: &str| [collection(c), vec![Word("repos"), Value("repo", r.to_owned())]].concat();
        let snapshot = |c, r, id: &str| [repo(c, r), vec![Word("snapshots"), Value("id", id.to_owned())]].concat();
        let run = |c, id: i64| [collection(c), vec![Word("runs"), Value("id", id.to_string())]].concat();
        let and = |parts: Vec<Part>, more: &'static str| [parts, vec![Word(more)]].concat();
        match self {
            Self::Index => Vec::new(),
            Self::Health => vec![Word("health")],
            Self::Metrics => vec![Word("metrics")],
            Self::Status => vec![Word("status")],
            Self::Login => vec![Word("login")],
            Self::Logout => vec![Word("logout")],
            Self::Events => vec![Word("events")],
            Self::Asset(file) => vec![Word("assets"), Value("file", file.to_owned())],
            Self::Setup => vec![Word("setup")],
            Self::SetupNotifications => vec![Word("setup"), Word("notifications")],
            Self::SetupFree => vec![Word("setup"), Word("free")],
            Self::ProfileFolder => vec![Word("setup"), Word("folder")],
            Self::Kit => vec![Word("kit")],
            Self::Runs => vec![Word("runs")],
            Self::Refresh => vec![Word("refresh")],
            Self::Collection(c) => collection(c),
            Self::CollectionRuns(c) => and(collection(c), "runs"),
            Self::NewRun(c) => and(and(collection(c), "runs"), "new"),
            Self::Run(c, id) => run(c, id),
            Self::Step(c, id, step) => [and(run(c, id), "steps"), vec![Value("step", step.to_string())]].concat(),
            Self::CollectionRefresh(c) => and(collection(c), "refresh"),
            Self::Failure(c, key) => [and(collection(c), "failures"), vec![Value("key", key.to_owned())]].concat(),
            Self::Notifications(c) => and(collection(c), "notifications"),
            Self::Repo(c, r) => repo(c, r),
            Self::RepoRuns(c, r) => and(repo(c, r), "runs"),
            Self::RepoRefresh(c, r) => and(repo(c, r), "refresh"),
            Self::Find(c, r) => and(repo(c, r), "find"),
            Self::Snapshot(c, r, id) => snapshot(c, r, id),
            Self::Download(c, r, id) => and(snapshot(c, r, id), "download"),
            Self::Diff(c, r, id) => and(snapshot(c, r, id), "diff"),
        }
    }

    /// The path, with `value` for each value.
    fn path(self, value: impl Fn(&'static str, &str) -> String) -> String {
        let parts = self.parts().into_iter().map(|p| match p {
            Part::Word(word) => format!("/{word}"),
            Part::Value(name, v) => format!("/{}", value(name, &v)),
        });
        Some(parts.collect::<String>()).filter(|p| !p.is_empty()).unwrap_or_else(|| "/".into())
    }

    /// As the router matches it: `/collections/{collection}`. The values are ignored.
    pub(super) fn pattern(self) -> String {
        self.path(|name, _| format!("{{{name}}}"))
    }
}

impl std::fmt::Display for Route<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.path(|_, v| {
            let Ok(encoded) = askama::filters::urlencode_strict(v);
            encoded.to_string()
        }))
    }
}

impl askama::filters::HtmlSafe for Route<'_> {}

#[cfg(test)]
mod tests {
    use super::Route;

    #[test]
    fn paths_and_patterns() {
        assert_eq!(Route::Index.to_string(), "/");
        assert_eq!(Route::Step("My Docs", 3, 1).to_string(), "/collections/My%20Docs/runs/3/steps/1");
        assert_eq!(Route::Failure("Docs", "copy:Docs/usb").to_string(), "/collections/Docs/failures/copy%3ADocs%2Fusb");
        assert_eq!(
            Route::Download("", "", "").pattern(),
            "/collections/{collection}/repos/{repo}/snapshots/{id}/download"
        );
    }
}
