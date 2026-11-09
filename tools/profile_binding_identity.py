#!/usr/bin/env python3
"""Freeze BindingId candidates against the committed pre-migration revision.

Reuse the accepted immutable workload and output inventory, not an older binary.
Clean A/B and independent allocation ledgers use the existing evidence protocol.
"""
import argparse
import json
import shutil
import subprocess
import tarfile
from pathlib import Path

from profile_definition_identity import build_kind, measure, ledgers, verify
from profile_icexiv_refresh import freeze
from profile_support import BUILD_ENV, inventory, save

REPO = Path(__file__).resolve().parents[1]


def prepare(a):
    if a.out.exists() or a.out.is_relative_to(REPO):
        raise ValueError('use a fresh external evidence directory')
    reference = json.loads((a.reference / 'manifest.json').read_text())
    for name in ['bin', 'logs', 'sources/before']:
        (a.out / name).mkdir(parents=True)
    freeze(a.out / 'sources/after')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    archive = a.out / 'baseline.tar'
    subprocess.run(['git', 'archive', '--format=tar', '--output', str(archive), revision], cwd=REPO, check=True)
    with tarfile.open(archive) as source:
        source.extractall(a.out / 'sources/before', filter='data')
    archive.unlink()
    (a.out / 'site').symlink_to((a.reference / 'site').resolve(), target_is_directory=True)
    shutil.copy2(a.reference / 'output-inventory.json', a.out / 'output-inventory.json')
    inputs = {k: v for k, v in inventory(a.out / 'site').items() if not k.startswith('dist/')}
    assert inputs == reference['inputs']
    save(a.out / 'manifest.json', dict(
        reference=str(a.reference), revision=revision, build_env=BUILD_ENV,
        inputs=inputs, binaries={},
        sources={kind: inventory(a.out / 'sources' / kind) for kind in ['before', 'after']},
    ))


def build(a):
    for kind in ['before', 'after']:
        build_kind(a, kind)
    binaries = json.loads((a.out / 'manifest.json').read_text())['binaries']
    if binaries['before'] == binaries['after']:
        raise ValueError('identical baseline and candidate executables: reject possible stale build reuse')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'build', 'measure', 'ledgers', 'verify'])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--reference', type=Path, default=Path('/tmp/opencode/notist-definition-identity-complete-20261108'))
    parser.add_argument('--target', type=Path, default=Path('/tmp/opencode/notist-source-identity-complete-20261107/target'))
    parser.add_argument('--pairs', type=int, default=8)
    args = parser.parse_args()
    args.out, args.reference, args.target = args.out.resolve(), args.reference.resolve(), args.target.resolve()
    if not 1 <= args.pairs <= 20:
        parser.error('--pairs must be 1..20')
    globals()[args.action](args)
