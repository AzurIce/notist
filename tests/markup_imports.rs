use notist::{Ctor, FrontendOptions, MemoryResources, Pipeline, Value, Vault};

fn files() -> MemoryResources {
    let mut files = MemoryResources::new("/project");
    files.insert(
        "views.notc",
        b"import \"parts/text.notc\" as text; fn view() -> InlineContent { [#text.value()] }"
            .to_vec(),
    );
    files.insert(
        "parts/text.notc",
        b"fn value() -> InlineContent { [imported result] }".to_vec(),
    );
    files.insert("lib.notc", b"invalid automatically loaded code".to_vec());
    files
}

#[test]
fn both_frontends_lower_imports_separately_and_load_the_same_module_graph() {
    let source = "#import \"./views.notc\" as ui;\n\n#ui.view()";
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let lowered = pipeline
            .lower(path, source, FrontendOptions::default())
            .unwrap();
        assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
        assert_eq!(lowered.imports.len(), 1);
        let import = &lowered.imports[0];
        assert_eq!(import.path, "./views.notc");
        assert_eq!(import.alias, "ui");
        assert_eq!(
            &source[usize::from(import.span.start())..usize::from(import.span.end())],
            "#import \"./views.notc\" as ui;"
        );
        let mut vault = Vault::new(files());
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
        assert!(output.rendered.html.contains("imported result"));
        assert_eq!(output.analysis.root().children.len(), 1);
        assert_eq!(output.analysis.root().children[0].ctor, Ctor::Paragraph);
        assert_eq!(
            vault.environment_for(path, source).unwrap().modules().len(),
            2
        );
    }
}

#[test]
fn imports_are_global_to_the_file_but_do_not_create_content_or_empty_paragraphs() {
    for path in ["doc.not", "doc.md"] {
        let mut vault = Vault::new(files());
        let source = "#import \"./views.notc\" as ui;\n";
        let a = vault.analyze(path, source).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        assert!(a.root().children.is_empty());
        let source = "#ui.view()\n\n#import \"./views.notc\" as ui;\n";
        let output = vault.render_html(path, source, Default::default()).unwrap();
        assert!(
            output.analysis.diagnostics().is_empty(),
            "{:?}",
            output.analysis.diagnostics()
        );
        assert!(output.rendered.html.contains("imported result"));
    }
}

#[test]
fn changing_source_imports_preserves_the_context_of_each_existing_analysis() {
    let mut resources = files();
    resources.insert(
        "first.notc",
        b"fn f() -> InlineContent { [first] }".to_vec(),
    );
    resources.insert(
        "second.notc",
        b"fn f() -> InlineContent { [second] }".to_vec(),
    );
    let mut vault = Vault::new(resources);
    let a = vault
        .analyze(
            "doc.not",
            "#import \"first.notc\" as selected;\n#selected.f()",
        )
        .unwrap();
    let b = vault
        .analyze(
            "doc.not",
            "#import \"second.notc\" as selected;\n#selected.f()",
        )
        .unwrap();
    for (analysis, expected) in [(&a, "first"), (&b, "second"), (&a, "first")] {
        let result = vault.evaluate("doc.not", analysis).unwrap();
        assert!(
            result
                .root
                .find(|i| i.fields.get("text") == Some(&Value::Str(expected.into())))
                .is_some()
        );
    }
}

#[test]
fn imports_in_literals_and_nested_content_are_diagnosed_without_loading_them() {
    let pipeline = Pipeline::default();
    for path in ["literal.not", "literal.md"] {
        let (lowered, _) = pipeline
            .analyze_content_literal(
                path,
                "[\n#import \"missing.notc\" as missing;\n]",
                &notist::Context::default(),
            )
            .unwrap();
        assert!(
            lowered
                .diagnostics
                .iter()
                .any(|d| d.message.contains("import")),
            "{:?}",
            lowered.diagnostics
        );
        let output = pipeline
            .lower(
                path,
                "#node[\n#import \"missing.notc\" as missing;\n]",
                FrontendOptions::default(),
            )
            .unwrap();
        assert!(output.imports.is_empty());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| d.message.contains("import")),
            "{:?}",
            output.diagnostics
        );
    }
}

#[test]
fn import_like_raw_text_comments_and_escaped_markers_do_not_load_modules() {
    for (path, source) in [
        (
            "doc.not",
            "`#import \"missing.notc\" as missing;`\n\n\\#import \"missing.notc\" as missing;\n\n// #import \"missing.notc\" as missing;",
        ),
        (
            "doc.md",
            "`#import \"missing.notc\" as missing;`\n\n\\#import \"missing.notc\" as missing;\n\n```not\n#import \"missing.notc\" as missing;\n```",
        ),
    ] {
        let output = Pipeline::default()
            .lower(path, source, FrontendOptions::default())
            .unwrap();
        assert!(output.imports.is_empty());
    }
}

#[test]
fn import_parser_recovers_for_every_unicode_prefix() {
    let source = "#import \"./视图.notc\" as 页面;";
    for end in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
        let source = format!("{}\n\nFollowing content", &source[..end]);
        for path in ["doc.not", "doc.md"] {
            let output = Pipeline::default()
                .lower(path, &source, FrontendOptions::default())
                .unwrap();
            assert!(
                output
                    .diagnostics
                    .iter()
                    .all(|d| usize::from(d.span.end()) <= source.len())
            );
        }
        let parsed = notist::syntax::markup::parse_document(&source);
        assert_eq!(parsed.syntax().text().to_string(), source);
    }
}

#[test]
fn malformed_import_tails_recover_losslessly_at_frontend_token_boundaries() {
    for source in [
        "#import \"views.notc\";trailing\n\nFollowing",
        "#node[ #import \"views.notc\";trailing ]\n\nFollowing",
        "#import /* multiline\ncomment */ \"views.notc\";\n\nFollowing",
        "#import \"views.notc\" as ui;/* comment\ncontinues */\n\nFollowing",
    ] {
        let parsed = notist::syntax::markup::parse_document(source);
        assert_eq!(parsed.syntax().text().to_string(), source);
        for path in ["doc.not", "doc.md"] {
            let result = Pipeline::default()
                .lower(path, source, FrontendOptions::default())
                .unwrap();
            assert!(!result.diagnostics.is_empty(), "{path}: {source}");
        }
    }
}

#[test]
fn import_comments_and_invalid_declarations_use_the_same_rules_in_both_frontends() {
    for path in ["doc.not", "doc.md"] {
        let mut vault = Vault::new(files());
        let source = "#import /* path */ \"views.notc\"; // trailing comment\n#views.view()";
        let output = vault.render_html(path, source, Default::default()).unwrap();
        assert!(
            output.analysis.diagnostics().is_empty(),
            "{path}: {:?}",
            output.analysis.diagnostics()
        );
        assert!(output.rendered.html.contains("imported result"));

        for source in [
            "#import \"missing.notc\" as missing",
            "#import \"missing.notc\";trailing",
            "#import \"missing.notc\" as notist;",
        ] {
            let analysis = vault.analyze(path, source).unwrap();
            assert!(!analysis.diagnostics().is_empty(), "{path}: {source}");
        }

        let source = "#import \"views.notc\" as ui;\n#import \"missing.notc\" as ui;\n#ui.view()";
        let output = vault.render_html(path, source, Default::default()).unwrap();
        assert!(!output.analysis.diagnostics().is_empty());
        assert!(output.rendered.html.contains("imported result"));
    }
}
