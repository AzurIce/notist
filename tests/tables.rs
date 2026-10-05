use notist::builtins::{Accepts, Level};
use notist::syntax::ast::{Table, TableAlignment};
use notist::syntax::syntax::SyntaxKind;
use notist::{Ctor, Item, Pipeline, TextRange, Value};

fn analyze(path: &str, src: &str) -> Item {
    if path.ends_with(".not") {
        let parse = notist::syntax::parser::parse_document(src);
        assert_eq!(parse.syntax().to_string(), src, "{src:?}");
        assert!(
            parse.diagnostics.is_empty(),
            "{src:?}: {:?}",
            parse.diagnostics
        );
    }
    let analysis = Pipeline::default()
        .analyze(path, src, notist::builtins::registry())
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{src:?}: {:?}",
        analysis.diagnostics()
    );
    analysis.into_parts().0
}

fn normalized(mut item: Item) -> Item {
    item.span = TextRange::empty(0.into());
    item.children = item.children.into_iter().map(normalized).collect();
    item
}

fn text(item: &Item) -> String {
    item.descendants()
        .filter_map(|node| {
            if node.ctor == Ctor::Text {
                if let Some(Value::Str(text)) = node.fields.get("text") {
                    return Some(text.as_str());
                }
            }
            None
        })
        .collect()
}

#[test]
fn notist_markdown_and_content_functions_share_the_same_table_ir() {
    let not = "| 名称 | 数量 |\n| :--- | ---: |\n| *苹果* | $3$ |\n";
    let md = "| 名称 | 数量 |\n| :--- | ---: |\n| **苹果** | $3$ |\n";
    let calls = "#table(align: (\"left\", \"right\"))[\n#row(header: true)[\n#cell[名称]\n#cell[数量]\n]\n#row(header: false)[\n#cell[*苹果*]\n#cell[$3$]\n]\n]";
    let not = analyze("test.not", not);
    assert_eq!(normalized(not.clone()), normalized(analyze("test.md", md)));
    assert_eq!(
        normalized(not.clone()),
        normalized(analyze("test.not", calls))
    );
    let table = &not.children[0];
    assert_eq!(table.ctor, Ctor::Table);
    assert_eq!(table.ctor.accepts(), Some(Accepts::Rows));
    assert_eq!(table.ctor.level(), Some(Level::Block));
    assert_eq!(table.children[0].ctor.accepts(), Some(Accepts::Cells));
    assert_eq!(
        table.children[0].children[0].ctor.accepts(),
        Some(Accepts::Content)
    );
    assert_eq!(
        table.children[0].fields.get("header"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        table.children[1].fields.get("header"),
        Some(&Value::Bool(false))
    );
    assert!(table.children.iter().all(|row| row.ctor == Ctor::TableRow));
    assert!(
        table
            .children
            .iter()
            .flat_map(|row| &row.children)
            .all(|cell| cell.ctor == Ctor::TableCell)
    );
    assert_eq!(
        table.children[1].children[0].children[0].ctor,
        Ctor::Paragraph
    );
    assert!(
        table.children[1].children[0]
            .find(|node| node.ctor == Ctor::Strong)
            .is_some()
    );
}

#[test]
fn table_ast_exposes_rows_cells_header_and_all_alignments_losslessly() {
    let src = "| a | b | c | d |\r\n| --- | :--- | :---: | ---: |\r\n| 1 | 2 | 3 | 4 |";
    let parse = notist::syntax::parser::parse_document(src);
    assert!(parse.diagnostics.is_empty());
    assert_eq!(parse.syntax().to_string(), src);
    let table = Table::cast(parse.syntax().children().next().unwrap()).unwrap();
    assert_eq!(
        table.alignments(),
        [
            TableAlignment::None,
            TableAlignment::Left,
            TableAlignment::Center,
            TableAlignment::Right
        ]
    );
    let rows: Vec<_> = table.rows().collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_header());
    assert!(!rows[1].is_header());
    assert_eq!(rows[0].cells().count(), 4);
    let cell = rows[0].cells().next().unwrap();
    assert_eq!(
        cell.content()
            .map(|e| e.to_string())
            .collect::<String>()
            .trim(),
        "a"
    );
    assert_eq!(usize::from(table.range().end()), src.len());
    let json = notist::cst_json::analyze_json(src);
    assert!(json.contains("\"kind\":\"Table\""));
    assert!(json.contains("\"kind\":\"TableDelimiter\""));
    assert_eq!(
        normalized(analyze("test.not", src)),
        normalized(analyze("test.md", src))
    );
}

#[test]
fn outer_pipes_are_optional_and_header_only_and_single_column_tables_work() {
    for src in [
        "a | b\n--- | :---:\n1 | 2",
        "| a | b\n--- | ---: |\n| 1 | 2",
        "| a |\n| --- |",
        "a\n| --- |\nvalue",
        "| a | b |\n| - | :-: |",
        "| | |\n| --- | --- |\n| | |",
    ] {
        let not = analyze("test.not", src);
        assert_eq!(not.children[0].ctor, Ctor::Table, "{src:?}");
        assert_eq!(
            normalized(not),
            normalized(analyze("test.md", src)),
            "{src:?}"
        );
    }
}

#[test]
fn body_rows_pad_missing_cells_and_ignore_excess_cells() {
    let src = "| a | b |\n| --- | --- |\n| short |\n| x | y | extra |\n";
    let not = analyze("test.not", src);
    let table = &not.children[0];
    assert!(table.children.iter().all(|row| row.children.len() == 2));
    assert!(table.children[1].children[1].children.is_empty());
    assert_eq!(text(table), "abshortxy");
    let md = analyze("test.md", src);
    for row in &md.children[0].children {
        assert!(
            row.descendants()
                .all(|node| row.span.contains_range(node.span))
        );
    }
    assert!(md.children[0].children[1].children[1].span.is_empty());
    assert_eq!(normalized(not), normalized(md));
}

#[test]
fn table_pipes_are_escaped_in_markup_and_owned_by_opaque_payloads() {
    let not = "| value |\n| --- |\n| a\\|b *c\\|d* `e|f` $g|h$ |";
    let md = "| value |\n| --- |\n| a\\|b **c\\|d** `e\\|f` $g\\|h$ |";
    let not = analyze("test.not", not);
    assert_eq!(normalized(not.clone()), normalized(analyze("test.md", md)));
    let cell = &not.children[0].children[1].children[0];
    assert_eq!(text(cell), "a|b c|d  ");
    let raw = cell.find(|n| n.ctor == Ctor::RawInline).unwrap();
    assert_eq!(raw.fields.get("text"), Some(&Value::Str("e|f".into())));
    let math = cell.find(|n| n.ctor == Ctor::Math).unwrap();
    assert_eq!(math.fields.get("text"), Some(&Value::Str("g|h".into())));
    let calls = analyze(
        "test.not",
        "| value |\n| --- |\n| #raw(\"a\\\\|b\") #math(\"c\\\\|d\") |",
    );
    assert_eq!(
        calls
            .find(|n| n.ctor == Ctor::RawInline)
            .unwrap()
            .fields
            .get("text"),
        Some(&Value::Str("a\\|b".into()))
    );
    assert_eq!(
        calls
            .find(|n| n.ctor == Ctor::Math)
            .unwrap()
            .fields
            .get("text"),
        Some(&Value::Str("c\\|d".into()))
    );
}

#[test]
fn block_math_splits_cell_content_and_preserves_frontend_pipe_rules() {
    let source = r#"| formula | tail |
| --- | --- |
| before $ x|y $ after | end |
"#;
    for (path, source) in [
        ("test.not", source.to_owned()),
        ("test.md", source.replace("x|y", r"x\|y")),
    ] {
        let root = analyze(path, &source);
        let cell = &root.children[0].children[1].children[0];
        assert_eq!(
            cell.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
            [&Ctor::Paragraph, &Ctor::Math, &Ctor::Paragraph]
        );
        assert_eq!(
            cell.children[1].fields.get("text"),
            Some(&Value::Str("x|y".into()))
        );
        assert_eq!(text(&root.children[0].children[1].children[1]), "end");
    }
    let root = analyze(
        "test.not",
        r##"| formula |
| --- |
| #[
$ x|y $
] #math(r#"a\|b"#, block: true) |
"##,
    );
    let equations = root
        .descendants()
        .filter(|n| n.ctor == Ctor::Math)
        .collect::<Vec<_>>();
    assert_eq!(equations.len(), 2);
    assert_eq!(
        equations[0].fields.get("text"),
        Some(&Value::Str("x|y".into()))
    );
    assert_eq!(
        equations[1].fields.get("text"),
        Some(&Value::Str("a\\|b".into()))
    );
}

#[test]
fn multiline_math_keeps_the_example_in_one_cell_and_preserves_source_spans() {
    for newline in ["\n", "\r\n", "\r"] {
        let payload = [
            r"\f\relax{x} = \int_{-\infty}^\infty",
            r"\f\hat\xi\,e^{2 \pi i \xi x}",
            r"\,d\xi",
        ]
        .join(newline);
        let formula = format!("${newline}{payload}{newline}$");
        let source = format!(
            "| package | content |{newline}| --- | --- |{newline}\
             | [katex](https://github.com/AzurIce/notist/blob/main/packages/katex/README.not) | LaTeX 数学呈现 {formula} |{newline}\
             | after | following |{newline}{newline}outside"
        );
        let root = analyze("test.not", &source);
        let table = &root.children[0];
        assert_eq!(table.ctor, Ctor::Table);
        assert_eq!(table.children.len(), 3);
        assert!(table.children.iter().all(|row| row.children.len() == 2));
        let cell = &table.children[1].children[1];
        assert_eq!(
            cell.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
            [&Ctor::Paragraph, &Ctor::Math]
        );
        let math = &cell.children[1];
        assert_eq!(math.level, Level::Block);
        assert_eq!(math.fields.get("text"), Some(&Value::Str(payload)));
        assert_eq!(
            &source[usize::from(math.span.start())..usize::from(math.span.end())],
            formula
        );
        assert!(cell.span.contains_range(math.span));
        assert_eq!(text(&table.children[2]), "afterfollowing");
        assert_eq!(text(&root.children[1]), "outside");
        let html = notist_html::Renderer::new().render_with_diagnostics(&root);
        assert!(html.diagnostics.is_empty(), "{:?}", html.diagnostics);
        assert_eq!(html.html.matches("<tr>").count(), 3);
        assert_eq!(html.html.matches("<div class=\"notist-math\"").count(), 1);
    }
}

#[test]
fn math_in_headers_and_cells_owns_blank_lines_pipes_and_markup_tokens() {
    let payload = r#"x|y

#strong[literal | text]
= also literal
\| remains escaped"#;
    let source = format!(
        "| $\n{payload}\n$ | $a|b$ |\n| --- | --- |\n\
         | before $\n{payload}\n$ after | end |\n| next | row |"
    );
    let root = analyze("test.not", &source);
    let table = &root.children[0];
    assert_eq!(table.children.len(), 3);
    assert!(table.children.iter().all(|row| row.children.len() == 2));
    let equations: Vec<_> = table
        .descendants()
        .filter(|n| n.ctor == Ctor::Math)
        .collect();
    assert_eq!(equations.len(), 3);
    assert_eq!(
        equations[0].fields.get("text"),
        Some(&Value::Str(payload.into()))
    );
    assert_eq!(
        equations[1].fields.get("text"),
        Some(&Value::Str("a|b".into()))
    );
    assert_eq!(equations[1].level, Level::Inline);
    assert_eq!(
        equations[2].fields.get("text"),
        Some(&Value::Str(payload.into()))
    );
    let cell = &table.children[1].children[0];
    assert_eq!(
        cell.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
        [&Ctor::Paragraph, &Ctor::Math, &Ctor::Paragraph]
    );
    assert_eq!(text(&cell.children[0]).trim(), "before");
    assert_eq!(text(&cell.children[2]).trim(), "after");
    assert!(table.find(|n| n.ctor == Ctor::Strong).is_none());
    assert_eq!(text(&table.children[2]), "nextrow");
}

#[test]
fn newline_after_a_math_closer_ends_the_row_even_before_a_standalone_pipe() {
    let source = "| name | content |\n| --- | --- |\n| math | $\nx\n$\n|\n| next | row |";
    let root = analyze("test.not", source);
    let table = &root.children[0];
    assert_eq!(table.children.len(), 4);
    assert!(
        table.children[2]
            .children
            .iter()
            .all(|cell| cell.children.is_empty())
    );
    assert_eq!(text(&table.children[3]), "nextrow");
}

#[test]
fn markdown_tables_keep_physical_rows_and_gfm_pipe_escaping() {
    let source = "| name | content |\n| --- | --- |\n| math | $\nx +\ny\n$ |\n| next | row |";
    let not = analyze("test.not", source);
    let md = analyze("test.md", source);
    assert_eq!(not.children[0].children.len(), 3);
    assert_eq!(md.children[0].children.len(), 6);
    assert!(md.find(|n| n.ctor == Ctor::Math).is_none());
    let source = "| first | second |\n| --- | --- |\n| `left | right` |\n| $left | right$ |";
    let md = analyze("test.md", source);
    assert!(
        md.find(|n| matches!(n.ctor, Ctor::RawInline | Ctor::Math))
            .is_none()
    );
}

#[test]
fn multiline_strings_keep_their_payload_and_following_cells_and_rows() {
    for newline in ["\n", "\r\n", "\r"] {
        let payload = [
            "stateDiagram-v2",
            "    [*] --> s1",
            "",
            "    s1 --> [*]",
            "a|b ) ]",
        ]
        .join(newline);
        for (open, close) in [
            ("\"\"\"", "\"\"\""),
            ("r#\"\"\"", "\"\"\"#"),
            ("r##\"\"\"", "\"\"\"##"),
        ] {
            let src = format!(
                "| name | diagram | last |{newline}| --- | --- | --- |{newline}\
                 | mermaid | #raw({open}{newline}{payload}{newline}{close}) | tail |{newline}\
                 | next | plain | end |{newline}{newline}after"
            );
            let root = analyze("test.not", &src);
            let table = &root.children[0];
            assert_eq!(table.ctor, Ctor::Table);
            assert_eq!(table.children.len(), 3);
            assert!(table.children.iter().all(|row| row.children.len() == 3));
            let raw = table.children[1].children[1]
                .find(|node| node.ctor == Ctor::RawInline)
                .unwrap();
            assert_eq!(raw.fields.get("text"), Some(&Value::Str(payload.clone())));
            assert_eq!(text(&table.children[1].children[2]), "tail");
            assert_eq!(text(&table.children[2]), "nextplainend");
            assert_eq!(text(&root.children[1]), "after");
            for cell in table.children.iter().flat_map(|row| &row.children) {
                assert!(
                    cell.descendants()
                        .all(|node| cell.span.contains_range(node.span))
                );
            }
        }
    }
}

#[test]
fn mermaid_diagram_source_stays_in_one_table_cell_and_renders_as_a_component() {
    let environment = notist::Environment::from_packages([notist::Package {
        name: "mermaid".into(),
        root: "packages/mermaid".into(),
        source: include_str!("../packages/mermaid/lib.notc").into(),
    }])
    .unwrap();
    let src = r##"| Package | 内容 |
| --- | --- |
| [mermaid](packages/mermaid/README.not) | Mermaid 图形组件 #mermaid::diagram(r#"""
stateDiagram-v2
    [*] --> s1
    s1 --> [*]
"""#) |
| after | following |
"##;
    let parse = notist::syntax::parser::parse_document(src);
    assert_eq!(parse.syntax().to_string(), src);
    assert!(parse.diagnostics.is_empty(), "{:?}", parse.diagnostics);
    let analysis = Pipeline::default()
        .analyze("test.not", src, environment.registry())
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let table = &analysis.root().children[0];
    assert_eq!(table.children.len(), 3);
    assert!(table.children.iter().all(|row| row.children.len() == 2));
    let id = notist::FunctionId::new("mermaid", "diagram");
    let diagram = table.children[1].children[1]
        .find(|node| node.ctor.function_id() == Some(id.clone()))
        .unwrap();
    assert_eq!(diagram.level, Level::Block);
    assert_eq!(
        diagram.fields.get("source"),
        Some(&Value::Str(
            "stateDiagram-v2\n    [*] --> s1\n    s1 --> [*]".into()
        ))
    );
    assert_eq!(text(&table.children[2]), "afterfollowing");
    let mut html_registry = notist_html::HtmlRegistry::default();
    html_registry
        .bind_component(
            environment.registry().get(&id).unwrap(),
            "/components/mermaid/diagram.js",
        )
        .unwrap();
    let html = notist_html::Renderer::new()
        .with_registry(html_registry)
        .render_with_diagnostics(analysis.root());
    assert!(html.diagnostics.is_empty(), "{:?}", html.diagnostics);
    assert_eq!(html.used_components.len(), 1);
    assert_eq!(html.used_components[0].id, id);
}

#[test]
fn complete_calls_protect_pipes_and_wrap_headers_and_arguments() {
    let src = "#strong[name\ncontinued] | value\n--- | ---\n\
               #embed(\n  \"asset.svg\",\n  title: \"a|b\",\n) | #strong[one | #emph[two]]\n\
               after | plain";
    let calls = "#table(align: (\"none\", \"none\"))[\n\
                 #row(header: true)[#cell[#strong[name\ncontinued]] #cell[value]]\n\
                 #row(header: false)[#cell[#embed(\"asset.svg\", title: \"a|b\")] #cell[#strong[one | #emph[two]]]]\n\
                 #row(header: false)[#cell[after] #cell[plain]]\n]";
    let root = analyze("test.not", src);
    assert_eq!(
        normalized(root.clone()),
        normalized(analyze("test.not", calls))
    );
    let table = &root.children[0];
    assert_eq!(table.children.len(), 3);
    assert_eq!(text(&table.children[1].children[1]), "one | two");
    let embed = table.children[1].children[0]
        .find(|node| node.ctor == Ctor::Embed)
        .unwrap();
    assert_eq!(embed.fields.get("title"), Some(&Value::Str("a|b".into())));
}

#[test]
fn grouped_cells_accept_paragraphs_lists_and_nested_pipe_tables() {
    let body = "#[\nfirst | still inside\n\nsecond\n\n- one\n- two\n\n\
                | inner | value |\n| --- | --- |\n| x | y |\n]";
    let src =
        format!("| name | body |\n| --- | --- |\n| rich | {body} |\n| after | end |\n\noutside");
    let calls = format!(
        "#table(align: (\"none\", \"none\"))[\n\
         #row(header: true)[#cell[name] #cell[body]]\n\
         #row(header: false)[\n#cell[rich]\n#cell[\n{body}\n]\n]\n\
         #row(header: false)[#cell[after] #cell[end]]\n]\n\noutside"
    );
    let root = analyze("test.not", &src);
    assert_eq!(
        normalized(root.clone()),
        normalized(analyze("test.not", &calls))
    );
    let table = &root.children[0];
    assert_eq!(table.children.len(), 3);
    let cell = &table.children[1].children[1];
    assert_eq!(
        cell.find(|node| node.ctor == Ctor::Table)
            .unwrap()
            .children
            .len(),
        2
    );
    assert!(cell.find(|node| node.ctor == Ctor::List).is_some());
    assert_eq!(text(&table.children[2]), "afterend");
    assert_eq!(text(&root.children[1]), "outside");
    assert!(
        cell.descendants()
            .all(|node| cell.span.contains_range(node.span))
    );
    let html = notist_html::Renderer::new().render_with_diagnostics(&root);
    assert!(html.diagnostics.is_empty(), "{:?}", html.diagnostics);
    assert!(html.html.contains("<p>first | still inside</p>"));
    assert!(html.html.contains("<ul>"));
    assert_eq!(html.html.matches("<table>").count(), 2);
}

#[test]
fn calls_and_nested_tables_keep_opaque_payloads_unchanged() {
    let body = concat!(
        "#[\n",
        r"`raw|literal\|escaped` $math|literal\|escaped$",
        "\n\n| value |\n| --- |\n",
        r"| `nested\\\|pipe` |",
        "\n]"
    );
    let src = format!("| value |\n| --- |\n| {body} |");
    let calls = format!(
        "#table(align: (\"none\",))[\n\
         #row(header: true)[#cell[value]]\n\
         #row(header: false)[\n#cell[\n{body}\n]\n]\n]"
    );
    let root = analyze("test.not", &src);
    assert_eq!(
        normalized(root.clone()),
        normalized(analyze("test.not", &calls))
    );
    let payloads: Vec<_> = root
        .descendants()
        .filter(|node| matches!(node.ctor, Ctor::RawInline | Ctor::Math))
        .map(|node| node.fields.get("text").unwrap().clone())
        .collect();
    assert_eq!(
        payloads,
        [
            Value::Str(r"raw|literal\|escaped".into()),
            Value::Str(r"math|literal\|escaped".into()),
            Value::Str(r"nested\\\|pipe".into()),
        ]
    );
}

#[test]
fn logical_rows_work_inside_call_bodies_and_list_items() {
    let table =
        "| name | body |\n| --- | --- |\n| rich | #[\nfirst\n\nsecond\n] |\n| after | end |";
    let indented = table
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    for src in [
        format!("#callout[\n{table}\n]\n\noutside"),
        format!("- item\n{indented}\n- next\n\noutside"),
    ] {
        let root = analyze("test.not", &src);
        let table = root.find(|node| node.ctor == Ctor::Table).unwrap();
        assert_eq!(table.children.len(), 3);
        assert_eq!(text(&table.children[1]), "richfirstsecond");
        assert_eq!(text(&table.children[2]), "afterend");
        assert_eq!(text(root.children.last().unwrap()), "outside");
    }
}

#[test]
fn multiline_comments_are_opaque_inside_a_logical_row() {
    let src = "| a | b |\n| --- | --- |\n\
               | first /* a|b\n\n#call[x] */ last | #raw(\n/* ) |\ncomment */ \"value\"\n) |\n\
               | next | row |";
    let root = analyze("test.not", src);
    let table = &root.children[0];
    assert_eq!(table.children.len(), 3);
    assert_eq!(text(&table.children[1].children[0]), "first  last");
    assert_eq!(text(&table.children[2]), "nextrow");
    let raw = table.find(|node| node.ctor == Ctor::RawInline).unwrap();
    assert_eq!(raw.fields.get("text"), Some(&Value::Str("value".into())));
}

#[test]
fn opaque_payloads_own_literal_calls_and_pipes() {
    for (open, close) in [("`", "`"), ("$", "$"), ("$ ", " $")] {
        let payload = r"#strong[one | two]\|escaped";
        let src =
            format!("| a | b |\n| --- | --- |\n| {open}{payload}{close} | next |\n| next | row |");
        let parse = notist::syntax::parser::parse_document(&src);
        assert_eq!(parse.syntax().to_string(), src);
        let table = Table::cast(parse.syntax().children().next().unwrap()).unwrap();
        let rows: Vec<_> = table.rows().collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].cells().count(), 2, "{src}");
        let root = analyze("test.not", &src);
        let cell = &root.children[0].children[1].children[0];
        let node = cell
            .find(|node| matches!(node.ctor, Ctor::RawInline | Ctor::Math))
            .unwrap();
        assert_eq!(node.fields.get("text"), Some(&Value::Str(payload.into())));
        assert!(cell.find(|node| node.ctor == Ctor::Strong).is_none());
    }
    let root = analyze(
        "test.not",
        "| a | b |\n| --- | --- |\n| `left | #strong[x | y] right` |",
    );
    assert_eq!(
        root.children[0].children[1].children[0]
            .find(|n| n.ctor == Ctor::RawInline)
            .unwrap()
            .fields
            .get("text"),
        Some(&Value::Str("left | #strong[x | y] right".into()))
    );
}

#[test]
fn incomplete_calls_and_plain_brackets_do_not_join_table_rows() {
    for body in [
        "#call[x",
        "#[x",
        "#call(\"x\")[body",
        "#call(",
        "#pkg::",
        "#pkg::()",
        "#call [body",
        "#()[body",
        "plain (text",
        "plain [text",
        "`unclosed",
        "$unclosed",
        "$ unclosed",
    ] {
        let src = format!("| a | b |\n| --- | --- |\n| {body} | tail |\n| next | row |");
        let parse = notist::syntax::parser::parse_document(&src);
        assert_eq!(parse.syntax().to_string(), src);
        let table = Table::cast(parse.syntax().children().next().unwrap()).unwrap();
        assert_eq!(table.rows().count(), 3, "{src}");
    }
}

#[test]
fn emphasis_and_links_cannot_pair_across_cells_or_rows() {
    let src = "| *left | right* |\n| --- | --- |\n| `left | right` |\n| $left | right$ |\n| [broken | label](target) |";
    let root = analyze("test.not", src);
    let table = &root.children[0];
    assert_eq!(table.children.len(), 4);
    assert!(
        table
            .find(|n| matches!(n.ctor, Ctor::Strong | Ctor::Link))
            .is_none()
    );
    for cell in table.children.iter().flat_map(|r| &r.children) {
        for node in cell.descendants() {
            assert!(cell.span.contains_range(node.span));
        }
    }
}

#[test]
fn tables_split_surrounding_paragraphs_and_work_in_block_bodies_with_attrs() {
    let src = "before\n@(id: \"grid\")\n| a | b |\n| --- | --- |\n| x | y |\n\nafter\n---";
    let root = analyze("test.not", src);
    assert_eq!(
        root.children.iter().map(|n| &n.ctor).collect::<Vec<_>>(),
        [
            &Ctor::Paragraph,
            &Ctor::Table,
            &Ctor::Paragraph,
            &Ctor::Divider
        ]
    );
    assert_eq!(
        root.children[1].attrs.get("id"),
        Some(&Value::Str("grid".into()))
    );
    let direct = analyze(
        "test.not",
        "before\n| a | b |\n| --- | --- |\n| x | y |\n\nafter",
    );
    assert_eq!(direct.children[1].ctor, Ctor::Table);
    let nested = analyze(
        "test.not",
        "#callout[\n| a | b |\n| --- | --- |\n| x | y |\n]",
    );
    assert_eq!(nested.children[0].children[0].ctor, Ctor::Table);
    let headed = analyze(
        "test.not",
        "= Section\n| a |\n| --- |\n| x |\n\n== Next\ntext",
    );
    assert_eq!(headed.children[0].children[1].ctor, Ctor::Table);
}

#[test]
fn explicit_cells_accept_block_content_and_inline_structural_mounts_recurse() {
    let root = analyze(
        "test.not",
        "#table[\n#row[\n@(id: \"cell\")#cell[\n= Inside\nfirst\n\nsecond\n]\n]\n]",
    );
    let cell = &root.children[0].children[0].children[0];
    assert_eq!(cell.ctor, Ctor::TableCell);
    assert_eq!(cell.attrs.get("id"), Some(&Value::Str("cell".into())));
    assert_eq!(cell.children[0].ctor, Ctor::Section);
    assert_eq!(cell.children[0].children.len(), 3);
    let inline = analyze(
        "test.not",
        "#table[#row[#cell[first #callout[hint] second]]]",
    );
    assert_eq!(
        inline.children[0].children[0].children[0]
            .children
            .iter()
            .map(|n| &n.ctor)
            .collect::<Vec<_>>(),
        [&Ctor::Paragraph, &Ctor::Callout, &Ctor::Paragraph]
    );
    let empty = analyze("test.not", "#table[#row[#cell[]]]");
    assert!(
        empty.children[0].children[0].children[0]
            .children
            .is_empty()
    );
}

#[test]
fn malformed_table_markers_remain_prose_and_wrong_structural_children_diagnose() {
    for src in [
        "a | b\n---",
        "a | b\n--- | nope",
        "a | b\n--- | --- | ---",
        "a | b\n--- | ::---",
        "a | b\n--- | - - -",
        " a | b\n--- | ---",
        "plain\n---",
        "#strong[a | b]\n---",
        "| a | b |",
    ] {
        let root = analyze("test.not", src);
        assert!(root.find(|n| n.ctor == Ctor::Table).is_none(), "{src:?}");
    }
    for (src, message) in [
        ("#table[text]", "`table` takes only row children"),
        ("#table[#paragraph[]]", "`table` takes only row children"),
        ("#table[#cell[x]]", "`table` takes only row children"),
        ("#row[text]", "`row` takes only cell children"),
        ("#row[#row[]]", "`row` takes only cell children"),
    ] {
        let analysis = Pipeline::default()
            .analyze("test.not", src, notist::builtins::registry())
            .unwrap();
        assert!(
            analysis
                .diagnostics()
                .iter()
                .any(|d| d.message.contains(message)),
            "{src:?}"
        );
    }
}

#[test]
fn malformed_inline_content_in_cells_keeps_the_cst_lossless() {
    for cell in [
        "*unclosed",
        "_unclosed",
        "~unclosed",
        "$unclosed",
        "`unclosed",
        "[unclosed",
        "#call(",
        "@(id:",
        "#call[x",
        "]",
        "\\",
        "[[wiki|text]]",
    ] {
        let src = format!("| a | b |\n| --- | --- |\n| {cell} | next |\n");
        let parse = notist::syntax::parser::parse_document(&src);
        assert_eq!(parse.syntax().to_string(), src);
        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == SyntaxKind::Table)
        );
    }
}
