//! Book presentation of image resources; other embeds retain the resource link.
use notist::{Item, Value};
use notist_html::{escape_attribute, is_safe_url};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn image(item: &Item, resolved: &BTreeMap<String, String>) -> Option<String> {
    let Value::Str(target) = item.fields.get("target")? else {
        return None;
    };
    let url = resolved.get(target).map_or(target.as_str(), String::as_str);
    // Returning None preserves the renderer's diagnostics for unsafe URLs.
    if !is_safe_url(target) || !is_safe_url(url) {
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
    let description = match item.fields.get("description") {
        Some(Value::Str(value)) => value.as_str(),
        _ => "",
    };
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        escape_attribute(url),
        escape_attribute(description)
    ))
}
