#!/usr/bin/env python3
"""After-only field teardown probe, based on the frozen original-order cohort."""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess

from profile_icexiv import inventory, process
from profile_xiv_history import once, save, sha


def instrument(source, schema=None):
    prefix = 'crate::xiv_profile'
    vault = source / 'src/vault.rs'
    fields = ['resources', 'pipeline', 'extensions', 'environments', 'environment_cache',
              'identity', 'config', 'module_urls', 'evaluation_limits']
    code = '\nimpl<R: Resources> Vault<R> {\n    #[doc(hidden)]\n    pub fn profile_drop(self) {\n'
    code += f'        {prefix}::count("objects.vault.environments", self.environments.len());\n'
    code += '        let Self { ' + ', '.join(fields) + ' } = self;\n'
    for field in fields:
        operation = f'{field}.profile_drop()' if field in ['resources', 'environment_cache'] else f'drop({field})'
        code += f'        {prefix}::measure("vault.drop.{field}", || {operation});\n'
    code += '    }\n}\n'
    vault.write_text(vault.read_text() + code)

    session = source / 'src/resource_session.rs'
    fields = ['resources', 'roots', 'limits', 'state', 'data_cache']
    state_fields = ['files', 'kinds', 'sources', 'texts', 'dependencies', 'bytes', 'requests']
    code = '\nimpl<R: Resources> ResourceSession<R> {\n    pub(crate) fn profile_drop(self) {\n'
    code += '        let Self { ' + ', '.join(fields) + ' } = self;\n'
    for field in fields:
        if field == 'state':
            operation = 'drop_profile_state(state.into_inner())'
        else:
            operation = f'drop({field})'
        code += f'        {prefix}::measure("session.drop.{field}", || {operation});\n'
    code += '    }\n}\nfn drop_profile_state(state: State) {\n'
    for field in state_fields[:5]:
        code += f'    {prefix}::count("objects.session.{field}", state.{field}.len());\n'
    code += '    let State { ' + ', '.join(state_fields) + ' } = state;\n'
    for field in state_fields:
        code += f'    {prefix}::measure("session.state.drop.{field}", || drop({field}));\n'
    code += '}\n'
    session.write_text(session.read_text() + code)

    loader = source / 'src/environment/loader.rs'
    loader.write_text(loader.read_text() + '\nimpl EnvironmentCache {\n'
        '    pub(crate) fn profile_drop(self) {\n'
        '        crate::xiv_profile::count("objects.environment.graphs", self.graphs.len());\n'
        '        crate::xiv_profile::count("objects.environment.configs", self.configs.len());\n'
        '        let Self { graphs, configs } = self;\n'
        '        crate::xiv_profile::measure("environment_cache.drop.graphs", || drop(graphs));\n'
        '        crate::xiv_profile::measure("environment_cache.drop.configs", || drop(configs));\n'
        '    }\n}\n')
    cli = source / 'crates/notist-cli/src/ssg.rs'
    cli.write_text(once(cli.read_text(), '|| drop(vault)', '|| vault.profile_drop()'))
    if schema is not None:
        sink = source / 'crates/notist-core/src/current_profile.rs'
        code = '\npub fn reserve_probe() {\n    STATE.with(|state| {\n        let mut state = state.borrow_mut();\n        state.stack.reserve(32);\n'
        for name in schema['metrics']:
            code += f'        state.metrics.entry("{name}").or_default();\n'
        for edge in schema['edges']:
            code += f'        state.edges.entry(("{edge["parent"]}", "{edge["name"]}")).or_default();\n'
        code += '    });\n}\n'
        sink.write_text(sink.read_text() + code)
        adapter = source / 'src/xiv_profile.rs'
        adapter.write_text(adapter.read_text() + '\npub use notist_core::current_profile::reserve_probe;\n')
        main = source / 'crates/notist-cli/src/main.rs'
        main.write_text(once(main.read_text(), 'let _report = notist::xiv_profile::Report;',
                            'notist::xiv_profile::reserve_probe();\nlet _report = notist::xiv_profile::Report;'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--prewarm', action='store_true', help='reserve timing records before building, using the first field-probe schema')
    args = parser.parse_args()
    args.out = args.out.resolve()
    out = args.out
    kind = 'teardown-prewarm' if args.prewarm else 'teardown'
    source = out / 'sources' / kind
    shutil.copytree(out / 'sources/ordered', source)
    schema = json.loads((out / 'teardown.json').read_text())['samples'][0] if args.prewarm else None
    instrument(source, schema)
    patch = []
    for path in source.rglob('*.rs'):
        rel = path.relative_to(source)
        before = (out / 'sources/ordered' / rel).read_text()
        patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True), fromfile='a/' + str(rel), tofile='b/' + str(rel)))
    (out / f'{kind}.patch').write_text(''.join(patch))
    env = dict(os.environ, CARGO_TARGET_DIR=str(out / 'target'), CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='2', CARGO_PROFILE_RELEASE_DEBUG='1')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=source, env=env))
    packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    with (out / 'logs' / f'build-{kind}.log').open('wb') as log:
        subprocess.run(['cargo', 'clean', '--release', *[flag for p in packages for flag in ['-p', p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'notist-cli', '--bin', 'notist', '--features', 'profiling'], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(out / 'target/release/notist', out / 'bin' / kind)
    shutil.copy2(Path(__file__), out / f'{kind}-reproducer.py')
    proof = {'sources': inventory(source), 'binary': sha((out / 'bin' / kind).read_bytes()), 'samples': []}
    for rep in range(-1, 3):
        row = process(args, kind, f'{kind}-{rep}')
        row['round'] = rep
        proof['samples'].append(row)
        save(out / f'{kind}.json', proof)
    assert inventory(source) == proof['sources']


if __name__ == '__main__':
    main()
