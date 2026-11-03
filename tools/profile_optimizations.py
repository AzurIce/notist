#!/usr/bin/env python3
"""Same-batch full-workload comparison of the exact optimization patch."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

from profile_current import ADAPTER, MODULE_STATS, edit, entries, parse_detail
from profile_history_matrix import WORKLOADS, inventory
from profile_resource_overhead import fingerprints, parse_strace
from profile_xiv_history import REPO, extract, git, once, save, sha
from xiv_history_probes import enter_function, measure_rhs

OLD = Path('/tmp/opencode/notist-current-profile-20261103')


def instrument(root):
    core = root / 'crates/notist-core/src'
    shutil.copy2(REPO / 'tools/current_profile_probe.rs', core / 'current_profile.rs')
    with (core / 'lib.rs').open('a') as file:
        file.write('\n#[doc(hidden)]\npub mod current_profile;\n')
    with (root / 'src/lib.rs').open('a') as file:
        file.write('\n#[doc(hidden)]\npub mod xiv_profile;\n')
    (root / 'src/xiv_profile.rs').write_text(ADAPTER.replace('__MODULE_STATS__', MODULE_STATS))
    def site(text):
        def measure_build(text, binding, end, label):
            start = text.index('pub(crate) fn build')
            return text[:start] + measure_rhs(text[start:], binding, end, label)
        text = enter_function(text, 'build', '    let _total = notist::xiv_profile::span("ssg.total");\n    let phase = notist::xiv_profile::span("ssg.discover");')
        for anchor, label in [
            ('    let routes = crate::extensions::prepare', 'ssg.prepare'),
            ('    let mut source_dirs: BTreeSet<_> =', 'ssg.source_directories'),
            ('    let mut documents = vec![];', 'ssg.pipeline'),
            ('    let mut assets = Assets::new(output);', 'ssg.assets_templates'),
            ('    let by_source = DocumentIndex::new(&documents);', 'ssg.bodies'),
            ('    let dirs = directories(&pages, &source_dirs, &content);', 'ssg.shared_context'),
            ('    let workers = if pages.len() >= 1024 {', 'ssg.page_templates'),
            ('    Ok(BuildOutput {', 'ssg.finish'),
        ]:
            text = once(text, anchor, f'    drop(phase);\n    let phase = notist::xiv_profile::span("{label}");\n'+anchor)
        text = measure_build(text, 'let (analysis, handle) =', ';\n        errors.extend(', 'pipeline.analyze')
        text = measure_build(text, 'let evaluation = ', ';\n        errors.extend(', 'pipeline.evaluate')
        text = measure_build(text, 'let (registry, environment) = ', ';\n        let rendered', 'body.registry')
        text = measure_build(text, 'let rendered = ', ';\n        errors.extend(rendered', 'body.html')
        text = measure_build(text, 'let registrations = ', ';\n        if !registrations', 'body.components')
        text = once(text, '        let mut resolved = BTreeMap::new();', '        let profile_links = notist::xiv_profile::span("body.links");\n        let mut resolved = BTreeMap::new();')
        text = once(text, '        let (registry, environment) = ', '        drop(profile_links);\n        let (registry, environment) = ')
        text = once(text, '        assets.write_reserved(&page.route.output, bytes)?;', '        notist::xiv_profile::measure("template.output", || assets.write_reserved(&page.route.output, bytes))?;')
        text = measure_rhs(text, 'let pages: Vec<_> = ', ';\n        let directories:', 'template.shared_pages')
        text = measure_rhs(text, 'let directories: BTreeMap<_, _> = ', ';\n        let site = ', 'template.shared_directories')
        text = once(text, 'Value::try_from_serializable(data).map_err', 'notist::xiv_profile::measure("template.shared_data", || Value::try_from_serializable(data)).map_err')
        text = once(text, '    drop(data);', '    notist::xiv_profile::measure("drop.site_json", || drop(data));')
        text = once(text, '    Ok(BuildOutput {', '    let result = Ok(BuildOutput {')
        drops = '\n'.join(f'    notist::xiv_profile::measure("drop.{label}", || drop({name}));' for name,label in [
            ('values','template_values'),('dirs','directory_maps'),('components','component_map'),
            ('by_source','source_index'),('engine','template_engine'),('handles','document_handles'),('documents','document_trees')])
        return once(text, '        warnings,\n    })\n}', '        warnings,\n    });\n    drop(phase);\n'+drops+'\n    result\n}')
    edit(root, 'crates/notist-ssg/src/build.rs', site)
    entries(root, 'crates/notist-ssg/src/build.rs', [('render_templates','template.render_all')], 'notist::xiv_profile')
    for relative, methods in [
        ('src/vault.rs', [('ensure_environment','vault.ensure_environment'),('compile_bound','vault.compile'),('evaluate_in','vault.evaluate_in'),('evaluate_document','vault.evaluate_document'),('html_registry_for_document','vault.html_registry_document')]),
        ('src/resource_session.rs',[('source_bytes','input.source'),('load','input.load'),('data','input.data'),('register_source','input.register_document'),('source_shared','input.text')]),
        ('src/environment/loader.rs',[('document','environment.document'),('visit','environment.visit')]),
        ('src/environment/config.rs',[('parse_config','environment.parse_config')]),
        ('crates/notist-pipeline/src/frontend/markup/mod.rs',[('compile','frontend.notist')]),
        ('crates/notist-pipeline/src/code/check.rs',[('document','check.document'),('ensure_instance','check.instance')]),
        ('crates/notist-pipeline/src/code/content.rs',[('content','check.content')]),
        ('crates/notist-pipeline/src/materialize.rs',[('materialize','check.materialize')]),
        ('crates/notist-pipeline/src/code/eval.rs',[('run','eval.run')]),
        ('crates/notist-pipeline/src/context.rs',[('checked_program','plan.lookup'),('remember_checked_program','plan.store')]),
        ('crates/notist-core/src/data/parse.rs',[('parse','data.parse_actual')]),
        ('crates/notist-pipeline/src/code/data.rs',[('read_data','data.read')]),
    ]:
        entries(root, relative, methods, 'crate::current_profile' if relative.startswith('crates/notist-core/') else 'notist_core::current_profile')
    def eval_steps(text):
        counted = 'let (forest, _) = ' in text
        forest = 'let (forest, _) = ' if counted else 'let forest = '
        roots = 'let (mut roots, _) = ' if counted else 'let mut roots = '
        text = measure_rhs(text, forest, ';\n    let ', 'eval.bind_content', 'notist_core::current_profile')
        return measure_rhs(text, roots, ';\n    if !e.needed', 'eval.present', 'notist_core::current_profile')
    edit(root, 'crates/notist-pipeline/src/code/eval.rs', eval_steps)
    edit(root, 'crates/notist-pipeline/src/frontend/markup/mod.rs', lambda text: once(text,
        'let parsed = markup::parse_document(source);',
        'let parsed = notist_core::current_profile::measure("frontend.notist_parse", || markup::parse_document(source));'))
    edit(root, 'src/resources.rs', lambda text: once(text,
        'impl ResourceReader for FsReader {\n    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {',
        'impl ResourceReader for FsReader {\n    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {\n        let _io = notist_core::current_profile::span("io.bounded_read");'))
    def transport(text):
        text = enter_function(text,'exchange','    let _total = crate::xiv_profile::span("stdio.total");\n    let phase = crate::xiv_profile::span("stdio.encode");')
        text = once(text,'    let mut child = Command::new(program)','    drop(phase);\n    let phase = crate::xiv_profile::span("stdio.process");\n    let mut child = Command::new(program)')
        return once(text,'    serde_json::from_slice(&output.stdout)','    drop(phase);\n    let _decode = crate::xiv_profile::span("stdio.decode");\n    serde_json::from_slice(&output.stdout)')
    edit(root,'src/stdio.rs',transport)
    def publication(text):
        text = enter_function(text,'publish','    let _publish = notist::xiv_profile::span("cli.publish");')
        text = enter_function(text,'unchanged','    let _compare = notist::xiv_profile::span("publish.compare");')
        text = once(text,'    for (relative, bytes) in files {','    let write = notist::xiv_profile::span("publish.write");\n    for (relative, bytes) in files {')
        text = once(text,'    let backup = tempfile::Builder::new()','    drop(write);\n    let backup = tempfile::Builder::new()')
        return once(text,'    Ok(())\n}\n\n#[cfg(test)]','    drop(retired);\n    notist::xiv_profile::measure("publish.cleanup_retired", || drop(backup));\n    notist::xiv_profile::measure("publish.cleanup_staging", || drop(staging));\n    Ok(())\n}\n\n#[cfg(test)]')
    edit(root,'crates/notist-cli/src/publication.rs',publication)
    edit(root,'crates/notist-cli/src/ssg.rs',lambda text: once(once(text,
        '    result\n        .map(|artifact|','    let result = result\n        .map(|artifact|'),
        '        .map_err(|e| (e, observed))\n}',
        '        .map_err(|e| (e, observed));\n    notist::xiv_profile::measure("drop.vault", || drop(vault));\n    result\n}'))
    edit(root,'crates/notist-cli/src/main.rs',lambda text: enter_function(text,'main',
        '    let _report = notist::xiv_profile::Report;\n    let _total = notist::xiv_profile::span("cli.total");'))


def prepare(args):
    out=args.out.resolve()
    assert not out.exists() and out!=REPO and REPO not in out.parents
    old=json.loads((OLD/'manifest.json').read_text())
    assert subprocess.check_output(['rustc','-Vv'],text=True)==old['rustc']
    out.mkdir(parents=True)
    for name in ['sources','bin','logs','inventories','retired']:(out/name).mkdir()
    patch=git(REPO,'diff','--binary','214de5f','--','src','crates','Cargo.toml','Cargo.lock')
    assert patch and git(REPO,'diff','--','Cargo.lock')==b''
    (out/'optimization.patch').write_bytes(patch)
    source=out/'sources/after'
    extract(REPO,'214de5f',source)
    subprocess.run(['git','apply',str(out/'optimization.patch')],cwd=source,check=True)
    # Only tracked production changes are applied to a known disposable archive.
    normal=fingerprints(source)
    for rel,value in normal.items():
        if rel.endswith('.rs') and (rel.startswith('src/') or rel.startswith('crates/') and '/src/' in rel):
            assert sha((REPO/rel).read_bytes())==value,rel
    for mode,key in [('before','normal'),('before-probe','detail')]:
        binary=OLD/'bin'/key
        assert sha(binary.read_bytes())==old['binaries'][key]
        shutil.copy2(binary,out/'bin'/mode)
    shutil.copytree(source,out/'sources/after-probe')
    instrument(out/'sources/after-probe')
    for workload,_,_ in WORKLOADS:
        shutil.copytree(OLD/'sites'/workload,out/'sites'/workload,ignore=shutil.ignore_patterns('dist','__pycache__'))
        shutil.copy2(OLD/'inventories'/f'{workload}-expected.json',out/'inventories'/f'{workload}-expected.json')
    shutil.copytree(OLD/'sites/data-smoke',out/'sites/data-smoke',ignore=shutil.ignore_patterns('dist','__pycache__'))
    shutil.copy2(OLD/'inventories/data-smoke-expected.json',out/'inventories/data-smoke-expected.json')
    m={'base_revision':old['revision'],'patch_sha256':sha(patch),'rustc':old['rustc'],'cpu':args.cpu,
       'sources':{'after':normal,'after-probe':fingerprints(out/'sources/after-probe')},
       'inputs':fingerprints(out/'sites',exclude_dist=True),'binaries':{k:sha((out/'bin'/k).read_bytes()) for k in ['before','before-probe']},
       'build_env':old['build_env'],'baseline_artifact_source':str(OLD),'normal_rounds':{'same':5,'replace':3,'first':3},'probe_rounds':3}
    (out/'reproducer').mkdir()
    for path in (REPO/'tools').glob('*.py'):shutil.copy2(path,out/'reproducer'/path.name)
    shutil.copy2(REPO/'tools/current_profile_probe.rs',out/'reproducer/current_profile_probe.rs')
    save(out/'manifest.json',m)
    print('prepared',m['patch_sha256'],flush=True)


def guard(args,binaries=True):
    out=args.out.resolve();m=json.loads((out/'manifest.json').read_text())
    assert fingerprints(out/'sites',exclude_dist=True)==m['inputs']
    for mode,expected in m['sources'].items():assert fingerprints(out/'sources'/mode)==expected,mode
    if binaries:
        for mode,value in m['binaries'].items():assert sha((out/'bin'/mode).read_bytes())==value,mode
    return m


def build(args):
    out=args.out.resolve();m=guard(args,False)
    for mode in ['after','after-probe']:
        env=dict(os.environ,**m['build_env'],CARGO_TARGET_DIR=str(out/f'target-{mode}'),NO_COLOR='1',TERM='dumb')
        command=['cargo','build','--locked','--release','-p','notist-cli','--bin','notist']
        if mode.endswith('probe'):command+=['--features','profiling']
        with (out/'logs'/f'build-{mode}.log').open('wb') as log:
            subprocess.run(command,cwd=out/'sources'/mode,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        shutil.copy2(out/f'target-{mode}/release/notist',out/'bin'/mode)
        m['binaries'][mode]=sha((out/'bin'/mode).read_bytes());save(out/'manifest.json',m)
        print('built',mode,flush=True)


def output_root(out,workload):
    return out/'sites'/workload/'site/dist'/('xiv-db' if workload=='xiv' else 'bench')


def process(args,mode,workload,scenario,label,affinity=None,prefix=None):
    out=args.out.resolve();root=output_root(out,workload)
    if scenario=='first' and root.exists():
        assert (root/'_notist/publication.json').is_file()
        retired=out/'retired'/label
        root.rename(retired)
        # Only this profiler's own output is removed, outside the first-build timer.
        shutil.rmtree(retired)
    elif scenario=='replace':
        assert root.exists()
        (root/'index.html').write_bytes(b'profiling output replacement control\n')
    command=['taskset','-c',affinity or str(args.cpu),*(prefix or []),str(out/'bin'/mode),'build']
    env=dict(os.environ,NO_COLOR='1',TERM='dumb',PYTHONDONTWRITEBYTECODE='1')
    for key in ['RESOURCE_PROFILE','RESOURCE_DIAGNOSTIC','XIV_PROFILE_REQUESTS']:env.pop(key,None)
    start=time.perf_counter()
    with (out/'logs'/f'{label}.stdout').open('wb') as stdout,(out/'logs'/f'{label}.stderr').open('wb') as stderr:
        child=subprocess.Popen(command,cwd=out/'sites'/workload/'site',env=env,stdout=stdout,stderr=stderr)
        _,status,usage=os.wait4(child.pid,0);child.returncode=os.waitstatus_to_exitcode(status)
    row={'mode':mode,'workload':workload,'scenario':scenario,'label':label,'command':command,'wall_s':time.perf_counter()-start,
         'user_s':usage.ru_utime,'system_s':usage.ru_stime,'exit':child.returncode}
    save(out/'logs'/f'{label}.json',row)
    assert row['exit']==0,row
    if mode.endswith('probe'):
        row['metrics'],row['edges'],row['modules']=parse_detail((out/'logs'/f'{label}.stderr').read_text())
    actual=inventory(root);expected=json.loads((out/'inventories'/f'{workload}-expected.json').read_text())
    assert actual==expected,label
    save(out/'inventories'/f'{label}.json',actual)
    return row


def run(args):
    out=args.out.resolve();m=guard(args);samples=[]
    def keep(row,rep):
        row['round']=rep;samples.append(row);save(out/'samples.json',samples)
        print(row['label'],round(row['wall_s'],4),flush=True)
    # Seed valid old outputs; every scenario and binary then has its own warmup.
    for workload,_,_ in WORKLOADS:
        keep(process(args,'before',workload,'first',f'seed-{workload}'),-2)
    for scenario,rounds in m['normal_rounds'].items():
        for rep in range(-1,rounds):
            for wi,(workload,_,_) in enumerate(WORKLOADS):
                modes=['before','after'] if (rep+wi)%2 else ['after','before']
                for mode in modes:
                    label=f'{scenario}-{rep}-{workload}-{mode}'
                    keep(process(args,mode,workload,scenario,label),rep)
    for rep in range(-1,m['probe_rounds']):
        for wi,(workload,_,_) in enumerate(WORKLOADS):
            for mode in (['before-probe','after-probe'] if (rep+wi)%2 else ['after-probe','before-probe']):
                keep(process(args,mode,workload,'same',f'probe-{rep}-{workload}-{mode}'),rep)
        for mode in ['before-probe','after-probe']:
            keep(process(args,mode,'xiv','replace',f'probe-replace-{rep}-{mode}'),rep)
    for mode in ['before','after','after-probe']:
        keep(process(args,mode,'data-smoke','same',f'data-control-{mode}'),-2)
    # Four-core supplement: not mixed into the pinned single-core comparison.
    affinity=','.join(str(args.cpu+i) for i in range(4))
    for rep in range(-1,3):
        for mode in (['before','after'] if rep%2 else ['after','before']):
            keep(process(args,mode,'xiv','same',f'multicore-{rep}-{mode}',affinity=affinity),rep)
    guard(args)


def trace(args):
    out=args.out.resolve();guard(args);rows=[]
    for mode in ['before','after']:
        for scenario in ['same','replace']:
            label=f'trace-{scenario}-{mode}';path=out/'logs'/f'{label}.strace'
            row=process(args,mode,'xiv',scenario,label,prefix=['strace','-qq','-f','-c','-e','trace=%file,close,read,write,getdents64','-o',str(path)])
            row['syscalls']=parse_strace(path.read_text());rows.append(row)
    save(out/'strace.json',rows);guard(args)


def report(args):
    out=args.out.resolve();m=guard(args);samples=json.loads((out/'samples.json').read_text());rows=[]
    for workload,title,pages in WORKLOADS:
        row={'workload':workload,'title':title,'html':pages,'scenarios':{}}
        for scenario in m['normal_rounds']:
            values={}
            for mode in ['before','after']:
                selected=[s for s in samples if s['workload']==workload and s['scenario']==scenario and s['mode']==mode and s['round']>=0 and not s['label'].startswith('multicore')]
                values[mode]={k:{'median':statistics.median(s[k] for s in selected),'mean':statistics.mean(s[k] for s in selected),'min':min(s[k] for s in selected),'max':max(s[k] for s in selected)} for k in ['wall_s','user_s','system_s']}
            values['speedup']=values['before']['wall_s']['median']/values['after']['wall_s']['median'];row['scenarios'][scenario]=values
        row['probes']={}
        for mode in ['before-probe','after-probe']:
            selected=[s for s in samples if s['workload']==workload and s['scenario']=='same' and s['mode']==mode and s['round']>=0]
            names=set().union(*(s['metrics'] for s in selected))
            row['probes'][mode]={name:{field:statistics.mean(s['metrics'].get(name,{}).get(field,0) for s in selected) for field in ['calls','seconds','self_seconds']} for name in sorted(names)}
            row['probes'][mode]['module_work']=selected[0]['modules']
        rows.append(row)
    summary={'base_revision':m['base_revision'],'patch_sha256':m['patch_sha256'],'rows':rows}
    save(out/'summary.json',summary)
    traced=json.loads((out/'strace.json').read_text()) if (out/'strace.json').exists() else []
    preflight=json.loads((out/'preflight.json').read_text()) if (out/'preflight.json').exists() else []
    all_samples=samples+traced+preflight;files=0;html=0
    for sample in all_samples:
        actual=json.loads((out/'inventories'/f"{sample['label']}.json").read_text())
        assert actual==json.loads((out/'inventories'/f"{sample['workload']}-expected.json").read_text())
        files+=actual['files'];html+=actual['html']
    control=next(s for s in samples if s['label']=='data-control-after-probe')
    assert control['metrics']['data.read']['calls']==4 and control['metrics']['data.parse_actual']['calls']==2
    current=git(REPO,'diff','--binary','214de5f','--','src','crates','Cargo.toml','Cargo.lock')
    assert sha(current)==m['patch_sha256'],'production sources changed after snapshot'
    integrity={'complete_builds':len(all_samples),'file_hashes_verified':files,'html_outputs_verified':html,
        'all_outputs_identical':True,'production_patch_unchanged':True,'base_revision':m['base_revision'],'patch_sha256':m['patch_sha256'],
        'normal_probe_and_multicore_samples_separate':True,'data_reads':4,'data_actual_parses':2}
    save(out/'integrity.json',integrity)
    write_visualization(out,summary,integrity,samples)
    print(json.dumps({r['workload']:{k:round(v['speedup'],3) for k,v in r['scenarios'].items()} for r in rows},indent=2))


def write_visualization(out,summary,integrity,samples):
    multicore={}
    for mode in ['before','after']:
        selected=[s for s in samples if s['label'].startswith('multicore-') and s['mode']==mode and s['round']>=0]
        multicore[mode]={k:{'median':statistics.median(s[k] for s in selected),'min':min(s[k] for s in selected),'max':max(s[k] for s in selected)} for k in ['wall_s','user_s','system_s']}
    names=['pipeline.analyze','pipeline.evaluate','eval.run','cli.publish',
           'ssg.assets_templates','ssg.page_templates','ssg.finish']
    plot_rows=[]
    for row in summary['rows']:
        scenarios={}
        for state,values in row['scenarios'].items():
            scenarios[state]={mode:{'wall_s':{key:values[mode]['wall_s'][key] for key in ['median','min','max']}} for mode in ['before','after']}
            scenarios[state]['speedup']=values['speedup']
        probes={mode:{name:{'seconds':metrics.get(name,{}).get('seconds',0)} for name in names} for mode,metrics in row['probes'].items()}
        plot_rows.append({'workload':row['workload'],'html':row['html'],'scenarios':scenarios,'probes':probes})
    data={'summary':dict(summary,rows=plot_rows),'integrity':integrity,'multicore':multicore}
    save(out/'visualization-data.json',data)
    template=(REPO/'tools/optimization_profile_chart.html').read_text()
    assert template.count('__PROFILE_DATA__')==1
    payload=json.dumps(data,ensure_ascii=False).replace('<','\\u003c')
    (out/'comparison.html').write_text(template.replace('__PROFILE_DATA__',payload))


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['prepare','build','run','trace','report'])
    p.add_argument('--out',type=Path,required=True);p.add_argument('--cpu',type=int,default=8);args=p.parse_args()
    globals()[args.action](args)


if __name__=='__main__':main()
