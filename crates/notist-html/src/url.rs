/// Whether a URL can be emitted by the default link renderer.
///
/// Relative paths, fragments and HTTP(S), mailto, tel and FTP URLs are
/// supported. Other schemes (including javascript and data) are omitted.
/// This does not check resource existence or rewrite document extensions.
pub fn is_safe_url(url: &str) -> bool {
    // Browsers remove ASCII tabs/newlines before interpreting the scheme
    // and trim leading C0 controls and spaces.
    // The removed bytes are all C0 controls, so trimming first is equivalent.
    // They cannot change the path delimiters or colon; only the scheme needs
    // filtering, and it can be compared without materializing a normalized URL.
    let url = url.trim_start_matches(|c: char| c <= '\u{20}');
    let prefix = url.split(['/', '\\', '?', '#']).next().unwrap_or("");
    match prefix.split_once(':') {
        None => true,
        Some((scheme, _)) => ["http", "https", "mailto", "tel", "ftp"]
            .iter()
            .any(|allowed| {
                scheme
                    .bytes()
                    .filter(|c| !matches!(c, b'\t' | b'\n' | b'\r'))
                    .map(|c| c.to_ascii_lowercase())
                    .eq(allowed.bytes())
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::is_safe_url;

    fn normalized_is_safe_url(url: &str) -> bool {
        let normalized: String = url
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .collect();
        let normalized = normalized.trim_start_matches(|c: char| c <= '\u{20}');
        let prefix = normalized.split(['/', '\\', '?', '#']).next().unwrap_or("");
        match prefix.split_once(':') {
            None => true,
            Some((scheme, _)) => ["http", "https", "mailto", "tel", "ftp"]
                .iter()
                .any(|allowed| scheme.eq_ignore_ascii_case(allowed)),
        }
    }

    #[test]
    fn schemes_keep_browser_control_normalization() {
        for (url, safe) in [
            ("", true),
            ("\0\u{1f} \t\r\nHTTPS://example.com/中文", true),
            ("h\tt\nt\rpS://example.com", true),
            ("MaIlTo:user@example.com", true),
            ("t\re\nl:+123", true),
            ("f\ttp://example.com", true),
            ("//example.com/a:b", true),
            ("relative/文:字", true),
            ("\\javascript:alert(1)", true),
            ("?javascript:alert(1)", true),
            ("#javascript:alert(1)", true),
            ("javascript:alert(1)", false),
            ("\0 \tJaVa\nScRi\rPt:alert(1)", false),
            ("d\ta\nt\ra:text/html,unsafe", false),
            ("vbscript:unsafe", false),
            (":empty", false),
            ("http\0:example.com", false),
            ("http :example.com", false),
            ("\u{a0}https://example.com", false),
            ("文https://example.com", false),
        ] {
            assert_eq!(is_safe_url(url), safe, "{url:?}");
        }
    }

    #[test]
    fn inserted_controls_match_normalized_reference() {
        for scheme in [
            "http",
            "HTTPS",
            "mailto",
            "tel",
            "ftp",
            "javascript",
            "data",
            "",
        ] {
            let url = format!("{scheme}:example.com/中文?query#fragment");
            for control in (0..=32).map(char::from).chain(['\u{7f}', '\u{a0}', '文']) {
                for position in 0..=scheme.len() + 1 {
                    let candidate =
                        format!("\t\u{1f} {}{control}{}", &url[..position], &url[position..]);
                    assert_eq!(
                        is_safe_url(&candidate),
                        normalized_is_safe_url(&candidate),
                        "{candidate:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn short_delimiter_combinations_match_normalized_reference() {
        let alphabet = [
            "h", ":", "/", "\\", "?", "#", "\t", "\n", "\r", "\0", " ", "文",
        ];
        for a in alphabet {
            for b in alphabet {
                for c in alphabet {
                    for d in alphabet {
                        let candidate = format!("{a}{b}{c}{d}");
                        assert_eq!(
                            is_safe_url(&candidate),
                            normalized_is_safe_url(&candidate),
                            "{candidate:?}"
                        );
                    }
                }
            }
        }
    }
}
