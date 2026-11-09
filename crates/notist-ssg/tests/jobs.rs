use notist::{
    Executor, MemoryResources, Vault,
    extensions::{DocumentContext, DocumentExtension, ExtensionResult},
};
use notist_ssg::{Builder, SiteConfig};
use std::{
    collections::HashSet,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
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
