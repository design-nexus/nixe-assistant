import tempfile
from pathlib import Path
from assistant_plugin import Plugin, tool
from desktop_plugin import run, request_tools
TOOLS = [tool('capture.clipboard', 'Read clipboard text once on explicit request; no monitoring'),
         tool('capture.screen', 'Capture screen once and return OCR text; temporary screenshot is deleted', {'output': {'type': 'string', 'maxLength': 128}, 'language': {'type': 'string', 'pattern': '^[a-z_+]{3,32}$'}})]
def handle(name, args, cancelled):
    if name == 'capture.clipboard':
        text = run(['wl-paste', '--no-newline', '--type', 'text'], cancelled)
        return {'text': text[:32768], 'truncated': len(text) > 32768, 'source': 'clipboard', 'untrusted_content': True}
    with tempfile.TemporaryDirectory(prefix='assistant-ocr-') as folder:
        path = Path(folder) / 'screen.png'
        argv = ['grim']
        if args.get('output'): argv += ['-o', args['output']]
        run(argv + [str(path)], cancelled, 4)
        text = run(['tesseract', str(path), 'stdout', '-l', args.get('language', 'eng')], cancelled, 9)
        return {'text': text[:32768], 'truncated': len(text) > 32768, 'source': 'screen OCR', 'untrusted_content': True}
if __name__ == '__main__': Plugin('capture', request_tools(TOOLS), handle).run()
