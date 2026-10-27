Set Height 320
Source "/screenshots/scripts/settings.tape"
Hide
Type "/screenshots/scripts/new-files.sh; clear"
Enter
Wait
Show
Type "rusticlone backup --all"
Enter
# Pictures waits for the disk and CPU that Documents' snapshot holds, then for the upload of its copy.
Wait+Screen@30s /snapshot to Documents-hdd: [0-9]+%/
Screenshot "/screenshots/cli-THEME-backup-1.png"
Wait+Screen@60s /copy to Documents-b2: [0-9]+%/
Screenshot "/screenshots/cli-THEME-backup-2.png"
Wait+Screen@120s /copy to Pictures-gdrive: [0-9]+%/
Screenshot "/screenshots/cli-THEME-backup-3.png"
Wait@300s
Sleep 500ms
Screenshot "/screenshots/cli-THEME-backup-4.png"
