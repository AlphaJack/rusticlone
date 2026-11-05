//! `install` and `uninstall`: the service, `PATH`, the env file and the state.

use super::*;

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
    // Longer than the line install adds, which it looks for in it, and without a newline at its end.
    let mine = format!("# mine{}", " and more".repeat(30));
    fs::write(home.join(".bashrc"), &mine).unwrap();
    for _ in 0..2 {
        sb.run_with(&["install"], &fake.env()).assert_exit(0);
    }
    let line = |file: &str| fs::read_to_string(home.join(file)).unwrap().matches("env\"\n").count();
    assert_eq!((line(".profile"), line(".bashrc")), (1, 1), "once each");
    assert!(
        fs::read_to_string(home.join(".bashrc")).unwrap().starts_with(&format!("{mine}\n")),
        "below the user's own lines"
    );
    assert!(!home.join(".bash_profile").exists(), "not created");
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
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
    let packaged = include_str!("../../../packaging/linux/rusticlone.service").split_once('\n').map(|(_, unit)| unit);
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

/// A first `install`, without profiles, opens the web UI, where the first collection is set up. With profiles, it
/// opens nothing.
#[cfg(target_os = "linux")]
#[test]
fn a_first_install_opens_the_web_ui() {
    let sb = Sandbox::new();
    let log = sb.path("manager.log");
    let Some(fake) = fake_service_manager(&sb, &log) else { return };
    // Not even the profile that the others share.
    fs::remove_file(sb.profile_dir().join("common.toml")).unwrap();
    let server = sb.server(&[]);
    let listen = var("LISTEN");
    let env = [fake.env(), vec![(listen.as_str(), server.addr.as_str())]].concat();
    sb.run_with(&["install"], &env).assert_exit(0);
    let opens = || fs::read_to_string(&log).unwrap().matches("OpenURI ").count();
    assert_eq!(opens(), 1, "{}", fs::read_to_string(&log).unwrap());
    sb.common_env(&[]);
    sb.backup_profile("Docs", &[&["docs"]], &[], "");
    sb.run_with(&["install"], &env).assert_exit(0);
    assert_eq!(opens(), 1, "{}", fs::read_to_string(&log).unwrap());
    sb.run_with(&["uninstall"], &fake.env()).assert_exit(0);
}
