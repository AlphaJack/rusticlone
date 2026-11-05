//! The web UI's tokens, logins and health check.

use super::*;

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
fn healthcheck_tells_whether_the_server_answers() {
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let addr = server.addr.clone();
    let healthcheck = || sb.run_with(&["healthcheck"], &[(&var("LISTEN"), &addr)]);
    healthcheck().assert_exit(0);
    drop(server);
    healthcheck().assert_exit(1).assert_result("error", json!({}));
}
