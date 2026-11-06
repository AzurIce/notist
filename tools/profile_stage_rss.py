#!/usr/bin/env python3
"""Diagnostic only: ordinary verbose CLI's coarse phase memory, never A/B timing."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from profile_deadline import OUT
import profile_runtime_cow as common


def run(kind):
    label = 'stage-rss-' + kind
    points = []
    start = time.perf_counter()
    with (OUT / 'logs' / (label + '.stderr')).open('wb') as log:
        child = subprocess.Popen([str(OUT / 'bin' / kind), 'build', '--verbose'], cwd=OUT / 'site',
                                 env=dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1'),
                                 stdout=(OUT / 'logs' / (label + '.stdout')).open('wb'), stderr=subprocess.PIPE)
        for line in child.stderr:
            log.write(line)
            text = line.decode(errors='replace').strip()
            if text.startswith(('├─ ✓', '├─ …', '✓ Build')):
                try:
                    status = Path(f'/proc/{child.pid}/status').read_text()
                except FileNotFoundError:
                    status = ''
                point = dict(event=text, elapsed_s=time.perf_counter()-start,
                             status={line.split(':')[0]: line.split(':', 1)[1].strip()
                                     for line in status.splitlines() if line.startswith(('VmRSS:', 'VmHWM:', 'VmSwap:'))})
                points.append(point)
                print(json.dumps(point, ensure_ascii=False), flush=True)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    assert child.returncode == 0
    assert common.inventory(OUT / 'site/dist/icexiv-db') == json.loads((OUT / 'output-inventory.json').read_text())
    common.save(OUT / (label + '.json'), dict(points=points, maxrss_bytes=usage.ru_maxrss*1024,
                                          note='Verbose diagnostic overhead; phase samples are not proof of a peak bound'))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('kind')
    run(p.parse_args().kind)
