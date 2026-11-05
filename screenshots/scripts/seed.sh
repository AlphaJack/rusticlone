#!/bin/bash
# Gives jack the Documents collection of the book, a Pictures one with two copy targets, and a few snapshots, as after two weeks of use.
set -euo pipefail
cd ~
source /screenshots/scripts/bytes.sh
# The same sizes and dates on every retake.
RANDOM=1

# fill PATH SIZE [DATE]: random content, last modified on DATE or some day of the year before the first snapshot.
fill() { mkdir -p "$(dirname "$1")"; bytes "$2" "$1 $2 ${3:-}" > "$1"; touch -d "${3:-2026-09-07 $((RANDOM % 365)) days ago}" "$1"; }
for f in Taxes/2025/{Tax-return.pdf,Receipts.zip} Taxes/2024/Tax-return.pdf Contracts/{Rent-agreement.pdf,Car-lease.pdf} \
  CV/{CV-Jack.odt,Cover-letter.odt} Recipes/{Tiramisu,Carbonara,Lasagne}.md Notes/{Ideas,Books}.md; do
  fill "Documents/$f" $((RANDOM * 40))
done
for month in 0{1..9} 10; do fill Documents/Invoices/2026-$month.pdf 90000 "2026-$month-05 10:00"; done
for trip in 2025/Lisbon 2025/Dolomites 2026/Sardinia; do
  for i in $(seq -w 1 15); do fill "Pictures/$trip/IMG_$RANDOM$i.jpg" $((3000000 + RANDOM * 30)); done
done

mkdir -p .config/rustic && cd .config/rustic
# primary NAME TARGETS: the backup profile of ~/NAME, on the HDD.
primary() {
  cat > "$1-hdd.toml" <<TOML
[[backup.snapshots]]
sources = ["/home/jack/$1"]

[repository]
repository = "/mnt/hdd/backups/$1"
password = "correct-horse-battery-staple"

[copy]
targets = [$2]

[forget]
keep-last = 10
keep-daily = 7
keep-monthly = 12

[global.env]
RUSTICLONE_BACKUP_CRON = "@daily"
RUSTICLONE_CHECK_CRON = "@weekly"
RUSTICLONE_PRUNE_CRON = "@monthly"
TOML
}
# remote PROFILE CLOUD [SECTION]: a copy target in the cloud, faked by a folder that takes 20 MiB/s.
remote() {
  cat > "$1.toml" <<TOML
[repository]
repository = "opendal:fs"
password = "correct-horse-battery-staple"

[repository.options]
root = "/mnt/$2/backups/${1%-*}"
throttle = "20MiB,64MiB"
${3:-}
TOML
}
primary Documents '"Documents-b2"'
primary Pictures '"Pictures-gdrive", "Pictures-b2"'
# B2 has Object Lock, so no [forget].
remote Documents-b2 b2
remote Pictures-b2 b2
remote Pictures-gdrive gdrive $'\n[forget]\nkeep-weekly = 8\nkeep-monthly = 24'
cd ~

# The first snapshot holds everything; the next ones only what changed: a scan changed, one deleted and one added; then,
# in rusticlone's first run, which copies them to the cloud, two added and one deleted.
for i in $(seq -w 1 40); do fill Documents/Scans/Scan-$i 10000000 "2026-10-11 18:00"; done
for name in Documents Pictures; do rustic -P $name-hdd backup --init --time "2026-10-12 21:00:00" >/dev/null 2>&1; done
rm Documents/Scans/Scan-02
for i in 01 41; do fill Documents/Scans/Scan-$i 10000000 "2026-10-19 18:00"; done
rustic -P Documents-hdd backup --time "2026-10-19 21:00:00" >/dev/null 2>&1
rm Documents/Scans/Scan-03
for i in 42 43; do fill Documents/Scans/Scan-$i 10000000 "2026-10-26 18:00"; done
rusticlone backup --all >/dev/null
rusticlone check --all >/dev/null
