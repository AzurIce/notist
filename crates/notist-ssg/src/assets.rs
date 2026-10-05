use crate::{
    Error,
    routes::{encode_path, relative_url},
};
use notist::{Environment, FunctionId, Resources};
use notist_html::Component;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Exact source identities for published resources, with shared destinations.
#[derive(Default)]
pub(crate) struct Assets {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    sources: BTreeMap<PathBuf, PathBuf>,
    packages: BTreeMap<String, PathBuf>,
    output: PathBuf,
}
impl Assets {
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            ..Self::default()
        }
    }
    pub fn generated(&mut self, path: PathBuf, bytes: Vec<u8>) -> Result<(), Error> {
        if self
            .files
            .keys()
            .any(|previous| previous.starts_with(&path) || path.starts_with(previous))
        {
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
            Some(notist::resources::ResourceKind::Directory) => {
                for entry in resources.entries(source)? {
                    self.copy(
                        resources,
                        &entry,
                        &destination.join(entry.file_name().unwrap()),
                    )?;
                }
            }
            Some(notist::resources::ResourceKind::File) => {
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
            let package = &environment.packages()[&component.id.package];
            if let Some(previous) = self
                .packages
                .insert(package.name.clone(), package.root.clone())
                && previous != package.root
            {
                return Err(Error::Message(format!(
                    "site package `{}` has conflicting resource roots `{}` and `{}`",
                    package.name,
                    previous.display(),
                    package.root.display()
                )));
            }
            let source = component.module.resource().ok_or_else(|| {
                Error::Message(format!(
                    "cannot publish URL-only component `{}`",
                    component.id
                ))
            })?;
            let relative = source
                .strip_prefix(&package.root)
                .map_err(|_| Error::Message("component is outside its package".into()))?;
            let destination = PathBuf::from("_notist/packages")
                .join(notist_html::components::encode_name(&package.name))
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

/// Stable file URL encoding for publication hosts.
pub fn file_url(path: &Path) -> String {
    encode_path(path)
}
