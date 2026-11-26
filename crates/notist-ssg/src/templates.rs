use crate::{Error, SiteConfig};
use notist::{Resources, resources::ResourceKind};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tera::{Filter, Kwargs, State, Tera};

pub(crate) fn files(resources: &impl Resources, root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut found = vec![];
    match resources.kind(root)? {
        None => return Ok(found),
        Some(ResourceKind::File) => {
            return Err(Error::Message(format!(
                "expected directory `{}`",
                root.display()
            )));
        }
        _ => {}
    }
    for path in resources.entries(root)? {
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        match resources.kind(&path)? {
            Some(ResourceKind::Directory) => found.extend(files(resources, &path)?),
            Some(ResourceKind::File) => found.push(path),
            None => {}
        }
    }
    found.sort();
    Ok(found)
}

/// Relative prefix from a page's output file to the output root.
pub(crate) fn page_root(output: &Path) -> String {
    notist::publication::url::relative_url(output, Path::new(""), true)
}

fn external(path: &str) -> bool {
    path.starts_with("//")
        || path
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains(':')
}

fn state_page_root(state: &State) -> Option<String> {
    let page = state.get::<tera::Value>("page").ok().flatten()?;
    page.as_map()?
        .get(&tera::value::Key::from("root".to_owned()))?
        .as_str()
        .map(str::to_owned)
}

/// JSON-encode a template value for embedding in an HTML document. Escaping
/// `</` keeps the result inert inside `<script type="application/json">`.
struct JsonEncode;

impl Filter<tera::Value, Result<tera::Value, tera::Error>> for JsonEncode {
    fn call(&self, value: tera::Value, _: Kwargs, _: &State) -> Result<tera::Value, tera::Error> {
        let encoded = serde_json::to_string(&value).map_err(tera::Error::message)?;
        Ok(tera::Value::from(encoded.replace("</", "<\\/")))
    }

    fn is_safe(&self) -> bool {
        true
    }
}

pub(crate) fn load(resources: &impl Resources, config: &SiteConfig) -> Result<Tera, Error> {
    let mut templates: BTreeMap<String, String> = BTreeMap::from([("page".into(), "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{{page.title}} · {{site.title}}</title><link rel=\"stylesheet\" href=\"{{assets.highlight}}\"></head><body><main>{{page.html | safe}}</main>{% if assets.components %}<script type=\"module\" src=\"{{assets.components}}\"></script>{% endif %}</body></html>".into())]);
    let mut directories = vec![];
    if let Some(theme) = &config.theme {
        if crate::themes::is_builtin(theme) {
            templates.extend(crate::themes::templates(theme.to_str().unwrap()).ok_or_else(|| Error::Message(format!("unknown builtin theme `{}`", theme.display())))?);
        } else {
        let theme = resources.resolve(theme);
        if resources.kind(&theme)? != Some(ResourceKind::Directory) {
            return Err(Error::Message(format!(
                "theme directory `{}` does not exist",
                theme.display()
            )));
        }
        directories.push(theme.join("templates"));
        }
    }
    directories.push(resources.resolve(&config.templates));
    for directory in directories {
        for path in files(resources, &directory)? {
            if path.extension().is_some_and(|s| s == "tera") {
                let name = path
                    .strip_prefix(&directory)
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .replace('\\', "/");
                templates.insert(name, resources.source(&path)?);
            }
        }
    }
    let mut engine = Tera::default();
    engine.autoescape_on([""]);
    engine.register_filter("to_json", JsonEncode);
    engine.register_filter("json_encode", JsonEncode);
    let config = config.clone();
    engine.register_function(
        "url",
        move |args: Kwargs, state: &State| -> Result<String, tera::Error> {
            let path = args.must_get::<&str>("path")?;
            if !notist_html::is_safe_url(path) {
                return Err(tera::Error::message("unsafe site URL"));
            }
            if let Some(from) = args.get::<&str>("relative_to")? {
                if !external(path) {
                    let target = percent_encoding::percent_decode_str(path.trim_start_matches('/'))
                        .decode_utf8()
                        .map_err(|_| tera::Error::message("invalid UTF-8 in site URL"))?;
                    return Ok(notist::publication::url::relative_url(
                        Path::new(from),
                        Path::new(target.as_ref()),
                        path.ends_with('/'),
                    ));
                }
                return Ok(path.into());
            }
            if config.uses_relative_urls() && !external(path) {
                let root = state_page_root(state).ok_or_else(|| {
                    tera::Error::message("relative url() requires a page context")
                })?;
                let mut url = root;
                url.push_str(path.trim_start_matches('/'));
                return Ok(url);
            }
            Ok(config.url(path))
        },
    );
    engine
        .add_raw_templates(templates)
        .map_err(|e| Error::Message(format!("templates: {e}")))?;
    Ok(engine)
}
