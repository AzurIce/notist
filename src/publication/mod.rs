//! Resource planning shared by HTML document exports and book publication.
//! This module returns owned files; filesystem mutation belongs to the host.
use self::url::relative_url;
use crate::{Environment, FunctionId, Resources, resources::ResourceError};
use notist_html::Component;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use std::fmt;

pub mod url;

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
    files: BTreeMap<PathBuf, Vec<u8>>,
    sources: BTreeMap<PathBuf, PathBuf>,
    output: PathBuf,
}
impl Assets {
    pub fn finish(mut self) -> Result<BTreeMap<PathBuf, Vec<u8>>, Error> {
        self.generated(
            "_notist/publication.json".into(),
            br#"{"format":"notist-publication","version":1}"#.to_vec(),
        )?;
        Ok(self.files)
    }
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            ..Self::default()
        }
    }
    pub fn generated(&mut self, path: PathBuf, bytes: Vec<u8>) -> Result<(), Error> {
        // Check exact/ancestor files, then the first possible descendant in the
        // ordered map. A full scan here makes publishing N pages quadratic.
        let ancestor_exists = path
            .ancestors()
            .any(|parent| self.files.contains_key(parent));
        let descendant_exists = self
            .files
            .range(path.clone()..)
            .next()
            .is_some_and(|(previous, _)| previous.starts_with(&path));
        if ancestor_exists || descendant_exists {
            return Err(Error::Message(format!(
                "output collision at `{}`",
                path.display()
            )));
        }
        self.files.insert(path, bytes);
        Ok(())
    }
    pub fn copy(
        &mut self,
        resources: &(impl Resources + ?Sized),
        source: &Path,
        destination: &Path,
    ) -> Result<(), Error> {
        if self.output.starts_with(source) || source.starts_with(&self.output) {
            return Err(Error::Message(format!(
                "resource source and site output overlap: {}",
                source.display()
            )));
        }
        if let Some(previous) = self.sources.get(destination) {
            if previous == source {
                return Ok(());
            }
            return Err(Error::Message(format!(
                "resource collision at `{}` between `{}` and `{}`",
                destination.display(),
                previous.display(),
                source.display()
            )));
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
                self.generated(destination.into(), bytes)?;
                self.sources.insert(destination.into(), source.into());
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
    pub fn components(
        &mut self,
        resources: &(impl Resources + ?Sized),
        environment: &Environment,
        components: &[Component],
        page: &Path,
    ) -> Result<String, Error> {
        let mut registrations = String::new();
        let mut tags = BTreeMap::<String, (FunctionId, PathBuf)>::new();
        for (index, component) in components.iter().enumerate() {
            let definition_module = &environment.modules()[&component.id.module];
            let root = definition_module.path.parent().unwrap();
            let source = component.module.resource().ok_or_else(|| {
                Error::Message(format!(
                    "cannot publish URL-only component `{}`",
                    component.id
                ))
            })?;
            let relative = source.strip_prefix(root).map_err(|_| {
                Error::Message("component is outside its defining module resource root".into())
            })?;
            let destination = PathBuf::from("_notist/modules")
                .join(notist_html::components::module_key(&component.id.module))
                .join(relative);
            if let Some(previous) =
                tags.insert(component.tag.clone(), (component.id.clone(), source.into()))
            {
                if previous != (component.id.clone(), source.into()) {
                    return Err(Error::Message(format!(
                        "conflicting component tag `{}`",
                        component.tag
                    )));
                }
                continue;
            }
            if source.file_name().is_some_and(|name| name == "index.js") {
                self.copy(
                    resources,
                    source.parent().unwrap(),
                    destination.parent().unwrap(),
                )?;
            } else {
                self.copy(resources, source, &destination)?;
            }
            let url = relative_url(page, &destination, false);
            let url = if url.starts_with('.') {
                url
            } else {
                format!("./{url}")
            };
            let url = serde_json::to_string(&url).unwrap();
            let tag = serde_json::to_string(&component.tag).unwrap();
            registrations.push_str(&format!("import Component{index} from {url};\n{{ const tag = {tag}; const existing = customElements.get(tag); if (existing && existing !== Component{index}) throw new Error(`Conflicting Notist component: ${{tag}}`); if (!existing) customElements.define(tag, Component{index}); }}\n"));
        }
        Ok(registrations)
    }
}

#[cfg(test)]
mod tests {
    use super::Assets;

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
}
