#!/usr/bin/env python3
"""Fresh full-workload profile of a frozen dirty workspace; never optimizes it."""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import threading
import time

from profile_xiv_history import extract, git, once, save
from xiv_history_probes import enter_function, measure_rhs, MODULE_STATS
from profile_current import parse_detail

REPO = Path(__file__).resolve().parents[1]
OLD = Path('/tmp/opencode/notist-icexiv-profile-20261104')
KINDS = ['before', 'current', 'coarse', 'detail']
INPUTS = ['Notist.toml', 'content', 'data', 'inputs', 'modules', 'scripts', 'sources', 'static', 'templates']
BUILD_ENV = dict(CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='2', CARGO_PROFILE_RELEASE_DEBUG='1')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inventory(root):
    return {str(p.relative_to(root)): [p.stat().st_size, digest(p)]
            for p in sorted(root.rglob('*')) if p.is_file() and '__pycache__' not in p.parts}


def freeze(destination):
    names = sorted(set(git(REPO, 'ls-files', '-co', '--exclude-standard', '-z').split(b'\0')) - {b''})
    destination.mkdir(parents=True)
    expected = {}
    for raw in names:
        path = REPO / os.fsdecode(raw)
        if path.is_file():
            target = destination / path.relative_to(REPO)
            target.parent.mkdir(parents=True, exist_ok=True)
            before = digest(path)
            shutil.copy2(path, target)
            assert digest(target) == before == digest(path), f'file changed during freeze: {path}'
            expected[str(path.relative_to(REPO))] = before
    assert all(digest(REPO / name) == value for name, value in expected.items()), 'workspace changed during freeze'


def instrument(root, detailed):
    def edit(relative, transform):
        path = root / relative
        path.write_text(transform(path.read_text()))

    def entries(relative, methods, prefix='notist_core::current_profile'):
        def transform(text):
            for name, label in methods:
                text = enter_function(text, name, f'let _refresh = {prefix}::span("{label}");')
            return text
        edit(relative, transform)

    core = root / 'crates/notist-core/src'
    shutil.copy2(REPO / 'tools/current_profile_probe.rs', core / 'current_profile.rs')
    edit('crates/notist-core/src/lib.rs', lambda t: t + '\n#[doc(hidden)]\npub mod current_profile;\n')
    edit('src/lib.rs', lambda t: t + '\n#[doc(hidden)]\npub mod xiv_profile;\n')
    (root / 'src/xiv_profile.rs').write_text(
        'pub use notist_core::current_profile::{span, measure, count};\npub struct Report;\n'
        'impl Drop for Report { fn drop(&mut self) { notist_core::current_profile::report();\n'
        + MODULE_STATS + '\n} }\n')
    edit('crates/notist-cli/src/main.rs', lambda t: enter_function(t, 'main',
        'let _report = notist::xiv_profile::Report;\nlet _root = notist::xiv_profile::span("cli.total");'))
    entries('crates/notist-cli/src/ssg.rs', [('assemble', 'cli.assemble')], 'notist::xiv_profile')
    entries('crates/notist-cli/src/publication.rs', [('publish', 'cli.publish'), ('unchanged', 'publish.compare')], 'notist::xiv_profile')

    def site(text):
        head, body = text.split('pub(crate) fn build', 1)
        # Insert after the function's opening brace without touching worker code.
        opening = body.index('{') + 1
        body = body[:opening] + '\n    let _root = notist::xiv_profile::span("ssg.total");\n    let phase = notist::xiv_profile::span("ssg.discover");' + body[opening:]
        for anchor, label in [
            ('    let routes = notist::progress::run(', 'ssg.extension'),
            ('    let mut source_dirs: BTreeSet<_> =', 'ssg.source_directories'),
            ('    let mut documents = vec![];', 'ssg.pipeline'),
            ('    let asset_stage = ', 'ssg.assets'),
            ('    let by_source = DocumentIndex::new(&documents);', 'ssg.bodies'),
            ('    let templates_stage =', 'ssg.shared_context'),
            ('    let workers = if pages.len()', 'ssg.templates'),
            ('    Ok(BuildOutput {', 'ssg.finish'),
        ]:
            body = once(body, anchor, f'    drop(phase);\n    let phase = notist::xiv_profile::span("{label}");\n' + anchor)
        body = measure_rhs(body, 'let (analysis, handle) =', ';\n        errors.extend(', 'pipeline.analyze')
        body = measure_rhs(body, 'let evaluation = ', ';\n        errors.extend(', 'pipeline.evaluate')
        if detailed:
            for binding, end, label in [
                ('let (registry, environment) = ', ';\n        let rendered', 'body.registry'),
                ('let rendered = ', ';\n        errors.extend(rendered', 'body.html'),
                ('let registrations = ', ';\n        if !registrations', 'body.components'),
            ]:
                body = measure_rhs(body, binding, end, label)
        body = once(body, '    Ok(BuildOutput {', '    let result = Ok(BuildOutput {')
        body = once(body, '        warnings,\n    })\n}', '        warnings,\n    });\n    drop(phase);\n    result\n}')
        # No explicit object drops: production auto-destruction and the new
        # consume-during-HTML lifetime remain exactly where they were.
        return head + 'pub(crate) fn build' + body
    edit('crates/notist-ssg/src/build.rs', site)
    entries('src/stdio.rs', [('exchange', 'stdio.total')])
    edit('src/stdio.rs', lambda t: once(once(once(t,
        '    let input = serde_json::to_vec(request)', '    let phase = crate::xiv_profile::span("stdio.encode");\n    let input = serde_json::to_vec(request)'),
        '    let mut child = Command::new(program)', '    drop(phase);\n    let phase = crate::xiv_profile::span("stdio.process");\n    let mut child = Command::new(program)'),
        '    serde_json::from_slice(&output.stdout)', '    drop(phase);\n    let _decode = crate::xiv_profile::span("stdio.decode");\n    serde_json::from_slice(&output.stdout)'))
    if not detailed:
        return
    for relative, methods in [
        ('src/environment/loader.rs', [('document', 'environment.document'), ('visit', 'environment.visit'), ('resolve_imports', 'environment.imports'), ('new', 'environment.graph')]),
        ('src/environment/config.rs', [('parse_config', 'environment.config')]),
        ('src/resource_session.rs', [('source_bytes', 'input.source'), ('load', 'input.load'), ('data', 'input.data'), ('register_source', 'input.register')]),
        ('src/vault.rs', [('ensure_environment', 'vault.environment'), ('analyze_document_with_resource_root', 'vault.analyze'), ('evaluate_in', 'vault.evaluate')]),
    ]:
        entries(relative, methods)
    for relative, methods in [
        ('frontend/markup/mod.rs', [('compile', 'frontend.notist'), ('lower_content', 'frontend.content')]),
        ('code/check.rs', [('document', 'check.document'), ('install', 'check.install')]),
        ('code/content.rs', [('content', 'check.content')]),
        ('shape/mod.rs', [('shape', 'check.shape'), ('shape_inline', 'check.inline')]),
        ('materialize.rs', [('materialize', 'check.materialize')]),
        ('definitions.rs', [('compile_uncached', 'module.compile')]),
        ('modules.rs', [('bind_imports', 'module.bind'), ('bind_cached', 'module.bind_actual')]),
    ]:
        entries('crates/notist-pipeline/src/' + relative, methods)
    # Older frozen snapshots keep evaluation inside pipeline; new snapshots
    # supply it from the independent crate. Instrument the actual implementation.
    evaluator = ('crates/notist-eval/src/' if (root / 'crates/notist-eval/src/eval.rs').exists()
                 else 'crates/notist-pipeline/src/code/')
    entries(evaluator + 'data.rs', [('data_native', 'data.native'), ('read_data', 'data.read')])
    entries(evaluator + 'eval.rs', [('run', 'eval.run'), ('constant', 'eval.comptime'), ('bind_content', 'eval.bind_content'), ('present', 'eval.present'), ('execute', 'eval.execute'), ('implementation', 'eval.implementation'), ('synthetic_call', 'eval.synthetic_call')])
    entries('crates/notist-core/src/data/parse.rs', [('parse', 'data.parse')], 'crate::current_profile')
    entries('crates/notist-core/src/data/mod.rs', [('decode', 'data.decode')], 'crate::current_profile')
    edit('crates/notist-core/src/data/cache.rs', lambda t: once(t,
        '        if let Some((_, result)) = state.parsed.get(&key) {',
        '        if let Some((_, result)) = state.parsed.get(&key) {\n            crate::current_profile::count("data.parse_hit", 1);'))


def prepare(args):
    out = args.out
    assert not out.exists() and not out.is_relative_to(REPO) and not out.is_relative_to(args.consumer)
    out.mkdir(parents=True)
    for name in ['sources', 'bin', 'logs', 'reproducer']:
        (out / name).mkdir()
    m = dict(head=git(REPO, 'rev-parse', 'HEAD').decode().strip(),
             status=git(REPO, 'status', '--porcelain=v1').decode(),
             consumer_head=git(args.consumer, 'rev-parse', 'HEAD').decode().strip(),
             consumer_status=git(args.consumer, 'status', '--porcelain=v1').decode(),
             rustc=subprocess.check_output(['rustc', '-Vv'], text=True), system=platform.platform(),
             cpu_affinity=sorted(os.sched_getaffinity(0)), build_env=BUILD_ENV,
             started_at=subprocess.check_output(['date', '-Ins'], text=True).strip())
    (out / 'workspace.patch').write_bytes(git(REPO, 'diff', '--binary', 'HEAD'))
    freeze(out / 'sources/current')
    old = json.loads((OLD / 'manifest.json').read_text())
    extract(REPO, old['revision'], out / 'sources/before')
    m['before_revision'] = old['revision']
    for name in INPUTS:
        src, dst = args.consumer / name, out / 'site' / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns('__pycache__'))
        else:
            shutil.copy2(src, dst)
    m['inputs'] = inventory(out / 'site')
    m['inputs_identical_to_previous_profile'] = m['inputs'] == old['inputs']
    if m['inputs_identical_to_previous_profile']:
        shutil.copy2(OLD / 'output-inventory.json', out / 'expected-output.json')
    for kind in ['coarse', 'detail']:
        source = out / 'sources' / kind
        shutil.copytree(out / 'sources/current', source)
        instrument(source, kind == 'detail')
        patch = []
        for path in sorted(source.rglob('*.rs')):
            relative = path.relative_to(source)
            original = out / 'sources/current' / relative
            before = original.read_text() if original.exists() else ''
            patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True), fromfile='a/' + str(relative), tofile='b/' + str(relative)))
        (out / f'{kind}.patch').write_text(''.join(patch))
    m['sources'] = {kind: inventory(out / 'sources' / kind) for kind in KINDS}
    m['current_tree_sha256'] = hashlib.sha256(json.dumps(m['sources']['current'], sort_keys=True).encode()).hexdigest()
    for path in (REPO / 'tools').iterdir():
        if path.suffix in ['.py', '.rs']:
            shutil.copy2(path, out / 'reproducer' / path.name)
    m['reproducer'] = inventory(out / 'reproducer')
    m['binaries'] = {}
    save(out / 'manifest.json', m)
    print('Frozen', m['current_tree_sha256'], 'same consumer inputs:', m['inputs_identical_to_previous_profile'], flush=True)


def guard(args):
    out = args.out
    m = json.loads((out / 'manifest.json').read_text())
    assert all(inventory(out / 'sources' / k) == v for k, v in m['sources'].items())
    assert all(digest(out / 'bin' / k) == v for k, v in m['binaries'].items())
    actual = inventory(out / 'site')
    assert {k: v for k, v in actual.items() if not k.startswith('dist/')} == m['inputs']
    assert inventory(out / 'reproducer') == m['reproducer']
    return m


def build(args):
    m = guard(args)
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(args.out / 'target'))
    for kind in KINDS:
        if kind in m['binaries']:
            continue
        source = args.out / 'sources' / kind
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=source, env=env))
        packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
        command = ['cargo', 'build', '--locked', '--release', '-p', 'notist-cli', '--bin', 'notist']
        if kind in ['coarse', 'detail']:
            command += ['--features', 'profiling']
        with (args.out / 'logs' / f'build-{kind}.log').open('wb') as log:
            subprocess.run(['cargo', 'clean', '--release', *[v for p in packages for v in ['-p', p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(args.out / 'target/release/notist', args.out / 'bin' / kind)
        m['binaries'][kind] = digest(args.out / 'bin' / kind)
        save(args.out / 'manifest.json', m)
        print('built', kind, flush=True)


def environment():
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', NO_COLOR='1', TERM='dumb')
    for name in ['LD_PRELOAD', 'RESOURCE_PROFILE', 'RESOURCE_DIAGNOSTIC', 'XIV_PROFILE_REQUESTS', 'EVAL_AB_OBSERVER']:
        assert not env.get(name), name
    return env


def verify_output(args):
    actual = inventory(args.out / 'site/dist/icexiv-db')
    assert sum(name.endswith('.html') for name in actual) == 51739
    expected = args.out / 'expected-output.json'
    if expected.exists():
        assert actual == json.loads(expected.read_text()), 'complete output mismatch'
    else:
        save(expected, actual)
    return len(actual)


def process(args, kind, label, prefix=(), sample_rss=False, extra_env=None):
    env = dict(environment(), **(extra_env or {}))
    command = [*prefix, str(args.out / 'bin' / kind), 'build']
    points, stopped = [], threading.Event()
    started = time.perf_counter()
    with (args.out / 'logs' / f'{label}.stdout').open('wb') as stdout, (args.out / 'logs' / f'{label}.stderr').open('wb') as stderr:
        child = subprocess.Popen(command, cwd=args.out / 'site', env=env, stdout=stdout, stderr=stderr)
        def observe():
            while not stopped.wait(.02):
                try:
                    status = Path(f'/proc/{child.pid}/status').read_text()
                    rss = re.search(r'VmRSS:\s+(\d+)', status)
                    if rss:
                        points.append([time.perf_counter() - started, int(rss[1])])
                except OSError:
                    pass
        if sample_rss:
            observer = threading.Thread(target=observe)
            observer.start()
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.perf_counter() - started
        stopped.set()
        if sample_rss:
            observer.join()
    row = dict(kind=kind, label=label, command=command, wall_s=elapsed, user_s=usage.ru_utime,
               system_s=usage.ru_stime, kernel_maxrss_kib=usage.ru_maxrss, major_faults=usage.ru_majflt,
               involuntary_context_switches=usage.ru_nivcsw, exit=child.returncode,
               load=Path('/proc/loadavg').read_text().strip())
    save(args.out / 'logs' / f'{label}.json', row)
    assert child.returncode == 0, row
    if kind in ['coarse', 'detail']:
        row['metrics'], row['edges'], row['modules'] = parse_detail((args.out / 'logs' / f'{label}.stderr').read_text())
    row['output_files'] = verify_output(args)
    if sample_rss:
        row['observed_rust_rss_kib'] = max(p[1] for p in points)
        save(args.out / 'logs' / f'{label}.rss.json', points)
    save(args.out / 'logs' / f'{label}.json', row)
    print(label, round(elapsed, 4), flush=True)
    return row


def run(args):
    guard(args)
    assert not (args.out / 'samples.json').exists()
    rows = []
    for rep in range(-1, 6):
        for kind in ['before', 'current'] if rep % 2 == 0 else ['current', 'before']:
            row = process(args, kind, f'normal-{rep}-{kind}')
            row['round'] = rep
            rows.append(row)
            save(args.out / 'samples.json', rows)
    for rep in range(-1, 3):
        for kind in ['coarse', 'detail'] if rep % 2 == 0 else ['detail', 'coarse']:
            row = process(args, kind, f'probe-{rep}-{kind}')
            row['round'] = rep
            rows.append(row)
            save(args.out / 'samples.json', rows)


def serve(args):
    guard(args)
    assert not (args.out / 'serve.json').exists()
    rows = []
    trigger = args.out / 'site/content/index.not'
    original = trigger.read_bytes()
    for session, kind in enumerate(['before', 'current', 'current', 'before']):
        label = f'serve-{session}-{kind}'
        log = args.out / 'logs' / f'{label}.stderr'
        with log.open('wb') as stderr:
            child = subprocess.Popen([str(args.out / 'bin' / kind), 'serve', '--port', '0', '--no-clear-screen'], cwd=args.out / 'site', env=environment(), stdout=subprocess.DEVNULL, stderr=stderr)
            completed = 0
            deadline = time.monotonic() + 600
            try:
                while completed < 4:
                    assert child.poll() is None, log.read_text()
                    matches = re.findall(r'Updated · (\d+) pages · (\d+)ms', log.read_text())
                    if len(matches) > completed:
                        pages, milliseconds = matches[completed]
                        assert int(pages) == 51739
                        rows.append(dict(session=session, kind=kind, round=completed, pages=int(pages), elapsed_ms=int(milliseconds)))
                        save(args.out / 'serve.json', rows)
                        print(label, rows[-1], flush=True)
                        completed += 1
                        if completed < 4:
                            time.sleep(2)
                            trigger.write_bytes(original)
                            deadline = time.monotonic() + 600
                    if time.monotonic() > deadline:
                        raise TimeoutError(log.read_text())
                    time.sleep(.1)
            finally:
                child.terminate()
                child.wait(timeout=30)


def diagnostics(args):
    rows = []
    for rep in range(2):
        for kind in ['before', 'current'] if rep == 0 else ['current', 'before']:
            rows.append(process(args, kind, f'rss-{rep}-{kind}', sample_rss=True))
            save(args.out / 'rss.json', rows)
    process(args, 'current', 'strace-current', ['strace', '-qq', '-f', '-c', '-e', 'trace=%file,read,write,close,getdents64', '-o', str(args.out / 'strace-current.txt')])
    wrapper = args.out / 'python-wrapper'
    wrapper.mkdir()
    (wrapper / 'python3').write_text(f'#!/bin/sh\nexec "{shutil.which("python3")}" -m cProfile -o "{args.out}/python.pstats" "$@"\n')
    (wrapper / 'python3').chmod(0o755)
    process(args, 'coarse', 'python-cprofile', extra_env={'PATH': str(wrapper) + os.pathsep + os.environ['PATH']})
    check = subprocess.run(['python3', 'scripts/check_site.py', 'dist/icexiv-db'], cwd=args.out / 'site', env=environment(), capture_output=True, check=True)
    (args.out / 'semantic-check.stdout').write_bytes(check.stdout)


def allocations(args):
    """Separate glibc API traffic observer, not live heap or speedup data."""
    guard(args)
    assert not os.environ.get('LD_PRELOAD')
    source = args.out / 'sources/current/tools/evaluator_ab_alloc.c'
    library = args.out / 'bin/allocation-observer.so'
    assert not library.exists()
    command = ['cc', '-O2', '-shared', '-fPIC', str(source), '-o', str(library)]
    subprocess.run(command, check=True)
    shutil.copy2(Path(__file__), args.out / 'allocation-reproducer.py')
    proof = dict(source_sha256=digest(source), library_sha256=digest(library),
                 reproducer_sha256=digest(args.out / 'allocation-reproducer.py'),
                 compile_command=command, compiler=subprocess.check_output(['cc', '--version'], text=True), samples=[])
    for rep in range(2):
        for kind in ['before', 'current'] if rep == 0 else ['current', 'before']:
            label = f'alloc-{rep}-{kind}'
            process(args, kind, label, extra_env={'LD_PRELOAD': str(library), 'EVAL_AB_ALLOC_TARGET': str(args.out / 'bin' / kind)})
            text = (args.out / 'logs' / f'{label}.stderr').read_text()
            lines = [line for line in text.splitlines() if line.startswith('[eval-native-alloc] ')]
            assert len(lines) == 1, lines
            values = {key: int(value) for key, value in re.findall(r'(\w+)=(\d+)', lines[0])}
            proof['samples'].append(dict(kind=kind, round=rep, **values))
            save(args.out / 'allocations.json', proof)
    assert digest(source) == proof['source_sha256'] and digest(library) == proof['library_sha256']


def report(args):
    m = guard(args)
    rows = json.loads((args.out / 'samples.json').read_text())
    result = {}
    for kind in KINDS:
        selected = [r for r in rows if r['kind'] == kind and r['round'] >= 0]
        result[kind] = {field: dict(median=statistics.median(r[field] for r in selected), min=min(r[field] for r in selected), max=max(r[field] for r in selected), samples=[r[field] for r in selected]) for field in ['wall_s', 'user_s', 'system_s', 'kernel_maxrss_kib']}
        if kind in ['coarse', 'detail']:
            result[kind]['metrics'] = {name: {field: statistics.mean(r['metrics'][name][field] for r in selected) for field in selected[0]['metrics'][name]} for name in selected[0]['metrics']}
            result[kind]['modules'] = selected[0]['modules']
    module_counts = [{k: v['calls'] for k, v in r['modules'].items()} for r in rows if 'modules' in r]
    assert all(counts == module_counts[0] for counts in module_counts)
    before = {r['round']: r for r in rows if r['kind'] == 'before' and r['round'] >= 0}
    paired = [dict(round=r['round'], before_s=before[r['round']]['wall_s'], current_s=r['wall_s'],
                   reduction_percent=100 * (1 - r['wall_s'] / before[r['round']]['wall_s']))
              for r in rows if r['kind'] == 'current' and r['round'] >= 0]
    result['comparison'] = dict(pairs=paired, faster_pairs=sum(r['current_s'] < r['before_s'] for r in paired),
        wall_reduction_percent=100 * (1 - result['current']['wall_s']['median'] / result['before']['wall_s']['median']),
        speedup=result['before']['wall_s']['median'] / result['current']['wall_s']['median'])
    serve_rows = json.loads((args.out / 'serve.json').read_text())
    result['serve'] = {}
    for kind in ['before', 'current']:
        result['serve'][kind] = {}
        for case in ['initial', 'rebuild']:
            found = [r['elapsed_ms'] for r in serve_rows if r['kind'] == kind and (r['round'] == 0) == (case == 'initial')]
            result['serve'][kind][case] = dict(n=len(found), median_ms=statistics.median(found), min_ms=min(found), max_ms=max(found), samples_ms=found)
    rss = json.loads((args.out / 'rss.json').read_text())
    result['rss'] = {kind: dict(observed_rust_peak_kib=max(r['observed_rust_rss_kib'] for r in rss if r['kind'] == kind),
                             sampled_peaks_kib=[r['observed_rust_rss_kib'] for r in rss if r['kind'] == kind],
                             kernel_peak_kib=max(r['kernel_maxrss_kib'] for r in rss if r['kind'] == kind)) for kind in ['before', 'current']}
    t = lambda name: result['coarse']['metrics'][name]['seconds']
    phases = ['ssg.discover', 'ssg.extension', 'ssg.source_directories', 'ssg.pipeline', 'ssg.assets', 'ssg.bodies', 'ssg.shared_context', 'ssg.templates', 'ssg.finish']
    partition = {name: t(name) for name in phases if name != 'ssg.pipeline'}
    partition.update(analyze=t('pipeline.analyze'), evaluate=t('pipeline.evaluate'),
                     pipeline_other=t('ssg.pipeline') - t('pipeline.analyze') - t('pipeline.evaluate'),
                     ssg_tail=t('ssg.total') - sum(t(name) for name in phases),
                     host_and_vault=t('cli.assemble') - t('ssg.total'))
    assert all(value >= -1e-7 for value in partition.values())
    assert abs(sum(partition.values()) - t('cli.assemble')) < 1e-7
    result['assembly_partition'] = {name: dict(seconds=value, percent=value / t('cli.assemble') * 100) for name, value in partition.items()}
    result['top_detail_self'] = sorted((dict(name=name, **v) for name, v in result['detail']['metrics'].items() if v['self_seconds'] > 0), key=lambda r: r['self_seconds'], reverse=True)
    additional = 0
    if (args.out / 'allocations.json').exists():
        proof = json.loads((args.out / 'allocations.json').read_text())
        assert digest(args.out / 'sources/current/tools/evaluator_ab_alloc.c') == proof['source_sha256']
        assert digest(args.out / 'bin/allocation-observer.so') == proof['library_sha256']
        assert digest(args.out / 'allocation-reproducer.py') == proof['reproducer_sha256']
        additional = len(proof['samples'])
        result['allocation_traffic'] = {kind: {field: statistics.mean(r[field] for r in proof['samples'] if r['kind'] == kind)
                                            for field in ['allocations', 'requested_bytes', 'frees', 'reallocations']} for kind in ['before', 'current']}
    save(args.out / 'summary.json', result)
    save(args.out / 'integrity.json', dict(current_tree_sha256=m['current_tree_sha256'], frozen_sources_binaries_inputs_reproducer_verified=True, full_cli_builds=len(rows) + len(rss) + 2 + additional, serve_builds=len(serve_rows), complete_cli_output_verified=True, semantic_links_verified=(args.out / 'semantic-check.stdout').exists()))
    print(json.dumps({k: result[k]['wall_s'] for k in KINDS}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'build', 'run', 'serve', 'diagnostics', 'allocations', 'report'])
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--consumer', type=Path, default=Path('/home/azurice/Files/_ffxiv/icexiv-db'))
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)
