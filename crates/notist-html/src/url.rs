/// Whether a URL can be emitted by the default link renderer.
///
/// Relative paths, fragments and HTTP(S), mailto, tel and FTP URLs are
/// supported. Other schemes (including javascript and data) are omitted.
/// This does not check resource existence or rewrite document extensions.
pub fn is_safe_url(url: &str) -> bool {
    // Browsers remove ASCII tabs/newlines before interpreting the scheme
    // and trim leading C0 controls and spaces.
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
