#!/usr/bin/env python3
"""After-only residual-cost investigation of a frozen optimization snapshot.

The IO and evaluator probes are separate binaries/cohorts. No production source,
input policy, allocator configuration, consumer, or previous timing sample is changed.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
from types import SimpleNamespace

from profile_optimizations import WORKLOADS, guard, process
from profile_publication_checks import instrument_publication
from profile_resource_overhead import fingerprints
from profile_xiv_history import REPO, git, once, save, sha
from xiv_history_probes import enter_function


SINK = 'notist_core::current_profile'


def change(root, relative, rewrite):
    path = root / relative
    path.write_text(rewrite(path.read_text()))


def io_probe(root):
    def publication(text):
        text = instrument_publication(text)
        text = once(text,
                    'let metadata = std::fs::metadata(&output).map_err(io)?;',
                    'let metadata = notist::xiv_profile::measure("ownership.metadata", || std::fs::metadata(&output)).map_err(io)?;')
        text = once(text,
                    'let empty = std::fs::read_dir(&output).map_err(io)?.next().is_none();',
                    '''let empty = notist::xiv_profile::measure("ownership.empty", || {
            let mut entries = notist::xiv_profile::measure("ownership.directory_open", || std::fs::read_dir(&output)).map_err(io)?;
            let first = notist::xiv_profile::measure("ownership.directory_next", || entries.next());
            let empty = notist::xiv_profile::measure("ownership.entry_drop", || first.is_none());
            notist::xiv_profile::measure("ownership.iterator_drop", || drop(entries));
            Ok::<_, Error>(empty)
        })?;''')
        text = once(text,
                    'let owned = std::fs::read(output.join("_notist/publication.json"))',
                    'let owned = notist::xiv_profile::measure("ownership.manifest", || notist::xiv_profile::measure("ownership.manifest_read", || std::fs::read(output.join("_notist/publication.json")))')
        text = once(text,
                    '.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())',
                    '.and_then(|bytes| notist::xiv_profile::measure("ownership.manifest_parse", || serde_json::from_slice::<serde_json::Value>(&bytes).ok()))')
        text = once(text,
                    'manifest["format"] == "notist-publication" && manifest["version"] == 1\n            });',
                    'manifest["format"] == "notist-publication" && manifest["version"] == 1\n            }));')
        text = once(text,
                    '    let directories: BTreeSet<_> = files',
                    '    let planning = notist::xiv_profile::span("compare.directory_plan");\n    let directories: BTreeSet<_> = files')
        text = once(text,
                    '    let mut pending = vec![output.to_path_buf()];',
                    '    drop(planning);\n    let mut pending = vec![output.to_path_buf()];')
        text = once(text,
                    '        let Ok(entries) = std::fs::read_dir(directory) else {',
                    '        let Ok(entries) = notist::xiv_profile::measure("compare.directory_open", || std::fs::read_dir(directory)) else {')
        return text

    change(root, 'crates/notist-cli/src/publication.rs', publication)

    def resources(text):
        head, body = text.split('impl ResourceReader for FsReader {', 1)
        marker = '#[cfg(not(target_arch = "wasm32"))]\nfn same_metadata'
        body, tail = body.split(marker, 1)
        body = once(body,
                    'let file = self.0.open_with(path, &options).map_err(input_io)?;',
                    f'let file = {SINK}::measure("bounded.open", || self.0.open_with(path, &options)).map_err(input_io)?;')
        body = once(body,
                    'let metadata = file.metadata().map_err(input_io)?;',
                    f'let metadata = {SINK}::measure("bounded.metadata_before", || file.metadata()).map_err(input_io)?;')
        body = once(body,
                    '''            (&file)
                .take((limit as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(input_io)?;''',
                    f'''            {SINK}::measure("bounded.read_bytes", || (&file)
                .take((limit as u64).saturating_add(1))
                .read_to_end(&mut bytes))
                .map_err(input_io)?;
            {SINK}::count("work.bounded_bytes", bytes.len());''')
        body = once(body,
                    '&file.metadata().map_err(input_io)?',
                    f'&{SINK}::measure("bounded.metadata_after", || file.metadata()).map_err(input_io)?')
        body = once(body,
                    'Ok(Arc::from(bytes))',
                    f'Ok({SINK}::measure("bounded.share_bytes", || Arc::from(bytes)))')
        return head + 'impl ResourceReader for FsReader {' + body + marker + tail

    change(root, 'src/resources.rs', resources)


def evaluator_probe(root):
    def evaluator(text):
        text = enter_function(text, 'execute', f'''    let _residual_execute = {SINK}::span(
        if self.mode == EvaluationMode::Runtime {{ "eval.execute.runtime" }} else {{ "eval.execute.comptime" }});
    let preparation = {SINK}::span("call.prepare");''')
        for name, label in [
            ('implementation', 'call.implementation'),
            ('capture_call', 'capture.call'),
            ('capture_expr', 'capture.expr'),
            ('item_shell', 'tree.boundary_copy'),
            ('content_count', 'tree.bind_budget_walk'),
            ('valid_structure', 'tree.valid_structure'),
            ('expand', 'tree.expand_origin'),
        ]:
            text = enter_function(text, name, f'    let _residual = {SINK}::span("{label}");')
        text = once(text,
                    '    if !e.needed.is_empty() {',
                    f'    {SINK}::count("work.output_nodes", roots.iter().flat_map(Item::descendants).count());\n    if !e.needed.is_empty() {{')
        text = enter_function(text, 'bind_content', f'    {SINK}::count("work.bind_nodes", forest.len());')
        text = enter_function(text, 'present', f'    {SINK}::count("work.present_nodes", forest.len());')
        walk = '''            let added = nodes
                .iter()
                .flat_map(Item::descendants)
                .filter(|i| !matches!(i.evaluation, Some(EvaluationState::Error(_))))
                .count();'''
        if walk in text:
            text = once(text, walk,
                    f'''            let added = {SINK}::measure("tree.present_budget_walk", || nodes
                .iter()
                .flat_map(Item::descendants)
                .inspect(|_| {SINK}::count("work.present_budget_visits", 1))
                .filter(|i| !matches!(i.evaluation, Some(EvaluationState::Error(_))))
                .count());''')
        else:
            assert 'let (item, added) = self.present_node(node, limit);' in text
        head, tail = text.split('fn content_count(', 1)
        body, rest = tail.split('fn valid_structure(', 1)
        body = once(body,
                    '.flat_map(Item::descendants)',
                    f'.flat_map(Item::descendants)\n        .inspect(|_| {SINK}::count("work.bind_budget_visits", 1))')
        text = head + 'fn content_count(' + body + 'fn valid_structure(' + rest
        text = once(text,
                    '        let mut inputs = Scope::new();',
                    f'''        {SINK}::count(match (self.mode == EvaluationMode::Runtime, code) {{
            (true, true) => "work.runtime_code_calls", (true, false) => "work.runtime_native_calls",
            (false, true) => "work.comptime_code_calls", (false, false) => "work.comptime_native_calls",
        }}, 1);
        let mut inputs = Scope::new();''')
        text = once(text,
                    '        let result = match definition.implementation {',
                    f'''        drop(preparation);
        let body = {SINK}::span(if code {{ "call.body.code" }} else {{ "call.body.native" }});
        let result = match definition.implementation {{''')
        text = once(text,
                    '        if value_call {\n            self.depth -= 1;',
                    f'        drop(body);\n        let _result = {SINK}::span("call.result");\n        if value_call {{\n            self.depth -= 1;')
        text = once(text,
                    '        let key = instance.key();',
                    f'        let key = instance.key();\n        {SINK}::count("work.instance_key_bytes", key.len());')
        return text

    change(root, 'crates/notist-pipeline/src/code/eval.rs', evaluator)

    def graph(text):
        text = enter_function(text, 'new', f'    let _residual = {SINK}::span("graph.new");')
        text = once(text,
                    'let module = match self.pipeline.compile_module_with_root(',
                    f'let module = match {SINK}::measure("graph.compile", || self.pipeline.compile_module_with_root(')
        text = once(text,
                    '&self.resources.source_root(path).to_string_lossy(),\n        ) {',
                    '&self.resources.source_root(path).to_string_lossy(),\n        )) {')
        text = once(text,
                    'let linked = match self.context.link_module(module, targets) {',
                    f'let linked = match {SINK}::measure("graph.link", || self.context.link_module(module, targets)) {{')
        text = once(text,
                    'match std::mem::take(&mut self.context).with_linked_module(linked) {',
                    f'match {SINK}::measure("graph.install", || std::mem::take(&mut self.context).with_linked_module(linked)) {{')
        return text

    change(root, 'src/environment/loader.rs', graph)


def checker_probe(root):
    def definitions(text):
        text = enter_function(text, 'compile_uncached', f'    let _residual = {SINK}::span("module.compile_work");')
        return once(text,
                    'let parsed = notist_syntax::code::parse_module(source, frontend);',
                    f'let parsed = {SINK}::measure("module.parse", || notist_syntax::code::parse_module(source, frontend));')

    change(root, 'crates/notist-pipeline/src/definitions.rs', definitions)
    def body_lookup(text):
        cloning = 'self.context.program().body(&def.id).cloned()' in text
        expression = 'self.context.program().body(&def.id)' + ('.cloned()' if cloning else '')
        label = 'check.body_clone' if cloning else 'check.body_borrow'
        return once(text, f'let body = {expression};', f'let body = {SINK}::measure("{label}", || {expression});')
    change(root, 'crates/notist-pipeline/src/code/check.rs', body_lookup)
    change(root, 'crates/notist-pipeline/src/context.rs', lambda text: once(text,
           'impl Context {', '''impl Context {
    #[doc(hidden)]
    pub fn residual_checked_storage(&self) -> (usize, usize, usize) {
        let checked = self.checked.lock().unwrap();
        (std::sync::Arc::as_ptr(&self.checked) as usize, checked.len(),
         checked.values().map(|program| program.instances.len()).sum())
    }
'''))
    change(root, 'src/environment/loader.rs', lambda text: once(text,
           'struct Graph<', f'''impl EnvironmentCache {{
    pub(crate) fn residual_cache_counts(&self) {{
        {SINK}::count("retained.graphs", self.graphs.len());
        {SINK}::count("retained.configs", self.configs.len());
    }}
}}

struct Graph<'''))
    change(root, 'src/vault.rs', lambda text: once(text,
           'impl<R: Resources> Vault<R> {', f'''impl<R: Resources> Vault<R> {{
    #[doc(hidden)]
    pub fn residual_cache_counts(&self) {{
        let mut stores = BTreeMap::new();
        let mut snapshots = 0;
        for versions in self.environments.values() {{
            for environment in versions.values() {{
                snapshots += 1;
                let (identity, plans, records) = environment.context().residual_checked_storage();
                stores.insert(identity, (plans, records));
            }}
        }}
        {SINK}::count("retained.environments", snapshots);
        {SINK}::count("retained.checked_stores", stores.len());
        {SINK}::count("retained.checked_plans", stores.values().map(|(plans, _)| *plans).sum());
        {SINK}::count("retained.checked_records", stores.values().map(|(_, records)| *records).sum());
        self.environment_cache.residual_cache_counts();
    }}
'''))
    change(root, 'crates/notist-cli/src/ssg.rs', lambda text: once(text,
           '    notist::xiv_profile::measure("drop.vault", || drop(vault));',
           '    vault.residual_cache_counts();\n    notist::xiv_profile::measure("drop.vault", || drop(vault));'))


def prepare(args):
    base = args.base.resolve()
    out = args.out.resolve()
    primary = guard(SimpleNamespace(out=base))
    assert not out.exists() and REPO not in out.parents and out != REPO
    out.mkdir(parents=True)
    for name in ['bin', 'logs', 'inventories', 'sources', 'retired', 'reproducer']:
        (out / name).mkdir()
    for workload, _, _ in WORKLOADS:
        shutil.copytree(base / 'sites' / workload, out / 'sites' / workload,
                        ignore=shutil.ignore_patterns('dist', '__pycache__'))
        shutil.copy2(base / 'inventories' / f'{workload}-expected.json',
                     out / 'inventories' / f'{workload}-expected.json')
    for path in (REPO / 'tools').glob('*.py'):
        shutil.copy2(path, out / 'reproducer' / path.name)
    manifest = {'kind': 'after-only-residual-analysis', 'primary_directory': str(base),
                'base_revision': primary['base_revision'], 'production_patch_sha256': primary['patch_sha256'],
                'inputs': fingerprints(out / 'sites', exclude_dist=True),
                'sources': {}, 'binaries': {}, 'build_env': primary['build_env'],
                'cpu': args.cpu, 'rounds': 3, 'warmups': 1,
                'not_used_for_primary_speedups': True}
    for mode, rewrite in [('io-probe', io_probe), ('evaluator-probe', evaluator_probe)]:
        source = out / 'sources' / mode
        shutil.copytree(base / 'sources/after-probe', source)
        rewrite(source)
        manifest['sources'][mode] = fingerprints(source)
    save(out / 'manifest.json', manifest)
    print('prepared', manifest['production_patch_sha256'], flush=True)


def verify(args, binaries=True):
    out = args.out.resolve()
    manifest = json.loads((out / 'manifest.json').read_text())
    guard(SimpleNamespace(out=Path(manifest['primary_directory'])))
    assert sha(git(REPO, 'diff', '--binary', '214de5f', '--', 'src', 'crates', 'Cargo.toml', 'Cargo.lock')) == manifest['production_patch_sha256']
    assert fingerprints(out / 'sites', exclude_dist=True) == manifest['inputs']
    for mode, expected in manifest['sources'].items():
        assert fingerprints(out / 'sources' / mode) == expected, mode
    if binaries:
        for mode, expected in manifest['binaries'].items():
            assert sha((out / 'bin' / mode).read_bytes()) == expected, mode
        if 'allocator_observer' in manifest:
            observer = manifest['allocator_observer']
            assert sha((out / 'opendir-probe.so').read_bytes()) == observer['library_sha256']
            assert sha((out / 'reproducer/residual_opendir_probe.c').read_bytes()) == observer['source_sha256']
        if 'ownership_trace_source' in manifest:
            evidence = Path(manifest['ownership_trace_source'])
            proof = json.loads((evidence / 'manifest.json').read_text())
            assert fingerprints(evidence / 'source') == proof['sources']
    return manifest


def build(args):
    out = args.out.resolve()
    manifest = verify(args, False)
    target = args.base.resolve() / 'target-after-probe'
    env = dict(os.environ, **manifest['build_env'], CARGO_TARGET_DIR=str(target),
               NO_COLOR='1', TERM='dumb')
    for mode in manifest['sources']:
        if args.mode and mode != args.mode:
            continue
        with (out / 'logs' / f'build-{mode}.log').open('wb') as log:
            subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'notist-cli',
                            '--bin', 'notist', '--features', 'profiling'],
                           cwd=out / 'sources' / mode, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(target / 'release/notist', out / 'bin' / mode)
        manifest['binaries'][mode] = sha((out / 'bin' / mode).read_bytes())
        save(out / 'manifest.json', manifest)
        print('built', mode, flush=True)


def run(args):
    out = args.out.resolve()
    verify(args)
    assert not (out / 'samples.json').exists()
    rows = []

    def keep(mode, workload, scenario, rep, label):
        row = process(args, mode, workload, scenario, label)
        row['round'] = rep
        rows.append(row)
        save(out / 'samples.json', rows)
        print(label, round(row['wall_s'], 4), flush=True)

    for workload, _, _ in WORKLOADS:
        keep('io-probe', workload, 'first', -2, f'seed-{workload}')
    for mode in ['io-probe', 'evaluator-probe']:
        for rep in range(-1, 3):
            for workload, _, _ in WORKLOADS:
                keep(mode, workload, 'same', rep, f'{mode}-{workload}-same-{rep}')
    for rep in range(-1, 3):
        keep('io-probe', 'xiv', 'replace', rep, f'io-probe-xiv-replace-{rep}')
    verify(args)


def extend(args):
    out = args.out.resolve()
    manifest = verify(args)
    assert 'checker-probe' not in manifest['sources']
    for name in ['manifest.json', 'samples.json', 'summary.json', 'integrity.json']:
        shutil.copy2(out / name, out / ('initial-' + name))
    source = out / 'sources/checker-probe'
    shutil.copytree(args.base.resolve() / 'sources/after-probe', source)
    checker_probe(source)
    manifest['sources']['checker-probe'] = fingerprints(source)
    save(out / 'manifest.json', manifest)


def run_checker(args):
    out = args.out.resolve()
    verify(args)
    rows = json.loads((out / 'samples.json').read_text())
    assert not any(s['mode'] == 'checker-probe' for s in rows)
    for rep in range(-1, 3):
        for workload, _, _ in WORKLOADS:
            label = f'checker-probe-{workload}-same-{rep}'
            row = process(args, 'checker-probe', workload, 'same', label)
            row['round'] = rep
            rows.append(row)
            save(out / 'samples.json', rows)
            print(label, round(row['wall_s'], 4), flush=True)
    verify(args)


def trace(args):
    out = args.out.resolve()
    manifest = verify(args)
    base = args.base.resolve()
    proof = json.loads((base / 'publication-checks/manifest.json').read_text())
    assert fingerprints(base / 'publication-checks/source') == proof['sources']
    binary = base / 'bin/publication-checks-probe'
    assert sha(binary.read_bytes()) == proof['binary_sha256']
    mode = 'ownership-trace-probe'
    shutil.copy2(binary, out / 'bin' / mode)
    manifest['binaries'][mode] = proof['binary_sha256']
    manifest['ownership_trace_source'] = str(base / 'publication-checks')
    save(out / 'manifest.json', manifest)
    row = process(args, mode, 'xiv', 'same', 'ownership-trace-xiv-same', prefix=[
        'strace', '-qq', '-f', '-ttt', '-T', '-e',
        'trace=openat,openat2,close,read,write,statx,getdents64,brk,mmap,munmap,madvise',
        '-o', str(out / 'logs/ownership.strace')])
    row['not_a_performance_sample'] = True
    save(out / 'trace.json', [row])
    verify(args)


def allocator(args):
    out = args.out.resolve()
    manifest = verify(args)
    assert not os.environ.get('LD_PRELOAD'), 'do not replace a pre-existing allocator interposer'
    assert not (out / 'allocator-samples.json').exists()
    source = REPO / 'tools/residual_opendir_probe.c'
    shutil.copy2(source, out / 'reproducer' / source.name)
    library = out / 'opendir-probe.so'
    command = ['cc', '-shared', '-fPIC', '-O2', '-fno-builtin', str(source), '-ldl', '-o', str(library)]
    with (out / 'logs/build-allocator.log').open('wb') as log:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    binary = args.base.resolve() / 'bin/publication-checks-probe'
    assert sha(binary.read_bytes()) == manifest['binaries']['ownership-trace-probe']
    mode = 'ownership-allocator-probe'
    shutil.copy2(binary, out / 'bin' / mode)
    manifest['binaries'][mode] = sha(binary.read_bytes())
    manifest['allocator_observer'] = {'source_sha256': sha(source.read_bytes()),
         'library_sha256': sha(library.read_bytes()), 'command': command,
         'same_glibc_allocator': True, 'not_used_for_speedups': True}
    save(out / 'manifest.json', manifest)
    rows = []
    for rep in range(-1, 3):
        for workload in ['xiv', 'private']:
            label = f'allocator-{workload}-same-{rep}'
            try:
                os.environ['LD_PRELOAD'] = str(library)
                row = process(args, mode, workload, 'same', label)
            finally:
                os.environ.pop('LD_PRELOAD', None)
            row['round'] = rep
            row['not_a_performance_sample'] = True
            rows.append(row)
            save(out / 'allocator-samples.json', rows)
            print(label, round(row['wall_s'], 4), flush=True)
    verify(args)


def report(args):
    out = args.out.resolve()
    manifest = verify(args)
    samples = json.loads((out / 'samples.json').read_text())
    groups = []
    for mode in manifest['sources']:
        for workload, _, _ in WORKLOADS:
            for scenario in ['same', 'replace']:
                selected = [s for s in samples if s['mode'] == mode and s['workload'] == workload
                            and s['scenario'] == scenario and s['round'] >= 0]
                if not selected:
                    continue
                names = sorted({k for s in selected for k in s['metrics']})
                groups.append({'mode': mode, 'workload': workload, 'scenario': scenario,
                               'rounds': len(selected),
                               'metrics': {k: {field: statistics.mean(s['metrics'].get(k, {}).get(field, 0) for s in selected)
                                               for field in ['calls', 'seconds', 'self_seconds']} for k in names},
                               'wall_s': {'median': statistics.median(s['wall_s'] for s in selected),
                                          'min': min(s['wall_s'] for s in selected),
                                          'max': max(s['wall_s'] for s in selected)}})
    traces = json.loads((out / 'trace.json').read_text()) if (out / 'trace.json').exists() else []
    allocations = json.loads((out / 'allocator-samples.json').read_text()) if (out / 'allocator-samples.json').exists() else []
    files = html = 0
    for sample in samples + traces + allocations:
        actual = json.loads((out / 'inventories' / f"{sample['label']}.json").read_text())
        assert actual == json.loads((out / 'inventories' / f"{sample['workload']}-expected.json").read_text())
        files += actual['files']
        html += actual['html']
        assert abs(sample['metrics']['cli.total']['seconds'] - sum(v['self_seconds'] for v in sample['metrics'].values())) < 1e-7
    save(out / 'summary.json', {'kind': manifest['kind'], 'groups': groups})
    save(out / 'integrity.json', {'complete_builds': len(samples) + len(traces) + len(allocations), 'file_hashes_verified': files,
         'html_outputs_verified': html, 'all_outputs_identical': True,
         'production_patch_and_primary_snapshots_unchanged': True,
         'self_times_partition_roots': True, 'io_and_evaluator_cohorts_separate': True,
         'checker_trace_and_allocator_cohorts_separate': True,
         'not_used_for_primary_speedups': True})
    derive_evidence(out, groups, allocations)
    print('verified', len(samples) + len(traces) + len(allocations), 'complete builds', flush=True)


def allocator_events(text):
    pattern = re.compile(r'\[residual-allocator\] pid=(\d+) kind=(\w+) size=(\d+) malloc_calls=(\d+) wall_ns=(\d+) cpu_ns=(\d+) path=(.*)')
    events = []
    for line in text.splitlines():
        match = pattern.fullmatch(line)
        if not match:
            continue
        pid, kind, size, calls, wall, cpu, path = match.groups()
        events.append({'pid': int(pid), 'kind': kind, 'size': int(size), 'calls': int(calls),
                       'wall_s': int(wall) / 1e9, 'cpu_s': int(cpu) / 1e9, 'path': path})
    return events


def derive_evidence(out, groups, allocations):
    # These generated fixtures use literal, line-leading imports; this is an
    # inventory traversal, not a second production language parser.
    imports = re.compile(rb'(?m)^\s*#?import\s+"([^"\n]+)"')
    structures = {}
    for workload in ['shared', 'private', 'wide']:
        root = out / 'sites' / workload / 'site'
        seen = {}

        def visit(path):
            path = path.resolve()
            if path in seen:
                return
            source = path.read_bytes()
            seen[path] = source
            for imported in imports.findall(source):
                visit(path.parent / imported.decode('utf-8'))

        for document in sorted((root / 'content').rglob('*.not')):
            for imported in imports.findall(document.read_bytes()):
                visit(document.parent / imported.decode('utf-8'))
        unique = {sha(source): source for source in seen.values()}
        structures[workload] = {'reachable_modules': len(seen), 'unique_source_texts': len(unique),
            'total_source_bytes': sum(map(len, seen.values())),
            'unique_source_bytes': sum(map(len, unique.values())),
            'modules': {str(path.relative_to(root)): {'bytes': len(source), 'sha256': sha(source)}
                        for path, source in sorted(seen.items())}}
    save(out / 'structural-inputs.json', structures)
    observations = []
    for row in allocations:
        events = allocator_events((out / 'logs' / f"{row['label']}.stderr").read_text())
        observations.append({'label': row['label'], 'workload': row['workload'], 'round': row['round'],
                             'events': [event for event in events if '/site/dist/' in event['path']]})
    save(out / 'allocator-observations.json', observations)
    tree = []
    for group in groups:
        if group['mode'] != 'evaluator-probe':
            continue
        metrics = group['metrics']
        nodes = metrics['work.output_nodes']['calls']
        visits = metrics.get('work.bind_budget_visits', {}).get('calls', 0) + metrics.get('work.present_budget_visits', {}).get('calls', 0)
        shells = metrics['tree.boundary_copy']['calls']
        tree.append({'workload': group['workload'], 'output_nodes': nodes,
                     'budget_visits': visits, 'budget_visits_per_output': visits / nodes,
                     'boundary_copies': shells, 'boundary_copies_per_output': shells / nodes,
                     'code_calls': metrics.get('work.runtime_code_calls', {}).get('calls', 0),
                     'native_calls': metrics.get('work.runtime_native_calls', {}).get('calls', 0)})
    save(out / 'structural-work.json', tree)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'build', 'run', 'report', 'extend', 'run_checker', 'trace', 'allocator'])
    parser.add_argument('--base', type=Path, default=Path('/tmp/opencode/notist-optimized-profile-20261103'))
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--cpu', type=int, default=8)
    parser.add_argument('--mode', choices=['io-probe', 'evaluator-probe', 'checker-probe'])
    args = parser.parse_args()
    globals()[args.action](args)


if __name__ == '__main__':
    main()
