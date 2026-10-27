Set Height 300
Source "/screenshots/scripts/settings.tape"
# Another command backs Documents up meanwhile: what it holds shows in this one's wait.
Hide
Type "/screenshots/scripts/new-files.sh; (rusticlone backup --collection Documents >/dev/null 2>&1 &); sleep 3; clear"
Enter
Wait
Show
Type "rusticlone backup --collection Pictures"
Enter
Wait+Screen@60s /waiting for cli/
Sleep 300ms
Screenshot "/screenshots/cli-THEME-holds.png"
Wait@300s
Hide
Type "while pgrep -x rusticlone >/dev/null; do sleep 1; done; clear"
Enter
Wait@300s
