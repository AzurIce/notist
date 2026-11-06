#!/usr/bin/env python3
"""Small real-process reproducer for observed hidden data edits."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.request

from profile_deadline import OUT


def run(kind):
    root = OUT / ('hidden-' + kind)
    root.mkdir()
    (root / 'content/.data').mkdir(parents=True)
    (root / 'Notist.toml').write_text('[project]\nname="hidden"\n[[site]]\noutput="public"\n')
    (root / 'content/index.not').write_text('#import "../lib.notc" as data;\n#data.view()\n')
    (root / 'lib.notc').write_text('fn view() -> InlineContent { [#text(text: read_text(p"content/.data/value.txt"))] }')
    data = root / 'content/.data/value.txt'
    data.write_text('original hidden value')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    child = subprocess.Popen([str(OUT / 'bin' / kind), 'serve', '--address', f'127.0.0.1:{port}', '--no-clear-screen'], cwd=root,
                             env=dict(os.environ, NO_COLOR='1', TERM='dumb'),
                             stdout=(OUT / 'logs' / f'hidden-{kind}.stdout').open('wb'),
                             stderr=(OUT / 'logs' / f'hidden-{kind}.stderr').open('wb'))
    def read():
        return urllib.request.urlopen(f'http://127.0.0.1:{port}/', timeout=3).read().decode()
    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                if 'original hidden value' in read():
                    break
            except OSError:
                pass
            assert time.monotonic() < deadline
            time.sleep(.05)
        data.write_text('updated hidden value')
        time.sleep(1)
        updated = 'updated hidden value' in read()
        result = dict(kind=kind, hidden_edit_observed=updated)
        (OUT / f'hidden-{kind}.json').write_text(json.dumps(result, indent=2))
        print(result, flush=True)
    finally:
        child.terminate()
        child.wait(timeout=5)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('kind')
    run(parser.parse_args().kind)
