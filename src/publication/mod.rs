//! Resource planning shared by HTML document exports and site publication.
//! This module returns owned files; filesystem mutation belongs to the host.
use crate::{Resources, resources::ResourceError};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

mod index;
mod plan;
pub mod url;
pub use plan::{OutputDirectory, OutputEntry, OutputPlan, PlannedFile};

#[derive(Debug)]
pub enum Error {
    Message(String),
    Resource(ResourceError),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Resource(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<ResourceError> for Error {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}
/// Exact source identities for published resources, with shared destinations.
#[derive(Default)]
pub struct Assets {
    destinations: index::Index,
    files: Vec<Asset>,
    output: PathBuf,
}
struct Asset {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    source: Option<PathBuf>,
}
impl Assets {
    pub fn finish(mut self) -> Result<BTreeMap<PathBuf, Vec<u8>>, Error> {
        self.generated(
            "_notist/publication.json".into(),
            br#"{"format":"notist-publication","version":1}"#.to_vec(),
        )?;
        if let Some(file) = self
            .files
            .iter()
            .filter(|f| f.bytes.is_none())
            .min_by(|a, b| a.path.cmp(&b.path))
        {
            return Err(Error::Message(format!(
                "unwritten reserved output `{}`",
                file.path.display()
            )));
        }
        let order = self.destinations.ordered_files();
        let mut files: Vec<_> = self.files.into_iter().map(Some).collect();
        Ok(order
            .into_iter()
            .map(|id| {
                let file = files[id]
                    .take()
                    .expect("each planned file is exported once");
                (file.path, file.bytes.unwrap())
            })
            .collect())
    }
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            ..Self::default()
        }
    }
    pub fn generated(&mut self, path: PathBuf, bytes: Vec<u8>) -> Result<(), Error> {
        self.insert(path, Some(bytes))
    }
    /// Check additional destinations without mutating the current output plan.
    pub fn validate_generated_paths<'a>(
        &'a self,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<(), Error> {
        let mut planned = Self::default();
        for path in self
            .files
            .iter()
            .map(|file| file.path.as_path())
            .chain(paths)
        {
            planned.reserve_generated(path.to_path_buf())?;
        }
        Ok(())
    }
    /// Validate a known generated destination before constructing its bytes.
    pub fn reserve_generated(&mut self, path: PathBuf) -> Result<(), Error> {
        self.insert(path, None)
    }
    /// Fill exactly one reserved destination, without repeating path planning.
    pub fn write_reserved(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), Error> {
        match self
            .destinations
            .get(path)
            .map(|id| &mut self.files[id].bytes)
        {
            Some(slot @ None) => {
                *slot = Some(bytes);
                Ok(())
            }
            _ => Err(Error::Message(format!(
                "output is not an unwritten reservation: {}",
                path.display()
            ))),
        }
    }
    fn insert(&mut self, path: PathBuf, bytes: Option<Vec<u8>>) -> Result<(), Error> {
        if self
            .destinations
            .insert(
                &path,
                self.files.len(),
                self.files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.path.as_path(), i)),
            )
            .is_err()
        {
            return Err(Error::Message(format!(
                "output collision at `{}`",
                path.display()
            )));
        }
        self.files.push(Asset {
            path,
            bytes,
            source: None,
        });
        Ok(())
    }
    pub fn copy(
        &mut self,
        resources: &(impl Resources + ?Sized),
        source: &Path,
        destination: &Path,
    ) -> Result<(), Error> {
        if self.already_copied(source, destination)? {
            return Ok(());
        }
        match resources.kind(source)? {
            Some(crate::resources::ResourceKind::Directory) => {
                for entry in resources.entries(source)? {
                    self.copy(
                        resources,
                        &entry,
                        &destination.join(entry.file_name().unwrap()),
                    )?;
                }
            }
            Some(crate::resources::ResourceKind::File) => {
                let bytes = resources.read(source)?;
                self.insert_copy(source, destination, bytes)?;
            }
            None => {
                return Err(Error::Message(format!(
                    "missing resource `{}`",
                    source.display()
                )));
            }
        }
        Ok(())
    }
    /// Copy a discovered file through one checked capture operation. Unlike
    /// `copy`, a file replaced by a directory is rejected, not traversed.
    pub fn copy_file(
        &mut self,
        resources: &(impl Resources + ?Sized),
        source: &Path,
        destination: &Path,
    ) -> Result<(), Error> {
        if self.already_copied(source, destination)? {
            return Ok(());
        }
        let bytes = resources.capture_file(source)?;
        self.insert_copy(source, destination, bytes)
    }
    fn insert_copy(
        &mut self,
        source: &Path,
        destination: &Path,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        let id = self.files.len();
        self.generated(destination.into(), bytes)?;
        self.files[id].source = Some(source.into());
        Ok(())
    }
    fn already_copied(&self, source: &Path, destination: &Path) -> Result<bool, Error> {
        if self.output.starts_with(source) || source.starts_with(&self.output) {
            return Err(Error::Message(format!(
                "resource source and site output overlap: {}",
                source.display()
            )));
        }
        if let Some(previous) = self
            .destinations
            .get(destination)
            .and_then(|id| self.files[id].source.as_ref())
        {
            if previous == source {
                return Ok(true);
            }
            return Err(Error::Message(format!(
                "resource collision at `{}` between `{}` and `{}`",
                destination.display(),
                previous.display(),
                source.display()
            )));
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::Assets;

    #[test]
    fn additional_destinations_validate_without_mutating_reserved_or_written_files() {
        use std::path::Path;
        let mut assets = Assets::default();
        assets.reserve_generated("page/index.html".into()).unwrap();
        assets
            .generated("static.txt".into(), b"static".to_vec())
            .unwrap();
        assets
            .validate_generated_paths([Path::new("extra.bin")])
            .unwrap();
        for paths in [
            vec![Path::new("page")],
            vec![Path::new("page/index.html")],
            vec![Path::new("static.txt/child")],
            vec![Path::new("extra"), Path::new("extra/child")],
        ] {
            assert!(assets.validate_generated_paths(paths).is_err());
        }
        assets
            .write_reserved(Path::new("page/index.html"), b"page".to_vec())
            .unwrap();
        assets.generated("extra.bin".into(), vec![255]).unwrap();
        let files = assets.finish().unwrap();
        assert_eq!(files[Path::new("page/index.html")], b"page");
        assert_eq!(files[Path::new("extra.bin")], [255]);
    }

    #[test]
    fn generated_paths_reject_exact_and_ancestor_collisions_in_both_orders() {
        for (first, second) in [
            ("items/a.html", "items/a.html"),
            ("items", "items/a.html"),
            ("items/a.html", "items"),
            ("items/deep/a.html", "items"),
        ] {
            let mut assets = Assets::new("dist".into());
            assets.generated(first.into(), vec![]).unwrap();
            assert!(
                assets.generated(second.into(), vec![]).is_err(),
                "{first} / {second}"
            );
        }
    }

    #[test]
    fn generated_paths_compare_components_not_string_prefixes() {
        let mut assets = Assets::new("dist".into());
        for path in [
            "items/a.html",
            "items/a.html-more",
            "items-other/a.html",
            "items/b.html",
        ] {
            assets.generated(path.into(), vec![]).unwrap();
        }
        assert_eq!(assets.finish().unwrap().len(), 5);
    }
    #[test]
    fn reservations_preserve_collisions_and_must_be_written_once() {
        let mut assets = Assets::new("dist".into());
        assets.reserve_generated("items/a.html".into()).unwrap();
        assert!(assets.generated("items/a.html".into(), vec![]).is_err());
        assert!(assets.generated("items".into(), vec![]).is_err());
        assert!(
            assets
                .write_reserved(std::path::Path::new("missing"), vec![])
                .is_err()
        );
        assets
            .write_reserved(std::path::Path::new("items/a.html"), b"body".to_vec())
            .unwrap();
        assert!(
            assets
                .write_reserved(std::path::Path::new("items/a.html"), vec![])
                .is_err()
        );
        assert_eq!(
            assets.finish().unwrap()[std::path::Path::new("items/a.html")],
            b"body"
        );
        let mut incomplete = Assets::new("dist".into());
        incomplete.reserve_generated("missing.html".into()).unwrap();
        assert!(incomplete.finish().is_err());
    }

    #[test]
    fn copy_reuses_source_identity_without_reordering_probes_or_collision_errors() {
        use crate::{ResourceError, Resources, resources::ResourceKind};
        use std::{
            cell::RefCell,
            path::{Path, PathBuf},
        };
        struct Files(RefCell<Vec<String>>);
        impl Resources for Files {
            fn root(&self) -> &Path {
                Path::new("/input")
            }
            fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
                self.0.borrow_mut().push(format!("kind:{}", path.display()));
                Ok(Some(ResourceKind::File))
            }
            fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
                self.0.borrow_mut().push(format!("read:{}", path.display()));
                Ok(b"bytes".to_vec())
            }
            fn entries(&self, _: &Path) -> Result<Vec<PathBuf>, ResourceError> {
                unreachable!()
            }
        }
        let files = Files(RefCell::default());
        let mut assets = Assets::new("/output".into());
        assets
            .copy(&files, Path::new("/input/a"), Path::new("dir/a"))
            .unwrap();
        assert_eq!(*files.0.borrow(), ["kind:/input/a", "read:/input/a"]);
        files.0.borrow_mut().clear();
        assets
            .copy(&files, Path::new("/input/a"), Path::new("dir//a/"))
            .unwrap();
        assert!(files.0.borrow().is_empty());
        assert!(
            assets
                .copy(&files, Path::new("/input/other"), Path::new("dir/a"))
                .unwrap_err()
                .to_string()
                .contains("resource collision")
        );
        assert!(files.0.borrow().is_empty());
        assets.generated("generated".into(), vec![]).unwrap();
        assert!(
            assets
                .copy(&files, Path::new("/input/new"), Path::new("generated"))
                .unwrap_err()
                .to_string()
                .contains("output collision")
        );
        assert_eq!(*files.0.borrow(), ["kind:/input/new", "read:/input/new"]);
    }

    #[test]
    fn unfinished_reservations_report_standard_path_order_not_insertion_order() {
        let mut assets = Assets::new("/output".into());
        assets.reserve_generated("z/file".into()).unwrap();
        assets.reserve_generated("a/file".into()).unwrap();
        assert_eq!(
            assets.finish().unwrap_err().to_string(),
            "unwritten reserved output `a/file`"
        );
    }
}
