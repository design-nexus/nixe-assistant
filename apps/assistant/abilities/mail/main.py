"""Nyx Mail, worked through its control interface: search and read mail, show a
conversation, and open a filled-in message for the user to check and send. There is no
sending tool: mail goes out only when the user presses Send in Mail. Message text comes
back marked untrusted."""
from assistant_plugin import Plugin
from desktop_plugin import request_tools
import nyx_app

APP = ('io.github.design_nexus.nyx.Mail', 'nyx-mail')
NAMES = {'mail.search', 'mail.inbox', 'mail.read', 'mail.open', 'mail.draft'}
TEXT = {'type': 'string', 'maxLength': 2000}
ID = {'type': 'integer', 'minimum': 1}

def schema(properties, required=()):
    return {'type': 'object', 'properties': properties, 'required': list(required)}

# Offered while Mail isn't installed (the host wants every declared tool); calls say so.
STAND_IN = [
    {'name': 'mail.search', 'description': 'Search mail (needs Nyx Mail).', 'inputSchema': schema({'query': TEXT}, ['query'])},
    {'name': 'mail.inbox', 'description': 'The newest inbox conversations (needs Nyx Mail).', 'inputSchema': schema({'unread': {'type': 'boolean'}})},
    {'name': 'mail.read', 'description': 'Read a message (needs Nyx Mail).', 'inputSchema': schema({'id': ID}, ['id'])},
    {'name': 'mail.open', 'description': 'Show a conversation (needs Nyx Mail).', 'inputSchema': schema({'id': ID}, ['id'])},
    {'name': 'mail.draft', 'description': 'Open a filled-in message for the user to send (needs Nyx Mail).', 'inputSchema': schema({'to': TEXT, 'subject': TEXT, 'body': {'type': 'string', 'maxLength': 20000}})},
]
FROM_APP = nyx_app.describe(APP[1], NAMES)

def handle(name, args, cancelled):
    if FROM_APP is None:
        raise ValueError('Nyx Mail is not installed')
    return nyx_app.call(*APP, name, args, cancelled)

if __name__ == '__main__': Plugin('mail', request_tools(FROM_APP or STAND_IN), handle).run()
