"""The desktop portal's setting that the tray icon reads to draw itself dark or light: the color scheme of argv[1]."""

import sys

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

# 1 prefers dark, 2 prefers light.
SCHEME = 1 if sys.argv[1] == "dark" else 2


class Settings(dbus.service.Object):
    @dbus.service.method("org.freedesktop.portal.Settings", in_signature="ss", out_signature="v")
    def ReadOne(self, namespace: str, key: str) -> dbus.UInt32:
        return dbus.UInt32(SCHEME, variant_level=1)


DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName("org.freedesktop.portal.Desktop", bus)
Settings(bus, "/org/freedesktop/portal/desktop")
GLib.MainLoop().run()
