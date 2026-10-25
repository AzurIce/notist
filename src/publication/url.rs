use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use std::path::{Path, PathBuf};

const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'%')
    .add(b'/')
    .add(b'\\')
    .add(b'?')
    .add(b'#')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b':');
pub fn encode_path(path: &Path) -> String {
    path.iter()
        .map(|part| utf8_percent_encode(&part.to_string_lossy(), SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}
pub fn fragment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// Relative URL from one output directory to a published file or directory.
pub fn relative_url(current_page: &Path, target: &Path, directory: bool) -> String {
    let from: Vec<_> = current_page
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let to: Vec<_> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut result = "../".repeat(from.len() - common);
    let rest: PathBuf = to[common..].iter().collect();
    result.push_str(&encode_path(&rest));
    if directory && !result.ends_with('/') {
        result.push('/');
    }
    if result.is_empty() || result == "/" {
        "./".into()
    } else {
        result
    }
}
