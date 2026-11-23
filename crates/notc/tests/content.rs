use notc::{AttrValue, BlockKind, InlineKind, Type, TypeError, Value, check, parse};

fn eval_entry(source: &str, entry: &str) -> Value {
    let parsed = parse(source);
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected parse diagnostics: {:?}",
        parsed.diagnostics
    );
    let program = check(&parsed.program).expect("program should type-check");
    program.run(entry).expect("entry should evaluate")
}

#[test]
fn flank_rule_distinguishes_inline_and_block_literals() {
    let source = r#"
        fn empty_inline() -> Inline { [] }
        fn empty_block() -> Block { [ ] }
        fn inline_lit() -> Inline { [Hello *world* and _notc_ with `code`!] }
        fn block_single_line() -> Block { [ Hello *world* ] }
        fn block_multi_line() -> Block {
            [
                First paragraph
                continued here.

                Second paragraph.
            ]
        }
    "#;

    let Value::Inline(empty_i) = eval_entry(source, "empty_inline") else {
        panic!("expected Inline");
    };
    assert_eq!(empty_i.plain_text(), "");
    assert!(matches!(empty_i.node().kind, InlineKind::Fragment(ref xs) if xs.is_empty()));

    let Value::Block(empty_b) = eval_entry(source, "empty_block") else {
        panic!("expected Block");
    };
    assert_eq!(empty_b.plain_text(), "");
    assert!(matches!(empty_b.node().kind, BlockKind::Fragment(ref xs) if xs.is_empty()));

    let Value::Inline(inl) = eval_entry(source, "inline_lit") else {
        panic!("expected Inline");
    };
    assert_eq!(inl.plain_text(), "Hello world and notc with code!");

    let Value::Block(blk) = eval_entry(source, "block_single_line") else {
        panic!("expected Block");
    };
    assert_eq!(blk.plain_text(), "Hello world");
    assert!(matches!(blk.node().kind, BlockKind::Paragraph(_)));

    let Value::Block(multi) = eval_entry(source, "block_multi_line") else {
        panic!("expected Block");
    };
    assert_eq!(
        multi.plain_text(),
        "First paragraph continued here.\nSecond paragraph."
    );
    assert!(matches!(multi.node().kind, BlockKind::Fragment(ref xs) if xs.len() == 2));
}

#[test]
fn trailing_content_call_sugar_and_multi_block_calls() {
    let source = r#"
        fn columns(left: Block, right: Block) -> Block {
            left + right
        }

        fn Inline.wrap(self, inner: Inline) -> Inline {
            [#self #inner]
        }

        fn main() -> Block {
            let title = strong[Hello];
            let subtitle = [Prefix:].wrap[ready];
            let pair = columns[
                Left column.
            ][
                Right column.
            ];
            callout("tip")[
                #title

                #subtitle

                #pair
            ]
        }
    "#;

    let Value::Block(doc) = eval_entry(source, "main") else {
        panic!("expected Block");
    };
    let BlockKind::Callout { kind, body } = &doc.node().kind else {
        panic!("expected Callout, got {doc}");
    };
    assert_eq!(kind.as_ref(), "tip");
    assert_eq!(
        body.plain_text(),
        "Hello\nPrefix: ready\nLeft column.\nRight column."
    );
}

#[test]
fn statically_rejects_block_in_inline_and_implicit_conversions() {
    // 1. Passing Block literal to Inline parameter via trailing `[ ... ]`.
    let parsed = parse(r#"fn bad() -> Inline { strong[ not inline ] }"#);
    assert!(parsed.diagnostics.is_empty());
    let err = check(&parsed.program).unwrap_err();
    assert_eq!(
        err[0].error,
        TypeError::Mismatch {
            expected: Type::Inline,
            actual: Type::Block,
        }
    );

    // 2. Interpolating Block inside Inline literal.
    let parsed = parse(
        r#"
        fn bad() -> Inline {
            let b = [ block ];
            [hello #b]
        }
    "#,
    );
    assert!(parsed.diagnostics.is_empty());
    let err = check(&parsed.program).unwrap_err();
    assert_eq!(
        err[0].error,
        TypeError::Mismatch {
            expected: Type::Inline,
            actual: Type::Block,
        }
    );

    // 3. No implicit Inline -> Block conversion in code mode.
    let parsed = parse(r#"fn bad() -> Block { [inline only] }"#);
    assert!(parsed.diagnostics.is_empty());
    let err = check(&parsed.program).unwrap_err();
    assert_eq!(
        err[0].error,
        TypeError::Mismatch {
            expected: Type::Block,
            actual: Type::Inline,
        }
    );

    // 4. Rejects non-content expression interpolated inside Block literal.
    let parsed = parse(
        r#"
        fn bad() -> Block {
            let n = 42;
            [
                #n
            ]
        }
    "#,
    );
    assert!(parsed.diagnostics.is_empty());
    let err = check(&parsed.program).unwrap_err();
    assert_eq!(
        err[0].error,
        TypeError::Mismatch {
            expected: Type::Block,
            actual: Type::Int,
        }
    );
}

#[test]
fn generic_content_polymorphism_with_where_and_match() {
    let source = r#"
        fn decorate<C: Type>(content: C) -> C
        where C == Inline || C == Block
        {
            match C {
                Inline => strong(content),
                Block  => callout("note", content),
                _      => compile_error("unreachable"),
            }
        }

        fn inline_out() -> Inline { decorate[hi] }
        fn block_out() -> Block { decorate[ hello ] }
        fn explicit_fn_value() -> Inline {
            let f = decorate<Inline>;
            f[ok]
        }
    "#;

    let Value::Inline(i) = eval_entry(source, "inline_out") else {
        panic!("expected Inline");
    };
    assert!(matches!(i.node().kind, InlineKind::Strong(_)));
    assert_eq!(i.plain_text(), "hi");

    let Value::Block(b) = eval_entry(source, "block_out") else {
        panic!("expected Block");
    };
    assert!(matches!(b.node().kind, BlockKind::Callout { ref kind, .. } if kind.as_ref() == "note"));
    assert_eq!(b.plain_text(), "hello");

    let Value::Inline(i2) = eval_entry(source, "explicit_fn_value") else {
        panic!("expected Inline");
    };
    assert_eq!(i2.plain_text(), "ok");

    // Calling decorate with Int fails at the call site via `where`.
    let bad = r#"
        fn decorate<C: Type>(content: C) -> C
        where C == Inline || C == Block
        {
            match C {
                Inline => strong(content),
                Block  => callout("note", content),
                _      => compile_error("unreachable"),
            }
        }
        fn main() -> Int { decorate(1) }
    "#;
    let parsed = parse(bad);
    assert!(parsed.diagnostics.is_empty());
    let err = check(&parsed.program).unwrap_err();
    assert_eq!(
        err[0].error,
        TypeError::WhereUnsatisfied {
            function: "decorate".into(),
        }
    );
}

#[test]
fn structural_sharing_on_reuse_and_immutable_rewrites() {
    // 1. Reusing a shared block binding twice keeps exact Arc pointer identity.
    let reuse_src = r#"
        fn main() -> Block {
            let p = [ Hello *world* ];
            [
                #p
                #p
            ]
        }
    "#;
    let Value::Block(doc) = eval_entry(reuse_src, "main") else {
        panic!("expected Block");
    };
    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected Block Fragment, got {doc}");
    };
    assert_eq!(children.len(), 2);
    assert!(children[0].ptr_eq(&children[1]));

    // 2. Identity rewrite preserves root & child Arc pointers.
    let id_src = r#"
        fn keep(x: Inline) -> Inline { x }
        fn main() -> Block {
            let original = [
                First *para*.

                Second _para_.
            ];
            let rewritten = original.rewrite_inlines(keep);
            [
                #original
                #rewritten
            ]
        }
    "#;
    let Value::Block(both) = eval_entry(id_src, "main") else {
        panic!("expected Block");
    };
    // Because original and rewritten are both unannotated 2-child fragments,
    // canonical flattening yields 4 paragraphs where [0] ptr_eq [2] and [1] ptr_eq [3]!
    let BlockKind::Fragment(paras) = &both.node().kind else {
        panic!("expected flattened fragment, got {both}");
    };
    assert_eq!(paras.len(), 4);
    assert!(paras[0].ptr_eq(&paras[2]));
    assert!(paras[1].ptr_eq(&paras[3]));

    // 3. Selective rewrite rebuilds only modified branch while sharing untouched sibling.
    let selective_src = r#"
        fn promote_tagged(node: Inline) -> Inline {
            if node.has_attr("mark") {
                strong(node)
            } else {
                node
            }
        }
        fn main() -> Block {
            let a = [
                Untouched paragraph.
            ];
            let b = [
                Changed @(mark: true)*here*.
            ];
            let doc = a + b;
            let next = doc.rewrite_inlines(promote_tagged);
            doc + next
        }
    "#;
    let Value::Block(combined) = eval_entry(selective_src, "main") else {
        panic!("expected Block");
    };
    let BlockKind::Fragment(items) = &combined.node().kind else {
        panic!("expected fragment, got {combined}");
    };
    assert_eq!(items.len(), 4);
    // Untouched paragraph keeps exact Arc identity across the rewrite.
    assert!(items[0].ptr_eq(&items[2]));
    // Modified paragraph was rebuilt cleanly without mutating the original.
    assert!(!items[1].ptr_eq(&items[3]));
    assert_ne!(items[1], items[3]);
}

#[test]
fn canonical_fragment_flattening_preserves_annotated_shells() {
    let source = r#"
        fn main() -> Block {
            let plain = [
                One.

                Two.
            ];
            let tagged = plain.with_attr("group", "keep");
            [
                #plain
                #tagged
            ]
        }
    "#;
    let Value::Block(doc) = eval_entry(source, "main") else {
        panic!("expected Block");
    };
    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected fragment, got {doc}");
    };
    // `plain` flattened into 2 paragraphs; `tagged` kept its annotated Fragment shell as item #3.
    assert_eq!(children.len(), 3);
    assert!(matches!(children[0].node().kind, BlockKind::Paragraph(_)));
    assert!(matches!(children[1].node().kind, BlockKind::Paragraph(_)));
    assert!(matches!(children[2].node().kind, BlockKind::Fragment(ref xs) if xs.len() == 2));
    assert_eq!(
        children[2].node().attrs.get("group"),
        Some(&AttrValue::String("keep".into()))
    );
}

#[test]
fn sectionize_folds_flat_headings_into_nested_sections() {
    let source = r#"
        fn doc() -> Block {
            [
                Preamble.

                @(id: "ch1")
                = Chapter 1

                Intro.

                == Section 1.1

                Body 1.1.

                == Section 1.2

                Body 1.2.

                = Chapter 2

                End.
            ]
        }

        fn sectioned() -> Block { doc().sectionize() }
        fn count() -> Int { doc().heading_count() }
        fn summary() -> String { doc().headings_summary() }
    "#;

    let Value::Int(count) = eval_entry(source, "count") else {
        panic!("expected Int");
    };
    assert_eq!(count, 4);

    let Value::String(summary) = eval_entry(source, "summary") else {
        panic!("expected String");
    };
    assert_eq!(
        summary.as_ref(),
        "h1: Chapter 1 | h2: Section 1.1 | h2: Section 1.2 | h1: Chapter 2"
    );

    let Value::Block(sec) = eval_entry(source, "sectioned") else {
        panic!("expected Block");
    };
    let BlockKind::Fragment(top) = &sec.node().kind else {
        panic!("expected top fragment, got {sec}");
    };
    // Top-level: Preamble, Section(Chapter 1), Section(Chapter 2)
    assert_eq!(top.len(), 3);
    assert!(matches!(top[0].node().kind, BlockKind::Paragraph(_)));

    let BlockKind::Section { heading, body } = &top[1].node().kind else {
        panic!("expected Section for Chapter 1, got {}", top[1]);
    };
    assert_eq!(
        top[1].node().attrs.get("id"),
        Some(&AttrValue::String("ch1".into()))
    );
    assert!(heading.node().attrs.is_empty());
    let BlockKind::Fragment(ch1_children) = &body.node().kind else {
        panic!("expected Chapter 1 body fragment, got {body}");
    };
    // Chapter 1 body: Intro paragraph + Section 1.1 + Section 1.2
    assert_eq!(ch1_children.len(), 3);
    assert!(matches!(ch1_children[0].node().kind, BlockKind::Paragraph(_)));
    assert!(matches!(ch1_children[1].node().kind, BlockKind::Section { .. }));
    assert!(matches!(ch1_children[2].node().kind, BlockKind::Section { .. }));

    assert!(matches!(top[2].node().kind, BlockKind::Section { .. }));
}

#[test]
fn attributes_equality_escapes_and_first_class_builtins() {
    let source = r#"
        fn eq_check() -> Bool {
            let a = [Hello *world*];
            let b = [Hello ] + strong[world];
            a == b
        }

        fn attr_check() -> String {
            let p = [
                @(id: "p1", draft: true)
                Hello @(note: "a]b")*bold* and #(text("ok]here"))!
            ];
            p.attr_or("id", "none") + ":" + p.attr_or("draft", "no") + ":" + p.attr_or("missing", "fb")
        }

        fn builtin_as_callback() -> Block {
            let doc = [
                Alpha
            ];
            doc.rewrite_inlines(emph)
        }
    "#;

    assert!(matches!(eval_entry(source, "eq_check"), Value::Bool(true)));

    let Value::String(s) = eval_entry(source, "attr_check") else {
        panic!("expected String");
    };
    assert_eq!(s.as_ref(), "p1:true:fb");

    let Value::Block(b) = eval_entry(source, "builtin_as_callback") else {
        panic!("expected Block");
    };
    let BlockKind::Paragraph(inner) = &b.node().kind else {
        panic!("expected Paragraph, got {b}");
    };
    assert!(matches!(inner.node().kind, InlineKind::Emph(_)));
    assert_eq!(inner.plain_text(), "Alpha");
}

#[test]
fn plain_text_inline_attrs_and_single_line_block_attr_expr() {
    let doc_src = r#"
- @(collapsed: true)待整理
  - [Sub](./sub.not)

@(id: "box-1", aria-label: "Info")#callout("info")[
    Body text.
]
"#;
    let project = notc::check_project(&[("index.not", doc_src)]).expect("checks");
    let doc = project.eval_document("index.not").expect("evaluates");
    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected fragment");
    };
    assert_eq!(children.len(), 2);
    let BlockKind::List { items, .. } = &children[0].node().kind else {
        panic!("expected list");
    };
    let BlockKind::Fragment(item_blocks) = &items[0].node().kind else {
        panic!("expected list item fragment");
    };
    let BlockKind::Paragraph(label) = &item_blocks[0].node().kind else {
        panic!("expected paragraph");
    };
    assert_eq!(
        label.node().attrs.get("collapsed"),
        Some(&AttrValue::Bool(true))
    );
    assert_eq!(label.plain_text(), "待整理");
    assert_eq!(
        children[1].node().attrs.get("id"),
        Some(&AttrValue::String("box-1".into()))
    );
    assert_eq!(
        children[1].node().attrs.get("aria-label"),
        Some(&AttrValue::String("Info".into()))
    );
}

#[test]
fn ast_level_attrs_support_multiline_list_items_soft_breaks_stacking_and_lists() {
    let doc_src = r#"
@!(tags: ("one", "two"), single: ("only",), empty: ())

- @(collapsed: true)
  Group
  - [Child](./child.not)

@(a: 1)
@(b: 2)
@(k: "v")soft
wrapped
"#;
    let project = notc::check_project(&[("index.not", doc_src)]).expect("checks");
    let doc = project.eval_document("index.not").expect("evaluates");
    assert_eq!(
        doc.node().attrs.get("tags"),
        Some(&AttrValue::List(std::sync::Arc::from([
            AttrValue::String("one".into()),
            AttrValue::String("two".into()),
        ])))
    );
    assert_eq!(
        doc.node().attrs.get("single"),
        Some(&AttrValue::List(std::sync::Arc::from([
            AttrValue::String("only".into()),
        ])))
    );
    assert_eq!(
        doc.node().attrs.get("empty"),
        Some(&AttrValue::List(std::sync::Arc::from([])))
    );

    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected fragment");
    };
    let BlockKind::List { items, .. } = &children[0].node().kind else {
        panic!("expected list");
    };
    let BlockKind::Fragment(item_blocks) = &items[0].node().kind else {
        panic!("expected item fragment");
    };
    assert_eq!(
        item_blocks[0].node().attrs.get("collapsed"),
        Some(&AttrValue::Bool(true))
    );
    assert_eq!(item_blocks[0].plain_text(), "Group");

    // Stacked block attrs + soft-wrapped inline text attr
    assert_eq!(children[1].node().attrs.get("a"), Some(&AttrValue::Int(1)));
    assert_eq!(children[1].node().attrs.get("b"), Some(&AttrValue::Int(2)));
    let BlockKind::Paragraph(inl) = &children[1].node().kind else {
        panic!("expected paragraph");
    };
    assert_eq!(
        inl.node().attrs.get("k"),
        Some(&AttrValue::String("v".into()))
    );
    assert_eq!(inl.plain_text(), "soft wrapped");
}

#[test]
fn strike_underline_ordered_list_start_and_table_alignments() {
    let doc_src = r#"
Text with ~~deleted~~ and #strike[also deleted] and #underline[underlined].

3. Third item
4. Fourth item

| Left | Center | Right | Plain |
| :--- | :---: | ---: | --- |
| a | b | c | d |
"#;
    let project = notc::check_project(&[("index.not", doc_src)]).expect("checks");
    let doc = project.eval_document("index.not").expect("evaluates");
    let BlockKind::Fragment(children) = &doc.node().kind else {
        panic!("expected fragment, got {doc}");
    };

    // Paragraph with strike and underline
    let BlockKind::Paragraph(inl) = &children[0].node().kind else {
        panic!("expected paragraph");
    };
    let InlineKind::Fragment(parts) = &inl.node().kind else {
        panic!("expected inline fragment");
    };
    assert_eq!(
        parts
            .iter()
            .filter(|p| matches!(p.node().kind, InlineKind::Strike(_)))
            .count(),
        2
    );
    assert_eq!(
        parts
            .iter()
            .filter(|p| matches!(p.node().kind, InlineKind::Underline(_)))
            .count(),
        1
    );

    // Ordered list starting at 3
    let BlockKind::List {
        ordered: true,
        start,
        items,
    } = &children[1].node().kind
    else {
        panic!("expected ordered list, got {}", children[1]);
    };
    assert_eq!(*start, 3);
    assert_eq!(items.len(), 2);

    // Pipe table with alignments
    let BlockKind::Table {
        headers,
        alignments,
        rows,
    } = &children[2].node().kind
    else {
        panic!("expected table, got {}", children[2]);
    };
    assert_eq!(headers.len(), 4);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        alignments.as_ref(),
        &[
            notc::TableAlign::Left,
            notc::TableAlign::Center,
            notc::TableAlign::Right,
            notc::TableAlign::None,
        ]
    );
}
