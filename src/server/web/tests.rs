//! Each page rendered from its view: valid HTML, unique ids, and what the view decided.

use std::collections::BTreeSet;

use askama::Template;
use jiff::Timestamp;

use super::{
    Base,
    collections::{CollectionInput, CollectionMain, IndexMain},
    forms::SetupFormView,
    render_str,
    repo::{RepoMain, RepoView},
    runs::{LogLineView, LogLines, LogView, RunRow},
};
use crate::{
    engine::{CollectionOverview, Issue, RepoOverview, RepoStatus, Schedule},
    live::{Activity, Running},
    log::Level,
    profile::Sources,
    request::{RunKind, Step, StepKey, Trigger},
    result::{Status, StepResult},
    rustic::{Origin, Progress},
    server::listing::Listing,
    state::{Holds, Inventory, Lag, LastStep, LogLine, ResultRecord, RunRecord, RunStep, StepRecord},
};

fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
}

/// How a repository in sync reads, which the catalog words.
fn synced() -> String {
    RepoStatus::InSync.to_string()
}

fn repo(name: &str, status: RepoStatus) -> RepoOverview {
    RepoOverview { name: name.into(), repository: Some(format!("/srv/{name}")), status, ..Default::default() }
}

fn listed(snapshots: usize, stored: u64) -> Listing {
    let snapshot = |i: usize| {
        serde_json::from_value(serde_json::json!({
            "id": format!("{i:064x}"), "time": "2026-10-16T03:00:00Z", "paths": ["/docs"]
        }))
        .unwrap()
    };
    let info =
        crate::state::RepoInfo { snapshots: (0..snapshots).map(snapshot).collect(), stored: Some(stored), data: None };
    Listing::Listed(Inventory { listed: ts("2026-10-16T12:00:00Z"), result: Ok(info), size_outdated: false })
}

/// `Docs`, backed up this morning to its primary and `Docs-usb`, with `Docs-offsite` behind.
fn healthy() -> CollectionInput {
    let backup = LastStep { last_ok: Some(ts("2026-10-16T03:10:00Z")), ..LastStep::default() };
    let mut usb = repo("Docs-usb", RepoStatus::InSync);
    usb.copy = Some(backup.clone());
    let mut offsite = repo("Docs-offsite", RepoStatus::Behind { since: ts("2026-10-16T03:00:00Z") });
    offsite.lag = Some(Lag {
        checked_at: ts("2026-10-16T03:11:00Z"),
        behind_since: Some(ts("2026-10-16T03:00:00Z")),
        listed: true,
    });
    let overview = CollectionOverview {
        title: "Docs".into(),
        last_backup: Some(ts("2026-10-16T03:10:00Z")),
        backup_schedule: Some(Schedule {
            cron: "0 3 * * *".into(),
            next: Some(ts("2026-10-17T03:00:00Z")),
            error: None,
        }),
        snapshot: Some(backup),
        primary: RepoOverview { latest_snapshot: Some(ts("2026-10-16T03:00:00Z")), ..repo("Docs", RepoStatus::InSync) },
        targets: vec![usb, offsite],
        ..CollectionOverview::default()
    };
    let docs = Sources::new(vec!["/home/me/docs".into()], vec!["/home/me/docs".into()], "laptop".into());
    CollectionInput {
        overview,
        runs: Vec::new(),
        listings: [("Docs".into(), listed(2, 4096)), ("Docs-usb".into(), listed(2, 4096))].into(),
        existing: ["Docs", "Docs-usb", "Docs-offsite"].map(String::from).into(),
        remote: ["Docs-offsite".to_owned()].into(),
        kept: BTreeSet::new(),
        file: Some("/etc/rustic/Docs.toml".into()),
        notifies: true,
        configured: true,
        copy_blocked: None,
        sources: vec![docs],
        snapshots: Vec::new(),
        listed: true,
    }
}

/// A rendered page, checked for valid HTML and unique ids.
struct Page(scraper::Html);

impl Page {
    fn of(page: &impl Template) -> Self {
        Self::under(page, &Base::default())
    }

    fn under(page: &impl Template, base: &Base) -> Self {
        let html = scraper::Html::parse_fragment(&render_str(page, base));
        assert!(html.errors.is_empty(), "{:?}", html.errors);
        let ids: Vec<_> =
            html.select(&scraper::Selector::parse("[id]").unwrap()).filter_map(|e| e.attr("id")).collect();
        let unique: BTreeSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "{ids:?}");
        Self(html)
    }

    /// The text of each element that `css` selects, with its parts separated by spaces.
    fn texts(&self, css: &str) -> Vec<String> {
        let words = |e: scraper::ElementRef<'_>| {
            e.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
        };
        self.0.select(&scraper::Selector::parse(css).unwrap()).map(words).collect()
    }

    /// The `attr` of each element that `css` selects and that has it.
    fn attrs(&self, css: &str, attr: &str) -> Vec<String> {
        self.0.select(&scraper::Selector::parse(css).unwrap()).filter_map(|e| Some(e.attr(attr)?.to_owned())).collect()
    }
}

#[test]
#[expect(clippy::too_many_lines, reason = "tests one page in each state")]
fn collection_pages() {
    let page = Page::of(&CollectionMain::build(healthy(), LogView::default()));
    let tag = |status: RepoStatus| format!("success {status}");
    let behind = RepoStatus::Behind { since: ts("2026-10-16T03:00:00Z") };
    assert_eq!(page.texts("td .tag"), [tag(RepoStatus::InSync), tag(RepoStatus::InSync), tag(behind)]);
    // Copy shows where it helps: on the primary and the target behind, not on the one in sync.
    let copies = page.0.select(&scraper::Selector::parse(".tools use[href='#copy']").unwrap()).count();
    assert_eq!(copies, 2);
    // Each source and repository shows whether it is on a disk or reached over a network.
    assert_eq!(page.texts("td [data-copy] use").len(), 4);
    // A browser on this computer shows each repository's profile and opens each source's folder instead.
    let mut local_browser = CollectionMain::build(healthy(), LogView::default());
    local_browser.local_browser = true;
    let local_browser = Page::of(&local_browser);
    let shown = local_browser.attrs("td button[hx-post$='/folder']", "hx-vals");
    assert_eq!(shown.iter().filter(|v| v.contains(r#""repo""#)).count(), 3, "{shown:?}");
    assert_eq!(shown.iter().filter(|v| v.contains(r#""source": "0/0""#)).count(), 1, "{shown:?}");
    assert!(local_browser.texts("td [data-copy] use").is_empty());
    assert_eq!(page.0.select(&scraper::Selector::parse("use[href='#cloud']").unwrap()).count(), 1);
    assert!(page.texts(".issues li").is_empty());

    let mut failing = healthy();
    failing.overview.level = Level::Failure;
    failing.overview.issues = vec![
        Issue::Unreadable { error: "source /home/me/docs is missing".into() },
        Issue::CopyFailed {
            step: StepKey::Copy("Docs".into(), "Docs-usb".into()),
            error: (&StepResult::Failed {
                step: "copy to Docs-usb".into(),
                error: "unreachable".into(),
                stopped: false,
            })
                .into(),
            by: Some(RunStep { collection: "Docs".into(), run: 7, step: Some(1), scheduled: false }),
        },
    ];
    let failing = Page::of(&CollectionMain::build(failing, LogView::default()));
    // The worst first, each with its dot, whose text is kept when copied.
    assert_eq!(
        failing.texts(".issues li"),
        ["failing source /home/me/docs is missing", "degraded copy to Docs-usb: unreachable"]
    );
    assert_eq!(failing.texts(".issues .dot.failure").len(), 1);
    // Only a step's issue can be dismissed. The next attempt reports it again if it persists.
    assert_eq!(failing.texts(".issues [title=Dismiss]").len(), 1);
    // A step's issue links to the run that caused it.
    assert_eq!(failing.attrs(".issues a", "href"), ["/collections/Docs/runs/7#log"]);

    let run = |started: &str, status: Status, steps: Vec<StepRecord>| RunRecord {
        id: ts(started).as_millisecond(),
        collection: "Docs".into(),
        label: "backup".into(),
        kind: RunKind::Backup,
        scope: None,
        trigger: Trigger::Schedule,
        pid: 1,
        started: ts(started),
        finished: (status != Status::Running).then(|| ts(started)),
        status,
        results: Vec::new(),
        calls: Vec::new(),
        steps,
    };
    let step = |step: Step, status: Status, lines: &[StepResult]| StepRecord {
        status,
        results: lines.iter().map(ResultRecord::from).collect(),
        ..StepRecord::queued(step.label(), Vec::new())
    };
    let copy_failed =
        StepResult::Failed { step: "copy to Docs-usb".into(), error: "unreachable".into(), stopped: false };
    let backup = run(
        "2026-10-16T03:00:00Z",
        Status::Failed,
        vec![
            StepRecord {
                started: Some(ts("2026-10-16T03:00:00Z")),
                finished: Some(ts("2026-10-16T04:04:00Z")),
                waited: 3600,
                ..step(
                    Step::Snapshot { repo: "Docs".into() },
                    Status::Succeeded,
                    &[StepResult::BackedUp {
                        repo: "Docs".into(),
                        snapshots: vec!["0123abcd".into()],
                        new_repository: false,
                    }],
                )
            },
            step(Step::Copy { to: "Docs-usb".into() }, Status::Failed, &[copy_failed]),
            step(Step::Forget { repo: "Docs".into() }, Status::Cancelled, &[]),
        ],
    );
    // Waited an hour for another backup, and ran for five minutes.
    let backup = RunRecord { finished: Some(ts("2026-10-16T04:05:00Z")), ..backup };
    // The summary lists every repository it succeeded on, not only the first.
    let copies = RunRecord {
        results: [
            StepResult::InSync { target: "Docs-usb".into() },
            StepResult::Copied { target: "Docs-offsite".into(), snapshots: 1 },
        ]
        .iter()
        .map(ResultRecord::from)
        .collect(),
        steps: Vec::new(),
        ..backup.clone()
    };
    assert_eq!(
        RunRow::new("Docs", "Docs", &copies, None).summary,
        format!("Docs-usb: {}; Docs-offsite: copied 1 snapshot", synced())
    );
    let mut history = healthy();
    let holds = Holds { using: vec!["cpu".into(), "disk:/mnt/usb".into()], waiting_for: None };
    let snapshot = StepRecord {
        started: Some(ts("2026-10-16T11:58:00Z")),
        holds,
        ..step(Step::Snapshot { repo: "Docs".into() }, Status::Running, &[])
    };
    let running = RunRecord { ..run("2026-10-16T11:58:00Z", Status::Running, vec![snapshot]) };
    let live = Running {
        id: running.id,
        kind: RunKind::Backup,
        label: "backup".into(),
        trigger: Trigger::Web,
        started: running.started,
        cancelling: false,
        elsewhere: false,
        activities: vec![Activity {
            step: Some(0),
            doing: None,
            on: None,
            since: running.started,
            progress: Some(Progress { percent_done: 0.42, ..Progress::default() }),
        }],
        holds: running.holds(),
    };
    history.runs = vec![RunRow::new("Docs", "Docs", &running, Some(&live))];
    // The running step shows the progress, and for how long it has run.
    let steps = LogView::build(&history.runs[0], Some(running), (LogLines::default(), 0, false)).steps;
    assert!(steps[0].progress.as_ref().is_some_and(|p| p.percent == Some(42)) && steps[0].since.is_some());
    history.runs.extend(
        [
            backup.clone(),
            run("2026-10-15T03:00:00Z", Status::Cancelled, Vec::new()),
            run("2026-10-14T03:00:00Z", Status::Running, Vec::new()),
        ]
        .iter()
        .map(|r| RunRow::new("Docs", "Docs", r, None)),
    );
    let log = LogView::build(&history.runs[1], Some(backup), (LogLines::default(), 0, false));
    let history = Page::of(&CollectionMain::build(history, log));
    // The run whose log is shown, with its duration excluding the wait; how the others ended.
    let shown = history.texts("tr[aria-current]");
    assert!(shown.len() == 1 && shown[0].contains("1 hour 5 minutes"), "{shown:?}");
    assert_eq!(
        history.texts("td.ellipsis"),
        ["1. snapshot to Docs", "copy to Docs-usb: unreachable", "cancelled", "unfinished: its process stopped"]
    );
    // The running run: for how long, and a way to stop it.
    assert_eq!(history.attrs("td.right time", "datetime"), ["2026-10-16T11:58:00Z"]);
    assert_eq!(history.texts("tr button[title=Stop]").len(), 1);
    // What it holds, which another run may wait for.
    assert_eq!(history.texts("tbody tr:first-child td:nth-last-child(2) .tag"), ["cpu", "disk:/mnt/usb"]);
    // Each step with its wait, its duration without it, and its results. The forget that the stop interrupted has none.
    assert_eq!(
        history.texts("#log tbody tr"),
        [
            "1 succeeded snapshot to Docs 1 hour 4 minutes Docs: backed up 0123abcd",
            "2 failed copy to Docs-usb unreachable",
            "3 cancelled forget on Docs cancelled"
        ]
    );
}

#[test]
fn overview() {
    let mut photos = healthy();
    (photos.overview.primary.name, photos.overview.title) = ("Photos".into(), "Photos".into());
    photos.overview.level = Level::Failure;
    photos.overview.issues = vec![Issue::Unreadable { error: "source /home/me/photos is missing".into() }];
    let page = Page::of(&IndexMain::build(vec![healthy(), photos]));
    assert_eq!(page.texts("td a.strong"), ["Docs", "Photos"]);
    // The dots show each collection's state, with no counts above them.
    assert!(page.texts(".meta").is_empty());
    // A row per repository. The collection's issues are on its own page.
    assert_eq!(page.texts("tbody").len(), 2);
    assert_eq!(page.texts("td .tag").len(), 6);
    assert!(page.texts("td[title]").is_empty());
    assert_eq!(page.texts("[hx-post='/runs']"), ["Back up all", "Check all", "Prune all"]);
    assert_eq!(page.texts("[hx-post='/refresh']"), ["Refresh all"]);
}

/// A broken collection has no page: only its name and the error are shown.
#[test]
fn a_broken_profile_links_nowhere() {
    let page = Page::of(&IndexMain::build(vec![CollectionInput { configured: false, ..healthy() }]));
    assert!(
        page.attrs("a[href]", "href").iter().all(|l| !l.starts_with("/collections/Docs")),
        "{:?}",
        page.attrs("a[href]", "href")
    );
}

/// A repository that does not exist (not created, or missing) has no page, so its name is not a link.
#[test]
fn a_repository_not_there_links_nowhere() {
    let input = || {
        let mut input = healthy();
        input.existing.remove("Docs-usb");
        input
    };
    for page in
        [Page::of(&IndexMain::build(vec![input()])), Page::of(&CollectionMain::build(input(), LogView::default()))]
    {
        let links = page.attrs("a[href]", "href");
        assert!(!links.iter().any(|l| l.ends_with("/repos/Docs-usb")), "{links:?}");
        assert!(links.iter().any(|l| l.ends_with("/repos/Docs-offsite")), "{links:?}");
    }
}

/// Until its primary is first listed, a collection's state is unknown: a spinner, no color.
#[test]
fn unlisted_profile_shows_a_spinner() {
    let page = Page::of(&IndexMain::build(vec![CollectionInput { listed: false, ..healthy() }]));
    assert_eq!(page.texts("td .spinner").len(), 1);
    assert!(page.texts("td > .dot").is_empty());
}

#[test]
fn links_start_where_the_reverse_proxy_serves_the_web_ui() {
    let base = Base::new("/backups/");
    let pages = [
        Page::under(&IndexMain::build(vec![healthy()]), &base),
        Page::under(&CollectionMain::build(healthy(), LogView::default()), &base),
        Page::under(&RepoMain { repo: RepoView::build(healthy(), "Docs") }, &base),
    ];
    for page in pages {
        for attr in ["href", "hx-get", "hx-post", "hx-delete"] {
            let links = page.attrs(&format!("[{attr}]"), attr);
            let outside: Vec<_> = links.iter().filter(|l| l.starts_with('/') && !l.starts_with("/backups/")).collect();
            assert!(outside.is_empty(), "{outside:?}");
        }
    }
    for spoofed in ["//evil.example", "/a\"b", "relative", "/"] {
        assert_eq!(Base::new(spoofed).0, "", "{spoofed}");
    }
}

#[test]
fn repo_pages() {
    let page = |repo| Page::of(&RepoMain { repo: RepoView::build(healthy(), repo) });
    let primary = page("Docs");
    assert_eq!(primary.texts("h1"), [format!("{} Docs", synced())]);
    assert_eq!(primary.texts("tbody tr").len(), 2);
    assert!(primary.texts(".spinner").is_empty());
    assert_eq!(page("Docs-usb").texts("h1"), [format!("{} Docs-usb", synced())]);
    // Never listed: it is listed now, with nothing to show meanwhile.
    let unlisted = page("Docs-offsite");
    assert_eq!(unlisted.texts("h2 .spinner[title=listing]").len(), 1);
    assert!(unlisted.texts("tbody tr").is_empty());
}

#[test]
fn log_lines_are_timed_in_the_local_time_zone_and_tell_where_they_come_from() {
    let origin = Origin { step: Some(1), call: Some(0) };
    let line = LogLine { at: Some(ts("2026-10-22T09:03:42Z")), origin, text: "[Docs] rustic backup".into() };
    let html = LogLines(vec![LogLineView::from(line)]).render().unwrap();
    let text = html.strip_prefix(r#"<span data-step="1" data-call="0">"#).and_then(|t| t.strip_suffix("\n</span>"));
    let (time, rest) = text.and_then(|t| t.split_once(' ')).unwrap();
    assert!(time.len() == 8 && time.ends_with(":03:42") && rest == "[Docs] rustic backup", "{html}");
}

/// Every word on a page is a [`super::Label`] or a value. Templates contain no text.
#[test]
#[expect(clippy::disallowed_methods, reason = "reads the template files")]
fn templates_contain_no_literal_text() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for file in std::fs::read_dir(dir).unwrap() {
        let path = file.unwrap().path();
        let mut html = std::fs::read_to_string(&path).unwrap();
        for (open, close) in [("{{", "}}"), ("{%", "%}"), ("{#", "#}")] {
            while let Some((before, after)) = html.split_once(open) {
                html = format!("{before}{}", after.split_once(close).map_or("", |(_, after)| after));
            }
        }
        let page = scraper::Html::parse_fragment(&html);
        let shown = page.tree.values().filter_map(|node| {
            let attrs = |e: &scraper::node::Element| {
                ["title", "aria-label", "placeholder", "alt"].iter().filter_map(|a| e.attr(a)).collect()
            };
            node.as_text().map(|text| text.to_string()).or_else(|| node.as_element().map(attrs))
        });
        let has_text = |s: &String| !attributes(s) && s.replace("&nbsp;", "").chars().any(char::is_alphabetic);
        let words: Vec<String> = shown.filter(has_text).collect();
        assert!(words.is_empty(), "{}: {words:?}", path.display());
    }
}

/// Whether `text` is only `name="value"` attributes that a macro adds to a tag.
fn attributes(text: &str) -> bool {
    let (mut names, mut chars) = (String::new(), text.chars());
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => _ = chars.by_ref().find(|&q| q == c),
            c => names.push(c),
        }
    }
    let name = |t: &str| t.strip_suffix('=').is_some_and(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
    names.split_whitespace().all(name)
}

#[test]
fn notification_toggles_follow_the_urls() {
    let setup = crate::first_run::Setup { notify: "tgram://token/chat mailtos://me".into(), ..Default::default() };
    let page = Page::of(&SetupFormView::new(setup));
    assert_eq!(page.attrs("[data-schemes][checked]", "data-schemes"), ["mailto mailtos", "tgram"]);
    // Generic endpoints are typed, not toggled.
    assert!(!page.attrs("[data-schemes]", "data-schemes").contains(&"json jsons".into()));
    let templates = page.attrs("[data-schemes=discord]", "data-template");
    assert_eq!(templates, ["discord://{webhook_id}/{webhook_token}"]);
}
