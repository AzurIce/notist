use notist_grammar::{ExprKind, RenderOptions, Theme, parse, render_source};

#[test]
fn parses_and_draws_actual_documentation_productions() {
    let documents = [
        ("notation", include_str!("../../../grammar/notation.not")),
        ("markup", include_str!("../../../grammar/README.not")),
        ("code", include_str!("../../../grammar/code.not")),
        ("types", include_str!("../../../types.not")),
        ("builtin", include_str!("../../../builtin.not")),
        ("diagrams", include_str!("../../../grammar/diagrams.not")),
        ("package", include_str!("../README.not")),
    ];
    let mut count = 0;
    for (name, source) in documents {
        for block in source.split("#grammar::diagram(r#\"\"\"\n").skip(1) {
            let block = block.split("\n\"\"\"#").next().unwrap();
            let grammar = parse(block).unwrap_or_else(|error| panic!("{name}: {error}\n{block}"));
            if name == "code" {
                let directory =
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../grammar");
                for import in &grammar.imports {
                    assert!(
                        directory.join(&import.from).is_file(),
                        "invalid import locator: {}",
                        import.from
                    );
                }
            }
            let svg = render_source(block, &RenderOptions::default()).unwrap();
            assert_eq!(svg.matches("<svg ").count(), grammar.rules.len(), "{name}");
            assert!(svg.contains("<title>"));
            count += grammar.rules.len();
        }
    }
    assert!(count >= 100, "only covered {count} documentation rules");
}

#[test]
fn prefixes_have_lower_precedence_than_repetition() {
    let grammar = parse("syntax Example -> &A* | value:B? | !@{ ok(ctx) } NL").unwrap();
    let ExprKind::OrderedChoice(choices) = &grammar.rules[0].expression.kind else {
        panic!()
    };
    let ExprKind::Lookahead {
        expression,
        positive: true,
    } = &choices[0].kind
    else {
        panic!()
    };
    assert!(matches!(expression.kind, ExprKind::Repeat { .. }));
    let ExprKind::Capture { name, expression } = &choices[1].kind else {
        panic!()
    };
    assert_eq!(name, "value");
    assert!(matches!(expression.kind, ExprKind::Repeat { .. }));
    let ExprKind::Sequence(sequence) = &choices[2].kind else {
        panic!()
    };
    assert_eq!(sequence.len(), 2);
    assert!(matches!(
        sequence[0].kind,
        ExprKind::Lookahead {
            positive: false,
            ..
        }
    ));
    assert!(matches!(&sequence[1].kind, ExprKind::Reference { name, .. } if name == "NL"));
}

#[test]
fn predicate_boundaries_preserve_following_lookahead_and_optional_guards() {
    let grammar = parse("syntax A -> @{ ok(ctx) } !ANY (@{ ready })? @{ value != 3 }").unwrap();
    let ExprKind::Sequence(expressions) = &grammar.rules[0].expression.kind else {
        panic!()
    };
    assert_eq!(expressions.len(), 4);
    assert!(matches!(&expressions[0].kind, ExprKind::Predicate(p) if p == "ok(ctx)"));
    assert!(matches!(
        expressions[1].kind,
        ExprKind::Lookahead {
            positive: false,
            ..
        }
    ));
    assert!(matches!(expressions[2].kind, ExprKind::Repeat { .. }));
    assert!(matches!(&expressions[3].kind, ExprKind::Predicate(p) if p == "value != 3"));
}

#[test]
fn retains_arguments_predicates_scanners_and_boundaries() {
    let source = "import scanner Extent(ctx) from `parser.rs`;\nsyntax Item(ctx) ->\n  let i = { column(cursor) }\n  @{ column(cursor) == i }\n  extent:probe Extent(i; ctx)\n  body:within(extent.end, ItemBody(ctx with indent=i, start=cursor))\n  tail:scan Tail(size(open); ctx)";
    let grammar = parse(source).unwrap();
    assert_eq!(grammar.imports[0].from, "parser.rs");
    let ExprKind::Sequence(sequence) = &grammar.rules[0].expression.kind else {
        panic!()
    };
    assert_eq!(sequence.len(), 5);
    assert!(matches!(&sequence[1].kind, ExprKind::Predicate(p) if p == "column(cursor) == i"));
    let svg = render_source(source, &RenderOptions::default()).unwrap();
    for label in [
        "binding · zero-width",
        "predicate · zero-width",
        "probe Extent(i; ctx)",
        "within extent.end",
        "ItemBody(ctx with indent=i, start=cursor)",
        "consume through result.end",
    ] {
        assert!(svg.contains(label), "missing {label}");
    }
}

#[test]
fn decisions_have_visible_semantic_annotations_and_local_links() {
    let svg = render_source(
        "syntax A -> B | `a`\nsyntax B -> `b` ^ ITEM* !ANY",
        &RenderOptions::default(),
    )
    .unwrap();
    for label in [
        "ordered choice · first match",
        "committed · failure",
        "Broken",
        "greedy",
        "negative lookahead · zero-width",
        "#grammar-42",
    ] {
        assert!(svg.contains(label), "missing {label}");
    }
    let one = render_source(
        "syntax A -> B\nsyntax B -> `b`",
        &RenderOptions {
            rule: Some("A".into()),
            theme: Theme::Rust,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(one.matches("<svg ").count(), 1);
    assert!(
        !one.contains("href="),
        "do not link to rules omitted by the filter"
    );
    assert!(
        render_source(
            "syntax A -> B",
            &RenderOptions {
                rule: Some("Missing".into()),
                ..Default::default()
            }
        )
        .unwrap_err()
        .message
        .contains("Missing")
    );
}

#[test]
fn literal_text_is_escaped_and_whitespace_is_visible() {
    let svg = render_source(
        r#"lex A -> `<script>&"` | `\n` | ` ` | [`0`-`9` U+0009]"#,
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(!svg.contains("<script>"));
    assert!(svg.contains("&lt;script&gt;"));
    assert!(svg.contains("\\n"));
    assert!(svg.contains("␠"));
    assert!(svg.contains("\\t"));
}

#[test]
fn repetitions_and_empty_matches_are_explicit() {
    let svg = render_source(
        "syntax A -> ITEM{2..4} | ITEM{n:2..=4} ITEM{n} | ITEM{0} | ITEM{1..} | ε",
        &RenderOptions::default(),
    )
    .unwrap();
    for label in [
        "total 2..4 times",
        "exactly n times",
        "0 times",
        "ε · zero-width",
    ] {
        assert!(svg.contains(label), "missing {label}");
    }
}

#[test]
fn reports_malformed_input_without_panicking() {
    for source in [
        "",
        "syntax A ->",
        "syntax A -> A |",
        "syntax A -> | A",
        "syntax A -> [`ab`]",
        "syntax A -> [`z`-`a`]",
        "syntax A -> ``",
        "syntax A -> `unclosed",
        "syntax A -> `line\nterminal`",
        "syntax A -> (A",
        "syntax A -> R([})",
        "syntax A -> A**",
        "syntax A -> (&A)*",
        "syntax A -> A{4..2}",
        "syntax A -> A{2..2}",
        "syntax A -> A{4..=2}",
        "syntax A -> A{2..=}",
        "syntax A -> A{n:3}",
        "syntax A -> A{4294967296}",
        "syntax A -> A{0..infinity}",
        "syntax A -> within(, A)",
        "syntax A -> let x = {}",
        "syntax A -> @{ }",
        "syntax A -> @{ ([)] }",
        "syntax A -> @{ /* unclosed }",
        "syntax A -> ~!A",
        "syntax A -> ~~A",
        "syntax A -> U+D800",
        "syntax A -> U+110000",
        "syntax A -> U+123",
        "syntax A -> A[^]",
        "syntax A -> A _unterminated",
        "syntax A -> A ^",
        "syntax A -> A ^? B",
        "import syntax A from ",
        "syntax A -> B\nsyntax A -> C",
        // The previous notation is deliberately not an alternate spelling.
        "syntax A ::= B",
        "syntax A -> A / B",
        "syntax A -> \"a\"",
        "syntax A -> @ready(ctx)",
        "syntax A -> let x = column(cursor)",
    ] {
        assert!(parse(source).is_err(), "accepted {source:?}");
    }
    for newline in ["\r", "\r\n", "\n"] {
        let error = parse(&format!("// 第一行{newline}syntax 字 -> U+D800")).unwrap_err();
        assert_eq!((error.line, error.column), (2, 13));
    }
}

#[test]
fn bounds_nesting_and_wraps_long_sequences() {
    let allowed = format!("syntax A -> {}ITEM{}", "(".repeat(63), ")".repeat(63));
    assert!(parse(&allowed).is_ok());
    let nested = format!("syntax A -> {}ITEM{}", "(".repeat(128), ")".repeat(128));
    assert!(parse(&nested).unwrap_err().message.contains("nesting"));
    let prefixes = format!("syntax A -> {}ITEM", "!".repeat(128));
    assert!(parse(&prefixes).unwrap_err().message.contains("nesting"));
    let large = format!("syntax A -> {}", "ITEM ".repeat(5000));
    assert!(
        parse(&large)
            .unwrap_err()
            .message
            .contains("expression limit")
    );
    let sequence = format!("syntax A -> {}", "ITEM ".repeat(30));
    let svg = render_source(
        &sequence,
        &RenderOptions {
            max_width: 300,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(svg.contains("class=\"stack\""));
}

#[test]
fn accepts_rust_reference_comment_notation_without_layer_or_semicolon() {
    let source = r#"
@root
COMMENT -> LINE_COMMENT | BLOCK_COMMENT

LINE_COMMENT -> `//` (~[`/` `!` LF] ~LF*)? | `//` _immediately followed by LF_
BLOCK_COMMENT -> `/*` ^ (BLOCK_COMMENT | ~`*/`)* `*/`
LF -> U+000A
"#;
    let grammar = parse(source).unwrap();
    assert_eq!(grammar.rules.len(), 4);
    assert!(grammar.rules[0].is_root);
    assert!(
        grammar
            .rules
            .iter()
            .all(|rule| rule.layer == notist_grammar::Layer::Syntax)
    );
    let svg = render_source(source, &RenderOptions::default()).unwrap();
    assert!(svg.contains("@root"));
    assert!(svg.contains("with the exception of"));
    assert!(svg.contains("immediately followed by LF"));
    assert!(svg.contains("U+000A"));
}

#[test]
fn repetition_intervals_preserve_open_closed_and_named_counts() {
    use notist_grammar::RangeLimit::{Closed, HalfOpen};
    for (suffix, expected_min, expected_max, expected_limit, expected_count) in [
        ("{2..4}", "2", Some("4"), HalfOpen, None),
        ("{2..=4}", "2", Some("4"), Closed, None),
        ("{..4}", "0", Some("4"), HalfOpen, None),
        ("{2..}", "2", None, HalfOpen, None),
        ("{..}", "0", None, HalfOpen, None),
        ("{..=4}", "0", Some("4"), Closed, None),
        ("{n:3..=255}", "3", Some("255"), Closed, Some("n")),
        ("{n}", "n", Some("n"), Closed, None),
        ("{3}", "3", Some("3"), Closed, None),
    ] {
        let grammar = parse(&format!("lex A -> `x`{suffix};")).unwrap();
        let ExprKind::Repeat {
            min,
            max,
            limit,
            count,
            ..
        } = &grammar.rules[0].expression.kind
        else {
            panic!()
        };
        assert_eq!(
            (min.as_str(), max.as_deref(), *limit, count.as_deref()),
            (expected_min, expected_max, expected_limit, expected_count)
        );
    }
    let svg = render_source(
        "lex A -> `x`{2..4} | `x`{2..=4} | `x`{n:3..=255} `x`{n};",
        &RenderOptions::default(),
    )
    .unwrap();
    for label in [
        "total 2..4 times",
        "total 2..=4 times",
        "bind repeat count n",
        "exactly n times",
    ] {
        assert!(svg.contains(label), "missing {label}");
    }
}

#[test]
fn raw_terminals_unicode_character_sets_prose_and_notes_remain_distinct() {
    let grammar = parse(r#"lex A -> `\n` U+0060 ~[`a`-`z` U+000A LF] <a documented input unit> _except `x_y`_ [^constraint];"#).unwrap();
    let ExprKind::Sequence(parts) = &grammar.rules[0].expression.kind else {
        panic!()
    };
    assert!(matches!(&parts[0].kind, ExprKind::Literal(text) if text == "\\n"));
    assert!(matches!(&parts[1].kind, ExprKind::Unicode { character: '`', hex } if hex == "0060"));
    assert!(matches!(&parts[2].kind, ExprKind::Complement(_)));
    let ExprKind::Annotated {
        expression,
        suffix,
        footnote,
    } = &parts[3].kind
    else {
        panic!()
    };
    assert!(matches!(&expression.kind, ExprKind::Prose(text) if text == "a documented input unit"));
    assert_eq!(suffix.as_deref(), Some("except `x_y`"));
    assert_eq!(footnote.as_deref(), Some("constraint"));
    let svg = render_source(
        r#"lex A -> ~[`a`-`z` LF] <input> _a note_ [^constraint];"#,
        &RenderOptions::default(),
    )
    .unwrap();
    for label in [
        "CHAR",
        "with the exception of",
        "a-z",
        "LF",
        "prose",
        "a note",
        "footnote constraint",
    ] {
        assert!(svg.contains(label), "missing {label}");
    }
}

#[test]
fn computations_have_explicit_boundaries_independent_of_line_breaks() {
    for gap in [" ", "\n", "\r\n"] {
        let source = format!(
            r#"syntax A -> @{{ x < 3 && y > 2 && z != "}}" }}{gap}!ANY{gap}let n = {{ (size(open) / 2) + values["}}"]; /* }} */ }}{gap}ITEM{{n}};"#
        );
        let grammar = parse(&source).unwrap();
        let ExprKind::Sequence(parts) = &grammar.rules[0].expression.kind else {
            panic!()
        };
        assert_eq!(parts.len(), 4);
        assert!(
            matches!(&parts[0].kind, ExprKind::Predicate(code) if code == r#"x < 3 && y > 2 && z != "}""#)
        );
        assert!(
            matches!(&parts[2].kind, ExprKind::Binding { name, value } if name == "n" && value.contains("size(open) / 2"))
        );
        assert!(matches!(&parts[3].kind, ExprKind::Repeat { min, .. } if min == "n"));
    }
}

#[test]
fn invocation_arguments_are_opaque_and_grouping_requires_separation() {
    let grammar =
        parse(r#"syntax A -> Rule(x < 3, "}", nested([1, 2]), /* ) */ ctx) Rule (B | C);"#)
            .unwrap();
    let ExprKind::Sequence(parts) = &grammar.rules[0].expression.kind else {
        panic!()
    };
    assert_eq!(parts.len(), 3);
    assert!(
        matches!(&parts[0].kind, ExprKind::Reference { arguments: Some(args), .. } if args.contains("x < 3") && args.contains("/* ) */"))
    );
    assert!(matches!(
        &parts[1].kind,
        ExprKind::Reference {
            arguments: None,
            ..
        }
    ));
    assert!(matches!(&parts[2].kind, ExprKind::OrderedChoice(_)));
    assert!(parse("syntax A -> R(x // )\n, y);").is_ok());
}

#[test]
fn construction_has_its_own_structure_spans_and_shared_sequence_nodes() {
    let source = "syntax Heading -> marker:EQ WS body:BODY => heading(level: size(marker))[lower(body) tail?];";
    let grammar = parse(source).unwrap();
    let rule = &grammar.rules[0];
    assert!(matches!(rule.expression.kind, ExprKind::Sequence(_)));
    let result = rule.result.as_ref().unwrap();
    assert_eq!(
        &source[result.span.clone()],
        "heading(level: size(marker))[lower(body) tail?]"
    );
    let ExprKind::Node {
        name,
        arguments,
        children,
    } = &result.kind
    else {
        panic!()
    };
    assert_eq!(name, "heading");
    assert_eq!(arguments.as_deref(), Some("level: size(marker)"));
    let ExprKind::Sequence(parts) = &children.kind else {
        panic!()
    };
    assert_eq!(parts.len(), 2);
    assert!(
        matches!(&parts[0].kind, ExprKind::Reference { name, arguments: Some(args) } if name == "lower" && args == "body")
    );
    assert!(matches!(parts[1].kind, ExprKind::Repeat { .. }));
    let svg = render_source(source, &RenderOptions::default()).unwrap();
    for label in [
        "capture marker",
        "=> result",
        "no input consumption",
        "construct heading",
        "optional result",
    ] {
        assert!(svg.contains(&label.replace('>', "&gt;")), "missing {label}");
    }
}

#[test]
fn result_conditions_stay_in_results_and_do_not_become_input_guards() {
    let source =
        "syntax P -> body:BODY => { if lower(body) != [] then paragraph()[lower(body)] else [] };";
    let grammar = parse(source).unwrap();
    assert!(matches!(
        grammar.rules[0].expression.kind,
        ExprKind::Capture { .. }
    ));
    assert!(
        matches!(&grammar.rules[0].result.as_ref().unwrap().kind, ExprKind::Computation(code) if code.starts_with("if lower(body)"))
    );
    let svg = render_source(source, &RenderOptions::default()).unwrap();
    assert!(svg.contains("pure computation · result only"));
    assert!(!svg.contains("predicate · zero-width"));
}

#[test]
fn ast_node_patterns_and_parameter_only_helpers_share_rules() {
    use notist_grammar::Layer;
    let grammar = parse("ast MD_Heading -> MD.Heading(level, body)[child:ANY*] => heading(level: level)[lower(body)];\nast helper(value) -> ε => wrapper()[value];").unwrap();
    assert!(grammar.rules.iter().all(|rule| rule.layer == Layer::Ast));
    assert!(
        matches!(&grammar.rules[0].expression.kind, ExprKind::Node { name, arguments: Some(args), .. } if name == "MD.Heading" && args == "level, body")
    );
    assert_eq!(grammar.rules[1].parameters.as_deref(), Some("value"));
    assert!(matches!(grammar.rules[1].expression.kind, ExprKind::Empty));
}

#[test]
fn result_groups_repetitions_and_computations_have_unambiguous_boundaries() {
    let grammar =
        parse("syntax A -> ITEM => outer()[(a b){2..=4} { choose(a, b) } inner()[]];").unwrap();
    let ExprKind::Node { children, .. } = &grammar.rules[0].result.as_ref().unwrap().kind else {
        panic!()
    };
    let ExprKind::Sequence(parts) = &children.kind else {
        panic!()
    };
    assert_eq!(parts.len(), 3);
    assert!(
        matches!(&parts[0].kind, ExprKind::Repeat { min, max, .. } if min == "2" && max.as_deref() == Some("4"))
    );
    assert!(matches!(&parts[1].kind, ExprKind::Computation(code) if code == "choose(a, b)"));
    assert!(
        matches!(&parts[2].kind, ExprKind::Node { children, .. } if matches!(children.kind, ExprKind::Empty))
    );
    let svg = render_source(
        "syntax A -> ITEM => outer()[item*];",
        &RenderOptions::default(),
    )
    .unwrap();
    assert!(svg.contains("repeated result"));
    assert!(!svg.contains("greedy"));
}

#[test]
fn malformed_results_and_matching_operations_in_results_are_rejected() {
    for source in [
        "syntax A -> A =>",
        "syntax A -> A => {}",
        "syntax A -> A => node()[",
        "syntax A -> A => node()[B)",
        "syntax A -> A => !B",
        "syntax A -> A => &B",
        "syntax A -> A => @{ ready }",
        "syntax A -> A => ^ B",
        "syntax A -> A => scan Tail(ctx)",
        "syntax A -> A => let n = { 3 }",
        "syntax A -> A => body:B",
        "syntax A -> A => B => C",
        "syntax A -> => B",
    ] {
        assert!(parse(source).is_err(), "accepted {source:?}");
    }
    let deep = format!(
        "syntax A -> ITEM => {}ε{};",
        "node()[".repeat(128),
        "]".repeat(128)
    );
    assert!(parse(&deep).unwrap_err().message.contains("nesting"));
}
