# Restore

## Disaster recovery

Don't panic! On a reinstalled computer:

1. download your rustic profiles to a folder. You kept a copy of them, right?
2. download Rusticlone and move it to the folder with your profiles
3. run `rusticlone restore --all`. Rusticlone offers to download rustic if it's missing. If you don't feel sure yet, append `--dry-run` to see what would happen, or restore one `--collection` at a time

<img class="light-only" alt="CLI restore" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-light-restore.png">
<img class="dark-only" alt="CLI restore" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-dark-restore.png" loading="lazy">

This will restore each collection from the latest snapshot of available repositories, giving preference to the primary.
Rusticlone then offers to copy the profiles where it usually looks for them, so the backup server can use them too.

## Partial restore

For more granular control over file restore you can use flags:

```bash
# check how many files would be created or replaced in a real restore
rusticlone restore --collection Documents --dry-run
# extract one folder of a snapshot to /tmp/restored/Taxes
rusticlone restore --collection Documents --snapshot 1a2b3c4d --snapshot-path /home/jack/Documents/Taxes --to /tmp/restored
# restore documents as they were on the 1st of October, from the repository with the most recent snapshots before that date
rusticlone restore --collection Documents --at-time 2026-10-01
```

More flags are available with `rusticlone restore --help`

The web UI offers the same options:

<img class="light-only" alt="Web UI restore" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/web-light-restore.png">
<img class="dark-only" alt="Web UI restore" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/web-dark-restore.png" loading="lazy">

## Resuming backups after disaster recovery

**After a disaster recovery, re-create the primary backup repository from an existing copy repository, never with a backup operation.**

If the primary repository is lost too, a plain `rusticlone restore --all` or `rusticlone restore --collection NAME` does this for you: it re-creates the primary from an available copy target (e.g. `Documents-b2`), then extracts the files to their original location, so you only download once.

The reason is the chunker: a fresh repository created by a backup gets a random chunker polynomial, incompatible with the existing copy targets.
Snapshots would succeed, but copies to the targets would fail with a chunker polynomial mismatch.
[At the moment](https://github.com/rustic-rs/rustic/pull/1954) a rustic profile cannot set the chunker polynomial.

To re-create a compatible primary repository from an existing copy target (or vice-versa) by hand, run:

```bash
rusticlone recreate --repo Documents-hdd
```
