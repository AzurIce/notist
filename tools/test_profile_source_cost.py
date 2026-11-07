from pathlib import Path
import gzip
import json
import struct
import tempfile
import unittest

import analyze_source_cost as analysis
import profile_source_cost as profile


class SourceCostTests(unittest.TestCase):
    def test_preparation_never_overwrites_an_existing_directory(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory)
            keep=root/'keep';keep.write_text('unchanged')
            with self.assertRaises(ValueError):
                profile.prepare(type('Args',(),dict(out=root))())
            self.assertEqual(keep.read_text(),'unchanged')

    def test_cpu_includes_rust_workers_but_excludes_extension_processes(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory)
            (root/'cpu-samples.json').write_text(json.dumps([dict(kind='after',label='cpu-0-after')]))
            block='worker 123/456 1.0: cpu-clock:u:\n  abc malloc (/nix/libc.so)\n  def notist_core::source::Sources::validate_references (/test/bin/after)\n\n'
            python='python 999 1.0: cpu-clock:u:\n  abc PyEval (/nix/python)\n\n'
            (root/'cpu-0-after.stacks.txt').write_text(block*150+python*20)
            analysis.cpu_stacks(root)
            row=json.loads((root/'cpu-summary.json').read_text())[0]
            self.assertEqual(row['rust_samples'],150)
            self.assertEqual(row['overlapping_stack_categories']['validation'],150)
            self.assertEqual(row['flat'][0],['malloc',150])

    def test_allocation_stack_weights_do_not_claim_old_requested_size(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory);(root/'bin').mkdir()
            label='alloc-0-after'
            (root/'alloc-samples.json').write_text(json.dumps([dict(kind='after',label=label)]))
            # Text vaddr is file offset + 0x1000, matching lld's actual PIEs.
            (root/f'{label}.bin.maps').write_text(f'00001000-00002000 r--p 00000000 00:00 0 {root}/bin/after\n00004000-00005000 r-xp 00002000 00:00 0 {root}/bin/after\n')
            (root/f'{label}.bin').write_bytes(struct.pack('<28Q',2,256,128,2,0x4101,0x4121,*([0]*22)))
            (root/f'{label}.load-bias-v2.symbols.json').write_text(json.dumps({0x3100:['alloc::alloc','alloc.rs:1'],0x3120:['notist_core::source::Sources::validate_references','source.rs:1']}))
            analysis.alloc_stacks(root)
            row=json.loads((root/'alloc-attribution.json').read_text())[0]
            self.assertEqual(row['categories']['realloc | source.validation']['estimated_events'],256)
            self.assertEqual(row['categories']['realloc | source.validation']['estimated_requested_bytes'],65536)
            self.assertIn('not old requested bytes',row['warning'])

    def test_samply_inline_expansion_preserves_samples_and_does_not_call_sources_registration(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory);label='cpu-0-after'
            (root/'cpu-samples.json').write_text(json.dumps([dict(kind='after',label=label)]))
            thread=dict(name='worker',processName='after',stringArray=['0x100','0x200'],
                funcTable=dict(length=2,name=[0,1],resource=[0,0],fileName=[None,None]),
                frameTable=dict(length=2,address=[256,512],func=[0,1],inlineDepth=[0,0],line=[None,None]),
                resourceTable=dict(length=1,lib=[0]),
                stackTable=dict(length=2,prefix=[None,0],frame=[1,0],category=[0,0]),
                samples=dict(length=150,stack=[1]*150,weight=[1]*150))
            data=dict(meta=dict(symbolicated=False),libs=[dict(debugPath='/missing')],threads=[thread])
            (root/f'{label}.json.gz').write_bytes(gzip.compress(json.dumps(data).encode()))
            (root/f'{label}.symbols-by-lib.json').write_text(json.dumps({0:{256:['BTreeMap::get','btree.rs:1','<notist_core::source::Sources>::validate_references','source.rs:1'],512:['notist_eval::eval::run','eval.rs:1']}}))
            analysis.cpu_stacks(root)
            row=json.loads((root/'cpu-summary.json').read_text())[0]
            self.assertEqual(row['rust_samples'],150)
            self.assertEqual(row['flat'][0],['BTreeMap::get',150])
            self.assertNotIn('source_registration',row['overlapping_stack_categories'])
            expanded=json.loads(gzip.decompress((root/f'{label}.symbolicated.json.gz').read_bytes()))
            t=expanded['threads'][0]
            self.assertTrue(expanded['meta']['symbolicated'])
            self.assertEqual(t['samples']['length'],150)
            self.assertTrue(all(s<t['stackTable']['length'] for s in t['samples']['stack']))


if __name__=='__main__':
    unittest.main()
