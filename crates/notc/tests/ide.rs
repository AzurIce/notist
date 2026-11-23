use notc::ide::{CompletionKind, SymbolKind, WorkspaceAnalysis};

#[test]
fn ide_provides_cross_file_hover_goto_def_symbols_and_type_aware_completions() {
    let ui = r#"
fn badge<C: Type>(content: C) -> C
where C == Inline || C == Block
{
    match C {
        Inline => strong(content),
        Block  => callout("tip", content),
        _      => compile_error("unreachable"),
    }
}

fn Block.banner(self) -> Block {
    callout("banner", self)
}
"#;
    let doc = r#"
@!(title: "Home")
#import "./ui.notc" as ui

@(id: "welcome")
= Welcome

See [details](./other.not#sec) and #ui.badge[Notist].

#ui.badge[
    Hello
]
"#;
    let other = r#"
@(id: "sec")
= Details

== Subtopic

Body text.
"#;

    let analysis = WorkspaceAnalysis::analyze(&[
        ("ui.notc", ui),
        ("index.not", doc),
        ("other.not", other),
    ]);
    assert!(
        analysis.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        analysis.diagnostics()
    );

    // 1. Document symbols for `.notc` and `.not`
    let ui_syms = analysis.document_symbols("ui.notc");
    assert_eq!(ui_syms.len(), 2);
    assert_eq!(ui_syms[0].name, "badge");
    assert_eq!(ui_syms[0].kind, SymbolKind::Function);
    assert_eq!(ui_syms[1].name, "Block.banner");
    assert_eq!(ui_syms[1].kind, SymbolKind::Method);

    let other_syms = analysis.document_symbols("other.not");
    assert_eq!(other_syms.len(), 1);
    assert_eq!(other_syms[0].name, "Details");
    assert_eq!(other_syms[0].kind, SymbolKind::Heading);
    assert_eq!(other_syms[0].children.len(), 1);
    assert_eq!(other_syms[0].children[0].name, "Subtopic");

    // 2. Hover & Goto Definition from `.not` -> `.notc`
    let badge_offset = doc.find("badge").expect("badge in doc") as u32;
    let hover = analysis.hover("index.not", badge_offset).expect("hover on badge");
    assert!(hover.contents.contains("fn badge<C: Type>(content: C) -> C"));
    assert!(hover.contents.contains("where C == Inline || C == Block"));

    let def = analysis
        .goto_definition("index.not", badge_offset)
        .expect("goto def on badge");
    assert_eq!(def.module, "ui.notc");
    assert_eq!(
        &ui[def.span.start as usize..def.span.end as usize],
        "badge"
    );

    // 3. Cross-document link jump `./other.not#sec`
    let link_offset = doc.find("./other.not#sec").expect("link in doc") as u32;
    let link_def = analysis
        .goto_definition("index.not", link_offset)
        .expect("goto def on link");
    assert_eq!(link_def.module, "other.not");
    assert_eq!(
        &other[link_def.span.start as usize..link_def.span.end as usize],
        "= Details"
    );

    // 4. Namespace dot completion inside `.not`: `#ui.`
    let typing_doc = "#import \"./ui.notc\" as ui\n#ui.";
    let typing_analysis = WorkspaceAnalysis::analyze(&[("ui.notc", ui), ("doc.not", typing_doc)]);
    let items = typing_analysis.completions("doc.not", typing_doc.len() as u32);
    assert!(items.iter().any(|c| c.label == "badge" && c.kind == CompletionKind::Function));
    assert!(!items.iter().any(|c| c.label == "strong"));

    // 5. Type-aware method completion on `Block` inside `.notc`: `doc.`
    let code_typing = r#"
import "./ui.notc";
fn test(doc: Block) -> Block {
    doc.
}
"#;
    let dot_pos = (code_typing.find("doc.").unwrap() + 4) as u32;
    let method_analysis = WorkspaceAnalysis::analyze(&[("ui.notc", ui), ("main.notc", code_typing)]);
    let methods = method_analysis.completions("main.notc", dot_pos);
    // Should include BOTH user-defined `Block.banner` and built-in `Block.sectionize`!
    assert!(
        methods
            .iter()
            .any(|c| c.label == "banner" && c.kind == CompletionKind::Method),
        "expected user method `banner`, got {methods:?}"
    );
    assert!(
        methods
            .iter()
            .any(|c| c.label == "sectionize" && c.kind == CompletionKind::Method),
        "expected built-in method `sectionize`, got {methods:?}"
    );
    // Should NOT include `Inline`-only methods like `rewrite`
    assert!(!methods.iter().any(|c| c.label == "rewrite"));
}
