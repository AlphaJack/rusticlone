Set Height 300
Source "/screenshots/scripts/settings.tape"
# Two checks in a row read two subsequent subsets.
Type "rusticlone check --collection Documents"
Enter
Wait@60s
Type "rusticlone check --collection Documents"
Enter
# Its first step running, then the time the terminal takes to draw it.
Wait+Screen@30s /[◐◓◑◒] check Documents-hdd/
Sleep 300ms
Screenshot "/screenshots/cli-THEME-check-progress.png"
Wait@60s
Sleep 500ms
Screenshot "/screenshots/cli-THEME-check.png"
