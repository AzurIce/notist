use notist::{Executor, FsResources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    builtins::book::BookConfig,
    extensions::{CompileInput, SiteContext, SiteExtension},
};
use std::{
    num::NonZeroUsize,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
struct CapturePaths(Arc<Mutex<Vec<String>>>);

impl SiteExtension for CapturePaths {
    fn name(&self) -> &str {
        "capture-paths"
    }

    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        let documents = serde_json::to_value(&input.documents).unwrap();
        *self.0.lock().unwrap() = documents.as_object().unwrap().keys().cloned().collect();
        Ok(())
    }
}

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

fn builder() -> Builder {
    Builder::default().with_executor(Arc::new(
        Executor::new(NonZeroUsize::new(2).unwrap()).unwrap(),
    ))
}

#[test]
fn filesystem_discovery_exposes_portable_keys_with_and_without_book() {
    for book in [false, true] {
        for recover in [false, true] {
            let root = tempfile::tempdir().unwrap();
            for (path, source) in [
                ("docs/README.md", "# Home\n\n[AI topic](ai/topic.md)"),
                ("docs/ai/topic.md", "# Topic\n\n[Deep page](deep/page.md)"),
                ("docs/ai/deep/page.md", "# Deep"),
            ] {
                write(root.path(), path, source);
            }
            let mut expected = vec![
                "README.md".to_string(),
                "ai/deep/page.md".to_string(),
                "ai/topic.md".to_string(),
            ];
            let mut config = SiteConfig {
                content: "docs".into(),
                output: "public".into(),
                theme: Some(
                    if book {
                        "builtins.book"
                    } else {
                        "builtins.default"
                    }
                    .into(),
                ),
                directory_urls: true,
                copy_content_assets: false,
                ..Default::default()
            };
            if book {
                config.extensions.builtins.book = Some(BookConfig {
                    select: true,
                    ..Default::default()
                });
                write(
                    root.path(),
                    "docs/SUMMARY.md",
                    "- [Home](README.md)\n- @(collapsed: true)[AI](ai/)",
                );
                write(root.path(), "docs/unlisted.not", "#missing[]");
                expected.extend(["SUMMARY.md".into(), "unlisted.not".into()]);
            }
            expected.sort();
            let captured = Arc::new(Mutex::new(vec![]));
            let builder = builder().with_extension(CapturePaths(captured.clone()));
            let mut vault = Vault::new(FsResources::new(root.path()));
            let output = if recover {
                builder.serve(&mut vault, &config)
            } else {
                builder.build(&mut vault, &config)
            }
            .unwrap();

            assert_eq!(*captured.lock().unwrap(), expected);
            assert_eq!(output.pages.len(), 3);
            assert!(output.diagnostics.is_empty());
            for path in [
                "index.html",
                "ai/topic/index.html",
                "ai/deep/page/index.html",
            ] {
                let html = std::str::from_utf8(&output.files[Path::new(path)]).unwrap();
                for attribute in ["href=\"", "src=\""] {
                    for value in html.split(attribute).skip(1) {
                        let url = value.split('"').next().unwrap();
                        assert!(!url.contains('\\'), "{path}: {url}");
                        assert!(!url.contains("%5C"), "{path}: {url}");
                    }
                }
            }
            let home = std::str::from_utf8(&output.files[Path::new("index.html")]).unwrap();
            assert!(home.contains("href=\"ai/topic/\""), "{home}");
            if book {
                assert!(home.contains("AI</span>"), "{home}");
                assert!(!output.files.contains_key(Path::new("unlisted/index.html")));
            }
        }
    }
}

#[derive(Clone, Debug)]
struct BackslashDocument;

impl SiteExtension for BackslashDocument {
    fn name(&self) -> &str {
        "backslash-document"
    }

    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        input.documents.insert(r"ai\bad.md".into(), "# Bad".into());
        Ok(())
    }
}

#[test]
fn generated_backslash_document_keys_are_rejected_on_every_platform() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "content/index.md", "# Home");
    let error = builder()
        .with_extension(BackslashDocument)
        .build(
            &mut Vault::new(FsResources::new(root.path())),
            &SiteConfig::default(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(r"invalid site document path `ai\bad.md`")
    );
}

#[cfg(unix)]
#[test]
fn unix_backslash_filenames_are_not_reinterpreted_as_directories() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), r"content/ai\topic.md", "# Topic");
    let error = builder()
        .build(
            &mut Vault::new(FsResources::new(root.path())),
            &SiteConfig::default(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(r"invalid site document path `ai\topic.md`")
    );
}
