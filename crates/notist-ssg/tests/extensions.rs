use notist::{MemoryResources, Resources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{CompileInput, SiteContext, SiteExtension, TemplateInput},
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
            "templates/page.tera",
            "{% if site.data.generated %}{{site.data.generated.name}}{% endif %}|{{page.html | safe}}|{% for path, dir in site.directories %}{{dir.path}};{% endfor %}",
        ),
    ] {
        resources.insert(path, source.as_bytes().to_vec());
    }
    Vault::new(resources)
}
fn html(site: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn generated_documents_share_fixed_text_inputs_without_publication() {
    #[derive(Debug, Clone)]
    struct Pages;
    impl SiteExtension for Pages {
        fn name(&self) -> &str {
            "text-pages"
        }
        fn before_compile(
            &mut self,
            _: &SiteContext<'_>,
            input: &mut CompileInput,
        ) -> Result<(), Error> {
            // Read a later-sorted virtual page and an edited on-disk page. All
            // sources must be registered before the first page is evaluated.
            input.documents.insert("a.not".into(), "#import \"../views/lib.notc\" as view;\n#view.show()\n\n#view.show(p\"./z.not\")\n\n#view.show(p\"./old.not\")".into());
            input
                .documents
                .insert("z.not".into(), "virtual page text".into());
            input
                .documents
                .insert("old.not".into(), "edited source text".into());
            Ok(())
        }
    }
    let mut resources = MemoryResources::new("/site");
    resources.insert("content/old.not", "stale disk source");
    resources.insert("views/lib.notc", "fn show(source: Path = p\"../private/input.txt\") -> BlockContent { [ #text(text: read_text(source)) ] }");
    resources.insert("private/input.txt", "private body <&>");
    let mut vault = Vault::new(resources);
    let site = Builder::default()
        .with_extension(Pages)
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let output = html(&site, "a.html");
    assert!(output.contains("private body &lt;&amp;&gt;"));
    assert!(output.contains("virtual page text"));
    assert!(output.contains("edited source text"));
    assert!(!output.contains("stale disk source"));
    assert!(!output.contains("input.txt"));
    assert!(!site.files.contains_key(Path::new("private/input.txt")));
    assert_eq!(vault.input_session().dependencies().len(), 3);
}

#[derive(Debug, Clone, Default)]
struct Generate;
impl SiteExtension for Generate {
    fn name(&self) -> &str {
        "generate"
    }
    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        input.documents.remove(Path::new("removed.md"));
        input.documents.insert(
            "index.not".into(),
            "= Edited\n\n[Generated](new/deep/page.md#generated)".into(),
        );
        input.documents.insert("new/deep/page.md".into(), "#import \"../../../lib.notc\" as demo;\n# Generated\n\n#demo.message()\n\n[Home](../../index.not)".into());
        Ok(())
    }
    fn before_template(
        &mut self,
        context: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        assert_eq!(input.data["original"]["value"], "original");
        assert!(input.pages.iter().all(|page| !page.html.is_empty()));
        let package: toml::Value =
            toml::from_str(&context.resources.source(Path::new("Cargo.toml"))?).unwrap();
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

#[test]
fn generated_documents_resolve_typed_and_sugar_embeds_without_source_files() {
    #[derive(Debug, Clone)]
    struct Page;
    impl SiteExtension for Page {
        fn name(&self) -> &str {
            "page"
        }
        fn before_compile(
            &mut self,
            _: &SiteContext<'_>,
            input: &mut CompileInput,
        ) -> Result<(), Error> {
            input.documents.insert(
                "generated/deep/page.md".into(),
                r#"# Generated

#embed(p"../../image%20.svg", fragment: "one") ![Legacy](/image%2520.svg#two)"#
                    .into(),
            );
            Ok(())
        }
    }
    let mut resources = MemoryResources::new("/site");
    resources.insert("content/image%20.svg", "image");
    let mut vault = Vault::new(resources);
    let site = Builder::default()
        .with_extension(Page)
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let output = html(&site, "generated/deep/page.html");
    assert!(
        output.contains("src=\"../../image%2520.svg#one\""),
        "{output}"
    );
    assert!(
        output.contains("src=\"../../image%2520.svg#two\""),
        "{output}"
    );
    assert!(
        vault
            .resources()
            .kind(Path::new("content/generated/deep/page.md"))
            .unwrap()
            .is_none()
    );
}

#[derive(Debug, Clone)]
struct Record {
    id: &'static str,
    fail: bool,
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl SiteExtension for Record {
    fn name(&self) -> &str {
        self.id
    }
    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        let mut events = self.events.lock().unwrap();
        assert_eq!(
            input.documents.get(Path::new("previous.not")),
            events.last().map(|id| id.to_string()).as_ref()
        );
        events.push(self.id);
        input
            .documents
            .insert("previous.not".into(), self.id.into());
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
                    .contains("site extension `second` (before_compile): failed deliberately")
            );
            assert_eq!(*events.lock().unwrap(), ["first", "second"]);
        } else {
            assert!(result.is_ok());
            assert_eq!(*events.lock().unwrap(), ["first", "second", "third"]);
        }
    }
}

#[derive(Debug, Clone)]
struct Add {
    path: &'static str,
    source: &'static str,
}
impl SiteExtension for Add {
    fn name(&self) -> &str {
        "add"
    }
    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
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
    let config: SiteConfig = toml::from_str(
        "[extensions.examples]\ncommand=['python3','examples.py']\nwatch=['../Cargo.toml']",
    )
    .unwrap();
    assert_eq!(config.extensions.commands.len(), 1);
    for config in [
        "[extensions.edit]\ncommand=[]",
        "[extensions.builtins.unknown]\n",
        "[extensions.edit]\ncommand=['sh']\nstages=['prepare']",
    ] {
        assert!(
            toml::from_str::<SiteConfig>(config)
                .map_err(|e| e.to_string())
                .and_then(|c| c.validate().map_err(|e| e.to_string()))
                .is_err()
        );
    }
}

#[derive(Debug, Clone, Default)]
struct Stateful {
    title: Option<String>,
}
impl SiteExtension for Stateful {
    fn name(&self) -> &str {
        "stateful"
    }
    fn before_compile(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        assert!(
            self.title.is_none(),
            "a build must not inherit the previous instance"
        );
        self.title = input.documents.get(Path::new("index.not")).cloned();
        Ok(())
    }
    fn before_template(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        input
            .data
            .insert("captured".into(), json!(self.title.as_ref().unwrap()));
        Ok(())
    }
}

#[test]
fn native_state_is_build_scoped_even_after_compilation_failure() {
    let builder = Builder::default().with_extension(Stateful::default());
    let mut input = vault();
    for (source, succeeds) in [("= First", true), ("#missing()", false), ("= Last", true)] {
        let mut resources = input.resources().clone();
        resources.insert("content/index.not", source);
        input = Vault::new(resources);
        let result = builder.build(&mut input, &SiteConfig::default());
        if succeeds {
            assert_eq!(result.unwrap().data["captured"], source);
        } else {
            assert!(result.is_err());
        }
    }
}

#[derive(Debug, Clone)]
struct TemplateRecord {
    id: &'static str,
    events: Arc<Mutex<Vec<&'static str>>>,
    fail: bool,
}
impl SiteExtension for TemplateRecord {
    fn name(&self) -> &str {
        self.id
    }
    fn before_template(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
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
fn template_hooks_chain_data_in_order_and_stop_on_failure() {
    for fail in [false, true] {
        let events = Arc::new(Mutex::new(vec![]));
        let mut builder = Builder::default();
        for id in ["first", "second", "third"] {
            builder = builder.with_extension(TemplateRecord {
                id,
                events: events.clone(),
                fail: fail && id == "second",
            });
        }
        let result = builder.build(&mut vault(), &SiteConfig::default());
        if fail {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("site extension `second` (before_template)")
            );
            assert_eq!(*events.lock().unwrap(), ["first", "second"]);
        } else {
            assert_eq!(result.unwrap().data["previous"], "third");
            assert_eq!(*events.lock().unwrap(), ["first", "second", "third"]);
        }
    }
}
