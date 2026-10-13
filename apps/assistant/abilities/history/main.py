from assistant_plugin import Plugin, tool
from desktop_plugin import request_tools
TOOLS = [tool('history.list', 'List the host action journal and available undo actions'),
         tool('history.undo', 'Undo the latest recorded change if reversible; original owner must be enabled; requires confirmation')]
def handle(name, args, cancelled):
    if cancelled.is_set(): raise ValueError('cancelled')
    return {'host_history': {'version': 1, 'action': 'undo' if name == 'history.undo' else 'list'}}
if __name__ == '__main__': Plugin('history', request_tools(TOOLS), handle).run()
