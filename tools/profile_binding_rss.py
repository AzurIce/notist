#!/usr/bin/env python3
"""Decompose retained binding memory and glibc/RSS in disposable frozen copies."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from types import SimpleNamespace

from profile_support import BUILD_ENV, digest, inventory, save
from profile_definition_identity import verify as verify_reference
from profile_icexiv_refresh import environment

REPO = Path(__file__).resolve().parents[1]
LABELS = {'source_identity':1,'source_binding_registry':2,'empty_binding_snapshot':3,'binding_declaration':4,'binding_snapshot_merge':5,'resolved_signatures':6,'module_scope_pool':7,'source_views':8,'runtime_scope':9,'checker_specialization':10,'registry_install':11,'html_registry':12}

def scope(text, signature, tag):
    start = text.index(signature)
    # Find the implementation opening brace, after -> (possibly a Result).
    brace = text.index('{', start)
    return text[:brace+1]+f'\nlet _rss_scope = crate::binding_rss::Scope::new({tag});'+text[brace+1:]

def instrument(root, tagged):
    core = root/'crates/notist-core/src'
    shutil.copy2(REPO/'tools/binding_rss_probe.rs', core/'binding_rss.rs')
    with (core/'lib.rs').open('a') as f:f.write('\n#[doc(hidden)] pub mod binding_rss;\n')
    with (root/'src/lib.rs').open('a') as f:f.write('\n#[doc(hidden)] pub use notist_core::binding_rss;\n')
    main=root/'crates/notist-cli/src/main.rs'
    main.write_text(scope(main.read_text(),'fn main(',0).replace('let _rss_scope = crate::binding_rss::Scope::new(0);','let _rss_report = notist::binding_rss::Report;'))
    build=root/'crates/notist-ssg/src/build.rs'
    text=build.read_text()
    for anchor,label in [('    let routes = notist::progress::run(', 'after_discover'),('    let mut documents = vec![];', 'before_pipeline'),('    let asset_stage = ', 'after_pipeline'),('    let by_source = DocumentIndex::new(&documents);','before_bodies'),('    let templates_stage =','after_bodies'),('    Ok(BuildOutput {','before_return')]:
        assert text.count(anchor)==1,(anchor,text.count(anchor))
        text=text.replace(anchor,f'    notist::binding_rss::checkpoint("{label}");\n'+anchor,1)
    build.write_text(text)
    if not tagged:return
    def edit(path,transform):
        p=root/path;p.write_text(transform(p.read_text()))
    def add(path,sigs,tag):
        edit(path,lambda t: add_scopes(t,sigs,tag))
    def add_scopes(t,sigs,tag):
        for sig in sigs:t=scope(t,sig,tag)
        return t
    add('crates/notist-core/src/source.rs',['pub fn detached(', 'pub fn in_namespace(', 'pub fn anonymous('],1)
    edit('crates/notist-core/src/source.rs',lambda t:t.replace('bindings: Default::default(),','bindings: { let _rss_scope = crate::binding_rss::Scope::new(2); Default::default() },'))
    binding=core/'binding.rs'
    if binding.exists():
        text=binding.read_text().replace('#[derive(Clone, Debug, Default, PartialEq, Eq)]','#[derive(Clone, Debug, PartialEq, Eq)]')
        text+='\nimpl Default for Bindings { fn default() -> Self { let _rss_scope = crate::binding_rss::Scope::new(3); Self(Default::default()) } }\n'
        for sig,tag in [('pub fn declare_with_owner(',4),('pub fn merge(',5),('pub fn retain_sources(',5)]:text=scope(text,sig,tag)
        binding.write_text(text)
        add('crates/notist-core/src/signature.rs',['fn resolve_declaration('],6)
        edit('crates/notist-pipeline/src/modules.rs',lambda t:scope(t,'fn shared(',7).replace('crate::binding_rss::','notist_core::binding_rss::'))
    add('crates/notist-core/src/source.rs',['pub fn insert(', 'pub fn merge(', 'fn from(source: Source)'],8)
    edit('crates/notist-eval/src/eval.rs',lambda t:scope(scope(t,'fn insert(&mut self,',9),'fn new() -> Self',9).replace('crate::binding_rss::','notist_core::binding_rss::'))
    edit('crates/notist-pipeline/src/code/check.rs',lambda t:scope(t,'fn ensure_instance(',10).replace('crate::binding_rss::','notist_core::binding_rss::'))
    add('crates/notist-core/src/registry.rs',['fn install_shared('],11)
    edit('crates/notist-html/src/components.rs',lambda t:scope(t,'pub fn bind_component(',12).replace('crate::binding_rss::','notist_core::binding_rss::'))

def prepare(a):
    if a.out.exists():raise ValueError('never overwrite evidence')
    original=verify_reference(SimpleNamespace(out=a.reference))
    for d in ['sources','bin','logs']:(a.out/d).mkdir(parents=True,exist_ok=True)
    (a.out/'site').symlink_to((a.reference/'site').resolve(),target_is_directory=True)
    shutil.copy2(a.reference/'output-inventory.json',a.out/'output-inventory.json')
    m={'reference':str(a.reference),'reference_binaries':original['binaries'],'sources':{},'binaries':{}}
    for flavor in ['pages','tags']:
        for parent in ['before','after']:
            kind=parent+'-'+flavor
            root=a.out/'sources'/kind
            shutil.copytree(a.reference/'sources'/parent,root)
            instrument(root,flavor=='tags')
            m['sources'][kind]=inventory(root)
    save(a.out/'manifest.json',m)

def build(a):
    m=json.loads((a.out/'manifest.json').read_text())
    for kind in m['sources']:
        assert inventory(a.out/'sources'/kind)==m['sources'][kind]
        if kind in m['binaries']:continue
        env=dict(os.environ,**BUILD_ENV,CARGO_TARGET_DIR=str(a.out/'targets'/kind))
        if kind.endswith('tags'):env['RUSTFLAGS']='--cfg binding_rss_tags -A unexpected_cfgs'
        else:env['RUSTFLAGS']='-A unexpected_cfgs'
        with (a.out/'logs'/('build-'+kind+'.log')).open('wb') as log:
            subprocess.run(['cargo','build','--release','--locked','--offline','-p','notist-cli','--bin','notist'],cwd=a.out/'sources'/kind,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
        shutil.copy2(a.out/'targets'/kind/'release/notist',a.out/'bin'/kind)
        m['binaries'][kind]=digest(a.out/'bin'/kind);save(a.out/'manifest.json',m)

def run(a):
    m=json.loads((a.out/'manifest.json').read_text());expected=json.loads((a.out/'output-inventory.json').read_text())
    jobs=[(parent+'-pages',f'{parent}-pages-{i}',False) for i in range(3) for parent in (['before','after'] if i%2==0 else ['after','before'])]
    jobs += [(parent+'-pages',parent+'-trim',True) for parent in ['before','after']]
    jobs += [(parent+'-tags',parent+'-tags',False) for parent in ['before','after']]
    results=[]
    for kind,label,trim in jobs:
        assert digest(a.out/'bin'/kind)==m['binaries'][kind]
        log=a.out/'logs'/f'{label}.stderr'
        if log.exists():raise ValueError('do not overwrite '+label)
        env=dict(environment())
        if trim:env['BINDING_RSS_TRIM']='1'
        with log.open('wb') as stderr,(a.out/'logs'/f'{label}.stdout').open('wb') as stdout:
            subprocess.run([str(a.out/'bin'/kind),'build'],cwd=a.out/'site',env=env,stdout=stdout,stderr=stderr,check=True)
        assert inventory(a.out/'site/dist/icexiv-db')==expected,label
        row={'kind':kind,'label':label,'trim':trim,'outputs_identical':True}
        for prefix,field in [('[binding-rss] ','phases'),('[binding-owner] ','owners'),('[binding-trim] ','trimmed')]:row[field]=[json.loads(line[len(prefix):]) for line in log.read_text().splitlines() if line.startswith(prefix)]
        results.append(row);save(a.out/'results.json',results)
        print(label,'verified',flush=True)
    for kind,expected_source in m['sources'].items():assert inventory(a.out/'sources'/kind)==expected_source

def ablate(a):
    """Causal, diagnostic-only variants; never change the accepted implementation."""
    m=json.loads((a.out/'manifest.json').read_text())
    for name,shared,lazy in [('shared-empty',True,False),('lazy-registry',False,True),('both',True,True)]:
        kind='after-'+name
        if kind in m['sources']:raise ValueError('do not overwrite '+kind)
        root=a.out/'sources'/kind
        shutil.copytree(a.out/'sources/after-pages',root)
        if shared:
            p=root/'crates/notist-core/src/binding.rs'
            t=p.read_text().replace('#[derive(Clone, Debug, Default, PartialEq, Eq)]','#[derive(Clone, Debug, PartialEq, Eq)]')
            t+='\nimpl Default for Bindings { fn default() -> Self { static EMPTY: std::sync::LazyLock<Arc<BTreeMap<BindingId, Arc<BindingInfo>>>> = std::sync::LazyLock::new(Default::default); Self(EMPTY.clone()) } }\n'
            p.write_text(t)
        if lazy:
            p=root/'crates/notist-core/src/source.rs';t=p.read_text()
            t=t.replace('bindings: Arc<Mutex<crate::binding::BindingRegistry>>,','bindings: std::sync::OnceLock<Arc<Mutex<crate::binding::BindingRegistry>>>,')
            t=t.replace('self.0.identity.bindings.clone()','self.0.identity.bindings.get_or_init(Default::default).clone()')
            assert 'bindings: std::sync::OnceLock' in t
            p.write_text(t)
        p=root/'crates/notist-ssg/src/build.rs';t=p.read_text()
        anchor='    notist::binding_rss::checkpoint("after_pipeline");'
        assert anchor in t
        t=t.replace(anchor,'    eprintln!("[binding-layout] documents={} capacity={} document_size={} sources_size={}", documents.len(), documents.capacity(), std::mem::size_of::<Document>(), std::mem::size_of::<notist::Sources>());\n'+anchor)
        p.write_text(t)
        m['sources'][kind]=inventory(root)
    save(a.out/'manifest.json',m)

def run_ablations(a):
    m=json.loads((a.out/'manifest.json').read_text());expected=json.loads((a.out/'output-inventory.json').read_text())
    results=[]
    kinds=['after-pages','after-shared-empty','after-lazy-registry','after-both']
    for pair in range(2):
        for kind in (kinds if pair==0 else kinds[::-1]):
            label=f'ablation-{pair}-{kind}';log=a.out/'logs'/f'{label}.stderr'
            if log.exists():raise ValueError('do not overwrite '+label)
            assert digest(a.out/'bin'/kind)==m['binaries'][kind]
            with log.open('wb') as stderr,(a.out/'logs'/f'{label}.stdout').open('wb') as stdout:
                subprocess.run([str(a.out/'bin'/kind),'build'],cwd=a.out/'site',env=environment(),stdout=stdout,stderr=stderr,check=True)
            assert inventory(a.out/'site/dist/icexiv-db')==expected,label
            row={'kind':kind,'label':label,'outputs_identical':True}
            for prefix,field in [('[binding-rss] ','phases')]:row[field]=[json.loads(line[len(prefix):]) for line in log.read_text().splitlines() if line.startswith(prefix)]
            results.append(row);save(a.out/'ablation-results.json',results)
            print(label,'verified',flush=True)

def census(a):
    m=json.loads((a.out/'manifest.json').read_text())
    for parent in ['before','after']:
        kind=parent+'-census'
        if kind in m['sources']:raise ValueError('do not overwrite '+kind)
        root=a.out/'sources'/kind
        shutil.copytree(a.out/'sources'/(parent+'-pages'),root)
        p=root/'src/vault.rs'
        with p.open('a') as f:f.write('''
impl<R> Vault<R> {
    pub fn binding_rss_census(&self) {
        let environments: usize = self.environments.values().map(|versions| versions.len()).sum();
        eprintln!("[binding-census] environments={} graphs={} environment_size={} context_size={}", environments, self.environment_cache.binding_rss_graphs(), std::mem::size_of::<Environment>(), std::mem::size_of::<crate::Context>());
    }
}
''')
        p=root/'src/environment/loader.rs'
        with p.open('a') as f:f.write('\nimpl EnvironmentCache { pub(crate) fn binding_rss_graphs(&self) -> usize { self.graphs.len() } }\n')
        p=root/'crates/notist-ssg/src/build.rs';t=p.read_text()
        anchor='    notist::binding_rss::checkpoint("after_pipeline");'
        t=t.replace(anchor,'    vault.binding_rss_census();\n    eprintln!("[binding-layout] documents={} capacity={} document_size={} sources_size={}", documents.len(), documents.capacity(), std::mem::size_of::<Document>(), std::mem::size_of::<notist::Sources>());\n'+anchor)
        p.write_text(t)
        m['sources'][kind]=inventory(root)
    save(a.out/'manifest.json',m)

def run_census(a):
    m=json.loads((a.out/'manifest.json').read_text());expected=json.loads((a.out/'output-inventory.json').read_text())
    for kind in ['before-census','after-census']:
        p=a.out/'logs'/f'{kind}.stderr'
        if p.exists():raise ValueError('do not overwrite '+kind)
        assert digest(a.out/'bin'/kind)==m['binaries'][kind]
        with p.open('wb') as stderr,(a.out/'logs'/f'{kind}.stdout').open('wb') as stdout:
            subprocess.run([str(a.out/'bin'/kind),'build'],cwd=a.out/'site',env=environment(),stdout=stdout,stderr=stderr,check=True)
        assert inventory(a.out/'site/dist/icexiv-db')==expected
        print(kind,'verified',flush=True)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('action',choices=['prepare','build','run','ablate','run_ablations','census','run_census','all'])
    p.add_argument('--out',type=Path,required=True)
    p.add_argument('--reference',type=Path,default=Path('/tmp/opencode/notist-binding-identity-complete-20261108-v2'))
    a=p.parse_args();a.out=a.out.resolve();a.reference=a.reference.resolve()
    for action in (['prepare','build','run'] if a.action=='all' else [a.action]):globals()[action](a)
