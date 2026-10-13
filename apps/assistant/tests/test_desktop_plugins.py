import datetime as dt
import json
import subprocess
import time
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'plugin-sdk'))
def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT/'abilities'/name/'main.py')
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result)
    return result
routines, projects, calendar, search, capture, media, power = [module(name) for name in ['routines','projects','calendar','file-search','capture','media','power']]
notes = module('notes')
class DesktopPlugins(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        os.environ['ASSISTANT_PLUGIN_DATA'] = self.temp.name
        self.token = threading.Event()
    def tearDown(self): self.temp.cleanup()
    def test_routine_preview_digest_and_undo_conflict(self):
        steps = [{'tool':'notes.create','arguments':{'text':'hello'}}]
        saved = routines.handle('routines.save', {'name':'coding','steps':steps}, self.token)
        preview = routines.handle('routines.preview', {'name':'coding'}, self.token)
        result = routines.handle('routines.run', {'name':'coding','expected_digest':preview['digest']}, self.token)
        self.assertEqual(result['host_actions']['steps'], steps)
        changed = [{'tool':'clock.now','arguments':{}}]
        routines.handle('routines.save', {'name':'coding','steps':changed}, self.token)
        with self.assertRaises(ValueError): routines.handle('routines.run', {'name':'coding','expected_digest':preview['digest']}, self.token)
        with self.assertRaises(ValueError): routines.handle('routines.undo', saved['undo']['arguments'], self.token)
        with self.assertRaises(ValueError): routines.handle('routines.save', {'name':'bad','steps':[{'tool':'routines.run','arguments':{'name':'bad'}}]}, self.token)
    def test_notes_undo_delete_search_and_conflict(self):
        created = notes.handle('notes.create', {'text':'violet camera'}, self.token)
        edited = notes.handle('notes.edit', {'id':created['id'],'text':'violet microphone'}, self.token)
        with self.assertRaises(ValueError): notes.handle('notes.undo', created['undo']['arguments'], self.token)
        notes.handle('notes.undo', edited['undo']['arguments'], self.token)
        self.assertEqual(notes.handle('notes.search', {'query':'microphone'}, self.token)['notes'], [])
        self.assertEqual(notes.handle('notes.search', {'query':'camera'}, self.token)['notes'][0]['id'], created['id'])
        deleted = notes.handle('notes.delete', {'id':created['id']}, self.token)
        notes.handle('notes.undo', deleted['undo']['arguments'], self.token)
        self.assertEqual(notes.handle('notes.read', {'id':created['id']}, self.token)['text'], 'violet camera')
        with self.assertRaises(ValueError): notes.handle('notes.undo', deleted['undo']['arguments'], self.token)
    def test_project_context_exact_argument_array(self):
        with tempfile.TemporaryDirectory(dir=Path.home()) as folder:
            projects.handle('projects.save', {'name':'demo','directory':folder,'commands':{'tests':['cargo','test']},'urls':['https://example.com']}, self.token)
            projects.handle('projects.use', {'name':'demo'}, self.token)
            result = projects.handle('projects.action', {'action':'tests'}, self.token)
            self.assertEqual(result['host_actions']['steps'], [{'tool':'terminal.run','arguments':{'argv':['cargo','test'],'cwd':folder,'show':True}}])
            self.assertEqual(projects.handle('projects.current', {}, self.token)['name'], 'demo')
        with self.assertRaises(ValueError): projects.handle('projects.save', {'name':'bad','directory':'/etc'}, self.token)
    def test_calendar_timezone_export_and_undo(self):
        with self.assertRaises(ValueError): calendar.handle('calendar.create', {'title':'call','start':'2027-01-01T09:00:00','end':'2027-01-01T10:00:00'}, self.token)
        saved = calendar.handle('calendar.create', {'title':'Call, camera; café','start':'2027-01-01T09:00:00-06:00','end':'2027-01-01T10:00:00-06:00'}, self.token)
        with calendar.db_open() as db:
            past = {'title':'past','start':'2025-01-01T09:00:00+00:00','end':'2025-01-01T10:00:00+00:00'}
            db.executemany('INSERT INTO records VALUES(?,?)', [(str(i).zfill(4), json.dumps(past)) for i in range(101)])
        self.assertEqual(calendar.handle('calendar.next', {}, self.token)['event']['id'], saved['id'])
        exported = calendar.handle('calendar.export', {}, self.token)
        self.assertEqual(exported['count'], 102)
        text = Path(exported['path']).read_text()
        self.assertIn('DTSTART:20270101T150000Z', text)
        self.assertIn('Call\\, camera\\; café', text)
        self.assertTrue(all(len(line.encode()) <= 75 for line in text.splitlines()))
        calendar.handle('calendar.undo', saved['undo']['arguments'], self.token)
        self.assertIsNone(calendar.handle('calendar.next', {}, self.token)['event'])
    def test_search_opt_in_private_symlinks_stale_and_clear(self):
        with tempfile.TemporaryDirectory(dir=Path.home()) as folder:
            root = Path(folder); (root/'readme.md').write_text('violet searchable camera')
            (root/'.env').write_text('violet token'); (root/'.ssh').mkdir(); (root/'.ssh'/'secret.txt').write_text('violet private')
            (root/'link.txt').symlink_to(root/'.ssh'/'secret.txt')
            with patch.dict(search.SETTINGS, {}, clear=True):
                with self.assertRaises(ValueError): search.handle('file_search.index', {}, self.token)
            with patch.dict(search.SETTINGS, {'roots':[folder]}, clear=True):
                self.assertEqual(search.handle('file_search.index', {}, self.token)['indexed'], 1)
                self.assertEqual(len(search.handle('file_search.search', {'query':'violet'}, self.token)['results']), 1)
                (root/'readme.md').write_text('changed contents')
                self.assertEqual(search.handle('file_search.search', {'query':'violet'}, self.token)['results'], [])
                search.handle('file_search.clear', {}, self.token)
                self.assertEqual(search.handle('file_search.status', {}, self.token)['count'], 0)
    def test_capture_no_retained_screenshot(self):
        paths = []
        def fake(argv, cancelled, timeout=10):
            if argv[0] == 'grim':
                paths.append(Path(argv[-1])); paths[-1].write_bytes(b'fake'); return ''
            if argv[0] == 'tesseract': return 'screen words'
            return 'clipboard words'
        with patch.object(capture, 'run', fake):
            self.assertEqual(capture.handle('capture.screen', {}, self.token)['text'], 'screen words')
            self.assertFalse(paths[0].exists())
            self.assertEqual(capture.handle('capture.clipboard', {}, self.token)['text'], 'clipboard words')
    def test_media_ambiguity_exact_bus_calls(self):
        calls = []
        def fake(argv, cancelled, timeout=10):
            calls.append(argv)
            if argv[-1] == 'list': return 'org.mpris.MediaPlayer2.demo 1 x\norg.mpris.MediaPlayer2.other 2 x\n'
            return ''
        with patch.object(media, 'run', fake):
            with self.assertRaises(ValueError): media.handle('media.control', {'action':'pause'}, self.token)
            media.handle('media.control', {'action':'pause','player':'demo'}, self.token)
            self.assertEqual(calls[-1][-1], 'Pause')
            media.handle('media.volume', {'node_id':42,'percent':50}, self.token)
            self.assertEqual(calls[-1], ['wpctl','set-volume','42','0.5'])
    def test_power_is_a_host_request_not_system_mutation(self):
        with patch.object(power, 'run', side_effect=AssertionError('must not execute')):
            self.assertEqual(power.handle('power.battery', {}, self.token)['host_power']['profile'], 'battery')
            self.assertTrue(power.handle('power.unload', {}, self.token)['host_power']['unload'])
    def test_cancellation_reaps_running_helper(self):
        from desktop_plugin import run
        original = subprocess.Popen
        processes = []
        def launch(*args, **kwargs):
            proc = original(*args, **kwargs); processes.append(proc); return proc
        timer = threading.Timer(.15, self.token.set)
        started = time.monotonic(); timer.start()
        try:
            with patch('desktop_plugin.subprocess.Popen', launch):
                with self.assertRaises(ValueError): run(['sleep','30'], self.token)
        finally: timer.join()
        self.assertLess(time.monotonic()-started, 2)
        self.assertIsNotNone(processes[0].returncode)
        with self.assertRaises(ProcessLookupError): os.kill(processes[0].pid, 0)
    def test_cancellation_before_process_launch(self):
        from desktop_plugin import run
        self.token.set()
        with patch('desktop_plugin.subprocess.Popen', side_effect=AssertionError('must not launch')):
            with self.assertRaises(ValueError): run(['true'], self.token)
if __name__ == '__main__': unittest.main()
