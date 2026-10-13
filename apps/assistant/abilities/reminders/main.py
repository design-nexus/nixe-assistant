import datetime as dt
import re
import sqlite3
import threading
import time
import uuid
from zoneinfo import ZoneInfo
from assistant_plugin import Plugin, tool, data_path

TEXT = {'type': 'string', 'minLength': 1, 'maxLength': 4096}
ID = {'type': 'string', 'minLength': 1, 'maxLength': 128}
SECONDS = {'type': 'integer', 'minimum': 1, 'maximum': 31536000}
TOOLS = [tool('reminders.timer', 'Set a timer and acknowledge the exact deadline', {'seconds': SECONDS, 'text': TEXT}, ['seconds']),
         tool('reminders.create', 'Create one-time reminder with timezone-aware RFC3339 deadline. Ask about ambiguous local times.', {'deadline': TEXT, 'text': TEXT}, ['deadline', 'text']),
         tool('reminders.list', 'List pending reminders'),
         tool('reminders.cancel', 'Cancel a reminder', {'id': ID}, ['id']),
         tool('reminders.snooze', 'Snooze a reminder', {'id': ID, 'seconds': SECONDS}, ['id', 'seconds']),
         tool('reminders.voice', 'Parse an explicit timer or reminder phrase; return clarification for ambiguous times', {'request': TEXT}, ['request'])]

def connect():
    db = sqlite3.connect(data_path('reminders.sqlite3'), timeout=5)
    db.row_factory = sqlite3.Row
    db.execute('PRAGMA journal_mode=WAL')
    db.execute('CREATE TABLE IF NOT EXISTS reminders(id TEXT PRIMARY KEY,text TEXT NOT NULL,deadline REAL NOT NULL,resolved TEXT NOT NULL,status TEXT NOT NULL DEFAULT "pending")')
    return db

def timezone():
    try:
        with open('/etc/timezone') as f:
            return ZoneInfo(f.read().strip())
    except FileNotFoundError:
        import os
        name = os.path.realpath('/etc/localtime').split('/zoneinfo/')[-1]
        return ZoneInfo(name)

NUMBERS = {'one': 1, 'two': 2, 'three': 3, 'four': 4, 'five': 5, 'six': 6, 'seven': 7, 'eight': 8, 'nine': 9, 'ten': 10, 'fifteen': 15, 'twenty': 20, 'thirty': 30, 'sixty': 60}
def number(raw):
    return int(raw) if raw.isdigit() else NUMBERS.get(raw)

def parse(request, now=None):
    now = now or dt.datetime.now(timezone())
    request = request.strip()
    if re.fullmatch(r'list (?:my )?(?:timers|reminders)', request, re.I):
        return {'operation': 'reminders.list'}
    match = re.fullmatch(r'cancel (?:timer|reminder) ([a-zA-Z0-9-]+)', request, re.I)
    if match:
        return {'operation': 'reminders.cancel', 'id': match[1]}
    match = re.fullmatch(r'snooze (?:timer|reminder) ([a-zA-Z0-9-]+) for (\w+) (second|minute|hour)s?', request, re.I)
    if match and number(match[2].lower()):
        return {'operation': 'reminders.snooze', 'id': match[1], 'seconds': number(match[2].lower()) * {'second': 1, 'minute': 60, 'hour': 3600}[match[3].lower()]}
    match = re.fullmatch(r'(?:set (?:a |an |the )?timer for|remind me in) (\w+) (second|minute|hour)s?(?: to (.+))?', request, re.I)
    if match:
        count = number(match[1].lower())
        if count:
            return {'seconds': count * {'second': 1, 'minute': 60, 'hour': 3600}[match[2].lower()], 'text': match[3] or 'Timer finished'}
    match = re.fullmatch(r'remind me (today|tomorrow) at (\w+)(?::(\d{2}))?\s*(am|pm)? to (.+)', request, re.I)
    if match:
        hour = number(match[2].lower())
        if hour is None or not match[4]:
            return {'clarification': 'Do you mean AM or PM? Please repeat the reminder with AM or PM.'}
        if not 1 <= hour <= 12:
            return {'clarification': 'Please specify a time from 1 to 12 with AM or PM.'}
        hour = hour % 12 + (12 if match[4].lower() == 'pm' else 0)
        day = now.date() + dt.timedelta(days=match[1].lower() == 'tomorrow')
        local = dt.datetime.combine(day, dt.time(hour, int(match[3] or 0)), now.tzinfo)
        if local.replace(fold=0).utcoffset() != local.replace(fold=1).utcoffset():
            return {'clarification': 'That time changes with daylight saving. Please give an explicit date and UTC offset.'}
        if local.astimezone(dt.timezone.utc).astimezone(now.tzinfo).replace(tzinfo=None) != local.replace(tzinfo=None):
            return {'clarification': 'That local time does not exist. Please choose another time.'}
        return {'deadline': local.isoformat(), 'text': match[5]}
    return {'clarification': 'Please specify a duration, or a date and time with AM or PM. Recurring reminders are not supported.'}

PENDING = None
PENDING_LOCK = threading.Lock()

def handle(name, args, cancelled):
    global PENDING
    if cancelled.is_set():
        raise ValueError('cancelled')
    if name == 'reminders.voice':
        request = args['request']
        followup = request.strip().lower().replace(' ', '')
        with PENDING_LOCK:
            if followup in ('am', 'pm'):
                if PENDING is None:
                    return {'clarification': 'Please repeat the full reminder with a time and AM or PM.'}
                request = re.sub(r'(at \w+(?::\d{2})?)(\s+to )', r'\1 ' + followup + r'\2', PENDING, flags=re.I)
            args = parse(request)
            if 'clarification' in args:
                if 'AM or PM' in args['clarification']:
                    PENDING = request
                return args
            PENDING = None
        name = args.pop('operation', 'reminders.timer' if 'seconds' in args else 'reminders.create')
    db = connect()
    try:
        with db:
            if name == 'reminders.list':
                return {'reminders': [{**dict(r), 'text': r['text'][:512]} for r in db.execute('SELECT * FROM reminders WHERE status="pending" ORDER BY deadline LIMIT 100')]}
            if name == 'reminders.cancel':
                changed = db.execute('UPDATE reminders SET status="cancelled" WHERE id=?', (args['id'],)).rowcount
                if not changed:
                    raise ValueError('reminder not found')
                return {'id': args['id'], 'cancelled': True}
            if name in ('reminders.timer', 'reminders.snooze'):
                deadline = dt.datetime.now(timezone()) + dt.timedelta(seconds=args['seconds'])
            else:
                deadline = dt.datetime.fromisoformat(args['deadline'].replace('Z', '+00:00'))
                if deadline.tzinfo is None:
                    raise ValueError('timezone/UTC offset required')
            if deadline.timestamp() <= time.time():
                raise ValueError('deadline must be in the future')
            ident = args['id'] if name == 'reminders.snooze' else str(uuid.uuid4())
            if name == 'reminders.snooze':
                changed = db.execute('UPDATE reminders SET deadline=?,resolved=?,status="pending" WHERE id=?', (deadline.timestamp(), deadline.isoformat(), ident)).rowcount
                if not changed:
                    raise ValueError('reminder not found')
            else:
                db.execute('INSERT INTO reminders VALUES(?,?,?,?,"pending")', (ident, args.get('text', 'Timer finished'), deadline.timestamp(), deadline.isoformat()))
            return {'id': ident, 'deadline': deadline.isoformat(), 'acknowledgement': f'Reminder set for {deadline.strftime("%A %B %d at %I:%M %p %Z")}.'}
    finally:
        db.close()

class Delivery:
    def __init__(self, plugin):
        self.plugin = plugin
        self.started = time.time()
        self.overdue = None
        self.last = {}
        self.lock = threading.Lock()
    def ack(self, ident):
        db = connect()
        try:
            with self.lock, db:
                ids = self.overdue if ident == 'overdue' else [ident]
                for item in ids or []:
                    db.execute('UPDATE reminders SET status="delivered" WHERE id=? AND status="pending" AND deadline<=?', (item, time.time()))
            return {'acknowledged': ident}
        finally:
            db.close()
    def tick(self):
        db = connect()
        try:
            with self.lock:
                if self.overdue is None:
                    self.overdue = [r['id'] for r in db.execute('SELECT id FROM reminders WHERE status="pending" AND deadline<=?', (self.started,))]
                overdue = [dict(r) for r in db.execute('SELECT * FROM reminders WHERE status="pending" AND deadline<=?', (self.started,)) if r['id'] in self.overdue]
                due = [dict(r) for r in db.execute('SELECT * FROM reminders WHERE status="pending" AND deadline>? AND deadline<=?', (self.started, time.time()))]
            events = [('overdue', f'{len(overdue)} overdue reminders: ' + '; '.join(r['text'][:120] for r in overdue[:5]), None)] if overdue else []
            events += [(r['id'], r['text'], r['text']) for r in due]
            for ident, text, speech in events:
                if time.time() - self.last.get(ident, 0) >= 30:
                    self.plugin.event(ident, 'Reminders', text, speech)
                    self.last[ident] = time.time()
        finally:
            db.close()
    def run(self):
        while True:
            self.tick()
            time.sleep(1)

if __name__ == '__main__':
    plugin = Plugin('reminders', TOOLS, handle)
    delivery = Delivery(plugin)
    plugin.event_ack = delivery.ack
    threading.Thread(target=delivery.run, daemon=True).start()
    plugin.run()
