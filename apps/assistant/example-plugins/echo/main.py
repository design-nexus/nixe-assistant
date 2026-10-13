from assistant_plugin import Plugin, tool
Plugin('example.echo', [tool('example.echo', 'Echo text', {'text': {'type': 'string'}}, ['text'])],
       lambda name, args, cancelled: {'text': args['text']}).run()
