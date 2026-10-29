use notist::{MemoryResources, Resources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{SiteContext, SiteExtension, SiteInput},
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

fn vault() -> Vault<MemoryResources> {
    let mut resources = MemoryResources::new("/site");
    for (path, source) in [
        ("content/index.not", "= Original"),
        ("content/removed.md", "# Removed"),
        ("data/original.toml", "value='original'"),
        ("Cargo.toml", "name='generated'"),
        (
            "lib.notc",
            "fn message() -> InlineContent { [from module] }",
        ),
        (
            "templates/page.hbs",
            "{{site.data.generated.name}}|{{{page.html}}}|{{#each site.directories}}{{path}};{{/each}}",
        ),
    ] {
        resources.insert(path, source.as_bytes().to_vec());
    }
    Vault::new(resources)
}
fn html(site: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[derive(Debug)]
struct Generate;
impl SiteExtension for Generate {
    fn name(&self) -> &str {
        "generate"
    }
    fn prepare(&self, context: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        assert_eq!(input.data["original"]["value"], "original");
        let package: toml::Value =
            toml::from_str(&context.resources.source(Path::new("Cargo.toml"))?).unwrap();
        input.documents.remove(Path::new("removed.md"));
        input.documents.insert(
            "index.not".into(),
            "= Edited\n\n[Generated](new/deep/page.md#generated)".into(),
        );
        input.documents.insert("new/deep/page.md".into(), "#import \"../../../lib.notc\" as demo;\n# Generated\n\n#demo.message()\n\n[Home](../../index.not)".into());
        input.data.remove("original");
        input.data.insert(
            "generated".into(),
            json!({"name": package["name"].as_str().unwrap()}),
        );
        Ok(())
    }
}

#[test]
fn source_edits_use_normal_imports_routes_links_directories_and_template_data() {
    let mut vault = vault();
    let site = Builder::default()
        .with_extension(Generate)
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert_eq!(site.pages.len(), 2);
    assert!(site.warnings.is_empty(), "{:?}", site.warnings);
    let index = html(&site, "index.html");
    assert!(index.contains("generated|"));
    assert!(index.contains("Edited"));
    assert!(index.contains("new/deep/page.html#generated"));
    assert!(index.contains("new;new/deep;"));
    let generated = html(&site, "new/deep/page.html");
    assert!(generated.contains("from module"));
    assert!(generated.contains("../../index.html"));
    assert!(!site.files.contains_key(Path::new("removed.html")));
    assert_eq!(
        vault
            .resources()
            .source(Path::new("content/index.not"))
            .unwrap(),
        "= Original"
    );
    assert!(
        vault
            .resources()
            .kind(Path::new("content/new/deep/page.md"))
            .unwrap()
            .is_none()
    );
    let original = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert!(html(&original, "index.html").contains("Original"));
    assert!(original.files.contains_key(Path::new("removed.html")));
    assert!(!original.files.contains_key(Path::new("new/deep/page.html")));
}

#[derive(Debug)]
struct Record {
    id: &'static str,
    fail: bool,
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl SiteExtension for Record {
    fn name(&self) -> &str {
        self.id
    }
    fn prepare(&self, _: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        let mut events = self.events.lock().unwrap();
        assert_eq!(
            input.data.get("previous"),
            events.last().map(|id| json!(id)).as_ref()
        );
        events.push(self.id);
        input.data.insert("previous".into(), json!(self.id));
        if self.fail {
            Err(Error::Message("failed deliberately".into()))
        } else {
            Ok(())
        }
    }
}
#[test]
fn extensions_run_once_in_order_and_failure_stops_the_build() {
    for fail in [false, true] {
        let events = Arc::new(Mutex::new(vec![]));
        let mut builder = Builder::default();
        for id in ["first", "second", "third"] {
            builder = builder.with_extension(Record {
                id,
                fail: fail && id == "second",
                events: events.clone(),
            });
        }
        let result = builder.build(&mut vault(), &SiteConfig::default());
        if fail {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("site extension `second` (prepare): failed deliberately")
            );
            assert_eq!(*events.lock().unwrap(), ["first", "second"]);
        } else {
            assert!(result.is_ok());
            assert_eq!(*events.lock().unwrap(), ["first", "second", "third"]);
        }
    }
}

#[derive(Debug)]
struct Add {
    path: &'static str,
    source: &'static str,
}
impl SiteExtension for Add {
    fn name(&self) -> &str {
        "add"
    }
    fn prepare(&self, _: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        input.documents.insert(self.path.into(), self.source.into());
        Ok(())
    }
}
#[test]
fn invalid_paths_and_collisions_are_rejected_before_publication() {
    for path in [
        "",
        "/outside.md",
        "../outside.md",
        "a/../outside.md",
        "./a.md",
        "a//b.md",
        "a\\b.md",
        ".private/a.md",
        "_notist/a.md",
        "module.notc",
        "file.json",
        "index.md",
    ] {
        let error = Builder::default()
            .with_extension(Add {
                path,
                source: "# Generated",
            })
            .build(&mut vault(), &SiteConfig::default())
            .unwrap_err();
        assert!(
            error.to_string().contains("site extension `add`"),
            "{path}: {error}"
        );
    }
    let mut vault = vault();
    let mut resources = vault.resources().clone();
    resources.insert("static/new.html", b"collision".to_vec());
    vault = Vault::new(resources);
    let error = Builder::default()
        .with_extension(Add {
            path: "new.md",
            source: "# Generated",
        })
        .build(&mut vault, &SiteConfig::default())
        .unwrap_err();
    assert!(error.to_string().contains("collision"), "{error}");
}

#[test]
fn generated_documents_can_supply_all_content_and_report_their_own_source_errors() {
    let mut vault = Vault::new(MemoryResources::new("/site"));
    let generated = Builder::default().with_extension(Add {
        path: "index.md",
        source: "# Entirely generated",
    });
    let site = generated.build(&mut vault, &SiteConfig::default()).unwrap();
    assert!(html(&site, "index.html").contains("Entirely generated"));
    let invalid = Builder::default().with_extension(Add {
        path: "invalid.not",
        source: "#notist.missing()",
    });
    let Error::Diagnostics(errors) = invalid
        .build(&mut vault, &SiteConfig::default())
        .unwrap_err()
    else {
        panic!("expected diagnostics");
    };
    assert_eq!(errors[0].path, Path::new("/site/content/invalid.not"));
    assert_eq!(errors[0].source, "#notist.missing()");
}

#[test]
fn site_extension_config_is_separate_from_document_extension_config() {
    let config: SiteConfig = toml::from_str("[[extensions]]\nkind='command'\ncommand=['python3','examples.py']\nwatch=['../Cargo.toml']").unwrap();
    assert_eq!(config.extensions.len(), 1);
    for config in [
        "[[extensions]]\nkind='command'\ncommand=[]",
        "[[extensions]]\nkind='unknown'\ncommand=['sh']",
        "[[extensions]]\nkind='command'\ncommand=['sh']\nstages=['prepare']",
    ] {
        assert!(
            toml::from_str::<SiteConfig>(config)
                .map_err(|e| e.to_string())
                .and_then(|c| c.validate().map_err(|e| e.to_string()))
                .is_err()
        );
    }
}
