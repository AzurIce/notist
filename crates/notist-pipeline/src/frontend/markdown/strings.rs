use rushdown::util::{resolve_entity_references, resolve_numeric_references};

/// Decode a Markdown field once, preserving escaped ampersands and avoiding
/// decoding references introduced by an earlier replacement.
pub(crate) fn decode(src: &str) -> String {
    let mut out = String::new();
    let mut chars = src.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch == '\\' {
            if let Some(&(_, next)) = chars.peek() {
                if next.is_ascii_punctuation() {
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
        } else if ch == '&' {
            let len = src[start + 1..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'#')
                .count();
            let end = start + 1 + len;
            if src.as_bytes().get(end) == Some(&b';') {
                let reference = &src.as_bytes()[start..=end];
                let decoded = resolve_entity_references(resolve_numeric_references(reference));
                out.push_str(&String::from_utf8_lossy(&decoded));
                for _ in 0..=len {
                    chars.next();
                }
                continue;
            }
        }
        out.push(ch);
    }
    out
}
