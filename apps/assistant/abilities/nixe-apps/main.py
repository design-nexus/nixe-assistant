"""Notepad and Write, worked directly through their control interface on the session bus
(io.github.design_nexus.nixe.Control) instead of through their windows. An app that isn't
running is started first."""
import json
import subprocess
import threading
import time
from assistant_plugin import Plugin, tool
from desktop_plugin import TEXT, run, request_tools

PATH_ARG = {'type': 'string', 'minLength': 1, 'maxLength': 4096}
APPS = {
    'notepad': ('io.github.design_nexus.nixe.Notepad', 'nixe-notepad'),
    'write': ('io.github.design_nexus.nixe.Write', 'nixe-write'),
}
TOOLS = [
    tool('notepad.new_tab', 'Open a new, empty Notepad tab.'),
    tool('notepad.open', 'Open a text file (inside the home folder) in a Notepad tab.', {'path': PATH_ARG}, ['path']),
    tool('notepad.read', 'Read the current Notepad tab: name, file and text.'),
    tool('notepad.tabs', "List Notepad's tabs: name, file, unsaved changes."),
    tool('notepad.insert_text', 'Insert text into the current Notepad tab, at the cursor or at the end.',
         {'text': TEXT, 'at': {'enum': ['cursor', 'end']}}, ['text']),
    tool('notepad.save', 'Save the current Notepad tab; give a path (inside the home folder) for an untitled tab or to save under a new name.',
         {'path': PATH_ARG, 'overwrite': {'type': 'boolean'}}),
    tool('write.new_document', 'Start a new, empty Write document.'),
    tool('write.open', 'Open a document (inside the home folder) in Write.', {'path': PATH_ARG}, ['path']),
    tool('write.read', 'Read the open Write document: title, file and text (as Markdown).'),
    tool('write.insert_text', 'Insert text into the Write document at the cursor or the end; each line is a paragraph in the given style.',
         {'text': TEXT, 'style': {'enum': ['body', 'title', 'heading1', 'heading2', 'heading3', 'quote']}, 'at': {'enum': ['cursor', 'end']}}, ['text']),
    tool('write.save', 'Save the Write document; give a path (inside the home folder; .nwrite, .odt, .docx, .md...) for a new document or to save under a new name.',
         {'path': PATH_ARG, 'overwrite': {'type': 'boolean'}}),
    tool('write.export_pdf', 'Export the Write document as a PDF (a path inside the home folder ending in .pdf).',
         {'path': PATH_ARG, 'overwrite': {'type': 'boolean'}}, ['path']),
]
IFACE = 'io.github.design_nexus.nixe.Control'

def running(bus, cancelled):
    out = run(['busctl', '--user', '--json=short', 'call', 'org.freedesktop.DBus', '/org/freedesktop/DBus',
               'org.freedesktop.DBus', 'NameHasOwner', 's', bus], cancelled)
    return json.loads(out)['data'][0] is True

def ensure(bus, program, cancelled):
    if running(bus, cancelled):
        return
    proc = subprocess.Popen([program], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    # Reaped when the app quits, so no defunct process lingers.
    threading.Thread(target=proc.wait, daemon=True).start()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if cancelled.is_set():
            raise ValueError('cancelled')
        time.sleep(.25)
        if running(bus, cancelled):
            time.sleep(.5)  # the window's first tab or document
            return
    raise ValueError(f'{program} did not start')

def handle(name, args, cancelled):
    bus, program = APPS[name.split('.', 1)[0]]
    ensure(bus, program, cancelled)
    path = '/' + bus.replace('.', '/')
    args = {k: v for k, v in args.items() if k != 'request'}
    out = run(['busctl', '--user', '--json=short', 'call', bus, path, IFACE, 'Call', 'ss', name, json.dumps(args)], cancelled)
    reply = json.loads(json.loads(out)['data'][0])
    if 'error' in reply:
        raise ValueError(reply['error'])
    return reply['ok']

if __name__ == '__main__': Plugin('nixe-apps', request_tools(TOOLS), handle).run()
