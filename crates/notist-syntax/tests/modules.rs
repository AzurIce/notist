use notist_syntax::syntax::SyntaxKind;
use notist_syntax::{ast, parse_document, parse_module};

fn module(src: &str) -> ast::Module {
    let parsed = parse_module(src);
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
    assert_eq!(params[0].ty().unwrap().path().unwrap().text(), "String");
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
            .path()
            .unwrap()
            .text(),
        "InlineContent"
    );
    assert_eq!(
        functions[2].return_type().unwrap().path().unwrap().text(),
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
    let src = "fn config(\n items: Array<Array<Bool>> = ((true, false),),\n options: Dict = (label: r#\"fn;[]\"#, n: -2, unit: (), empty: (,)),\n) -> Content;";
    let module = module(src);
    let function = module.functions().next().unwrap();
    let params: Vec<_> = function.parameters().collect();
    let outer = params[0].ty().unwrap();
    assert_eq!(outer.path().unwrap().text(), "Array");
    let inner = outer.arguments().next().unwrap();
    assert_eq!(
        inner.arguments().next().unwrap().path().unwrap().text(),
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
    let src = "/* outer /* nested */ end */\nfn/*c*/foo(\n x/*c*/: String = r#\"\"\"\nfn fake();\n[]\n\"\"\"#,\n n: Int=-1,\n) [/*c*/children:Content]->Content;\n";
    assert_eq!(function_names(&module(src)), ["foo"]);
}

#[test]
fn syntax_keeps_unknown_types_for_semantic_validation() {
    let module = module("fn custom(value: some::Unknown<Other> = 1) -> FutureType;");
    let function = module.functions().next().unwrap();
    assert_eq!(
        function
            .parameters()
            .next()
            .unwrap()
            .ty()
            .unwrap()
            .path()
            .unwrap()
            .text(),
        "some::Unknown"
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
        "fn broken() [body: Content]",
        "fn broken()->Content",
        "import something;",
        "fn broken(x: String = [markup]) -> Content;",
        "fn broken(x: String = \"unterminated\n",
        "fn broken()->Content { body };",
    ];
    for prefix in broken {
        let src = format!("{prefix}\nfn good() -> Content;\nfn last() -> InlineContent;");
        let parsed = parse_module(&src);
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
    let parsed = parse_module("fn f(x?: Int = 1) -> Content;");
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
    for src in ["", " \r\n// comment\n/* comment */\t", "fn f()->Content;"] {
        module(src);
    }
    let parsed = parse_module("/* outer /* inner */");
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].message, "unclosed block comment");
}

#[test]
fn document_and_module_use_distinct_roots_and_dispatch() {
    let src = "fn foo() -> Content;";
    let document = parse_document(src);
    let code = parse_module(src);
    assert!(ast::Document::cast(document.syntax()).is_some());
    assert!(ast::Module::cast(document.syntax()).is_none());
    assert!(ast::Module::cast(code.syntax()).is_some());
    assert!(ast::Document::cast(code.syntax()).is_none());
}

#[test]
fn every_truncated_signature_recovers_at_the_next_function() {
    let signature = "fn broken(x: Array<Array<String>> = (1, (key: 2,)), title?: String)[children: Content] -> Content;";
    for end in 0..=signature.len() {
        let src = format!("{}\nfn good() -> Content;", &signature[..end]);
        let parsed = parse_module(&src);
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
            let src = format!("fn bad(x:String=\"broken{tail}{newline}fn good()->Content;");
            let parsed = parse_module(&src);
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
fn excessive_type_nesting_is_diagnosed_and_recovers() {
    let src = format!(
        "fn bad(x: {}Bool{}) -> Content; fn good()->Content;",
        "Array<".repeat(130),
        ">".repeat(130)
    );
    let parsed = parse_module(&src);
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
