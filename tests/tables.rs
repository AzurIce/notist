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
fn table_pipes_can_be_escaped_in_text_formatting_raw_and_math() {
    let not = "| value |\n| --- |\n| a\\|b *c\\|d* `e\\|f` $g\\|h$ |";
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
fn inline_delimiters_cannot_pair_across_cells_or_rows() {
    let src = "| *left | right* |\n| --- | --- |\n| `left | right` |\n| $left | right$ |\n| [broken | label](target) |";
    let root = analyze("test.not", src);
    let table = &root.children[0];
    assert_eq!(table.children.len(), 4);
    assert!(
        table
            .find(|n| matches!(
                n.ctor,
                Ctor::Strong | Ctor::RawInline | Ctor::Math | Ctor::Link
            ))
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
