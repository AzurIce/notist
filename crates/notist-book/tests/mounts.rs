use notist::{MemoryResources, Vault};
use notist_book::{BookConfig, BuildOutput, Mount, build, serve};
use std::path::Path;

fn config() -> BookConfig {
    BookConfig {
        mounts: vec![Mount {
            source: "../art".into(),
            path: "art".into(),
        }],
        ..BookConfig::new("../public")
    }
}

fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut resources = MemoryResources::new("/repo/docs");
    for (path, source) in files {
        resources.insert(path, source.as_bytes().to_vec());
    }
    resources
}

fn html(site: &BuildOutput, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn mounted_pages_resolve_real_and_mounted_links_and_copy_only_used_resources() {
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&[
            (
                "README.md",
                "# Home\n\n[Drill](art/models/drill/README.md?view=full#details)\n\n[Real](../art/models/drill/README.md)",
            ),
            (
                "../art/models/drill/README.md",
                "# Drill\n\n## Details\n\n![Preview](renders/a%20b.svg?raw=1#image)\n\n[Model](model/drill.glb)\n\n[Home](../../../docs/README.md)\n\n[Next](../next.not#topic)",
            ),
            (
                "../art/models/next.not",
                "= Next\n\n== Topic\n\n[Drill](drill/README.md#details)",
            ),
            ("../art/models/drill/renders/a b.svg", "<svg/>"),
            ("../art/models/drill/model/drill.glb", "model bytes"),
            ("../art/models/drill/model/drill.blend", "unreferenced"),
            ("../art/models/drill/source/run.py", "unreferenced"),
        ]));
        let site = if recover {
            serve(&mut vault, &config())
        } else {
            build(&mut vault, &config())
        }
        .unwrap();
        assert!(site.warnings.is_empty(), "{:?}", site.warnings);
        assert!(html(&site, "index.html").contains("href=\"art/models/drill/?view=full#details\""));
        let drill = html(&site, "art/models/drill/index.html");
        assert!(drill.contains("<img src=\"renders/a%20b.svg?raw=1#image\" alt=\"Preview\">"));
        assert!(drill.contains("href=\"model/drill.glb\""));
        assert!(drill.contains("href=\"../../../\""));
        assert!(drill.contains("href=\"../next/#topic\""));
        assert!(html(&site, "art/models/next/index.html").contains("href=\"../drill/#details\""));
        assert_eq!(site.pages.len(), 3);
        assert_eq!(
            site.files[Path::new("art/models/drill/model/drill.glb")],
            b"model bytes"
        );
        assert_eq!(
            site.files[Path::new("art/models/drill/renders/a b.svg")],
            b"<svg/>"
        );
        assert!(!site.files.keys().any(|path| {
            path.extension()
                .is_some_and(|ext| ext == "blend" || ext == "py")
        }));
        let page = site
            .pages
            .iter()
            .find(|page| page.title == "Drill")
            .unwrap();
        assert_eq!(
            page.route.source,
            Path::new("/repo/art/models/drill/README.md")
        );
        assert_eq!(page.route.path, Path::new("art/models/drill/README.md"));
    }
}

#[test]
fn mounted_chapters_use_book_filters_and_summary_selection_before_analysis() {
    for summary in [
        "- [Home](README.md)\n- [Models](art/models/)",
        "- [Home](README.md)\n- [Models](../art/models/)",
        "- [Everything](./)",
    ] {
        let mut vault = Vault::new(resources(&[
            ("README.md", "# Home"),
            ("SUMMARY.md", summary),
            (
                "../art/models/drill/README.md",
                "# Drill\n\n[Excluded](../draft.md)",
            ),
            (
                "../art/models/draft.md",
                "# Broken\n\n![Missing](missing.png)",
            ),
            ("../art/.hidden/README.md", "![Missing](missing.png)"),
            ("../art/node_modules/README.md", "![Missing](missing.png)"),
            ("../art/other.md", "![Missing](missing.png)"),
        ]));
        let mut options = config();
        options.include = vec!["README.md".into(), "art/models/**".into()];
        options.exclude = vec!["art/models/draft.md".into()];
        let site = build(&mut vault, &options).unwrap();
        assert_eq!(site.pages.len(), 2);
        assert_eq!(site.warnings.len(), 1);
        assert!(html(&site, "art/models/drill/index.html").contains("href=\"../draft/\""));
        assert!(
            !site
                .files
                .contains_key(Path::new("art/models/draft/index.html"))
        );
    }
    // A normal hyperlink cannot publish a chapter omitted by SUMMARY.
    let mut vault = Vault::new(resources(&[
        ("README.md", "# Home\n\n[Other](art/broken.md)"),
        (
            "SUMMARY.md",
            "- [Home](README.md)\n- [Drill](art/README.md)",
        ),
        ("../art/README.md", "# Drill"),
        ("../art/broken.md", "![Missing](missing.png)"),
    ]));
    let site = build(&mut vault, &config()).unwrap();
    assert_eq!(site.pages.len(), 2);
    assert_eq!(site.warnings.len(), 1);
}

#[test]
fn summary_supports_virtual_parent_directories_and_specific_mount_claims() {
    let mut vault = Vault::new(resources(&[
        ("README.md", "# Home"),
        (
            "SUMMARY.not",
            "- [Home](README.md)\n- [Assets](assets/)\n- [Pinned](assets/art/models/)",
        ),
        ("../art/README.md", "# Art"),
        ("../art/models/drill.md", "# Drill"),
    ]));
    let mut options = config();
    options.mounts[0].path = "assets/art".into();
    let site = build(&mut vault, &options).unwrap();
    assert_eq!(
        site.pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        ["Home", "Art", "Drill"]
    );
    assert!(
        site.files
            .contains_key(Path::new("assets/art/models/drill/index.html"))
    );
}

#[test]
fn mounts_do_not_open_unregistered_paths_or_hide_missing_resources() {
    for (target, expected) in [
        ("missing.png", "missing local resource"),
        ("../../private/secret.png", "outside Vault"),
    ] {
        let files = resources(&[
            ("../art/README.md", &format!("# Art\n\n![Image]({target})")),
            ("/private/secret.png", "do not publish"),
        ]);
        for recover in [false, true] {
            let mut vault = Vault::new(files.clone());
            let error = if recover {
                serve(&mut vault, &config())
            } else {
                build(&mut vault, &config())
            }
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}

#[test]
fn mount_configuration_rejects_ambiguous_paths_and_source_output_overlap() {
    for (source, path, expected) in [
        ("../art", "", "invalid book mount path"),
        ("../art", "../escape", "invalid book mount path"),
        ("../art", "/absolute", "invalid book mount path"),
        ("../art", "_notist/art", "invalid book mount path"),
        ("../missing", "art", "not a directory"),
        ("../art/file.png", "art", "not a directory"),
        (".", "art", "outside the Vault"),
        ("..", "art", "outside the Vault"),
        ("../art", "existing", "already exists"),
        ("../art", "file.png/art", "traverses file"),
    ] {
        let mut options = config();
        options.mounts[0] = Mount {
            source: source.into(),
            path: path.into(),
        };
        let mut vault = Vault::new(resources(&[
            ("README.md", "# Home"),
            ("existing/file.txt", "occupied"),
            ("file.png", "file"),
            ("../art/file.png", "file"),
        ]));
        let error = build(&mut vault, &options).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "{source} -> {path}: {error}"
        );
    }
    for (source, path) in [
        ("../art", "other"),
        ("../art/nested", "other"),
        ("../other", "art/nested"),
        ("../other", "art"),
    ] {
        let mut options = config();
        options.mounts.push(Mount {
            source: source.into(),
            path: path.into(),
        });
        let mut vault = Vault::new(resources(&[
            ("../art/nested/file.txt", ""),
            ("../other/file.txt", ""),
        ]));
        assert!(
            build(&mut vault, &options)
                .unwrap_err()
                .to_string()
                .contains("overlapping")
        );
    }
    for output in ["../art", "../art/output", ".."] {
        let mut options = config();
        options.output = output.into();
        let mut vault = Vault::new(resources(&[("../art/README.md", "# Art")]));
        assert!(
            build(&mut vault, &options)
                .unwrap_err()
                .to_string()
                .contains("overlap")
        );
    }
    let mut options = config();
    options.output = "art/output".into();
    let mut vault = Vault::new(resources(&[("../art/README.md", "# Art")]));
    assert!(
        build(&mut vault, &options)
            .unwrap_err()
            .to_string()
            .contains("mount path and site output overlap")
    );
}

#[test]
fn mounts_parse_without_changing_include_semantics() {
    let options = BookConfig::parse(
        "[book]\noutput='public'\ninclude=['art/**']\n[[book.mounts]]\nsource='../art'\npath='art'",
        None,
    )
    .unwrap();
    assert_eq!(options.include, ["art/**"]);
    assert_eq!(options.mounts.len(), 1);
    assert_eq!(options.mounts[0].source, Path::new("../art"));
    assert!(
        BookConfig::parse(
            "[book]\noutput='public'\n[[book.mounts]]\nsource='../art'\npath='art'\nunknown=true",
            None
        )
        .is_err()
    );
}

#[test]
fn mounted_documents_keep_their_real_import_and_configuration_scope() {
    let source = "= Art\n\n$x$";
    let mut vault = Vault::new(resources(&[
        ("Notist.toml", ""),
        ("README.md", "# Home"),
        ("../art/README.not", source),
        (
            "../art/Notist.toml",
            "[[transforms]]\nkind='replace'\nimports={widget='lib.notc'}\nfrom='notist.math'\nto='widget.formula'",
        ),
        (
            "../art/lib.notc",
            "fn formula<C: Type = InlineContent>(text: String) -> C;",
        ),
        (
            "../art/formula/index.js",
            "export default class extends HTMLElement {}",
        ),
        ("../art/formula/helper.js", "export const value = 1;"),
    ]));
    let site = build(&mut vault, &config()).unwrap();
    let key = notist_html::components::module_key("/repo/art/lib.notc");
    assert!(site.files.contains_key(Path::new(&format!(
        "_notist/modules/{key}/formula/helper.js"
    ))));
    assert!(
        html(&site, "art/components.js")
            .contains(&format!("../_notist/modules/{key}/formula/index.js"))
    );
    let environment = vault
        .environment_for("/repo/art/README.not", source)
        .unwrap();
    assert_eq!(
        environment.config_path(),
        Some(Path::new("/repo/art/Notist.toml"))
    );
}

#[test]
fn mounted_themes_and_additional_files_use_the_same_namespace() {
    let mut options = config();
    options.theme = Some("art/theme".into());
    options.additional_css = vec!["art/extra.css".into()];
    options.additional_js = vec!["../art/extra.js".into()];
    let mut vault = Vault::new(resources(&[
        ("README.md", "# Home"),
        ("../art/theme/assets/custom.css", "body {}"),
        ("../art/theme/hidden.md", "![Missing](bad.png)"),
        ("../art/extra.css", "h1 {}"),
        ("../art/extra.js", "// custom script"),
        ("../art/unused.png", "unused"),
    ]));
    let site = build(&mut vault, &options).unwrap();
    assert_eq!(site.pages.len(), 1);
    assert!(
        site.files
            .contains_key(Path::new("_notist/theme/custom.css"))
    );
    assert_eq!(site.files[Path::new("art/extra.css")], b"h1 {}");
    assert!(html(&site, "index.html").contains("src=\"art/extra.js\""));
    assert!(!site.files.contains_key(Path::new("art/unused.png")));
}

#[test]
fn mounted_references_cross_between_independently_mapped_directories() {
    let mut options = config();
    options.mounts.push(Mount {
        source: "../textures".into(),
        path: "shared/textures".into(),
    });
    let mut vault = Vault::new(resources(&[
        (
            "../art/README.md",
            "# Art\n\n![Texture](../textures/stone.png)",
        ),
        ("../textures/stone.png", "texture"),
    ]));
    let site = build(&mut vault, &options).unwrap();
    assert!(
        html(&site, "art/index.html")
            .contains("<img src=\"../shared/textures/stone.png\" alt=\"Texture\">")
    );
    assert_eq!(
        site.files[Path::new("shared/textures/stone.png")],
        b"texture"
    );
}
