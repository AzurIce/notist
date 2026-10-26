use crate::Error;
use notist::publication::url::encode_path;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// A source document keeps its relative path and only changes its extension.
#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub source: PathBuf,
    pub path: PathBuf,
    pub output: PathBuf,
    pub url: String,
}
impl Route {
    pub(crate) fn new(source: PathBuf, root: &Path) -> Result<Self, Error> {
        let path = source
            .strip_prefix(root)
            .map_err(|e| Error::Message(e.to_string()))?
            .to_path_buf();
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || path.starts_with("_notist")
        {
            return Err(Error::Message(format!(
                "invalid content path `{}`",
                path.display()
            )));
        }
        let output = path.with_extension("html");
        let public = if output.file_name().is_some_and(|s| s == "index.html") {
            let directory = encode_path(output.parent().unwrap());
            if directory.is_empty() {
                "/".into()
            } else {
                format!("/{directory}/")
            }
        } else {
            format!("/{}", encode_path(&output))
        };
        Ok(Self {
            source,
            path,
            output,
            url: public,
        })
    }
}
