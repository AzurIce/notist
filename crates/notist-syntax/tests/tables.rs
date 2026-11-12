use notist_syntax::{
    code::ast::CodeCall,
    markup::{ast::Table, parse_document},
};

#[test]
fn comptime_arguments_precede_value_arguments_and_bodies_in_table_calls() {
    for (call, args, body) in [
        ("#widget<BlockContent>()", 0, false),
        (
            "#widget<BlockContent>[body | #strong[nested | pipe]]",
            0,
            true,
        ),
        (
            "#widget<BlockContent>(title: \"x|y\")[body | #strong[nested | pipe]]",
            1,
            true,
        ),
        (
            "#widget<BlockContent>(r#\"\"\"\na|b ) ]\n\"\"\"#, title: \"x|y\")",
            2,
            false,
        ),
        (
            "#widget<BlockContent>(r#\"\"\"\na|b ) ]\n\"\"\"#, title: \"x|y\")[body | #strong[nested | pipe]]",
            2,
            true,
        ),
    ] {
        for newline in ["\n", "\r\n", "\r"] {
            let call = call.replace('\n', newline);
            let source = format!(
                "| {call} | header |{newline}| --- | --- |{newline}\
                 | {call} | tail |{newline}| next | row |{newline}{newline}outside"
            );
            let parsed = parse_document(&source);
            assert_eq!(parsed.syntax().to_string(), source);
            assert!(
                parsed.diagnostics.is_empty(),
                "{source:?}: {:?}",
                parsed.diagnostics
            );
            let table = Table::cast(parsed.syntax().children().next().unwrap()).unwrap();
            let rows: Vec<_> = table.rows().collect();
            assert_eq!(rows.len(), 3, "{source:?}");
            assert!(rows.iter().all(|row| row.cells().count() == 2));
            for row in &rows[..2] {
                let cell = row.cells().next().unwrap();
                let node = cell
                    .content()
                    .filter_map(|element| element.into_node())
                    .find_map(CodeCall::cast)
                    .unwrap();
                assert_eq!(node.syntax().text().to_string(), call);
                assert_eq!(node.args().len(), args);
                assert_eq!(node.has_body(), body);
            }
        }
    }
}
