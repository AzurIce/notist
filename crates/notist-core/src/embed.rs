//! Explicit embed inputs. Local URL spellings are normalized once at the
//! constructor boundary; publication, existence checks and IO belong to hosts.
use crate::{
    item::{Ctor, Item, Value},
    resource::ResourcePath,
    shared::Fields,
};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

const FRAGMENT: &AsciiSet = &CONTROLS
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

pub fn is_remote(url: &str) -> bool {
    url.starts_with("//")
        || url
            .split(['/', '\\', '?', '#'])
            .next()
            .unwrap_or("")
            .contains(':')
}

/// Whether this concrete output will use the native embed presentation. Expanded
/// or failed wrappers do not publish their unused original input resources.
pub fn is_embed(item: &Item) -> bool {
    item.ctor() == Ctor::Embed
}

/// Native embed outputs actually visited by presentation. Error boundaries hide
/// their children; expanded wrappers expose only their resulting children.
pub fn targets(root: &Item) -> impl Iterator<Item = &Item> {
    references(root).filter(|item| is_embed(item))
}

/// Effective link and embed references, excluding hidden error subtrees.
pub fn references(root: &Item) -> impl Iterator<Item = &Item> {
    let mut stack = vec![root];
    std::iter::from_fn(move || {
        while let Some(item) = stack.pop() {
            if item.is_error() {
                continue;
            }
            stack.extend(item.children().iter().rev());
            if is_embed(item) || item.ctor() == Ctor::Link {
                return Some(item);
            }
        }
        None
    })
}

/// Normalize source-level embed arguments, without checking or reading files.
/// Already bound Paths (including `?`, `#`, `%` in filenames) are never parsed.
pub fn normalize(fields: &mut impl Fields, root: &str, source: &str) -> Result<(), String> {
    let Some(target) = fields.get("target").cloned() else {
        return Err("embed requires a target".into());
    };
    match target {
        Value::Path(_) => {}
        Value::Str(target) if is_remote(&target) => {}
        Value::Str(target) => {
            let (before, fragment) = target
                .split_once('#')
                .map_or((target.as_str(), None), |(p, f)| (p, Some(f)));
            let (path, query) = before
                .split_once('?')
                .map_or((before, None), |(p, q)| (p, Some(q)));
            let path = percent_decode_str(path)
                .decode_utf8()
                .map_err(|_| "invalid UTF-8 in embed URL path")?;
            let path = ResourcePath::resolve_reference(root, source, &path)?;
            if let Some(query) = query {
                add_suffix(fields, "query", query.to_owned())?;
            }
            if let Some(fragment) = fragment {
                let fragment = percent_decode_str(fragment)
                    .decode_utf8()
                    .map_err(|_| "invalid UTF-8 in embed URL fragment")?;
                add_suffix(fields, "fragment", fragment.into_owned())?;
            }
            fields.insert("target", Value::Path(path));
        }
        _ => return Err("embed target requires a Path or URL string".into()),
    }
    append_suffix(string(fields, "target").unwrap_or(""), fields)?;
    Ok(())
}

fn add_suffix(fields: &mut impl Fields, name: &str, value: String) -> Result<(), String> {
    if fields.get(name).is_some() {
        return Err(format!(
            "embed {name} supplied both in the URL and as an argument"
        ));
    }
    fields.insert(name, Value::Str(value.into()));
    Ok(())
}

/// Add semantic suffix fields to a host-provided URL. Resource filenames never
/// enter this function; hosts encode the path when selecting its public URL.
pub fn append_suffix(base: &str, fields: &impl Fields) -> Result<String, String> {
    let query = string(fields, "query");
    let fragment = string(fields, "fragment");
    if query.is_some_and(|query| query.chars().any(|c| c.is_control() || c == '#')) {
        return Err("embed query must be URL query text without control characters or `#`".into());
    }
    let (before_fragment, existing_fragment) = base
        .split_once('#')
        .map_or((base, None), |(p, f)| (p, Some(f)));
    if query.is_some() && before_fragment.contains('?') {
        return Err("embed URL already contains a query".into());
    }
    if fragment.is_some() && existing_fragment.is_some() {
        return Err("embed URL already contains a fragment".into());
    }
    let mut url = before_fragment.to_owned();
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    if let Some(fragment) = fragment {
        url.push('#');
        url.push_str(&utf8_percent_encode(fragment, FRAGMENT).to_string());
    } else if let Some(fragment) = existing_fragment {
        url.push('#');
        url.push_str(fragment);
    }
    if url.is_empty() {
        url.push_str("./");
    }
    Ok(url)
}

pub fn string<'a>(fields: &'a impl Fields, name: &str) -> Option<&'a str> {
    match fields.get(name) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}
