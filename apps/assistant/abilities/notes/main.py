import json
import sqlite3
import uuid
from assistant_plugin import Plugin, tool, data_path

TEXT = {'type': 'string', 'minLength': 1, 'maxLength': 65536}
TITLE = {'type': 'string', 'minLength': 1, 'maxLength': 256}
ID = {'type': 'string', 'minLength': 1, 'maxLength': 128}
TOOLS = [
    tool('notes.create', 'Explicitly create a personal note, separate from conversation history', {'text': TEXT, 'title': TITLE}, ['text']),
    tool('notes.list', 'List local notes'),
    tool('notes.read', 'Read a note by stable ID', {'id': ID}, ['id']),
    tool('notes.search', 'Full text search in personal notes', {'query': TEXT}, ['query']),
    tool('notes.edit', 'Replace a note; requires host confirmation', {'id': ID, 'text': TEXT, 'title': TITLE}, ['id', 'text']),
    tool('notes.delete', 'Delete a note; requires host confirmation', {'id': ID}, ['id']),
    tool('notes.undo', 'Undo a note change only if it has not been edited since', {'token': ID}, ['token']),
    tool('notes.export', 'Export all notes as Markdown in the plugin data directory'),
]
def connect():
    db = sqlite3.connect(data_path('notes.sqlite3'), timeout=5)
    db.row_factory = sqlite3.Row
    db.executescript('''PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS notes(id TEXT PRIMARY KEY,title TEXT NOT NULL,text TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS undo(token TEXT PRIMARY KEY,id TEXT,before TEXT,after TEXT,used INTEGER DEFAULT 0);
        CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(id UNINDEXED,title,text);
        CREATE TRIGGER IF NOT EXISTS insert_note AFTER INSERT ON notes BEGIN INSERT INTO search VALUES(new.id,new.title,new.text); END;
        CREATE TRIGGER IF NOT EXISTS delete_note AFTER DELETE ON notes BEGIN DELETE FROM search WHERE id=old.id; END;
        CREATE TRIGGER IF NOT EXISTS update_note AFTER UPDATE ON notes BEGIN DELETE FROM search WHERE id=old.id; INSERT INTO search VALUES(new.id,new.title,new.text); END;''')
    return db

def save_undo(db, ident, before, after):
    token = str(uuid.uuid4())
    encode = lambda value: json.dumps(value, sort_keys=True) if value is not None else None
    db.execute('INSERT INTO undo(token,id,before,after) VALUES(?,?,?,?)', (token, ident, encode(before), encode(after)))
    return {'version': 1, 'tool': 'notes.undo', 'arguments': {'token': token}}

def handle(name, args, cancelled):
    if cancelled.is_set():
        raise ValueError('cancelled')
    db = connect()
    try:
        with db:
            if name == 'notes.undo':
                saved = db.execute('SELECT * FROM undo WHERE token=? AND used=0', (args['token'],)).fetchone()
                if not saved: raise ValueError('undo not found or already used')
                row = db.execute('SELECT * FROM notes WHERE id=?', (saved['id'],)).fetchone()
                current = json.dumps(dict(row), sort_keys=True) if row else None
                if current != saved['after']: raise ValueError('note changed since this action; refusing to overwrite later edits')
                if saved['before'] is None: db.execute('DELETE FROM notes WHERE id=?', (saved['id'],))
                else:
                    before = json.loads(saved['before'])
                    db.execute('INSERT INTO notes VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET title=excluded.title,text=excluded.text', (before['id'], before['title'], before['text']))
                db.execute('UPDATE undo SET used=1 WHERE token=?', (args['token'],))
                return {'undone': True, 'acknowledgement': 'Note change undone.'}
            if name == 'notes.create':
                ident = str(uuid.uuid4())
                db.execute('INSERT INTO notes VALUES(?,?,?)', (ident, args.get('title', args['text'].splitlines()[0][:80]), args['text']))
                return {'undo': save_undo(db, ident, None, dict(db.execute('SELECT * FROM notes WHERE id=?', (ident,)).fetchone())), 'id': ident, 'created': True, 'acknowledgement': 'Note saved.'}
            if name in ('notes.edit', 'notes.delete'):
                row = db.execute('SELECT * FROM notes WHERE id=?', (args['id'],)).fetchone()
                if not row:
                    raise ValueError('note not found')
                if name == 'notes.edit':
                    db.execute('UPDATE notes SET title=?,text=? WHERE id=?', (args.get('title', row['title']), args['text'], args['id']))
                else:
                    db.execute('DELETE FROM notes WHERE id=?', (args['id'],))
                after = db.execute('SELECT * FROM notes WHERE id=?', (args['id'],)).fetchone()
                inverse = save_undo(db, args['id'], dict(row), dict(after) if after else None)
                return {'undo': inverse, 'id': args['id'], 'changed': True, 'acknowledgement': 'Note updated.' if name == 'notes.edit' else 'Note deleted.'}
            if name == 'notes.read':
                row = db.execute('SELECT * FROM notes WHERE id=?', (args['id'],)).fetchone()
                if not row:
                    raise ValueError('note not found')
                return {**dict(row), 'acknowledgement': row['title'] + '. ' + row['text']}
            if name == 'notes.search':
                # Treat user text as literal terms, never an FTS query program.
                query = ' AND '.join('"' + word.replace('"', '""') + '"' for word in args['query'].split())
                return {'notes': [{**dict(r), 'title': r['title'][:80], 'text': r['text'][:512], 'preview': True} for r in db.execute('SELECT notes.* FROM notes JOIN search ON notes.id=search.id WHERE search MATCH ? ORDER BY rank LIMIT 100', (query,))] if query else []}
            if name == 'notes.export':
                rows = db.execute('SELECT * FROM notes ORDER BY rowid').fetchall()
                path = data_path('notes.md')
                with open(path, 'w', encoding='utf-8') as file:
                    for row in rows:
                        file.write(f"# {row['title']}\n\n<!-- {row['id']} -->\n\n{row['text']}\n\n")
                return {'path': path, 'count': len(rows)}
            return {'notes': [{**dict(r), 'title': r['title'][:80]} for r in db.execute('SELECT id,title FROM notes ORDER BY rowid DESC LIMIT 100')]}
    finally:
        db.close()

for spec in TOOLS:
    spec['inputSchema']['properties']['request'] = {'type': 'string'}

if __name__ == '__main__':
    Plugin('notes', TOOLS, handle).run()
