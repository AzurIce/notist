//! Component identities for relative output files; exact lookup is separate
//! from file/ancestor/descendant collision rules.
use std::{
    collections::{BTreeMap, HashMap},
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

#[derive(Default)]
struct Node {
    children: HashMap<OsString, usize>,
    file: Option<usize>,
}

pub(super) struct Index {
    nodes: Vec<Node>,
    // Preserve the complete std::Path behavior for unusual API inputs. Normal
    // publication-shaped paths never allocate or consult this compatibility map.
    unusual: Option<BTreeMap<PathBuf, usize>>,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            nodes: vec![Node::default()],
            unusual: None,
        }
    }
}

pub(super) fn ordinary(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

impl Index {
    pub fn insert<'a>(
        &mut self,
        path: &Path,
        file: usize,
        existing: impl IntoIterator<Item = (&'a Path, usize)>,
    ) -> Result<(), ()> {
        if self.unusual.is_none() && !ordinary(path) {
            self.unusual = Some(existing.into_iter().map(|(p, id)| (p.into(), id)).collect());
        }
        if let Some(paths) = &mut self.unusual {
            let collision = path.ancestors().any(|parent| paths.contains_key(parent))
                || paths
                    .range::<Path, _>((std::ops::Bound::Included(path), std::ops::Bound::Unbounded))
                    .next()
                    .is_some_and(|(previous, _)| previous.starts_with(path));
            if collision {
                return Err(());
            }
            paths.insert(path.into(), file);
            return Ok(());
        }
        let mut node = 0;
        for part in path.components() {
            if self.nodes[node].file.is_some() {
                return Err(());
            }
            let name = part.as_os_str();
            node = if let Some(&child) = self.nodes[node].children.get(name) {
                child
            } else {
                let child = self.nodes.len();
                self.nodes.push(Node::default());
                self.nodes[node].children.insert(name.into(), child);
                child
            };
        }
        if self.nodes[node].file.is_some() || !self.nodes[node].children.is_empty() {
            return Err(());
        }
        self.nodes[node].file = Some(file);
        Ok(())
    }
    pub fn get(&self, path: &Path) -> Option<usize> {
        if let Some(paths) = &self.unusual {
            return paths.get(path).copied();
        }
        self.node(path).and_then(|node| self.nodes[node].file)
    }
    pub fn node(&self, path: &Path) -> Option<usize> {
        let mut node = 0;
        for part in path.components() {
            let Component::Normal(name) = part else {
                return None;
            };
            node = *self.nodes[node].children.get(name)?;
        }
        Some(node)
    }
    pub fn child(&self, node: usize, name: &OsStr) -> Option<usize> {
        self.nodes[node].children.get(name).copied()
    }
    pub fn file(&self, node: usize) -> Option<usize> {
        self.nodes[node].file
    }
    pub fn ordered_files(&self) -> Vec<usize> {
        if let Some(paths) = &self.unusual {
            return paths.values().copied().collect();
        }
        // Standard relative Path order is lexicographic component order. Sort
        // sibling names once, not complete shared-prefix paths on every insert.
        let mut files = Vec::new();
        let mut pending = vec![0];
        while let Some(node) = pending.pop() {
            if let Some(file) = self.nodes[node].file {
                files.push(file);
            }
            let mut children: Vec<_> = self.nodes[node].children.iter().collect();
            children.sort_unstable_by(|a, b| a.0.cmp(b.0));
            pending.extend(children.into_iter().rev().map(|(_, &child)| child));
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare(paths: &[PathBuf]) {
        let mut old = BTreeMap::<PathBuf, usize>::new();
        let mut index = Index::default();
        for (id, path) in paths.iter().enumerate() {
            let collision = path.ancestors().any(|p| old.contains_key(p))
                || old
                    .range(path.clone()..)
                    .next()
                    .is_some_and(|(p, _)| p.starts_with(path));
            assert_eq!(
                index
                    .insert(path, id, old.iter().map(|(p, &i)| (p.as_path(), i)))
                    .is_err(),
                collision,
                "{paths:?}: {path:?}"
            );
            if !collision {
                old.insert(path.clone(), id);
            }
            for candidate in paths {
                assert_eq!(index.get(candidate), old.get(candidate).copied());
            }
            assert_eq!(
                index.ordered_files(),
                old.values().copied().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn component_index_matches_ordered_path_collision_semantics() {
        let names: Vec<PathBuf> = [
            "",
            ".",
            "./a",
            "a",
            "a/",
            "a//b",
            "a/./b",
            "a/b",
            "a/b/c",
            "a-more/b",
            "a.html",
            "a.html-more",
            "../a",
            "/",
            "/a",
            "/a/b",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        for a in &names {
            for b in &names {
                for c in &names {
                    compare(&[a.clone(), b.clone(), c.clone()]);
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_component_identity_is_not_lossy() {
        use std::os::unix::ffi::OsStringExt;
        let a = PathBuf::from(OsString::from_vec(b"dir/a\xff".to_vec()));
        let b = PathBuf::from(OsString::from_vec(b"dir/a\xfe".to_vec()));
        compare(&[a.clone(), b.clone(), a.join("child"), PathBuf::from("dir")]);
    }
}
