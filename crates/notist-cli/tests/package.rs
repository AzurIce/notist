use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn failure(output: Output, expected: &str) {
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
const SITE: &str = "[package]\nname='manual'\n[site]";
const BOOK: &str = "[package]\nname='manual'\n[site]\ncontent='.'\ntheme='builtins.book'\ndirectory_urls=true\ncopy_content_assets=false\nextensions.builtins.book={enable=true}";

#[test]
fn nearest_package_builds_one_site_into_its_named_output_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", SITE);
    write(root, "content/deep/index.md", "# Home");
    success(run(&root.join("content/deep"), &["build"]));
    assert!(root.join("notist-output/manual/deep/index.html").is_file());
    assert!(!root.join("dist").exists());
    assert!(!root.join(".notist-target-manual.toml").exists());
}

#[test]
fn explicit_config_roots_all_paths_at_the_manifest_not_the_callers_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "docs/Notist.toml",
        &format!("{SITE}\noutput='../published'"),
    );
    write(root, "docs/content/index.not", "= Docs");
    success(run(root, &["build", "--config", "docs/Notist.toml"]));
    assert!(root.join("published/index.html").is_file());
    success(run(
        root,
        &[
            "build",
            "--config",
            "docs/Notist.toml",
            "--out-dir",
            "override",
        ],
    ));
    assert!(root.join("override/index.html").is_file());
    assert!(!root.join("docs/override").exists());
}

#[test]
fn parent_configuration_and_sibling_packages_do_not_participate_in_a_build() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "invalid TOML [");
    for directory in ["book", "website"] {
        write(root, &format!("{directory}/Notist.toml"), SITE);
        write(
            root,
            &format!("{directory}/content/index.md"),
            "# Independent",
        );
    }
    success(run(&root.join("book/content"), &["build"]));
    assert!(root.join("book/notist-output/manual/index.html").is_file());
    assert!(!root.join("website/notist-output").exists());
    success(run(root, &["build", "--config", "website/Notist.toml"]));
    assert!(
        root.join("website/notist-output/manual/index.html")
            .is_file()
    );
}

#[test]
fn document_extensions_belong_to_the_nearest_package_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[package]\nname='parent'\n[[extensions]]\nkind='replace'\nimports={bad='missing.notc'}\nfrom='notist.math'\nto='bad.math'",
    );
    write(root, "child/Notist.toml", BOOK);
    write(root, "child/README.not", "= Child\n\n$x$");
    success(run(&root.join("child"), &["build"]));
    success(run(&root.join("child"), &["check", "README.not"]));
}

#[test]
fn site_and_single_file_commands_use_the_same_real_manifest_and_extension_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "docs/Notist.toml",
        &format!(
            "{BOOK}\n[[extensions]]\nkind='replace'\nimports={{custom='custom.notc'}}\nfrom='notist.math'\nto='custom.math'"
        ),
    );
    write(
        root,
        "docs/custom.notc",
        "fn math<C: Type = InlineContent>(text: String) -> C { [replaced] }",
    );
    write(root, "docs/README.not", "= Math\n\n$x$");
    success(run(root, &["build", "--config", "docs/Notist.toml"]));
    success(run(
        root,
        &["html", "docs/README.not", "--out-dir", "single"],
    ));
    for path in ["docs/notist-output/manual/index.html", "single/index.html"] {
        assert!(
            std::fs::read_to_string(root.join(path))
                .unwrap()
                .contains("replaced")
        );
    }
    let entries: Vec<_> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        !entries
            .iter()
            .any(|n| n.to_string_lossy().starts_with(".notist-target-"))
    );
}

#[test]
fn module_only_packages_support_single_files_but_have_no_site_to_build_or_serve() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[package]\nname='content-library'");
    write(root, "README.not", "= Library");
    success(run(root, &["check", "README.not"]));
    failure(run(root, &["build"]), "has no [site]");
    failure(run(root, &["serve"]), "has no [site]");
}

#[test]
fn site_requires_package_identity_but_does_not_accept_target_or_document_layers() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (manifest, message) in [
        ("[site]", "require [package]"),
        ("[package]\nname='../bad'\n[site]", "invalid package name"),
        ("[package]\nname=''\n[site]", "invalid package name"),
        ("[package]\nname='p'\nunknown=true\n[site]", "unknown field"),
        ("[package]\nname='p'\n[site]\nroot='docs'", "unknown field"),
        (
            "[package]\nname='p'\n[site]\nname='target'",
            "unknown field",
        ),
        (
            "[package]\nname='p'\n[site]\ndocument_extensions=[]",
            "unknown field",
        ),
    ] {
        write(root, "Notist.toml", manifest);
        failure(run(root, &["build"]), message);
    }
}

#[test]
fn obsolete_manifest_shapes_are_rejected_by_both_build_and_document_commands() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "README.not", "= Home");
    for manifest in [
        "[project]\nname='p'\n[[site]]",
        "[package]\nname='p'\n[[site]]",
        "[book]\noutput='public'",
        "[ssg]\noutput='public'",
    ] {
        write(root, "Notist.toml", manifest);
        failure(run(root, &["build"]), "use [package] and optional [site]");
        failure(
            run(root, &["check", "README.not"]),
            "use [package] and optional [site]",
        );
    }
}

#[test]
fn obsolete_selection_flags_are_not_part_of_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["build", "--target", "p"],
        vec!["serve", "--project", "p"],
    ] {
        failure(run(temp.path(), &args), "unexpected argument");
    }
}

#[test]
fn output_cannot_replace_package_inputs_and_publication_still_requires_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", SITE);
    write(root, "content/index.not", "= Home");
    write(root, "unowned/keep.txt", "Keep");
    failure(
        run(root, &["build", "--out-dir", "."]),
        "output would replace package inputs",
    );
    failure(
        run(root, &["build", "--out-dir", "unowned"]),
        "refusing to replace",
    );
    assert_eq!(
        std::fs::read_to_string(root.join("unowned/keep.txt")).unwrap(),
        "Keep"
    );
    failure(
        run(root, &["build", "--out-dir", ""]),
        "a value is required",
    );
}

#[test]
fn a_sibling_package_requires_an_explicit_publication_mount() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "book/Notist.toml", BOOK);
    write(
        root,
        "book/README.md",
        "# Home\n\n[Other](../website/content/index.md)",
    );
    write(root, "website/Notist.toml", SITE);
    write(root, "website/content/index.md", "# Other");
    let output = run(&root.join("book"), &["build"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside Vault"));
    success(output);
    assert!(!root.join("book/notist-output/manual/website").exists());
    write(
        root,
        "book/Notist.toml",
        &format!("{BOOK}\n[[site.mounts]]\nsource='../website/content'\npath='website'"),
    );
    write(
        root,
        "book/SUMMARY.md",
        "- [Home](README.md)\n- [Website](website/index.md)",
    );
    success(run(&root.join("book"), &["build"]));
    assert!(
        root.join("book/notist-output/manual/website/index.html")
            .is_file()
    );
    assert!(!root.join("website/notist-output").exists());
}

fn workspace(root: &Path) {
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['a','b','library']",
    );
    for name in ["a", "b"] {
        write(
            root,
            &format!("{name}/Notist.toml"),
            &format!("[package]\nname='{name}'\n[site]"),
        );
        write(
            root,
            &format!("{name}/content/index.md"),
            &format!("# Package {name}"),
        );
    }
    write(root, "library/Notist.toml", "[package]\nname='library'");
}

#[test]
fn workspace_outputs_are_shared_and_member_builds_remain_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    success(run(&root.join("a/content"), &["build"]));
    assert!(root.join("notist-output/a/index.html").is_file());
    assert!(!root.join("a/notist-output").exists());
    assert!(!root.join("notist-output/b").exists());
    success(run(root, &["build"]));
    assert!(root.join("notist-output/b/index.html").is_file());
    assert!(!root.join("notist-output/library").exists());
    write(root, "b/content/index.md", "# Changed B");
    success(run(root, &["build", "--config", "a/Notist.toml"]));
    assert!(
        !std::fs::read_to_string(root.join("notist-output/b/index.html"))
            .unwrap()
            .contains("Changed B")
    );
    success(run(&root.join("a"), &["build", "--workspace"]));
    assert!(
        std::fs::read_to_string(root.join("notist-output/b/index.html"))
            .unwrap()
            .contains("Changed B")
    );
}

#[test]
fn package_names_select_members_from_root_and_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    success(run(root, &["build", "-p", "a"]));
    assert!(root.join("notist-output/a/index.html").is_file());
    assert!(!root.join("notist-output/b").exists());
    success(run(&root.join("a"), &["build", "--package", "b"]));
    assert!(root.join("notist-output/b/index.html").is_file());
    success(run(root, &["build", "-p", "a", "-p", "b"]));
    failure(run(root, &["build", "-p", "missing"]), "unknown package");
    failure(run(root, &["build", "-p", "library"]), "has no [site]");
    failure(run(root, &["serve", "-p", "missing"]), "package not found");
    failure(
        run(root, &["build", "-p", "a", "--workspace"]),
        "cannot be used with",
    );
    failure(
        run(root, &["build", "-p", "a", "-p", "b", "--out-dir", "out"]),
        "exactly one package site",
    );
}

#[test]
fn workspace_root_package_is_automatic_but_default_build_only_selects_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['a','b','library']\n[package]\nname='root'\n[site]",
    );
    write(root, "content/index.not", "= Root");
    success(run(root, &["build"]));
    assert!(root.join("notist-output/root/index.html").is_file());
    assert!(!root.join("notist-output/a").exists());
    success(run(root, &["build", "--workspace"]));
    assert!(root.join("notist-output/a/index.html").is_file());
    assert!(root.join("notist-output/b/index.html").is_file());
}

#[test]
fn workspace_output_setting_changes_only_the_default_collection_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=['a','b','library']\noutput='artifacts'",
    );
    write(
        root,
        "b/Notist.toml",
        "[package]\nname='b'\n[site]\noutput='../published'",
    );
    success(run(root, &["build"]));
    assert!(root.join("artifacts/a/index.html").is_file());
    assert!(root.join("published/index.html").is_file());
    assert!(!root.join("artifacts/b").exists());
    success(run(
        root,
        &[
            "build",
            "--config",
            "a/Notist.toml",
            "--out-dir",
            "override",
        ],
    ));
    assert!(root.join("override/index.html").is_file());
    assert!(!root.join("override/a").exists());
    failure(
        run(root, &["build", "--out-dir", "shared"]),
        "requires exactly one package site",
    );
}

#[test]
fn workspace_checks_every_output_and_name_before_publishing_selected_packages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "b/Notist.toml",
        "[package]\nname='b'\n[site]\noutput='../notist-output/a/nested'",
    );
    failure(
        run(&root.join("a"), &["build"]),
        "overlapping package outputs",
    );
    assert!(!root.join("notist-output").exists());
    write(root, "b/Notist.toml", "[package]\nname='a'\n[site]");
    failure(run(root, &["build"]), "duplicate package name");
    write(root, "b/Notist.toml", "[package]\nname='b'\n[site]");
    failure(
        run(&root.join("a"), &["build", "--out-dir", "../b"]),
        "output would replace package inputs",
    );
    failure(
        run(
            &root.join("a"),
            &["build", "--out-dir", "../notist-output/b"],
        ),
        "overlapping package outputs",
    );
}

#[test]
fn workspace_membership_is_explicit_and_nested_workspaces_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    workspace(root);
    write(
        root,
        "unlisted/Notist.toml",
        "[package]\nname='unlisted'\n[site]",
    );
    failure(run(&root.join("unlisted"), &["build"]), "is not a member");
    write(root, "Notist.toml", "[workspace]\nmembers=['a','./a']");
    failure(run(root, &["build"]), "duplicate workspace member");
    write(root, "Notist.toml", "[workspace]\nmembers=['missing']");
    failure(run(root, &["build"]), "invalid workspace member");
    write(root, "Notist.toml", "[workspace]\nmembers=['a']");
    write(
        root,
        "a/Notist.toml",
        "[workspace]\nmembers=[]\n[package]\nname='a'\n[site]",
    );
    failure(run(root, &["build"]), "nested workspaces");
}

#[test]
fn aggregate_outputs_are_never_discovered_as_source_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[package]\nname='manual'\n[site]\ncontent='.'",
    );
    write(root, "index.not", "= Home");
    write(root, "notist-output/other/stale.not", "#missing()");
    success(run(root, &["build"]));
    assert!(root.join("notist-output/manual/index.html").is_file());
    assert!(!root.join("notist-output/manual/notist-output").exists());
}

#[test]
fn custom_workspace_collections_are_excluded_and_cannot_contain_package_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=[]\noutput='artifacts'\n[package]\nname='manual'\n[site]\ncontent='.'",
    );
    write(root, "index.not", "= Home");
    write(root, "artifacts/other/stale.not", "#missing()");
    success(run(root, &["build"]));
    assert!(root.join("artifacts/manual/index.html").is_file());
    write(
        root,
        "Notist.toml",
        "[workspace]\nmembers=[]\noutput='.'\n[package]\nname='manual'\n[site]",
    );
    failure(
        run(root, &["build"]),
        "output collection must not contain package inputs",
    );
}
