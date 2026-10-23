use notist::{MemoryResources, Vault};
use notist_ssg::{Error, Site, SiteConfig, build};
use std::path::Path;

fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut resources = MemoryResources::new("/vault");
    for (path, source) in files {
        resources.insert(path, source.as_bytes().to_vec());
    }
    resources
}
fn render(files: &[(&str, &str)]) -> Site {
    build(&mut Vault::new(resources(files)), &SiteConfig::default()).unwrap()
}
fn fail(files: &[(&str, &str)]) -> Error {
    build(&mut Vault::new(resources(files)), &SiteConfig::default()).unwrap_err()
}
fn html(site: &Site, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}
fn section<'a>(output: &'a str, marker: &str) -> &'a str {
    output
        .split(marker)
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap()
}

fn branch_open(output: &str, title: &str) -> bool {
    let navigation = section(output, "id=\"site-nav\"");
    let label = navigation.find(&format!(">{title}</")).unwrap();
    let opening = navigation[..label].rsplit_once("<details").unwrap().1;
    opening.split('>').next().unwrap().contains(" open")
}

#[test]
fn collapsed_branches_open_only_for_the_current_path_and_other_branches_default_open() {
    let site = render(&[
        ("README.not", "= Home"),
        ("guide/README.not", "= Guide"),
        ("guide/a.not", "= A"),
        ("_logs/README.md", "# Logs"),
        ("_logs/old.not", "= Old"),
        ("_logs/2026/new.not", "= New"),
        (
            "SUMMARY.not",
            "- [Home](README.not)\n- [Guide](guide/)\n- @(collapsed: true)[History](_logs/)\n  - @(collapsed: true)[Year](_logs/2026/)",
        ),
    ]);
    let home = html(&site, "index.html");
    assert!(branch_open(&home, "Home"));
    assert!(branch_open(&home, "Guide"));
    assert!(!branch_open(&home, "History"));
    assert!(!branch_open(&home, "Year"));
    let log_home = html(&site, "_logs/index.html");
    assert!(branch_open(&log_home, "History"));
    assert!(!branch_open(&log_home, "Year"));
    let old = html(&site, "_logs/old/index.html");
    assert!(branch_open(&old, "History"));
    assert!(!branch_open(&old, "Year"));
    let new = html(&site, "_logs/2026/new/index.html");
    assert!(branch_open(&new, "Home"));
    assert!(branch_open(&new, "History"));
    assert!(branch_open(&new, "Year"));
    assert!(branch_open(&new, "Guide"));
}

#[test]
fn collapsing_applies_to_explicit_pages_groups_and_drafts_without_changing_page_order() {
    let files = [
        ("README.not", "= Home"),
        ("a.not", "= A"),
        ("b.not", "= B"),
        ("c.not", "= C"),
        ("d.not", "= D"),
        ("e.not", "= E"),
        (
            "SUMMARY.not",
            "- [Home](README.not)\n- @(collapsed: true)[A](a.not)\n  - [B](b.not)\n- @(collapsed: true)\n  Group\n  - [C](c.not)\n- @(collapsed: true)[Draft]()\n  - [D](d.not)\n- @(collapsed: false)[Open group]()\n  - [E](e.not)",
        ),
    ];
    let site = render(&files);
    assert_eq!(
        site.pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        ["Home", "A", "B", "C", "D", "E"]
    );
    let home = html(&site, "index.html");
    for title in ["A", "Group", "Draft"] {
        assert!(!branch_open(&home, title), "{title}");
    }
    assert!(branch_open(&home, "Open group"));
    for (path, title) in [
        ("a/index.html", "A"),
        ("b/index.html", "A"),
        ("c/index.html", "Group"),
        ("d/index.html", "Draft"),
    ] {
        assert!(branch_open(&html(&site, path), title), "{path}");
    }
}

#[test]
fn collapsed_annotations_require_booleans() {
    for label in [
        "@(collapsed: \"true\")[History](_logs/)",
        "@(collapsed: 1)\n  Group",
    ] {
        let source = format!("- [Home](README.not)\n- {label}\n  - [A](_logs/a.not)");
        let error = fail(&[
            ("README.not", "= Home"),
            ("_logs/a.not", "= A"),
            ("SUMMARY.not", &source),
        ]);
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.path == Path::new("/vault/SUMMARY.not")
                    && error
                        .diagnostic
                        .message
                        .contains("`collapsed` must be a boolean")),
            "{errors:?}"
        );
    }
}

#[test]
fn declared_structure_orders_pages_and_groups_without_pages() {
    let site = render(&[
        ("README.not", "= Home"),
        ("b.not", "= B"),
        ("c/d.not", "= D"),
        ("a.md", "# A"),
        (
            "SUMMARY.not",
            "= 站点结构\n\n- [Home](README.not)\n- [Bee](b.not)\n- Group\n  - [D](c/d.not)\n  - [A](a.md)\n- [Draft]()",
        ),
    ]);
    assert_eq!(
        site.pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        ["Home", "B", "D", "A"]
    );
    assert!(!site.files.contains_key(Path::new("SUMMARY/index.html")));

    let home = html(&site, "index.html");
    let navigation = section(&home, "id=\"site-nav\"");
    assert!(navigation.contains("aria-current=\"page\""));
    assert!(navigation.contains(">Bee</a>"), "{navigation}");
    assert!(navigation.contains("<span class=\"nav-group\">Group</span>"));
    assert!(navigation.contains("<span class=\"nav-group\">Draft</span>"));
    assert!(home.contains("rel=\"next\" href=\"b/\""), "{home}");

    let page = html(&site, "c/d/index.html");
    assert!(page.contains("rel=\"prev\" href=\"../../b/\""), "{page}");
    assert!(page.contains("rel=\"next\" href=\"../../a/\""));
    let breadcrumbs = section(&page, "class=\"breadcrumbs\"");
    assert!(breadcrumbs.contains(">Home</a>"), "{breadcrumbs}");
    assert!(breadcrumbs.contains(">Group</span>"), "{breadcrumbs}");
}

#[test]
fn unlisted_pages_fail_the_build() {
    let error = fail(&[
        ("README.not", "= Home"),
        ("a.not", "= A"),
        ("b.not", "= B"),
        ("SUMMARY.not", "- [Home](README.not)\n- [A](a.not)"),
    ]);
    let Error::Message(message) = error else {
        panic!("expected a message: {error}")
    };
    assert!(message.contains("does not list 1 page: b.not"), "{message}");
}

#[test]
fn structure_entries_validate_targets() {
    for (entry, message) in [
        ("- [Missing](none.not)", "not a published page"),
        (
            "- [External](https://example.com)",
            "must reference a local document",
        ),
        (
            "- [Anchor](README.not#top)",
            "must not carry a query or fragment",
        ),
        (
            "- [Home](README.not)\n- [Again](README.not)",
            "duplicate structure entry",
        ),
    ] {
        let source = format!("= S\n\n{entry}");
        let error = fail(&[("README.not", "= Home"), ("SUMMARY.not", &source)]);
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.diagnostic.message.contains(message)),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|error| error.path == Path::new("/vault/SUMMARY.not")
                    && usize::from(error.diagnostic.span.end()) <= source.len())
        );
    }
}

#[test]
fn structure_is_data_not_code() {
    for (source, message) in [
        ("= S\n\nSome text\n\n- [Home](README.not)", "a single list"),
        ("= S\n\n= Again\n\n- [Home](README.not)", "a single list"),
        (
            "- [Home](README.not)\n\n---\n\n- [A](a.not)",
            "a single list",
        ),
        (
            "- [A](a.not) extra\n\n- [Home](README.not)",
            "must be the whole label",
        ),
        (
            "- [A](a.not) [B](b.not)\n\n- [Home](README.not)",
            "at most one link",
        ),
        (
            "- one\n\n  two\n\n- [Home](README.not)",
            "at most one nested list",
        ),
    ] {
        let error = fail(&[
            ("README.not", "= Home"),
            ("a.not", "= A"),
            ("b.not", "= B"),
            ("SUMMARY.not", source),
        ]);
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.diagnostic.message.contains(message)),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn the_structure_file_is_not_a_page() {
    let error = fail(&[
        ("README.md", "# Home\n\n[S](SUMMARY.not)"),
        ("a.md", "# A"),
        ("SUMMARY.not", "- [Home](README.md)\n- [A](a.md)"),
    ]);
    assert!(error.to_string().contains("not published"), "{error}");
}

#[test]
fn directories_discover_recursive_natural_order_and_directory_home_pages() {
    let site = render(&[
        ("README.not", "= Home"),
        ("_logs/README.md", "# Log home"),
        ("_logs/entry10.md", "# Ten"),
        ("_logs/entry2.not", "= Two"),
        ("_logs/no-home/one.not", "= Archived"),
        ("_logs/part/README.not", "= Nested"),
        ("_logs/part/child.not", "= Child"),
        ("SUMMARY.not", "- [Home](README.not)\n- [History](_logs/)"),
    ]);
    let titles: Vec<_> = site.pages.iter().map(|page| page.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Home", "Log home", "Two", "Ten", "Archived", "Nested", "Child"
        ]
    );
    let home = html(&site, "index.html");
    let navigation = section(&home, "id=\"site-nav\"");
    assert!(
        navigation.contains("href=\"_logs/\">History</a>"),
        "{navigation}"
    );
    assert!(!navigation.contains(">Log home</a>"), "{navigation}");
    let archived = html(&site, "_logs/no-home/one/index.html");
    let breadcrumbs = section(&archived, "class=\"breadcrumbs\"");
    assert!(breadcrumbs.contains(">History</a>"), "{breadcrumbs}");
    assert!(breadcrumbs.contains(">no-home</span>"), "{breadcrumbs}");
    assert!(archived.contains("rel=\"prev\" href=\"../../entry10/\""));
    assert!(archived.contains("rel=\"next\" href=\"../../part/\""));
}

#[test]
fn root_directory_discovery_without_a_label_matches_default_navigation() {
    let mut files = vec![
        ("README.md", "# Home"),
        ("guide/README.not", "= Guide"),
        ("guide/ch10.not", "= Ten"),
        ("guide/ch2.not", "= Two"),
        ("_logs/one.not", "= Log"),
    ];
    let default = render(&files);
    files.push(("SUMMARY.not", "- [](./)"));
    let declared = render(&files);
    assert_eq!(declared.files, default.files);
}

#[test]
fn explicit_pages_win_before_after_and_inside_directory_entries() {
    for (summary, expected) in [
        (
            "- [Home](README.not)\n- [History](_logs/)\n- Pinned\n  - [Featured](_logs/entry2.not)",
            vec!["Home", "Log home", "Ten", "Two"],
        ),
        (
            "- [Home](README.not)\n- Pinned\n  - [Featured](_logs/entry2.not)\n- [History](_logs/)",
            vec!["Home", "Two", "Log home", "Ten"],
        ),
        (
            "- [Home](README.not)\n- [History](_logs/)\n  - [Featured](_logs/entry10.not)",
            vec!["Home", "Log home", "Ten", "Two"],
        ),
    ] {
        let site = render(&[
            ("README.not", "= Home"),
            ("_logs/README.not", "= Log home"),
            ("_logs/entry2.not", "= Two"),
            ("_logs/entry10.not", "= Ten"),
            ("SUMMARY.not", summary),
        ]);
        assert_eq!(
            site.pages
                .iter()
                .map(|page| page.title.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let home = html(&site, "index.html");
        let navigation = section(&home, "id=\"site-nav\"");
        assert_eq!(
            navigation.matches(">Featured</a>").count(),
            1,
            "{navigation}"
        );
    }
}

#[test]
fn an_explicit_directory_home_elsewhere_leaves_a_group() {
    let site = render(&[
        ("README.not", "= Home"),
        ("_logs/README.not", "= Log home"),
        ("_logs/a.not", "= A"),
        (
            "SUMMARY.not",
            "- [Home](README.not)\n- [History](_logs/)\n- [Overview](_logs/README.not)",
        ),
    ]);
    assert_eq!(
        site.pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        ["Home", "A", "Log home"]
    );
    let home = html(&site, "index.html");
    let navigation = section(&home, "id=\"site-nav\"");
    assert!(
        navigation.contains("<span class=\"nav-group\">History</span>"),
        "{navigation}"
    );
    assert!(
        navigation.contains("href=\"_logs/\">Overview</a>"),
        "{navigation}"
    );
}

#[test]
fn nested_directory_claims_win_regardless_of_declaration_order() {
    for (summary, expected) in [
        (
            "- [Home](README.not)\n- [Rest](./)\n- [History](_logs/)\n- [Year](_logs/2026/)",
            vec!["Home", "Guide", "Old", "New"],
        ),
        (
            "- [Home](README.not)\n- [Year](_logs/2026/)\n- [History](_logs/)\n- [Rest](./)",
            vec!["Home", "New", "Old", "Guide"],
        ),
    ] {
        let site = render(&[
            ("README.not", "= Home"),
            ("guide.not", "= Guide"),
            ("_logs/old.not", "= Old"),
            ("_logs/2026/new.not", "= New"),
            ("SUMMARY.not", summary),
        ]);
        assert_eq!(
            site.pages
                .iter()
                .map(|page| page.title.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(!site.files.contains_key(Path::new("SUMMARY/index.html")));
    }
}

#[test]
fn directory_discovery_uses_selected_pages_and_allows_empty_groups() {
    let files = [
        ("README.not", "= Home"),
        ("_logs/a.md", "# A"),
        ("_logs/excluded/README.not", "#missing[]"),
        ("_logs/skip.not", "#missing[]"),
        ("_logs/.hidden.not", "#missing[]"),
        ("_logs/target/build.not", "#missing[]"),
        ("unselected.not", "#missing[]"),
        (
            "SUMMARY.not",
            "- [Home](README.not)\n- [History](_logs/)\n- [Empty](_logs/excluded/)",
        ),
    ];
    let config = SiteConfig {
        include: vec!["README.not".into(), "_logs/**".into()],
        exclude: vec!["_logs/excluded/**".into(), "_logs/skip.not".into()],
        ..SiteConfig::default()
    };
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&files));
        let site = if recover {
            notist_ssg::preview(&mut vault, &config)
        } else {
            build(&mut vault, &config)
        }
        .unwrap();
        assert_eq!(
            site.pages
                .iter()
                .map(|page| page.title.as_str())
                .collect::<Vec<_>>(),
            ["Home", "A"]
        );
        let home = html(&site, "index.html");
        let navigation = section(&home, "id=\"site-nav\"");
        assert!(
            navigation.contains("<span class=\"nav-group\">Empty</span>"),
            "{navigation}"
        );
    }
}

#[test]
fn pages_outside_directory_entries_still_require_coverage_in_build_and_preview() {
    let files = [
        ("README.not", "= Home"),
        ("_logs/a.not", "= A"),
        ("_logs-other/b.not", "= B"),
        ("SUMMARY.not", "- [Home](README.not)\n- [History](_logs/)"),
    ];
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&files));
        let error = if recover {
            notist_ssg::preview(&mut vault, &SiteConfig::default())
        } else {
            build(&mut vault, &SiteConfig::default())
        }
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not list 1 page: _logs-other/b.not"),
            "{error}"
        );
    }
}

#[test]
fn directory_targets_report_source_diagnostics() {
    for (entry, message) in [
        ("- [Missing](missing/)", "is not a directory"),
        ("- [File](README.not/)", "is not a directory"),
        (
            "- [Query](_logs/?sort=new)",
            "must not carry a query or fragment",
        ),
        (
            "- [Anchor](_logs/#top)",
            "must not carry a query or fragment",
        ),
        ("- [Outside](../)", "outside Vault"),
        (
            "- [External](https://example.com/)",
            "must reference a local document",
        ),
        (
            "- [History](_logs/)\n- [Again](./_logs/)",
            "duplicate structure directory",
        ),
    ] {
        let source = format!("- [Home](README.not)\n{entry}");
        let error = fail(&[
            ("README.not", "= Home"),
            ("_logs/a.not", "= A"),
            ("SUMMARY.not", &source),
        ]);
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.diagnostic.message.contains(message)),
            "{source}: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|error| error.path == Path::new("/vault/SUMMARY.not")
                    && usize::from(error.diagnostic.span.end()) <= source.len()),
            "{errors:?}"
        );
    }
}
