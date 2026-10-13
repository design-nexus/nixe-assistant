import csv
import io
from pathlib import Path
from assistant_plugin import Plugin, tool
from desktop_plugin import run, request_tools
TOOLS = [tool('power.status', 'Inspect assistant RAM, GPU usage, battery, and host power profile'),
         tool('power.configure', 'Configure normal or battery mode and idle model unload; host owns resources', {'profile': {'type':'string','enum':['normal','battery']}, 'idle_seconds': {'type':'integer','minimum':0,'maximum':86400}}, ['profile']),
         tool('power.battery', 'Enable battery mode with CPU language model and five-minute idle unload'),
         tool('power.normal', 'Restore configured language-model device and five-minute idle unload'),
         tool('power.unload', 'Unload the language model until next generation; host coordinates safely')]
def handle(name, args, cancelled):
    if name in ('power.battery', 'power.normal'): return {'host_power': {'version': 1, 'profile': 'battery' if name.endswith('battery') else 'normal', 'idle_seconds': 300}}
    if name == 'power.configure': return {'host_power': {'version': 1, 'profile': args['profile'], 'idle_seconds': args.get('idle_seconds', 300)}}
    if name == 'power.unload': return {'host_power': {'version': 1, 'unload': True}}
    data = {'assistant_processes': [], 'batteries': []}
    for path in Path('/proc').iterdir():
        if not path.name.isdecimal(): continue
        try:
            comm = (path/'comm').read_text().strip()
            groups = (path/'cgroup').read_text()
            if any(unit in groups for unit in ('/omarchy-assistantd.service', '/nyx-parakeet.service', '/omarchy-assistant-hud.service')):
                status = (path/'status').read_text().splitlines()
                rss = next((line.split(':',1)[1].strip() for line in status if line.startswith('VmRSS:')), '0 kB')
                data['assistant_processes'].append({'pid':int(path.name), 'name':comm, 'ram':rss})
        except (OSError, StopIteration): pass
    for battery in Path('/sys/class/power_supply').glob('BAT*'):
        try: data['batteries'].append({'name':battery.name, 'percent':int((battery/'capacity').read_text()), 'status':(battery/'status').read_text().strip()})
        except OSError: pass
    try:
        data['gpu'] = run(['nvidia-smi','--query-gpu=name,pstate,power.draw,utilization.gpu,memory.used','--format=csv'], cancelled, 3)
        processes = run(['nvidia-smi','--query-compute-apps=pid,process_name,used_gpu_memory','--format=csv,noheader,nounits'], cancelled, 3)
        owned = {row['pid'] for row in data['assistant_processes']}
        data['assistant_gpu_processes'] = [{'pid':int(row[0]),'name':row[1].strip(),'vram_mib':row[2].strip()} for row in csv.reader(io.StringIO(processes)) if len(row)==3 and row[0].strip().isdigit() and int(row[0]) in owned]
        data['gpu_measurements_scope'] = 'gpu is system-wide; assistant_gpu_processes lists owned compute allocations'
    except (OSError, ValueError) as err: data['gpu_error'] = str(err)
    return data
if __name__ == '__main__': Plugin('power', request_tools(TOOLS), handle).run()
