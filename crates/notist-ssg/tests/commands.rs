#![cfg(unix)]
use notist::{FsResources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{SiteContext, SiteExtension, SiteInput},
};
use serde_json::json;
use std::{fs, path::Path};

fn setup(root: &Path, response: &str) -> SiteConfig {
    fs::create_dir_all(root.join("content")).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("content/index.not"), "= Original").unwrap();
    fs::write(root.join("data/physical.toml"), "value='loaded'").unwrap();
    fs::write(
        root.join("templates/page.hbs"),
        "{{site.data.generated}}|{{{page.html}}}",
    )
    .unwrap();
    fs::write(root.join("response.json"), response).unwrap();
    fs::write(
        root.join("edit.sh"),
        "cat > request.json\ncat response.json\n",
    )
    .unwrap();
    toml::from_str("[[extensions]]\nkind='command'\ncommand=['sh','edit.sh']").unwrap()
}

#[derive(Debug)]
struct Native;
impl SiteExtension for Native {
    fn name(&self) -> &str {
        "native"
    }
    fn prepare(&self, _: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        input.data.insert("native".into(), json!(true));
        Ok(())
    }
}

#[test]
fn commands_receive_discovered_inputs_after_native_extensions_and_generate_sources() {
    let root = tempfile::tempdir().unwrap();
    let response = json!({"version": 1, "input": {
        "documents": {"generated/page.md": "# Generated\n\n[Home](../index.not)", "index.not": "= Modified\n\n[Generated](generated/page.md)"},
        "data": {"generated": "from command"}
    }});
    let config = setup(root.path(), &response.to_string());
    let site = Builder::default()
        .with_extension(Native)
        .build(&mut Vault::new(FsResources::new(root.path())), &config)
        .unwrap();
    assert_eq!(site.pages.len(), 2);
    assert!(site.warnings.is_empty());
    let html = String::from_utf8(site.files[Path::new("index.html")].clone()).unwrap();
    assert!(html.contains("from command|"));
    assert!(html.contains("Modified"));
    assert!(html.contains("generated/page.html"));
    let request: notist_ssg::command::Request =
        serde_json::from_str(&fs::read_to_string(root.path().join("request.json")).unwrap())
            .unwrap();
    assert_eq!(request.version, 1);
    assert_eq!(request.stage, "prepare");
    assert_eq!(request.root, root.path());
    assert_eq!(
        request.input.documents[Path::new("index.not")],
        "= Original"
    );
    assert_eq!(request.input.data["physical"]["value"], "loaded");
    assert_eq!(request.input.data["native"], true);
    assert!(!root.path().join("content/generated/page.md").exists());
    assert!(!root.path().join("data/generated.json").exists());
}

#[test]
fn invalid_protocol_paths_sources_and_command_failures_abort_the_site_build() {
    let root = tempfile::tempdir().unwrap();
    for (response, message) in [
        ("not json", "invalid JSON response"),
        (
            r##"{"version":2}"##,
            "unsupported site extension response version",
        ),
        (
            r##"{"version":1,"errors":["missing manifest"]}"##,
            "missing manifest",
        ),
        (r##"{"version":1,"input":{}}"##, "invalid JSON response"),
        (
            r##"{"version":1,"input":{"documents":{"../escape.md":"# Escape"},"data":{}}}"##,
            "invalid site document path",
        ),
        (
            r##"{"version":1,"input":{"documents":{"index.not":"= One","index.md":"# Two"},"data":{}}}"##,
            "route collision",
        ),
    ] {
        let config = setup(root.path(), response);
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert!(error.to_string().contains("site extension `sh`"));
    }
    for (command, message) in [
        (vec!["./missing"], "cannot start command"),
        (vec!["sh", "-c", "exit 7"], "exit status: 7"),
    ] {
        let config = SiteConfig {
            extensions: vec![notist_ssg::extensions::ExtensionConfig::Command {
                command: command.into_iter().map(str::to_owned).collect(),
                watch: vec![],
            }],
            ..Default::default()
        };
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}
