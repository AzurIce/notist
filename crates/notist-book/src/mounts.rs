//! Source identities stay physical; book paths select chapters and locate output.
use crate::{BookConfig, Error};
use notist::{Resources, resources::ResourceKind};
use std::path::{Component, Path, PathBuf};

struct Mapping {
    source: PathBuf,
    path: PathBuf,
}

pub(crate) struct Mounts {
    root: PathBuf,
    mappings: Vec<Mapping>,
}

impl Mounts {
    pub fn new(resources: &impl Resources, config: &BookConfig) -> Result<Self, Error> {
        let root = resources.root().to_path_buf();
        let output = resources.resolve(&config.output);
        let mut mappings = Vec::<Mapping>::new();
        for mount in &config.mounts {
            if mount.path.as_os_str().is_empty()
                || mount
                    .path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || mount.path.starts_with("_notist")
            {
                return Err(Error::Message(format!(
                    "invalid book mount path `{}`: expected a nonempty relative directory outside _notist",
                    mount.path.display()
                )));
            }
            let source = resources.resolve(&mount.source);
            if source.starts_with(&root) || root.starts_with(&source) {
                return Err(Error::Message(
                    "book mount source must be outside the Vault and cannot contain it".into(),
                ));
            }
            if source.starts_with(&output) || output.starts_with(&source) {
                return Err(Error::Message(
                    "book mount source and site output overlap".into(),
                ));
            }
            if mappings.iter().any(|previous| {
                source.starts_with(&previous.source)
                    || previous.source.starts_with(&source)
                    || mount.path.starts_with(&previous.path)
                    || previous.path.starts_with(&mount.path)
            }) {
                return Err(Error::Message(
                    "book mounts have overlapping sources or paths".into(),
                ));
            }
            // A mount may create virtual parent directories, but cannot shadow
            // an existing Vault entry or traverse a file in its namespace.
            let destination = root.join(&mount.path);
            if destination.starts_with(&output) || output.starts_with(&destination) {
                return Err(Error::Message(
                    "book mount path and site output overlap".into(),
                ));
            }
            for ancestor in destination.ancestors().take_while(|path| *path != root) {
                match resources.kind(ancestor)? {
                    Some(_) if ancestor == destination => {
                        return Err(Error::Message(format!(
                            "book mount path `{}` already exists in the Vault",
                            mount.path.display()
                        )));
                    }
                    Some(ResourceKind::File) => {
                        return Err(Error::Message(format!(
                            "book mount path traverses file `{}`",
                            ancestor.display()
                        )));
                    }
                    _ => {}
                }
            }
            if resources.kind(&source)? != Some(ResourceKind::Directory) {
                return Err(Error::Message(format!(
                    "book mount source `{}` is not a directory",
                    source.display()
                )));
            }
            mappings.push(Mapping {
                source,
                path: mount.path.clone(),
            });
        }
        Ok(Self { root, mappings })
    }

    pub fn roots(&self) -> impl Iterator<Item = (&Path, &Path)> {
        std::iter::once((self.root.as_path(), Path::new(""))).chain(
            self.mappings
                .iter()
                .map(|mapping| (mapping.source.as_path(), mapping.path.as_path())),
        )
    }

    /// Translate a real source identity into its book-relative path.
    pub fn path(&self, source: &Path) -> Option<PathBuf> {
        self.roots().find_map(|(root, path)| {
            source
                .strip_prefix(root)
                .ok()
                .map(|relative| path.join(relative))
        })
    }

    /// Accept real relative references as well as paths through a virtual mount.
    pub fn resolve(&self, source: &Path) -> PathBuf {
        for mapping in &self.mappings {
            if let Ok(relative) = source.strip_prefix(self.root.join(&mapping.path)) {
                return mapping.source.join(relative);
            }
        }
        source.into()
    }

    pub fn is_virtual_directory(&self, path: &Path) -> bool {
        self.mappings
            .iter()
            .any(|mapping| mapping.path.starts_with(path))
    }
}
