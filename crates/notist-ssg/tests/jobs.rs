use notist::{Executor, MemoryResources, Vault};
use notist_ssg::{Builder, SiteConfig};
use std::{num::NonZeroUsize, sync::Arc};

#[test]
fn forward_anchors_work_across_parallel_preparation_windows() {
    let mut resources = files();
    resources.insert(
        "content/page-0.md",
        "# Start\n\n[Later](z.md#later) [Also](z.md#later)",
    );
    resources.insert("content/z.md", "# Later\n\n[Start](page-0.md#start)");
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
                .find(|p| p.route.path == std::path::Path::new("page-0.md"))
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
            format!("content/page-{i}.md"),
            format!("# Page {i}\n\nBody *{i}*."),
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
    let serial =
        Builder::default().with_executor(Arc::new(Executor::new(NonZeroUsize::MIN).unwrap()));
    let mut host = Vault::new(resources.clone());
    let expected = serial.build(&mut host, &config).unwrap();
    assert!(expected.warnings.is_empty());
    assert!(expected.diagnostics.is_empty());

    let executor = Arc::new(Executor::new(NonZeroUsize::new(4).unwrap()).unwrap());
    let builder = Builder::default().with_executor(executor);
    for _ in 0..2 {
        let mut host = Vault::new(resources.clone());
        let actual = builder.build(&mut host, &config).unwrap();
        assert_eq!(actual.files, expected.files);
        assert!(actual.warnings.is_empty());
        assert!(actual.diagnostics.is_empty());
    }
    let mut host = Vault::new(resources);
    assert_eq!(
        builder.serve(&mut host, &config).unwrap().files,
        expected.files
    );
}
