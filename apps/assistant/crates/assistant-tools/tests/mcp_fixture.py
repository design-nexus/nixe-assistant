import sys, json, os, time
log = os.environ.get('FIXTURE_LOG')
def record(value):
    if log:
        with open(log, 'a') as f: f.write(json.dumps(value) + '\n')
record({'pid':os.getpid()})
for line in sys.stdin:
    value = json.loads(line)
    record(value)
    method=value.get('method')
    if 'id' not in value: continue
    if method=='initialize':
        result={'protocolVersion':os.environ.get('VERSION','2025-11-25'),'capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
    elif method=='tools/list':
        result={'tools':[{'name':'echo','description':'Harmless echo','inputSchema':{'type':'object','properties':{'text':{'type':'string'}},'required':['text'],'additionalProperties':False},'outputSchema':{'type':'object','required':['text'],'properties':{'text':{'type':'string'}}}}]}
    elif method=='tools/call':
        text=value['params']['arguments']['text']
        if text=='disconnect': sys.exit(0)
        if text=='wait': continue
        result={'content':[{'type':'text','text':text}],'structuredContent':{'text':text},'isError':text=='error'}
    else: continue
    print(json.dumps({'jsonrpc':'2.0','id':value['id'],'result':result}),flush=True)
