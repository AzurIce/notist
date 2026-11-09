//! Pruned directory watches. Never ask a backend to recursively traverse a tree:
//! pruning events afterward is too late to avoid symlinks and watch exhaustion.
use super::Error;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use notify::{Event, EventKind, RecursiveMode, Watcher, event::ModifyKind};
use notist::resources::normalize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(super) struct Plan {
    pub directories: BTreeSet<PathBuf>,
    root: PathBuf,
    content: Vec<(PathBuf, PathBuf)>,
    auxiliary: Vec<PathBuf>,
    exclude: GlobSet,
    outputs: BTreeSet<PathBuf>,
    files: BTreeSet<PathBuf>,
    inspected: BTreeSet<PathBuf>,
}

impl Plan {
    pub fn new(
        target: &crate::package::Package,
        observed: &BTreeSet<PathBuf>,
        previous_output: Option<&Path>,
    ) -> Self {
        let mut plan = Self {
            directories: BTreeSet::new(),
            root: target.root.clone(),
            content: Vec::new(),
            auxiliary: Vec::new(),
            exclude: GlobSet::empty(),
            outputs: previous_output.into_iter().map(Path::to_path_buf).collect(),
            files: target.inputs(),
            inspected: BTreeSet::new(),
        };
        // Configuration failures still need the manifest's parent watched so
        // repairing the configuration can recover without restarting serve.
        if let Ok(prepared) = target.prepare() {
            let config = prepared.options;
            plan.outputs
                .insert(normalize(&target.root.join(&config.output)));
            let mut builder = GlobSetBuilder::new();
            let compiled = config
                .exclude
                .iter()
                .try_for_each(|pattern| {
                    builder.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
                    Ok::<_, globset::Error>(())
                })
                .and_then(|_| builder.build());
            if let Ok(exclude) = compiled {
                plan.exclude = exclude;
                plan.content.push((
                    normalize(&target.root.join(&config.content)),
                    PathBuf::new(),
                ));
                for mount in &config.mounts {
                    plan.content.push((
                        normalize(&target.root.join(&mount.source)),
                        mount.path.clone(),
                    ));
                }
                for path in [
                    &config.templates,
                    &config.static_dir,
                    &config.data,
                    &config.sass,
                ] {
                    plan.auxiliary.push(normalize(&target.root.join(path)));
                }
                if let Some(theme) = &config.theme
                    && !theme
                        .to_str()
                        .is_some_and(|name| name.starts_with("builtins."))
                {
                    plan.auxiliary.push(normalize(&target.root.join(theme)));
                }
                for extension in config.extensions.commands.values() {
                    for path in &extension.watch {
                        plan.auxiliary.push(normalize(&target.root.join(path)));
                    }
                }
            }
        }
        for (source, _) in plan.content.clone() {
            plan.anchor(&source);
            plan.walk(&source, false);
        }
        for source in plan.auxiliary.clone() {
            plan.anchor(&source);
            plan.walk(&source, true);
        }
        for input in observed {
            if plan.output(input) {
                continue;
            }
            if regular_directory(input) {
                // Observing a directory listing does not authorize traversal
                // of its children, especially a package root or excluded tree.
                if !plan.content_ignored(input, true) && !hidden_in(&plan.root, input) {
                    plan.directories.insert(input.clone());
                    plan.inspected.insert(input.clone());
                }
            } else {
                // A concrete read (including hidden data or an excluded source
                // imported as a module) remains an actual build dependency.
                plan.files.insert(input.clone());
            }
        }
        for input in plan.files.clone() {
            plan.anchor(&input);
        }
        plan
    }

    fn output(&self, path: &Path) -> bool {
        self.outputs.iter().any(|output| path.starts_with(output))
    }

    fn mapped(&self, path: &Path) -> Option<PathBuf> {
        self.content
            .iter()
            .filter_map(|(source, prefix)| {
                path.strip_prefix(source)
                    .ok()
                    .map(|relative| (source.components().count(), prefix.join(relative)))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, relative)| relative)
    }

    fn content_ignored(&self, path: &Path, directory: bool) -> bool {
        let Some(relative) = self.mapped(path) else {
            return false;
        };
        ignored(&relative)
            || relative.ancestors().enumerate().any(|(index, part)| {
                !part.as_os_str().is_empty()
                    && (self.exclude.is_match(part)
                        || (index > 0 || directory)
                            && self
                                .exclude
                                .is_match(format!("{}/", part.to_string_lossy())))
            })
    }

    fn anchor(&mut self, input: &Path) {
        if let Some(parent) = input.parent()
            && let Some(directory) = closest_directory(parent)
            && !self.output(&directory)
        {
            self.directories.insert(directory);
        }
    }

    fn walk(&mut self, root: &Path, auxiliary: bool) {
        let mut pending = vec![root.to_path_buf()];
        let mut visited = BTreeSet::new();
        while let Some(directory) = pending.pop() {
            if self.output(&directory)
                || !regular_directory(&directory)
                || !visited.insert(directory.clone())
                || if auxiliary {
                    hidden_in(root, &directory)
                } else {
                    self.content_ignored(&directory, true)
                }
            {
                continue;
            }
            self.directories.insert(directory.clone());
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                // DirEntry::file_type does not follow links, unlike is_dir().
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    pending.push(entry.path());
                }
            }
        }
    }

    pub fn relevant(&self, path: &Path) -> bool {
        if self.output(path) {
            return false;
        }
        if self.files.iter().any(|input| input.starts_with(path)) {
            return true;
        }
        for root in &self.auxiliary {
            if root.starts_with(path) || path.starts_with(root) && !hidden_in(root, path) {
                return true;
            }
        }
        if self.mapped(path).is_some() {
            return !self.content_ignored(path, path.is_dir());
        }
        if hidden_in(&self.root, path) {
            return false;
        }
        self.content.iter().any(|(root, _)| root.starts_with(path))
            || path
                .parent()
                .is_some_and(|parent| self.inspected.contains(parent))
    }
}

fn ignored(relative: &Path) -> bool {
    relative.iter().any(|part| {
        part.to_string_lossy().starts_with('.') || part == "target" || part == "node_modules"
    })
}
fn hidden_in(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(ignored)
}

/// Walk ancestors from the filesystem root, stopping before any symlink or
/// missing component. Never inspect descendants through an intermediate link.
fn closest_directory(path: &Path) -> Option<PathBuf> {
    let mut closest = None;
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if !std::fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_dir())
        {
            break;
        }
        closest = Some(ancestor.to_path_buf());
    }
    closest
}
fn regular_directory(path: &Path) -> bool {
    closest_directory(path).as_deref() == Some(path)
}

#[derive(Default)]
pub(super) struct Watches {
    active: BTreeSet<PathBuf>,
    retired: BTreeSet<PathBuf>,
}
impl Watches {
    pub fn observe(&mut self, event: &Result<Event, notify::Error>) {
        if let Ok(event) = event
            && matches!(
                event.kind,
                EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
            )
        {
            self.retired.extend(
                self.active
                    .iter()
                    .filter(|directory| event.paths.iter().any(|path| directory.starts_with(path)))
                    .cloned(),
            );
        }
    }

    pub fn sync(&mut self, watcher: &mut impl Watcher, plan: &Plan) -> Result<(), Error> {
        for directory in self.active.clone() {
            if !plan.directories.contains(&directory)
                || self.retired.contains(&directory)
                || !regular_directory(&directory)
            {
                if let Err(error) = watcher.unwatch(&directory)
                    && !matches!(
                        error.kind,
                        notify::ErrorKind::WatchNotFound | notify::ErrorKind::PathNotFound
                    )
                {
                    return Err(Error::Message(error.to_string()));
                }
                self.active.remove(&directory);
            }
        }
        self.retired.clear();
        for directory in &plan.directories {
            if !self.active.contains(directory) && regular_directory(directory) {
                watcher
                    .watch(directory, RecursiveMode::NonRecursive)
                    .map_err(|error| Error::Message(error.to_string()))?;
                self.active.insert(directory.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, source: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn package(root: &Path, site: &str) -> crate::package::Package {
        write(
            root,
            "Notist.toml",
            &format!("[package]\nname='watch'\n[site]\noutput='public'\n{site}"),
        );
        crate::package::Package::load(Some(&root.join("Notist.toml")), None, None).unwrap()
    }

    #[test]
    fn content_exclusions_prune_registration_and_events_not_just_publication() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let target = package(root, "exclude=['excluded/**', 'private', '**/cache/**']");
        for path in [
            "content/visible/nested/page.not",
            "content/excluded/deep/a.not",
            "content/private/deep/a.not",
            "content/visible/cache/deep/a.not",
            "content/.hidden/deep/a.not",
            "content/node_modules/deep/a.not",
            "content/target/deep/a.not",
            "public/deep/index.html",
        ] {
            write(root, path, "= Page");
        }
        // An observed root listing must not make its excluded descendants live.
        let observed = BTreeSet::from([root.join("content"), root.join("content/excluded")]);
        let plan = Plan::new(&target, &observed, None);
        assert!(
            plan.directories
                .contains(&root.join("content/visible/nested"))
        );
        for path in [
            "content/excluded",
            "content/private",
            "content/visible/cache",
            "content/.hidden",
            "content/target",
            "content/node_modules",
            "public",
        ] {
            assert!(
                !plan
                    .directories
                    .iter()
                    .any(|directory| directory.starts_with(root.join(path))),
                "{path}: {:?}",
                plan.directories
            );
            assert!(!plan.relevant(&root.join(path).join("new.not")), "{path}");
        }
        assert!(plan.relevant(&root.join("content/visible/new.not")));
        assert!(plan.relevant(&root.join("Notist.toml")));
    }

    #[test]
    fn mapped_mount_exclusions_use_content_namespace_and_configuration_can_change() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("site");
        write(&root, "content/index.not", "= Page");
        write(temp.path(), "models/private/deep/page.not", "= Private");
        write(temp.path(), "models/public/page.not", "= Public");
        let settings =
            "exclude=['art/private/**']\n[[site.mounts]]\nsource='../models'\npath='art'";
        let target = package(&root, settings);
        let plan = Plan::new(&target, &BTreeSet::new(), None);
        assert!(
            plan.directories
                .contains(&temp.path().join("models/public"))
        );
        assert!(
            !plan
                .directories
                .contains(&temp.path().join("models/private"))
        );
        assert!(!plan.relevant(&temp.path().join("models/private/deep/page.not")));
        assert!(plan.relevant(&temp.path().join("models/public/page.not")));
        write(
            &root,
            "Notist.toml",
            "[package]\nname='watch'\n[site]\noutput='public'\nexclude=['art/**']\n[[site.mounts]]\nsource='../models'\npath='art'",
        );
        let plan = Plan::new(&target, &BTreeSet::new(), None);
        assert!(!plan.directories.contains(&temp.path().join("models")));
        assert!(!plan.relevant(&temp.path().join("models/public/page.not")));
    }

    #[test]
    fn explicit_dependencies_keep_narrow_watches_without_reopening_ignored_trees() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let target = package(
            root,
            "exclude=['templates/**', 'excluded/**']\ncontent='.'\ntemplates='templates'",
        );
        write(root, "templates/nested/page.tera", "template");
        write(root, ".secret/value.txt", "data");
        write(root, ".secret/noise/deep/unused.txt", "noise");
        write(root, "excluded/lib.notc", "fn f() -> Int { 1 }");
        write(root, "excluded/noise/deep/unused.txt", "noise");
        let observed = BTreeSet::from([
            root.join(".secret/value.txt"),
            root.join("excluded/lib.notc"),
        ]);
        let plan = Plan::new(&target, &observed, None);
        for path in ["templates/nested", ".secret", "excluded"] {
            assert!(plan.directories.contains(&root.join(path)), "{path}");
        }
        for path in [".secret/noise", "excluded/noise"] {
            assert!(!plan.directories.contains(&root.join(path)), "{path}");
        }
        assert!(plan.relevant(&root.join(".secret/value.txt")));
        assert!(plan.relevant(&root.join("excluded/lib.notc")));
        assert!(!plan.relevant(&root.join(".secret/unused.txt")));
        assert!(!plan.relevant(&root.join("excluded/unused.txt")));
        assert!(plan.relevant(&root.join("templates/nested/page.tera")));
    }

    #[test]
    fn missing_roots_watch_only_the_nearest_existing_parent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("site");
        let target = package(&root, "content='missing/deep'");
        let plan = Plan::new(&target, &BTreeSet::new(), None);
        assert!(plan.directories.contains(&root));
        assert!(plan.relevant(&root.join("missing")));
        assert!(!plan.directories.contains(temp.path()));
        write(&root, "missing/deep/new/page.not", "= Ready");
        let plan = Plan::new(&target, &BTreeSet::new(), None);
        assert!(plan.directories.contains(&root.join("missing/deep/new")));
    }

    #[test]
    #[cfg(unix)]
    fn wine_symlinks_and_cycles_never_enter_the_watch_plan() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("site");
        let target = package(&root, "content='.'");
        write(&root, "index.not", "= Ready");
        write(temp.path(), "outside/many/deep/value.txt", "outside");
        std::fs::create_dir_all(root.join(".ci/pfx/dosdevices")).unwrap();
        symlink("/", root.join(".ci/pfx/dosdevices/z:")).unwrap();
        symlink(temp.path().join("outside"), root.join("visible-link")).unwrap();
        symlink(&root, root.join("cycle")).unwrap();
        // Even a failed build may have observed a rejected symlink input.
        let observed = BTreeSet::from([root.join("visible-link/many/deep/value.txt")]);
        let plan = Plan::new(&target, &observed, None);
        assert!(
            !plan
                .directories
                .iter()
                .any(|path| path.starts_with(root.join(".ci"))
                    || path.starts_with(root.join("visible-link"))
                    || path.starts_with(root.join("cycle"))
                    || path.starts_with(temp.path().join("outside")))
        );
    }
}
