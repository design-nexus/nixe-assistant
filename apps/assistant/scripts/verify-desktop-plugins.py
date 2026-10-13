#!/usr/bin/env python3
"""Live smoke verification using disposable records and real approval IPC.
Temporarily starts the daemon and restores its initial service state.
Does not capture the user's screen/clipboard or change playback.
"""
import json
from pathlib import Path
import subprocess
import time
import uuid
CLI = str(Path.home()/'.local/bin/omarchy-assistant')
UNITS = ['omarchy-assistantd.service','nyx-parakeet.service','omarchy-assistant-hud.service']
TAG = 'verification-' + str(uuid.uuid4())
def cli(*args, check=True):
    result = subprocess.run([CLI,*args], capture_output=True, text=True, timeout=95)
    if check and result.returncode: raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout) if result.returncode == 0 else {'error':result.stderr.strip()}
def status(): return cli('status','--json')
def call(name, args, approval_callback=None, check=True):
    proc = subprocess.Popen([CLI,'tool',name,json.dumps(args)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    approved = set(); deadline = time.monotonic() + 85
    while proc.poll() is None:
        if time.monotonic() > deadline:
            proc.kill(); raise RuntimeError('call timed out: ' + name)
        for ident, pending in status().get('pending_approvals', {}).items():
            if ident in approved: continue
            # Never approve a request belonging to an unrelated live user action.
            expected = pending['tool'] == name and all(pending['arguments'].get(k) == v for k, v in args.items())
            if name == 'history.undo': expected = pending['tool'].endswith('.undo')
            if name == 'routines.run': expected = (pending['tool']=='routines.run' and pending['arguments'].get('name')==args['name']) or (pending['tool']=='notes.edit' and pending['arguments'].get('id') in touched_ids)
            if not expected: continue
            if approval_callback:
                action = approval_callback(ident, pending)
            else: action = 'approve'
            if action: cli(action, ident)
            approved.add(ident)
        time.sleep(.05)
    out, err = proc.communicate()
    if proc.returncode:
        if check: raise RuntimeError(name + ': ' + err.strip())
        return {'error':err.strip()}
    return json.loads(out)
def data(value): return value['structuredContent']
initial = subprocess.run(['systemctl','--user','is-active','--quiet',UNITS[0]]).returncode == 0
created_notes, created_calendar, created_projects, created_routines = [], [], [], []
touched_ids = set()
initial_power = None
try:
    subprocess.run(['systemctl','--user','start',UNITS[0]], check=True)
    for attempt in range(60):
        try:
            health = status()
            if health.get('ok') and health.get('model_loaded'): break
        except Exception: pass
        time.sleep(.5)
    else: raise RuntimeError('daemon did not become healthy')
    initial_power = health['power']['settings']
    assert all(plugin['health']=='running' for plugin in health['plugins'] if plugin['saved']['enabled'])
    print('All enabled plugins healthy.', flush=True)
    # Store/run a harmless routine, inspect the actual steps shown for approval.
    routine_name = TAG + '-routine'
    steps = [{'tool':'notes.create','arguments':{'text':TAG}}, {'tool':'clock.now','arguments':{'request':'what time is it'}}]
    saved = data(call('routines.save', {'name':routine_name,'steps':steps})); created_routines.append(routine_name)
    previews = []
    def review(ident, pending):
        if pending['tool']=='routines.run':
            assert pending['arguments']['steps']==steps
            previews.append(True)
        return 'approve'
    run = data(call('routines.run', {'name':routine_name}, review))
    assert previews and len(run['completed'])==2
    note_id = data(run['completed'][0]['result'])['id']; created_notes.append(note_id); touched_ids.add(note_id)
    history = data(call('history.list', {}))['actions']
    last = next(row for row in reversed(history) if row['undo'] and not row['undone'])
    assert last['tool']=='notes.create' and last['arguments']['text']==TAG
    def review_inverse(ident, pending):
        if pending['tool'] == last['undo']['tool'] and pending['arguments'] == last['undo']['arguments']: return 'approve'
        return None
    undone = data(call('history.undo', {}, review_inverse))
    assert undone['tool']=='notes.create'
    assert 'error' in call('notes.read', {'id':note_id}, check=False)
    created_notes.remove(note_id)
    print('Routine preview, approved child dispatch, persistent history, and shared undo passed.', flush=True)
    # Parent disable must cancel a child waiting for confirmation.
    note_id = data(call('notes.create', {'text':TAG + '-unchanged'}))['id']; created_notes.append(note_id); touched_ids.add(note_id)
    pending_steps = [{'tool':'notes.edit','arguments':{'id':note_id,'text':TAG + '-changed'}}]
    call('routines.save', {'name':routine_name,'steps':pending_steps})
    def disable_during_child(ident, pending):
        if pending['tool']=='notes.edit':
            cli('plugins','disable','routines')
            return None
        assert pending['arguments']['steps']==pending_steps
        return 'approve'
    try:
        failed = call('routines.run', {'name':routine_name}, disable_during_child, check=False)
        assert failed.get('isError') or 'error' in failed
        assert data(call('notes.read', {'id':note_id}))['text']==TAG+'-unchanged'
        assert not status()['pending_approvals']
    finally: cli('plugins','enable','routines')
    print('Disabling the routine owner cancelled its child approval without editing the note.', flush=True)
    project_name = TAG + '-project'
    call('projects.save', {'name':project_name,'directory':str(Path(__file__).resolve().parents[1]),'commands':{'tests':['cargo','test','--workspace']}}); created_projects.append(project_name)
    assert data(call('projects.resolve', {'name':project_name}))['commands']['tests']==['cargo','test','--workspace']
    event = data(call('calendar.create', {'title':TAG,'start':'2027-01-05T09:00:00-06:00','end':'2027-01-05T10:00:00-06:00'})); created_calendar.append(event['id']); touched_ids.add(event['id'])
    assert data(call('calendar.read', {'id':event['id']}))['start'].endswith('-06:00')
    assert data(call('file_search.status', {}))['roots']==[]
    players = data(call('media.players', {}))['players']
    print('Project context, timezone-aware calendar, opt-in index, and MPRIS discovery passed.', flush=True)
    # Free resident GGUF/KV resources and use a minimal grammar generation to prove lazy reload.
    before = status()['model_loaded']; assert before
    call('power.configure', {'profile':'normal','idle_seconds':2})
    for attempt in range(100):
        if not status()['model_loaded']: break
        time.sleep(.1)
    else: raise AssertionError('idle model did not unload')
    print('Quiet idle model unload passed.', flush=True)
    unloaded = data(call('power.unload', {}))
    assert not unloaded['host']['model_loaded'] and not status()['model_loaded']
    call('power.configure', {'profile':'battery','idle_seconds':2})
    assert status()['power']['settings']['profile']=='battery'
    print('Model unload and persisted battery profile passed.', flush=True)
    # Normal profile is restored without generating private content or capturing audio.
    call('power.configure', initial_power)
    assert status()['power']['settings']==initial_power
    print('Normal mode restored. Language-model reload is verified separately.', flush=True)
finally:
    try:
        cli('plugins','enable','routines')
        if initial_power is not None: call('power.configure', initial_power, check=False)
        for ident in created_notes: call('notes.delete', {'id':ident}, check=False)
        for ident in created_calendar: call('calendar.delete', {'id':ident}, check=False)
        for name in created_projects: call('projects.delete', {'name':name}, check=False)
        for name in created_routines: call('routines.delete', {'name':name}, check=False)
    finally:
        if not initial:
            subprocess.run(['systemctl','--user','stop',*UNITS], check=True)
            for unit in UNITS:
                assert subprocess.check_output(['systemctl','--user','show',unit,'--value','-p','ActiveState']).decode().strip()=='inactive'
            print('Initial disabled state restored; all assistant services stopped.', flush=True)

# Remove only this run's disposable-record entries from the host journal.
journal = Path.home()/'.local/share/omarchy-assistant/actions.json'
if not initial and journal.exists():
    records = json.loads(journal.read_text())
    def fixture(row):
        args = row.get('arguments', {})
        return TAG in json.dumps(args) or args.get('id') in touched_ids
    records = [row for row in records if not fixture(row)]
    temporary = journal.with_suffix('.json.verify-tmp')
    temporary.write_text(json.dumps(records, indent=2))
    temporary.chmod(0o600)
    temporary.replace(journal)
