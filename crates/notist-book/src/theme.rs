use crate::{BookConfig, Error};
use handlebars::Handlebars;
use notist::publication::{Assets, url::relative_url};
use notist::{Resources, resources::ResourceKind};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) struct Theme {
    environment: Handlebars<'static>,
    styles: Vec<PathBuf>,
    scripts: Vec<PathBuf>,
}
impl Theme {
    pub fn load(
        resources: &(impl Resources + ?Sized),
        config: &BookConfig,
        assets: &mut Assets,
    ) -> Result<Self, Error> {
        let mut templates = BTreeMap::from([
            (
                "index".to_string(),
                include_str!("theme/index.hbs").to_string(),
            ),
            (
                "navigation".to_string(),
                include_str!("theme/navigation.hbs").to_string(),
            ),
            (
                "navigation-label".to_string(),
                include_str!("theme/navigation-label.hbs").to_string(),
            ),
        ]);
        let mut files = BTreeMap::from([
            (
                PathBuf::from("book.css"),
                include_bytes!("theme/book.css").to_vec(),
            ),
            (
                PathBuf::from("book.js"),
                include_bytes!("theme/book.js").to_vec(),
            ),
            (
                PathBuf::from("status.css"),
                notist_html::STATUS_CSS.as_bytes().to_vec(),
            ),
        ]);
        if let Some(path) = &config.theme {
            let root = resources.resolve(path);
            if !root.starts_with(resources.root()) {
                return Err(Error::Message("theme must be within the Vault".into()));
            }
            let output = resources.resolve(&config.output);
            if root.starts_with(&output) || output.starts_with(&root) {
                return Err(Error::Message(
                    "theme source and site output overlap".into(),
                ));
            }
            fn walk(
                resources: &(impl Resources + ?Sized),
                root: &Path,
                path: &Path,
                templates: &mut BTreeMap<String, String>,
                files: &mut BTreeMap<PathBuf, Vec<u8>>,
            ) -> Result<(), Error> {
                for entry in resources.entries(path)? {
                    match resources.kind(&entry)? {
                        Some(ResourceKind::Directory) => {
                            walk(resources, root, &entry, templates, files)?
                        }
                        Some(ResourceKind::File) => {
                            let relative = entry.strip_prefix(root).unwrap();
                            if let Ok(asset) = relative.strip_prefix("assets") {
                                files.insert(asset.into(), resources.read(&entry)?);
                            } else if entry
                                .extension()
                                .is_some_and(|extension| extension == "hbs")
                            {
                                templates.insert(
                                    relative
                                        .with_extension("")
                                        .to_string_lossy()
                                        .replace('\\', "/"),
                                    resources.source(&entry)?,
                                );
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            walk(resources, &root, &root, &mut templates, &mut files)?;
        }
        let mut styles = vec![];
        for (name, bytes) in files {
            let destination = PathBuf::from("_notist/theme").join(&name);
            if name.extension().is_some_and(|extension| extension == "css") {
                styles.push(destination.clone());
            }
            assets.generated(destination, bytes)?;
        }
        styles.sort_by_key(|path| {
            let priority = match path.to_str() {
                Some("_notist/theme/book.css") => 0,
                Some("_notist/theme/status.css") => 1,
                _ => 2,
            };
            (priority, path.clone())
        });
        let mut scripts = vec![PathBuf::from("_notist/theme/book.js")];
        for (paths, destinations) in [
            (&config.additional_css, &mut styles),
            (&config.additional_js, &mut scripts),
        ] {
            for path in paths {
                let source = resources.resolve(path);
                if !source.starts_with(resources.root()) {
                    return Err(Error::Message(
                        "additional theme assets must be within the Vault".into(),
                    ));
                }
                if resources.kind(&source)? != Some(ResourceKind::File) {
                    return Err(Error::Message(format!(
                        "additional theme asset `{}` is not a file",
                        path.display()
                    )));
                }
                let destination = source.strip_prefix(resources.root()).unwrap();
                if destination.starts_with("_notist") {
                    return Err(Error::Message(
                        "_notist is reserved for generated resources".into(),
                    ));
                }
                assets.copy(resources, &source, destination)?;
                destinations.push(destination.into());
            }
        }
        let mut environment = Handlebars::new();
        environment.set_strict_mode(true);
        for (name, source) in templates {
            environment.register_template_string(&name, source)?;
        }
        Ok(Self {
            environment,
            styles,
            scripts,
        })
    }
    pub fn styles(&self, page: &Path) -> Vec<String> {
        self.styles
            .iter()
            .map(|path| relative_url(page, path, false))
            .collect()
    }
    pub fn scripts(&self, page: &Path) -> Vec<String> {
        self.scripts
            .iter()
            .map(|path| relative_url(page, path, false))
            .collect()
    }
    pub fn render(&self, context: impl serde::Serialize) -> Result<String, Error> {
        Ok(self.environment.render("index", &context)?)
    }
}
