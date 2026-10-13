import json
import re
from assistant_plugin import Plugin, tool
from desktop_plugin import NAME, run, request_tools
TOOLS = [tool('media.players', 'List MPRIS media players'), tool('media.status', 'Read current song and playback state', {'player': NAME}),
         tool('media.control', 'Play, pause, toggle, next, previous, or stop a media player', {'player': NAME, 'action': {'type':'string','enum':['play','pause','toggle','next','previous','stop']}}, ['action']),
         tool('media.volume', 'Set a player or PipeWire application volume from zero to 100 percent', {'player': NAME, 'node_id': {'type':'integer','minimum':1}, 'percent': {'type':'number','minimum':0,'maximum':100}}, ['percent']),
         tool('media.applications', 'List PipeWire streams to select an application volume target')]
BASE = ['busctl', '--user', '--json=short']
PATH = '/org/mpris/MediaPlayer2'
IFACE = 'org.mpris.MediaPlayer2.Player'
def players(cancelled):
    text = run(['busctl', '--user', '--no-pager', '--no-legend', 'list'], cancelled)
    return [line.split()[0] for line in text.splitlines() if line.startswith('org.mpris.MediaPlayer2.')]
def choose(args, cancelled):
    names = players(cancelled)
    wanted = args.get('player')
    if wanted:
        names = [name for name in names if name == wanted or name.removeprefix('org.mpris.MediaPlayer2.') == wanted]
    if len(names) != 1: raise ValueError('select one player; available: ' + ', '.join(names))
    return names[0]
def handle(name, args, cancelled):
    if name == 'media.players': return {'players': players(cancelled)}
    if name == 'media.applications': return {'streams': run(['wpctl', 'status', '--name'], cancelled)}
    if name == 'media.volume' and 'node_id' in args:
        if args.get('player'): raise ValueError('select a player or a node, not both')
        run(['wpctl', 'set-volume', str(args['node_id']), str(args['percent']/100)], cancelled)
        return {'percent': args['percent'], 'node_id': args['node_id']}
    player = choose(args, cancelled)
    if name == 'media.control':
        method = {'play':'Play','pause':'Pause','toggle':'PlayPause','next':'Next','previous':'Previous','stop':'Stop'}[args['action']]
        run(BASE + ['call', player, PATH, IFACE, method], cancelled)
        return {'player': player, 'action': args['action']}
    if name == 'media.volume':
        run(BASE + ['set-property', player, PATH, IFACE, 'Volume', 'd', str(args['percent']/100)], cancelled)
        return {'player': player, 'percent': args['percent']}
    data = {}
    for prop in ('PlaybackStatus','Metadata','Volume'):
        data[prop] = json.loads(run(BASE + ['get-property', player, PATH, IFACE, prop], cancelled))['data']
    return {'player': player, **data, 'untrusted_content': True}
if __name__ == '__main__': Plugin('media', request_tools(TOOLS), handle).run()
