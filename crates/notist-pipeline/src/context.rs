//! Explicit semantic inputs shared by content analysis and output transforms.
use notist_core::{builtins, diag::Diagnostic, registry::Registry};

use crate::transforms::{Replace, TransformPlan};

/// Installed function definitions and a transform plan checked against them.
///
/// Context construction performs no IO. Hosts load configuration and Code
/// sources, register their definitions, then provide this value to the pipeline.
/// The registry is immutable after construction so the plan cannot become stale.
#[derive(Debug, Clone)]
pub struct Context {
    registry: Registry,
    transforms: TransformPlan,
    program: crate::definitions::CodeProgram,
}

impl Context {
    /// Use an explicitly assembled registry with no output transforms.
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            transforms: TransformPlan::default(),
            program: Default::default(),
        }
    }

    pub fn with_module(
        mut self,
        module: crate::definitions::CodeModule,
    ) -> Result<Self, Vec<Diagnostic>> {
        self.registry.register(module.definitions)?;
        self.program.bodies.extend(module.program.bodies);
        self.program.sources.extend(module.program.sources);
        self.program.modules.extend(module.program.modules);
        Ok(self)
    }

    /// Replace a source module for tooling, retaining dependencies and validating
    /// the transform plan against the updated signatures before returning it.
    pub fn replace_module(
        mut self,
        module: crate::definitions::CodeModule,
    ) -> Result<Self, Vec<Diagnostic>> {
        let package = &module.definitions.package;
        self.registry.replace(module.definitions.clone())?;
        self.transforms = self.transforms.revalidate(&self.registry)?;
        self.program.bodies.retain(|id, _| &id.package != package);
        if let Some(path) = self.program.modules.remove(package) {
            if !self.program.modules.values().any(|p| p == &path) {
                self.program.sources.remove(&path);
            }
        }
        self.program.bodies.extend(module.program.bodies);
        self.program.sources.extend(module.program.sources);
        self.program.modules.extend(module.program.modules);
        Ok(self)
    }
    pub fn program(&self) -> &crate::definitions::CodeProgram {
        &self.program
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn transforms(&self) -> &TransformPlan {
        &self.transforms
    }

    /// Compile output rules against this context's installed definitions.
    /// Invalid rules return diagnostics instead of a partially updated context.
    pub fn with_transforms(mut self, rules: &[Replace]) -> Result<Self, Vec<Diagnostic>> {
        self.transforms = TransformPlan::compile(rules, &self.registry)?;
        Ok(self)
    }
}

impl Default for Context {
    /// Standard built-in definitions and no output transforms.
    fn default() -> Self {
        Self::new(builtins::registry().clone())
    }
}
