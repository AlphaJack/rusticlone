#![expect(clippy::panic, reason = "missing HTML sections fail the test")]

use std::{
    fs, thread,
    time::{Duration, Instant},
};

use rusticlone::{Event, Status};
use serde_json::json;

use crate::{
    client::Server,
    common::{
        KEEP, NAME, Sandbox, cmd, docs_with_remote, global_env, json_has, noise, slow_profile, stdin_profile, var,
        wait_for,
    },
};

#[test]
fn server_runs_due_jobs_and_serves_status() {
    let sb = docs_with_remote("");
    sb.common_env(&[("BACKUP_CRON", "0 0 * * *")]);
    let server = sb.server(&[]);
    // Overdue at start: backup and copy run on the first tick.
    wait_for(60, "backup and copy", || {
        let profile = server.collection("Docs");
        let target = &profile["targets"][0];
        profile["snapshot"]["last_ok"].is_string()
            && target["lag"]["checked_at"].is_string()
            && target["lag"]["behind_since"].is_null()
    });
    let page = server.get("/");
    assert!(page.code == 200 && page.body.contains("Docs"), "{}", page.body);
    assert_eq!(server.collection("Docs")["targets"][0]["status"]["kind"], "in-sync");
    drop(server);
    assert_eq!(sb.snapshots("Docs-remote").len(), 1);
}

#[test]
fn web_ui_needs_the_token() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    // Without the token, neither other local users nor websites (DNS rebinding) get in.
    assert_eq!(server.request("GET", "/status", &[], "").code, 401);
    assert_eq!(server.request("GET", "/status", &["Authorization: Bearer wrong-token"], "").code, 401);
    let page = server.request("GET", "/", &[], "");
    assert!(page.code == 303 && page.head.contains("location: /login"), "{}", page.head);
    let htmx = server.request("GET", "/", &["HX-Request: true"], "");
    assert!(htmx.code == 401 && htmx.head.contains("hx-redirect: /login"), "{}", htmx.head);
    let field = r#"<input type="password" name="token""#;
    let page = server.request("GET", "/login", &[], "");
    assert!(page.code == 200 && page.body.contains(field), "{}", page.body);
    assert_eq!(server.request("GET", "/assets/style.css", &[], "").code, 200);

    let form = ["Content-Type: application/x-www-form-urlencoded"];
    let guess = server.request("POST", "/login", &form, "token=guess");
    assert!(
        guess.code == 401 && guess.body.contains(field) && guess.body.contains(r#"class="error""#),
        "{}",
        guess.body
    );
    let login = server.request("POST", "/login", &form, &format!("token={}", server.token));
    assert_eq!(login.code, 303);
    let cookie = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"), "{cookie}");
    // The token itself never reaches the browser.
    assert!(!cookie.contains(&server.token), "{cookie}");
    assert!(cookie.starts_with("rusticlone=") && cookie.contains("Max-Age=31536000"), "{cookie}");
    // Behind an HTTPS proxy, the browser binds it to this host.
    let https = [form[0], "X-Forwarded-Proto: https"];
    let secure = server.request("POST", "/login", &https, &format!("token={}", server.token));
    let secure = secure.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
    assert!(secure.starts_with("__Host-rusticlone=") && secure.ends_with("; Secure"), "{secure}");
    // A session that was not started here does not log in.
    assert_eq!(server.request("GET", "/", &["Cookie: rusticlone=made-up"], "").code, 303);
    let cookie = format!("Cookie: {}", cookie.split(';').next().unwrap());
    let page = server.request("GET", "/", &[&cookie], "");
    // The icons are inlined in the page.
    assert!(page.code == 200 && page.body.contains(r#"<symbol id="file""#), "{}", page.body);
    assert!(page.body.contains(r#"hx-post="/logout""#));
    // Browsers get a 404 page.
    let html = "Accept: text/html,application/xhtml+xml";
    let missing = server.request("GET", "/collections/Unknown", &[&cookie, html], "");
    assert!(missing.code == 404 && missing.body.contains("<h1>404</h1>"));
    // Not for htmx, which swaps the text in, nor for a form that was sent.
    let swapped = server.request("GET", "/collections/Unknown", &[&cookie, html, "HX-Request: true"], "");
    assert!(swapped.code == 404 && !swapped.body.contains("<h1>404</h1>"), "{}", swapped.body);
    let sent = server.request("POST", "/unknown", &[&cookie, html, "Sec-Fetch-Site: same-origin"], "");
    assert!(sent.code == 404 && !sent.body.contains("<h1>404</h1>"), "{}", sent.body);
    assert_eq!(server.request("GET", "/unknown", &[&cookie], "").code, 404);
    assert_eq!(server.request("GET", "/collections/Docs/unknown", &[&cookie], "").code, 404);
    assert_eq!(server.get("/collections/Unknown").code, 404);
    // A reverse proxy that adds the token: its users get pages, without a logout button.
    let auth = format!("Authorization: Bearer {}", server.token);
    let proxied = server.request("GET", "/", &[&auth, html], "");
    assert!(proxied.code == 200 && !proxied.body.contains(r#"hx-post="/logout""#), "{}", proxied.body);
    assert!(server.request("GET", "/collections/Unknown", &[&auth, html], "").body.contains("<h1>404</h1>"));
    // An empty fragment, so that the browser drops the token that `serve --open` put in it.
    let login = server.request("GET", "/login", &[&auth, html], "");
    assert!(login.code == 303 && login.head.contains("location: /#"), "{}", login.head);

    // A request that another website makes the browser send changes nothing (CSRF).
    let form = "Content-Type: application/x-www-form-urlencoded";
    let backup = |site: &str| server.request("POST", "/collections/Docs/runs", &[&cookie, form, site], "kind=backup");
    assert_eq!(backup("Sec-Fetch-Site: cross-site").code, 403);
    let started = backup("Sec-Fetch-Site: same-origin");
    assert!(started.code == 201 && started.body.contains(r#"class="flash success""#), "{}", started.body);
    // The page opens the run.
    let id = started.head.split("hx-location: /collections/Docs/runs/").nth(1).unwrap().lines().next().unwrap();
    assert_eq!(server.get(&format!("/collections/Docs/runs/{id}")).code, 200);
    wait_for(60, "the backup", || server.collection("Docs")["targets"][0]["lag"]["checked_at"].is_string());
    let output = server.output();
    assert!(server.logged(&json!({"collection": "Docs", "event": {"kind": "requested"}})), "{output}");
    // The web server's own debug events are not logged.
    assert!(!output.contains("accepted"), "{output}");
    // Logging out ends the session, whichever cookie holds it.
    assert_eq!(server.request("POST", "/logout", &[&cookie], "").code, 204);
    assert_eq!(server.request("GET", "/", &[&cookie], "").code, 303);
}

/// A token set in the environment (systemd, a container) stays the same across starts.
#[test]
fn the_token_can_be_set() {
    let sb = docs_with_remote("");
    let token = "a".repeat(32);
    let server = sb.server(&[(&var("ADMIN_TOKEN"), &token)]);
    assert_eq!(server.request("GET", "/status", &[&format!("Authorization: Bearer {token}")], "").code, 200);
    let form = "Content-Type: application/x-www-form-urlencoded";
    let login = server.request("POST", "/login", &[form], &format!("token={token}"));
    let cookie = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
    let cookie = format!("Cookie: {}", cookie.split(';').next().unwrap());
    drop(server);
    // Its sessions last across starts, until the token changes.
    let server = sb.server(&[(&var("ADMIN_TOKEN"), &token)]);
    assert_eq!(server.request("GET", "/", &[&cookie], "").code, 200);
    drop(server);
    let server = sb.server(&[(&var("ADMIN_TOKEN"), &"b".repeat(32))]);
    assert_eq!(server.request("GET", "/", &[&cookie], "").code, 303);
}

/// A generated token is kept, so that its sessions last across starts. A set one can be read from a
/// file, so that it is not in the environment.
#[test]
fn the_generated_token_is_kept() {
    let sb = docs_with_remote("");
    let first = sb.server(&[]).token.clone();
    assert_eq!(first.len(), 64);
    assert_eq!(sb.server(&[]).token, first);
    let file = sb.path("token");
    fs::write(&file, format!("{}\n", "c".repeat(32))).unwrap();
    let server = sb.server(&[(&format!("{}_FILE", var("ADMIN_TOKEN")), file.to_str().unwrap())]);
    let bearer = format!("Authorization: Bearer {}", "c".repeat(32));
    assert_eq!(server.request("GET", "/status", &[&bearer], "").code, 200);
}

/// The metrics token opens only `/metrics` and `/status`, for monitors. It cannot log in.
#[test]
fn the_metrics_token_reads_only_the_metrics() {
    let sb = docs_with_remote("");
    let metrics = "m".repeat(32);
    let server = sb.server(&[(&var("METRICS_TOKEN"), &metrics)]);
    let auth = format!("Authorization: Bearer {metrics}");
    for path in ["/metrics", "/status"] {
        assert_eq!(server.request("GET", path, &[&auth], "").code, 200, "{path}");
    }
    for path in ["/", "/collections/Docs", "/collections/Docs/repos/Docs-remote"] {
        assert_eq!(server.request("GET", path, &[&auth], "").code, 403, "{path}");
    }
    let form = "Content-Type: application/x-www-form-urlencoded";
    assert_eq!(server.request("POST", "/collections/Docs/runs", &[&auth, form], "kind=backup").code, 403);
    assert_eq!(server.request("POST", "/login", &[form], &format!("token={metrics}")).code, 401);
}

#[test]
fn runs_are_kept_with_their_log() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    // One run, with a step per repository, which the page opens.
    assert_eq!(server.post_form("/collections/Docs/runs", "kind=check").code, 201);
    wait_for(60, "the checks", || server.collection("Docs")["targets"][0]["check"]["last_ok"].is_string());
    let records = || sb.store().runs("Docs", usize::MAX);
    let checks = || records().into_iter().filter(|r| json!(r.kind) == "check" && r.finished.is_some()).count();
    wait_for(10, "the run to be recorded", || checks() == 1);
    let scopes: Vec<_> = records().iter().map(|r| (json!(r.kind), json!(r.trigger), r.scope.clone())).collect();
    assert_eq!(scopes, [(json!("check"), json!("web"), None), (json!("backup"), json!("cli"), None)]);
    let runs = card(&server.get("/collections/Docs").body, "runs");
    assert!(runs.contains(">web<") && runs.contains(">cli<"), "{runs}");
    // Newest first: the check, whose log the collection page shows. Picking another run replaces it.
    let table = runs.get(runs.find("<table").unwrap()..).unwrap();
    let ids: Vec<&str> =
        table.split(r#"href="/collections/Docs/runs/"#).skip(1).map(|s| s.split('#').next().unwrap()).collect();
    let log = card(&server.get("/collections/Docs").body, "log");
    assert!(log.contains("[Docs-remote] rustic check"), "{log}");
    let verified = |id: &str| {
        let run = records().into_iter().find(|r| r.id.to_string() == id).unwrap();
        run.all_results().any(|l| l.values["kind"] == "verified")
    };
    assert!(verified(ids[0]), "{ids:?}");
    // With its rustic commands, their durations and exits. A directory with a space, as on macOS, is quoted.
    let dir = sb.profile_dir().display().to_string();
    assert!(
        log.contains(
            "-P Docs-remote --json-progress --progress-interval 1s check --read-data --read-data-subset 1/10</code>"
        ) && [format!("cd {dir}"), format!("cd &#39;{dir}&#39;")].iter().any(|cd| log.contains(cd))
            && log.contains(r#"<span class="dot success""#),
        "{log}"
    );
    // Its commands and log lines tell which step made them: the primary's check and the target's, which run at
    // the same time, so in either order.
    assert!(
        [0, 1].iter().all(|step| log.contains(&format!(r#"<tr data-step="{step}">"#))
            && log.contains(&format!(r#"" data-step="{step}">"#))
            && log.contains(&format!(r#"<span data-step="{step}" data-call="#))),
        "{log}"
    );
    let picked = card(&server.get(&format!("/collections/Docs/runs/{}", ids[1])).body, "log");
    assert!(picked.contains("[Docs] rustic backup"), "{picked}");
    // Run ids are times: no run has 1, and its page is not another run's.
    assert_eq!(server.get("/collections/Docs/runs/1").code, 404);
    let page = server.get("/collections/Docs").body;
    assert!(page.matches(r#"id="log""#).count() == 1, "{page}");
    // The overview's runs link to them.
    let overview = card(&server.get("/").body, "recent-runs");
    let id = overview.split("/collections/Docs/runs/").nth(1).unwrap().split('#').next().unwrap();
    assert!(verified(id), "{id}");
}

/// The card with this `id` on a page.
pub(crate) fn card(page: &str, id: &str) -> String {
    shown_card(page, id).unwrap_or_else(|| panic!("no {id} in {page}"))
}

/// [`card`], if the page shows it yet.
fn shown_card(page: &str, id: &str) -> Option<String> {
    let start = page.find(&format!(r#"id="{id}""#))?;
    let end = page.get(start..)?.find("</section>").map_or(page.len(), |e| start + e);
    Some(page.get(start..end)?.to_owned())
}

#[test]
fn a_stopped_step_lets_the_run_go_on() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 60, "[forget]\nkeep-last = 1\n");
    // Created first, so that the forget always finds the repository, however soon the snapshot stops.
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(30, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let id = server.collection("Slow")["running"][0]["id"].clone();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}/steps/0")).code, 200);
    // Under the load of the whole suite, stopping rustic and running the forget can take a while; under 60
    // seconds, when the snapshot would end on its own.
    wait_for(45, "the run to end", || server.collection("Slow")["running"] == serde_json::json!([]));
    // The snapshot stopped; the forget, which does not need it, still ran (with nothing to forget), and
    // so did the run.
    let run = &sb.store().runs("Slow", 1)[0];
    let steps: Vec<_> = run.steps.iter().map(|s| serde_json::json!(s.status)).collect();
    assert_eq!(steps, ["cancelled", "succeeded"]);
    assert!(
        serde_json::json!(run.status) == "succeeded-with-warnings"
            && !run.all_results().any(|l| l.values["kind"] == "unmet")
    );
}

#[test]
fn cancelled_backup_waits_for_its_next_schedule() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 60, KEEP);
    sb.backup_profile("Fast", &[&["fast"]], &[], "");
    sb.write("fast/a.txt", "a");
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    let server = sb.server(&[]);
    wait_for(30, "the scheduled backup", || {
        server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow"
    });
    assert!(server.get("/collections/Slow").body.contains(r#"class="progress""#));
    // The running backup shows on the repository it writes to, and can be stopped.
    let repos = card(&server.get("/collections/Slow").body, "repos");
    assert!(repos.contains(r#"class="spinner""#), "{repos}");
    // Requested meanwhile: each request is its own run, and can be stopped while waiting. The prune waits
    // for the repository the backup writes to; the check waits only on a disk that may spin.
    wait_for(20, "the repository", || sb.path("local/Slow/config").is_file());
    assert!(
        server.post_form("/collections/Slow/repos/Slow/runs", "kind=check").body.contains(r#"class="flash success""#)
    );
    server.post_form("/collections/Slow/repos/Slow/runs", "kind=prune");
    let running = || server.collection("Slow")["running"].as_array().unwrap().clone();
    wait_for(20, "the prune to wait", || {
        running().iter().any(|r| r["kind"] == "prune" && r["activities"][0]["doing"]["kind"] == "waiting-for")
    });
    for run in running().iter().filter(|r| r["kind"] != "backup") {
        // On a solid-state disk the check does not wait, and may end before it is stopped.
        let code = server.delete(&format!("/collections/Slow/runs/{}", run["id"])).code;
        assert!(code == 200 || running().iter().all(|r| r["id"] != run["id"]), "{code} for {run}");
    }
    wait_for(10, "them to stop", || running().len() == 1);
    let stopped = Instant::now();
    assert_eq!(server.delete("/collections/Slow/runs").code, 200);
    wait_for(20, "the backup to stop", || server.collection("Slow")["running"] == serde_json::json!([]));
    assert!(stopped.elapsed() < Duration::from_secs(20), "rustic was not killed");
    let profile = server.collection("Slow");
    assert!(profile["snapshot"]["cancelled"] == true && profile["snapshot"]["last_ok"].is_null(), "{profile}");
    // Not a issue: the user requested it.
    assert!(!profile["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "failed"), "{profile}");
    let runs = sb.store().runs("Slow", usize::MAX);
    assert!(runs.iter().any(|r| json!(r.trigger) == "schedule" && r.status == Status::Cancelled), "{profile}");
    // What rustic started (the stdin command) is gone too: it would hold the backup's locks.
    wait_for(20, "Fast's backup", || server.collection("Fast")["snapshot"]["last_ok"].is_string());
    drop(server);

    // Not due again until the schedule fires after the cancellation.
    sb.run(&["due", "--collection", "Slow"]).assert_exit(0).assert_no_output("rustic backup");
    assert!(sb.snapshots("Slow").is_empty());
}

/// A server, and a command line backup of collection Slow that it sees running.
fn backing_up_elsewhere() -> (Sandbox, Server, std::process::Child) {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 3, "");
    let server = sb.server(&[]);
    let cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(20, "the run of the other process", || server.collection("Slow")["running"][0]["elsewhere"] == true);
    (sb, server, cli)
}

#[test]
fn a_command_line_run_is_followed_live() {
    let (_sb, server, mut cli) = backing_up_elsewhere();
    wait_for(10, "its log", || card(&server.get("/collections/Slow").body, "log").contains("[Slow] rustic backup"));
    assert!(card(&server.get("/collections/Slow").body, "runs").contains(r#"class="spinner""#));
    assert!(cli.wait().unwrap().success());
    assert_eq!(server.collection("Slow")["running"], serde_json::json!([]));
    let runs = card(&server.get("/collections/Slow").body, "runs");
    assert!(runs.contains(">cli<") && !runs.contains(r#"class="spinner""#), "{runs}");
}

#[test]
fn server_shows_a_running_backup_with_progress() {
    let sb = docs_with_remote("");
    // Data that rustic takes a while over: it neither compresses nor deduplicates.
    for i in 0..8 {
        fs::write(sb.path(&format!("source/docs/big{i}.bin")), noise(i, 12 << 20)).unwrap();
    }
    sb.common_env(&[("BACKUP_CRON", "0 0 * * *")]);
    let server = sb.server(&[]);
    // Seen together while it runs, as the backup may end at any time: its progress, the step that the recent
    // runs on the home page name, the collection page's progress bar, and its commands so far.
    wait_for(60, "progress", || {
        let profile = server.collection("Docs");
        assert!(profile["snapshot"]["last_ok"].is_null(), "the backup finished without progress showing up: {profile}");
        let seen = &profile["running"][0]["activities"][0];
        let (home, page) = (server.get("/").body, server.get("/collections/Docs").body);
        let (home, log) =
            (shown_card(&home, "recent-runs").unwrap_or_default(), shown_card(&page, "log").unwrap_or_default());
        let steps = sb.store().runs("Docs", 1).first().map(|r| r.steps.clone()).unwrap_or_default();
        seen["progress"]["percent_done"].is_number()
            && ["backing up", "copying"].iter().any(|verb| seen["on"][1] == *verb)
            && steps.iter().enumerate().any(|(i, s)| home.contains(&format!(">{}. {}<", i + 1, s.label)))
            && page.contains("class=\"progress\"")
            && log.contains("[Docs] rustic ")
            && log.contains(r#"<tr data-call="0""#)
    });
    // Once done, the activity is gone, and no rustic keeps running after the server stops.
    wait_for(60, "the backup to finish", || {
        let profile = server.collection("Docs");
        profile["running"] == serde_json::json!([]) && profile["targets"][0]["lag"]["checked_at"].is_string()
    });
}

/// A folder on another disk than `here`, if there is one: `/tmp` or `/var/tmp`, or the root of another fixed
/// drive on Windows.
fn other_disk(here: &std::path::Path) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let device = |p: &std::path::Path| fs::metadata(p).unwrap().dev();
        ["/tmp", "/var/tmp"].map(std::path::PathBuf::from).into_iter().find(|d| device(d) != device(here))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        // `GetDriveTypeW`'s answer for a hard disk or a solid-state drive.
        const DRIVE_FIXED: u32 = 3;
        let fixed = |root: &str| {
            let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
            // SAFETY: a NUL-terminated string.
            #[expect(unsafe_code, reason = "asks a drive's type")]
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            kind == DRIVE_FIXED
        };
        let here = here.components().next();
        let roots = (b'C'..=b'Z').map(|letter| format!("{}:\\", char::from(letter)));
        roots.filter(|root| fixed(root)).map(std::path::PathBuf::from).find(|root| root.components().next() != here)
    }
}

/// On another disk, a check does not wait for a backup.
#[test]
fn server_runs_jobs_independently() {
    let sb = Sandbox::new();
    let Some(elsewhere) = other_disk(&sb.path("")) else {
        eprintln!("skipped: no other disk to put a repository on");
        return;
    };
    let repo = elsewhere.join(NAME).join(format!("{}-fast", std::process::id()));
    _ = fs::remove_dir_all(&repo);
    sb.backup_profile_in("Fast", repo.to_str().unwrap(), &[&["fast"]], &[], &global_env(&[("CHECK_CRON", "@daily")]));
    sb.write("fast/a.txt", "a");
    sb.run(&["backup", "--collection", "Fast"]).assert_exit(0);
    // Slow's backup goes on until the test lets it end, however long Fast's check takes.
    let release = sb.path("release");
    stdin_profile(&sb, "Slow", &cmd::until_exists(&release), &global_env(&[("BACKUP_CRON", "@daily")]));
    sb.scheduled_long_ago(&["check:Fast"]);
    let server = sb.server(&[]);
    let slow_runs = || server.collection("Slow")["running"][0]["activities"][0].is_object();
    wait_for(30, "Slow's backup to start", slow_runs);
    // Fast's check does not wait for Slow's backup.
    wait_for(30, "Fast's check", || server.collection("Fast")["primary"]["check"]["last_ok"].is_string());
    assert!(slow_runs(), "Slow should still be running");
    fs::write(&release, "").unwrap();
    wait_for(60, "Slow's backup", || server.collection("Slow")["snapshot"]["last_ok"].is_string());
    fs::remove_dir_all(&repo).unwrap();
    _ = fs::remove_dir(repo.parent().unwrap());
}

/// Auto-update set to neither `true` nor `false` stops the server from starting, rather than guessing.
#[cfg(feature = "self-update")]
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn a_server_with_an_invalid_auto_update_does_not_start() {
    let sb = Sandbox::new();
    let (listen, update) = (var("LISTEN"), var("AUTO_UPDATE"));
    let mut server = sb.spawn(&["serve"], &[(listen.as_str(), "127.0.0.1:0"), (update.as_str(), "maybe")]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    let ended = server.try_wait().unwrap();
    _ = server.kill();
    assert_eq!(ended.and_then(|s| s.code()), Some(1));
}

/// With nothing due, the server is idle: reacting to state changes does not change the state.
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn an_idle_server_leaves_its_state_alone() {
    let sb = Sandbox::new();
    let notify = global_env(&[("NOTIFY_URLS", "json://127.0.0.1:1")]);
    sb.backup_profile("Docs", &[&["docs"]], &[], &notify);
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.common_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "@weekly")]);
    let server = sb.server(&[]);
    let db = sb.path("store/state.redb");
    let modified = || fs::metadata(&db).and_then(|m| m.modified()).unwrap();
    thread::sleep(Duration::from_secs(2));
    let before = modified();
    thread::sleep(Duration::from_secs(2));
    assert_eq!(modified(), before, "written while idle: {}", server.output());
}

/// A notification that cannot be sent is retried with backoff, not at every wake of the server, and shows on
/// its collection.
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn a_failed_notification_waits_before_retrying() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], &global_env(&[("NOTIFY_URLS", "json://127.0.0.1:1")]));
    // No sources: the backup fails, and is reported from its second failure.
    sb.run(&["backup", "--all"]);
    sb.run(&["backup", "--all"]);
    sb.common_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "@weekly")]);
    let server = sb.server(&[]);
    thread::sleep(Duration::from_secs(3));
    let attempts = server.output().matches(r#""kind":"undelivered""#).count();
    assert!(attempts <= 1, "{attempts} attempts: {}", server.output());
    assert!(json_has(&server.collection("Docs")["issues"].to_string(), &json!({"kind": "undelivered"})));
}

/// A check shares the repository with a backup, but not the disk: they run one at a time. The
/// sandbox's tmpfs is not reported as solid-state, so it counts as a disk that may spin. macOS reports
/// its disk as solid-state, where both run at once.
#[cfg(target_os = "linux")]
#[test]
fn a_check_waits_for_a_backup_on_the_same_disk() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 5, "");
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    server.post_form("/collections/Slow/repos/Slow/runs", "kind=check");
    let waiting = || server.collection("Slow")["running"].to_string().contains(r#""locks":["disk:"#);
    wait_for(30, "the check to wait", waiting);
    // The run that holds the disk shows the same name in both columns.
    let holds = || {
        server.collection("Slow")["running"].as_array().unwrap().iter().map(|r| r["holds"].clone()).collect::<Vec<_>>()
    };
    wait_for(10, "what the check waits for", || holds().iter().any(|h| h["waiting_for"].is_string()));
    let holds = holds();
    let disk = holds.iter().find_map(|h| h["waiting_for"].as_str()).unwrap().to_owned();
    assert!(
        disk.starts_with("disk:/") && holds.iter().any(|h| h["using"].as_array().unwrap().iter().any(|u| *u == disk)),
        "{holds:?}"
    );
    let runs = card(&server.get("/collections/Slow").body, "runs");
    assert!(runs.matches(&format!(">{disk}<")).count() == 2, "{runs}");
    // The waiting run links to the holding one.
    let backup =
        server.collection("Slow")["running"].as_array().unwrap().iter().find(|r| r["kind"] == "backup").unwrap()["id"]
            .clone();
    assert!(runs.contains(&format!(r#"<a class="tag" href="/collections/Slow/runs/{backup}#log""#)), "{runs}");
    wait_for(60, "the check and the backup", || sb.store().runs("Slow", 2).iter().all(|r| r.finished.is_some()));
    // After the backup wrote. Listing and measuring afterwards do not take the disk.
    let runs = sb.store().runs("Slow", 2);
    let (check, backup) = (&runs[0], &runs[1]);
    let call = |args: &str| backup.calls.iter().find(|c| c.args.ends_with(args)).unwrap();
    let wrote = call("backup --json");
    let written = wrote.started + Duration::from_secs_f64(wrote.secs);
    assert!(check.calls[0].started >= written, "the check read while the backup wrote");
}

#[test]
fn one_backup_at_a_time() {
    let sb = Sandbox::new();
    slow_profile(&sb, "A", 2, "");
    slow_profile(&sb, "B", 2, "");
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    let server: Server = sb.server(&[]);
    let mut waited = false;
    wait_for(60, "both backups", || {
        let (a, b) = (server.collection("A"), server.collection("B"));
        let activity = [&a["running"][0]["activities"][0], &b["running"][0]["activities"][0]];
        assert!(!activity.iter().all(|t| t["on"][1] == "backing up"), "two backups at once: {a} {b}");
        waited |= activity.iter().any(|t| t["doing"]["lock"] == "cpu" && t["doing"]["by"][0]["trigger"] == "schedule");
        a["snapshot"]["last_ok"].is_string() && b["snapshot"]["last_ok"].is_string()
    });
    assert!(waited, "one backup should have waited for the other");
}

/// A copy that is due while a command's backup holds its lock waits silently. A request would be
/// skipped, and its record would wake the scheduler again.
#[test]
fn a_due_copy_waits_for_a_backup_elsewhere() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 5, "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(10, "the command's backup", || {
        sb.store().unfinished(Some("Slow")).iter().any(|r| !r.holds().using.is_empty())
    });
    // A snapshot the target lacks: a copy is due.
    sb.store().apply(&[Event::Pending { collection: "Slow".into(), targets: vec!["Slow-remote".into()] }]).unwrap();
    let server = sb.server(&[]);
    thread::sleep(Duration::from_secs(2));
    let output = server.output();
    assert!(!server.logged(&json!({"event": {"kind": "running-already"}})), "{output}");
    assert!(!server.logged(&json!({"event": {"kind": "requested"}})), "{output}");
    _ = cli.wait();
}

/// What a command's run holds shows in the server, like the run itself.
#[test]
fn a_command_s_locks_show_in_the_server() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 10, "");
    let server = sb.server(&[]);
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let runs = || card(&server.get("/collections/Slow").body, "runs");
    wait_for(20, "its locks", || runs().contains(r#"<span class="tag">cpu</span>"#));
    cli.kill().unwrap();
    _ = cli.wait();
    wait_for(10, "them to go with it", || !runs().contains(r#"<span class="tag">cpu</span>"#));
}

/// A queued run says what it waits for, as its own request names it: a copy waits for the backup, which copies too.
#[test]
fn a_queued_copy_says_it_waits_for_the_backup() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 20, "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let queued = || {
        let store = sb.store();
        store.unfinished(Some("Slow")).iter().any(|r| r.status == Status::Queued && store.alive(r))
    };
    let mut copy = sb.spawn(&["copy", "--collection", "Slow"], &[]);
    // A busy machine takes a while to start the command.
    wait_for(30, "the copy to wait", queued);
    let runs = card(&server.get("/collections/Slow").body, "runs");
    copy.kill().unwrap();
    copy.wait().unwrap();
    assert!(runs.contains("(copy to every target, running)"), "{runs}");
}

/// A command waiting for the server's run shows there as queued, until it ends.
#[test]
fn a_waiting_command_shows_as_queued() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 20, "");
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let runs = || card(&server.get("/collections/Slow").body, "runs");
    // A killed command leaves its record, but no longer waits.
    let queued = || {
        let store = sb.store();
        store.unfinished(Some("Slow")).iter().any(|r| r.status == Status::Queued && store.alive(r))
    };
    wait_for(10, "the command to wait", queued);
    assert!(runs().contains(r#"<span class="tag">cli</span>"#), "{}", runs());
    assert!(runs().contains("(backup collection, running)"), "{}", runs());
    // Killed: its record stays, but nothing waits behind it any more.
    cli.kill().unwrap();
    let out = cli.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    let behind_web = json!({"event": {"kind": "waiting-for"}});
    assert!(
        json_has(&said, &behind_web) && json_has(&said, &json!({"trigger": "web", "locks": ["backup collection"]})),
        "{said}"
    );
    wait_for(10, "it to go", || !queued());
    // Cancelled from the web UI: it stops waiting at once, not when the backup ends.
    let cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let stop = || {
        let row = runs().split("<tr").find(|r| r.contains(r#"<span class="tag">cli</span>"#))?.to_owned();
        Some(row.split(r#"hx-delete=""#).nth(1)?.split('"').next()?.to_owned())
    };
    wait_for(10, "the command to wait", || stop().is_some());
    let cancelled = Instant::now();
    assert_eq!(server.delete(&stop().unwrap()).code, 200);
    let out = cli.wait_with_output().unwrap();
    assert!(cancelled.elapsed() < Duration::from_secs(3), "{:?}", cancelled.elapsed());
    assert!(json_has(&String::from_utf8_lossy(&out.stdout), &json!({"event": {"kind": "dequeued"}})), "{out:?}");
    assert_eq!(server.collection("Slow")["running"][0]["activities"][0]["on"][0], "Slow");
}

/// Requested while another process runs the collection: it waits, then runs.
#[test]
fn a_web_request_waits_for_a_command_line_run() {
    let (sb, server, mut cli) = backing_up_elsewhere();
    // The same request runs after it.
    assert_eq!(server.post_form("/collections/Slow/runs", "kind=backup").code, 201);
    assert!(cli.wait().unwrap().success());
    wait_for(30, "its own backup", || sb.snapshots("Slow").len() == 2);
    assert!(!server.logged(&json!({"event": {"kind": "running-already"}})), "{}", server.output());
}

#[test]
fn healthcheck_tells_whether_the_server_answers() {
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let addr = server.addr.clone();
    let healthcheck = || sb.run_with(&["healthcheck"], &[(&var("LISTEN"), &addr)]);
    healthcheck().assert_exit(0);
    drop(server);
    healthcheck().assert_exit(1).assert_result("error", json!({}));
}

/// The service manager that `install` reaches, which only records what it is asked in `log`, so that the
/// tests touch no real one. On macOS, a `launchctl` first on `PATH`. On Linux, a D-Bus of its own with
/// systemd and logind. None for root, whose install writes to the system's folders, which no sandbox holds.
#[cfg(unix)]
#[expect(clippy::disallowed_methods, reason = "asks the system who runs the tests")]
fn fake_service_manager(sb: &Sandbox, log: &std::path::Path) -> Option<Fake> {
    if std::process::Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout == b"0\n") {
        eprintln!("skipped: as root, install writes to the system's folders");
        return None;
    }
    let path = std::env::var("PATH").unwrap();
    #[cfg(target_os = "linux")]
    return Some(Fake { path, bus: crate::bus::FakeBus::start(sb, log, None) });
    #[cfg(not(target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = sb.path("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("launchctl"), format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display())).unwrap();
        fs::set_permissions(bin.join("launchctl"), fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([bin].into_iter().chain(std::env::split_paths(&path)));
        Some(Fake { path: path.unwrap().into_string().unwrap() })
    }
}

/// What `fake_service_manager` sets up, kept for as long as the test runs.
#[cfg(unix)]
struct Fake {
    path: String,
    #[cfg(target_os = "linux")]
    bus: crate::bus::FakeBus,
}

#[cfg(unix)]
impl Fake {
    /// The variables that send `install` to the fake.
    fn env(&self) -> Vec<(&str, &str)> {
        #[cfg(target_os = "linux")]
        let bus = self.bus.env();
        #[cfg(not(target_os = "linux"))]
        let bus: [(&str, &str); 0] = [];
        [("PATH", self.path.as_str())].into_iter().chain(bus).collect()
    }
}

/// `install --auto-update` turns auto-update on in the env file, in place of the line that set it and
/// keeping the user's other lines, however often it runs, and creates the file readable by its owner only;
/// `install` alone leaves the file as it is.
#[cfg(all(unix, feature = "self-update"))]
#[test]
fn install_turns_auto_update_on_in_the_env_file() {
    use std::os::unix::fs::PermissionsExt;
    let sb = docs_with_remote("");
    let Some(fake) = fake_service_manager(&sb, &sb.path("manager.log")) else { return };
    let config = if cfg!(target_vendor = "apple") { "home/Library/Application Support" } else { "config" };
    let file = sb.path(&format!("{config}/{NAME}/{NAME}.env"));
    let install = |args: &[&str]| {
        sb.run_with(args, &fake.env()).assert_exit(0);
    };
    install(&["install", "--auto-update"]);
    assert_eq!(fs::read_to_string(&file).unwrap(), "RUSTICLONE_AUTO_UPDATE=true\n");
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    let user = "# mine\nRUSTICLONE_STALL_TIMEOUT=2h\nRUSTICLONE_AUTO_UPDATE=false\n";
    fs::write(&file, user).unwrap();
    install(&["install", "--auto-update"]);
    install(&["install", "--auto-update"]);
    let written = fs::read_to_string(&file).unwrap();
    assert_eq!(written, user.replace("=false", "=true"));
    install(&["install"]);
    assert_eq!(fs::read_to_string(&file).unwrap(), written);
    install(&["uninstall"]);
}

/// `install` puts its folder on `PATH` for new terminals: in `.profile`, which it creates, and in the other startup
/// files of the shell that exist, once however often it runs.
#[cfg(unix)]
#[test]
fn install_puts_its_folder_on_path_for_new_terminals() {
    let sb = docs_with_remote("");
    let Some(fake) = fake_service_manager(&sb, &sb.path("manager.log")) else { return };
    let home = sb.path("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(".bashrc"), "# mine").unwrap();
    for _ in 0..2 {
        sb.run_with(&["install"], &fake.env()).assert_exit(0);
    }
    let line = |file: &str| fs::read_to_string(home.join(file)).unwrap().matches("env\"\n").count();
    assert_eq!((line(".profile"), line(".bashrc")), (1, 1), "once each");
    assert!(fs::read_to_string(home.join(".bashrc")).unwrap().starts_with("# mine\n"), "below the user's own lines");
    assert!(!home.join(".bash_profile").exists(), "not created");
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
}

/// What `install` left in `dir`: files named after rusticlone.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn leftovers(dir: &std::path::Path) -> Vec<String> {
    let entries = fs::read_dir(dir).into_iter().flatten().flatten();
    entries.map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains(NAME)).collect()
}

/// Under systemd, the server says when it is ready and when it stops.
#[cfg(target_os = "linux")]
#[test]
fn the_server_tells_systemd_when_it_is_ready_and_stops() {
    use std::os::unix::net::UnixDatagram;
    fn said(socket: &UnixDatagram) -> String {
        let mut said = [0; 256];
        let n = socket.recv(&mut said).unwrap();
        String::from_utf8_lossy(&said[..n]).into_owned()
    }
    let sb = docs_with_remote("");
    // A socket's path is short: the sandbox's may be too long.
    let path = std::env::temp_dir().join(format!("{NAME}-{}.sock", std::process::id()));
    let socket = UnixDatagram::bind(&path).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let server = sb.server(&[("NOTIFY_SOCKET", path.to_str().unwrap())]);
    assert_eq!(said(&socket), "READY=1\n");
    drop(server);
    assert_eq!(said(&socket), "STOPPING=1\n");
    fs::remove_file(&path).unwrap();
}

/// `install` writes the unit and a drop-in that runs this executable with this `PATH`, enables it
/// and restarts it. `uninstall` disables it and leaves nothing of it, and has nothing to do the second time.
#[cfg(target_os = "linux")]
#[test]
fn install_enables_a_user_service() {
    #[expect(unsafe_code, reason = "geteuid has no preconditions")]
    // SAFETY: no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return; // As root, it would write in /etc/systemd/system.
    }
    let sb = docs_with_remote("");
    let log = sb.path("systemd.log");
    let Some(fake) = fake_service_manager(&sb, &log) else { return };
    sb.run_with(&["install"], &fake.env()).assert_exit(0);
    let dir = sb.path("config/systemd/user");
    let (unit, drop_in) =
        (dir.join(format!("{NAME}.service")), dir.join(format!("{NAME}.service.d")).join("install.conf"));
    // The packaged unit, below the line that says what generates it.
    let packaged = include_str!("../../packaging/linux/rusticlone.service").split_once('\n').map(|(_, unit)| unit);
    assert_eq!(Some(fs::read_to_string(&unit).unwrap().as_str()), packaged);
    let conf = fs::read_to_string(&drop_in).unwrap();
    assert!(conf.contains(&format!("{NAME}\" serve\n")) && conf.contains(&format!("PATH={}\"", fake.path)), "{conf}");
    let calls = fs::read_to_string(&log).unwrap();
    let started = format!("Reload\nEnableUnitFiles {NAME}.service\nRestartUnit {NAME}.service\n");
    assert!(calls.ends_with(&started), "{calls}");
    // The desktop's entry and its icon, which open the web UI.
    let data = sb.path("home/.local/share");
    let desktop = [data.join("applications"), data.join("icons").join("hicolor").join("scalable").join("apps")];
    assert!(desktop.iter().all(|dir| leftovers(dir).len() == 1), "{desktop:?}");
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
    assert_eq!(leftovers(&dir), Vec::<String>::new());
    assert!(desktop.iter().all(|dir| leftovers(dir).is_empty()), "{desktop:?}");
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains(&format!("DisableUnitFiles {NAME}.service\nStopUnit {NAME}.service\n"))
    );
    // Again: nothing to undo.
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0).assert_result("not-installed", json!({}));
}

/// With a unit from a package, which systemd knows, `install` writes only the drop-in. `uninstall` takes the
/// drop-in back and leaves the unit to the package.
#[cfg(target_os = "linux")]
#[test]
fn install_keeps_a_packaged_unit() {
    #[expect(unsafe_code, reason = "geteuid has no preconditions")]
    // SAFETY: no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return; // As root, it would write in /etc/systemd/system.
    }
    let sb = docs_with_remote("");
    let log = sb.path("systemd.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, Some("enabled"));
    sb.run_with(&["install"], &bus.env()).assert_exit(0);
    let dir = sb.path("config/systemd/user");
    assert_eq!(leftovers(&dir), [format!("{NAME}.service.d")]);
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.ends_with(&format!("EnableUnitFiles {NAME}.service\nRestartUnit {NAME}.service\n")), "{calls}");
    sb.run_with(&["uninstall"], &bus.env()).assert_exit(0);
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}

/// `install` writes a login agent that runs the app it writes, signed, and loads it; `uninstall` unloads it
/// and leaves nothing of either.
#[cfg(target_vendor = "apple")]
#[test]
fn install_loads_a_login_agent() {
    use rusticlone::APP_NAME;
    let sb = docs_with_remote("");
    let log = sb.path("launchctl.log");
    let Some(fake) = fake_service_manager(&sb, &log) else { return };
    sb.run_with(&["install"], &fake.env()).assert_exit(0);
    let (dir, apps) = (sb.path("home/Library/LaunchAgents"), sb.path("home/Applications"));
    let plist = fs::read_to_string(dir.join(format!("{NAME}.plist"))).unwrap();
    let exe = apps.join(format!("{APP_NAME}.app")).join("Contents").join("MacOS").join(APP_NAME);
    assert!(plist.contains("<string>serve</string>") && plist.contains(&fake.path), "{plist}");
    assert!(plist.contains(&format!("<string>{}</string>", exe.display())), "{plist}");
    let app = exe.ancestors().nth(3).unwrap();
    let status = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).arg(app).output();
    assert!(status("codesign", &["--verify"]).unwrap().status.success());
    assert!(status("xattr", &["-p", "com.apple.metadata:kMDItemKeywords"]).unwrap().status.success());
    assert!(fs::read_to_string(&log).unwrap().contains("bootstrap gui/"));
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
    assert_eq!(leftovers(&dir), Vec::<String>::new());
    assert_eq!(leftovers(&apps), Vec::<String>::new());
    assert!(fs::read_to_string(&log).unwrap().contains("bootout gui/"));
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0).assert_result("not-installed", json!({}));
}

/// Run from a folder that is not on `PATH`, such as Downloads, `install` copies itself where the installer
/// script puts it, and puts that folder on `PATH` in the shells' startup files, once however often it runs.
#[cfg(unix)]
#[test]
fn install_copies_itself_onto_path() {
    let sb = docs_with_remote("");
    let Some(fake) = fake_service_manager(&sb, &sb.path("manager.log")) else { return };
    // An empty `CARGO_HOME` is unset, as for the installer script.
    let env = [fake.env(), vec![("SHELL", "/bin/zsh"), ("CARGO_HOME", "")]].concat();
    let install = || sb.run_with(&["install"], &env);
    install().assert_exit(0).assert_result("copied", json!({})).assert_result("on-path", json!({}));
    // The copy it finds may be an older version: replaced.
    install().assert_exit(0).assert_result("copied", json!({}));
    assert!(sb.path("home/.cargo/bin").join(NAME).is_file());
    let source = format!(". \"{}\"", sb.path("home/.cargo/env").display());
    for rc in [".profile", ".zshenv"] {
        assert_eq!(fs::read_to_string(sb.path("home").join(rc)).unwrap().matches(&source).count(), 1, "{rc}");
    }
    // Created, a startup file holds that line alone.
    assert_eq!(fs::read_to_string(sb.path("home/.profile")).unwrap(), format!("{source}\n"));
    let sourced = std::process::Command::new("sh").arg("-c").arg(format!("{source}; echo $PATH")).output().unwrap();
    assert!(String::from_utf8_lossy(&sourced.stdout).starts_with(&sb.path("home/.cargo/bin").display().to_string()));
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
}

/// `uninstall --delete` also deletes the state directory, never the profiles nor the repositories.
#[cfg(unix)]
#[test]
fn uninstall_deletes_the_state() {
    let sb = docs_with_remote("");
    let Some(fake) = fake_service_manager(&sb, &sb.path("manager.log")) else { return };
    sb.run(&["backup", "--all"]).assert_exit(0);
    assert!(sb.path("store/state.redb").is_file());
    sb.run_with(&["uninstall", "--delete"], &fake.env()).assert_exit(0).assert_result("deleted", json!({}));
    assert!(!sb.path("store").exists());
    assert!(sb.profile_path("Docs").is_file() && sb.path("local/Docs/config").is_file());
}

/// A run of the server is stopped by its id, which says so. Stopping nothing says nothing.
#[test]
fn a_run_is_stopped_by_its_id() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 30, "");
    let server = sb.server(&[]);
    let cancelled = json!({"event": {"kind": "cancelled-from-web"}});
    assert_eq!(server.delete("/collections/Slow/runs").code, 200);
    assert!(!server.logged(&cancelled), "nothing was stopped: {}", server.output());
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let id = server.collection("Slow")["running"][0]["id"].clone();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}")).code, 200);
    wait_for(20, "it to stop", || server.collection("Slow")["running"] == json!([]));
    assert!(server.logged(&cancelled), "{}", server.output());
}

/// Of the commands waiting behind the server's backup, only the one stopped stops. The record of one whose process
/// is gone cannot be stopped: nothing waits there.
#[test]
fn only_the_waiting_command_stopped_stops() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 30, "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let (backup, mut copy) =
        (sb.spawn(&["backup", "--collection", "Slow"], &[]), sb.spawn(&["copy", "--collection", "Slow"], &[]));
    let queued = |kind: &str| {
        let store = sb.store();
        let waiting =
            store.unfinished(Some("Slow")).into_iter().filter(|r| r.status == Status::Queued && store.alive(r));
        waiting.filter(|r| serde_json::json!(r.kind) == kind).map(|r| r.id).next()
    };
    wait_for(10, "both to wait", || queued("backup").is_some() && queued("copy").is_some());
    let id = queued("backup").unwrap();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}")).code, 200);
    assert!(json_has(
        &String::from_utf8_lossy(&backup.wait_with_output().unwrap().stdout),
        &json!({"event": {"kind": "dequeued"}})
    ));
    let left = queued("copy").expect("the copy still waits");
    copy.kill().unwrap();
    _ = copy.wait();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{left}")).code, 422, "its process is gone");
}

/// The profiles' folder opens in the file manager, for a browser on this computer. Without a file manager on the
/// desktop, nor `xdg-open`, the page says that it could not.
#[cfg(target_os = "linux")]
#[test]
fn the_profile_folder_opens_in_the_file_manager() {
    let sb = docs_with_remote("");
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    let open = |server: &Server| {
        let form = "Content-Type: application/x-www-form-urlencoded";
        let login = server.request("POST", "/login", &[form], &format!("token={}", server.token));
        let session = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
        let cookie = format!("Cookie: {}", session.split(';').next().unwrap());
        server.request("POST", "/setup/folder", &[form, &cookie], "").code
    };
    assert_eq!(open(&sb.server(&bus.env())), 204);
    let folder = format!("ShowFolders file://{}", sb.profile_dir().display());
    assert!(fs::read_to_string(&log).unwrap().contains(&folder), "{}", fs::read_to_string(&log).unwrap());
    drop(bus);
    let path = std::env::var_os("PATH").unwrap();
    let without = std::env::split_paths(&path).filter(|d| !d.join("xdg-open").exists());
    let without = std::env::join_paths(without).unwrap().into_string().unwrap();
    assert_eq!(open(&sb.server(&[("PATH", &without)])), 422);
}

/// `serve --open` beside a server that runs opens that one in the browser, logged in, and starts no other.
#[cfg(target_os = "linux")]
#[test]
fn serve_open_opens_the_running_server() {
    let sb = docs_with_remote("");
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    let server = sb.server(&[]);
    let listen = var("LISTEN");
    let env = [bus.env().as_slice(), &[(listen.as_str(), server.addr.as_str())]].concat();
    sb.run_with(&["serve", "--open"], &env).assert_exit(0);
    let opened = fs::read_to_string(&log).unwrap();
    assert!(opened.contains(&format!("OpenURI http://{}/", server.addr)), "{opened}");
}

/// A first `install`, without profiles, opens the web UI, where the first collection is set up. With profiles, it
/// opens nothing.
#[cfg(target_os = "linux")]
#[test]
fn a_first_install_opens_the_web_ui() {
    let sb = Sandbox::new();
    let log = sb.path("manager.log");
    let Some(fake) = fake_service_manager(&sb, &log) else { return };
    let server = sb.server(&[]);
    let listen = var("LISTEN");
    let env = [fake.env(), vec![(listen.as_str(), server.addr.as_str())]].concat();
    sb.run_with(&["install"], &env).assert_exit(0);
    let opens = || fs::read_to_string(&log).unwrap().matches("OpenURI ").count();
    assert_eq!(opens(), 1, "{}", fs::read_to_string(&log).unwrap());
    sb.backup_profile("Docs", &[&["docs"]], &[], "");
    sb.run_with(&["install"], &env).assert_exit(0);
    assert_eq!(opens(), 1, "{}", fs::read_to_string(&log).unwrap());
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
}

/// A desktop notification shows the app's own icon, written where the notification server reads it.
#[cfg(target_os = "linux")]
#[test]
fn a_desktop_notification_shows_the_app_icon() {
    let sb = Sandbox::new();
    let log = sb.path("desktop.log");
    let bus = crate::bus::FakeBus::start(&sb, &log, None);
    sb.backup_profile("Docs", &[&["docs"]], &[], &global_env(&[("NOTIFY_URLS", "dbus://")]));
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run_with(&["notify", "--all"], &bus.env()).assert_exit(0);
    let notified = fs::read_to_string(&log).unwrap();
    let icon = notified.lines().find_map(|l| l.strip_prefix("Notify ")?.rsplit(' ').next()).unwrap_or_default();
    // The app's PNG, not the bundled SVG that mercurio falls back to.
    let icon = std::path::Path::new(icon);
    let named = icon.file_name().is_some_and(|n| n.to_string_lossy().starts_with("icon-"));
    assert!(named && icon.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")), "{notified}");
}
