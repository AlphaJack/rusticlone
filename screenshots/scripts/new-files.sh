#!/bin/bash
# New scans and photos, for backups slow enough to show their progress.
set -eu
source /screenshots/scripts/bytes.sh
# Numbered, so that each run's files are new to rustic, and the same on every retake.
run=$(($(cat ~/.new-files 2>/dev/null || echo 0) + 1)) && echo $run > ~/.new-files
# new DIR NAME COUNT: COUNT files of 10 MB in DIR, replacing those of the previous run.
new() { rm -rf "$1" && mkdir -p "$1" && for i in $(seq -w 1 "$3"); do bytes 10000000 "$1/$2-$i $run" > "$1/$2-$i"; done; }
new ~/Documents/Scans Scan 40
new ~/Pictures/2026/Iceland IMG 30
