<!-- Generated from src/settings.rs by cargo xtask generate -->

# Settings

The environment and the configuration file are read at start: restart rusticlone after editing them. The others are read from the `[global.env]` of rustic profiles at each run.

| Variable | Read from | Default | What it does |
|---|---|---|---|
| `RUSTICLONE_ADMIN_TOKEN` | The environment, the configuration file or the file that `<VARIABLE>_FILE` names |  | Token for the web UI |
| `RUSTICLONE_AUTO_UPDATE` | The environment or the configuration file | `false` | Update rusticlone and the rustic at each restart |
| `RUSTICLONE_BACKUP_CRON` | The backup profile |  | Backup schedule (cron expression, @daily...) |
| `RUSTICLONE_CHECK_CRON` | The repository's profile, then the backup profile |  | Check schedule of the repository |
| `RUSTICLONE_CHECK_SUBSETS` | The repository's profile, then the backup profile | `10` | Data each check reads. N reads the next of N subsets (`--read-data-subset n/N`). `5%`, `2GiB` or `all` reads that much at random. 0 reads the structure only |
| `RUSTICLONE_LISTEN` | The environment or the configuration file | `127.0.0.1:32110` | Web UI address |
| `RUSTICLONE_METRICS_TOKEN` | The environment, the configuration file or the file that `<VARIABLE>_FILE` names |  | Token for Prometheus and other monitors, to send as `Authorization: Bearer` |
| `RUSTICLONE_NOTIFY_URLS` | The backup profile |  | Notification URLs (Mercurio) for when a collection's status changes |
| `RUSTICLONE_PRUNE_CRON` | The repository's profile, then the backup profile |  | Prune schedule of the repository |
| `RUSTICLONE_RUSTIC_BIN` | The environment or the configuration file |  | Location of the rustic executable |
| `RUSTICLONE_STALL_TIMEOUT` | The environment or the configuration file | `1h` | Time to wait before stopping an unresponsive rustic command |
| `RUSTICLONE_STATE_DIR` | The environment or the configuration file |  | Folder for the database and resource locks |
