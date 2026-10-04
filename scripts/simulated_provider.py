"""Offline CLI fixtures. Only a loopback server and a synthetic key are used."""
from collections import deque
import contextlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import threading


def reply(text='fixture reply', calls=(), *, complete=True):
    delta = {'content': text} if text else {}
    if calls:
        delta['tool_calls'] = [
            {'index': i, 'id': f'call-{i}', 'type': 'function',
             'function': {'name': name, 'arguments': json.dumps(args)}}
            for i, (name, args) in enumerate(calls)
        ]
    frames = [{'choices': [{'delta': delta}]},
              {'choices': [{'delta': {}, 'finish_reason': 'tool_calls' if calls else 'stop'}],
               'usage': {'prompt_tokens': 1000, 'completion_tokens': 10}}]
    # Leave off the terminal event to simulate a transport ending after usage.
    if not complete:
        frames[-1]['choices'] = []
    return ''.join('data: ' + json.dumps(frame) + '\n\n' for frame in frames) + (
        'data: [DONE]\n\n' if complete else '')


class Provider(contextlib.AbstractContextManager):
    def __init__(self):
        self.responses = deque()
        self.requests = []
        self.errors = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            # Chunked bodies need HTTP/1.1; each response closes its own
            # connection, so a handler never waits for a second request.
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_args):
                pass

            def do_POST(self):
                self.connection.settimeout(10)
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                owner.requests.append(request)
                if not owner.responses:
                    owner.errors.append('unexpected provider request')
                    self.send_error(500, 'fixture exhausted')
                    self.close_connection = True
                    return
                response = owner.responses.popleft()
                if callable(response):
                    response = response(request)
                if isinstance(response, str):
                    payload = response.encode()
                    self.send_response(200)
                    self.send_header('Content-Type', 'text/event-stream')
                    self.send_header('Content-Length', str(len(payload)))
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    self.wfile.write(payload)
                    return
                # An iterable of pieces, each sent as it comes: a model that
                # streams at its own pace (a piece may sleep before it yields).
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Transfer-Encoding', 'chunked')
                self.send_header('Connection', 'close')
                self.end_headers()
                for piece in response:
                    data = piece.encode()
                    self.wfile.write(f'{len(data):x}\r\n'.encode() + data + b'\r\n')
                    self.wfile.flush()
                self.wfile.write(b'0\r\n\r\n')

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self):
        return f'http://127.0.0.1:{self.server.server_port}/v1'

    def __exit__(self, *_args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=10)


class Fixture:
    def __init__(self, root, binary, provider):
        self.root = Path(root)
        self.binary = Path(binary).resolve()
        self.home = self.root / 'home'
        self.project = self.root / 'project'
        self.home.mkdir(parents=True)
        self.project.mkdir()
        self.env = {'PATH': os.environ.get('PATH', ''), 'HOME': str(self.home),
                    'RYTER_HOME': str(self.home / 'ryter'), 'LANG': 'C.UTF-8',
                    'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': os.devnull}
        self.config = Path(self.env['RYTER_HOME']) / 'config.toml'
        self.config.parent.mkdir()
        self.base = f'''default_connection = "fixture"
[connections.fixture]
kind = "openai_compat"
base_url = "{provider.url}"
api_backend = "chat_completions"
api_key = "synthetic-fixture-key"
default_model = "fixture-model"
[orchestrator]
model = "fixture-model"
[pricing.fixture-model]
input_per_million = 1.0
output_per_million = 1.0
[update]
mode = "off"
'''
        self.configure()

    def configure(self, extra=''):
        self.config.write_text(self.base + extra)
        self.config.chmod(0o600)

    def run(self, *args):
        return subprocess.run([str(self.binary), *args], cwd=self.project, env=self.env,
                              capture_output=True, text=True, timeout=30)

    def events(self, *args):
        result = self.run('--json', *args)
        events = [json.loads(line) for line in result.stdout.splitlines()]
        return result, events
