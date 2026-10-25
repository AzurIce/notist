use notist::{MemoryResources, Vault};
use notist_book::{BookConfig, BuildOutput, Error, build};
use std::path::Path;

fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut resources = MemoryResources::new("/vault");
    for (path, source) in files {
        resources.insert(path, source.as_bytes().to_vec());
    }
    resources
}
fn render(files: &[(&str, &str)]) -> BuildOutput {
    build(
        &mut Vault::new(resources(files)),
        &BookConfig::new("target/book"),
    )
    .unwrap()
}
fn fail(files: &[(&str, &str)]) -> Error {
    build(
        &mut Vault::new(resources(files)),
        &BookConfig::new("target/book"),
    )
    .unwrap_err()
}
fn html(site: &BuildOutput, path: &str) -> String {
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
    let navigation = section(output, "id=\"book-nav\"");
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
    let navigation = section(&home, "id=\"book-nav\"");
    assert!(navigation.contains("aria-current=\"page\""));
    assert!(navigation.contains(">Bee</span>"), "{navigation}");
    assert!(
        navigation.contains("<span class=\"nav-group\"><span class=\"nav-title\">Group</span>")
    );
    assert!(navigation.contains("class=\"nav-draft\" role=\"link\" aria-disabled=\"true\""));
    assert!(home.contains("rel=\"next\" href=\"b/\""), "{home}");

    let page = html(&site, "c/d/index.html");
    assert!(page.contains("rel=\"prev\" href=\"../../b/\""), "{page}");
    assert!(page.contains("rel=\"next\" href=\"../../a/\""));
    let breadcrumbs = section(&page, "class=\"breadcrumbs\"");
    assert!(!breadcrumbs.contains(">Home</a>"), "{breadcrumbs}");
    assert!(breadcrumbs.contains(">Group</span>"), "{breadcrumbs}");
}

#[test]
fn root_page_keeps_its_declared_peers_children_and_position() {
    for (summary, navigation, titles) in [
        (
            "- @(collapsed: true)[Home](README.not)\n  - [Intro](intro.not)\n- Guide\n  - [Start](guide/start.not)\n- [Last](last.not)",
            "[Home:(Intro)][Guide:(Start)][Last:]",
            vec!["Home body", "Intro body", "Start body", "Last body"],
        ),
        (
            "- Guide\n  - [Start](guide/start.not)\n- @(collapsed: true)[Home](README.not)\n  - [Intro](intro.not)\n- [Last](last.not)",
            "[Guide:(Start)][Home:(Intro)][Last:]",
            vec!["Start body", "Home body", "Intro body", "Last body"],
        ),
    ] {
        let mut config = BookConfig::new("public");
        config.title = "Book title".into();
        config.theme = Some("theme".into());
        let site = build(&mut Vault::new(resources(&[
            ("README.not", "= Home body"),
            ("intro.not", "= Intro body"),
            ("guide/start.not", "= Start body"),
            ("last.not", "= Last body"),
            ("SUMMARY.not", summary),
            ("theme/index.hbs", "{{#each navigation}}[{{title}}:{{#each children}}({{title}}){{/each}}]{{/each}}|{{#each breadcrumbs}}[{{title}}]{{/each}}|{{#each navigation}}{{#if url}}{{title}}={{expanded}};{{/if}}{{/each}}"),
        ])), &config).unwrap();
        assert_eq!(
            site.pages
                .iter()
                .map(|page| page.title.as_str())
                .collect::<Vec<_>>(),
            titles
        );
        let start = html(&site, "guide/start/index.html");
        assert!(
            start.starts_with(&format!("{navigation}|[Guide][Start]|")),
            "{start}"
        );
        assert!(start.contains("Home=false;"), "{start}");
        let intro = html(&site, "intro/index.html");
        assert!(
            intro.starts_with(&format!("{navigation}|[Home][Intro]|")),
            "{intro}"
        );
        assert!(intro.contains("Home=true;"), "{intro}");
        let last = html(&site, "last/index.html");
        assert!(last.starts_with(&format!("{navigation}|[Last]|")), "{last}");
    }
}

#[test]
fn mdbook_summary_supports_parts_prefix_suffix_numbers_drafts_and_separators() {
    let mut config = BookConfig::new("public");
    config.theme = Some("theme".into());
    let site = build(&mut Vault::new(resources(&[
        ("README.not", "= Introduction body"),
        ("preface.not", "= Preface body"),
        ("install.not", "= Installation body"),
        ("guide.not", "= Guide body"),
        ("advanced.not", "= Advanced body"),
        ("reference.not", "= Reference body"),
        ("thanks.not", "= Contributors body"),
        ("license.not", "= License body"),
        ("SUMMARY.not", "= Summary\n\n[Introduction](README.not)\n[Preface](preface.not)\n\n---\n\n= User guide\n\n- [Installation](install.not)\n- [Guide](guide.not)\n  - [Advanced](advanced.not)\n  - [Draft]()\n\n== Ignored heading\n\n---\n\n= Reference guide\n\n- [Reference](reference.not)\n\n---\n\n[Contributors](thanks.not)\n\n[License](license.not)\n\n---"),
        ("theme/index.hbs", "{{#each navigation}}[{{kind}}:{{title}}:{{number}}{{#each children}}({{kind}}:{{title}}:{{number}}){{/each}}]{{/each}}|{{#each breadcrumbs}}[{{title}}]{{/each}}|{{#if previous}}{{previous.title}}{{/if}}>{{#if next}}{{next.title}}{{/if}}"),
    ])), &config).unwrap();
    let home = html(&site, "index.html");
    let expected = "[chapter:Introduction:][chapter:Preface:][separator::][part:User guide:][chapter:Installation:1][chapter:Guide:2(chapter:Advanced:2.1)(draft:Draft:2.2)][separator::][part:Reference guide:][chapter:Reference:3][separator::][chapter:Contributors:][chapter:License:][separator::]";
    assert!(home.starts_with(expected), "{home}");
    assert!(!home.contains("Summary"));
    assert!(!home.contains("Ignored heading"));
    let advanced = html(&site, "advanced/index.html");
    assert!(advanced.contains("|[Guide][Advanced]|"), "{advanced}");
    assert!(
        advanced.ends_with("|Guide body>Reference body"),
        "{advanced}"
    );
    assert_eq!(
        site.pages
            .iter()
            .map(|page| page.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Introduction body",
            "Preface body",
            "Installation body",
            "Guide body",
            "Advanced body",
            "Reference body",
            "Contributors body",
            "License body"
        ]
    );
}

#[test]
fn chapter_numbering_continues_through_lists_parts_and_directory_discovery() {
    let mut config = BookConfig::new("public");
    config.theme = Some("theme".into());
    let site = build(&mut Vault::new(resources(&[
        ("README.not", "= Introduction"),
        ("guide/README.not", "= Guide"),
        ("guide/a.not", "= A"),
        ("guide/b.not", "= B"),
        ("end.not", "= End"),
        ("SUMMARY.not", "= Summary\n\n[Introduction](README.not)\n\n= Guide\n\n- [Guide](guide/)\n\n---\n\n- [Draft]()\n\n= Reference\n\n- [End](end.not)"),
        ("theme/index.hbs", "{{#each navigation}}[{{title}}:{{number}}{{#each children}}({{title}}:{{number}}){{/each}}]{{/each}}"),
    ])), &config).unwrap();
    assert_eq!(
        html(&site, "index.html"),
        "[Introduction:][Guide:][Guide:1(A:1.1)(B:1.2)][:][Draft:2][Reference:][End:3]"
    );
}

#[test]
fn directory_chapters_keep_their_numbering_with_or_without_home_pages() {
    for has_home in [false, true] {
        let mut files = vec![
            ("README.not", "= Home"),
            ("_logs/old.not", "= Old"),
            ("_logs/2026/new.not", "= New"),
            ("empty/placeholder.txt", "Not a chapter"),
            ("after.not", "= After"),
            (
                "SUMMARY.not",
                "= Summary\n\n[Home](README.not)\n\n- [History](_logs/)\n- [Empty](empty/)\n- [After](after.not)",
            ),
            (
                "theme/index.hbs",
                "{{#each navigation}}{{> tree this}}{{/each}}|{{#each breadcrumbs}}[{{kind}}:{{title}}:{{number}}]{{/each}}",
            ),
            (
                "theme/tree.hbs",
                "[{{kind}}:{{title}}:{{number}}:{{#if url}}linked{{else}}unlinked{{/if}}{{#each children}}{{> tree this}}{{/each}}]",
            ),
        ];
        if has_home {
            files.extend([
                ("_logs/README.not", "= Logs home"),
                ("_logs/2026/README.not", "= 2026"),
            ]);
        }
        let mut config = BookConfig::new("public");
        config.theme = Some("theme".into());
        let site = build(&mut Vault::new(resources(&files)), &config).unwrap();
        let link = if has_home { "linked" } else { "unlinked" };
        let expected = format!(
            "[chapter:Home::linked][chapter:History:1:{link}[chapter:2026:1.1:{link}[chapter:New:1.1.1:linked]][chapter:Old:1.2:linked]][chapter:Empty:2:unlinked][chapter:After:3:linked]"
        );
        let home = html(&site, "index.html");
        assert!(home.starts_with(&expected), "{home}");
        let child = html(&site, "_logs/2026/new/index.html");
        assert!(
            child.ends_with("|[chapter:History:1][chapter:2026:1.1][chapter:New:1.1.1]"),
            "{child}"
        );
        assert_eq!(
            site.files.contains_key(Path::new("_logs/index.html")),
            has_home
        );
        assert!(!site.files.contains_key(Path::new("empty/index.html")));
    }
}

#[test]
fn invalid_summary_phase_transitions_and_blocks_report_source_diagnostics() {
    for (summary, message) in [
        (
            "- [Home](README.not)\n\n[After](after.not)\n\n- [Again](again.not)",
            "numbered chapters cannot follow suffix chapters",
        ),
        (
            "- [Home](README.not)\n\n[After](after.not)\n\n= Too late",
            "part titles cannot follow suffix chapters",
        ),
        (
            "[Directory](guide/)\n\n- [Home](README.not)",
            "must reference a single document",
        ),
        ("+ [Home](README.not)", "use the `-` marker"),
        (
            "= Summary\n\n```text\nUnexpected\n```\n\n- [Home](README.not)",
            "structure allows only",
        ),
        ("= Summary\n\n= Only a part\n\n---", "declares no entries"),
    ] {
        let error = fail(&[
            ("README.not", "= Home"),
            ("after.not", "= After"),
            ("again.not", "= Again"),
            ("guide/README.not", "= Guide"),
            ("SUMMARY.not", summary),
        ]);
        let Error::Diagnostics(errors) = error else {
            panic!("expected diagnostics: {error}")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.path == Path::new("/vault/SUMMARY.not")
                    && error.diagnostic.message.contains(message)),
            "{summary}: {errors:?}"
        );
    }
}

#[test]
fn summary_selects_pages_before_analysis_and_route_validation() {
    let files = [
        ("README.not", "= Home"),
        ("a.not", "= A"),
        ("README.md", "# Unlisted root route collision"),
        ("a/README.md", "# Unlisted chapter route collision"),
        (
            "ai/broken.not",
            "#import \"missing.notc\" as missing;\n#missing[]",
        ),
        ("_notist/private.md", "# Unlisted reserved route"),
        ("future.not", "#missing[]"),
        (
            "SUMMARY.not",
            "- [Home](README.not)\n- [A](a.not)\n- [Future]()",
        ),
    ];
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&files));
        let config = BookConfig::new("target/book");
        let site = if recover {
            notist_book::serve(&mut vault, &config)
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
        assert!(site.diagnostics.is_empty());
        for path in [
            "ai/broken/index.html",
            "_notist/private/index.html",
            "future/index.html",
        ] {
            assert!(!site.files.contains_key(Path::new(path)), "{path}");
        }
    }
}

#[test]
fn selected_routes_still_reject_collisions() {
    for summary in [
        "- [Home](README.not)\n- [A](a.not)\n- [Directory home](a/README.md)",
        "- [All](./)",
    ] {
        let error = fail(&[
            ("README.not", "= Home"),
            ("a.not", "= A"),
            ("a/README.md", "# Directory home"),
            ("SUMMARY.not", summary),
        ]);
        assert!(error.to_string().contains("route collision"), "{error}");
    }
}

#[test]
fn content_links_do_not_publish_unlisted_documents() {
    let site = render(&[
        ("README.not", "= Home\n\n[Private](ai/private.md)"),
        ("ai/private.md", "# Private"),
        ("SUMMARY.not", "- [Home](README.not)"),
    ]);
    assert!(
        site.warnings.iter().any(|error| {
            error.path == Path::new("/vault/README.not")
                && error.diagnostic.message.contains("not published")
        }),
        "{:?}",
        site.warnings
    );
    assert_eq!(site.pages.len(), 1);
    assert!(!site.files.contains_key(Path::new("ai/private/index.html")));
    assert!(html(&site, "index.html").contains("href=\"ai/private/\""));
}

#[test]
fn explicit_summary_pages_must_pass_include_and_exclude_filters() {
    for config in [
        BookConfig {
            include: vec!["README.not".into()],
            ..BookConfig::new("public")
        },
        BookConfig {
            exclude: vec!["ai/**".into()],
            ..BookConfig::new("public")
        },
    ] {
        let mut vault = Vault::new(resources(&[
            ("README.not", "= Home"),
            ("ai/private.md", "# Private"),
            (
                "SUMMARY.not",
                "- [Home](README.not)\n- [Private](ai/private.md)",
            ),
        ]));
        let error = build(&mut vault, &config).unwrap_err();
        assert!(
            error.to_string().contains("not a published page"),
            "{error}"
        );
    }
}

#[test]
fn structure_entries_validate_targets() {
    for (entry, message) in [
        ("- [Missing](none.not)", "not a published page"),
        ("- [Outside](../outside.not)", "outside Vault"),
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
        (
            "= S\n\nSome text\n\n- [Home](README.not)",
            "standalone links",
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
    let site = render(&[
        ("README.md", "# Home\n\n[S](SUMMARY.not)"),
        ("a.md", "# A"),
        ("SUMMARY.not", "- [Home](README.md)\n- [A](a.md)"),
    ]);
    assert!(
        site.warnings
            .iter()
            .any(|warning| warning.diagnostic.message.contains("not published"))
    );
    assert!(!site.files.contains_key(Path::new("SUMMARY/index.html")));
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
    let navigation = section(&home, "id=\"book-nav\"");
    assert!(
        navigation.contains("href=\"_logs/\"") && navigation.contains(">History</span>"),
        "{navigation}"
    );
    assert!(!navigation.contains(">Log home</span>"), "{navigation}");
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
        let navigation = section(&home, "id=\"book-nav\"");
        assert_eq!(
            navigation.matches(">Featured</span>").count(),
            1,
            "{navigation}"
        );
    }
}

#[test]
fn an_explicit_directory_home_elsewhere_keeps_a_numbered_chapter() {
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
    let navigation = section(&home, "id=\"book-nav\"");
    assert!(
        navigation
            .contains("<span class=\"nav-chapter\"><span class=\"nav-number\" aria-hidden=\"true\">2.</span> <span class=\"nav-title\">History</span>"),
        "{navigation}"
    );
    assert!(
        navigation.contains("href=\"_logs/\"") && navigation.contains(">Overview</span>"),
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
fn directory_discovery_uses_selected_pages_and_allows_empty_chapters() {
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
    let config = BookConfig {
        include: vec!["README.not".into(), "_logs/**".into()],
        exclude: vec!["_logs/excluded/**".into(), "_logs/skip.not".into()],
        ..BookConfig::new("target/book")
    };
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&files));
        let site = if recover {
            notist_book::serve(&mut vault, &config)
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
        let navigation = section(&home, "id=\"book-nav\"");
        assert!(
            navigation.contains(
                "<span class=\"nav-chapter\"><span class=\"nav-number\" aria-hidden=\"true\">3.</span> <span class=\"nav-title\">Empty</span>"
            ),
            "{navigation}"
        );
    }
}

#[test]
fn directory_entries_select_their_subtree_in_build_and_serve() {
    let files = [
        ("README.not", "= Home"),
        ("_logs/a.not", "= A"),
        ("_logs-other/b.not", "#missing[]"),
        ("_logs-other/b.md", "# Unlisted route collision"),
        ("SUMMARY.not", "- [Home](README.not)\n- [History](_logs/)"),
    ];
    for recover in [false, true] {
        let mut vault = Vault::new(resources(&files));
        let site = if recover {
            notist_book::serve(&mut vault, &BookConfig::new("target/book"))
        } else {
            build(&mut vault, &BookConfig::new("target/book"))
        }
        .unwrap();
        assert_eq!(
            site.pages
                .iter()
                .map(|page| page.title.as_str())
                .collect::<Vec<_>>(),
            ["Home", "A"]
        );
        assert!(
            !site
                .files
                .contains_key(Path::new("_logs-other/b/index.html"))
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
