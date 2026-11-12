use notist::{
    Executor, MemoryResources, Vault,
    extensions::{DocumentContext, DocumentExtension, ExtensionResult},
};
use notist_ssg::{Builder, SiteConfig};
use std::{
    collections::HashSet,
    num::NonZeroUsize,
    sync::{Arc, Mutex, Weak},
    thread::{self, ThreadId},
};

#[derive(Debug)]
struct Threads(Arc<Mutex<HashSet<ThreadId>>>);
impl DocumentExtension for Threads {
    fn name(&self) -> &str {
        "test.worker-threads"
    }
    fn after_evaluate(&self, _: &DocumentContext<'_>, _: &mut notist::Item) -> ExtensionResult {
        self.0.lock().unwrap().insert(thread::current().id());
        Ok(())
    }
}

#[derive(Debug, Default)]
struct LiveTrees {
    values: Mutex<Vec<Weak<str>>>,
    maximum: std::sync::atomic::AtomicUsize,
}
#[derive(Debug)]
struct Trees(Arc<LiveTrees>);
impl DocumentExtension for Trees {
    fn name(&self) -> &str {
        "test.live-trees"
    }
    fn after_evaluate(&self, _: &DocumentContext<'_>, root: &mut notist::Item) -> ExtensionResult {
        // This new value cannot be held by analysis or an evaluator cache.
        let text: Arc<str> = Arc::from("tree-owned payload");
        let mut values = self.0.values.lock().unwrap();
        values.retain(|value| value.strong_count() != 0);
        values.push(Arc::downgrade(&text));
        self.0
            .maximum
            .fetch_max(values.len(), std::sync::atomic::Ordering::SeqCst);
        root.children_mut().unwrap().push(
            notist::Item::new(notist::Ctor::Text, Default::default())
                .with_field("text", notist::Value::Str(text.into())),
        );
        Ok(())
    }
}

#[test]
fn evaluated_trees_are_retired_on_workers_not_retained_in_page_plans() {
    for workers in [1, 4] {
        let tracker = Arc::new(LiveTrees::default());
        let mut vault = Vault::new(files()).with_extension(Trees(tracker.clone()));
        let builder = Builder::default().with_executor(Arc::new(
            Executor::new(NonZeroUsize::new(workers).unwrap()).unwrap(),
        ));
        let output = builder.build(&mut vault, &SiteConfig::default()).unwrap();
        assert_eq!(output.pages.len(), 521);
        assert!(tracker.maximum.load(std::sync::atomic::Ordering::SeqCst) <= workers);
        assert!(
            tracker
                .values
                .lock()
                .unwrap()
                .iter()
                .all(|value| value.strong_count() == 0)
        );
        assert!(
            output
                .pages
                .iter()
                .all(|page| page.html.contains("tree-owned payload"))
        );
    }
}

#[test]
fn forward_anchors_and_typed_suffixes_work_across_preparation_windows() {
    let mut resources = files();
    resources.insert(
        "content/page-0.not",
        "= Start\n\n[Later](z.not#later) #link(p\"./z.not\", fragment: \"later\")[typed]",
    );
    resources.insert("content/z.not", "= Later\n\n[Start](page-0.not#start)");
    let config = SiteConfig::default();
    let mut expected = None;
    for workers in [1, 2, 4] {
        let builder = Builder::default().with_executor(Arc::new(
            Executor::new(NonZeroUsize::new(workers).unwrap()).unwrap(),
        ));
        let site = builder
            .build(&mut Vault::new(resources.clone()), &config)
            .unwrap();
        assert!(site.warnings.is_empty(), "{:?}", site.warnings);
        assert!(
            site.pages
                .iter()
                .find(|p| p.route.path == std::path::Path::new("page-0.not"))
                .unwrap()
                .html
                .contains("href=\"z.html#later\"")
        );
        if let Some(expected) = &expected {
            assert_eq!(&site.files, expected);
        } else {
            expected = Some(site.files);
        }
    }
}

fn files() -> MemoryResources {
    let mut files = MemoryResources::new("/site");
    for i in 0..521 {
        files.insert(
            format!("content/page-{i}.not"),
            format!("= Page {i}\n\nBody *{i}*."),
        );
    }
    files.insert(
        "templates/page.tera",
        "{{page.title}}|{{page.html | safe}}|{{site.pages | length}}",
    );
    files
}

#[test]
fn serial_and_shared_executor_builds_are_identical_and_rebuilds_reuse_workers() {
    let config = SiteConfig::default();
    let resources = files();
    let serial_threads = Arc::new(Mutex::new(HashSet::new()));
    let serial =
        Builder::default().with_executor(Arc::new(Executor::new(NonZeroUsize::MIN).unwrap()));
    let mut host = Vault::new(resources.clone()).with_extension(Threads(serial_threads.clone()));
    let expected = serial.build(&mut host, &config).unwrap();
    assert!(expected.warnings.is_empty());
    assert!(expected.diagnostics.is_empty());
    assert_eq!(
        *serial_threads.lock().unwrap(),
        HashSet::from([thread::current().id()])
    );

    let threads = Arc::new(Mutex::new(HashSet::new()));
    let executor = Arc::new(Executor::new(NonZeroUsize::new(4).unwrap()).unwrap());
    let builder = Builder::default().with_executor(executor.clone());
    for _ in 0..2 {
        let mut host = Vault::new(resources.clone()).with_extension(Threads(threads.clone()));
        let actual = builder.build(&mut host, &config).unwrap();
        assert_eq!(actual.files, expected.files);
        assert!(actual.warnings.is_empty());
        assert!(actual.diagnostics.is_empty());
        assert!(threads.lock().unwrap().len() <= executor.jobs());
        assert!(!threads.lock().unwrap().contains(&thread::current().id()));
    }
    let mut host = Vault::new(resources).with_extension(Threads(threads.clone()));
    assert_eq!(
        builder.serve(&mut host, &config).unwrap().files,
        expected.files
    );
    assert!(threads.lock().unwrap().len() <= executor.jobs());
}
