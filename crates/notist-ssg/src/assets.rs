use crate::{Error, SiteConfig, templates::files};
use notist::{Resources, publication::Assets, resources::ResourceKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

struct SassFs<'a, R>(&'a R);
impl<R> std::fmt::Debug for SassFs<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SassResources")
    }
}
impl<R: Resources> grass::Fs for SassFs<'_, R> {
    fn is_dir(&self, path: &Path) -> bool {
        self.0.kind(path).ok().flatten() == Some(ResourceKind::Directory)
    }
    fn is_file(&self, path: &Path) -> bool {
        self.0.kind(path).ok().flatten() == Some(ResourceKind::File)
    }
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.0.read(path).map_err(std::io::Error::other)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        Ok(self.0.resolve(path))
    }
}

pub(crate) fn prepare(
    resources: &impl Resources,
    config: &SiteConfig,
    content_assets: &[PathBuf],
    assets: &mut Assets,
) -> Result<BTreeSet<PathBuf>, Error> {
    let content = resources.resolve(&config.content);
    let mut copies = BTreeMap::new();
    for source in content_assets {
        copies.insert(
            source.strip_prefix(&content).unwrap().to_path_buf(),
            source.clone(),
        );
    }
    let mut static_roots = vec![];
    let mut sass_roots = vec![];
    if let Some(theme) = &config.theme {
        let theme = resources.resolve(theme);
        static_roots.push(theme.join("static"));
        sass_roots.push(theme.join("sass"));
    }
    static_roots.push(resources.resolve(&config.static_dir));
    sass_roots.push(resources.resolve(&config.sass));
    let mut static_files = BTreeMap::new();
    // Site assets intentionally override theme assets; content collisions are errors.
    for root in static_roots {
        for source in files(resources, &root)? {
            static_files.insert(source.strip_prefix(&root).unwrap().to_path_buf(), source);
        }
    }
    let mut published = BTreeSet::new();
    for (destination, source) in copies.iter().chain(static_files.iter()) {
        if destination.starts_with("_notist") {
            return Err(Error::Message(
                "_notist is reserved for generated resources".into(),
            ));
        }
        assets.copy(resources, source, destination)?;
        published.insert(destination.clone());
    }
    let mut styles = BTreeMap::new();
    for root in sass_roots {
        for source in files(resources, &root)? {
            if source
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with('_'))
                || !source
                    .extension()
                    .is_some_and(|s| s == "scss" || s == "sass")
            {
                continue;
            }
            styles.insert(
                source.strip_prefix(&root).unwrap().with_extension("css"),
                source,
            );
        }
    }
    let fs = SassFs(resources);
    for (destination, source) in styles {
        if destination.starts_with("_notist") {
            return Err(Error::Message(
                "_notist is reserved for generated resources".into(),
            ));
        }
        let css = grass::from_path(&source, &grass::Options::default().fs(&fs))
            .map_err(|e| Error::Message(e.to_string()))?;
        assets.generated(destination.clone(), css.into_bytes())?;
        published.insert(destination);
    }
    Ok(published)
}

pub(crate) fn data(
    resources: &impl Resources,
    directory: &Path,
) -> Result<BTreeMap<String, serde_json::Value>, Error> {
    let mut data = BTreeMap::new();
    for path in files(resources, directory)? {
        let value = match path.extension().and_then(|s| s.to_str()) {
            Some("toml") => {
                let value: toml::Value = toml::from_str(&resources.source(&path)?)
                    .map_err(|e| Error::Message(format!("{}: {e}", path.display())))?;
                serde_json::to_value(value).map_err(|e| Error::Message(e.to_string()))?
            }
            Some("json") => serde_json::from_str(&resources.source(&path)?)
                .map_err(|e| Error::Message(format!("{}: {e}", path.display())))?,
            _ => continue,
        };
        let key = path
            .strip_prefix(directory)
            .unwrap()
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/");
        if data.insert(key.clone(), value).is_some() {
            return Err(Error::Message(format!("duplicate site data key `{key}`")));
        }
    }
    Ok(data)
}
