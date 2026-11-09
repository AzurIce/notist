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

/// Ordinary template data, distinct from bit-exact component/IR wire values.
/// Resource identities and private data views are never exposed to templates.
pub fn presentation_value(value: &crate::Value) -> serde_json::Value {
    use crate::Value;
    use serde_json::{Value as Json, json};
    match value {
        Value::Unit | Value::None | Value::Path(_) | Value::Data(_) | Value::DataError(_) => {
            Json::Null
        }
        Value::Bool(value) => json!(value),
        Value::Int(value) => json!(value),
        Value::Float(value) => json!(value),
        Value::Str(value) => json!(value.as_str()),
        Value::Array(values) => Json::Array(values.iter().map(presentation_value).collect()),
        Value::Dict(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_owned(), presentation_value(value)))
                .collect(),
        ),
        Value::Some(value) => presentation_value(value),
        Value::Ok(value) => json!({"ok": presentation_value(value)}),
        Value::Err(value) => json!({"err": presentation_value(value)}),
    }
}

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
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
    sources: BTreeMap<PathBuf, PathBuf>,
    output: PathBuf,
}
impl Assets {
    pub fn finish(mut self) -> Result<BTreeMap<PathBuf, Vec<u8>>, Error> {
        self.generated(
            "_notist/publication.json".into(),
            br#"{"format":"notist-publication","version":1}"#.to_vec(),
        )?;
        self.files
            .into_iter()
            .map(|(path, bytes)| match bytes {
                Some(bytes) => Ok((path, bytes)),
                None => Err(Error::Message(format!(
                    "unwritten reserved output `{}`",
                    path.display()
                ))),
            })
            .collect()
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
    /// Validate a known generated destination before constructing its bytes.
    pub fn reserve_generated(&mut self, path: PathBuf) -> Result<(), Error> {
        self.insert(path, None)
    }
    /// Fill exactly one reserved destination, without repeating path planning.
    pub fn write_reserved(&mut self, path: &Path, bytes: Vec<u8>) -> Result<(), Error> {
        match self.files.get_mut(path) {
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
        // Check exact/ancestor files, then the first possible descendant in the
        // ordered map. A full scan here makes publishing N pages quadratic.
        let collision = path
            .ancestors()
            .any(|parent| self.files.contains_key(parent))
            || self
                .files
                .range::<Path, _>((
                    std::ops::Bound::Included(path.as_path()),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .is_some_and(|(previous, _)| previous.starts_with(&path));
        if collision {
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
            let definition_module = &environment.modules()[&component.id.module_id()];
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
    use super::{Assets, presentation_value};

    #[test]
    fn template_values_preserve_scalars_and_variants_without_exporting_resources() {
        use crate::Value;
        let private = crate::ResourcePath::resolve("/private", "doc.not", "data.txt").unwrap();
        let values: Value = Value::Array(
            vec![
                Value::Unit,
                Value::None,
                Value::Bool(false),
                Value::Int(i64::MAX),
                Value::Float(f64::INFINITY),
                Value::Str("中文 \"<&>".into()),
                Value::Some(Box::new(Value::Ok(Box::new(Value::Err(Box::new(
                    Value::Path(private),
                )))))),
            ]
            .into(),
        );
        assert_eq!(
            presentation_value(&values),
            serde_json::json!([null, null, false, i64::MAX, null, "中文 \"<&>", {"ok":{"err":null}}])
        );
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
}
