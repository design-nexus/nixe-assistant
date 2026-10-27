"""Shared helpers for bundled desktop plugins; no shell command interpolation."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import time
import uuid
from assistant_plugin import data_path, tool

TEXT = {'type': 'string', 'minLength': 1, 'maxLength': 65536}
NAME = {'type': 'string', 'minLength': 1, 'maxLength': 128}
SETTINGS = json.loads(os.environ.get('ASSISTANT_PLUGIN_SETTINGS', '{}'))

def run(argv, cancelled, timeout=10, input=None):
    if cancelled.is_set():
        raise ValueError('cancelled')
    proc = subprocess.Popen(argv, stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    deadline = time.monotonic() + timeout
    try:
        first = True
        while True:
            if cancelled.is_set() or time.monotonic() >= deadline:
                raise ValueError('cancelled' if cancelled.is_set() else 'command timed out')
            try:
                out, err = proc.communicate(input=input if first else None, timeout=.1)
                if proc.returncode:
                    raise ValueError(err.decode(errors='replace')[:1000] or 'command failed')
                if len(out) > 1024 * 1024:
                    raise ValueError('output too large')
                return out.decode(errors='replace')
            except subprocess.TimeoutExpired:
                first = False
    finally:
        if proc.poll() is None:
            try: proc.kill()
            except ProcessLookupError: pass
        proc.wait()
        for stream in (proc.stdin, proc.stdout, proc.stderr):
            if stream is not None: stream.close()

@contextmanager
def db_open(filename='records.sqlite3'):
    db = sqlite3.connect(data_path(filename), timeout=5)
    db.row_factory = sqlite3.Row
    db.executescript('''PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS records(id TEXT PRIMARY KEY,body TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS undo(token TEXT PRIMARY KEY,id TEXT,before TEXT,after TEXT,used INTEGER NOT NULL DEFAULT 0);''')
    try:
        with db:
            yield db
    finally:
        db.close()

def get(db, ident):
    row = db.execute('SELECT body FROM records WHERE id=?', (ident,)).fetchone()
    if not row:
        raise ValueError('not found: ' + ident)
    return json.loads(row['body'])

def change(db, prefix, ident, body):
    before = db.execute('SELECT body FROM records WHERE id=?', (ident,)).fetchone()
    before = before['body'] if before else None
    after = json.dumps(body, sort_keys=True) if body is not None else None
    token = str(uuid.uuid4())
    if after is None:
        db.execute('DELETE FROM records WHERE id=?', (ident,))
    else:
        db.execute('INSERT OR REPLACE INTO records VALUES(?,?)', (ident, after))
    db.execute('INSERT INTO undo(token,id,before,after) VALUES(?,?,?,?)', (token, ident, before, after))
    return {'id': ident, 'changed': True, 'undo': {'version': 1, 'tool': prefix + '.undo', 'arguments': {'token': token}}}

def undo(db, token):
    row = db.execute('SELECT * FROM undo WHERE token=? AND used=0', (token,)).fetchone()
    if not row:
        raise ValueError('undo expired or already used')
    current = db.execute('SELECT body FROM records WHERE id=?', (row['id'],)).fetchone()
    if (current['body'] if current else None) != row['after']:
        raise ValueError('record changed since this action; refusing to overwrite later edits')
    if row['before'] is None:
        db.execute('DELETE FROM records WHERE id=?', (row['id'],))
    else:
        db.execute('INSERT OR REPLACE INTO records VALUES(?,?)', (row['id'], row['before']))
    db.execute('UPDATE undo SET used=1 WHERE token=?', (token,))
    return {'undone': True, 'id': row['id'], 'acknowledgement': 'Action undone.'}

def list_records(db):
    return [dict(id=row['id'], **json.loads(row['body'])) for row in db.execute('SELECT * FROM records ORDER BY id LIMIT 100')]

def specs(prefix):
    return [tool(prefix + '.list', 'List saved ' + prefix),
            tool(prefix + '.delete', 'Delete a saved record; confirmation required', {'name': NAME}, ['name']),
            tool(prefix + '.undo', 'Restore a recorded change if unchanged; confirmation required', {'token': NAME}, ['token'])]

def request_tools(tools):
    for item in tools:
        item['inputSchema']['properties']['request'] = {'type': 'string', 'maxLength': 65536}
    return tools

def home_path(text, directory=False):
    path = Path(text).expanduser().resolve(strict=True)
    home = Path.home().resolve()
    if not path.is_relative_to(home) or (directory and not path.is_dir()):
        raise ValueError('path must be inside HOME' + (' and a directory' if directory else ''))
    return path
