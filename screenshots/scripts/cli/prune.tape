Set Height 360
Source "/screenshots/scripts/settings.tape"
Type "rusticlone prune --all"
Enter
# Its first step running, then the time the terminal takes to draw it.
Wait+Screen@30s /[◐◓◑◒] prune Documents-hdd/
Sleep 300ms
Screenshot "/screenshots/cli-THEME-prune-progress.png"
Wait@120s
Sleep 500ms
Screenshot "/screenshots/cli-THEME-prune.png"
