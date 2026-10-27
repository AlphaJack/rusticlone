#!/bin/bash
# Screenshot of the tray icon's menu, in the color scheme of $1: a panel along the top of the screen with only a tray,
# drawn by Xfce in GTK's dark or light theme, and the server started from a terminal, as a user would.
set -euo pipefail
source /screenshots/scripts/colors.sh && colors "$1"
export DISPLAY=:98 GDK_SCALE=2 GTK_THEME=Adwaita$([ "$1" = dark ] && echo :dark || true)
# The server outlives the terminal that script gives it, so it is stopped by name.
Xvfb $DISPLAY -screen 0 2400x1200x24 & trap 'pkill -x rusticlone; kill $(jobs -p) 2>/dev/null; rm -f /tmp/.X98-lock' EXIT
until xsetroot -solid "$background" 2>/dev/null; do sleep 0.2; done
eval "$(dbus-launch --sh-syntax)"
config=~/.config/xfce4/xfconf/xfce-perchannel-xml
mkdir -p $config && cat > $config/xfce4-panel.xml <<XML
<?xml version="1.0" encoding="UTF-8"?>
<channel name="xfce4-panel" version="1.0">
  <property name="configver" type="int" value="2"/>
  <property name="panels" type="array">
    <value type="int" value="1"/>
    <property name="panel-1" type="empty">
      <property name="position" type="string" value="p=6;x=0;y=0"/>
      <property name="length" type="double" value="92"/>
      <property name="position-locked" type="bool" value="true"/>
      <property name="size" type="uint" value="28"/>
      <property name="plugin-ids" type="array">
        <value type="int" value="1"/>
        <value type="int" value="2"/>
      </property>
    </property>
  </property>
  <property name="plugins" type="empty">
    <property name="plugin-1" type="string" value="separator">
      <property name="expand" type="bool" value="true"/>
      <property name="style" type="uint" value="0"/>
    </property>
    <property name="plugin-2" type="string" value="systray"/>
  </property>
</channel>
XML
python3 /screenshots/scripts/portal.py "$1" &
# The icon reads the color scheme when it first shows.
until dbus-send --session --print-reply --dest=org.freedesktop.DBus / org.freedesktop.DBus.NameHasOwner \
  string:org.freedesktop.portal.Desktop 2>/dev/null | grep -q true; do sleep 0.2; done
xfce4-panel >/dev/null 2>&1 &
# The icon shows only for a server that a user runs: one with a terminal.
script -qfec "rusticlone serve" /dev/null >/dev/null &
until rusticlone healthcheck >/dev/null 2>&1; do sleep 0.5; done
# Until the panel shows the icon.
sleep 8
# The tray's only icon, at the right end of a panel that leaves the screen's corners free; its menu opens below it. Without a window manager, the screen
# around them is black.
eval "$(xdotool search --onlyvisible --class xfce4-panel | head -1 | xargs xdotool getwindowgeometry --shell)"
xdotool mousemove $((X + WIDTH - 24)) $((Y + HEIGHT / 2)) && sleep 0.5 && xdotool click 3
# Away from the icon, which it would highlight.
sleep 1 && xdotool mousemove 0 1199
sleep 2
import -window root png:- | convert - -gravity northeast -crop 740x500+0+0 +repage -fill "$background" -opaque black \
  -gravity southwest -background "$background" -splice 40x0 "/screenshots/tray-$1.png"
