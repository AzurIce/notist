//! Logical resource identities. Resolution is lexical and never performs IO.

/// An immutable path within a host-declared logical resource root.
/// A path carries no input revision, access permission or published URL.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ResourcePath {
    root: String,
    path: String,
}

impl ResourcePath {
    /// Resolve a literal at its defining source. Relative source identities are
    /// relative to `root`; a leading slash in the literal also denotes `root`.
    pub fn resolve(root: &str, source: &str, literal: &str) -> Result<Self, String> {
        validate(literal)?;
        if literal.is_empty() {
            return Err(
                "path literal must not be empty; use `p\".\"` for the current directory".into(),
            );
        }
        let root = normalized_root(root)?;
        if source.is_empty() || (source.starts_with('<') && source.ends_with('>')) {
            return Err("path literal requires a source identity".into());
        }
        validate(source)?;
        if matches!(source.rsplit('/').next(), Some("" | "." | "..")) {
            return Err("path literal requires a source file identity".into());
        }
        let source = if source.starts_with('/') {
            normalize(source.split('/'))?
        } else {
            normalize(root.split('/').chain(source.split('/')))?
        };
        let root_parts: Vec<_> = root.split('/').filter(|s| !s.is_empty()).collect();
        if !source.starts_with(&root_parts) {
            return Err("source is outside its logical resource root".into());
        }
        if source.len() == root_parts.len() {
            return Err("path literal requires a source file identity".into());
        }
        let mut parts = if literal.starts_with('/') {
            Vec::new()
        } else {
            source[root_parts.len()..].to_vec()
        };
        if !literal.starts_with('/') {
            parts.pop();
        }
        for part in literal.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err("path would escape its logical resource root".into());
                    }
                }
                part => parts.push(part),
            }
        }
        let path = parts.join("/");
        Ok(Self { root, path })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// The normalized, root-relative path, without a leading slash.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The logical host path, not necessarily an operating-system filename.
    pub fn logical_path(&self) -> String {
        if self.root == "/" {
            format!("/{}", self.path)
        } else if self.path.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{}", self.root, self.path)
        }
    }

    /// Resolve a legacy local URL reference for an explicit embed operation.
    /// Out-of-root identities remain untrusted: publication must authorize them
    /// against the Vault or its mounts. Path literals keep strict `resolve` rules.
    pub(crate) fn resolve_reference(root: &str, source: &str, path: &str) -> Result<Self, String> {
        if source.is_empty() || (source.starts_with('<') && source.ends_with('>')) {
            return Err("embed requires a source identity".into());
        }
        let path = if path.is_empty() {
            source.rsplit('/').next().unwrap_or("")
        } else {
            path
        };
        if let Ok(resolved) = Self::resolve(root, source, path) {
            return Ok(resolved);
        }
        let root = normalized_root(root)?;
        let source = if source.starts_with('/') {
            source.to_owned()
        } else {
            format!("{root}/{source}")
        };
        let path = if path.starts_with('/') {
            format!("{root}/{}", path.trim_start_matches('/'))
        } else {
            path.to_owned()
        };
        Self::resolve("/", &source, &path)
    }
}

fn validate(path: &str) -> Result<(), String> {
    if path.chars().any(char::is_control) {
        return Err("resource paths must not contain control characters".into());
    }
    if path.contains('\\') {
        return Err("resource paths use forward slashes, not backslashes".into());
    }
    if path
        .split('/')
        .next()
        .is_some_and(|part| part.contains(':'))
    {
        return Err("resource paths are logical paths, not URLs or drive paths".into());
    }
    Ok(())
}

fn normalize<'a>(parts: impl Iterator<Item = &'a str>) -> Result<Vec<&'a str>, String> {
    let mut result = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                if result.pop().is_none() {
                    return Err("path would escape its logical resource root".into());
                }
            }
            part => result.push(part),
        }
    }
    Ok(result)
}

fn normalized_root(root: &str) -> Result<String, String> {
    validate(root)?;
    if !root.starts_with('/') {
        return Err("logical resource root must be absolute".into());
    }
    let parts = normalize(root.split('/'))?;
    Ok(format!("/{}", parts.join("/")))
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ResourcePath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            root: String,
            path: String,
        }
        let stored = Stored::deserialize(deserializer)?;
        let path = Self::resolve(&stored.root, "__source__", &format!("/{}", stored.path))
            .map_err(serde::de::Error::custom)?;
        if path.root != stored.root || path.path != stored.path {
            return Err(serde::de::Error::custom("resource path must be normalized"));
        }
        Ok(path)
    }
}
