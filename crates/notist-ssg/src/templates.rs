use crate::{Error, SiteConfig};
use notist::{Resources, resources::ResourceKind};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tera::{Kwargs, State, Tera};

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
    let config = config.clone();
    engine.register_function(
        "url",
        move |args: Kwargs, _: &State| -> Result<String, tera::Error> {
            let path = args.must_get::<&str>("path")?;
            if !notist_html::is_safe_url(path) {
                return Err(tera::Error::message("unsafe site URL"));
            }
            Ok(config.url(path))
        },
    );
    engine
        .add_raw_templates(templates)
        .map_err(|e| Error::Message(format!("templates: {e}")))?;
    Ok(engine)
}
