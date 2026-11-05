//! A logind inhibitor, held by the file it hands over: closing it lets go. Polkit grants it to root, and to a user
//! logged in at the computer's screen, whose server runs as a user service.

/// Held until dropped: see the module.
pub(crate) struct Awake(#[expect(dead_code, reason = "held until dropped")] std::os::fd::OwnedFd);

pub(crate) fn stay_awake() -> anyhow::Result<Awake> {
    let inhibit = async {
        let bus = zbus::Connection::system().await?;
        let (login, why) = ("org.freedesktop.login1", crate::i18n::t!("awake-why"));
        let args = ("idle:sleep", crate::APP_NAME, why.as_str(), "block");
        let reply = bus
            .call_method(
                Some(login),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &args,
            )
            .await?;
        Ok::<_, zbus::Error>(std::os::fd::OwnedFd::from(reply.body().deserialize::<zbus::OwnedFd>()?))
    };
    // On a run's thread, outside the server's runtime.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    Ok(Awake(runtime.block_on(inhibit)?))
}

#[cfg(test)]
mod tests {
    use super::{super::unix::output, stay_awake};

    /// The inhibitors that rusticlone holds, as logind lists them.
    fn inhibitors() -> Vec<String> {
        let all = output(&["systemd-inhibit", "--list", "--no-pager", "--no-legend"]);
        all.lines().filter(|l| l.contains(crate::APP_NAME)).map(str::to_owned).collect()
    }

    #[test]
    /// Polkit grants logind's inhibitor to root, as the system's server runs, and to a process in a session at the
    /// computer. It refuses a user who has no such session. A user's service gets it only while the user's session
    /// at the computer is the active one, which this test does not tell. CI runs it as root, at a console and
    /// without a session.
    fn logind_lists_the_inhibitor_until_dropped() {
        let at_the_computer = |session: &str| {
            let shown = output(&["loginctl", "show-session", session, "-p", "Seat", "-p", "Remote"]);
            shown.lines().any(|l| l.strip_prefix("Seat=").is_some_and(|seat| !seat.is_empty()))
                && shown.contains("Remote=no")
        };
        let sessions = output(&["loginctl", "list-sessions", "--no-legend"]);
        let uid = output(&["id", "-u"]);
        let own: Vec<&str> = sessions
            .lines()
            .filter(|l| l.split_whitespace().nth(1) == Some(uid.trim()))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        let granted = crate::sys::is_root() || at_the_computer("self");
        let refused = !own.iter().any(|s| at_the_computer(s));
        let awake = match stay_awake() {
            Err(e) if !granted => {
                let polkit = ["InteractiveAuthorizationRequired", "AccessDenied"];
                assert!(polkit.iter().any(|r| format!("{e:#}").contains(r)), "{e:#}");
                return;
            }
            awake => awake.expect("refused although polkit grants it"),
        };
        assert!(!refused || granted, "granted although polkit refuses it");
        // logind lists it before it answers the call.
        let held = inhibitors();
        assert!(!held.is_empty(), "logind lists no inhibitor: the system bus refused it");
        // In logind's order, which is not the request's.
        assert!(held.iter().all(|l| l.contains("sleep:idle") && l.contains("block")), "{held:?}");
        drop(awake);
        assert!(inhibitors().is_empty(), "the inhibitor outlived the run");
    }
}
