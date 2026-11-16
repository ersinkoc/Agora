"""Private D-Bus notification daemon; no notice reaches the user's desktop.

Run with dbus-run-session -- /usr/bin/python3 this-file DRIVER (system dbus/gi).
The driver calls the real agora_platform::desktop_notify, which runs notify-send.
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

DBusGMainLoop(set_as_default=True)
loop = GLib.MainLoop()
seen = []
control = len(sys.argv) > 2 and sys.argv[2] == "--positive-control"


class Notifications(dbus.service.Object):
    @dbus.service.method("org.freedesktop.Notifications", in_signature="", out_signature="as")
    def GetCapabilities(self):
        return ["actions", "body"]

    @dbus.service.method("org.freedesktop.Notifications", in_signature="", out_signature="ssss")
    def GetServerInformation(self):
        return "isolated-review", "review", "1", "1.2"

    @dbus.service.method("org.freedesktop.Notifications", in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout):
        seen.append({"app": str(app), "summary": str(summary), "body": str(body),
                     "actions": [str(a) for a in actions], "hints": {str(k): str(v) for k, v in hints.items()}})
        if actions:
            GLib.timeout_add(50, self.activate, str(actions[0]))
        return 1

    @dbus.service.signal("org.freedesktop.Notifications", signature="us")
    def ActionInvoked(self, notification_id, action):
        pass

    @dbus.service.signal("org.freedesktop.Notifications", signature="uu")
    def NotificationClosed(self, notification_id, reason):
        pass

    def activate(self, action):
        self.ActionInvoked(1, action)
        self.NotificationClosed(1, 2)
        return False

    @dbus.service.method("org.freedesktop.Notifications", in_signature="u", out_signature="")
    def CloseNotification(self, notification_id):
        pass


bus = dbus.SessionBus()
name = dbus.service.BusName("org.freedesktop.Notifications", bus=bus)
server = Notifications(bus, "/org/freedesktop/Notifications")
if control:
    # This host confines /usr/bin/notify-send with an AppArmor profile that denies
    # GetCapabilities against a private daemon. Use an identical local copy for
    # the action-capable control; no installed binary or system policy is edited.
    control_program = Path(sys.argv[1]).resolve().parent / "0296-notify-send-control"
    shutil.copyfile("/usr/bin/notify-send", control_program)
    control_program.chmod(0o700)
command = ([str(control_program), "--action=default=Open", "--", "Review fixture", "An approval is waiting for you."]
           if control else [sys.argv[1], "Review fixture", "An approval is waiting for you."])
child = subprocess.Popen(command,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def poll():
    if child.poll() is None:
        return True
    loop.quit()
    return False


def expire():
    child.kill()
    loop.quit()
    return False


GLib.timeout_add(25, poll)
GLib.timeout_add_seconds(8, expire)
loop.run()
out, err = child.communicate(timeout=2)
assert child.returncode == 0, (child.returncode, out, err)
assert out.strip() == ("default" if control else "ok"), (out, err)
assert len(seen) == 1, seen
print(json.dumps({"positive_control": control, "notifier_result": out.strip(), "notification": seen[0]}, indent=2))
assert seen[0]["actions"], "No action is registered; clicking this notification cannot activate the desktop app."
