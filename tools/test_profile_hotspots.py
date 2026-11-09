from pathlib import Path
import os
import subprocess
import tempfile
import unittest

import profile_hotspots as profile
import analyze_hotspots as analysis
import struct
import json


class HotspotProfileTests(unittest.TestCase):
    def test_operation_categories_count_stacks_once_not_generic_type_mentions(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory)
            (root/'cpu-samples.json').write_text(json.dumps([{'label':'cpu-test'}]))
            real='_int_malloc\n<notist_core::definitions::ResolvedInstance>::key\n<notist_core::definitions::ResolvedInstance>::key'
            incidental='core::ptr::drop_glue::<notist_core::definitions::ResolvedInstance>'
            (root/'cpu-test-expanded-stacks.json').write_text(json.dumps({real:3,incidental:7}))
            analysis.analyze_operations(root)
            result=json.loads((root/'operation-summary.json').read_text())[0]
            self.assertEqual(result['samples'],10)
            self.assertEqual(result['counts']['instance_key'],3)
            self.assertEqual(result['counts']['glibc_allocator_self'],3)

    def test_cpu_profile_parser_preserves_weight_and_mapping_tail(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            path=Path(directory)/'sample.prof'
            path.write_bytes(struct.pack('<5Q',0,3,0,2004,0)+struct.pack('<4Q',7,2,0x123,0x456)+struct.pack('<3Q',0,1,0)+b'1000-2000 r-xp 00000000 00:00 0 /binary\n')
            period,records,maps=analysis.read_cpu_profile(path)
            self.assertEqual(period,2004)
            self.assertEqual(records,[(7,(0x123,0x456))])
            self.assertIn('/binary',maps)

    def test_never_overwrites_evidence(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            with self.assertRaises(ValueError):
                profile.prepare(type('Args',(),{'out':Path(directory)})())

    def test_cpu_guard_only_profiles_the_target(self):
        if not profile.PROFILER.is_file():self.skipTest('gperftools is not installed')
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root=Path(directory)
            source=root/'busy.c'
            source.write_text('#include <time.h>\nint main(void) { volatile unsigned long n=0; clock_t end=clock()+CLOCKS_PER_SEC/5; while(clock()<end) { for(int i=0;i<10000;i++)n+=i; } return 0; }\n')
            target=root/'busy';guard=root/'guard.so';output=root/'cpu.prof'
            subprocess.run(['cc','-O1','-fno-omit-frame-pointer',str(source),'-o',str(target)],check=True,capture_output=True)
            subprocess.run(['cc','-shared','-fPIC',str(Path(profile.__file__).with_name('hotspot_cpu_guard.c')),str(profile.PROFILER),'-Wl,-rpath,'+str(profile.PROFILER.parent),'-o',str(guard)],check=True,capture_output=True)
            env=dict(os.environ,LD_PRELOAD=str(guard),HOTSPOT_CPU_TARGET=str(target),HOTSPOT_CPU_OUTPUT=str(output),CPUPROFILE_FREQUENCY='499')
            env.pop('CPUPROFILE',None)
            mismatch=dict(env,HOTSPOT_CPU_TARGET=str(root/'not-the-target'))
            result=subprocess.run([str(target)],env=mismatch,capture_output=True,check=True)
            self.assertFalse(output.exists())
            self.assertNotIn(b'[hotspot-parent-cpu]',result.stderr)
            result=subprocess.run([str(target)],env=env,capture_output=True,check=True)
            self.assertGreater(output.stat().st_size,1000)
            self.assertIn(b'[hotspot-parent-cpu]',result.stderr)
            period,records,maps=analysis.read_cpu_profile(output)
            self.assertGreater(period,0)
            self.assertGreater(sum(count for count,pcs in records),10)
            self.assertIn(str(target),maps)
