use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use std::path::{Component, Components, Path};

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
    let mut output = String::with_capacity(path.as_os_str().len());
    append_components(&mut output, path.components());
    output
}

fn append_components<'a>(output: &mut String, components: impl Iterator<Item = Component<'a>>) {
    for (index, part) in components.enumerate() {
        if index > 0 {
            output.push('/');
        }
        let part = part.as_os_str().to_string_lossy();
        for encoded in utf8_percent_encode(&part, SEGMENT) {
            output.push_str(encoded);
        }
    }
}

fn append_relative_path(output: &mut String, components: Components<'_>) {
    // Windows PathBuf::push can reinterpret drive prefixes, verbatim paths,
    // embedded forward slashes and dot components. Keep that behavior for
    // these uncommon suffixes rather than changing their encoded bytes.
    #[cfg(windows)]
    if components.clone().any(|part| match part {
        Component::Prefix(_) | Component::CurDir => true,
        Component::Normal(part) => part
            .as_encoded_bytes()
            .iter()
            .any(|b| matches!(b, b':' | b'/')),
        _ => false,
    }) {
        let path: std::path::PathBuf = components.collect();
        append_components(output, path.components());
        return;
    }
    append_components(output, components);
}
pub fn fragment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// Relative URL from one output directory to a published file or directory.
pub fn relative_url(current_page: &Path, target: &Path, directory: bool) -> String {
    let mut from = current_page.parent().unwrap_or(Path::new("")).components();
    let mut to = target.components();
    while let (Some(a), Some(b)) = (from.clone().next(), to.clone().next()) {
        if a != b {
            break;
        }
        from.next();
        to.next();
    }
    let parents = from.count();
    let capacity = parents * 3 + to.as_path().as_os_str().len() + usize::from(directory);
    let mut result = String::with_capacity(capacity.max(2));
    for _ in 0..parents {
        result.push_str("../");
    }
    append_relative_path(&mut result, to);
    if directory && !result.ends_with('/') {
        result.push('/');
    }
    if result.is_empty() || result == "/" {
        result.clear();
        result.push_str("./");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn collected_encode_path(path: &Path) -> String {
        path.iter()
            .map(|part| utf8_percent_encode(&part.to_string_lossy(), SEGMENT).to_string())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn collected_relative_url(current: &Path, target: &Path, directory: bool) -> String {
        let from: Vec<_> = current
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .collect();
        let to: Vec<_> = target.components().collect();
        let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
        let mut result = "../".repeat(from.len() - common);
        let rest: PathBuf = to[common..].iter().collect();
        result.push_str(&collected_encode_path(&rest));
        if directory && !result.ends_with('/') {
            result.push('/');
        }
        if result.is_empty() || result == "/" {
            "./".into()
        } else {
            result
        }
    }

    fn check_paths(paths: &[&str]) {
        for target in paths.iter().map(Path::new) {
            assert_eq!(
                encode_path(target),
                collected_encode_path(target),
                "{target:?}"
            );
            for current in paths.iter().map(Path::new) {
                for directory in [false, true] {
                    assert_eq!(
                        relative_url(current, target, directory),
                        collected_relative_url(current, target, directory),
                        "current={current:?}, target={target:?}, directory={directory}"
                    );
                }
            }
        }
    }

    #[test]
    fn paths_and_relative_urls_match_collected_reference() {
        check_paths(&[
            "",
            ".",
            "./",
            "..",
            "../",
            "/",
            "//",
            "/a",
            "/a/",
            "/a/index.html",
            "a",
            "a/",
            "a/index.html",
            "a/b/index.html",
            "a/./b",
            "./a/b",
            "a//b/",
            "a/../b",
            "../a/b",
            "../../a",
            "a/./../b/./",
            "ab/index.html",
            "a/b/..",
            "中文/🙂.html",
            "a b/100%?query#fragment.html",
            "\0/\t/\r/\n/\u{7f}",
            "C:/a/index.html",
            "C:\\a\\index.html",
            "a/b:c",
            "a/b\\c",
            "a/%2F",
            "a/<>\"`",
        ]);
    }

    #[test]
    fn common_prefixes_empty_paths_and_directory_slashes() {
        for (current, target, directory, expected) in [
            ("", "", false, "./"),
            ("", "", true, "./"),
            ("index.html", "", false, "./"),
            ("a/index.html", "a", true, "./"),
            ("a/b/index.html", "a", true, "../"),
            ("a/index.html", "a/other.html", false, "other.html"),
            ("a/b/index.html", "a/c/", true, "../c/"),
            ("a/b/index.html", "ab/c.html", false, "../../ab/c.html"),
        ] {
            assert_eq!(
                relative_url(Path::new(current), Path::new(target), directory),
                expected
            );
        }
        assert_eq!(
            encode_path(Path::new("中文/100% ?#<>\"`:🙂")),
            "%E4%B8%AD%E6%96%87/100%25%20%3F%23%3C%3E%22%60%3A%F0%9F%99%82"
        );
        assert_eq!(fragment("/%文"), "%2F%25%E6%96%87");
    }

    #[test]
    fn component_combinations_match_collected_reference() {
        let parts = ["a", ".", "..", "", "中文", "%2F", "b:c", "b\\c"];
        for a in parts {
            for b in parts {
                for c in parts {
                    let target = PathBuf::from(format!("{a}/{b}/{c}"));
                    assert_eq!(encode_path(&target), collected_encode_path(&target));
                    for current in [
                        "",
                        "a/index.html",
                        "a/b/index.html",
                        "./a/index.html",
                        "/a/b",
                    ] {
                        for directory in [false, true] {
                            assert_eq!(
                                relative_url(Path::new(current), &target, directory),
                                collected_relative_url(Path::new(current), &target, directory),
                                "current={current:?}, target={target:?}, directory={directory}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_lossy_replacement_bytes() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let target = PathBuf::from(OsString::from_vec(b"a/\xff\xc0%/\0".to_vec()));
        assert_eq!(encode_path(&target), "a/%EF%BF%BD%EF%BF%BD%25/%00");
        for current in ["a/index.html", "/a/b/index.html"] {
            for directory in [false, true] {
                assert_eq!(
                    relative_url(Path::new(current), &target, directory),
                    collected_relative_url(Path::new(current), &target, directory)
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefixes_and_verbatim_components_keep_pathbuf_semantics() {
        check_paths(&[
            "",
            ".",
            "a\\index.html",
            "a\\C:other",
            "C:",
            "C:relative",
            "C:\\a\\index.html",
            "D:\\a\\index.html",
            "\\a\\index.html",
            "\\\\server\\share\\a\\index.html",
            "\\\\?\\C:\\a\\.\\b\\..\\index.html",
            "\\\\?\\C:\\a/b\\index.html",
            "\\\\?\\UNC\\server\\share\\a\\..\\index.html",
            "\\\\.\\COM1",
        ]);
    }
}
