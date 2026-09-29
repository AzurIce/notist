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
        insta::assert_snapshot!(name, notist::dump_str(&src));
    }
}
