//! Filesystem resources and safe publication shared by HTML export and sites.
use notist::{
    FsResources, InputError, InputRead, Resources,
    publication::Error,
    resources::{ResourceError, ResourceKind, ResourceReader, normalize},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) struct Files {
    base: FsResources,
    observed: Arc<Mutex<BTreeSet<PathBuf>>>,
}
impl Files {
    pub fn new(root: &Path) -> Self {
        Self {
            base: FsResources::new(root),
            observed: Arc::default(),
        }
    }
    pub fn with_inputs(self, inputs: BTreeSet<PathBuf>) -> Self {
        self.observed.lock().unwrap().extend(inputs);
        self
    }
    pub fn observed(&self) -> BTreeSet<PathBuf> {
        self.observed.lock().unwrap().clone()
    }
    fn inspect(&self, path: &Path) -> Result<PathBuf, ResourceError> {
        let path = self.resolve(path);
        self.observed.lock().unwrap().insert(path.clone());
        // Never traverse symlink directories or copy symlink files. This also
        // prevents directory cycles during discovery and component publication.
        for ancestor in path.ancestors() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ResourceError::Access {
                        path: ancestor.into(),
                        message: "site resources must be regular files or directories".into(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ResourceError::Access {
                        path: ancestor.into(),
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(path)
    }
}
impl Resources for Files {
    fn root(&self) -> &Path {
        self.base.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        self.base.read(&self.inspect(path)?)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.base.kind(&self.inspect(path)?)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.base.entries(&self.inspect(path)?)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        let root = self.resolve(root);
        Ok(Arc::new(ObservedReader {
            base: self.base.read_root(&root)?,
            root,
            observed: self.observed.clone(),
        }))
    }
}

struct ObservedReader {
    root: PathBuf,
    base: Arc<dyn ResourceReader>,
    observed: Arc<Mutex<BTreeSet<PathBuf>>>,
}
impl ResourceReader for ObservedReader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        let absolute = self.root.join(path);
        self.observed.lock().unwrap().insert(absolute.clone());
        self.base.read_bounded(path, limit)
    }
}

fn io(error: impl std::fmt::Display) -> Error {
    Error::Message(error.to_string())
}

/// An identical rebuild needs no directory replacement. Compare actual bounded
/// regular files, not timestamps or an untrusted manifest digest. Any unexpected
/// entry, symlink or read failure falls back to ordinary staged publication.
fn unchanged(files: &BTreeMap<PathBuf, Vec<u8>>, output: &Path) -> bool {
    let resources = FsResources::new(output);
    let Ok(reader) = resources.read_root(output) else {
        return false;
    };
    let directories: BTreeSet<_> = files
        .keys()
        .flat_map(|file| file.parent().unwrap().ancestors().map(Path::to_path_buf))
        .collect();
    let mut pending = vec![output.to_path_buf()];
    let mut seen = 0;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(output) else {
                return false;
            };
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            if kind.is_dir() {
                if !directories.contains(relative) {
                    return false;
                }
                pending.push(path);
            } else if kind.is_file() {
                let Some(expected) = files.get(relative) else {
                    return false;
                };
                match reader.read_bounded(relative, expected.len()) {
                    InputRead::Ready(actual) if actual.as_ref() == expected.as_slice() => seen += 1,
                    _ => return false,
                }
            } else {
                return false;
            }
        }
    }
    seen == files.len()
}
/// Stage every file before replacing an owned output directory. A failed build
/// or publication leaves the previous site intact.
pub fn publish(
    files: &BTreeMap<PathBuf, Vec<u8>>,
    output: &Path,
    root: &Path,
) -> Result<(), Error> {
    let output = normalize(&std::env::current_dir().map_err(io)?.join(output));
    if root.starts_with(&output) {
        return Err(Error::Message(
            "site output cannot replace the Vault or an ancestor".into(),
        ));
    }
    for ancestor in output.ancestors() {
        if std::fs::symlink_metadata(ancestor)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(Error::Message(
                "site output cannot traverse symlinks".into(),
            ));
        }
    }
    if output.exists() {
        let metadata = std::fs::metadata(&output).map_err(io)?;
        if !metadata.is_dir() {
            return Err(Error::Message("site output must be a directory".into()));
        }
        let empty = std::fs::read_dir(&output).map_err(io)?.next().is_none();
        let owned = std::fs::read(output.join("_notist/publication.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|manifest| {
                manifest["format"] == "notist-publication" && manifest["version"] == 1
            });
        if !empty && !owned {
            return Err(Error::Message(format!(
                "refusing to replace nonempty output without a Notist site manifest: {}",
                output.display()
            )));
        }
    }
    for relative in files.keys() {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(Error::Message("invalid publication file path".into()));
        }
    }
    if output.exists() && unchanged(files, &output) {
        return Ok(());
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::Message("invalid site output".into()))?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let staging = tempfile::Builder::new()
        .prefix(".notist-stage-")
        .tempdir_in(parent)
        .map_err(io)?;
    let mut directories = BTreeSet::new();
    for (relative, bytes) in files {
        let file = staging.path().join(relative);
        if directories.insert(relative.parent().unwrap()) {
            std::fs::create_dir_all(file.parent().unwrap()).map_err(io)?;
        }
        std::fs::write(file, bytes).map_err(io)?;
    }
    let backup = tempfile::Builder::new()
        .prefix(".notist-old-")
        .tempdir_in(parent)
        .map_err(io)?;
    let retired = backup.path().join("site");
    if output.exists() {
        std::fs::rename(&output, &retired).map_err(io)?;
    }
    if let Err(error) = std::fs::rename(staging.path(), &output) {
        if retired.exists()
            && let Err(restore) = std::fs::rename(&retired, &output)
        {
            let saved = backup.keep();
            return Err(io(format!(
                "publication failed: {error}; restore failed: {restore}; previous site preserved at {}",
                saved.join("site").display()
            )));
        }
        return Err(io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact() -> BTreeMap<PathBuf, Vec<u8>> {
        BTreeMap::from([
            (
                "_notist/publication.json".into(),
                br#"{"format":"notist-publication","version":1}"#.to_vec(),
            ),
            ("items/a.html".into(), b"first".to_vec()),
        ])
    }
    #[test]
    fn identical_publications_verify_bytes_and_prune_unexpected_entries() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("public");
        let root = temp.path().join("source");
        let files = artifact();
        publish(&files, &output, &root).unwrap();
        assert!(unchanged(&files, &output));
        let modified = std::fs::metadata(output.join("items/a.html"))
            .unwrap()
            .modified()
            .unwrap();
        publish(&files, &output, &root).unwrap();
        assert_eq!(
            std::fs::metadata(output.join("items/a.html"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        for stale in ["stale.html", "empty/directory"] {
            if stale.ends_with("html") {
                std::fs::write(output.join(stale), b"stale").unwrap();
            } else {
                std::fs::create_dir_all(output.join(stale)).unwrap();
            }
            assert!(!unchanged(&files, &output));
            publish(&files, &output, &root).unwrap();
            assert!(!output.join(stale).exists());
        }
        std::fs::write(output.join("items/a.html"), b"other").unwrap();
        assert!(!unchanged(&files, &output));
        publish(&files, &output, &root).unwrap();
        assert_eq!(
            std::fs::read(output.join("items/a.html")).unwrap(),
            b"first"
        );
        let mut invalid = files.clone();
        invalid.insert("../escape".into(), vec![]);
        assert!(publish(&invalid, &output, &root).is_err());
        assert!(unchanged(&files, &output));
    }
    #[cfg(unix)]
    #[test]
    fn no_op_detection_never_reuses_symlink_entries() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("public");
        let root = temp.path().join("source");
        let files = artifact();
        publish(&files, &output, &root).unwrap();
        let external = temp.path().join("private");
        std::fs::write(&external, b"first").unwrap();
        std::fs::remove_file(output.join("items/a.html")).unwrap();
        std::os::unix::fs::symlink(&external, output.join("items/a.html")).unwrap();
        assert!(!unchanged(&files, &output));
        publish(&files, &output, &root).unwrap();
        assert!(
            !std::fs::symlink_metadata(output.join("items/a.html"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(external).unwrap(), b"first");
    }
}
