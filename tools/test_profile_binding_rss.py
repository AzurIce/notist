from pathlib import Path
import subprocess
import tempfile
import unittest

import profile_binding_rss as profile


class BindingRssProfileTests(unittest.TestCase):
    def test_scope_insertion_preserves_signature_and_body(self):
        text = 'fn example(x: usize) -> Result<(), String> { Ok(()) }'
        result = profile.scope(text, 'fn example(', 6)
        self.assertIn('let _rss_scope = crate::binding_rss::Scope::new(6);', result)
        self.assertTrue(result.endswith(' Ok(()) }'))

    def test_probes_preserve_alignment_zeroing_and_original_owner_on_realloc(self):
        tests = r'''
#[test]
fn allocation_contract_and_owner() {
    for align in [1, 8, 16, 64, 4096] {
        let layout = Layout::from_size_align(131, align).unwrap();
        let before = M[13].live.load(Relaxed);
        let scope = Scope::new(13);
        let p = unsafe { ALLOCATOR.alloc_zeroed(layout) };
        assert!(!p.is_null());
        assert_eq!(p as usize % align, 0);
        assert!(unsafe { std::slice::from_raw_parts(p, 131) }.iter().all(|v| *v==0));
        if TAGGED { assert_eq!(M[13].live.load(Relaxed), before+131); }
        unsafe { p.write_bytes(7,131); }
        drop(scope);
        let other = Scope::new(14);
        let q = unsafe { ALLOCATOR.realloc(p, layout, 10001) };
        assert!(!q.is_null());
        assert_eq!(q as usize % align,0);
        assert!(unsafe { std::slice::from_raw_parts(q,131) }.iter().all(|v| *v==7));
        if TAGGED { assert_eq!(M[13].live.load(Relaxed), before+10001); }
        let grown = Layout::from_size_align(10001,align).unwrap();
        let r = unsafe { ALLOCATOR.realloc(q,grown,71) };
        assert!(!r.is_null());
        assert!(unsafe { std::slice::from_raw_parts(r,71) }.iter().all(|v| *v==7));
        unsafe { ALLOCATOR.dealloc(r,Layout::from_size_align(71,align).unwrap()); }
        if TAGGED { assert_eq!(M[13].live.load(Relaxed), before); }
        drop(other);
    }
}
'''
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            source = root/'probe.rs'
            source.write_text((Path(profile.__file__).parent/'binding_rss_probe.rs').read_text()+tests)
            for tagged in [False, True]:
                binary = root/('tags' if tagged else 'pages')
                cmd = ['rustc','--edition=2024','--test',str(source),'-o',str(binary)]
                if tagged: cmd += ['--cfg','binding_rss_tags']
                subprocess.run(cmd,check=True,capture_output=True)
                subprocess.run([str(binary),'--test-threads=1'],check=True,capture_output=True)
