# Notifications

Notifications are only sent when a collection's status changes (like Uptime Kuma up/down), not at every run.

<img class="light-only" alt="Notification" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/notification-light.png" width="503">
<img class="dark-only" alt="Notification" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/notification-dark.png" width="503" loading="lazy">

If you'd rather receive a notification at every backup operation, you can use rustic [hooks](https://github.com/rustic-rs/rustic/blob/main/config/full.toml#L207) in your profiles.

The `RUSTICLONE_NOTIFY_URLS` variable accepts space-delimited [Apprise](https://appriseit.com/services/) URLs.
`rusticlone notify --all` sends the current status, so you can use it to test URLs.
