//! Embedded themes use the same template names and static overlay as disk themes.
use std::collections::BTreeMap;

pub(crate) fn templates(name: &str) -> Option<BTreeMap<String, String>> {
    match name {
        "builtins.default" => Some(BTreeMap::from([(
            "page".into(),
            include_str!("default/templates/page.tera").into(),
        )])),
        "builtins.book" => Some(
            [
                ("page", include_str!("book/templates/page.tera")),
                ("navigation", include_str!("book/templates/navigation.tera")),
                (
                    "navigation-label",
                    include_str!("book/templates/navigation-label.tera"),
                ),
                (
                    "navigation-script",
                    include_str!("book/templates/navigation-script.tera"),
                ),
            ]
            .into_iter()
            .map(|(name, source)| (name.into(), source.into()))
            .collect(),
        ),
        _ => None,
    }
}

pub(crate) fn assets(name: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    match name {
        "builtins.default" => Some(BTreeMap::new()),
        "builtins.book" => Some(
            [
                ("book.css", include_bytes!("book/static/book.css").to_vec()),
                ("book.js", include_bytes!("book/static/book.js").to_vec()),
                ("status.css", notist_html::STATUS_CSS.as_bytes().to_vec()),
            ]
            .into_iter()
            .map(|(name, bytes)| (name.into(), bytes))
            .collect(),
        ),
        _ => None,
    }
}

pub(crate) fn is_builtin(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(|s| s.starts_with("builtins."))
}
