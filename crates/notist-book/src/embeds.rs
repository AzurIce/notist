//! Book presentation of image resources; other embeds retain the resource link.
use notist::{Item, ResourcePath, Value};
use notist_html::{escape_attribute, is_safe_url};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn image(item: &Item, resolved: &BTreeMap<ResourcePath, String>) -> Option<String> {
    let base = match item.field("target")? {
        Value::Path(path) => resolved.get(path)?.as_str(),
        Value::Str(url) => url.as_str(),
        _ => return None,
    };
    let url = notist::embed::append_suffix(base, &item.fields()).ok()?;
    // Returning None preserves the renderer's diagnostics for unsafe URLs.
    if !is_safe_url(base) || !is_safe_url(&url) {
        return None;
    }
    let path = url.split(['?', '#']).next()?;
    let extension = Path::new(path).extension()?.to_str()?;
    if ![
        "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "apng", "bmp", "ico",
    ]
    .iter()
    .any(|format| extension.eq_ignore_ascii_case(format))
    {
        return None;
    }
    let description = match item.field("description") {
        Some(Value::Str(value)) => value.as_str(),
        _ => "",
    };
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        escape_attribute(&url),
        escape_attribute(description)
    ))
}
