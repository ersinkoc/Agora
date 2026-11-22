# F5 on Linux: reads the spike window's accessibility tree over AT-SPI (what Orca reads) and presses the button through it.
# Usage: python3 -I atspi_check.py <pid>. Prints one JSON object. Needs python3-gi with Atspi 2.0.
import json, sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

pid = int(sys.argv[1])
found = {"app": None, "nodes": [], "list_items": [], "buttons": [], "lists": [], "button_action": None}

def walk(n, depth):
    if depth > 40:
        return
    try:
        role, name = n.get_role_name(), n.get_name()
    except Exception:
        return
    found["nodes"].append({"depth": depth, "role": role, "name": name})
    if role == "list item":
        found["list_items"].append(name)
    if role == "list":
        found["lists"].append(name)
    if role in ("push button", "button"):
        found["buttons"].append(name)
        if name == "Open terminal" and found["button_action"] is None:
            try:
                act = n.get_action_iface()
                names = [act.get_action_name(i) for i in range(act.get_n_actions())]
                ok = act.do_action(0) if names else False
                found["button_action"] = {"actions": names, "invoked": bool(ok)}
            except Exception as e:
                found["button_action"] = {"error": str(e)}
    try:
        for i in range(n.get_child_count()):
            c = n.get_child_at_index(i)
            if c is not None:
                walk(c, depth + 1)
    except Exception:
        pass

end = time.time() + 15
while time.time() < end and found["app"] is None:
    desk = Atspi.get_desktop(0)
    for i in range(desk.get_child_count()):
        a = desk.get_child_at_index(i)
        try:
            if a is not None and a.get_process_id() == pid:
                found["app"] = a.get_name()
                walk(a, 0)
        except Exception:
            pass
    if found["app"] is None:
        time.sleep(0.5)
found["nodes"] = found["nodes"][:80]
print(json.dumps(found))
