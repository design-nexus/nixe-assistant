"""Assistant plugin SDK v1: MCP stdio, cancellation, host-approved events.
Stdout belongs to the protocol. Log to stderr. No installation hooks.
"""
import asyncio
import json
import os
import sys
import threading

class Plugin:
    def __init__(self, name, tools, handler, event_ack=None):
        self.name, self.tools, self.handler = name, tools, handler
        self.event_ack = event_ack
        self.pending = {}
        self.lock = threading.Lock()

    def write(self, message):
        raw = json.dumps(message, ensure_ascii=False)
        if len(raw.encode('utf-8')) > 1024 * 1024:
            raise ValueError('response exceeds 1 MiB; return a bounded preview')
        with self.lock:
            print(raw, flush=True)

    def event(self, event_id, title, text, speech=None):
        self.write({'jsonrpc': '2.0', 'method': 'notifications/assistant/event',
                    'params': {'version': 1, 'id': event_id, 'title': title,
                               'text': text, 'speech': speech}})

    async def request(self, message, cancelled):
        ident = message['id']
        try:
            method, params = message['method'], message.get('params', {})
            if method == 'initialize':
                result = {'protocolVersion': '2025-11-25', 'capabilities': {'tools': {},
                          'experimental': {'assistant': {'version': 1}}},
                          'serverInfo': {'name': self.name, 'version': '1.0.0'}}
            elif method == 'tools/list':
                result = {'tools': self.tools}
            elif method == 'ping':
                result = {}
            elif method == 'assistant/event_ack' and self.event_ack:
                if params.get('version') != 1:
                    raise ValueError('unsupported event extension')
                result = await asyncio.to_thread(self.event_ack, params['id'])
            elif method == 'tools/call':
                tool = next((t for t in self.tools if t['name'] == params['name']), None)
                if tool is None:
                    raise ValueError('unknown tool')
                data = await asyncio.to_thread(self.handler, params['name'], params.get('arguments', {}), cancelled)
                result = {'content': [{'type': 'text', 'text': json.dumps(data, ensure_ascii=False)}],
                          'structuredContent': data, 'isError': False}
            else:
                raise ValueError('unsupported method')
            self.write({'jsonrpc': '2.0', 'id': ident, 'result': result})
        except Exception as error:
            self.write({'jsonrpc': '2.0', 'id': ident,
                        'error': {'code': -32603, 'message': str(error)}})
        finally:
            self.pending.pop(ident, None)

    async def serve(self):
        tasks = set()
        while True:
            line = await asyncio.to_thread(sys.stdin.readline, 1024 * 1024 + 1)
            if not line:
                break
            if len(line) > 1024 * 1024:
                raise ValueError('frame too large')
            message = json.loads(line)
            if message.get('method') == 'notifications/cancelled':
                token = self.pending.get(message.get('params', {}).get('requestId'))
                if token:
                    token.set()
            elif 'id' in message:
                token = threading.Event()
                self.pending[message['id']] = token
                task = asyncio.create_task(self.request(message, token))
                tasks.add(task)
                task.add_done_callback(tasks.discard)
        for token in self.pending.values():
            token.set()
        if tasks:
            await asyncio.gather(*tasks)

    def run(self):
        asyncio.run(self.serve())

def tool(name, description, properties=None, required=None):
    return {'name': name, 'description': description,
            'inputSchema': {'type': 'object', 'properties': properties or {},
                            'required': required or [], 'additionalProperties': False}}

def data_path(filename):
    root = os.environ['ASSISTANT_PLUGIN_DATA']
    os.makedirs(root, mode=0o700, exist_ok=True)
    return os.path.join(root, filename)
