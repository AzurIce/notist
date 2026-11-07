#!/usr/bin/env python3
"""Summarize immutable SourceId diagnostic evidence, retaining raw profiles."""
import argparse
from collections import Counter, defaultdict
import gzip
import json
from pathlib import Path
import re
import statistics
import struct
import subprocess


def save(path,data):
    path.write_text(json.dumps(data,indent=2,ensure_ascii=False)+'\n')


def symbols(binary,addresses):
    values=sorted(addresses)
    result={}
    for start in range(0,len(values),2000):
        text=subprocess.check_output(['addr2line','-a','-f','-i','-C','-e',str(binary)],input=''.join(f'{x:x}\n' for x in values[start:start+2000]),text=True)
        current=None
        for line in text.splitlines():
            if re.fullmatch(r'0x[0-9a-fA-F]+',line):
                current=int(line,16); result[current]=[]
            elif current is not None:
                result[current].append(line)
    return result


def alloc_stacks(out):
    results=[]
    for sample in json.loads((out/'alloc-samples.json').read_text()):
        label,kind=sample['label'],sample['kind']
        binary=out/'bin'/kind
        mappings=[]
        for line in (out/f'{label}.bin.maps').read_text().splitlines():
            fields=line.split(maxsplit=5)
            if len(fields)==6 and fields[5]==str(binary):
                lo,hi=(int(x,16) for x in fields[0].split('-'))
                mappings.append((lo,hi,int(fields[2],16)))
        assert mappings,label
        # lld ELF PT_LOAD text can have p_vaddr != p_offset (here +0x1000).
        # Do not subtract the executable mapping's file offset as load bias.
        # These profiled Rust PIEs have a PT_LOAD at vaddr/file offset zero.
        bias=next(lo for lo,hi,offset in mappings if offset==0)
        record=struct.Struct('<28Q')
        data=(out/f'{label}.bin').read_bytes()
        assert len(data)%record.size==0
        records=[]; addresses=set()
        for raw in record.iter_unpack(data):
            api,size,old,depth,*pcs=raw
            assert depth<=24 and api<3
            frames=[]
            for pc in pcs[:depth]:
                for lo,hi,offset in mappings:
                    if lo<=pc<hi:
                        address=pc-bias-1
                        addresses.add(address); frames.append(address); break
            records.append((api,size,old,frames))
        cache=out/f'{label}.load-bias-v2.symbols.json'
        if cache.exists(): lookup={int(k):v for k,v in json.loads(cache.read_text()).items()}
        else: lookup=symbols(binary,addresses); save(cache,lookup)
        groups=defaultdict(lambda:dict(samples=0,estimated_events=0,estimated_requested_bytes=0,old_usable_bytes_sampled=0))
        categories=defaultdict(lambda:dict(samples=0,estimated_events=0,estimated_requested_bytes=0))
        missing=0
        for api,size,old,frames in records:
            names=[line for frame in frames for line in lookup.get(frame,[])[::2] if line!='??']
            # Type parameters mentioning notist are not application call sites;
            # folded RawVec instantiations can even carry misleading type names.
            owner=next((name for name in names if re.match(r'^(?:notist_|notist::|<notist_|<notist::)',name)),'[no notist frame]')
            if owner=='[no notist frame]':missing+=1
            category='other'
            for needle,name in [('validate_references','source.validation'),('SourceNamespace','source.namespace'),('in_namespace','source.registration'),('source::Sources>::insert','source.snapshot_insert'),('source::Sources>::merge','source.snapshot_merge'),('source::Sources>::texts','source.wire_projection')]:
                if any(needle in s for s in names):category=name;break
            weight=256 if api==2 else 2048
            group=groups[f'{["malloc","calloc","realloc"][api]} | {owner}']
            group['samples']+=1;group['estimated_events']+=weight;group['estimated_requested_bytes']+=size*weight;group['old_usable_bytes_sampled']+=old
            c=categories[f'{["malloc","calloc","realloc"][api]} | {category}']
            c['samples']+=1;c['estimated_events']+=weight;c['estimated_requested_bytes']+=size*weight
        row=dict(kind=kind,label=label,samples=len(records),no_notist_frame=missing,categories=dict(categories),top_events=sorted(groups.items(),key=lambda x:x[1]['estimated_events'],reverse=True)[:80],top_bytes=sorted(groups.items(),key=lambda x:x[1]['estimated_requested_bytes'],reverse=True)[:80],warning='probability estimates; bytes can have high variance for rare large allocations; exact source scopes are in ledger.json; old_usable is allocator capacity, not old requested bytes')
        save(out/f'{label}.attribution.json',row);results.append(row)
    save(out/'alloc-attribution.json',results)


def perf_cpu_stacks(out):
    rows=[]
    for sample in json.loads((out/'cpu-samples.json').read_text()):
        label,kind=sample['label'],sample['kind']
        text=(out/f'{label}.stacks.txt').read_text()
        blocks=text.split('\n\n')
        flat=Counter(); inclusive=Counter(); categories=Counter(); total=0; source_samples=[]
        for block in blocks:
            lines=block.splitlines()
            if not lines:continue
            header=lines[0]
            # Command names of all Rust worker threads may differ: select by
            # sampled DSO, rather than excluding non-main-thread comm names.
            if not any(f'/bin/{kind})' in line or f'({kind})' in line for line in lines):continue
            frames=[]
            for line in lines[1:]:
                m=re.match(r'\s+[0-9a-f]+\s+(.+?)\s+\(([^)]*)\)',line)
                if m:frames.append(m[1].split('+0x')[0])
            if not frames:continue
            total+=1;flat[frames[0]]+=1;inclusive.update(set(frames))
            joined='\n'.join(frames)
            for needle,name in [('validate_references','validation'),('source::Sources','source_snapshots'),('source::Identity','source_identity_drop'),('source::Source','source_registration'),('memcmp','memcmp'),('memmove','memmove'),('malloc','allocator'),('realloc','allocator'),('free','allocator'),('Arc','arc'),('BTree','btree')]:
                if needle in joined:categories[name]+=1
            if 'notist_core::source::' in joined and len(source_samples)<20:source_samples.append(lines)
        row=dict(kind=kind,label=label,rust_samples=total,flat=flat.most_common(70),inclusive=inclusive.most_common(100),overlapping_stack_categories=dict(categories),source_stack_examples=source_samples,warning='statistical CPU samples, overlapping categories; stack truncation and inlining can limit attribution')
        assert total>100,label
        save(out/f'{label}.cpu-summary.json',row);rows.append(row)
    save(out/'cpu-summary.json',rows)


def cpu_stacks(out):
    samples=json.loads((out/'cpu-samples.json').read_text())
    if not all((out/f'{r["label"]}.json.gz').exists() for r in samples):
        return perf_cpu_stacks(out)
    rows=[]
    for sample in samples:
        label,kind=sample['label'],sample['kind']
        profile=json.loads(gzip.decompress((out/f'{label}.json.gz').read_bytes()))
        threads=[t for t in profile['threads'] if t.get('processName')==kind or t.get('name')==kind]
        needed=defaultdict(set)
        for t in threads:
            for i,address in enumerate(t['frameTable']['address']):
                resource=t['funcTable']['resource'][t['frameTable']['func'][i]]
                if resource is not None and resource>=0 and address is not None and address>=0:
                    lib=t['resourceTable']['lib'][resource]
                    if lib is not None:needed[lib].add(address)
        cache=out/f'{label}.symbols-by-lib.json'
        if cache.exists():lookup={int(k):{int(a):v for a,v in items.items()} for k,items in json.loads(cache.read_text()).items()}
        else:
            lookup={lib:symbols(Path(profile['libs'][lib]['debugPath']),addresses) if profile['libs'][lib].get('debugPath') and Path(profile['libs'][lib]['debugPath']).is_file() else {} for lib,addresses in needed.items()}
            save(cache,lookup)
        flat=Counter();inclusive=Counter();categories=Counter();total=0;unknown=0;examples=[]
        for t in threads:
            original_frames=t['frameTable']['length']
            original_stacks=t['stackTable']['length']
            strings=t['stringArray']
            def string(value):
                strings.append(value);return len(strings)-1
            expanded_frames={};frame_names={}
            for i in range(original_frames):
                old_func=t['frameTable']['func'][i]
                resource=t['funcTable']['resource'][old_func]
                lib=t['resourceTable']['lib'][resource] if resource is not None and resource>=0 else None
                lines=lookup.get(lib,{}).get(t['frameTable']['address'][i],[])
                pairs=[(lines[j],lines[j+1]) for j in range(0,len(lines)-1,2) if lines[j]!='??']
                if not pairs:
                    frame_names[i]=[strings[t['funcTable']['name'][old_func]]]
                    expanded_frames[i]=[i];continue
                frame_names[i]=[name for name,_ in pairs]
                expanded=[]
                for depth,(name,location) in enumerate(reversed(pairs)):
                    f=t['funcTable']['length'];t['funcTable']['length']+=1
                    for key,values in t['funcTable'].items():
                        if key!='length':values.append(values[old_func])
                    t['funcTable']['name'][f]=string(name)
                    loc=re.match(r'(.*):(\d+)(?:\s|$)',location)
                    if loc:t['funcTable']['fileName'][f]=string(loc[1])
                    frame=t['frameTable']['length'];t['frameTable']['length']+=1
                    for key,values in t['frameTable'].items():
                        if key!='length':values.append(values[i])
                    t['frameTable']['func'][frame]=f
                    t['frameTable']['inlineDepth'][frame]=depth
                    if loc:t['frameTable']['line'][frame]=int(loc[2])
                    expanded.append(frame)
                expanded_frames[i]=expanded
            expanded_stacks={}
            for i in range(original_stacks):
                old_prefix=t['stackTable']['prefix'][i]
                prefix=expanded_stacks[old_prefix] if old_prefix is not None else None
                for frame in expanded_frames[t['stackTable']['frame'][i]]:
                    index=t['stackTable']['length'];t['stackTable']['length']+=1
                    for key,values in t['stackTable'].items():
                        if key!='length':values.append(values[i])
                    t['stackTable']['prefix'][index]=prefix;t['stackTable']['frame'][index]=frame
                    prefix=index
                expanded_stacks[i]=prefix
            for index,stack in enumerate(t['samples']['stack']):
                if stack is None:continue
                weight=t['samples'].get('weight',[1]*t['samples']['length'])[index] or 1
                names=[];cursor=stack
                while cursor is not None:
                    names.extend(frame_names[t['stackTable']['frame'][cursor]])
                    cursor=t['stackTable']['prefix'][cursor]
                total+=weight
                if not names or all(n.startswith('0x') for n in names):unknown+=weight
                if names:flat[names[0]]+=weight;inclusive.update({name:weight for name in set(names)})
                joined='\n'.join(names)
                for needle,name in [('validate_references','validation'),('source::Sources','source_snapshots'),('source::Identity','source_identity_drop'),('source::Source>::in_namespace','source_registration'),('memcmp','memcmp'),('memmove','memmove'),('malloc','allocator'),('realloc','allocator'),('free','allocator'),('Arc','arc'),('BTree','btree')]:
                    if needle in joined:categories[name]+=weight
                if 'notist_core::source::' in joined and len(examples)<15:examples.append(names)
                t['samples']['stack'][index]=expanded_stacks[stack]
        assert total>100,(label,total)
        profile['meta']['symbolicated']=True
        target=out/f'{label}.symbolicated.json.gz'
        with gzip.open(target,'wt') as f:json.dump(profile,f,separators=(',',':'))
        row=dict(kind=kind,label=label,rust_samples=total,unknown_stack_samples=unknown,flat=flat.most_common(70),inclusive=inclusive.most_common(100),overlapping_stack_categories=dict(categories),source_stack_examples=examples,method='samply/framehop user-stack unwinding plus local addr2line inline expansion; perf script/libdw failed to unwind most Rust stacks and is not used for these totals')
        save(out/f'{label}.cpu-summary.json',row);rows.append(row)
    save(out/'cpu-summary.json',rows)


def counters(out):
    rows=[]
    for path in sorted([*out.glob('counters-*.csv'),*out.glob('cache-counters-*.csv')]):
        values={}
        for line in path.read_text().splitlines():
            f=line.split(';')
            if len(f)>=3 and f[0].strip() and not f[0].startswith('#'):
                try: values[f[2]]=float(f[0])
                except ValueError:pass
        rows.append(dict(label=path.stem,kind=path.stem.split('-')[-1],values=values))
    summary={kind:{key:statistics.median(r['values'][key] for r in rows if r['kind']==kind and key in r['values']) for key in set().union(*(r['values'] for r in rows if r['kind']==kind))} for kind in ['before','after']}
    save(out/'hardware-counters.json',dict(samples=rows,summary=summary))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out',type=Path,required=True)
    a=p.parse_args()
    cpu_stacks(a.out); counters(a.out); alloc_stacks(a.out)
