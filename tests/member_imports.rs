use notist::{
    Environment, FunctionId, ImportKind, MemoryResources, Pipeline, Value, Vault, VaultError,
};

fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut resources = MemoryResources::new("/project");
    for (path, text) in files {
        resources.insert(path, text.as_bytes().to_vec());
    }
    resources
}

fn library() -> MemoryResources {
    resources(&[(
        "parts/issue.notc",
        "fn issue() -> InlineContent { [ticket] } fn other() -> InlineContent { [other] }",
    )])
}

#[test]
fn all_import_forms_work_in_code_markup_and_markdown() {
    for (tail, calls) in [
        ("", "issue.issue()"),
        (" as issues", "issues.issue()"),
        (": issue", "issue()"),
        (": issue as ticket", "ticket()"),
        (": issue, other", "issue() #other()"),
        (": *", "issue() #other()"),
    ] {
        for path in ["doc.not", "doc.md"] {
            let mut vault = Vault::new(library());
            // Bindings apply before the declaration and create no content.
            let source = format!("#{calls}\n\n#import \"parts/issue.notc\"{tail};\n");
            let output = vault
                .render_html(path, &source, Default::default())
                .unwrap();
            assert!(
                output.analysis.diagnostics().is_empty(),
                "{source}: {:?}",
                output.analysis.diagnostics()
            );
            assert!(
                output.evaluation.diagnostics.is_empty(),
                "{:?}",
                output.evaluation.diagnostics
            );
            assert!(output.rendered.html.contains("ticket"));
            assert_eq!(output.analysis.root().children.len(), 1);
        }
        let mut files = library();
        let source = format!(
            "import \"parts/issue.notc\"{tail}; fn run() -> InlineContent {{ [#{calls}] }}"
        );
        files.insert("main.notc", source.as_bytes().to_vec());
        let env = Environment::load_from(&files, "main.notc").unwrap();
        let pipeline = Pipeline::default();
        let a = pipeline
            .analyze("doc.not", "#run()", env.context())
            .unwrap();
        assert!(
            a.diagnostics().is_empty(),
            "{source}: {:?}",
            a.diagnostics()
        );
        let e = pipeline.evaluate(&a, env.context());
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        assert!(
            e.root
                .find(|i| i.fields.get("text") == Some(&Value::Str("ticket".into())))
                .is_some()
        );
    }
}

#[test]
fn renamed_functions_keep_lexical_scope_canonical_identity_and_instance_cache() {
    let files = resources(&[
        (
            "root.notc",
            "import \"view.notc\" as views; import \"view.notc\": view as ticket; import \"wrong.notc\": value;",
        ),
        (
            "view.notc",
            "import \"right.notc\": value; fn view<T: Type = String>() -> InlineContent { [#text(text: value())] }",
        ),
        ("right.notc", "fn value() -> String { \"right\" }"),
        ("wrong.notc", "fn value() -> Int { 42 }"),
    ]);
    let env = Environment::load_from(&files, "root.notc").unwrap();
    let id = FunctionId::new("/project/view.notc", "view");
    assert_eq!(env.context().resolve("ticket").unwrap().id, id);
    assert_eq!(env.context().resolve("self.ticket").unwrap().id, id);
    assert!(
        env.context()
            .registry()
            .function("/project/root.notc", "ticket")
            .is_none()
    );
    let p = Pipeline::default();
    let a = p
        .analyze(
            "doc.not",
            "#views.view() #ticket<String>() #self.ticket()",
            env.context(),
        )
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let calls: Vec<_> = a
        .root()
        .descendants()
        .filter(|i| i.function_id() == Some(id.clone()))
        .collect();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .windows(2)
            .all(|pair| pair[0].instance == pair[1].instance)
    );
    let e = p.evaluate(&a, env.context());
    assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
    assert_eq!(
        e.root
            .descendants()
            .filter(|i| i.fields.get("text") == Some(&Value::Str("right".into())))
            .count(),
        3
    );
}

#[test]
fn wildcard_and_explicit_imports_reexport_public_namespaces_and_renamed_functions() {
    let files = resources(&[
        (
            "root.notc",
            "import \"middle.notc\": *; import \"middle.notc\": nested as ns, renamed as again;",
        ),
        (
            "middle.notc",
            "import \"leaf.notc\" as nested; import \"leaf.notc\": leaf as renamed; fn own() -> Int { 1 }",
        ),
        ("leaf.notc", "fn leaf() -> Int { 2 }"),
    ]);
    let env = Environment::load_from(&files, "root.notc").unwrap();
    let id = FunctionId::new("/project/leaf.notc", "leaf");
    for name in ["nested.leaf", "ns.leaf", "renamed", "again", "self.renamed"] {
        assert_eq!(env.context().resolve(name).unwrap().id, id);
    }
    assert!(env.context().resolve("own").is_some());
    for name in ["middle.own", "leaf", "nested.strong", "ns.notist.strong"] {
        assert!(env.context().resolve(name).is_none(), "{name}");
    }
    assert!(
        !env.context()
            .public_bindings("/project/middle.notc")
            .contains_key("strong")
    );
    assert_eq!(env.modules().len(), 3);
}

#[test]
fn locals_shadow_direct_imports_and_imports_are_not_runtime_values() {
    let files = resources(&[("dep.notc", "fn value() -> Int { 1 }")]);
    let vault = Vault::new(files);
    for source in [
        "import \"dep.notc\": value; fn bad(value: Int) -> Int { value() }",
        "import \"dep.notc\": value; fn bad() -> Int { let value = 2; value() }",
        "import \"dep.notc\": value; fn bad() -> Int { value }",
        "import \"dep.notc\" as dep; fn bad() -> Int { dep }",
    ] {
        let a = vault.analyze_code("root.notc", source).unwrap();
        assert!(!a.diagnostics.is_empty(), "{source}");
    }
    let a = vault
        .analyze_code(
            "root.notc",
            "import \"dep.notc\": value; fn good(value: Int) -> Int { self.value() }",
        )
        .unwrap();
    assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);
}

#[test]
fn explicit_and_wildcard_conflicts_report_both_origins_in_either_order() {
    let files = resources(&[
        ("a.notc", "fn shared() -> Int { 1 }"),
        ("b.notc", "fn shared() -> Int { 2 }"),
    ]);
    for first in [": shared", ": *", " as shared"] {
        for second in [": shared", ": *", " as shared"] {
            for (a, b) in [("a", "b"), ("b", "a")] {
                let source = format!("import \"{a}.notc\"{first};\nimport \"{b}.notc\"{second};");
                let analysis = Vault::new(files.clone())
                    .analyze_code("root.notc", &source)
                    .unwrap();
                let d = analysis
                    .diagnostics
                    .iter()
                    .find(|d| d.message.contains("import name conflict"))
                    .unwrap();
                assert!(
                    d.message.contains("shared")
                        && d.message.contains("a.notc")
                        && d.message.contains("b.notc"),
                    "{d:?}"
                );
                assert_eq!(d.source.as_deref(), Some("/project/root.notc"));
                assert!(usize::from(d.span.start()) > source.find('\n').unwrap());
                for path in ["doc.not", "doc.md"] {
                    let source = source
                        .lines()
                        .map(|l| format!("#{l}\n"))
                        .collect::<String>();
                    let error = Vault::new(files.clone())
                        .analyze(path, &source)
                        .unwrap_err();
                    let VaultError::Environment(errors) = error else {
                        panic!("{error:?}")
                    };
                    assert!(
                        errors
                            .iter()
                            .any(|e| e.diagnostic.message.contains("import name conflict"))
                    );
                }
            }
        }
    }
}

#[test]
fn repeated_identical_imports_and_duplicate_member_aliases_are_conflicts() {
    for source in [
        "import \"a.notc\": shared, shared;",
        "import \"a.notc\": shared, other as shared;",
        "import \"a.notc\": *; import \"a.notc\": *;",
        "import \"a.notc\": shared; import \"./a.notc\": shared;",
        "import \"a.notc\": *; import \"a.notc\": shared;",
    ] {
        let mut files = resources(&[("a.notc", "fn shared() -> Int; fn other() -> Int;")]);
        files.insert("root.notc", source.as_bytes().to_vec());
        let errors = Environment::load_from(&files, "root.notc").unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.diagnostic.message.contains("import name conflict")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn local_definitions_conflict_with_every_import_form_regardless_of_order() {
    for tail in [" as shared", ": shared", ": other as shared", ": *"] {
        for before in [true, false] {
            let declaration = "fn shared() -> Int { 0 }";
            let import = format!("import \"a.notc\"{tail};");
            let source = if before {
                format!("{declaration} {import}")
            } else {
                format!("{import} {declaration}")
            };
            let a = Vault::new(resources(&[(
                "a.notc",
                "fn shared() -> Int; fn other() -> Int;",
            )]))
            .analyze_code("root.notc", &source)
            .unwrap();
            assert!(
                a.diagnostics.iter().any(|d| d
                    .message
                    .contains("local function `/project/root.notc.shared`")
                    && d.message.contains("a.notc")),
                "{:?}",
                a.diagnostics
            );
        }
    }
}

#[test]
fn imports_shadow_prelude_but_qualified_builtins_and_markup_sugar_remain_stable() {
    for tail in [": text", ": *"] {
        for path in ["doc.not", "doc.md"] {
            let mut vault = Vault::new(resources(&[(
                "dep.notc",
                "fn text() -> InlineContent { [custom] }",
            )]));
            let source = format!(
                "#import \"dep.notc\"{tail};\n#text() #notist.text(text: \"builtin\") plain"
            );
            let output = vault
                .render_html(path, &source, Default::default())
                .unwrap();
            assert!(
                output.analysis.diagnostics().is_empty(),
                "{:?}",
                output.analysis.diagnostics()
            );
            assert!(
                output.rendered.html.contains("custom")
                    && output.rendered.html.contains("builtin")
                    && output.rendered.html.contains("plain")
            );
            let env = vault.environment_for(path, &source).unwrap();
            assert_eq!(
                env.context().resolve("text").unwrap().id.module,
                "/project/dep.notc"
            );
            assert_eq!(
                env.context().resolve("notist.text").unwrap().id.module,
                "notist"
            );
        }
        let a = Vault::new(resources(&[("dep.notc", "fn text() -> Int { 5 }")]))
            .analyze_code(
                "root.notc",
                &format!("import \"dep.notc\"{tail}; fn good() -> Int {{ text() }}"),
            )
            .unwrap();
        assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);
    }
    // A namespace also shadows a prelude function: it cannot itself be called.
    let mut files = resources(&[
        ("root.notc", "import \"dep.notc\" as text;"),
        ("dep.notc", "fn f() -> Int;"),
    ]);
    let env = Environment::load_from(&files, "root.notc").unwrap();
    assert!(env.context().resolve("text").is_none());
    assert!(env.context().resolve("text.f").is_some());
    files.insert(
        "root.notc",
        b"fn text() -> Int { 5 } fn good() -> Int { text() }".to_vec(),
    );
    assert!(
        Vault::new(files)
            .analyze_code(
                "root.notc",
                "fn text() -> Int { 5 } fn good() -> Int { text() }"
            )
            .unwrap()
            .diagnostics
            .is_empty()
    );
}

#[test]
fn reserved_names_are_rejected_for_all_binding_forms_and_can_be_renamed() {
    for name in [
        "notist",
        "self",
        "true",
        "false",
        "some",
        "none",
        "compile_error",
        "Array",
    ] {
        let files = resources(&[("dep.notc", &format!("fn {name}() -> Int {{ 1 }}"))]);
        for tail in [format!(" as {name}"), format!(": {name}"), ": *".into()] {
            let source = format!("import \"dep.notc\"{tail};");
            let a = Vault::new(files.clone())
                .analyze_code("root.notc", &source)
                .unwrap();
            assert!(
                a.diagnostics
                    .iter()
                    .any(|d| d.message.contains("reserved import name")),
                "{source}: {:?}",
                a.diagnostics
            );
            for path in ["doc.not", "doc.md"] {
                let mut vault = Vault::new(files.clone());
                let result = vault.analyze(path, &format!("#{source}"));
                match result {
                    Ok(a) => assert!(!a.diagnostics().is_empty()),
                    Err(VaultError::Environment(errors)) => assert!(
                        errors
                            .iter()
                            .any(|e| e.diagnostic.message.contains("reserved import name"))
                    ),
                    Err(error) => panic!("{error:?}"),
                }
            }
        }
        let source =
            format!("import \"dep.notc\": {name} as allowed; fn good() -> Int {{ allowed() }}");
        let a = Vault::new(files)
            .analyze_code("root.notc", &source)
            .unwrap();
        assert!(a.diagnostics.is_empty(), "{source}: {:?}", a.diagnostics);
    }
    for name in [
        "fn", "import", "as", "let", "if", "else", "match", "comptime",
    ] {
        let source = format!("import \"dep.notc\": f as {name}; fn last() -> Int {{ 1 }}");
        let a = Vault::new(resources(&[]))
            .analyze_code("root.notc", &source)
            .unwrap();
        assert!(
            a.diagnostics
                .iter()
                .any(|d| d.phase == notist::Phase::Syntax)
        );
    }
}

#[test]
fn missing_members_are_not_satisfied_by_prelude_and_use_member_source_ranges() {
    for path in ["root.notc", "doc.not", "doc.md"] {
        let prefix = if path.ends_with("notc") { "" } else { "#" };
        let source =
            format!("{prefix}import \"parts/issue.notc\": issue as okay, strong as missing;\n");
        let diagnostics = if prefix.is_empty() {
            Vault::new(library())
                .analyze_code(path, &source)
                .unwrap()
                .diagnostics
        } else {
            let error = Vault::new(library()).analyze(path, &source).unwrap_err();
            let VaultError::Environment(errors) = error else {
                panic!("{error:?}")
            };
            errors
                .into_iter()
                .map(|e| {
                    assert_eq!(e.path, std::path::Path::new("/project").join(path));
                    e.diagnostic
                })
                .collect()
        };
        let d = diagnostics
            .iter()
            .find(|d| d.message.contains("no public member `strong`"))
            .unwrap();
        assert!(d.message.contains("parts/issue.notc"));
        assert_eq!(
            &source[usize::from(d.span.start())..usize::from(d.span.end())].trim(),
            &"strong as missing"
        );
    }
}

#[test]
fn cycles_are_rejected_for_members_wildcards_and_mixed_edges() {
    for tail in [": f", ": f as renamed", ": *"] {
        for back in [": f", ": *", " as root"] {
            let files = resources(&[
                (
                    "root.notc",
                    &format!("import \"dep.notc\"{tail}; fn f() -> Int {{ 1 }}"),
                ),
                (
                    "dep.notc",
                    &format!("import \"root.notc\"{back}; fn f() -> Int {{ 2 }}"),
                ),
            ]);
            let errors = Environment::load_from(&files, "root.notc").unwrap_err();
            assert!(errors.iter().any(|e| e.path.ends_with("dep.notc")
                && e.diagnostic.message.contains("circular module import")
                && !e.diagnostic.span.is_empty()));
        }
    }
}

#[test]
fn preview_discovery_and_module_ast_preserve_selection_and_member_offsets() {
    for path in ["root.notc", "doc.not", "doc.md"] {
        let prefix = if path.ends_with("notc") { "" } else { "#" };
        let source = format!(
            "{prefix}import \"parts/issue.notc\": issue as ticket, other;\n{prefix}import \"parts/issue.notc\": *;\n"
        );
        let json: serde_json::Value =
            serde_json::from_str(&notist::preview::source_imports(path, &source)).unwrap();
        assert!(json["diagnostics"].as_array().unwrap().is_empty());
        assert_eq!(json["imports"][0]["kind"], "members");
        assert_eq!(json["imports"][0]["members"][0]["name"], "issue");
        assert_eq!(json["imports"][0]["members"][0]["alias"], "ticket");
        let member = &json["imports"][0]["members"][1];
        assert_eq!(
            source[member["start"].as_u64().unwrap() as usize
                ..member["end"].as_u64().unwrap() as usize]
                .trim(),
            "other"
        );
        assert_eq!(json["imports"][1]["kind"], "wildcard");
    }
    let source = "import \"parts/issue.notc\": issue as ticket;";
    let a = Vault::new(library())
        .analyze_code("root.notc", source)
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&notist::cst_json::module_json(source, &a)).unwrap();
    assert_eq!(
        json["ast"]["children"][0]["children"][0]["kind"],
        "ImportMember"
    );
    let lowered = Pipeline::default()
        .lower(
            "doc.md",
            "#import \"parts/issue.notc\": issue as ticket;",
            Default::default(),
        )
        .unwrap();
    assert!(
        matches!(&lowered.imports[0].kind, ImportKind::Members(m) if m[0].name == "issue" && m[0].alias == "ticket")
    );
}

#[test]
fn editor_navigation_follows_renames_and_wildcards_and_respects_local_shadowing() {
    let p = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        for (tail, name) in [(": issue as ticket", "ticket"), (": *", "issue")] {
            let source = format!("#import \"parts/issue.notc\"{tail};\n#{name}()");
            let mut vault = Vault::new(library());
            let env = vault.environment_for(path, &source).unwrap();
            let definition = p
                .definition_at(path, &source, env.context(), source.rfind(name).unwrap())
                .unwrap();
            assert_eq!(
                definition.id,
                FunctionId::new("/project/parts/issue.notc", "issue")
            );
            if name == "ticket" {
                assert_eq!(
                    p.definition_at(
                        path,
                        &source,
                        env.context(),
                        source.find("issue as").unwrap()
                    )
                    .unwrap()
                    .id,
                    definition.id
                );
            }
        }
    }
    let source = "import \"parts/issue.notc\": issue as ticket; fn native() -> Int; fn f() -> InlineContent { ticket() } fn g(ticket: Int) -> InlineContent { ticket() } fn h() -> InlineContent { let ticket = 1; ticket() } fn literal() -> InlineContent { [#ticket()] }";
    let mut files = library();
    files.insert("root.notc", source.as_bytes().to_vec());
    let env = Environment::load_from(&files, "root.notc").unwrap();
    let calls: Vec<_> = source
        .match_indices("ticket()")
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(calls.len(), 4);
    for (offset, expected) in calls.into_iter().zip([true, false, false, true]) {
        assert_eq!(
            p.definition_at("root.notc", source, env.context(), offset)
                .is_some(),
            expected
        );
    }
    // Code and raw text never become navigation targets in either document frontend.
    for path in ["doc.not", "doc.md"] {
        let source = "#import \"parts/issue.notc\": issue;\n`#issue()`";
        let mut vault = Vault::new(library());
        let env = vault.environment_for(path, source).unwrap();
        assert!(
            p.definition_at(path, source, env.context(), source.rfind("issue").unwrap())
                .is_none()
        );
    }
}

#[test]
fn direct_imports_preserve_native_resources_output_mappings_and_published_asset_identity() {
    use std::path::Path;
    for mapped in [false, true] {
        let mut files = resources(&[
            ("api/issue.notc", "fn issue() -> InlineContent;"),
            (
                "facade/lib.notc",
                "import \"../api/issue.notc\": issue as direct;",
            ),
            ("renderer/lib.notc", "fn card() -> InlineContent;"),
            (
                "api/issue/index.js",
                "import './style.js'; export default class extends HTMLElement {}",
            ),
            ("api/issue/style.js", "export const style = 'api';"),
            (
                "renderer/card/index.js",
                "import './style.js'; export default class extends HTMLElement {}",
            ),
            ("renderer/card/style.js", "export const style = 'renderer';"),
        ]);
        if mapped {
            files.insert("Notist.toml", b"[[extensions]]\nkind='replace'\nimports={api='api/issue.notc', renderer='renderer/lib.notc'}\nfrom='api.issue'\nto='renderer.card'\n".to_vec());
        }
        let mut vault = Vault::new(files.clone());
        let source = "#import \"api/issue.notc\" as api;\n#import \"api/issue.notc\": issue as ticket;\n#import \"facade/lib.notc\": *;\n#api.issue() #ticket() #direct()";
        for path in ["doc.not", "doc.md"] {
            let output = vault.render_html(path, source, Default::default()).unwrap();
            assert!(
                output.analysis.diagnostics().is_empty(),
                "{:?}",
                output.analysis.diagnostics()
            );
            assert!(
                output.evaluation.diagnostics.is_empty(),
                "{:?}",
                output.evaluation.diagnostics
            );
            assert!(
                output.rendered.diagnostics.is_empty(),
                "{:?}",
                output.rendered.diagnostics
            );
            let original = FunctionId::new("/project/api/issue.notc", "issue");
            let calls: Vec<_> = output
                .analysis
                .root()
                .descendants()
                .filter(|i| i.function_id() == Some(original.clone()))
                .collect();
            assert_eq!(calls.len(), 3);
            assert!(
                calls
                    .windows(2)
                    .all(|pair| pair[0].instance == pair[1].instance)
            );
            let (module, name, root) = if mapped {
                ("/project/renderer/lib.notc", "card", "renderer")
            } else {
                ("/project/api/issue.notc", "issue", "api")
            };
            assert_eq!(output.rendered.used_components.len(), 1);
            let component = &output.rendered.used_components[0];
            assert_eq!(component.id, FunctionId::new(module, name));
            assert_eq!(
                component.module.resource(),
                Some(Path::new(&format!("/project/{root}/{name}/index.js")))
            );
            assert_eq!(
                output
                    .rendered
                    .html
                    .matches(&format!("<{} ", component.tag))
                    .count(),
                3
            );
            let env = vault.environment_for(path, source).unwrap();
            let mut assets = notist::publication::Assets::new("/public".into());
            let registrations = assets
                .components(
                    &files,
                    &env,
                    &output.rendered.used_components,
                    Path::new("nested/page.html"),
                )
                .unwrap();
            assert_eq!(registrations.matches("customElements.define").count(), 1);
            let assets = assets.finish().unwrap();
            let destination = std::path::PathBuf::from("_notist/modules")
                .join(notist_html::components::module_key(module))
                .join(name);
            assert_eq!(
                assets[&destination.join("style.js")],
                format!("export const style = '{root}';").as_bytes()
            );
            assert!(assets.contains_key(&destination.join("index.js")));
            assert_eq!(assets.len(), 3);
        }
    }
}

#[test]
fn member_imports_keep_file_level_restrictions_and_do_not_require_a_valid_file_stem() {
    for path in ["doc.not", "doc.md"] {
        for tail in [": issue", ": issue as ticket", ": *"] {
            let source = format!("#node[\n#import \"missing.notc\"{tail};\n]");
            let lowered = Pipeline::default()
                .lower(path, &source, Default::default())
                .unwrap();
            assert!(lowered.imports.is_empty());
            assert!(
                lowered
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("import"))
            );
            let (literal, _) = Pipeline::default()
                .analyze_content_literal(
                    path,
                    &format!("[\n#import \"missing.notc\"{tail};\n]"),
                    &notist::Context::default(),
                )
                .unwrap();
            assert!(
                literal
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("import"))
            );
        }
        let mut vault = Vault::new(resources(&[(
            "if.notc",
            "fn f() -> InlineContent { [valid] }",
        )]));
        let output = vault
            .render_html(path, "#import \"if.notc\": *;\n#f()", Default::default())
            .unwrap();
        assert!(output.analysis.diagnostics().is_empty());
        assert!(output.rendered.html.contains("valid"));
        let mut vault = Vault::new(resources(&[("empty.notc", "")]));
        let a = vault.analyze(path, "#import \"empty.notc\": *;").unwrap();
        assert!(a.diagnostics().is_empty());
        assert!(a.root().children.is_empty());
    }
}

#[test]
fn prepared_preview_renders_direct_members_with_the_defining_component_url() {
    use std::collections::BTreeMap;
    let inputs = notist::PreparedInputs {
        root: "/preview".into(),
        config: None,
        files: BTreeMap::from([
            (
                "parts/issue.notc".into(),
                b"fn issue() -> InlineContent;".to_vec(),
            ),
            ("parts/issue.js".into(), Vec::new()),
        ]),
        module_urls: BTreeMap::from([(
            "parts/issue.js".into(),
            "https://example.test/parts/issue.js".into(),
        )]),
    };
    let inputs = serde_json::to_string(&inputs).unwrap();
    for path in ["doc.not", "doc.md"] {
        for (tail, name) in [
            (": issue", "issue"),
            (": issue as ticket", "ticket"),
            (": *", "issue"),
        ] {
            let source = format!("#import \"parts/issue.notc\"{tail};\n#{name}()");
            let output: serde_json::Value =
                serde_json::from_str(&notist::preview::analyze_prepared(path, &source, &inputs))
                    .unwrap();
            assert!(
                output["diagnostics"].as_array().unwrap().is_empty(),
                "{output}"
            );
            assert_eq!(
                output["used_components"][0]["identity"],
                "/preview/parts/issue.notc.issue"
            );
            assert_eq!(
                output["used_components"][0]["module"],
                "https://example.test/parts/issue.js"
            );
            assert_eq!(output["used_components"][0]["name"], "issue");
        }
    }
}
