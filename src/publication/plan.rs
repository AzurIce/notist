//! Immutable output views and directory identities, independent of filesystem IO.
use super::{
    Error,
    index::{Index, ordinary},
};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub struct PlannedFile<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
    parent: usize,
}
impl PlannedFile<'_> {
    /// An opaque, plan-local directory identity, not a path or read capability.
    pub fn parent_key(&self) -> usize {
        self.parent
    }
}

pub struct OutputPlan<'a> {
    files: Vec<PlannedFile<'a>>,
    index: Index,
}
impl<'a> OutputPlan<'a> {
    pub fn new(files: &'a BTreeMap<PathBuf, Vec<u8>>) -> Result<Self, Error> {
        let mut result = Self {
            files: Vec::with_capacity(files.len()),
            index: Index::default(),
        };
        for (path, bytes) in files {
            if !ordinary(path) {
                return Err(Error::Message("invalid publication file path".into()));
            }
            result
                .index
                .insert(path, result.files.len(), std::iter::empty())
                .map_err(|_| Error::Message(format!("output collision at `{}`", path.display())))?;
            let parent = result.index.node(path.parent().unwrap()).unwrap();
            result.files.push(PlannedFile {
                path,
                bytes,
                parent,
            });
        }
        Ok(result)
    }
    pub fn files(&self) -> &[PlannedFile<'a>] {
        &self.files
    }
    pub fn root(&self) -> OutputDirectory<'_, 'a> {
        OutputDirectory {
            plan: self,
            node: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct OutputDirectory<'p, 'a> {
    plan: &'p OutputPlan<'a>,
    node: usize,
}
pub enum OutputEntry<'p, 'a> {
    Directory(OutputDirectory<'p, 'a>),
    File(&'p PlannedFile<'a>),
}
impl<'p, 'a> OutputDirectory<'p, 'a> {
    pub fn child(&self, name: &OsStr) -> Option<OutputEntry<'p, 'a>> {
        let node = self.plan.index.child(self.node, name)?;
        Some(match self.plan.index.file(node) {
            Some(file) => OutputEntry::File(&self.plan.files[file]),
            None => OutputEntry::Directory(Self {
                plan: self.plan,
                node,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plan_borrows_bytes_keeps_order_and_directory_identity() {
        let files = BTreeMap::from([
            ("a/one".into(), vec![1]),
            ("a/two".into(), vec![2]),
            ("b/three".into(), vec![3]),
        ]);
        let plan = OutputPlan::new(&files).unwrap();
        assert_eq!(
            plan.files()[0].bytes.as_ptr(),
            files[Path::new("a/one")].as_ptr()
        );
        assert_eq!(plan.files()[0].parent_key(), plan.files()[1].parent_key());
        assert_ne!(plan.files()[0].parent_key(), plan.files()[2].parent_key());
        let OutputEntry::Directory(a) = plan.root().child(OsStr::new("a")).unwrap() else {
            panic!()
        };
        let OutputEntry::File(one) = a.child(OsStr::new("one")).unwrap() else {
            panic!()
        };
        assert_eq!(one.path, Path::new("a/one"));
        assert!(a.child(OsStr::new("missing")).is_none());
    }
}
