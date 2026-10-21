use notist_syntax::code::{ast, parse_module};
use notist_syntax::tree::SyntaxKind;

fn module(src: &str) -> ast::Module {
    let parsed = parse_module(src, &notist_syntax::markup::MarkupContentParser);
    assert_eq!(parsed.syntax().text().to_string(), src);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    ast::Module::cast(parsed.syntax()).unwrap()
}

fn function_names(module: &ast::Module) -> Vec<String> {
    module
        .functions()
        .filter_map(|function| function.name())
        .map(|name| name.text().to_string())
        .collect()
}

#[test]
fn signatures_preserve_names_types_defaults_and_mounts() {
    let src = "// API\r\nfn diagram(source: String, theme: String = \"default\") -> Content;\r\n\
        fn panel(title?: String)[children: Content] -> Content;\n\
        fn badge(label: String)[children: InlineContent] -> InlineContent;";
    let module = module(src);
    assert_eq!(function_names(&module), ["diagram", "panel", "badge"]);
    let functions: Vec<_> = module.functions().collect();
    let params: Vec<_> = functions[0].parameters().collect();
    assert_eq!(params[0].name().unwrap().text(), "source");
    assert_eq!(params[0].ty().unwrap().name().unwrap().text(), "String");
    assert!(!params[0].is_optional());
    assert!(params[0].default_value().is_none());
    assert_eq!(
        params[1]
            .default_value()
            .unwrap()
            .value()
            .unwrap()
            .into_token()
            .unwrap()
            .text(),
        "\"default\""
    );
    assert!(functions[0].children_decl().is_none());
    assert!(functions[1].parameters().next().unwrap().is_optional());
    assert_eq!(
        functions[2]
            .children_decl()
            .unwrap()
            .ty()
            .unwrap()
            .name()
            .unwrap()
            .text(),
        "InlineContent"
    );
    assert_eq!(
        functions[2].return_type().unwrap().name().unwrap().text(),
        "InlineContent"
    );
    for function in functions {
        let span = function.range();
        let slice = &src[u32::from(span.start()) as usize..u32::from(span.end()) as usize];
        assert!(slice.starts_with("fn "));
        assert!(slice.ends_with(';'));
    }
}

#[test]
fn generic_types_and_shared_collection_literals() {
    let src = "fn config(\n items: Array<Array<Bool>> = ((true, false),),\n options: Dict = (label: r#\"fn;[]\"#, n: -2, unit: (), empty: (,)),\n) -> BlockContent;";
    let module = module(src);
    let function = module.functions().next().unwrap();
    let params: Vec<_> = function.parameters().collect();
    let outer = params[0].ty().unwrap();
    assert_eq!(outer.name().unwrap().text(), "Array");
    let inner = outer.arguments().next().unwrap();
    assert_eq!(
        inner.arguments().next().unwrap().name().unwrap().text(),
        "Bool"
    );
    assert_eq!(
        params[0].default_value().unwrap().value().unwrap().kind(),
        SyntaxKind::Array
    );
    assert_eq!(
        params[1].default_value().unwrap().value().unwrap().kind(),
        SyntaxKind::Dict
    );
}

#[test]
fn comments_and_multiline_raw_strings_do_not_end_declarations() {
    let src = "/* outer /* nested */ end */\nfn/*c*/foo(\n x/*c*/: String = r#\"\"\"\nfn fake();\n[]\n\"\"\"#,\n n: Int=-1,\n) [/*c*/children:BlockContent]->BlockContent;\n";
    assert_eq!(function_names(&module(src)), ["foo"]);
}

#[test]
fn syntax_keeps_unknown_types_for_semantic_validation() {
    let module = module("fn custom(value: some.Unknown<Other> = 1) -> FutureType;");
    let function = module.functions().next().unwrap();
    assert_eq!(
        function
            .parameters()
            .next()
            .unwrap()
            .ty()
            .unwrap()
            .name()
            .unwrap()
            .text(),
        "some.Unknown"
    );
}

#[test]
fn recovery_keeps_following_declarations_and_source() {
    let broken = [
        "fn broken(x: String",
        "fn broken",
        "fn broken(x String)",
        "fn broken(x: String =)",
        "fn broken(x: Dict = (key:)",
        "fn broken(x: Dict = (key:",
        "fn broken(x: Array<Bool)",
        "fn broken() [body: BlockContent]",
        "fn broken()->BlockContent",
        "import something;",
        "fn broken(x: String = [markup]) -> BlockContent;",
        "fn broken(x: String = \"unterminated\n",
        "fn broken()->BlockContent { body };",
    ];
    for prefix in broken {
        let src = format!("{prefix}\nfn good() -> BlockContent;\nfn last() -> InlineContent;");
        let parsed = parse_module(&src, &notist_syntax::markup::MarkupContentParser);
        assert_eq!(parsed.syntax().text().to_string(), src, "{prefix}");
        assert!(!parsed.diagnostics.is_empty(), "{prefix}");
        let names = function_names(&ast::Module::cast(parsed.syntax()).unwrap());
        assert!(
            names.ends_with(&["good".to_string(), "last".to_string()]),
            "{prefix}: {names:?}"
        );
        for diagnostic in parsed.diagnostics {
            assert!(u32::from(diagnostic.span.end()) as usize <= src.len());
        }
    }
}

#[test]
fn optional_and_default_are_mutually_exclusive() {
    let parsed = parse_module(
        "fn f(x?: Int = 1) -> BlockContent;",
        &notist_syntax::markup::MarkupContentParser,
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("optional parameter"))
    );
    let module = ast::Module::cast(parsed.syntax()).unwrap();
    let parameter = module
        .functions()
        .next()
        .unwrap()
        .parameters()
        .next()
        .unwrap();
    assert!(parameter.is_optional());
    assert!(parameter.default_value().is_some());
}

#[test]
fn trivia_and_empty_modules_are_lossless() {
    for src in [
        "",
        " \r\n// comment\n/* comment */\t",
        "fn f()->BlockContent;",
    ] {
        module(src);
    }
    let parsed = parse_module(
        "/* outer /* inner */",
        &notist_syntax::markup::MarkupContentParser,
    );
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].message, "unclosed block comment");
}

#[test]
fn every_truncated_signature_recovers_at_the_next_function() {
    let signature = "fn broken(x: Array<Array<String>> = (1, (key: 2,)), title?: String)[children: BlockContent] -> BlockContent;";
    for end in 0..=signature.len() {
        let src = format!("{}\nfn good() -> BlockContent;", &signature[..end]);
        let parsed = parse_module(&src, &notist_syntax::markup::MarkupContentParser);
        assert_eq!(parsed.syntax().text().to_string(), src);
        let module = ast::Module::cast(parsed.syntax()).unwrap();
        assert_eq!(
            function_names(&module).last().map(String::as_str),
            Some("good"),
            "prefix: {}",
            &signature[..end]
        );
    }
}

#[test]
fn unterminated_inline_strings_stop_at_all_line_endings() {
    for newline in ["\n", "\r\n", "\r"] {
        for tail in ["", "\\"] {
            let src = format!("fn bad(x:String=\"broken{tail}{newline}fn good()->BlockContent;");
            let parsed = parse_module(&src, &notist_syntax::markup::MarkupContentParser);
            assert_eq!(parsed.syntax().text().to_string(), src);
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|d| d.message == "unclosed string")
            );
            assert_eq!(
                function_names(&ast::Module::cast(parsed.syntax()).unwrap())
                    .last()
                    .map(String::as_str),
                Some("good")
            );
        }
    }
}

#[test]
fn an_escaped_triple_quote_does_not_close_a_multiline_string() {
    let parsed = parse_module(
        "fn bad(x: String = \"\"\"\nescaped \\\"\"\"",
        &notist_syntax::markup::MarkupContentParser,
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message == "unclosed string")
    );
}

#[test]
fn excessive_type_nesting_is_diagnosed_and_recovers() {
    let src = format!(
        "fn bad(x: {}Bool{}) -> BlockContent; fn good()->BlockContent;",
        "Array<".repeat(130),
        ">".repeat(130)
    );
    let parsed = parse_module(&src, &notist_syntax::markup::MarkupContentParser);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nesting limit"))
    );
    assert_eq!(parsed.syntax().text().to_string(), src);
    assert_eq!(
        function_names(&ast::Module::cast(parsed.syntax()).unwrap())
            .last()
            .map(String::as_str),
        Some("good")
    );
}

#[test]
fn import_declarations_preserve_paths_aliases_and_unicode() {
    let module = module(
        "import \"./视图.notc\" as 页面;\nimport r#\"other/lib.notc\"#;\nfn f() -> Int { 页面.value() }",
    );
    let imports: Vec<_> = module.imports().collect();
    assert_eq!(imports[0].source().unwrap().text(), "\"./视图.notc\"");
    assert_eq!(imports[0].alias().unwrap().text(), "页面");
    assert!(imports[1].alias().is_none());
    let member = module
        .functions()
        .next()
        .unwrap()
        .body()
        .unwrap()
        .descendants()
        .find(|n| n.kind() == SyntaxKind::MemberExpr)
        .unwrap();
    assert_eq!(member.text().to_string(), "页面.value");
}

#[test]
fn truncated_imports_and_functions_recover_at_either_declaration_kind() {
    for source in [
        "import \"./视图.notc\" as 页面;",
        "fn bad(x: Dict = (key: value)) -> Int { a.b() }",
    ] {
        for end in (0..=source.len()).filter(|&i| source.is_char_boundary(i)) {
            let prefix = &source[..end];
            let input = format!(
                "{prefix}\nimport \"./good.notc\" as good;\nfn last() -> Int {{ good.value() }}"
            );
            let parsed = parse_module(&input, &notist_syntax::markup::MarkupContentParser);
            assert_eq!(parsed.syntax().text().to_string(), input);
            let module = ast::Module::cast(parsed.syntax()).unwrap();
            assert!(
                module
                    .imports()
                    .any(|i| i.alias().is_some_and(|a| a.text() == "good")),
                "prefix: {prefix}"
            );
            assert_eq!(
                function_names(&module).last().map(String::as_str),
                Some("last"),
                "prefix: {prefix}"
            );
        }
    }
}
