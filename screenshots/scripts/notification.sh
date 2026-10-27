#!/bin/bash
# Screenshot of the desktop notification about Documents, in the color scheme of $1, drawn by dunst in the web UI's colors;
# without a compositor, the corners it rounds are black.
set -euo pipefail
source /screenshots/scripts/colors.sh && colors "$1"
export DISPLAY=:99
Xvfb $DISPLAY -screen 0 1400x600x24 & trap 'kill $(jobs -p)' EXIT
until xsetroot -solid "$background" 2>/dev/null; do sleep 0.2; done
eval "$(dbus-launch --sh-syntax)"
mkdir -p ~/.config/dunst && cat > ~/.config/dunst/dunstrc <<INI
[global]
font = Inter 11
scale = 2
width = 460
origin = top-left
offset = (40, 40)
frame_width = 1
frame_color = "$border"
corner_radius = 12
padding = 16
horizontal_padding = 18
format = "<b>%s</b>\n%b"
background = "$card"
foreground = "$foreground"
timeout = 0
INI
dunst & sleep 1
rusticlone notify --collection Documents >/dev/null || true
sleep 1
import -window root png:- | convert - -fill "$background" -opaque black -trim +repage -bordercolor "$background" -border 40 "/screenshots/notification-$1.png"
