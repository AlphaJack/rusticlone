//! How pages are served: assets, languages, the proxy's path, metrics and refused requests.

use super::*;

#[test]
fn schedules_are_marked() {
    let sb = Sandbox::new();
    let env = &global_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "0 3 * * *")]);
    sb.backup_profile("Daily", &[&["a"]], &[], env);
    sb.backup_profile("Manual", &[&["b"]], &[], "");
    let server = sb.server(&[]);
    let overview = server.get("/").body;
    // Lit with its expression, or not.
    let schedules: Vec<&str> = overview.split(r#"<span class="schedule"#).skip(1).collect();
    assert!(schedules.iter().any(|s| s.starts_with(" on") && s.contains("@daily")), "{overview}");
    assert!(schedules.iter().any(|s| s.starts_with(r#"" title="#)), "{overview}");
    let repos = card(&server.get("/collections/Daily").body, "repos");
    let schedules: Vec<&str> = repos.split(r#"<span class="schedule"#).skip(1).collect();
    assert!(schedules.iter().any(|s| s.starts_with(" on") && s.contains("0 3 * * *")), "{repos}");
    assert!(schedules.iter().any(|s| s.starts_with(r#"" title="#)), "{repos}");
    let (daily, manual) = (server.collection("Daily"), server.collection("Manual"));
    assert!(daily["backup_schedule"]["cron"] == "@daily" && daily["backup_schedule"]["next"].is_string(), "{daily}");
    assert!(manual["backup_schedule"].is_null(), "{manual}");
}

#[test]
fn actions_from_other_sites_are_refused() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let auth = format!("Authorization: Bearer {}", server.token);
    // A proxy adds the token to every browser request, including cross-site ones.
    let other_site = server.request("POST", "/collections/Docs/runs", &[&auth, "Sec-Fetch-Site: cross-site"], "");
    assert_eq!(other_site.code, 403);
    let old_browser = server.request("POST", "/collections/Docs/runs", &[&auth, "Origin: https://evil.example"], "");
    assert_eq!(old_browser.code, 403);
    // Scripts send neither header.
    assert_eq!(server.request("POST", "/collections/Docs/repos/Docs/refresh", &[&auth], "").code, 200);
    // Monitors need no login, and learn only that the server runs.
    let health = server.request("GET", "/health", &[], "");
    assert!(health.code == 200 && health.body == "ok\n", "{}", health.body);
    let head = health.head.to_lowercase();
    assert!(head.contains("content-security-policy: default-src 'self';") && !head.contains("unsafe-inline"), "{head}");
}

#[test]
fn assets_are_cached_for_good() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let at = page.find("/assets/htmx.min.js?v=").unwrap();
    let asset = page.get(at..).unwrap().split('"').next().unwrap();
    assert!(server.get(asset).head.to_lowercase().contains("immutable"));
}

/// A page is in the language its visitor accepts, else English, whatever the server's own: it says which, and
/// that it depends on the header.
#[test]
fn pages_are_in_the_accepted_language() {
    let sb = docs_with_remote("");
    let server = sb.server(&[("LANG", "de_DE.UTF-8")]);
    for accepted in ["fr-FR,fr;q=0.9", "en-GB", "*"] {
        let page = server.get_with("/", &[&format!("Accept-Language: {accepted}")]);
        let head = page.head.to_lowercase();
        assert!(head.contains("content-language: en") && head.contains("vary: accept-language"), "{head}");
        assert!(page.body.contains(r#"<html lang="en">"#), "{accepted}");
    }
}

/// Behind a reverse proxy, every link starts where it serves the web UI. A page's live updates know what it shows
/// already, by its hash, so that an unchanged page is not sent again.
#[test]
fn pages_link_below_the_proxy_and_know_what_they_show() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let page = server.get_with("/", &["X-Forwarded-Prefix: /backups"]).body;
    let start = "hx-sse:connect=\"/backups/events?v=";
    let (_, live) = page.split_once(start).unwrap_or_else(|| panic!("{page}"));
    let hash = live.split("&amp;h=").nth(1).and_then(|h| h.split(['&', '"']).next()).unwrap();
    assert!(hash.parse::<u64>().is_ok_and(|h| h != 0), "{live}");
}

/// The page's stylesheet is served as one. The setup form shows the free space where a folder would go. Each kind of
/// issue a collection has counts once in the metrics.
#[test]
fn assets_free_space_and_issues_are_served() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("source/docs");
    sb.run(&["backup", "--all"]).assert_exit(1);
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let (_, css) = page.split_once(r#"<link rel="stylesheet" href=""#).unwrap();
    let css = server.get(css.split('"').next().unwrap());
    assert!(
        css.head.to_lowercase().contains("content-type: text/css") && css.body.contains("light-dark("),
        "{}",
        css.head
    );
    let free = server.get(&format!("/setup/free?folder={}", sb.path("local").display())).body;
    assert!(free.contains(" free</span>"), "{free}");
    let metrics = server.get("/metrics").body;
    let issues: Vec<&str> = metrics.lines().filter(|l| l.starts_with("rusticlone_issues{")).collect();
    assert!(!issues.is_empty() && issues.iter().all(|l| l.ends_with(" 1")), "{metrics}");
}

/// Monitors read `/metrics`: a renamed metric or label breaks their queries.
#[test]
fn metrics_keep_their_names() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    assert_eq!(server.request("GET", "/metrics", &[], "").code, 401);
    let filters = [
        (r"(_seconds(?:\{[^}]*\})?) [\d.]+", "$1 [time]"),
        (r"(_bytes\{[^}]*\}) \d+", "$1 [bytes]"),
        (r#"version="[^"]+""#, r#"version="[version]""#),
    ];
    insta::with_settings!({filters => filters}, {
        insta::assert_snapshot!(server.get("/metrics").body);
    });
}
