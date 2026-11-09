#!/usr/bin/env python3
"""Fresh committed full-site hotspots: separate clean, phase, CPU and alloc probes."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
from types import SimpleNamespace

from profile_source_identity import BUILD_ENV, digest, inventory, save
from profile_definition_identity import verify as verify_reference
from profile_icexiv_refresh import instrument
from profile_source_cost import run as run_build
from profile_current import parse_detail

REPO=Path(__file__).resolve().parents[1]
PROFILER=Path('/nix/store/ca8m3rhpgmd0q7k47cb9gb3a7622jdpc-gperftools-2.18.1/lib/libprofiler.so.0.5.19')
PPROF=Path('/nix/store/qrns8m17akfs90vcgj672q4bzx7gj1qd-gperftools-2.15/bin/pprof')

def prepare(a):
    if a.out.exists():raise ValueError('fresh evidence directory required')
    ref=verify_reference(SimpleNamespace(out=a.reference))
    for part in ['bin','logs','sources/normal','reproducer']:(a.out/part).mkdir(parents=True)
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    archive=a.out/'source.tar'
    subprocess.run(['git','archive','--format=tar','--output',str(archive),revision],cwd=REPO,check=True)
    with tarfile.open(archive) as stream:stream.extractall(a.out/'sources/normal',filter='data')
    archive.unlink()
    m={'revision':revision,'reference':str(a.reference),'inputs':ref['inputs'],'sources':{},'binaries':{},'build_env':BUILD_ENV,'perf_event_paranoid':Path('/proc/sys/kernel/perf_event_paranoid').read_text().strip(),'sampler':'gperftools CPU timer; target parent only; 499 Hz; frame-pointer diagnostic build','profiler':str(PROFILER),'pprof':str(PPROF)}
    for kind in ['coarse','detail','frames']:
        root=a.out/'sources'/kind
        shutil.copytree(a.out/'sources/normal',root)
        if kind in ['coarse','detail']:instrument(root,kind=='detail')
    m['sources']={kind:inventory(a.out/'sources'/kind) for kind in ['normal','coarse','detail','frames']}
    (a.out/'site').symlink_to((a.reference/'site').resolve(),target_is_directory=True)
    shutil.copy2(a.reference/'output-inventory.json',a.out/'output-inventory.json')
    for source in (REPO/'tools').iterdir():
        if source.is_file():shutil.copy2(source,a.out/'reproducer'/source.name)
    save(a.out/'manifest.json',m)

def build(a):
    m=json.loads((a.out/'manifest.json').read_text())
    for kind in m['sources']:
        if kind in m['binaries']:continue
        assert inventory(a.out/'sources'/kind)==m['sources'][kind]
        env=dict(os.environ,**BUILD_ENV,CARGO_TARGET_DIR=str(a.out/'targets'/kind))
        if kind=='frames':env['RUSTFLAGS']='-C force-frame-pointers=yes'
        cmd=['cargo','build','--release','--locked','--offline','-p','notist-cli','--bin','notist']
        if kind in ['coarse','detail']:cmd+=['--features','profiling']
        with (a.out/'logs'/f'build-{kind}.log').open('wb') as log:
            subprocess.run(cmd,cwd=a.out/'sources'/kind,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        shutil.copy2(a.out/'targets'/kind/'release/notist',a.out/'bin'/kind)
        m['binaries'][kind]=digest(a.out/'bin'/kind);save(a.out/'manifest.json',m)
        print('built',kind,flush=True)
    subprocess.run(['cc','-shared','-O2','-fPIC',str(REPO/'tools/hotspot_cpu_guard.c'),str(PROFILER),'-Wl,-rpath,'+str(PROFILER.parent),'-o',str(a.out/'bin/cpu-guard.so')],check=True)
    subprocess.run(['cc','-shared','-O2','-fPIC',str(REPO/'tools/source_alloc_stack.c'),'-o',str(a.out/'bin/alloc-stacks.so')],check=True)

def measure(a):
    if (a.out/'samples.json').exists():raise ValueError('never overwrite measurements')
    rows=[]
    jobs=[('normal','warmup-normal')]+[('normal',f'normal-{i}') for i in range(5)]+[('frames',f'frames-{i}') for i in range(2)]+[('coarse',f'coarse-{i}') for i in range(3)]+[('detail',f'detail-{i}') for i in range(2)]
    for kind,label in jobs:
        row=run_build(a,kind,label)
        if kind in ['coarse','detail']:
            row['metrics'],row['edges'],row['modules']=parse_detail((a.out/'logs'/f'{label}.stderr').read_text())
        rows.append(row);save(a.out/'samples.json',rows)

def cpu(a):
    rows=[]
    for i in range(2):
        label=f'cpu-{i}';profile=a.out/f'{label}.prof'
        if profile.exists():raise ValueError('never overwrite '+label)
        row=run_build(a,'frames',label,extra={'LD_PRELOAD':str(a.out/'bin/cpu-guard.so'),'HOTSPOT_CPU_TARGET':str(a.out/'bin/frames'),'HOTSPOT_CPU_OUTPUT':str(profile),'CPUPROFILE_FREQUENCY':'499'})
        assert profile.stat().st_size>1000
        rows.append(row);save(a.out/'cpu-samples.json',rows)
        for name,flags in [('self',[]),('cumulative',['--cum']),('stacks',['--stacks'])]:
            with (a.out/f'{label}-{name}.txt').open('wb') as log:
                subprocess.run([str(PPROF),'--text','--no_strip_temp','--nodefraction=0',*flags,str(a.out/'bin/frames'),str(profile)],stdout=log,stderr=subprocess.STDOUT,check=True)
        with (a.out/f'{label}-collapsed.txt').open('wb') as log:
            subprocess.run([str(PPROF),'--collapsed','--no_strip_temp',str(a.out/'bin/frames'),str(profile)],stdout=log,stderr=subprocess.STDOUT,check=True)

def allocations(a):
    row=run_build(a,'frames','allocation-stacks',extra={'LD_PRELOAD':str(a.out/'bin/alloc-stacks.so'),'SOURCE_ALLOC_TARGET':str(a.out/'bin/frames'),'SOURCE_ALLOC_STACKS':str(a.out/'allocation-stacks.bin')})
    save(a.out/'allocation-sample.json',row)

def verify(a):
    m=json.loads((a.out/'manifest.json').read_text())
    for kind,expected in m['sources'].items():assert inventory(a.out/'sources'/kind)==expected,kind
    for kind,expected in m['binaries'].items():assert digest(a.out/'bin'/kind)==expected,kind
    assert {k:v for k,v in inventory(a.out/'site').items() if not k.startswith('dist/')}==m['inputs']
    assert inventory(a.out/'site/dist/icexiv-db')==json.loads((a.out/'output-inventory.json').read_text())
    print('all frozen sources, binaries, inputs and full outputs verified',flush=True)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('action',choices=['prepare','build','measure','cpu','allocations','verify','all'])
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--reference',type=Path,default=Path('/tmp/opencode/notist-binding-identity-complete-20261108-v2'))
    a=p.parse_args();a.out=a.out.resolve();a.reference=a.reference.resolve()
    for name in (['prepare','build','measure','cpu','allocations','verify'] if a.action=='all' else [a.action]):globals()[name](a)
