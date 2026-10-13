import hashlib
import json
from assistant_plugin import Plugin, tool
from desktop_plugin import NAME, db_open, get, change, undo, list_records, specs, request_tools
STEP = {'type': 'object', 'properties': {'tool': NAME, 'arguments': {'type': 'object'}}, 'required': ['tool', 'arguments'], 'additionalProperties': False}
TOOLS = specs('routines') + [
    tool('routines.save', 'Save or replace a named routine; review exact steps before saving', {'name': NAME, 'steps': {'type': 'array', 'items': STEP, 'minItems': 1, 'maxItems': 4}}, ['name', 'steps']),
    tool('routines.preview', 'Preview the exact saved routine steps without executing', {'name': NAME}, ['name']),
    tool('routines.run', 'Run a saved routine after review; host checks every step separately', {'name': NAME, 'expected_digest': {'type':'string','maxLength':64}}, ['name']),
]
def handle(name, args, cancelled):
    if cancelled.is_set(): raise ValueError('cancelled')
    with db_open() as db:
        if name == 'routines.list': return {'routines': list_records(db)}
        if name == 'routines.undo': return undo(db, args['token'])
        ident = args['name'].strip().lower()
        if name == 'routines.save':
            if not 1 <= len(args['steps']) <= 4: raise ValueError('one to four steps required')
            for step in args['steps']:
                if step['tool'] in ('routines.run', 'projects.action', 'history.undo') or step['tool'].startswith('power.'):
                    raise ValueError('nested routines, undo, and power controls are excluded')
            return change(db, 'routines', ident, {'steps': args['steps']})
        if name == 'routines.delete':
            get(db, ident)
            return change(db, 'routines', ident, None)
        data = get(db, ident)
        digest = hashlib.sha256(json.dumps(data['steps'], sort_keys=True).encode()).hexdigest()
        if name == 'routines.preview': return {'name': ident, **data, 'digest': digest}
        if args.get('expected_digest') != digest: raise ValueError('routine changed since preview; review it again')
        return {'name': ident, 'host_actions': {'version': 1, 'steps': data['steps']}, 'acknowledgement': 'Routine completed.'}
if __name__ == '__main__': Plugin('routines', request_tools(TOOLS), handle).run()
