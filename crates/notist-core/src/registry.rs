//! Atomic registration and package-scoped content-function lookup.

use crate::definitions::{DefinitionModule, FunctionDef, FunctionId, validate_module};
use crate::diag::{Diagnostic, Phase};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Registry {
    packages: BTreeMap<String, BTreeMap<String, FunctionDef>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    Unknown,
    UnsupportedPath,
}

impl Registry {
    /// An empty registry. `builtins::registry()` provides the standard environment.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, module: DefinitionModule) -> Result<(), Vec<Diagnostic>> {
        self.install(module, false)
    }

    pub(crate) fn install(
        &mut self,
        module: DefinitionModule,
        builtin: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut diagnostics = validate_module(&module);
        if module.package == "notist" && !builtin {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                module.span,
                "package name `notist` is reserved",
            ));
        }
        if self.packages.contains_key(&module.package) {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                module.span,
                format!("package `{}` is already registered", module.package),
            ));
        }
        for function in &module.functions {
            let target = match function.children {
                crate::builtins::Accepts::Rows => Some("row"),
                crate::builtins::Accepts::Cells => Some("cell"),
                crate::builtins::Accepts::Items => Some("item"),
                _ => None,
            };
            if let Some(target) = target
                && self.function("notist", target).is_none()
                && !module
                    .functions
                    .iter()
                    .any(|f| f.id.package == "notist" && f.id.name == target)
            {
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    function.span,
                    format!("children contract references missing function `notist::{target}`"),
                ));
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        let functions = module
            .functions
            .into_iter()
            .map(|function| (function.id.name.clone(), function))
            .collect();
        self.packages.insert(module.package, functions);
        Ok(())
    }

    pub fn get(&self, id: &FunctionId) -> Option<&FunctionDef> {
        self.function(&id.package, &id.name)
    }

    pub fn function(&self, package: &str, name: &str) -> Option<&FunctionDef> {
        self.packages.get(package)?.get(name)
    }

    /// Bare names consult only the builtin prelude. Longer module paths are
    /// retained by syntax but are not resolved by the current package model.
    pub fn resolve(&self, path: &str) -> Result<&FunctionDef, LookupError> {
        let mut segments = path.split("::");
        let first = segments.next().unwrap();
        let (package, name) = match segments.next() {
            Some(name) => (first, name),
            None => ("notist", first),
        };
        if segments.next().is_some() {
            return Err(LookupError::UnsupportedPath);
        }
        self.function(package, name).ok_or(LookupError::Unknown)
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionDef> {
        self.packages
            .values()
            .flat_map(|functions| functions.values())
    }
}
