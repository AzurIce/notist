use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::Serialize;
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

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
pub(crate) fn fragment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub source: PathBuf,
    pub url: String,
    pub output: PathBuf,
}
/// Complete source identities and their canonical directory URLs.
#[derive(Debug, Clone, Default)]
pub struct Routes {
    entries: BTreeMap<PathBuf, Route>,
}
impl Routes {
    pub fn new(root: &Path, sources: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let mut entries = BTreeMap::new();
        let mut outputs = BTreeMap::<PathBuf, PathBuf>::new();
        for source in sources {
            let relative = source
                .strip_prefix(root)
                .map_err(|_| format!("page is outside Vault: {}", source.display()))?;
            if relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err("invalid page path".into());
            }
            let mut directory = relative.with_extension("");
            if relative.file_stem().is_some_and(|stem| stem == "README") {
                directory = relative.parent().unwrap().into();
            }
            if directory.starts_with("_notist") {
                return Err("_notist is reserved for generated site assets".into());
            }
            let output = directory.join("index.html");
            if let Some(previous) = outputs.insert(output.clone(), source.clone()) {
                return Err(format!(
                    "route collision: `{}` and `{}` both map to `/{}index.html`",
                    previous.display(),
                    source.display(),
                    if directory.as_os_str().is_empty() {
                        String::new()
                    } else {
                        format!("{}/", encode_path(&directory))
                    }
                ));
            }
            let url = if directory.as_os_str().is_empty() {
                "/".into()
            } else {
                format!("/{}/", encode_path(&directory))
            };
            entries.insert(
                source.clone(),
                Route {
                    source,
                    url,
                    output,
                },
            );
        }
        Ok(Self { entries })
    }
    pub fn get(&self, source: &Path) -> Option<&Route> {
        self.entries.get(source)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Route> {
        self.entries.values()
    }
}

/// Relative URL from one output directory to a published file or directory.
pub(crate) fn relative_url(current_page: &Path, target: &Path, directory: bool) -> String {
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

pub(crate) enum Target {
    External,
    Local {
        path: PathBuf,
        query: String,
        fragment: Option<String>,
    },
}
pub(crate) fn target(document: &Path, url: &str) -> Result<Target, String> {
    let prefix = url.split(['/', '\\', '?', '#']).next().unwrap_or("");
    if url.starts_with("//") || prefix.contains(':') {
        return Ok(Target::External);
    }
    let (before_fragment, anchor) = url
        .split_once('#')
        .map_or((url, None), |(path, anchor)| (path, Some(anchor)));
    let (path, query) = before_fragment
        .split_once('?')
        .map_or((before_fragment, ""), |(path, query)| (path, query));
    let path = percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| "invalid UTF-8 in URL path")?;
    if path.contains('\\') || path.contains('\0') {
        return Err("invalid local URL path".into());
    }
    let path = if path.is_empty() {
        document.into()
    } else {
        notist::resources::normalize(&document.parent().unwrap().join(path.as_ref()))
    };
    let anchor = anchor
        .map(|value| {
            percent_decode_str(value)
                .decode_utf8()
                .map(|value| value.into_owned())
                .map_err(|_| "invalid UTF-8 in URL fragment")
        })
        .transpose()?;
    Ok(Target::Local {
        path,
        query: if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        },
        fragment: anchor,
    })
}

pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let alen = a.iter().take_while(|byte| byte.is_ascii_digit()).count();
                let blen = b.iter().take_while(|byte| byte.is_ascii_digit()).count();
                let av = &a[..alen];
                let bv = &b[..blen];
                let az = av.iter().position(|byte| *byte != b'0').unwrap_or(alen);
                let bz = bv.iter().position(|byte| *byte != b'0').unwrap_or(blen);
                let order = av[az..]
                    .len()
                    .cmp(&bv[bz..].len())
                    .then(av[az..].cmp(&bv[bz..]))
                    .then(blen.cmp(&alen));
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[alen..];
                b = &b[blen..];
            }
            (Some(x), Some(y)) => {
                let order = x.cmp(y);
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}
