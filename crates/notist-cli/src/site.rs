//! Filesystem publication and source tracking shared by build, html and preview.
use notist::{
    FsResources, Resources, Vault,
    resources::{ResourceError, ResourceKind, normalize},
};
use notist_ssg::{Error, Site, SiteConfig};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub(crate) struct Files {
    base: FsResources,
    observed: Mutex<BTreeSet<PathBuf>>,
}
impl Files {
    pub fn new(root: &Path) -> Self {
        Self {
            base: FsResources::new(root),
            observed: Mutex::default(),
        }
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
}

pub(crate) struct Build {
    pub site: Site,
    pub output: PathBuf,
    pub observed: BTreeSet<PathBuf>,
}
pub fn assemble(
    root: &Path,
    config: Option<&Path>,
    output: Option<&Path>,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    let files = Files::new(root);
    let mut vault = Vault::new(files);
    if let Some(config) = config {
        vault = vault.with_config(normalize(&std::env::current_dir().unwrap().join(config)));
    }
    let result = (|| {
        let path = config
            .map(|path| normalize(&std::env::current_dir().unwrap().join(path)))
            .unwrap_or_else(|| root.join("Notist.toml"));
        let mut options = match vault.resources().kind(&path)? {
            Some(_) => SiteConfig::parse(&vault.resources().source(&path)?)
                .map_err(|error| Error::Message(format!("{}: {error}", path.display())))?,
            None if config.is_none() => SiteConfig::default(),
            None => return Err(ResourceError::NotFound(path).into()),
        };
        if let Some(output) = output {
            options.output = normalize(&std::env::current_dir().unwrap().join(output));
        }
        let output = vault.resources().resolve(&options.output);
        let site = notist_ssg::build(&mut vault, &options)?;
        Ok((site, output))
    })();
    let observed = vault.resources().observed();
    match result {
        Ok((site, output)) => Ok(Build {
            site,
            output,
            observed,
        }),
        Err(error) => Err((error, observed)),
    }
}

fn io(error: impl std::fmt::Display) -> Error {
    Error::Message(error.to_string())
}
/// Stage every file before replacing an owned output directory. A failed build
/// or publication leaves the previous site intact.
pub fn publish(site: &Site, output: &Path, root: &Path) -> Result<(), Error> {
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
        let owned = std::fs::read(output.join("_notist/site.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|manifest| {
                manifest["format"] == "notist-site" && manifest["version"] == 1
            });
        if !empty && !owned {
            return Err(Error::Message(format!(
                "refusing to replace nonempty output without a Notist site manifest: {}",
                output.display()
            )));
        }
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::Message("invalid site output".into()))?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let staging = tempfile::Builder::new()
        .prefix(".notist-stage-")
        .tempdir_in(parent)
        .map_err(io)?;
    for (relative, bytes) in &site.files {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(Error::Message("invalid publication file path".into()));
        }
        let file = staging.path().join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).map_err(io)?;
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
pub fn report(error: Error) {
    match error {
        Error::Diagnostics(errors) => {
            for error in errors {
                crate::emit_source(&error.path, &error.source, &[error.diagnostic]);
            }
        }
        error => eprintln!("{error}"),
    }
}
pub fn build(root: &Path, config: Option<&Path>, output: Option<&Path>) -> std::process::ExitCode {
    let result = (|| {
        let root = std::fs::canonicalize(root).map_err(io)?;
        let result = assemble(&root, config, output).map_err(|(error, _)| error)?;
        publish(&result.site, &result.output, &root)?;
        println!(
            "{} pages → {}",
            result.site.pages.len(),
            result.output.display()
        );
        Ok::<_, Error>(())
    })();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            report(error);
            std::process::ExitCode::FAILURE
        }
    }
}
