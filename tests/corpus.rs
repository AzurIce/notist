use std::fs;

#[test]
fn corpus() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/corpus");
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "not"))
        .collect();
    entries.sort();
    for path in entries {
        let src = fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_str().unwrap().to_string();
        let cst = format!(
            "{:#?}",
            notist_syntax::parser::parse_document(&src).syntax()
        );
        let analysis = notist::Pipeline::default()
            .analyze(&path, &src, notist::builtins::registry())
            .unwrap();
        let mut core = String::new();
        for diagnostic in analysis.diagnostics() {
            core.push_str(&format!(
                "error[{}] @{}..{}: {}\n",
                diagnostic.phase,
                u32::from(diagnostic.span.start()),
                u32::from(diagnostic.span.end()),
                diagnostic.message
            ));
        }
        core.push_str(&notist::dump::dump(analysis.root()));
        insta::assert_snapshot!(name, format!("{cst}\n=== core ===\n{core}"));
    }
}
