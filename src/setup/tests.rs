use super::{Choice, Error, Every, Keep, Kind, Role, Setup};
use crate::{profile, settings};

/// The default answers, with a target of `kind` and its values.
fn target(kind: Kind, values: &[(&str, &str)]) -> Setup {
    let mut setup = Setup { password: "pw".into(), target_password: "pw".into(), target: kind, ..Setup::default() };
    setup.values.extend(values.iter().map(|(k, v)| (format!("target.{}.{k}", kind.id()), (*v).to_owned())));
    setup
}

/// rustic would read a relative folder from wherever it runs, and back up nothing there without failing.
/// Every field and choice of every kind has its message.
#[test]
fn every_field_has_text() {
    for field in <Kind as strum::IntoEnumIterator>::iter().flat_map(Kind::fields) {
        assert!(!super::Field::text_of(field.label).is_empty(), "no message field-{}", field.label);
        for choice in field.choices {
            assert!(crate::i18n::find(&format!("field-choice-{choice}"), []).is_some(), "no message for {choice}");
        }
    }
}

#[test]
fn folders_are_full_paths_with_tilde_as_the_home_folder() {
    let home = etcetera::home_dir().unwrap().display().to_string();
    let mut setup = target(Kind::Local, &[("repository", "~/usb")]);
    setup.values.insert("source.0".into(), "~/Documents".into());
    assert_eq!(setup.sources(), [format!("{home}/Documents")]);
    assert!(setup.profiles()[1].1.contains(&format!("{home}/usb")), "{}", setup.profiles()[1].1);
    assert!(setup.check().is_ok());
    let prefixed = target(Kind::Local, &[("repository", "local:~/usb")]);
    let saved = profile::parse("Documents-local-copy", &prefixed.profiles()[1].1).repository;
    assert_eq!(saved, Some(format!("{home}/usb")));
    let with = |key: &str, path: &str| {
        let mut setup = setup.clone();
        setup.values.insert(key.into(), path.into());
        setup.check()
    };
    assert!(matches!(with("source.1", "Documents"), Err(Error::Relative { path }) if path == "Documents"));
    assert!(matches!(with("source.1", "~me/x"), Err(Error::Relative { .. })));
    assert!(matches!(with("target.local.repository", "backup"), Err(Error::Relative { .. })));
}

/// rustic backs up a repository inside its sources into itself. It mixes a new one in with other files.
#[cfg(feature = "server")]
#[test]
#[expect(clippy::disallowed_methods, reason = "lays out folders to back up into")]
fn a_backup_folder_is_neither_nested_nor_shared() {
    let dir = crate::sys::test_dir("folders");
    let path = |parts: &[&str]| parts.iter().fold(dir.clone(), |p, part| p.join(part)).display().to_string();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("full")).unwrap();
    std::fs::write(dir.join("full").join("notes.txt"), "mine").unwrap();
    std::fs::create_dir_all(dir.join("repo")).unwrap();
    std::fs::write(dir.join("repo").join("config"), "").unwrap();
    let into = |folder: &str| {
        let mut setup = target(Kind::None, &[]);
        setup.values.insert("source.0".into(), path(&["docs"]));
        setup.values.insert("primary.local.repository".into(), folder.to_owned());
        setup.check_folders()
    };
    assert!(matches!(into(&path(&["docs", "backup"])), Err(Error::Inside)));
    assert!(matches!(into(&dir.display().to_string()), Err(Error::Inside)), "it would hold the source");
    assert!(matches!(into(&path(&["full"])), Err(Error::NotEmpty { .. })));
    assert!(into(&path(&["repo"])).is_ok(), "an existing repository");
    assert!(into(&path(&["new", "backup"])).is_ok());
    assert!(matches!(into(&format!("local:{}", path(&["docs", "backup"]))), Err(Error::Inside)), "as rustic reads it");
    // On a disk that ignores case, a new folder written in another case is still in the source.
    if dir.join("DOCS").exists() {
        assert!(matches!(into(&path(&["DOCS", "new", "backup"])), Err(Error::Inside)));
    }
}

/// Each folder is its own entry, with its own snapshots. One given twice is backed up once.
#[test]
fn each_folder_is_its_own_entry() {
    let mut setup = target(Kind::None, &[]);
    setup.values.insert("source.0".into(), "/home/me/Documents".into());
    setup.values.insert("source.1".into(), "/home/me/Desktop".into());
    setup.values.insert("source.2".into(), "/home/me/Documents".into());
    let text = &setup.profiles()[0].1;
    let entries: Vec<Vec<String>> = profile::parse("Documents", text).sources.into_iter().map(|s| s.sources).collect();
    assert_eq!(entries, [["/home/me/Documents"], ["/home/me/Desktop"]], "{text}");
    assert_eq!(text.matches("# What do you want to back up?").count(), 1, "{text}");
    assert!(text.find("# What do you want to back up?") < text.find("/home/me/Documents"), "on the first: {text}");
}

/// Each schedule says which question it answers. Required options are written even when blank, so that the
/// example shows what to fill in, and optional ones only when filled. Notifications are set only when asked for.
#[test]
fn profiles_say_what_they_answer_and_what_to_fill_in() {
    let blank = target(Kind::Sftp, &[("endpoint", "ssh://nas:22"), ("key", "/home/me/.ssh/id")]);
    let (primary, target_text) = (&blank.profiles()[0].1, &blank.profiles()[1].1);
    let lines: Vec<&str> = primary.lines().collect();
    let at = lines.iter().position(|l| l.starts_with(settings::BACKUP.key)).unwrap();
    assert!(lines[at - 1].starts_with("# "), "{primary}");
    assert!(target_text.contains("user = \"\"") && target_text.contains("key = \"/home/me/.ssh/id\""), "{target_text}");
    let silent = &Setup { notify: "  ".into(), ..blank.clone() }.profiles()[0].1;
    assert!(!silent.contains(settings::NOTIFY.key), "{silent}");
    let notified = Setup { notify: " json://localhost/hook ".into(), ..blank }.profiles();
    assert!(
        notified[0].1.contains(&format!("{} = \"json://localhost/hook\"", settings::NOTIFY.key)),
        "{}",
        notified[0].1
    );
}

#[test]
fn paths_copied_from_file_explorer_lose_their_quotes() {
    let values = [("source.0", r#""C:\Users\me\Documents""#), ("source.10", ""), ("source.2", "  /home/me  ")];
    let setup = Setup { values: values.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(), ..Setup::default() };
    assert_eq!(setup.sources(), [r"C:\Users\me\Documents", "/home/me"]);
    let local = target(Kind::Local, &[("repository", r#""D:\backup""#)]);
    let (_, text) = &local.profiles()[1];
    assert!(text.contains(r"'D:\backup'"), "{text}");
}

#[test]
fn profiles_read_back_as_written() {
    let setup = target(Kind::Sftp, &[("user", "me"), ("endpoint", "ssh://nas:22")]);
    let profiles = setup.profiles();
    let [(file, primary), (target_file, target_text)] = profiles.as_slice() else { panic!("{profiles:?}") };
    assert_eq!((file.as_str(), target_file.as_str()), ("Documents-local.toml", "Documents-sftp.toml"));
    assert_eq!(Setup { target: Kind::None, ..setup.clone() }.profiles()[0].0, "Documents.toml");
    let same = Setup { target: Kind::Local, ..setup.clone() };
    assert_eq!(same.profile_name(Role::Target), "Documents-local-copy");
    let host = crate::sys::hostname().split('.').next().unwrap_or_default();
    assert!(primary.contains(&format!("[backup]\nhost = \"{host}\"")), "{primary}");
    let p = profile::parse("Documents-local", primary);
    assert!(p.is_backup_profile() && p.has_keep && p.targets == ["Documents-sftp"]);
    assert!(primary.contains("keep-last = 10"), "{primary}");
    let month = &Setup { keep: Keep::Month, ..setup.clone() }.profiles()[0].1;
    assert!(month.contains("keep-daily = 30") && !month.contains("keep-monthly"), "{month}");
    let all = &Setup { keep: Keep::All, ..setup.clone() }.profiles()[0].1;
    assert!(!profile::parse("Documents-local", all).has_keep, "{all}");
    let t = profile::parse("Documents-sftp", target_text);
    assert_eq!(t.repository.as_deref(), Some("opendal:sftp"));
    assert!(target_text.contains("endpoint = \"ssh://nas:22\"") && !target_text.contains("root"), "{target_text}");
    assert!(target_text.contains("known_hosts_strategy = \"accept\""), "{target_text}");
    let picked = target(Kind::Sftp, &[("user", "me"), ("endpoint", "ssh://nas:22"), ("known_hosts_strategy", "maybe")]);
    assert!(matches!(picked.check(), Err(Error::NotAChoice { role: Role::Target, .. })));
    assert!(primary.contains(&format!("{} = \"@daily\"", settings::BACKUP.key)), "{primary}");
    assert!(primary.contains(&format!("{} = \"10\"", settings::CHECK_SUBSETS.key)), "{primary}");
    // Prunes follow what is kept: monthly for a year, weekly for a month, never for everything.
    assert!(primary.contains(&format!("{} = \"@monthly\"", settings::PRUNE.key)), "{primary}");
    let month = Setup { keep: Keep::Month, ..setup.clone() }.profiles();
    assert!(month[0].1.contains(&format!("{} = \"@weekly\"", settings::PRUNE.key)), "{}", month[0].1);
    let manual = Setup { keep: Keep::All, check: Every::Manual, ..setup.clone() }.profiles();
    assert!(!manual[0].1.contains(settings::PRUNE.key), "{}", manual[0].1);
    assert!(!manual[0].1.contains(settings::CHECK_SUBSETS.key), "{}", manual[0].1);
    let mut custom = Setup { check: Every::Custom, ..setup.clone() };
    assert!(matches!(custom.check(), Err(Error::NoCustom)));
    let signal = Setup { notify: "signal://localhost/+1 555 123 4567 macosx://".into(), ..setup.clone() };
    assert!(signal.check().is_ok(), "{:?}", signal.check());
    for notify in ["tgram://{bot_token}", "tgram://only-a-token"] {
        assert!(matches!(Setup { notify: notify.into(), ..setup.clone() }.check(), Err(Error::Notify(_))), "{notify}");
    }
    custom.values.insert("check.custom".into(), "0 3 * * 1".into());
    assert!(custom.profiles()[0].1.contains(&format!("{} = \"0 3 * * 1\"", settings::CHECK.key)));
    assert!(<Every as strum::IntoEnumIterator>::iter().all(|e| Every::try_from(e.id()).is_ok_and(|c| c == e)));
    assert!(setup.check().is_ok());
    assert!(matches!(
        Setup { target: Kind::Rclone, ..setup.clone() }.check(),
        Err(Error::Incomplete { role: Role::Target, field: "remote" })
    ));
    assert!(matches!(
        Setup { primary: Kind::S3, ..setup.clone() }.check(),
        Err(Error::Incomplete { role: Role::Primary, field: "server" })
    ));
    assert!(matches!(Setup { name: "../x".into(), ..setup.clone() }.check(), Err(Error::Name)));
    assert!(matches!(Setup { name: "-rf".into(), ..setup }.check(), Err(Error::Name)));
    assert!(<Kind as strum::IntoEnumIterator>::iter().all(|k| Kind::try_from(k.id()).is_ok_and(|c| c == k)));
    let rest = target(Kind::Rest, &[("repository", "http://h/r")]);
    assert!(rest.profiles()[1].1.contains("repository = \"rest:http://h/r\""));
    let rclone = target(Kind::Rclone, &[("repository", "gd:b"), ("RCLONE_CONFIG_PASS", "x")]);
    let pass = "RCLONE_CONFIG_PASS = \"x\"\n";
    let profiles = rclone.profiles();
    assert!(profiles.iter().all(|(_, text)| text.contains(pass) && !text.contains("options")), "{profiles:?}");
    // A remote primary, with the target's values left over from another choice.
    let mut remote = Setup { primary: Kind::Rclone, ..rclone };
    remote.values.insert("primary.rclone.repository".into(), "gd:primary".into());
    let profiles = remote.profiles();
    assert!(profiles[0].1.contains("repository = \"rclone:gd:primary\""), "{profiles:?}");
    assert!(remote.check().is_ok());
    let other = target(Kind::Rclone, &[("repository", "my nas:backup")]);
    assert_eq!(other.profile_name(Role::Target), "Documents-my-nas");
    let unnamed = target(Kind::Rclone, &[("repository", ":local:/backup")]);
    assert_eq!(unnamed.profile_name(Role::Target), "Documents-rclone");
    let gdrive = target(Kind::Gdrive, &[("repository", "gdrive:backup")]);
    let profiles = gdrive.profiles();
    assert_eq!(profiles[1].0, "Documents-gdrive.toml");
    assert!(profiles[1].1.contains("repository = \"rclone:gdrive:backup\""), "{profiles:?}");
}

/// A user's setup notifies on their desktop, which a system service has none of. Passwords lose the spaces around
/// them, as pasted.
#[test]
fn a_setup_notifies_on_the_desktop_and_trims_passwords() {
    let desktop = super::answers::desktop();
    if crate::sys::is_root() {
        assert_eq!(desktop, None);
    } else {
        assert!(desktop.is_some_and(|url| !url.is_empty() && url.ends_with("://")), "{desktop:?}");
    }
    assert_eq!(Setup::default().notify, desktop.unwrap_or_default());
    let pasted = Setup { password: " pw ".into(), target_password: "\tother\n".into(), ..Setup::default() };
    assert_eq!([pasted.password(Role::Primary), pasted.password(Role::Target)], ["pw", "other"]);
}
