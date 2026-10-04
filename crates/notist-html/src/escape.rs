/// Escape a string for HTML text content. The input is the IR's decoded value.
pub fn escape_text(text: &str) -> String {
    let mut output = String::new();
    write(&mut output, text, false);
    output
}

/// Escape a string for a quoted HTML attribute value.
pub fn escape_attribute(text: &str) -> String {
    let mut output = String::new();
    write(&mut output, text, true);
    output
}

pub(crate) fn write(output: &mut String, text: &str, attribute: bool) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            '\'' if attribute => output.push_str("&#39;"),
            '\r' if attribute => output.push_str("&#13;"),
            '\0' => output.push('\u{fffd}'),
            _ => output.push(ch),
        }
    }
}
