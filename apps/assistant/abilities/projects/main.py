import json
from assistant_plugin import Plugin, tool, data_path
from desktop_plugin import NAME, TEXT, db_open, get, change, undo, list_records, specs, request_tools, home_path
ARGV = {'type': 'array', 'items': NAME, 'minItems': 1, 'maxItems': 128}
TOOLS = specs('projects') + [
    tool('projects.save', 'Associate a project folder, test/build argument arrays, configured app phrases, browser URLs, and coding agent', {'name': NAME, 'directory': TEXT, 'commands': {'type': 'object', 'additionalProperties': ARGV}, 'apps': {'type': 'array', 'items': NAME, 'maxItems': 4}, 'urls': {'type': 'array', 'items': TEXT, 'maxItems': 4}, 'agent': NAME}, ['name', 'directory']),
    tool('projects.use', 'Select the current project by name', {'name': NAME}, ['name']),
    tool('projects.current', 'Read selected project context'),
    tool('projects.resolve', 'Read one saved project', {'name': NAME}, ['name']),
    tool('projects.action', 'Run a saved project command, open its folder, browser URL, app, or coding agent through host approval', {'action': NAME, 'name': NAME, 'prompt': TEXT}, ['action']),
]
def selected(db):
    row = db.execute('SELECT name FROM selection WHERE id=1').fetchone()
    if not row: raise ValueError('choose a project with projects.use first')
    return row['name']
def handle(name, args, cancelled):
    if cancelled.is_set(): raise ValueError('cancelled')
    with db_open() as db:
        db.execute('CREATE TABLE IF NOT EXISTS selection(id INTEGER PRIMARY KEY,name TEXT)')
        if name == 'projects.list': return {'projects': list_records(db)}
        if name == 'projects.undo': return undo(db, args['token'])
        ident = args.get('name', '').strip().lower()
        if name == 'projects.save':
            folder = home_path(args['directory'], True)
            for url in args.get('urls', []):
                if not url.startswith(('http://', 'https://')): raise ValueError('browser URLs must use HTTP or HTTPS')
            data = {k: v for k, v in args.items() if k not in ('name', 'request')}
            data['directory'] = str(folder)
            return change(db, 'projects', ident, data)
        if name == 'projects.delete':
            get(db, ident)
            return change(db, 'projects', ident, None)
        if name == 'projects.use':
            get(db, ident)
            db.execute('INSERT OR REPLACE INTO selection VALUES(1,?)', (ident,))
            return {'current': ident, 'acknowledgement': 'Selected project ' + ident + '.'}
        ident = ident or selected(db)
        data = get(db, ident)
        folder = str(home_path(data['directory'], True))
        if name != 'projects.action': return {'name': ident, **data}
        action = args['action'].strip().lower()
        if action == 'tests' and 'tests' not in data.get('commands', {}) and 'test' in data.get('commands', {}): action = 'test'
        if action in data.get('commands', {}):
            step = {'tool': 'terminal.run', 'arguments': {'argv': data['commands'][action], 'cwd': folder, 'show': True}}
        elif action == 'terminal': step = {'tool': 'terminal.open', 'arguments': {'cwd': folder}}
        elif action == 'agent': step = {'tool': 'agent.handoff', 'arguments': {'cwd': folder, 'agent': data.get('agent', 'agy'), 'prompt': args.get('prompt', 'Help me work on this project.')}}
        elif action == 'browser' and data.get('urls'): step = {'tool': 'browser.navigate', 'arguments': {'url': data['urls'][0]}}
        elif action == 'app' and data.get('apps'): step = {'tool': 'apps.launch', 'arguments': {'phrase': data['apps'][0]}}
        else: raise ValueError('unknown project action; configure a command or app first')
        return {'project': ident, 'host_actions': {'version': 1, 'steps': [step]}}
if __name__ == '__main__': Plugin('projects', request_tools(TOOLS), handle).run()
