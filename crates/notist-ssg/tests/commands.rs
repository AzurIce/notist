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
        root.join("templates/page.tera"),
        "{{site.data.generated}}|{{page.html | safe}}",
    )
    .unwrap();
    fs::write(root.join("response.json"), response).unwrap();
    fs::write(
        root.join("edit.sh"),
        "cat > request.json\ncat response.json\n",
    )
    .unwrap();
    toml::from_str("[extensions.edit]\ncommand=['sh','edit.sh']").unwrap()
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
        assert!(error.to_string().contains("site extension `edit`"));
    }
    for (command, message) in [
        (vec!["./missing"], "cannot start command"),
        (vec!["sh", "-c", "exit 7"], "exit status: 7"),
    ] {
        let config = SiteConfig {
            extensions: notist_ssg::extensions::ExtensionsConfig {
                commands: [(
                    "edit".into(),
                    notist_ssg::extensions::CommandConfig {
                        command: command.into_iter().map(str::to_owned).collect(),
                        watch: vec![],
                    },
                )]
                .into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let error = Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &config)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn explicit_order_interleaves_commands_and_builtins() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("generate.sh"),
        "cat >/dev/null\ncat response.json\n",
    )
    .unwrap();
    fs::write(
        root.path().join("response.json"),
        json!({"version": 1, "input": {
            "documents": {"index.not": "= Generated", "SUMMARY.not": "- [Home](index.not)"},
            "data": {}
        }})
        .to_string(),
    )
    .unwrap();
    let config: SiteConfig = toml::from_str("extension_order=['generate','builtins.book']\ntheme='builtins.book'\n[extensions.builtins.book]\n[extensions.generate]\ncommand=['sh','generate.sh']").unwrap();
    let output = Builder::default()
        .build(&mut Vault::new(FsResources::new(root.path())), &config)
        .unwrap();
    assert_eq!(output.pages.len(), 1);
    assert_eq!(output.data["book"]["navigation"][0]["title"], "Home");
    let mut wrong = config;
    wrong.extension_order.reverse();
    assert!(
        Builder::default()
            .build(&mut Vault::new(FsResources::new(root.path())), &wrong)
            .unwrap_err()
            .to_string()
            .contains("book contains no selected documents")
    );
}
