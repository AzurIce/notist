#!/usr/bin/env python3
"""Summarize phase wall timers and inline-expanded user-mode CPU samples separately."""
from collections import Counter, defaultdict
import argparse
import json
from pathlib import Path
import re
import statistics
import struct

from analyze_source_cost import symbols, alloc_stacks
from profile_source_identity import save

def read_cpu_profile(path):
    data=path.read_bytes()
    if len(data)<40:raise ValueError('short CPU profile')
    header=struct.unpack_from('<5Q',data)
    if header[:3]!=(0,3,0):raise ValueError(('unsupported CPU profile header',header))
    records=[];position=40
    while position+16<=len(data):
        count,depth=struct.unpack_from('<2Q',data,position);position+=16
        if depth>1024 or position+depth*8>len(data):raise ValueError('invalid CPU stack depth')
        pcs=struct.unpack_from('<'+str(depth)+'Q',data,position);position+=depth*8
        if count==0 and depth==1 and pcs==(0,):break
        records.append((count,pcs))
    else:raise ValueError('missing CPU profile trailer')
    return header[3],records,data[position:].decode(errors='replace')

def analyze_cpu(out):
    result=[]
    for row in json.loads((out/'cpu-samples.json').read_text()):
        label=row['label'];period,records,maps=read_cpu_profile(out/f'{label}.prof')
        mappings=[];bases={}
        for line in maps.splitlines():
            fields=line.split(maxsplit=5)
            if len(fields)<6 or not fields[5].startswith('/'):continue
            lo,hi=(int(v,16) for v in fields[0].split('-'));offset=int(fields[2],16);path=fields[5]
            mappings.append((lo,hi,path))
            if offset==0:bases[path]=lo
        needed=defaultdict(set);raw=[]
        for count,pcs in records:
            frames=[]
            for index,pc in enumerate(pcs):
                mapping=next(((lo,hi,path) for lo,hi,path in mappings if lo<=pc<hi),None)
                if mapping is None:frames.append((None,pc));continue
                lo,hi,path=mapping
                if path not in bases:frames.append((None,pc));continue
                address=pc-bases[path]-(index!=0)
                needed[path].add(address);frames.append((path,address))
            raw.append((count,frames))
        cache=out/f'{label}-inline-symbols.json'
        if cache.exists():lookup={path:{int(k):v for k,v in values.items()} for path,values in json.loads(cache.read_text()).items()}
        else:
            lookup={path:symbols(Path(path),values) if Path(path).is_file() else {} for path,values in needed.items()}
            save(cache,lookup)
        flat=Counter();inclusive=Counter();stack_counts=Counter();total=0;target_samples=0
        for count,frames in raw:
            names=[]
            for path,address in frames:
                lines=lookup.get(path,{}).get(address,[])
                functions=[name for name in lines[::2] if name!='??']
                names.extend(functions or [f'[{Path(path).name if path else "unknown"}+0x{address:x}]'])
            total+=count
            if any(path==str(out/'bin/frames') for path,address in frames):target_samples+=count
            if names:flat[names[0]]+=count;inclusive.update({name:count for name in set(names)})
            stack_counts['\n'.join(names)]+=count
        assert total>1000,(label,total)
        summary={'label':label,'period_us':period,'samples':total,'samples_with_target_frame':target_samples,'self':flat.most_common(100),'inclusive':inclusive.most_common(150),'top_stacks':stack_counts.most_common(60),'warning':'Statistical parent-process CPU samples; cumulative entries overlap. Frame-pointer binary and timer overhead are diagnostic, not clean wall timings.'}
        parent=re.search(r'\[hotspot-parent-cpu\] user=([\d.]+) system=([\d.]+)',(out/'logs'/f'{label}.stderr').read_text())
        if parent:summary['parent_cpu']={'user_s':float(parent[1]),'system_s':float(parent[2])}
        summary['sampled_cpu_seconds_approx']=total*period/1_000_000
        save(out/f'{label}-inline-summary.json',summary)
        save(out/f'{label}-expanded-stacks.json',dict(stack_counts))
        result.append(summary)
    save(out/'cpu-summary.json',result)

def analyze_phases(out):
    rows=json.loads((out/'samples.json').read_text());summary={}
    for kind in ['normal','frames','coarse','detail']:
        samples=[row for row in rows if row['kind']==kind and not row['label'].startswith('warmup')]
        summary[kind]={key:statistics.median(row[key] for row in samples) for key in ['wall_s','user_s','system_s','maxrss_kib']}
        if kind in ['coarse','detail']:
            summary[kind]['metrics']={name:{key:statistics.median(row['metrics'][name][key] for row in samples) for key in ['calls','seconds','self_seconds']} for name in samples[0]['metrics']}
    save(out/'phase-summary.json',summary)

def analyze_allocations(out):
    row=json.loads((out/'allocation-sample.json').read_text())
    assert row['complete_output_identical']
    save(out/'alloc-samples.json',[{'kind':'frames','label':'allocation-stacks'}])
    alloc_stacks(out)

def analyze_operations(out):
    rows=[]
    for sample in json.loads((out/'cpu-samples.json').read_text()):
        label=sample['label'];stacks=json.loads((out/f'{label}-expanded-stacks.json').read_text())
        total=sum(stacks.values());counts=Counter();path_callers=Counter();allocator_callers=Counter()
        patterns={
            'dict_insert':r'^<notist_core::shared::Dict>::insert',
            'instance_key':r'^<?notist_core::definitions::(?:ResolvedInstance|FunctionInstance|ComptimeValue)(?:::|>).*::key',
            'source_validation':r'^<notist_core::source::Sources>::validate',
            'implementation_lookup':r'^<notist_eval::eval::Evaluator.*::implementation',
            'instance_cache':r'^<notist_eval::instance_cache::InstanceCache>::',
            'node_descendants':r'^<notist_core::item::Node>::descendants',
            'evaluation':r'^notist_eval::eval::run',
            'content_present':r'^<notist_eval::eval::Evaluator.*::present$',
            'execute':r'^<notist_eval::eval::Evaluator.*::execute$',
            'expression_eval':r'^<notist_eval::eval::Evaluator.*::expr$',
            'content_binding':r'^<notist_eval::eval::Evaluator.*::bind_content$',
            'html_render':r'^<notist_html::Renderer>::render_with_diagnostics$',
        }
        for text,n in stacks.items():
            frames=text.splitlines();leaf=frames[0]
            app=next((f for f in frames if re.match(r'^(?:<)?notist(?:::|_)',f)),'[none]')
            if re.search(r'(?:malloc|realloc|free|unlink_chunk|sysmalloc|consolidate|munmap_chunk)',leaf) and not leaf.startswith(('notist','<notist')):
                counts['glibc_allocator_self']+=n;allocator_callers[app]+=n
            if any('std::path::compare_components' in f for f in frames):counts['path_comparison']+=n;path_callers[app]+=n
            if re.search(r'^__mem(?:move|cpy)',leaf):counts['memory_copy_self']+=n
            if leaf.startswith('__memcmp'):counts['memcmp_self']+=n
            for name,pattern in patterns.items():
                if any(re.search(pattern,f) for f in frames):counts[name]+=n
        rows.append({'label':label,'samples':total,'counts':dict(counts),'percent':{k:100*v/total for k,v in counts.items()},'path_comparison_nearest_app':path_callers.most_common(30),'allocator_self_nearest_app':allocator_callers.most_common(30),'warning':'Process and helper cumulative categories overlap; allocator self is a leaf-only category. Generic types merely mentioning an application type do not count as app call sites.'})
    save(out/'operation-summary.json',rows)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();out=a.out.resolve()
    analyze_phases(out);analyze_cpu(out);analyze_allocations(out);analyze_operations(out)
