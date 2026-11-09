//! Source navigation uses the same lowered calls and lexical bindings as checking.
use crate::{CodeModule, Context, Pipeline};
use notist_core::{
    definitions::{FunctionDef, FunctionId, Import, ImportKind},
    expr::Expr,
};
use rowan::TextRange;
use std::collections::BTreeSet;

/// Resolved source targets for one immutable source/environment pair. Building
/// this index never evaluates code or runs document extensions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceNavigation {
    targets: Vec<(TextRange, FunctionId)>,
}

impl SourceNavigation {
    /// Reuse the original lowered source, before extension-generated edits.
    pub fn from_lowered(
        output: &notist_core::frontend::FrontendOutput,
        source: &str,
        context: &Context,
    ) -> Self {
        Self::document(&output.imports, &output.forest, source, context)
    }

    /// Reuse the module already loaded for checking, including its source text.
    pub fn from_module(module: &CodeModule, context: &Context) -> Self {
        let mut navigation = Self::default();
        navigation.imports(&module.imports, context);
        for definition in &module.definitions.functions {
            let Some(body) = module.body_by_id(definition.id.def_id()) else {
                continue;
            };
            let bound = definition
                .parameters
                .iter()
                .map(|p| p.name.to_string())
                .chain(definition.comptime.iter().map(|p| p.name.clone()))
                .chain(
                    definition
                        .children_type
                        .is_some()
                        .then(|| "children".into()),
                )
                .collect();
            if let Some(expression) = &body.expression {
                navigation.expression(expression, module.source(), context, &bound);
            }
        }
        navigation
    }

    pub fn definition_at<'a>(
        &self,
        context: &'a Context,
        offset: usize,
    ) -> Option<&'a FunctionDef> {
        let (_, id) = self
            .targets
            .iter()
            .find(|(span, _)| span.contains((offset as u32).into()))?;
        context.registry().get(id)
    }

    fn document(imports: &[Import], forest: &[Expr], source: &str, context: &Context) -> Self {
        let mut navigation = Self::default();
        navigation.imports(imports, context);
        for expression in forest {
            navigation.expression(expression, source, context, &BTreeSet::new());
        }
        navigation
    }

    fn imports(&mut self, imports: &[Import], context: &Context) {
        for import in imports {
            if let ImportKind::Members(members) = &import.kind {
                for member in members {
                    if let Some(definition) = context.resolve(&member.alias) {
                        self.targets.push((member.span, definition.id.clone()));
                    }
                }
            }
        }
    }

    fn expression(
        &mut self,
        expr: &Expr,
        source: &str,
        context: &Context,
        bound: &BTreeSet<String>,
    ) {
        visit_calls(expr, source, bound, &mut |span, name| {
            if let Some(definition) = context.resolve_in_id(context.root_id(), name) {
                self.targets.push((span, definition.id.clone()));
            }
        });
    }
}

impl Pipeline {
    /// Resolve an explicit call target or imported function at a byte offset.
    /// The host supplies the loaded context for this exact source snapshot.
    pub fn definition_at<'a>(
        &self,
        path: &str,
        source: &str,
        context: &'a Context,
        offset: usize,
    ) -> Option<&'a FunctionDef> {
        self.source_navigation(path, source, context)?
            .definition_at(context, offset)
    }

    /// Prepare navigation once, then share it across editor queries.
    pub fn source_navigation(
        &self,
        path: &str,
        source: &str,
        context: &Context,
    ) -> Option<SourceNavigation> {
        let scope = context.root();
        if path.ends_with(".notc") {
            let module = self
                .compile_module("content.not", scope.unwrap_or(path), path, source)
                .ok()?;
            Some(SourceNavigation::from_module(&module, context))
        } else {
            let lowered = self.lower(path, source, Default::default()).ok()?;
            Some(SourceNavigation::from_lowered(&lowered, source, context))
        }
    }
}

fn visit_calls(
    expr: &Expr,
    source: &str,
    bound: &BTreeSet<String>,
    visit: &mut impl FnMut(TextRange, &str),
) {
    match expr {
        Expr::Call {
            callee,
            args,
            comptime,
            children: body,
            ..
        } => {
            if let Some(name) = callee.name_path() {
                let root = name.split('.').next().unwrap_or("");
                let reserved = name.contains('.') && matches!(root, "self" | "notist");
                // Generated sugar has a synthetic callee range; only explicit
                // source names are navigation targets.
                if (!bound.contains(root) || reserved)
                    && source
                        .get(usize::from(callee.span().start())..usize::from(callee.span().end()))
                        == Some(name.as_str())
                {
                    visit(callee.span(), &name);
                }
            }
            for argument in args.iter().chain(comptime) {
                visit_calls(argument.value(), source, bound, visit);
            }
            for child in body {
                visit_calls(child, source, bound, visit);
            }
        }
        Expr::Block {
            bindings, value, ..
        } => {
            let mut bound = bound.clone();
            for (name, value) in bindings {
                visit_calls(value, source, &bound, visit);
                bound.insert(name.clone());
            }
            visit_calls(value, source, &bound, visit);
        }
        Expr::Content { forest, .. }
        | Expr::Fragment {
            children: forest, ..
        }
        | Expr::Array(forest, _) => {
            for child in forest {
                visit_calls(child, source, bound, visit);
            }
        }
        Expr::If {
            condition, yes, no, ..
        } => {
            for child in [condition.as_ref(), yes.as_ref(), no.as_ref()] {
                visit_calls(child, source, bound, visit);
            }
        }
        Expr::Match { subject, arms, .. } => {
            visit_calls(subject, source, bound, visit);
            for (pattern, value) in arms {
                visit_calls(pattern, source, bound, visit);
                visit_calls(value, source, bound, visit);
            }
        }
        Expr::Binary { left, right, .. } => {
            visit_calls(left, source, bound, visit);
            visit_calls(right, source, bound, visit);
        }
        Expr::Unary { value, .. } | Expr::Comptime { value, .. } => {
            visit_calls(value, source, bound, visit);
        }
        Expr::Dict(entries, _) => {
            for (_, value) in entries {
                visit_calls(value, source, bound, visit);
            }
        }
        _ => {}
    }
}
