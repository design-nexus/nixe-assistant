"""Local timezone-aware calendar with interoperable RFC 5545 UTC export."""
import datetime as dt
import json
from pathlib import Path
import uuid
from assistant_plugin import Plugin, tool, data_path
from desktop_plugin import NAME, TEXT, db_open, get, change, undo, list_records, request_tools
TIME = {'type': 'string', 'maxLength': 64}
TOOLS = [
    tool('calendar.list', 'List upcoming local calendar events', {'start': TIME, 'end': TIME}),
    tool('calendar.next', 'Read the next event'),
    tool('calendar.read', 'Read an event', {'id': NAME}, ['id']),
    tool('calendar.create', 'Create a local calendar event; review exact timezone-aware times before saving', {'title': NAME, 'start': TIME, 'end': TIME, 'description': TEXT, 'location': NAME}, ['title', 'start', 'end']),
    tool('calendar.edit', 'Replace event fields; confirmation required', {'id': NAME, 'title': NAME, 'start': TIME, 'end': TIME, 'description': TEXT, 'location': NAME}, ['id']),
    tool('calendar.delete', 'Delete event; confirmation required', {'id': NAME}, ['id']),
    tool('calendar.export', 'Export local calendar as .ics in plugin data directory'),
    tool('calendar.undo', 'Restore calendar change if unchanged', {'token': NAME}, ['token']),
]
def timestamp(value):
    parsed = dt.datetime.fromisoformat(value)
    if parsed.tzinfo is None or parsed.utcoffset() is None: raise ValueError('use an explicit timezone offset; ambiguous local times need clarification')
    return parsed.timestamp()
def escape(text): return text.replace('\\', '\\\\').replace('\n', '\\n').replace(';', '\\;').replace(',', '\\,').replace('\r', '')
def fold(line):
    parts, part, size = [], '', 0
    for char in line:
        n = len(char.encode())
        if size + n > 75:
            parts.append(part); part, size = ' ', 1
        part += char; size += n
    return '\r\n'.join(parts + [part])
def handle(name, args, cancelled):
    if cancelled.is_set(): raise ValueError('cancelled')
    with db_open() as db:
        if name == 'calendar.undo': return undo(db, args['token'])
        if name == 'calendar.create':
            ident = str(uuid.uuid4())
            data = {k:v for k,v in args.items() if k != 'request'}
            if timestamp(data['end']) <= timestamp(data['start']): raise ValueError('end must be after start')
            return {**change(db, 'calendar', ident, data), 'acknowledgement': f"Saved {data['title']} at {data['start']}."}
        if name in ('calendar.read', 'calendar.edit', 'calendar.delete'):
            ident = args['id']; data = get(db, ident)
            if name == 'calendar.read': return {'id': ident, **data}
            if name == 'calendar.delete': return change(db, 'calendar', ident, None)
            data.update({k:v for k,v in args.items() if k not in ('id', 'request')})
            if timestamp(data['end']) <= timestamp(data['start']): raise ValueError('end must be after start')
            return change(db, 'calendar', ident, data)
        rows = sorted([dict(id=row['id'], **json.loads(row['body'])) for row in db.execute('SELECT * FROM records')], key=lambda row: timestamp(row['start']))
        if name == 'calendar.export':
            utc = lambda value: dt.datetime.fromtimestamp(timestamp(value), dt.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
            lines = ['BEGIN:VCALENDAR', 'VERSION:2.0', 'PRODID:-//Nixe Assistant//Local Calendar 1//EN', 'CALSCALE:GREGORIAN']
            for row in rows:
                lines += ['BEGIN:VEVENT', 'UID:' + row['id'] + '@nixe-assistant.local', 'DTSTAMP:' + dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%SZ'), 'DTSTART:' + utc(row['start']), 'DTEND:' + utc(row['end']), 'SUMMARY:' + escape(row['title']), 'DESCRIPTION:' + escape(row.get('description', '')), 'LOCATION:' + escape(row.get('location', '')), 'END:VEVENT']
            path = data_path('calendar.ics')
            Path(path).write_bytes(('\r\n'.join(map(fold, lines)) + '\r\nEND:VCALENDAR\r\n').encode())
            return {'path': path, 'count': len(rows)}
        start = timestamp(args['start']) if args.get('start') else dt.datetime.now(dt.timezone.utc).timestamp()
        end = timestamp(args['end']) if args.get('end') else float('inf')
        rows = [row for row in rows if timestamp(row['end']) > start and timestamp(row['start']) < end]
        return {'event': rows[0] if rows else None} if name == 'calendar.next' else {'events': rows[:100], 'provider': 'local'}
if __name__ == '__main__': Plugin('calendar', request_tools(TOOLS), handle).run()
