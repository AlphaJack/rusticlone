# Smartphones

Rusticlone doesn't have a mobile app, but it can back up their files once they reach a computer or server:

- Android: sync the folders you care about with [Syncthing](https://syncthing.net/)
- iPhone: sync folders with [Möbius Sync](https://www.mobiussync.com/), or turn on "Download Originals" for iCloud Photos on a Mac or Windows computer
- Photos from both: upload them to a self-hosted [Immich](https://immich.app/) server

Then back up the folder where they land, creating a new profile:

`Mobile-hdd.toml`
```toml
[[backup.snapshots]]
sources = ["/home/jack/Sync/Mobile"]

[repository]
repository = "/mnt/hdd/backups/Mobile"
password = "XXXXXX"

[global.env]
RUSTICLONE_BACKUP_CRON="@daily"
```

To check on your backups from your smartphone on the same network:

1. Set `RUSTICLONE_LISTEN=0.0.0.0:32110` in the configuration file, then restart Rusticlone
2. Open `http://<computer ip address>:32110` on the phone and log in with the admin token (see [Web UI](web-ui.md))
3. Add the page to the home screen, where it opens as a progressive web app (PWA) in its own window

To see backups from another network, see [remote server](web-ui.md#remote-server) for a reverse proxy or Cloudflare Tunnel setup.
