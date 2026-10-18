from assistant_plugin import Plugin, tool
from desktop_plugin import request_tools
TOOLS = [tool('history.list', 'List the host action journal and available undo actions'),
         tool('history.undo', 'Undo the latest recorded change if reversible; original owner must be enabled; requires confirmation'),
         tool('history.undo_task', "Undo every reversible change one task made, newest first (the last task if none is given); says what can't be undone",
              {'task': {'type': 'string', 'maxLength': 64}})]
def handle(name, args, cancelled):
    if cancelled.is_set(): raise ValueError('cancelled')
    if name == 'history.undo_task':
        return {'host_history': {'version': 1, 'action': 'undo_task', 'task': args.get('task')}}
    return {'host_history': {'version': 1, 'action': 'undo' if name == 'history.undo' else 'list'}}
if __name__ == '__main__': Plugin('history', request_tools(TOOLS), handle).run()
