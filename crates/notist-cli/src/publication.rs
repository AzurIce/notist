//! Filesystem resources and safe publication shared by HTML export and sites.
use notist::{
    FsResources, InputError, InputRead, Resources,
    publication::{Error, OutputEntry, OutputPlan},
    resources::{ResolvedPath, ResourceError, ResourceKind, ResourceReader, normalize},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) struct Files {
    base: FsResources,
    observed: Arc<Mutex<HashSet<PathBuf>>>,
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
        self.observed.lock().unwrap().iter().cloned().collect()
    }
    fn inspect(&self, path: &Path) -> Result<ResolvedPath, ResourceError> {
        let path = self.base.resolve_path(path);
        self.observed.lock().unwrap().insert(path.as_path().into());
        // Never traverse symlink directories or copy symlink files. This also
        // prevents directory cycles during discovery and component publication.
        for ancestor in path.as_path().ancestors() {
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
        self.base.read_resolved(&self.inspect(path)?)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.base.kind_resolved(&self.inspect(path)?)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.base.entries_resolved(&self.inspect(path)?)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        let root = self.base.resolve_path(root);
        Ok(Arc::new(ObservedReader {
            base: self.base.read_root_resolved(&root)?,
            root: root.as_path().into(),
            observed: self.observed.clone(),
        }))
    }
}

struct ObservedReader {
    root: PathBuf,
    base: Arc<dyn ResourceReader>,
    observed: Arc<Mutex<HashSet<PathBuf>>>,
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
fn unchanged(plan: &OutputPlan<'_>, output: &Path) -> bool {
    let resources = FsResources::new(output);
    let Ok(reader) = resources.read_root(output) else {
        return false;
    };
    let mut pending = vec![(output.to_path_buf(), plan.root())];
    let mut buffer = [0u8; 64 * 1024];
    let mut seen = 0;
    while let Some((directory, planned)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let Some(expected) = planned.child(&entry.file_name()) else {
                return false;
            };
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            match expected {
                OutputEntry::Directory(child) if kind.is_dir() => {
                    pending.push((entry.path(), child))
                }
                OutputEntry::File(file) if kind.is_file() => {
                    if reader.matches_bytes(file.path, file.bytes, &mut buffer) != Ok(true) {
                        return false;
                    }
                    seen += 1;
                }
                _ => return false,
            }
        }
    }
    seen == plan.files().len()
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
    let plan = OutputPlan::new(files)?;
    if output.exists() && unchanged(&plan, &output) {
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
    let mut directories = HashSet::new();
    for planned in plan.files() {
        let file = staging.path().join(planned.path);
        if directories.insert(planned.parent_key()) {
            std::fs::create_dir_all(file.parent().unwrap()).map_err(io)?;
        }
        std::fs::write(file, planned.bytes).map_err(io)?;
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
    #[cfg(unix)]
    #[test]
    fn resolved_paths_and_observation_do_not_cache_symlink_safety_checks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("input");
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("dir/file"), "bytes").unwrap();
        let files = Files::new(&root);
        assert_eq!(files.read(Path::new("dir/./file")).unwrap(), b"bytes");
        std::fs::rename(root.join("dir"), root.join("old")).unwrap();
        std::os::unix::fs::symlink(root.join("old"), root.join("dir")).unwrap();
        assert!(files.read(Path::new("dir/file")).is_err());
        assert!(files.kind(Path::new("dir/file")).is_err());
        assert_eq!(files.observed(), BTreeSet::from([root.join("dir/file")]));
    }
    #[test]
    fn identical_publications_verify_bytes_and_prune_unexpected_entries() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("public");
        let root = temp.path().join("source");
        let files = artifact();
        publish(&files, &output, &root).unwrap();
        assert!(unchanged(&OutputPlan::new(&files).unwrap(), &output));
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
            assert!(!unchanged(&OutputPlan::new(&files).unwrap(), &output));
            publish(&files, &output, &root).unwrap();
            assert!(!output.join(stale).exists());
        }
        std::fs::write(output.join("items/a.html"), b"other").unwrap();
        assert!(!unchanged(&OutputPlan::new(&files).unwrap(), &output));
        publish(&files, &output, &root).unwrap();
        assert_eq!(
            std::fs::read(output.join("items/a.html")).unwrap(),
            b"first"
        );
        let mut invalid = files.clone();
        invalid.insert("../escape".into(), vec![]);
        assert!(publish(&invalid, &output, &root).is_err());
        assert!(unchanged(&OutputPlan::new(&files).unwrap(), &output));
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
        assert!(!unchanged(&OutputPlan::new(&files).unwrap(), &output));
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
