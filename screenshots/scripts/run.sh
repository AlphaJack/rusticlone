#!/bin/bash
# Takes the book's screenshots into screenshots/: MyComputer, with the book's Documents collection and a few snapshots, its terminal
# recorded by vhs and its web UI by Playwright, each in dark and light mode. Needs only Docker.
#
# run.sh                 sets MyComputer up, takes every screenshot, and removes it
# run.sh --setup         sets MyComputer up and leaves it running
# run.sh --only NAME...  takes these screenshots on the running MyComputer: web (the pages), a tape of cli/ (backup,
#                        check...), runs, tray or notification. Each changes what the later ones show, in this order
# run.sh --teardown      removes MyComputer
set -euo pipefail
cd "$(dirname "$0")/../.."
source screenshots/scripts/colors.sh
rustic=$(cargo metadata --format-version 1 --no-deps | jq -r .metadata.rustic.version)
playwright=1.58.0
screenshots=rusticlone-screenshots
everything=(web $(basename -s .tape screenshots/scripts/cli/*.tape) runs tray notification)

jack() { docker exec -i -u jack -w /home/jack $screenshots "$@"; }

setup() {
  local image
  image=$(docker build -q -f screenshots/scripts/Dockerfile --build-arg RUSTIC="$rustic" .)
  docker network create $screenshots >/dev/null
  # With an init that reaps what the tapes start in the background.
  docker run -d --init --name $screenshots --hostname MyComputer --network $screenshots --network-alias mycomputer -e TZ=Europe/Berlin \
    -e RUSTICLONE_ADMIN_TOKEN=screenshots -v "$PWD/screenshots:/screenshots" --entrypoint sleep "$image" infinity >/dev/null
  jack /screenshots/scripts/seed.sh
}

teardown() {
  docker rm -f $screenshots >/dev/null 2>&1 || true
  docker network rm $screenshots >/dev/null 2>&1 || true
}

# The server runs only for the web UI, so that it runs nothing the terminal shows.
serve() {
  docker exec -d -u jack -w /home/jack $screenshots rusticlone serve
  until jack rusticlone healthcheck >/dev/null 2>&1; do sleep 1; done
}
unserve() { jack pkill -f "rusticlone serve" || true; }

# web SCHEME [runs]: the web UI's pages, or its recent runs.
web() {
  echo "$1 web UI ${2:-}"
  docker run --rm --network $screenshots -v "$PWD/screenshots:/screenshots" mcr.microsoft.com/playwright/python:v$playwright-noble \
    bash -c "pip install -q --break-system-packages playwright==$playwright 2>/dev/null && python /screenshots/scripts/web.py $*"
}

# record TAPE SCHEME: the tape's screenshots, without the empty lines below the output, as vhs records a fixed height.
record() {
  local tape=screenshots/scripts/cli/$1.tape shots tries=0
  echo "$2 $1"
  shots=$(sed -n "s|^Screenshot \"\(.*\)\"|\1|p" "$tape" | sed "s/THEME/$2/g" | tr '\n' ' ')
  # vhs's browser sometimes starts without a terminal, and then writes no screenshot: a tape is done once they all exist.
  until jack rm -f $shots && { echo "Set Theme $(theme $2)"; sed "s/THEME/$2/g" "$tape"; echo "Sleep 1s"; } |
    jack bash -c 'cat > /tmp/t.tape && vhs /tmp/t.tape >/tmp/vhs.log 2>&1' && jack ls $shots >/dev/null 2>&1; do
    tries=$((tries + 1)) && [ $tries -lt 3 ] || { jack tail -20 /tmp/vhs.log >&2 && echo "$1 failed in $2" >&2 && exit 1; }
  done
  colors $2
  jack bash -c "mogrify -define trim:edges=south -trim +repage -background '$card' -gravity south -splice 0x24 $shots"
}

take() {
  case $1 in
    web)
      serve
      for scheme in dark light; do web $scheme; done
      unserve
      ;;
    tray)
      for scheme in dark light; do
        echo "$scheme tray"
        jack /screenshots/scripts/tray.sh $scheme
      done
      ;;
    runs)
      serve
      for scheme in dark light; do
        jack /screenshots/scripts/new-files.sh
        web $scheme runs
      done
      unserve
      ;;
    notification)
      # Documents' repository gone from the HDD, so that Documents is failing, as its notification says.
      jack mv /mnt/hdd/backups/Documents /mnt/hdd/backups/Documents.lost
      jack sed -i '/^\[global.env\]/a RUSTICLONE_NOTIFY_URLS = "dbus://"' .config/rustic/Documents-hdd.toml
      jack rusticlone backup --collection Documents >/dev/null || true
      for scheme in dark light; do
        echo "$scheme notification"
        jack /screenshots/scripts/notification.sh $scheme
      done
      ;;
    *) for scheme in dark light; do record "$1" $scheme; done ;;
  esac
}

# only NAME...: the screenshots, then each file they wrote at a fraction of its size. A palette of 256 colors without
# dithering holds the flat colors of a UI, and without the timestamps ImageMagick writes, a picture that did not change
# is the same file and adds nothing to git.
only() {
  jack touch /tmp/started
  for name in "$@"; do take "$name"; done
  docker exec $screenshots bash -c 'find /screenshots -maxdepth 1 -name "*.png" -newer /tmp/started -exec pngquant --nofs --strip --speed 1 --force --ext .png {} +'
}

case ${1:-} in
  --setup) setup ;;
  --teardown) teardown ;;
  --only) shift && only "$@" ;;
  "")
    trap teardown EXIT
    teardown
    setup
    only "${everything[@]}"
    ;;
  *) sed -n '5,9s/^# //p' "$0" >&2 && exit 2 ;;
esac
