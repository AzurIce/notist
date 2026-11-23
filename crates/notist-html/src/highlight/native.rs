//! Lossless lexical syntax highlighter for `.not` and `.notc` code snippets.
//! Preserves every input byte (including incomplete or malformed snippets)
//! without evaluation, imports, or external parser dependencies.
use crate::escape_text;

pub(super) fn render(source: &str, module: bool) -> String {
    let mut html = String::from(if module {
        "<span class=\"z-source z-notc\">"
    } else {
        "<span class=\"z-source z-not\">"
    });
    if module {
        highlight_code(source, &mut html);
    } else {
        highlight_markup(source, &mut html);
    }
    html.push_str("</span>");
    html
}

fn emit(html: &mut String, class: Option<&str>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(class) = class {
        html.push_str("<span class=\"");
        html.push_str(class);
        html.push_str("\">");
        html.push_str(&escape_text(text));
        html.push_str("</span>");
    } else {
        html.push_str(&escape_text(text));
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "fn" | "import"
            | "as"
            | "let"
            | "if"
            | "else"
            | "match"
            | "for"
            | "in"
            | "where"
            | "return"
    )
}

fn is_builtin_type(ident: &str) -> bool {
    matches!(
        ident,
        "Int"
            | "Float"
            | "Bool"
            | "String"
            | "Inline"
            | "Block"
            | "Json"
            | "List"
            | "Type"
            | "Never"
            | "Unit"
    )
}

fn scan_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else if bytes[i] == b'"' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn scan_raw_string(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }
    let mut i = start + 1;
    let mut hashes = 0usize;
    while i < bytes.len() && bytes[i] == b'#' {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut k = 0;
            while k < hashes && i + 1 + k < bytes.len() && bytes[i + 1 + k] == b'#' {
                k += 1;
            }
            if k == hashes {
                return Some(i + 1 + hashes);
            }
        }
        i += 1;
    }
    Some(bytes.len())
}

fn scan_balanced_bracket(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'`' => {
                let ticks = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                i += ticks;
                while i < bytes.len() {
                    if bytes[i] == b'`' {
                        let close = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                        i += close;
                        if close == ticks {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'"' => i = scan_string(bytes, i),
            b'[' => {
                depth += 1;
                i += 1;
            }
            b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

fn highlight_code(source: &str, html: &mut String) {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut prev_kw: Option<&str> = None;
    let mut expect_type = false;

    while i < bytes.len() {
        let b = bytes[i];
        // Line comment
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
            }
            emit(html, Some("z-comment"), &source[start..i]);
            continue;
        }
        // Block comment
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            emit(html, Some("z-comment"), &source[start..i]);
            continue;
        }
        // Raw string r#"..."#
        if let Some(end) = scan_raw_string(bytes, i) {
            emit(html, Some("z-string"), &source[i..end]);
            i = end;
            prev_kw = None;
            expect_type = false;
            continue;
        }
        // String literal
        if b == b'"' {
            let end = scan_string(bytes, i);
            emit(html, Some("z-string"), &source[i..end]);
            i = end;
            prev_kw = None;
            expect_type = false;
            continue;
        }
        // Content literal [...]
        if b == b'[' {
            let end = scan_balanced_bracket(bytes, i);
            emit(html, None, "[");
            if end > i + 1 {
                let inner_end = if bytes[end - 1] == b']' { end - 1 } else { end };
                highlight_markup(&source[i + 1..inner_end], html);
                if inner_end < end {
                    emit(html, None, "]");
                }
            }
            i = end;
            prev_kw = None;
            expect_type = false;
            continue;
        }
        // Number literal
        if b.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            emit(html, Some("z-constant z-numeric"), &source[start..i]);
            prev_kw = None;
            expect_type = false;
            continue;
        }
        // Identifier / keyword / type / function
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_cont(bytes[i]) {
                i += 1;
            }
            let ident = &source[start..i];
            let next_non_ws = bytes[i..].iter().copied().find(|c| !c.is_ascii_whitespace());
            let class = if is_keyword(ident) {
                prev_kw = Some(ident);
                expect_type = false;
                Some("z-keyword")
            } else if matches!(ident, "true" | "false") {
                prev_kw = None;
                expect_type = false;
                Some("z-constant z-language")
            } else if prev_kw == Some("fn") {
                prev_kw = None;
                expect_type = false;
                if bytes.get(i) == Some(&b'.') {
                    Some("z-storage z-type")
                } else {
                    Some("z-entity z-name z-function")
                }
            } else if expect_type || is_builtin_type(ident) {
                prev_kw = None;
                expect_type = false;
                Some("z-storage z-type")
            } else if matches!(next_non_ws, Some(b'(' | b'[')) {
                prev_kw = None;
                Some("z-entity z-name z-function")
            } else {
                prev_kw = None;
                None
            };
            emit(html, class, ident);
            continue;
        }
        // Two-char operators
        if i + 1 < bytes.len() {
            let pair = &source[i..i + 2];
            if matches!(
                pair,
                "->" | "=>" | "==" | "!=" | "<=" | ">=" | "&&" | "||"
            ) {
                if pair == "->" {
                    expect_type = true;
                }
                emit(html, Some("z-keyword z-operator"), pair);
                i += 2;
                continue;
            }
        }
        // Single-char punctuation / operators
        if b == b':' {
            expect_type = true;
            emit(html, None, ":");
            i += 1;
            continue;
        }
        if matches!(b, b'@' | b'#') {
            emit(html, Some("z-keyword"), &source[i..i + 1]);
            i += 1;
            continue;
        }
        if matches!(b, b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'!') {
            emit(html, Some("z-keyword z-operator"), &source[i..i + 1]);
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() && !matches!(b, b'<' | b',' | b'.') {
            expect_type = false;
        }
        // Copy one UTF-8 char
        let ch_len = source[i..].chars().next().map_or(1, char::len_utf8);
        emit(html, None, &source[i..i + ch_len]);
        i += ch_len;
    }
}

fn highlight_markup(source: &str, html: &mut String) {
    let mut offset = 0usize;
    let mut fence: Option<(u8, usize)> = None;

    while offset < source.len() {
        let rest = &source[offset..];
        let line_len = match rest.find('\n') {
            Some(pos) => pos + 1,
            None => rest.len(),
        };
        let line = &rest[..line_len];
        offset += line_len;

        let trimmed_start = line.trim_start_matches([' ', '\t']);
        if let Some((fch, flen)) = fence {
            emit(html, Some("z-markup z-raw"), line);
            let trimmed_end = trimmed_start.trim_end_matches(['\r', '\n', ' ', '\t']);
            let count = trimmed_end.bytes().take_while(|&b| b == fch).count();
            if count >= flen && count == trimmed_end.len() {
                fence = None;
            }
            continue;
        }
        // Opening fenced code block
        if let Some(&first) = trimmed_start.as_bytes().first()
            && matches!(first, b'`' | b'~')
        {
            let count = trimmed_start.bytes().take_while(|&b| b == first).count();
            if count >= 3 {
                fence = Some((first, count));
                emit(html, Some("z-markup z-raw"), line);
                continue;
            }
        }
        // Line comment in markup
        if trimmed_start.starts_with("//") {
            let lead = line.len() - trimmed_start.len();
            emit(html, None, &line[..lead]);
            let body_end = line.trim_end_matches(['\r', '\n']).len();
            emit(html, Some("z-comment"), &line[lead..body_end]);
            emit(html, None, &line[body_end..]);
            continue;
        }
        // Heading (= ...)
        if trimmed_start.starts_with('=') {
            let eqs = trimmed_start.bytes().take_while(|&b| b == b'=').count();
            if eqs <= 6
                && trimmed_start
                    .as_bytes()
                    .get(eqs)
                    .is_some_and(|&b| b == b' ' || b == b'\t')
            {
                let body_end = line.trim_end_matches(['\r', '\n']).len();
                emit(html, Some("z-markup z-heading"), &line[..body_end]);
                emit(html, None, &line[body_end..]);
                continue;
            }
        }
        // Leading #import line
        if trimmed_start.starts_with("#import")
            && trimmed_start
                .as_bytes()
                .get(7)
                .is_none_or(|b| !is_ident_cont(*b))
        {
            let lead = line.len() - trimmed_start.len();
            emit(html, None, &line[..lead]);
            highlight_code(&line[lead..], html);
            continue;
        }

        highlight_inline_markup(line, html);
    }
}

fn highlight_inline_markup(line: &str, html: &mut String) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // Escape sequence
        if b == b'\\' && i + 1 < bytes.len() {
            let next_len = line[i + 1..].chars().next().map_or(1, char::len_utf8);
            emit(
                html,
                Some("z-constant z-character z-escape"),
                &line[i..i + 1 + next_len],
            );
            i += 1 + next_len;
            continue;
        }
        // Inline raw `...`
        if b == b'`' {
            let ticks = bytes[i..].iter().take_while(|&&c| c == b'`').count();
            let start = i;
            i += ticks;
            while i < bytes.len() {
                if bytes[i] == b'`' {
                    let close = bytes[i..].iter().take_while(|&&c| c == b'`').count();
                    i += close;
                    if close == ticks {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            emit(html, Some("z-markup z-raw"), &line[start..i]);
            continue;
        }
        // Inline math $...$
        if b == b'$' {
            let start = i;
            if let Some(rel_end) = line[i + 1..].find('$') {
                i = i + 1 + rel_end + 1;
                emit(html, Some("z-markup z-raw"), &line[start..i]);
                continue;
            }
        }
        // Attribute annotation @(...) or @!(...)
        if b == b'@'
            && (bytes.get(i + 1) == Some(&b'(')
                || (bytes.get(i + 1) == Some(&b'!') && bytes.get(i + 2) == Some(&b'(')))
        {
            let prefix_len = if bytes.get(i + 1) == Some(&b'!') { 2 } else { 1 };
            let open = i + prefix_len;
            let end = scan_balanced_paren(bytes, open);
            emit(html, Some("z-keyword"), &line[i..open]);
            highlight_code(&line[open..end], html);
            i = end;
            continue;
        }
        // Code interpolation #ident...(...)
        if b == b'#' && bytes.get(i + 1).is_some_and(|&c| is_ident_start(c)) {
            emit(html, Some("z-keyword"), "#");
            i += 1;
            let start = i;
            while i < bytes.len() && (is_ident_cont(bytes[i]) || bytes[i] == b'.') {
                i += 1;
            }
            emit(html, Some("z-entity z-name z-function"), &line[start..i]);
            if bytes.get(i) == Some(&b'(') {
                let end = scan_balanced_paren(bytes, i);
                highlight_code(&line[i..end], html);
                i = end;
            }
            continue;
        }
        // Strikethrough ~~...~~
        if b == b'~'
            && bytes.get(i + 1) == Some(&b'~')
            && let Some(rel_end) = line[i + 2..].find("~~")
        {
            let end = i + 2 + rel_end + 2;
            emit(html, Some("z-markup z-strikethrough"), &line[i..end]);
            i = end;
            continue;
        }
        // Bold **...** or *...*
        if b == b'*' {
            let delim = if bytes.get(i + 1) == Some(&b'*') { "**" } else { "*" };
            if let Some(rel_end) = line[i + delim.len()..].find(delim)
                && rel_end > 0
            {
                let end = i + delim.len() + rel_end + delim.len();
                emit(html, Some("z-markup z-bold"), &line[i..end]);
                i = end;
                continue;
            }
        }
        // Italic _..._
        if b == b'_'
            && let Some(rel_end) = line[i + 1..].find('_')
            && rel_end > 0
        {
            let end = i + 1 + rel_end + 1;
            emit(html, Some("z-markup z-italic"), &line[i..end]);
            i = end;
            continue;
        }
        // Link [label](url) or image ![alt](url)
        if b == b'[' || (b == b'!' && bytes.get(i + 1) == Some(&b'[')) {
            let start = i;
            let bracket_start = if b == b'!' { i + 1 } else { i };
            if let Some(rel_close) = line[bracket_start + 1..].find("](") {
                let paren_open = bracket_start + 1 + rel_close + 1;
                let end = scan_balanced_paren(bytes, paren_open);
                emit(html, Some("z-markup z-underline z-link"), &line[start..end]);
                i = end;
                continue;
            }
        }
        let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
        emit(html, None, &line[i..i + ch_len]);
        i += ch_len;
    }
}

fn scan_balanced_paren(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = scan_string(bytes, i),
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}
