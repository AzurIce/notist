use notist::{
    Executor, MemoryResources, Resources, Vault,
    extensions::{DocumentContext, DocumentExtension, ExtensionResult},
    resources::{ResourceError, ResourceKind},
};
use notist_ssg::{Builder, SiteConfig};
use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const PAGES: usize = 257;

#[derive(Debug)]
struct State {
    files: Mutex<MemoryResources>,
    analyzed: AtomicUsize,
    evaluated: AtomicUsize,
    component_probes: AtomicUsize,
}

#[derive(Clone)]
struct Inputs(Arc<State>);
impl Resources for Inputs {
    fn root(&self) -> &Path {
        Path::new("/site")
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        if path == Path::new("/site/content/picture.svg")
            || path.extension().is_some_and(|e| e == "js")
        {
            assert_eq!(
                self.0.evaluated.load(Ordering::SeqCst),
                PAGES,
                "publication bytes were read before all evaluations completed: {}",
                path.display()
            );
        }
        self.0.files.lock().unwrap().read(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        if path == Path::new("/site/module/panel.js")
            || path == Path::new("/site/module/panel/index.js")
        {
            assert_eq!(
                self.0.evaluated.load(Ordering::SeqCst),
                PAGES,
                "component selection moved into page preparation"
            );
            self.0.component_probes.fetch_add(1, Ordering::SeqCst);
        }
        self.0.files.lock().unwrap().kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.0.files.lock().unwrap().entries(path)
    }
}

#[derive(Debug)]
struct Hooks(Arc<State>);
impl DocumentExtension for Hooks {
    fn name(&self) -> &str {
        "test.phase-order"
    }
    fn after_check(&self, _: &DocumentContext<'_>, _: &notist::Item) -> ExtensionResult {
        assert_eq!(self.0.evaluated.load(Ordering::SeqCst), 0);
        self.0.analyzed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn after_evaluate(&self, _: &DocumentContext<'_>, _: &mut notist::Item) -> ExtensionResult {
        assert_eq!(self.0.analyzed.load(Ordering::SeqCst), PAGES);
        if self.0.evaluated.fetch_add(1, Ordering::SeqCst) + 1 == PAGES {
            let mut files = self.0.files.lock().unwrap();
            // An early page cannot decide whether this component exists until
            // every output hook has run, even across a preparation window.
            files.insert(
                "module/panel.js",
                "export default class extends HTMLElement {}",
            );
            files.insert("content/picture.svg", "final publication bytes");
        }
        Ok(())
    }
}

#[test]
fn preparation_does_not_move_component_probes_or_publication_reads_before_evaluation() {
    for workers in [1, 2] {
        let mut files = MemoryResources::new("/site");
        for i in 0..PAGES {
            files.insert(format!("content/page-{i:04}.not"), if i == 0 {
                "#import \"../module/lib.notc\" as view;\n#view.panel[\n#embed(p\"./picture.svg\")\n]".into()
            } else { format!("= Page {i}") });
        }
        files.insert(
            "module/lib.notc",
            "fn panel[children: BlockContent] -> BlockContent;",
        );
        files.insert("content/picture.svg", "stale bytes");
        let state = Arc::new(State {
            files: Mutex::new(files),
            analyzed: AtomicUsize::new(0),
            evaluated: AtomicUsize::new(0),
            component_probes: AtomicUsize::new(0),
        });
        let mut vault = Vault::new(Inputs(state.clone())).with_extension(Hooks(state.clone()));
        let builder = Builder::default().with_executor(Arc::new(
            Executor::new(NonZeroUsize::new(workers).unwrap()).unwrap(),
        ));
        let config = SiteConfig {
            copy_content_assets: false,
            ..Default::default()
        };
        let site = builder.build(&mut vault, &config).unwrap();
        // Publication may also inspect the selected module's directory.
        assert!(state.component_probes.load(Ordering::SeqCst) >= 2);
        assert_eq!(
            site.files[Path::new("picture.svg")],
            b"final publication bytes"
        );
        assert!(site.pages[0].html.contains("notist-protocol=\"2\""));
        assert!(site.pages[0].html.contains("<img src=\"picture.svg\""));
    }
}

#[test]
fn missing_or_conflicting_components_fail_without_a_publishable_partial_result() {
    for entries in [0, 2] {
        let mut files = MemoryResources::new("/site");
        files.insert(
            "content/index.not",
            "#import \"../module/lib.notc\" as view;\n#view.panel[\nVisible fallback\n]",
        );
        files.insert(
            "module/lib.notc",
            "fn panel[children: BlockContent] -> BlockContent;",
        );
        if entries == 2 {
            files.insert(
                "module/panel.js",
                "export default class extends HTMLElement {}",
            );
            files.insert(
                "module/panel/index.js",
                "export default class extends HTMLElement {}",
            );
        }
        let error = Builder::default()
            .build(&mut Vault::new(files), &SiteConfig::default())
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains(if entries == 0 {
                "no HTML implementation"
            } else {
                "conflicting component entries"
            }),
            "{message}"
        );
    }
}

#[test]
fn repeated_resource_bindings_do_not_duplicate_failed_anchor_diagnostics() {
    for references in [
        "#link(p\"./target.not\", fragment: \"present\")[ok] #link(p\"./target.not\", fragment: \"absent\")[bad]",
        "#link(p\"./target.not\", fragment: \"absent\")[bad] #link(p\"./target.not\", fragment: \"present\")[ok]",
    ] {
        let mut files = MemoryResources::new("/site");
        files.insert("content/index.not", references);
        files.insert("content/target.not", "@(id: \"present\")\n= Target");
        let error = Builder::default()
            .build(&mut Vault::new(files), &SiteConfig::default())
            .unwrap_err();
        let notist_ssg::Error::Diagnostics(errors) = error else {
            panic!("{error}");
        };
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .diagnostic
                .message
                .contains("missing anchor `#absent`")
        );
    }
}
