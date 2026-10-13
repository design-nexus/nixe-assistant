import datetime as dt
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from zoneinfo import ZoneInfo
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'plugin-sdk'))
def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'abilities' / name / 'main.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result
notes, reminders = module('notes'), module('reminders')
class Abilities(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        os.environ['ASSISTANT_PLUGIN_DATA'] = self.temp.name
        self.token = threading.Event()
    def tearDown(self):
        self.temp.cleanup()
    def test_notes_search_edit_export_and_persistence(self):
        ident = notes.handle('notes.create', {'text': 'Project violet\nremember batteries'}, self.token)['id']
        self.assertEqual(notes.handle('notes.search', {'query': 'batteries'}, self.token)['notes'][0]['id'], ident)
        notes.handle('notes.edit', {'id': ident, 'text': 'Project violet: charge camera'}, self.token)
        self.assertEqual(notes.handle('notes.search', {'query': 'batteries'}, self.token)['notes'], [])
        self.assertEqual(notes.handle('notes.search', {'query': 'camera'}, self.token)['notes'][0]['id'], ident)
        self.assertIn('charge camera', Path(notes.handle('notes.export', {}, self.token)['path']).read_text())
        notes.handle('notes.delete', {'id': ident}, self.token)
        self.assertEqual(notes.handle('notes.list', {}, self.token)['notes'], [])
    def test_timezone_clarification_and_dst(self):
        now = dt.datetime(2026, 10, 7, 12, tzinfo=ZoneInfo('America/Chicago'))
        self.assertIn('clarification', reminders.parse('remind me tomorrow at nine to call', now))
        parsed = reminders.parse('remind me tomorrow at nine am to call', now)
        self.assertEqual(parsed['deadline'], '2026-10-08T09:00:00-05:00')
        self.assertEqual(reminders.parse('set a timer for ten minutes', now)['seconds'], 600)
        dst = dt.datetime(2026, 10, 31, 12, tzinfo=ZoneInfo('America/Chicago'))
        self.assertIn('clarification', reminders.parse('remind me tomorrow at one am to call', dst))
        with self.assertRaises(ValueError):
            reminders.handle('reminders.create', {'deadline': '2027-01-01T09:00:00', 'text': 'call'}, self.token)
    def test_ambiguous_time_voice_followup(self):
        question = reminders.handle('reminders.voice', {'request': 'remind me tomorrow at nine to call'}, self.token)
        self.assertIn('clarification', question)
        result = reminders.handle('reminders.voice', {'request': 'AM'}, self.token)
        self.assertIn('deadline', result)
        self.assertIn('acknowledgement', result)
        reminders.handle('reminders.cancel', {'id': result['id']}, self.token)

    def test_reminders_persist_summary_retry_deduplicate_and_snooze(self):
        one = reminders.handle('reminders.timer', {'seconds': 600, 'text': 'one'}, self.token)['id']
        two = reminders.handle('reminders.timer', {'seconds': 600, 'text': 'two'}, self.token)['id']
        with reminders.connect() as db:
            db.execute('UPDATE reminders SET deadline=?', (time.time() - 10,))
        db.close()
        events = []
        class Sink:
            def event(self, *args):
                events.append(args)
        delivery = reminders.Delivery(Sink())
        delivery.tick()
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0][0], 'overdue')
        self.assertIsNone(events[0][3])
        # No ack means pending delivery survives restart/service failure.
        self.assertEqual(len(reminders.handle('reminders.list', {}, self.token)['reminders']), 2)
        delivery.ack('overdue')
        reminders.Delivery(Sink()).tick()
        self.assertEqual(len(events), 1)
        self.assertEqual(reminders.handle('reminders.list', {}, self.token)['reminders'], [])
        reminders.handle('reminders.snooze', {'id': one, 'seconds': 120}, self.token)
        self.assertEqual(len(reminders.handle('reminders.list', {}, self.token)['reminders']), 1)
        reminders.handle('reminders.cancel', {'id': one}, self.token)
        self.assertEqual(reminders.handle('reminders.list', {}, self.token)['reminders'], [])
if __name__ == '__main__':
    unittest.main()
