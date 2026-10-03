use std::path::PathBuf;

fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("notist-lib-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("a.not"),
        "@(id: \"target\")\n#note[目标]\n\n[到 b](sub/b.not) 与 [坏链](missing.not) 与 [外](https://x.y)\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sub").join("b.not"),
        "@(id: \"sec\")\n= 小节\n\n[回 a](../a.not) 与 [好 item](../a.not#target) 与 [坏 item](../a.not#nope)\n",
    )
    .unwrap();
    dir
}

#[test]
fn library_link_graph() {
    let dir = fixture();
    let library = notist::vault::Vault::load(&dir).unwrap();
    let diags = library.check();
    let messages: Vec<_> = diags.iter().map(|(_, d)| d.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("unresolved link target `missing.not`")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("missing item `#nope`")),
        "{messages:?}"
    );
    assert!(
        !messages.iter().any(|m| m.contains("b.not")),
        "{messages:?}"
    );
    // 反链：a.not 被 sub/b.not 引用（3 条链接）
    let backlinks = library.backlinks(&PathBuf::from("a.not"));
    assert_eq!(backlinks.len(), 3);
    assert!(
        backlinks
            .iter()
            .all(|(p, _)| p == &PathBuf::from("sub/b.not"))
    );
    std::fs::remove_dir_all(&dir).ok();
}
