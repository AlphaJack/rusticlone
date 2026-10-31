# Pruning old snapshots

`rusticlone prune --collection Documents` deletes files of snapshots marked for deletion.

<img class="light-only" alt="CLI prune" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-light-prune.png">
<img class="dark-only" alt="CLI prune" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-dark-prune.png" loading="lazy">

As this can be a slow operation, its schedule is independent of the backup and check ones.
Like checks, repositories in the same collection can have separate prune schedules.

A repository without `keep-*` options in `[forget]` won't be pruned by Rusticlone (ideal for Object Lock or append-only repositories).
