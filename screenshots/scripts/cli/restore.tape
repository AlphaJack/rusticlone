Set Height 200
Source "/screenshots/scripts/settings.tape"
Hide
# Documents lost, Pictures still in place: one restores everything, the other nothing.
Type "mkdir -p /tmp/lost && mv Documents /tmp/lost; clear"
Enter
Wait
Show
Type "rusticlone restore --all --dry-run"
Enter
# Its first step running, then the time the terminal takes to draw it.
Wait+Screen@30s /[◐◓◑◒] restore/
Sleep 300ms
Screenshot "/screenshots/cli-THEME-restore-progress.png"
Wait@600s
Sleep 500ms
Screenshot "/screenshots/cli-THEME-restore.png"
Hide
Type "mv /tmp/lost/* ~"
Enter
Wait
