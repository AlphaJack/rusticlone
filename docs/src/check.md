# Checks

`rusticlone check --collection Documents` runs a data consistency check on each repository of the collection.
By default, 1/10th of each repo is checked every time and Rusticlone remembers which subset is due next, so with 10 runs it covers the entire repo.
If a subset is failing, it will not advance to the next subset, and will notify you of the issue.

<img class="light-only" alt="CLI check" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-light-check-progress.png">
<img class="dark-only" alt="CLI check" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-dark-check-progress.png" loading="lazy">

The `RUSTICLONE_CHECK_SUBSETS` variable also accepts 0 for fast metadata-only checks, and values like "10MiB" or "10%" for random subsets.
Repositories in the same collection can have separate check schedules.
