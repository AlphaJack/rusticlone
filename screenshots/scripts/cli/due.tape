Set Height 300
Source "/screenshots/scripts/settings.tape"
# Pictures-gdrive checked every minute, so that one of the runs is due.
Hide
Type `printf '\n[global.env]\nRUSTICLONE_CHECK_CRON = "* * * * *"\n' >> .config/rustic/Pictures-gdrive.toml; clear`
Enter
Wait
Show
Type "rusticlone due --all"
Enter
Wait@300s
Sleep 500ms
Screenshot "/screenshots/cli-THEME-due.png"
Hide
Type `sed -i '/^\[global.env\]$/,$d' .config/rustic/Pictures-gdrive.toml`
Enter
Wait
