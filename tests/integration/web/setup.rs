//! A first collection set up from the web UI, and its profiles' folder.

use super::*;

/// Only a browser on this computer, logged in, gets a button that opens the saved profiles' folder: for a
/// script or a reverse proxy, it would open on the server's desktop.
#[test]
fn the_profile_folder_opens_for_this_computer_only() {
    if as_root() {
        return;
    }
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let form = "Content-Type: application/x-www-form-urlencoded";
    let login = server.request("POST", "/login", &[form], &format!("token={}", server.token));
    let session = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
    let cookie = format!("Cookie: {}", session.split(';').next().unwrap());
    let (docs, repo) = (sb.path("docs"), sb.path("repos"));
    let setup = |name: &str| {
        let repo = repo.join(name);
        format!(
            "name={name}&source.0={}&primary=local&primary.local.repository={}&primary.password=pw&target=none&every=manual&check=manual&keep=all&notify=",
            docs.display(),
            repo.display()
        )
    };
    let button = r#"hx-post="/setup/folder""#;
    let here = server.request("POST", "/setup", &[form, &cookie], &setup("Here"));
    assert!(here.body.contains(button), "{}", here.body);
    let proxied = server.request("POST", "/setup", &[form, &cookie, "X-Forwarded-For: 192.0.2.1"], &setup("Proxied"));
    assert!(proxied.code == 200 && !proxied.body.contains(button), "{}", proxied.body);
    let scripted = server.post_form("/setup", &setup("Scripted"));
    assert!(scripted.code == 200 && !scripted.body.contains(button), "{}", scripted.body);
    // Elsewhere, the folder's path is there to copy.
    let dir = sb.profile_path("x").parent().unwrap().display().to_string();
    assert!(scripted.body.contains(&format!(r#"data-copy="{dir}""#)), "{}", scripted.body);
    // Asked anyway, it opens nothing.
    assert_eq!(server.post_form("/setup/folder", "").code, 404);
    // Nor a repository's profile or a source.
    assert_eq!(server.post_form("/collections/Here/folder", "repo=Here").code, 404);
    assert_eq!(server.post_form("/collections/Here/folder", "source=0/0").code, 404);
}

/// A profile file in the way, even a link that leads nowhere, is never written over, and the form says so. A name
/// too long for the system fails as the system says.
#[cfg(unix)]
#[test]
fn setup_never_writes_over_a_profile_in_the_way() {
    if as_root() {
        return;
    }
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let (docs, repo) = (sb.path("docs"), sb.path("repos").join("docs"));
    let setup = |name: &str| {
        format!(
            "name={name}&source.0={}&primary=local&primary.local.repository={}&primary.password=pw&target=none&every=manual&check=manual&keep=all&notify=",
            docs.display(),
            repo.display()
        )
    };
    let link = sb.profile_path("Docs");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(sb.path("nowhere"), &link).unwrap();
    let refused = server.post_form("/setup", &setup("Docs")).body;
    assert!(flash(&refused, "failure") && refused.contains("already exists"), "{refused}");
    let long = server.post_form("/setup", &setup(&"D".repeat(300))).body;
    assert!(flash(&long, "failure") && !long.contains("already exists"), "{long}");
}

#[test]
fn a_first_user_is_shown_where_to_start() {
    if as_root() {
        return;
    }
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let dir = sb.profile_path("x").parent().unwrap().display().to_string();
    assert!(page.contains(&dir) && page.contains("/setup"), "{page}");
    // As JSON writes it, with Windows's backslashes doubled.
    let listed = json!(dir).to_string();
    sb.run(&["backup", "--all"])
        .assert_exit(2)
        .assert_result("no-profiles", json!({}))
        .assert_output(listed.trim_matches('"'));
    // A generated password, and a field for each value of each kind of repository.
    let form = server.get("/setup").body;
    assert!(
        form.contains(r#"name="primary.password" id="setup-primary-password" value=""#)
            && form.contains(r#"name="primary.s3.bucket""#)
            && form.contains(r#"name="target.s3.bucket""#),
        "{form}"
    );
    let (docs, local, usb) = (sb.path("docs"), sb.path("repos/local"), sb.path("repos/usb"));
    let setup = format!(
        "name=Docs&source.0={}&primary=local&primary.local.repository={}&primary.password=pw&target=local&target.local.repository={}&target.password=pw&every=daily&notify=",
        docs.display(),
        local.display(),
        usb.display()
    );
    let saved = server.post_form("/setup", &setup);
    assert!(saved.body.contains("restore --all") && !saved.body.contains("password = "), "{}", saved.body);
    assert!(sb.profile_path("Docs-local").is_file() && sb.profile_path("Docs-local-copy").is_file());
    // Listed once read, even if its first backup fails.
    wait_for(15, "the listing", || {
        let page = server.get("/").body;
        page.contains("/collections/Docs-local") && sb.store().inventory("Docs-local", "Docs-local").is_some()
    });
    assert!(flash(&server.post_form("/setup", &setup).body, "failure"));
}
