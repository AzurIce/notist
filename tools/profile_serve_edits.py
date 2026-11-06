#!/usr/bin/env python3
"""Real serve edit sequence against same-revision cold publication, full HTTP hashes."""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request
import urllib.error

import profile_runtime_cow as common
from profile_deadline import OUT


def without_preview_script(actual):
    actual, count = re.subn(rb'<script type="module" src="(?:\.\./|\./)*_notist/serve/client\.js\?v=\d+"></script>', b'', actual)
    assert count == 1, count
    return actual


def read_body(response, headers):
    if headers.get(b'transfer-encoding') == b'chunked':
        chunks = []
        while True:
            size = int(response.readline().split(b';', 1)[0], 16)
            if size == 0:
                while True:
                    trailer = response.readline()
                    assert trailer, 'truncated chunk trailers'
                    if trailer == b'\r\n':
                        break
                return b''.join(chunks)
            chunk = response.read(size)
            assert len(chunk) == size
            chunks.append(chunk)
            assert response.read(2) == b'\r\n'
    actual = response.read(int(headers[b'content-length']))
    assert len(actual) == int(headers[b'content-length'])
    return actual


def verify_partition(items, port):
    # Verification only, outside rebuild timings. Pipeline bounded batches on
    # loopback instead of creating 78,004 separate client connections per edit.
    for start in range(0, len(items), 32):
        batch = items[start:start + 32]
        with socket.create_connection(('127.0.0.1', port), timeout=60) as sock:
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            requests = b''.join(f'GET /{urllib.parse.quote(path)} HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n'.encode() for path, _ in batch)
            sock.sendall(requests)
            with sock.makefile('rb') as response:
                for path, expected in batch:
                    status = response.readline()
                    assert status.split()[1] == b'200', (path, status)
                    headers = {}
                    while True:
                        line = response.readline()
                        assert line, path
                        if line == b'\r\n':
                            break
                        key, value = line.split(b':', 1)
                        headers[key.lower()] = value.strip()
                    actual = read_body(response, headers)
                    if path.endswith('.html'):
                        actual = without_preview_script(actual)
                    assert [len(actual), hashlib.sha256(actual).hexdigest()] == expected, path


def process_cpu_s(pid):
    # /proc process stat accounts the thread group and waited-for children.
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return sum(int(fields[i]) for i in [11, 12, 13, 14]) / os.sysconf('SC_CLK_TCK')


def run(kind, label, tail=False, stop_after_error=False, measure_cpu=False):
    root = OUT / ('serve-' + label)
    shutil.copytree(OUT / 'site', root, ignore=shutil.ignore_patterns('dist', '__pycache__'))
    if tail:
        index = root / 'content/index.not'
        index.write_text(index.read_text() + '\n\ndeadline retained edit\n')
        (root / 'content/deadline-added.not').write_text('= Added\n\n[Home](index.not)\n')
    logs = OUT / 'logs'
    binary = OUT / 'bin' / kind
    cold_output = OUT / ('cold-' + label)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    env = dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1')
    events = queue.Queue()
    process = subprocess.Popen([str(binary), 'serve', '--address', f'127.0.0.1:{port}', '--no-clear-screen'],
                               cwd=root, env=env, stdout=(logs / f'{label}-serve.stdout').open('wb'),
                               stderr=(logs / f'{label}-serve.stderr').open('wb'))

    def receive():
        try:
            deadline = time.monotonic() + 120
            while True:
                try:
                    stream = urllib.request.urlopen(base + '/_notist/serve/events', timeout=120)
                    break
                except OSError:
                    if time.monotonic() > deadline:
                        raise
                    time.sleep(.1)
            for raw in stream:
                if raw.startswith(b'data: '):
                    events.put(json.loads(raw[6:]))
        except Exception as error:
            events.put(error)

    threading.Thread(target=receive, daemon=True).start()
    rows = []
    revision = 0
    last_build = 0
    edit_started = time.perf_counter()
    edit_cpu_started = process_cpu_s(process.pid) if measure_cpu else None
    def start_edit():
        nonlocal edit_started, edit_cpu_started
        edit_started = time.perf_counter()
        edit_cpu_started = process_cpu_s(process.pid) if measure_cpu else None

    def wait(timeout=120):
        nonlocal last_build, revision
        while True:
            event = events.get(timeout=timeout)
            if isinstance(event, Exception):
                raise event
            report = event['report']
            if report['build'] > last_build and report['status'] != 'building':
                last_build = report['build']
                revision = event['revision']
                event['observed_edit_to_event_s'] = time.perf_counter() - edit_started
                if measure_cpu:
                    event['observed_process_and_children_cpu_s'] = process_cpu_s(process.pid) - edit_cpu_started
                return event

    def verify(name, event):
        # The cold oracle must not generate filesystem events in the watched
        # project (even removing the output's parent would trigger a rebuild).
        output = cold_output
        if output.exists():
            shutil.rmtree(output)
        start = time.perf_counter()
        cold = subprocess.run([str(binary), 'build', '--out-dir', str(output)], cwd=root, env=env, capture_output=True)
        wall = time.perf_counter() - start
        (logs / f'{label}-{name}-cold.stdout').write_bytes(cold.stdout)
        (logs / f'{label}-{name}-cold.stderr').write_bytes(cold.stderr)
        report = event['report']
        row = dict(edit=name, event=event, cold_wall_s=wall, cold_exit=cold.returncode)
        if report['status'] == 'failed':
            assert cold.returncode != 0
            text = cold.stderr.decode()
            assert all(issue['message'] in text for issue in report['errors']), report
            row['diagnostics_match'] = True
            # Failed edits retain the last successful artifact, not partial output.
            html = urllib.request.urlopen(base + '/').read()
            assert b'deadline retained edit' in html
        else:
            assert cold.returncode == 0, cold.stderr.decode()
            inventory = common.inventory(output)
            common.save(logs / f'{label}-{name}-cold-inventory.json', inventory)
            assert json.loads((output / '_notist/publication.json').read_text()) == {'format': 'notist-publication', 'version': 1}
            items = list(inventory.items())
            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                verify_start = time.perf_counter()
                list(pool.map(lambda partition: verify_partition(partition, port), [items[i::8] for i in range(8)]))
            row.update(full_http_hashes_match=True, files=len(inventory), http_verification_s=time.perf_counter()-verify_start)
            assert not report['errors']
            assert report['pages'] == sum(p.endswith('.html') for p in inventory)
        rows.append(row)
        common.save(OUT / (label + '-serve-samples.json'), rows)
        print(label, name, report['elapsed_ms'], 'ms, cold', round(wall, 3), flush=True)

    try:
        verify('initial', wait())
        index = root / 'content/index.not'
        original = index.read_text()
        added = root / 'content/deadline-added.not'
        if not tail:
            start_edit()
            index.write_text(original + '\n\ndeadline retained edit\n')
            verify('content-edit', wait())
            start_edit()
            index.write_text(original + '\n\ndeadline retained edit\n\n![missing](deadline-missing.png)\n')
            verify('error', wait())
            if stop_after_error:
                return rows
            start_edit()
            index.write_text(original + '\n\ndeadline retained edit\n')
            verify('fix', wait())
            start_edit()
            added.write_text('= Added\n\n[Home](index.not)\n')
            verify('add', wait())
        start_edit()
        added.unlink()
        verify('delete', wait())
        try:
            urllib.request.urlopen(base + '/deadline-added.html')
            raise AssertionError('removed page still served')
        except urllib.error.HTTPError as error:
            assert error.code == 404
        # A shared semantic dependency changes every consuming generated page.
        module = root / 'modules/views.notc'
        start_edit()
        text = module.read_text()
        assert text.count('固定版本数据') == 1
        module.write_text(text.replace('固定版本数据', '固定版本数据 · deadline dependency edit'))
        verify('module-dependency', wait())
        data = root / 'content/.data/items/0.json'
        records = json.loads(data.read_text())
        records[0][1] += ' deadline data edit'
        start_edit()
        data.write_text(json.dumps(records, ensure_ascii=False, separators=(',', ':')) + '\n')
        try:
            event = wait(timeout=4 if kind == 'before' else 120)
        except queue.Empty:
            assert kind == 'before', 'candidate missed a concrete dependency'
            if cold_output.exists():
                shutil.rmtree(cold_output)
            cold = subprocess.run([str(binary), 'build', '--out-dir', str(cold_output)], cwd=root, env=env, capture_output=True, check=True)
            (logs / f'{label}-data-dependency-cold.stdout').write_bytes(cold.stdout)
            (logs / f'{label}-data-dependency-cold.stderr').write_bytes(cold.stderr)
            path = f'items/{records[0][0]}.html'
            assert b'deadline data edit' in (cold_output / path).read_bytes()
            assert b'deadline data edit' not in urllib.request.urlopen(base + '/' + path).read()
            rows.append(dict(edit='data-dependency', known_baseline_failure='observed hidden input edit ignored; preview stale versus cold build', no_rebuild_within_s=4))
            common.save(OUT / (label + '-serve-samples.json'), rows)
            print(label, 'data-dependency: baseline stale preview reproduced', flush=True)
        else:
            verify('data-dependency', event)
        asset = root / 'static/catalog.js'
        start_edit()
        asset.write_text(asset.read_text() + '\n// deadline static dependency edit\n')
        verify('asset-dependency', wait())
    finally:
        process.terminate()
        process.wait(timeout=15)
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kind', required=True)
    parser.add_argument('--label', required=True)
    args = parser.parse_args()
    run(args.kind, args.label)
