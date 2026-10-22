"""Nixe Mail, worked through its control interface: search and read mail, show a
conversation, and open a filled-in message for the user to check and send. There is no
sending tool: mail goes out only when the user presses Send in Mail. Message text comes
back marked untrusted."""
from assistant_plugin import Plugin
from desktop_plugin import request_tools
import nixe_app

APP = ('io.github.design_nexus.nixe.Mail', 'nixe-mail')
NAMES = {'mail.search', 'mail.inbox', 'mail.read', 'mail.open', 'mail.draft'}
TEXT = {'type': 'string', 'maxLength': 2000}
ID = {'type': 'integer', 'minimum': 1}

def schema(properties, required=()):
    return {'type': 'object', 'properties': properties, 'required': list(required)}

# Offered while Mail isn't installed (the host wants every declared tool); calls say so.
STAND_IN = [
    {'name': 'mail.search', 'description': 'Search mail (needs Nixe Mail).', 'inputSchema': schema({'query': TEXT}, ['query'])},
    {'name': 'mail.inbox', 'description': 'The newest inbox conversations (needs Nixe Mail).', 'inputSchema': schema({'unread': {'type': 'boolean'}})},
    {'name': 'mail.read', 'description': 'Read a message (needs Nixe Mail).', 'inputSchema': schema({'id': ID}, ['id'])},
    {'name': 'mail.open', 'description': 'Show a conversation (needs Nixe Mail).', 'inputSchema': schema({'id': ID}, ['id'])},
    {'name': 'mail.draft', 'description': 'Open a filled-in message for the user to send (needs Nixe Mail).', 'inputSchema': schema({'to': TEXT, 'subject': TEXT, 'body': {'type': 'string', 'maxLength': 20000}})},
]
FROM_APP = nixe_app.describe(APP[1], NAMES)

def handle(name, args, cancelled):
    if FROM_APP is None:
        raise ValueError('Nixe Mail is not installed')
    return nixe_app.call(*APP, name, args, cancelled)

if __name__ == '__main__': Plugin('mail', request_tools(FROM_APP or STAND_IN), handle).run()
