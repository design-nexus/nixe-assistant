"""The Nixe apps, worked directly through their control interface on the session bus
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
    'music': ('io.github.design_nexus.nixe.Music', 'nixe-music'),
    'contacts': ('io.github.design_nexus.nixe.Contacts', 'nixe-contacts'),
    'archive': ('io.github.design_nexus.nixe.Archive', 'nixe-archive'),
    'chat': ('io.github.design_nexus.nixe.Chat', 'nixe-chat'),
    'fonts': ('io.github.design_nexus.nixe.Fonts', 'nixe-fonts'),
    'photos': ('io.github.design_nexus.nixe.Photos', 'nixe-photos'),
    'video': ('io.github.design_nexus.nixe.MediaPlayer', 'nixe-media-player'),
    'pdf': ('io.github.design_nexus.nixe.Pdf', 'nixe-pdf'),
    'settings': ('io.github.design_nexus.nixe.Settings', 'nixe-settings'),
    'torrents': ('io.github.design_nexus.nixe.Torrents', 'nixe-torrents'),
}
# Answered by an app's command line, with no window: Glimpse's file search, Store's
# catalogue and Tasks' sampler.
HEADLESS = {'glimpse.search', 'store.search', 'tasks.top'}
# Shown by starting the app with arguments (its window comes up on that page).
SHOWN = {'glimpse.show', 'store.open', 'tasks.open'}
WHAT = {'enum': ['track', 'album', 'artist', 'playlist']}
LIST = {'type': 'array', 'items': {'type': 'string', 'maxLength': 500}, 'maxItems': 20}
SHORT = {'type': 'string', 'maxLength': 2000}
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
    tool('music.search', 'Search the music library: tracks, albums, artists and playlists matching the words.', {'query': TEXT}, ['query']),
    tool('music.play', 'Play a track, album, artist or playlist from the library (by name), optionally shuffled.',
         {'what': WHAT, 'name': TEXT, 'shuffle': {'type': 'boolean'}}, ['what', 'name']),
    tool('music.queue', 'Add a track, album, artist or playlist to the queue: next, or at the end.',
         {'what': WHAT, 'name': TEXT, 'next': {'type': 'boolean'}}, ['what', 'name']),
    tool('music.status', "What Music is playing: track, artist, album, playing or paused, shuffle, repeat, what's next."),
    tool('music.control', 'Play, pause, toggle, next, previous or stop in Music; or set shuffle on/off.',
         {'action': {'enum': ['play', 'pause', 'toggle', 'next', 'previous', 'stop', 'shuffle_on', 'shuffle_off']}}, ['action']),
    tool('archive.list', "List what's inside an archive file (zip, 7z, tar...) in the home folder.", {'path': PATH_ARG}, ['path']),
    tool('archive.open', "Show an archive's contents in the Archive window.", {'path': PATH_ARG}, ['path']),
    tool('archive.extract', 'Extract an archive into a folder in the home folder (by default a new folder beside it, named after it).',
         {'path': PATH_ARG, 'to': PATH_ARG, 'overwrite': {'type': 'boolean'}}, ['path']),
    tool('archive.create', 'Pack files or folders (in the home folder) into a new archive; the format follows the name (.7z, .zip, .tar.gz, .tar.xz).',
         {'files': {'type': 'array', 'items': PATH_ARG, 'maxItems': 500}, 'path': PATH_ARG, 'overwrite': {'type': 'boolean'}}, ['files', 'path']),
    tool('chat.list', 'The most recent conversations in Chat: id, title, model.'),
    tool('chat.search', "Find Chat conversations whose title or messages match the words.", {'query': TEXT}, ['query']),
    tool('chat.read', 'Read a Chat conversation (by id; the open one if none): its last messages.', {'id': {'type': 'integer'}}),
    tool('chat.open', 'Show a conversation in the Chat window.', {'id': {'type': 'integer'}}, ['id']),
    tool('chat.send', 'Send a message in Chat, in a new conversation or the open one; read the answer later with chat.read.',
         {'text': TEXT, 'new': {'type': 'boolean'}}, ['text']),
    tool('fonts.installed', 'The font families on this computer, and the ones Fonts installed.', {'query': SHORT}),
    tool('fonts.search', 'Search Google Fonts and Nerd Fonts by name or kind (serif, monospace...).', {'query': TEXT, 'source': {'enum': ['google', 'nerd']}}, ['query']),
    tool('fonts.install', 'Download and install a Google Font or Nerd Font family (by name) for this user.', {'family': SHORT, 'source': {'enum': ['google', 'nerd']}}, ['family']),
    tool('photos.search', 'Find photos and videos in the Photos library by words (file name, camera), year, month, album or favourites.',
         {'query': SHORT, 'year': {'type': 'integer'}, 'month': {'type': 'integer', 'minimum': 1, 'maximum': 12}, 'album': SHORT, 'favorites': {'type': 'boolean'}}),
    tool('photos.albums', 'The albums in Photos, with how many photos each holds.'),
    tool('photos.open', 'Show a photo (by its path) in the Photos viewer.', {'path': PATH_ARG}, ['path']),
    tool('glimpse.search', "Search the files Glimpse has indexed (the person's folders) by name.", {'query': TEXT}, ['query']),
    tool('glimpse.show', 'Open the Glimpse launcher with a search typed in.', {'query': SHORT}),
    tool('video.search', 'Search the Media Player library: movies, shows and episodes matching the words.', {'query': TEXT}, ['query']),
    tool('video.play', 'Play a movie, a show (its next episode), an episode or a playlist from the Media Player library, by name.',
         {'what': {'enum': ['movie', 'show', 'episode', 'playlist']}, 'name': TEXT}, ['what', 'name']),
    tool('video.status', 'What Media Player is playing: title, show, position, playing or paused.'),
    tool('video.control', 'Play, pause, toggle, next, previous or stop in Media Player.',
         {'action': {'enum': ['play', 'pause', 'toggle', 'next', 'previous', 'stop']}}, ['action']),
    tool('pdf.open', 'Open a PDF (inside the home folder) in the PDF window, optionally at a page (from 1).',
         {'path': PATH_ARG, 'page': {'type': 'integer', 'minimum': 1}}, ['path']),
    tool('pdf.read', 'Read the text of the open PDF: a page (from 1; the one showing if none) and up to 4 after it.',
         {'page': {'type': 'integer', 'minimum': 1}, 'pages': {'type': 'integer', 'minimum': 1, 'maximum': 5}}),
    tool('pdf.search', 'Find words in the open PDF: the pages they are on and how often.', {'text': TEXT}, ['text']),
    tool('pdf.go_to', 'Turn the open PDF to a page (from 1).', {'page': {'type': 'integer', 'minimum': 1}}, ['page']),
    tool('settings.pages', 'The pages of Nixe Settings (id, title, what each covers).', {'query': SHORT}),
    tool('settings.open', 'Show a page of Nixe Settings (by id or title from settings.pages) for the person to change something themselves.', {'page': SHORT}, ['page']),
    tool('store.search', 'Search the Store catalogue for apps and packages (name, version, whether installed).', {'query': TEXT}, ['query']),
    tool('store.open', "Show the Store with a search typed in, for the person to install or remove things themselves.", {'query': SHORT}, ['query']),
    tool('tasks.top', 'The busiest processes (by cpu or memory) and the CPU, memory and graphics totals.', {'by': {'enum': ['cpu', 'memory']}}),
    tool('tasks.open', 'Show a page of the Tasks system monitor (processes, cpu, memory, disks, network, gpu).', {'page': SHORT}),
    tool('torrents.list', 'The torrents in Torrents: name, progress, state, speeds, time left.'),
    tool('torrents.add', 'Add a torrent from a magnet link, an http(s) link to a .torrent, or an info-hash.', {'link': {'type': 'string', 'maxLength': 8192}}, ['link']),
    tool('torrents.pause', 'Pause a torrent (by name, or all).', {'name': SHORT, 'all': {'type': 'boolean'}}),
    tool('torrents.resume', 'Resume a paused torrent (by name, or all).', {'name': SHORT, 'all': {'type': 'boolean'}}),
    tool('contacts.find', 'Find contacts by name, company, email or phone: their details.', {'query': TEXT}, ['query']),
    tool('contacts.add', 'Add a new contact.', {'first': SHORT, 'last': SHORT, 'company': SHORT, 'title': SHORT, 'emails': LIST,
         'phones': LIST, 'birthday': SHORT, 'notes': SHORT}, ['first']),
    tool('contacts.update', "Change a contact's details (by id from contacts.find) or add an email or phone; nothing is removed.",
         {'id': SHORT, 'first': SHORT, 'last': SHORT, 'company': SHORT, 'title': SHORT, 'birthday': SHORT, 'notes': SHORT,
          'add_email': SHORT, 'add_phone': SHORT, 'favorite': {'type': 'boolean'}}, ['id']),
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

def page_name(text):
    if not text or not all(c.isalnum() or c in '-_' for c in text) or len(text) > 40:
        raise ValueError('give a page name like processes or cpu')
    return text

def headless(name, args, cancelled):
    if name == 'glimpse.search':
        out = run(['nixe-glimpse', 'search', args['query']], cancelled, timeout=20)
        hits = json.loads(out)
        return {'files': [{'name': h.get('name'), 'path': h.get('path')} for h in hits[:30]], 'untrusted_content': True}
    if name == 'store.search':
        return json.loads(run(['nixe-store', '--find', args['query']], cancelled, timeout=60))
    if name == 'tasks.top':
        return json.loads(run(['nixe-tasks', '--top', 'memory' if args.get('by') == 'memory' else 'cpu'], cancelled, timeout=20))
    raise ValueError(f'unknown tool {name}')

def show(name, args):
    if name == 'glimpse.show':
        argv = ['nixe-glimpse'] + (['--query', args['query']] if args.get('query') else [])
    elif name == 'store.open':
        argv = ['nixe-store', '--search', args['query']]
    else:
        argv = ['nixe-tasks'] + (['--section', page_name(args['page'])] if args.get('page') else [])
    proc = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    threading.Thread(target=proc.wait, daemon=True).start()
    return {'shown': argv[0]}

def handle(name, args, cancelled):
    args = {k: v for k, v in args.items() if k != 'request'}
    if name in HEADLESS:
        return headless(name, args, cancelled)
    if name in SHOWN:
        return show(name, args)
    bus, program = APPS[name.split('.', 1)[0]]
    ensure(bus, program, cancelled)
    path = '/' + bus.replace('.', '/')
    out = run(['busctl', '--user', '--json=short', 'call', bus, path, IFACE, 'Call', 'ss', name, json.dumps(args)], cancelled)
    reply = json.loads(json.loads(out)['data'][0])
    if 'error' in reply:
        raise ValueError(reply['error'])
    return reply['ok']

if __name__ == '__main__': Plugin('nixe-apps', request_tools(TOOLS), handle).run()
