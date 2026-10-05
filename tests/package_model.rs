use notist::{Environment, FunctionId, MemoryResources, RenderOptions, SourceDiagnostic, Vault};
use std::path::Path;

const MATH: &str = "fn math(text: String) -> InlineContent;";
fn rule(target: &str) -> String {
    format!("\n[[transforms]]\nkind = 'replace'\nfrom = 'notist::math'\nto = '{target}::math'\n")
}
fn package(resources: &mut MemoryResources, name: &str, suffix: &str) {
    resources.insert(
        format!("packages/{name}/Notist.toml"),
        format!("[package]\nname = '{name}'\n{suffix}").into_bytes(),
    );
    resources.insert(
        format!("packages/{name}/lib.notc"),
        MATH.as_bytes().to_vec(),
    );
    resources.insert(format!("packages/{name}/components/math.js"), Vec::new());
}
fn project(config: &str) -> MemoryResources {
    let mut resources = MemoryResources::new("/repo");
    resources.insert("Notist.toml", config.as_bytes().to_vec());
    resources
}
fn load(resources: &MemoryResources) -> Result<Environment, Vec<SourceDiagnostic>> {
    Environment::load_from(resources, "Notist.toml")
}
fn assert_math(resources: MemoryResources, path: &str, target: &str) {
    let output = Vault::new(resources)
        .render_html(path, "$frac(a, b)$", RenderOptions::default())
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.transformed.diagnostics.is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    assert_eq!(
        output.rendered.used_components[0].id,
        FunctionId::new(target, "math")
    );
}

#[test]
fn root_package_registers_itself_and_exports_only_public_configuration() {
    let mut resources = project("[dependencies]\nbase = {path = 'packages/base'}");
    package(&mut resources, "helper", &rule("helper"));
    let manifest = format!(
        "[dev-dependencies]\nhelper = {{path = '../helper'}}\n{}\n[[dev-transforms]]\nkind = 'replace'\nfrom = 'notist::math'\nto = 'helper::math'\n",
        rule("base")
    );
    package(&mut resources, "base", &manifest);
    assert_math(resources.clone(), "doc.not", "base");
    let consumer = load(&resources).unwrap();
    assert_eq!(consumer.packages().len(), 1);
    assert!(consumer.registry().resolve("helper::math").is_err());
    let own = Environment::load_from(&resources, "packages/base/Notist.toml").unwrap();
    assert_eq!(own.packages().len(), 2);
    assert!(own.registry().resolve("base::math").is_ok());
    assert_math(resources, "packages/base/README.not", "helper");
}

#[test]
fn transitive_diamond_dependencies_and_identical_defaults_are_loaded_once() {
    let mut resources = project(
        "[dependencies]\nleft = {path = 'packages/left'}\nright = {path = 'packages/right'}",
    );
    package(&mut resources, "shared", &rule("shared"));
    for name in ["left", "right"] {
        package(
            &mut resources,
            name,
            &format!(
                "[dependencies]\nshared = {{path = '../shared'}}\n{}",
                rule("shared")
            ),
        );
    }
    let environment = load(&resources).unwrap();
    assert_eq!(environment.packages().len(), 3);
    for name in ["left", "right", "shared"] {
        assert!(
            environment
                .registry()
                .resolve(&format!("{name}::math"))
                .is_ok()
        );
    }
    assert_math(resources, "doc.not", "shared");
}

#[test]
fn conflicting_defaults_need_a_root_selection_independent_of_dependency_order() {
    let config = "[dependencies]\none = {path = 'packages/one'}\ntwo = {path = 'packages/two'}";
    let mut resources = project(config);
    for name in ["one", "two"] {
        package(&mut resources, name, &rule(name));
    }
    let errors = load(&resources).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .diagnostic
            .message
            .contains("conflicting default transforms")
    );
    assert!(errors[0].diagnostic.message.contains("one/Notist.toml"));
    assert!(errors[0].diagnostic.message.contains("two/Notist.toml"));
    assert!(!errors[0].diagnostic.span.is_empty());
    for name in ["one", "two"] {
        resources.insert(
            "Notist.toml",
            format!("{config}{}", rule(name)).into_bytes(),
        );
        assert_math(resources.clone(), "doc.not", name);
    }
}

#[test]
fn development_dependencies_and_transforms_also_work_for_a_document_root() {
    let mut resources = project("[dev-dependencies]\nrenderer = {path = 'packages/renderer'}");
    package(&mut resources, "renderer", "");
    assert!(
        load(&resources)
            .unwrap()
            .registry()
            .resolve("renderer::math")
            .is_ok()
    );
    resources.insert(
        "Notist.toml",
        format!(
            "[dev-dependencies]\nrenderer = {{path = 'packages/renderer'}}\n{}",
            rule("renderer").replace("[[transforms]]", "[[dev-transforms]]")
        )
        .into_bytes(),
    );
    assert_math(resources, "doc.md", "renderer");
}

#[test]
fn missing_manifest_metadata_and_mismatched_names_keep_their_origins() {
    let mut resources = project("[dependencies]\nexpected = {path = 'packages/folder'}");
    resources.insert("packages/folder/lib.notc", MATH.as_bytes().to_vec());
    let missing = load(&resources).unwrap_err();
    assert_eq!(missing[0].path, Path::new("/repo/Notist.toml"));
    assert!(missing[0].diagnostic.message.contains("cannot load"));
    resources.insert("packages/folder/Notist.toml", Vec::new());
    let unnamed = load(&resources).unwrap_err();
    assert_eq!(
        unnamed[0].path,
        Path::new("/repo/packages/folder/Notist.toml")
    );
    assert!(unnamed[0].diagnostic.message.contains("[package].name"));
    resources.insert(
        "packages/folder/Notist.toml",
        b"[package]\nname = 'actual'".to_vec(),
    );
    let mismatch = load(&resources).unwrap_err();
    assert_eq!(mismatch[0].path, Path::new("/repo/Notist.toml"));
    assert!(mismatch[0].diagnostic.message.contains("does not match"));
    for source in [
        "[package]\nname = 'notist'",
        "[package]\nname = ''",
        "[package]\nname = false",
        "[package]\nversion = '1'",
    ] {
        assert!(
            notist::environment::parse_config(source).is_err(),
            "{source}"
        );
    }
}

#[test]
fn cycles_and_different_directories_with_one_name_are_rejected() {
    let mut resources = project("[dependencies]\na = {path = 'packages/a'}");
    package(&mut resources, "a", "[dependencies]\nb = {path = '../b'}");
    package(&mut resources, "b", "[dependencies]\na = {path = '../a'}");
    assert!(
        load(&resources)
            .unwrap_err()
            .iter()
            .any(|error| error.diagnostic.message.contains("cycle"))
    );
    resources.insert(
        "packages/b/Notist.toml",
        b"[package]\nname = 'b'\n[dependencies]\na = {path = '../other'}".to_vec(),
    );
    resources.insert(
        "packages/other/Notist.toml",
        b"[package]\nname = 'a'".to_vec(),
    );
    resources.insert("packages/other/lib.notc", MATH.as_bytes().to_vec());
    assert!(
        load(&resources)
            .unwrap_err()
            .iter()
            .any(|error| error.diagnostic.message.contains("provided by both"))
    );
}

#[test]
fn invalid_public_defaults_report_the_dependency_manifest_even_with_an_override() {
    let mut resources = project(&format!(
        "[dependencies]\nbad = {{path = 'packages/bad'}}\n{}",
        rule("good")
    ));
    package(&mut resources, "good", "");
    package(
        &mut resources,
        "bad",
        &format!(
            "[dependencies]\ngood = {{path = '../good'}}\n{}",
            rule("bad")
        ),
    );
    resources.insert(
        "packages/bad/lib.notc",
        b"fn math(text: Int) -> InlineContent;".to_vec(),
    );
    let errors = load(&resources).unwrap_err();
    assert!(
        errors
            .iter()
            .all(|error| error.path == Path::new("/repo/packages/bad/Notist.toml"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.diagnostic.message.contains("parameter"))
    );
    assert!(
        errors
            .iter()
            .all(|error| error.source.contains("[[transforms]]")
                && !error.diagnostic.span.is_empty())
    );
}

#[test]
fn dependency_development_files_are_not_required_by_consumers() {
    let mut resources = project("[dependencies]\nbase = {path = 'packages/base'}");
    package(
        &mut resources,
        "base",
        "[dev-dependencies]\nmissing = {path = '../missing'}\n[[dev-transforms]]\nkind = 'replace'\nfrom = 'notist::math'\nto = 'missing::math'",
    );
    assert_eq!(load(&resources).unwrap().packages().len(), 1);
    let errors = Environment::load_from(&resources, "packages/base/Notist.toml").unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.diagnostic.message.contains("cannot load"))
    );
}
