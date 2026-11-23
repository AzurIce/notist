use notc::{
    AttrValue, BlockKind, DataHost, JsonValue, ProjectDiagnosticKind, TypeError, Value,
    check_project,
};
use std::collections::HashMap;

struct MemoryDataHost {
    json_files: HashMap<String, JsonValue>,
    text_files: HashMap<String, String>,
}

impl MemoryDataHost {
    fn new(entries: &[(&str, &str)]) -> Self {
        let mut json_files = HashMap::new();
        for &(path, raw) in entries {
            json_files.insert(
                path.to_owned(),
                JsonValue::parse(raw).expect("valid test json"),
            );
        }
        Self {
            json_files,
            text_files: HashMap::new(),
        }
    }

    fn with_texts(mut self, entries: &[(&str, &str)]) -> Self {
        for &(path, raw) in entries {
            self.text_files.insert(path.to_owned(), raw.to_owned());
        }
        self
    }
}

impl DataHost for MemoryDataHost {
    fn read_text(&self, from_doc: &str, rel_path: &str) -> Result<String, String> {
        let resolved = notc::resolve_import_path(from_doc, rel_path)
            .map_err(|_| format!("invalid path `{rel_path}` from `{from_doc}`"))?;
        self.text_files
            .get(&resolved)
            .cloned()
            .ok_or_else(|| format!("missing text `{resolved}`"))
    }

    fn read_json(&self, from_doc: &str, rel_path: &str) -> Result<JsonValue, String> {
        let resolved = notc::resolve_import_path(from_doc, rel_path)
            .map_err(|_| format!("invalid path `{rel_path}` from `{from_doc}`"))?;
        self.json_files
            .get(&resolved)
            .cloned()
            .ok_or_else(|| format!("missing json `{resolved}`"))
    }
}

#[test]
fn parses_json_including_escapes_surrogates_and_nested_containers() {
    let raw = r#"
    {
        "count": 51225,
        "ratio": -1.5e2,
        "active": true,
        "missing": null,
        "label": "火之碎晶 \u0041 \uD83D\uDE00 \"ok\"\nline2",
        "items": [
            ["1", "金币", 63],
            ["2", "火之碎晶", 59]
        ]
    }
    "#;
    let json = JsonValue::parse(raw).expect("parses json");
    assert_eq!(json.get("count").as_int(0), 51225);
    assert!(json.get("active").as_bool(false));
    assert!(json.get("missing").is_null());
    assert!(json.get("nonexistent").is_null());
    assert_eq!(
        json.get("label").as_string("").as_ref(),
        "火之碎晶 A 😀 \"ok\"\nline2"
    );
    let items = json.get("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items.at(1).at(1).as_string("").as_ref(), "火之碎晶");
    assert_eq!(items.at(1).at(2).as_int(0), 59);
    assert!(items.at(99).is_null());
    assert!(items.at(-1).is_null());

    assert!(JsonValue::parse("{bad").is_err());
    assert!(JsonValue::parse("[1, 2] trailing").is_err());
    assert!(JsonValue::parse("\"\\uD800\"").is_err());
}

#[test]
fn list_literals_for_expressions_to_string_and_dynamic_containers() {
    let views = r#"
fn catalog(source: String) -> Block {
    let page = read_json(source);
    let count = page.get("count").as_int(0);
    let version = page.get("version").as_string("unknown");
    let records = page.get("items").as_list();
    let rows = for record in records {
        let id = record.at(0).as_string("");
        let name = record.at(1).as_string("");
        let cat = record.at(2).as_string("");
        let target = "/items/" + id + ".html";
        (
            text(id).with_attr("class", "numeric"),
            link(target, text(name)),
            text(cat).with_attr("class", "numeric"),
        )
    };
    let count_text = count.to_string() + " items · " + version + " · ok=" + true.to_string();
    [
        = Catalog

        #para(text(count_text))

        @(id: "catalog")#div[
            @(class: "table-scroll")#div[
                #table((text("ID"), text("Name"), text("Category")), rows)
            ]
        ]

        @(id: "notes")#section[
            == Notes

            #blocks(for r in records { para(text(r.at(1).as_string(""))) })
        ]
    ]
}
"#;
    let doc = r#"
#import "../modules/views.notc" as views
#views.catalog("./data/page-1.json")
"#;

    let host = MemoryDataHost::new(&[(
        "content/data/page-1.json",
        r#"{"count": 2, "version": "7.1", "items": [["1", "Gil", "63"], ["2", "Fire Shard", "59"]]}"#,
    )]);

    let project = check_project(&[("modules/views.notc", views), ("content/index.not", doc)])
        .expect("data-driven project checks cleanly");
    let block = project
        .eval_document_with_host("content/index.not", &host)
        .expect("evaluates with host");

    let plain = block.plain_text();
    assert!(plain.contains("2 items · 7.1 · ok=true"), "plain: {plain}");
    assert!(plain.contains("Gil"), "plain: {plain}");
    assert!(plain.contains("Fire Shard"), "plain: {plain}");

    // Verify sectionize preserves explicit Div and SectionBox shells
    let sectioned = block.sectionize();
    let BlockKind::Section { body, .. } = &sectioned.node().kind else {
        panic!("expected top-level section around h1, got {sectioned}");
    };
    let BlockKind::Fragment(children) = &body.node().kind else {
        panic!("expected section body fragment, got {body}");
    };
    assert!(matches!(children[1].node().kind, BlockKind::Div(_)));
    assert_eq!(
        children[1].node().attrs.get("id"),
        Some(&AttrValue::String("catalog".into()))
    );
    assert!(matches!(children[2].node().kind, BlockKind::SectionBox(_)));
    assert_eq!(
        children[2].node().attrs.get("id"),
        Some(&AttrValue::String("notes".into()))
    );
}

#[test]
fn list_concat_equality_len_and_contextual_empty_list() {
    let src = r#"
fn check_lists() -> Bool {
    let a = (1, 2);
    let b = (3,);
    let c = a + b;
    let doubled = for x in c { x * 2 };
    let empty_table = table((), ());
    let ok_len = doubled.len() == 3;
    let ok_eq = doubled == (2, 4, 6);
    let ok_table = empty_table.plain_text() == "";
    ok_len && ok_eq && ok_table
}
"#;
    let project = check_project(&[("main.notc", src)]).expect("checks");
    assert!(matches!(
        project.run("main.notc", "check_lists").expect("runs"),
        Value::Bool(true)
    ));
}

#[test]
fn statically_rejects_bad_list_and_for_types() {
    let bad_empty_iter = "fn f() -> List<Int> { for x in () { 1 } }";
    let errs = check_project(&[("a.notc", bad_empty_iter)]).expect_err("unit () is not iterable");
    assert!(matches!(
        &errs[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::NotIterable(_),
            ..
        }
    ));

    let bad_iter = "fn f() -> List<Int> { for x in 42 { x } }";
    let errs = check_project(&[("a.notc", bad_iter)]).expect_err("int is not iterable");
    assert!(matches!(
        &errs[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::NotIterable(_),
            ..
        }
    ));

    let bad_mixed = "fn f() -> List<Int> { (1, \"two\") }";
    let errs = check_project(&[("a.notc", bad_mixed)]).expect_err("mixed list elements fail");
    assert!(matches!(
        &errs[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::Mismatch { .. },
            ..
        }
    ));
}

#[test]
fn standalone_document_check_reuses_shared_lowered_functions_and_splits_eval() {
    let shared_src = r#"
fn load_item(source: String) -> Block {
    let data = read_json(source);
    para(text(data.get("name").as_string("missing")))
}

fn wrap<C: Type>(content: C) -> C
where C == Inline || C == Block
{
    match C {
        Inline => strong(content),
        Block => callout("note", content),
        _ => compile_error("unreachable"),
    }
}
"#;
    let project = check_project(&[("lib/shared.notc", shared_src)]).expect("shared project checks");

    let doc1 = notc::parse_document(
        "#import \"../lib/shared.notc\" as shared\n#shared.load_item(\"./item.json\")\n",
    );
    assert!(doc1.diagnostics.is_empty());
    // Pure static check: no DataHost needed, and zero extra functions duplicated!
    let checked1 = project
        .check_standalone_document("content/a.not", &doc1.program.root)
        .expect("static check succeeds without DataHost");
    assert_eq!(checked1.path(), "content/a.not");
    assert_eq!(checked1.extra_function_count(), 0);

    // Evaluation happens separately and hits DataHost only now.
    let err = project
        .eval_checked_document(&checked1, &notc::NoDataHost)
        .expect_err("evaluating without JSON fails at runtime");
    assert!(matches!(err, notc::ExecutionError::HostError(_)));

    let host = MemoryDataHost::new(&[("content/item.json", r#"{"name": "Potion"}"#)]);
    let block1 = project
        .eval_checked_document(&checked1, &host)
        .expect("evaluating with JSON succeeds");
    assert_eq!(block1.plain_text(), "Potion");

    // Document calling a shared generic function lowers only that new specialization.
    let doc2 = notc::parse_document(
        "#import \"../lib/shared.notc\" as shared\n#shared.wrap[\nHello\n]\n",
    );
    assert!(doc2.diagnostics.is_empty());
    let checked2 = project
        .check_standalone_document("content/b.not", &doc2.program.root)
        .expect("generic document checks");
    assert_eq!(checked2.extra_function_count(), 1);
    let block2 = project
        .eval_checked_document(&checked2, &notc::NoDataHost)
        .expect("generic document evaluates");
    assert_eq!(block2.plain_text(), "Hello");
}

#[test]
fn read_text_and_panic_builtins_work_with_never_coercion() {
    let lib = r#"
fn checked_text(path: String, must_exist: Bool) -> Block {
    let raw = if must_exist {
        read_text(path)
    } else {
        panic("refused to load: " + path)
    };
    para(text(raw))
}
"#;
    let doc_ok = r#"
#import "./lib.notc" as lib
#lib.checked_text("./.data/greeting.txt", true)
"#;
    let doc_panic = r#"
#import "./lib.notc" as lib
#lib.checked_text("./.data/greeting.txt", false)
"#;
    let project = check_project(&[
        ("lib.notc", lib),
        ("ok.not", doc_ok),
        ("fail.not", doc_panic),
    ])
    .expect("checks with Never coercion");
    let host = MemoryDataHost::new(&[]).with_texts(&[(".data/greeting.txt", "Hello from text!")]);

    let ok = project
        .eval_document_with_host("ok.not", &host)
        .expect("read_text evaluates");
    assert_eq!(ok.plain_text(), "Hello from text!");

    let err = project
        .eval_document_with_host("fail.not", &host)
        .expect_err("panic aborts evaluation");
    assert_eq!(
        err,
        notc::ExecutionError::Panic("refused to load: ./.data/greeting.txt".into())
    );
}
