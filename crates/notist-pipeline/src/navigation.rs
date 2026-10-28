//! Source navigation uses the same lowered calls and lexical bindings as checking.
use crate::{Context, Pipeline};
use notist_core::{
    definitions::{FunctionDef, Import, ImportKind},
    expr::Expr,
};
use std::collections::BTreeSet;

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
        let scope = context.root();
        if path.ends_with(".notc") {
            let module = self
                .compile_module("content.not", scope.unwrap_or(path), path, source)
                .ok()?;
            if let Some(definition) = imported_at(&module.imports, context, offset) {
                return Some(definition);
            }
            for definition in &module.definitions.functions {
                let Some(body) = module.program.bodies.get(&definition.id) else {
                    continue;
                };
                let bound = definition
                    .parameters
                    .iter()
                    .map(|p| p.name.clone())
                    .chain(definition.comptime.iter().map(|p| p.name.clone()))
                    .chain(
                        definition
                            .children_type
                            .is_some()
                            .then(|| "children".into()),
                    )
                    .collect();
                if let Some(expression) = &body.expression
                    && let Some(name) = called_at(expression, source, offset, &bound)
                {
                    return context.resolve_in(scope, &name);
                }
            }
        } else {
            let lowered = self.lower(path, source, Default::default()).ok()?;
            if let Some(definition) = imported_at(&lowered.imports, context, offset) {
                return Some(definition);
            }
            for expression in &lowered.forest {
                if let Some(name) = called_at(expression, source, offset, &BTreeSet::new()) {
                    return context.resolve_in(scope, &name);
                }
            }
        }
        None
    }
}

fn imported_at<'a>(
    imports: &[Import],
    context: &'a Context,
    offset: usize,
) -> Option<&'a FunctionDef> {
    for import in imports {
        if let ImportKind::Members(members) = &import.kind {
            for member in members {
                if member.span.contains((offset as u32).into()) {
                    return context.resolve(&member.alias);
                }
            }
        }
    }
    None
}

fn called_at(expr: &Expr, source: &str, offset: usize, bound: &BTreeSet<String>) -> Option<String> {
    if !expr.span().contains((offset as u32).into()) {
        return None;
    }
    let mut children: Vec<&Expr> = Vec::new();
    match expr {
        Expr::Call {
            callee,
            args,
            comptime,
            children: body,
            ..
        } => {
            if callee.span().contains((offset as u32).into()) {
                let name = callee.name_path()?;
                let root = name.split('.').next()?;
                let reserved = name.contains('.') && matches!(root, "self" | "notist");
                // Generated sugar has a synthetic callee range; only explicit
                // source names are navigation targets.
                if (!bound.contains(root) || reserved)
                    && source
                        .get(usize::from(callee.span().start())..usize::from(callee.span().end()))
                        == Some(name.as_str())
                {
                    return Some(name);
                }
            }
            children.extend(args.iter().chain(comptime).map(|a| a.value()));
            children.extend(body);
        }
        Expr::Block {
            bindings, value, ..
        } => {
            let mut bound = bound.clone();
            for (name, value) in bindings {
                if let Some(name) = called_at(value, source, offset, &bound) {
                    return Some(name);
                }
                bound.insert(name.clone());
            }
            return called_at(value, source, offset, &bound);
        }
        Expr::Content { forest, .. }
        | Expr::Fragment {
            children: forest, ..
        }
        | Expr::Array(forest, _) => children.extend(forest),
        Expr::If {
            condition, yes, no, ..
        } => children.extend([condition.as_ref(), yes.as_ref(), no.as_ref()]),
        Expr::Match { subject, arms, .. } => {
            children.push(subject);
            children.extend(arms.iter().flat_map(|(pattern, value)| [pattern, value]));
        }
        Expr::Binary { left, right, .. } => children.extend([left.as_ref(), right.as_ref()]),
        Expr::Unary { value, .. } | Expr::Comptime { value, .. } => children.push(value),
        Expr::Dict(entries, _) => children.extend(entries.iter().map(|(_, value)| value)),
        _ => {}
    }
    children
        .into_iter()
        .find_map(|child| called_at(child, source, offset, bound))
}
