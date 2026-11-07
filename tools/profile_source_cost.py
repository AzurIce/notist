#!/usr/bin/env python3
"""Independent full-site SourceId cost attribution. Never edits production Rust.

All variants are frozen diagnostic copies, not proposed implementations. In
particular unchecked intentionally omits ownership checks: an upper-bound probe
only, not a semantically acceptable optimization or performance candidate.
"""
import argparse
from collections import Counter, defaultdict
import gzip
import json
import math
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import struct
import subprocess
import sys
import threading
import time

from profile_source_identity import digest, inventory, save, BUILD_ENV
from profile_icexiv_refresh import environment
from xiv_history_probes import enter_function

REPO = Path(__file__).resolve().parents[1]
PERF = Path('/nix/store/bw8frr0nk26m740a2mwaw5yi06mgxakb-perf-linux-7.3.1/bin/perf')
SAMPLY = Path('/nix/store/yv2wrdb685wy1dpv6fz7j93hx0d2fcqg-samply-0.13.1/bin/samply')


def prepare(a):
    if a.out.exists() or a.out.is_relative_to(REPO):
        raise ValueError('fresh external directory required')
    m=json.loads((a.reference/'manifest.json').read_text())
    for directory in ['bin','logs','sources','reproducer']:
        (a.out/directory).mkdir(parents=True)
    shutil.copytree(a.reference/'site',a.out/'site',symlinks=True,ignore=shutil.ignore_patterns('dist','__pycache__'))
    assert inventory(a.out/'site')==m['inputs']
    shutil.copy2(a.reference/'output-inventory.json',a.out/'output-inventory.json')
    sources={}
    for kind in ['before','after']:
        source=a.out/'sources'/kind
        shutil.copytree(a.reference/'sources'/kind,source)
        assert inventory(source)==m['sources'][kind]
        sources[kind]=m['sources'][kind]
        shutil.copy2(a.reference/'bin'/kind,a.out/'bin'/kind)
        assert digest(a.out/'bin'/kind)==m['binaries'][kind]
    for path in (REPO/'tools').glob('*.py'):
        shutil.copy2(path,a.out/'reproducer'/path.name)
    for name in ['source_cost_probe.rs','source_alloc_stack.c']:
        shutil.copy2(REPO/'tools'/name,a.out/'reproducer'/name)
    save(a.out/'manifest.json',dict(reference=str(a.reference),inputs=m['inputs'],sources=sources,binaries={k:m['binaries'][k] for k in ['before','after']},build_env=BUILD_ENV,perf=str(PERF),samply=str(SAMPLY),tools={str(p):digest(p) for p in [PERF,SAMPLY]}))


def verify(a):
    m=json.loads((a.out/'manifest.json').read_text())
    for kind,expected in m['sources'].items():
        assert inventory(a.out/'sources'/kind)==expected,kind
    for kind,expected in m['binaries'].items():
        assert digest(a.out/'bin'/kind)==expected,kind
    assert {k:v for k,v in inventory(a.out/'site').items() if not k.startswith('dist/')}==m['inputs']
    assert inventory(a.out/'site/dist/icexiv-db')==json.loads((a.out/'output-inventory.json').read_text())
    return m


def run(a,kind,label,prefix=(),extra=None,rss=False):
    m=json.loads((a.out/'manifest.json').read_text())
    binary=a.out/'bin'/kind
    assert digest(binary)==m['binaries'][kind]
    if (a.out/'logs'/f'{label}.json').exists():
        raise ValueError('do not overwrite evidence: '+label)
    env=dict(environment(),**(extra or {}))
    command=[*map(str,prefix),str(binary),'build']
    points=[]
    stop=threading.Event()
    started=time.perf_counter()
    with (a.out/'logs'/f'{label}.stdout').open('wb') as stdout,(a.out/'logs'/f'{label}.stderr').open('wb') as stderr:
        child=subprocess.Popen(command,cwd=a.out/'site',env=env,stdout=stdout,stderr=stderr)
        def observe():
            while not stop.wait(.025):
                try:
                    fields={s.split(':')[0]:s.split(':')[1].strip() for s in Path(f'/proc/{child.pid}/status').read_text().splitlines() if ':' in s}
                    points.append([time.perf_counter()-started,int(fields.get('VmRSS','0 kB').split()[0]),int(fields.get('RssAnon','0 kB').split()[0])])
                except (OSError,ValueError):
                    pass
        watcher=threading.Thread(target=observe)
        if rss: watcher.start()
        _,status,usage=os.wait4(child.pid,0)
        child.returncode=os.waitstatus_to_exitcode(status)
        elapsed=time.perf_counter()-started
        stop.set()
        if rss: watcher.join()
    row=dict(kind=kind,label=label,command=command,wall_s=elapsed,user_s=usage.ru_utime,system_s=usage.ru_stime,maxrss_kib=usage.ru_maxrss,minor_faults=usage.ru_minflt,major_faults=usage.ru_majflt,voluntary_context_switches=usage.ru_nvcsw,involuntary_context_switches=usage.ru_nivcsw,exit=child.returncode,diagnostic=bool(prefix or extra or rss or label.startswith('ablation-')),load=Path('/proc/loadavg').read_text().strip())
    assert child.returncode==0,(label,child.returncode)
    assert inventory(a.out/'site/dist/icexiv-db')==json.loads((a.out/'output-inventory.json').read_text()),label
    row['complete_output_identical']=True
    save(a.out/'logs'/f'{label}.json',row)
    if rss: save(a.out/'logs'/f'{label}.rss.json',points)
    print(label,round(elapsed,3),flush=True)
    return row


def summarize(rows):
    fields=['wall_s','user_s','system_s','maxrss_kib','minor_faults','major_faults','involuntary_context_switches']
    return {k:{f:statistics.median(r[f] for r in rows if r['kind']==k) for f in fields} for k in sorted({r['kind'] for r in rows})}


def clean(a):
    run(a,'before','initial-publication')
    rows=[]
    for i in range(a.pairs):
        for kind in (['before','after'] if i%2==0 else ['after','before']):
            rows.append(run(a,kind,f'clean-{i}-{kind}'))
            save(a.out/'clean-samples.json',rows)
    summary=summarize(rows)
    ratios=[next(r['wall_s'] for r in rows if r['label']==f'clean-{i}-after')/next(r['wall_s'] for r in rows if r['label']==f'clean-{i}-before') for i in range(a.pairs)]
    rng=random.Random(714)
    boot=sorted(statistics.median(rng.choices(ratios,k=len(ratios))) for _ in range(20000))
    summary['paired_wall']={'ratios':ratios,'median_percent':100*(statistics.median(ratios)-1),'bootstrap_95_percent':[100*(boot[500]-1),100*(boot[19499]-1)],'warning':'local interleaved pairs, not independent machine-level replications'}
    save(a.out/'clean-summary.json',summary)
    verify(a)


def cpu(a):
    rows=[]
    for i in range(2):
        for kind in (['before','after'] if i%2==0 else ['after','before']):
            label=f'cpu-{i}-{kind}'
            raw=a.out/f'{label}.perf.data'
            row=run(a,kind,label,[PERF,'record','--no-kernel-callchains','--user-callchains','--no-bpf-event','-e','cpu-clock:u','-F','499','--call-graph','dwarf,16384','-o',raw,'--'])
            for view,flags in [('self',['--no-children']),('cumulative',['--children'])]:
                with (a.out/f'{label}-{view}.txt').open('wb') as log:
                    subprocess.run([str(PERF),'report','-i',str(raw),'--stdio','--percent-limit','0.1',*flags],stdout=log,stderr=subprocess.STDOUT,check=True)
            with (a.out/'logs'/f'{label}-import.log').open('wb') as log:
                subprocess.run([str(SAMPLY),'import','--save-only','--unstable-presymbolicate','--include-args=2','--profile-name',f'SourceId {kind} #{i}','-o',str(a.out/f'{label}.json.gz'),str(raw)],stdout=log,stderr=subprocess.STDOUT,check=True)
            with (a.out/f'{label}.stacks.txt').open('wb') as log:
                subprocess.run([str(PERF),'script','-i',str(raw)],stdout=log,stderr=subprocess.STDOUT,check=True)
            rows.append(row)
            save(a.out/'cpu-samples.json',rows)
    events='task-clock:u,cycles:u,instructions:u,branches:u,branch-misses:u,context-switches,page-faults,cpu-migrations'
    for i in range(2):
        for kind in (['before','after'] if i%2==0 else ['after','before']):
            run(a,kind,f'counters-{i}-{kind}',[PERF,'stat','-x',';','-e',events,'-o',a.out/f'counters-{i}-{kind}.csv','--'])
    verify(a)


def alloc(a):
    library=a.out/'bin/alloc-stacks.so'
    subprocess.run(['cc','-O2','-shared','-fPIC',str(a.out/'reproducer/source_alloc_stack.c'),'-o',str(library)],check=True)
    rows=[]
    for i in range(2):
        for kind in (['before','after'] if i%2==0 else ['after','before']):
            label=f'alloc-{i}-{kind}'
            row=run(a,kind,label,extra={'LD_PRELOAD':str(library),'SOURCE_ALLOC_TARGET':str(a.out/'bin'/kind),'SOURCE_ALLOC_STACKS':str(a.out/f'{label}.bin')})
            text=(a.out/'logs'/f'{label}.stderr').read_text()
            lines=[s for s in text.splitlines() if s.startswith('[source-alloc-stack]')]
            assert len(lines)==1
            row['api_counts']={k:int(v) for k,v in re.findall(r'(\w+)=(\d+)',lines[0])}
            assert row['api_counts']['dropped']==0
            rows.append(row)
            save(a.out/'alloc-samples.json',rows)
    verify(a)


def cache_counters(a):
    events='cache-references:u,cache-misses:u,dTLB-loads:u,dTLB-load-misses:u'
    for i in range(2):
        for kind in (['before','after'] if i%2==0 else ['after','before']):
            run(a,kind,f'cache-counters-{i}-{kind}',[PERF,'stat','-x',';','-e',events,'-o',a.out/f'cache-counters-{i}-{kind}.csv','--'])
    verify(a)


def safe_compare(a):
    rows=[]
    for i in range(4):
        for kind in (['before','walk-once'] if i%2==0 else ['walk-once','before']):
            rows.append(run(a,kind,f'ablation-safe-{i}-{kind}'))
            save(a.out/'safe-comparison-samples.json',rows)
    save(a.out/'safe-comparison-summary.json',summarize(rows))
    verify(a)


def instrument(source,scoped):
    core=source/'crates/notist-core/src'
    shutil.copy2(REPO/'tools/source_cost_probe.rs',core/'source_cost.rs')
    with (core/'lib.rs').open('a') as f: f.write('\n#[doc(hidden)] pub mod source_cost;\n')
    with (source/'src/lib.rs').open('a') as f: f.write('\n#[doc(hidden)] pub use notist_core::source_cost;\n')
    main=source/'crates/notist-cli/src/main.rs'
    main.write_text(enter_function(main.read_text(),'main','let _source_cost_report = notist::source_cost::Report;'))
    site=source/'crates/notist-ssg/src/build.rs'
    text=site.read_text()
    for anchor,label in [('    let routes = notist::progress::run(', 'after_discover'),('    let mut documents = vec![];', 'before_pipeline'),('    let asset_stage = ', 'after_pipeline'),('    let by_source = DocumentIndex::new(&documents);','before_bodies'),('    let templates_stage =','after_bodies'),('    Ok(BuildOutput {','before_return')]:
        assert text.count(anchor)==1,(anchor,text.count(anchor))
        text=text.replace(anchor,f'    notist::source_cost::checkpoint("{label}");\n'+anchor,1)
    site.write_text(text)
    if not scoped: return
    path=core/'source.rs'
    text=path.read_text()
    for name,index in [('in_namespace',1),('drop',2),('detached',3),('insert',4),('merge',5),('validate_node',6),('validate_program',7),('validate_expression',8),('validate_references',9),('by_name',10),('records',11),('texts',12),('same_revision',13),('anonymous',14)]:
        text=enter_function(text,name,f'let _source_cost_scope = crate::source_cost::Scope::new({index});')
    text=enter_function(text,'source','crate::source_cost::count(0,1);')
    replacements=[
        ('Reference::Node(node) => {','Reference::Node(node) => { crate::source_cost::count(1,1); if node.as_expr().is_some() { crate::source_cost::count(10,node.children().len()); }'),
        ('Reference::Expression(expression) => {','Reference::Expression(expression) => { crate::source_cost::count(2,1);'),
        ('check(origin.source)?;','crate::source_cost::count(3,1); check(origin.source)?;'),
        ('check(location.source)?;','crate::source_cost::count(4,1); check(location.source)?;'),
        ('let identity = Arc::new(Identity {','crate::source_cost::count(5,1); crate::source_cost::count(13,key.name.len()); crate::source_cost::count(14,key.namespace.name().len()); let identity = Arc::new(Identity {'),
        ('entries.remove(&self.key);','entries.remove(&self.key); crate::source_cost::count(6,1);'),
        ('Ok(Self(Arc::new(Revision { identity, text })))','crate::source_cost::count(7,1); Ok(Self(Arc::new(Revision { identity, text })))'),
        ('if let Some(existing) = entries.get(&key).and_then(Weak::upgrade) {','if let Some(existing) = entries.get(&key).and_then(Weak::upgrade) { crate::source_cost::count(11,1);'),
        ('let identity = {','let identity = { crate::source_cost::count(12,1);'),
        ('Arc::make_mut(&mut self.0).insert(id, source);','if Arc::strong_count(&self.0)>1 { crate::source_cost::count(8,1); } Arc::make_mut(&mut self.0).insert(id, source);'),
        ('for (id, source) in other.0.iter() {','crate::source_cost::count(9,other.0.len()); for (id, source) in other.0.iter() {'),
    ]
    for old,new in replacements:
        assert old in text,old
        text=text.replace(old,new,1)
    lock='''let mut entries = IDENTITIES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);'''
    assert text.count(lock)==2
    text=text.replace(lock, 'let wait = std::time::Instant::now();\n            '+lock+'\n            crate::source_cost::count(15,wait.elapsed().as_nanos() as usize);')
    # Count misses separately from attempts (reported attempts - hits).
    path.write_text(text)


def build_diagnostics(a):
    m=json.loads((a.out/'manifest.json').read_text())
    env=dict(os.environ,**BUILD_ENV,CARGO_TARGET_DIR=str(a.reference/'target'))
    for kind,parent in [('before-ledger','before'),('after-ledger','after'),('walk-once','after'),('unchecked','after'),('walk-once-ledger','walk-once')]:
        source=a.out/'sources'/kind
        if kind in m['sources'] and kind in m['binaries']:
            assert inventory(source)==m['sources'][kind]
            assert digest(a.out/'bin'/kind)==m['binaries'][kind]
            continue
        shutil.copytree(a.out/'sources'/parent,source)
        if kind.endswith('ledger'):
            instrument(source,parent!='before')
        else:
            path=source/'crates/notist-core/src/source.rs'
            text=path.read_text()
            if kind=='walk-once':
                old='''                    pending.extend(node.children().iter().map(Reference::Node));
                    if let Some(expression) = node.as_expr() {
                        pending.push(Reference::Expression(expression));
                    }'''
                new='''                    if let Some(expression) = node.as_expr() {
                        pending.push(Reference::Expression(expression));
                    } else {
                        pending.extend(node.children().iter().map(Reference::Node));
                    }'''
                assert text.count(old)==1
                text=text.replace(old,new)
            else:
                old='''        use crate::evaluation::ExprKind;'''
                assert text.count(old)==1
                text=text.replace(old,'        if std::hint::black_box(true) { return Ok(()); }\n'+old,1)
            path.write_text(text)
        with (a.out/'logs'/f'build-{kind}.log').open('wb') as log:
            subprocess.run(['cargo','build','--release','--locked','--offline','-p','notist-cli','--bin','notist'],cwd=source,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        shutil.copy2(a.reference/'target/release/notist',a.out/'bin'/kind)
        m['sources'][kind]=inventory(source)
        m['binaries'][kind]=digest(a.out/'bin'/kind)
        save(a.out/'manifest.json',m)
        print('built',kind,flush=True)


def ledger_run(a,kind):
    row=run(a,kind,kind,rss=True)
    text=(a.out/'logs'/f'{kind}.stderr').read_text()
    for prefix,name in [('[source-cost] ','scopes'),('[source-count] ','counts'),('[source-memory] ','memory')]:
        row[name]=[json.loads(line[len(prefix):]) for line in text.splitlines() if line.startswith(prefix)]
    assert len(row['scopes'])==16 and row['memory']
    return row


def walk_ledger(a):
    save(a.out/'walk-once-ledger.json',ledger_run(a,'walk-once-ledger'))
    verify(a)


def diagnostics(a):
    ledgers=[ledger_run(a,kind) for kind in ['before-ledger','after-ledger']]
    save(a.out/'ledger.json',ledgers)
    rows=[]
    for i in range(4):
        kinds=['after','walk-once','unchecked']
        kinds=kinds[i%3:]+kinds[:i%3]
        if i%2:kinds.reverse()
        for kind in kinds:
            row=run(a,kind,f'ablation-{i}-{kind}')
            row['diagnostic']=True
            rows.append(row)
            save(a.out/'ablation-samples.json',rows)
    save(a.out/'ablation-summary.json',summarize(rows))
    verify(a)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('action',choices=['prepare','clean','cpu','alloc','cache-counters','safe-compare','build-diagnostics','diagnostics','walk-ledger','verify'])
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--reference',type=Path,default=Path('/tmp/opencode/notist-source-identity-accepted-20261107'))
    p.add_argument('--pairs',type=int,default=8)
    a=p.parse_args();a.out=a.out.resolve();a.reference=a.reference.resolve()
    globals()[a.action.replace('-','_')](a)
