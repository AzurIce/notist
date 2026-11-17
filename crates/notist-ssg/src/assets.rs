use crate::{Error, SiteConfig, templates::files};
use notist::{Resources, publication::Assets, resources::ResourceKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

struct SassFs<'a, R>(&'a R);

/// Discover static files using enumeration hints, but never use a hint to
/// authorize a later read. Selected files go through `capture_file` separately.
fn static_files(resources: &impl Resources, root: &Path) -> Result<Vec<PathBuf>, Error> {
    fn visit(
        resources: &impl Resources,
        root: &Path,
        found: &mut Vec<PathBuf>,
    ) -> Result<(), Error> {
        for entry in resources.entries_with_kind(root)? {
            if entry
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            let kind = match entry.kind_hint {
                Some(kind) => Some(kind),
                None => resources.kind(&entry.path)?,
            };
            match kind {
                Some(ResourceKind::Directory) => visit(resources, &entry.path, found)?,
                Some(ResourceKind::File) => found.push(entry.path),
                None => {}
            }
        }
        Ok(())
    }
    let mut found = vec![];
    match resources.kind(root)? {
        None => return Ok(found),
        Some(ResourceKind::File) => {
            return Err(Error::Message(format!(
                "expected directory `{}`",
                root.display()
            )));
        }
        Some(ResourceKind::Directory) => visit(resources, root, &mut found)?,
    }
    found.sort();
    Ok(found)
}
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
    mounts: &crate::mounts::Mounts,
    assets: &mut Assets,
) -> Result<BTreeSet<PathBuf>, Error> {
    let mut copies = BTreeMap::new();
    for source in content_assets.iter().filter(|_| config.copy_content_assets) {
        copies.insert(mounts.path(source).unwrap(), source.clone());
    }
    let mut static_roots = vec![];
    let mut sass_roots = vec![];
    let mut embedded = BTreeMap::new();
    if let Some(theme) = &config.theme {
        if crate::themes::is_builtin(theme) {
            embedded = crate::themes::assets(theme.to_str().unwrap()).ok_or_else(|| {
                Error::Message(format!("unknown builtin theme `{}`", theme.display()))
            })?;
        } else {
            let theme = mounts.resolve(&resources.resolve(theme));
            static_roots.push(theme.join("static"));
            sass_roots.push(theme.join("sass"));
        }
    }
    static_roots.push(mounts.resolve(&resources.resolve(&config.static_dir)));
    sass_roots.push(mounts.resolve(&resources.resolve(&config.sass)));
    let mut static_copies = BTreeMap::new();
    // Site assets intentionally override theme assets; content collisions are errors.
    for root in static_roots {
        for source in static_files(resources, &root)? {
            static_copies.insert(source.strip_prefix(&root).unwrap().to_path_buf(), source);
        }
    }
    let mut published = BTreeSet::new();
    for (name, bytes) in embedded {
        let destination = PathBuf::from(name);
        if !static_copies.contains_key(&destination) {
            assets.generated(destination.clone(), bytes)?;
            published.insert(destination);
        }
    }
    for (destination, source) in copies.iter().chain(static_copies.iter()) {
        if destination.starts_with("_notist") {
            return Err(Error::Message(
                "_notist is reserved for generated resources".into(),
            ));
        }
        assets.copy_file(resources, source, destination)?;
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
        let timing = notist::progress::Span::stage("site.sass", || {
            format!("Compile Sass {}", source.display())
        });
        timing.attribute("input", || source.display().to_string());
        if destination.starts_with("_notist") {
            return Err(Error::Message(
                "_notist is reserved for generated resources".into(),
            ));
        }
        let css = grass::from_path(&source, &grass::Options::default().fs(&fs))
            .map_err(|e| Error::Message(e.to_string()))?;
        assets.generated(destination.clone(), css.into_bytes())?;
        published.insert(destination);
        timing.finish();
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

#[cfg(test)]
mod tests {
    use super::*;
    use notist::{
        MemoryResources,
        resources::{ResourceEntry, ResourceError},
    };
    use std::cell::RefCell;

    struct Hinted {
        base: MemoryResources,
        kinds: RefCell<Vec<PathBuf>>,
    }
    impl Resources for Hinted {
        fn root(&self) -> &Path {
            self.base.root()
        }
        fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
            self.base.read(path)
        }
        fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
            assert!(
                !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            );
            self.kinds.borrow_mut().push(path.into());
            self.base.kind(path)
        }
        fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
            self.base.entries(path)
        }
        fn entries_with_kind(&self, path: &Path) -> Result<Vec<ResourceEntry>, ResourceError> {
            self.base
                .entries(path)?
                .into_iter()
                .map(|path| {
                    let kind_hint = if path.file_name().unwrap().to_string_lossy().starts_with('.')
                    {
                        None
                    } else {
                        self.base.kind(&path)?
                    };
                    Ok(ResourceEntry { path, kind_hint })
                })
                .collect()
        }
    }

    #[test]
    fn typed_static_discovery_sorts_once_and_skips_hidden_before_kind() {
        let mut base = MemoryResources::new("/assets");
        for path in [
            "static/z",
            "static/a/file",
            "static/.hidden",
            "static/a/.hidden",
        ] {
            base.insert(path, vec![]);
        }
        let resources = Hinted {
            base,
            kinds: Default::default(),
        };
        let root = resources.resolve(Path::new("static"));
        let expected = files(&resources.base, &root).unwrap();
        assert_eq!(static_files(&resources, &root).unwrap(), expected);
        assert_eq!(static_files(&resources.base, &root).unwrap(), expected);
        assert_eq!(*resources.kinds.borrow(), [root]);
    }
}
