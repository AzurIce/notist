#![cfg(feature = "highlight")]

use notist_html::highlight;

// Inspect the text represented by the generated spans, rather than depending
// on a grammar's exact span boundaries or nesting.
fn text(html: &str) -> String {
    let mut inside = false;
    html.chars()
        .filter(|c| match c {
            '<' => {
                inside = true;
                false
            }
            '>' => {
                inside = false;
                false
            }
            _ => !inside,
        })
        .collect::<String>()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[test]
fn common_languages_and_aliases_preserve_exact_code_text() {
    for (languages, source) in [
        (
            &[
                "rust",
                "rs",
                "Rust flags",
                "rust,ignore",
                "rust,no_run",
                "rust,ignore,no_run",
                " Rust, ignore",
            ][..],
            "fn main() { let x = 1 < 2; }\n",
        ),
        (&["typescript", "ts"], "const name: string = \"你好\";\n"),
        (&["tsx"], "const view = <div title=\"你好\">Hello</div>;\n"),
        (
            &["javascript", "js"],
            "const text = '<script>&lt;</script>';\n",
        ),
        (&["json"], "{\"name\": \"你好\", \"ok\": true}\n"),
        (&["html"], "<div title=\"你好\">Hello &amp; goodbye</div>\n"),
        (&["markdown", "md"], "# Heading\n\n**Bold** and `code`\n"),
        (&["toml"], "[book]\ntitle = \"你好\"\n"),
        (&["bash", "sh", "shell"], "echo \"<script>&\"\n"),
        (&["yaml", "yml"], "name: hello\n"),
        (&["dockerfile", "docker"], "FROM alpine:3\nRUN echo hi\n"),
    ] {
        for language in languages {
            for source in [source, source.trim_end_matches('\n')] {
                let html = highlight::highlight_str(language, source)
                    .unwrap_or_else(|| panic!("missing language: {language}"));
                assert!(html.contains("<span class="), "{language}: {html}");
                assert_eq!(text(&html), source, "{language}: {html}");
                assert!(!html.contains("<script>"));
                assert!(!html.contains("style="), "palette must remain CSS-driven");
            }
        }
    }
}

#[test]
fn plain_and_unknown_languages_fall_back_to_none() {
    for language in [
        "",
        " ",
        "text",
        "text,ignore",
        "txt",
        "plaintext",
        "plain",
        "none",
        "nohighlight",
        "unknown-language",
    ] {
        assert!(
            highlight::highlight_str(language, "<script> &\n").is_none(),
            "{language}"
        );
    }
}

#[test]
fn native_snippets_preserve_exact_bytes_and_highlight_scopes() {
    for (language, source, scopes) in [
        (
            "not",
            "= 标题\r\n\r\n*粗体* #missing(label: \"<script>\", n: 2)\r\n// 注释\r\n",
            &["z-heading", "z-bold", "z-string", "z-numeric", "z-comment"][..],
        ),
        (
            "notc",
            "import \"missing.notc\" as demo;\nfn badge(label: String) -> Inline { [你好 #demo.f(label)] }\n",
            &["z-keyword", "z-type", "z-function"],
        ),
        (
            "notc",
            "fn unfinished(label: String = r#\"<script>&\"#",
            &["z-keyword", "z-string"],
        ),
        ("not", "#call(\"未完成\"\n", &["z-string"]),
        (
            "notc",
            "fn source(p: String) -> String { read_text(\"./missing.json\") }",
            &["z-type", "z-string"],
        ),
    ] {
        let html = highlight::highlight_str(language, source).unwrap();
        assert_eq!(text(&html), source, "{language}: {html}");
        for scope in scopes {
            assert!(html.contains(scope), "missing {scope}: {html}");
        }
        assert!(!html.contains("<script>"));
    }
}

#[test]
fn native_raw_payloads_do_not_become_calls_or_comments() {
    let source = "```text\n#not_a_call(1) // not a comment\n```\n";
    let html = highlight::highlight_str("not", source).unwrap();
    assert_eq!(text(&html), source);
    assert!(html.contains("z-raw"));
    assert!(!html.contains("z-function"));
    assert!(!html.contains("z-comment"));
}

#[test]
fn code_content_literals_keep_native_markup_highlighting() {
    let source = "fn content() -> Inline { [*你好* #missing(\"world\")] }\n";
    let html = highlight::highlight_str("notc", source).unwrap();
    assert_eq!(text(&html), source);
    assert!(html.contains("z-markup z-bold"), "{html}");
    assert!(html.contains("class=\"z-string\">\"world\""), "{html}");
    assert!(html.contains("class=\"z-keyword\">#</span>"), "{html}");
}
