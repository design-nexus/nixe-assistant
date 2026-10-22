"""Hand tools to a Nixe app's control interface (io.github.design_nexus.nixe.Control)
when the app is installed. The app describes its tools with `--describe` (no window);
calls start it if it isn't running and go to its primary instance over the session bus."""
import json
import os
import shutil
import subprocess
import threading
import time
from pathlib import Path

from desktop_plugin import run

IFACE = 'io.github.design_nexus.nixe.Control'


def program(name):
    local = Path.home() / '.local/bin' / name
    if local.is_file() and os.access(local, os.X_OK):
        return str(local)
    return shutil.which(name)


def describe(name, allowed):
    """The app's tools named in `allowed`, or None when the app isn't there (or lacks one)."""
    exe = program(name)
    if not exe:
        return None
    try:
        # An older app would take --describe for a window; its --help never opens one.
        usage = subprocess.run([exe, '--help'], capture_output=True, timeout=10, check=True).stdout
        if b'--describe' not in usage:
            return None
        out = subprocess.run([exe, '--describe'], capture_output=True, timeout=10, check=True).stdout
        tools = json.loads(out)
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    offered = [t for t in tools if isinstance(t, dict) and t.get('name') in allowed and isinstance(t.get('inputSchema'), dict)]
    # All or nothing: the host wants every declared tool offered.
    return offered if {t['name'] for t in offered} == set(allowed) else None


def _running(bus, cancelled):
    out = run(['busctl', '--user', '--json=short', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus',
               'org.freedesktop.DBus', 'NameHasOwner', 's', bus], cancelled)
    return json.loads(out)['data'][0] is True


def _ensure(bus, name, cancelled):
    if _running(bus, cancelled):
        return
    exe = program(name)
    if not exe:
        raise ValueError(f'{name} is not installed')
    proc = subprocess.Popen([exe], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    # Reaped when the app quits, so no defunct process lingers.
    threading.Thread(target=proc.wait, daemon=True).start()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if cancelled.is_set():
            raise ValueError('cancelled')
        time.sleep(.25)
        if _running(bus, cancelled):
            time.sleep(1)  # its lists and calendars loading
            return
    raise ValueError(f'{name} did not start')


def call(bus, name, tool, args, cancelled):
    args = {k: v for k, v in args.items() if k != 'request'}
    _ensure(bus, name, cancelled)
    path = '/' + bus.replace('.', '/')
    out = run(['busctl', '--user', '--json=short', 'call', bus, path, IFACE, 'Call', 'ss', tool, json.dumps(args)],
              cancelled, timeout=12)
    reply = json.loads(json.loads(out)['data'][0])
    if 'error' in reply:
        raise ValueError(reply['error'])
    return reply['ok']
