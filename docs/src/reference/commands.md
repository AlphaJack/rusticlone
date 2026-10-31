<!-- Generated from src/args.rs by cargo xtask generate -->

# Commands

The same help as `rusticlone --help`, and `--help` after each command.

## Options of every command

- `--json`: Log JSON lines
- `-v, --verbose`: Also print the rustic log that every run saves

## `rusticlone due`

Run what is due and exit, for cron: scheduled backups, pending or failed copies, scheduled prunes and checks

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target

## `rusticlone backup`

Back up a collection, copy it to its copy targets and forget old snapshots

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it

## `rusticlone copy`

Copy missing snapshots from the primary to the copy targets and forget old snapshots

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target

## `rusticlone prune`

Prune the repositories

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target

## `rusticlone check`

Check the repositories: read the next subset of their data or only their structure

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target

## `rusticlone restore`

Restore the sources from --repo or from the repository with the newest snapshots

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the restore would write where. Without other options, read the snapshots and count what a restore in place creates, replaces and keeps
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target
- `--at-time <TIME>`: The newest snapshots taken at or before TIME (2026-10-01, "2026-10-01 12:00")
- `--snapshot <ID>`: This snapshot of the --repo repository (default: the primary)
- `--to <DIR>`: Restore into DIR. A --snapshot-path goes under its name. Each source goes under its full path
- `--missing`: Only missing or empty paths. Never a mount point whose device is missing
- `--source <PATH>`: Only the source with this path (default: every source)
- `--snapshot-path <PATH>`: Only this path of the snapshot

## `rusticlone recreate`

Copy a missing repository back: the primary from its newest target, a target from the primary

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target
- `--from <PROFILE>`: Copy from this profile instead. For the primary: a copy target. For a target: the primary or another target

## `rusticlone notify`

Send each collection's status to its notification URLs now, to test them

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target

## `rusticlone collections`

Show every collection, or the ones named: sources, repositories, schedules, last runs and issues

- `-a, --all`: Every collection
- `-c, --collection <COLLECTION>`: All repositories of the collection with this name or backup profile
- `--dry-run`: Show what the command does without doing or recording it
- `-r, --repo <PROFILE>`: The repository with this profile: a collection's primary or a copy target
- `--refresh`: List the repositories first: their snapshots, sizes and how far the copies are behind

## `rusticlone serve`

Start each run when it is due and serve the web UI

- `--open`: Open the web UI in the browser and log in

## `rusticlone install`

Start the server now and at each login

- `--auto-update`: Also turn on automatic updates in rusticlone.env: at each start, the server updates rusticlone and the rustic it is tested with, after checking their checksums and GitHub attestations

## `rusticlone uninstall`

Stop the server and no longer start it at login

- `--delete`: Also delete the state folder: the runs and the login token. The profiles and the repositories stay

## `rusticlone update`

Replace this executable with a newer release, if there is one

- `--dry-run`: Only show whether a newer release exists

## `rusticlone healthcheck`

Exit 0 if the server answers (a container healthcheck)

