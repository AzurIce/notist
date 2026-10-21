//! Shared Markup backend in the current lexical session.
//! IR₁ enters Resolve → Shape → Materialize;
//! the resulting Item forest returns to the Code checker or document entry.
use super::check::{Checker, content_type};
use super::*;
use notist_core::{
    annotation::Annotation,
    builtins::{Accepts, Level},
    definitions::ParameterMode,
    expr::{BodyFlavor, Expr},
    item::{Ctor, ExtensionCtor, Value},
};
impl Checker<'_> {
    pub(super) fn compile_call(&mut self, call: &Expr) -> Item {
        let start = self.content_annotations.len();
        let r = self.prepare_call(call, false);
        let mut shaped = crate::shape::shape_inline(vec![r]);
        let annotations = self.content_annotations.split_off(start);
        crate::materialize::materialize(
            &mut shaped,
            &annotations,
            &self.source,
            &mut self.diagnostics,
        )
        .remove(0)
    }
    pub(crate) fn content(
        &mut self,
        forest: &[Expr],
        annotations: &[Annotation],
        body: BodyFlavor,
    ) -> (Vec<Item>, Vec<RExpr>) {
        let start = self.content_annotations.len();
        let r: Vec<_> = forest.iter().map(|e| self.prepare(e)).collect();
        let mut shaped = if body == BodyFlavor::Block {
            crate::shape::shape(r)
        } else {
            crate::shape::shape_inline(r)
        };
        if body == BodyFlavor::Inline {
            for r in &shaped {
                if crate::shape::is_block_element(r) {
                    self.report(r.span, "InlineContent literal cannot contain BlockContent");
                }
            }
        }
        let mut all_annotations = annotations.to_vec();
        all_annotations.extend(self.content_annotations.split_off(start));
        let items = crate::materialize::materialize(
            &mut shaped,
            &all_annotations,
            &self.source,
            &mut self.diagnostics,
        );
        (items, shaped)
    }
    fn prepare(&mut self, e: &Expr) -> RExpr {
        self.failures.push(Vec::new());
        let result = self.prepare_interpolation(e);
        self.failures.pop();
        result
    }
    fn prepare_interpolation(&mut self, e: &Expr) -> RExpr {
        let span = e.span();
        if let Expr::Fragment { children, .. } = e {
            let mut prepared: Vec<_> = children.iter().map(|e| self.prepare(e)).collect();
            if prepared.len() == 1 && crate::shape::is_block_element(&prepared[0]) {
                return prepared.pop().unwrap();
            }
            let invalid = prepared.iter().any(crate::shape::is_block_element);
            let mut paragraph = RExpr::new(Ctor::Paragraph, span).with_children(prepared);
            if invalid {
                let message = "a content fragment must be all inline or a single block expression; separate blocks with a blank line";
                self.report(span, message);
                paragraph.pending = Some(Box::new(PendingEvaluation::Failure {
                    ty: Some(Type::BlockContent),
                    message: message.into(),
                }));
            }
            return paragraph;
        }
        let Expr::Call {
            callee,
            children,
            body,
            attrs,
            ..
        } = e
        else {
            let value = self.check(e);
            let level = value.ty.as_ref().and_then(Type::level);
            if level.is_none() && value.ty.is_some() {
                self.report(
                    span,
                    "content interpolation requires InlineContent or BlockContent",
                );
            }
            let mut r = RExpr::new(Ctor::Interpolation, span);
            r.pending = Some(Box::new(PendingEvaluation::Splice(value)));
            r.level = level;
            return r;
        };
        let Some(_) = callee.name_path() else {
            self.report(callee.span(), "content calls require a named function");
            let mut r = RExpr::new(Ctor::Custom("<call>".into()), span)
                .with_children(children.iter().map(|e| self.prepare(e)).collect());
            r.body = *body;
            r.attrs = attrs.clone();
            return r;
        };
        self.prepare_call(e, true)
    }
    fn prepare_call(&mut self, e: &Expr, interpolation: bool) -> RExpr {
        let Expr::Call {
            callee,
            args,
            comptime,
            children,
            body,
            attrs,
            span,
        } = e
        else {
            unreachable!()
        };
        let span = *span;
        let name = callee.name_path().expect("named function target");
        // Signature inference uses the written mount category, never payload values.
        let definition = self.definition(&name);
        let structural = definition
            .as_ref()
            .is_some_and(|d| matches!(d.children, Accepts::Items | Accepts::Rows | Accepts::Cells));
        // Rows, cells and list items form structural axes, not ordinary
        // paragraph/block content. Their separators do not create paragraphs.
        let children = if let [
            Expr::Content {
                forest,
                annotations,
                ..
            },
        ] = children.as_slice()
        {
            self.content_annotations.extend(annotations.iter().cloned());
            forest
        } else {
            children
        };
        let prepared: Vec<_> = children
            .iter()
            .flat_map(|e| match e {
                Expr::Fragment { children, .. } if structural => children.as_slice(),
                e => std::slice::from_ref(e),
            })
            .map(|e| self.prepare(e))
            .collect();
        let flavor = if structural {
            BodyFlavor::Block
        } else if *body == BodyFlavor::None {
            if definition
                .as_ref()
                .is_some_and(|d| d.children == Accepts::Inline)
            {
                BodyFlavor::Inline
            } else {
                BodyFlavor::Block
            }
        } else {
            *body
        };
        let child = if *body != BodyFlavor::None || !children.is_empty() {
            Some(Box::new(self.typed(
                Some(content_type(flavor)),
                ExprKind::Content(Vec::new()),
                None,
                span,
            )))
        } else {
            None
        };
        let call = self.invocation(&name, args, comptime, child, attrs.clone(), span);
        let mut call = match call {
            Ok(call) => call,
            Err(failure) => {
                let ty = failure
                    .definition
                    .as_ref()
                    .map(|d| d.returns.clone())
                    .filter(Type::runtime);
                let ctor = failure
                    .definition
                    .as_ref()
                    .map(resolved_ctor)
                    .unwrap_or_else(|| Ctor::Custom(name.clone()));
                let mut r = RExpr::new(ctor, span).with_children(prepared);
                r.fields = failure.fields;
                r.instance = failure.instance;
                r.level = ty.as_ref().and_then(Type::level);
                r.body = *body;
                r.attrs = attrs.clone();
                r.pending = Some(Box::new(PendingEvaluation::Failure {
                    ty,
                    message: failure.message,
                }));
                return r;
            }
        };
        if interpolation && call.definition.returns.level().is_none() {
            let message = "content interpolation requires a content result";
            self.report(span, message);
            call.invalid = Some(message.into());
        }
        call.children = None;
        if call.definition.children_type == Some(Type::InlineContent)
            && prepared.iter().any(contains_block)
        {
            let message = format!("`{name}` takes inline content; block children are not allowed");
            self.report(span, &message);
            call.invalid = Some(message);
        }

        let mut fields = known_fields(
            &call.definition,
            call.arguments.iter().map(|(k, v)| (k.as_str(), v)),
        );
        if call.definition.id == notist_core::definitions::FunctionId::new("notist", "heading")
            && let Some(ComptimeValue::Data(v)) = call.instance.arguments.first()
        {
            fields.insert("level", v.clone());
        }
        let level = call.definition.returns.level().unwrap_or(Level::Inline);
        let ctor = resolved_ctor(&call.definition);
        let mut r = RExpr::new(ctor, span).with_children(prepared);
        r.fields = fields;
        r.body = *body;
        r.attrs = attrs.clone();
        r.level = Some(level);
        r.instance = Some(call.instance.clone());
        if matches!(
            call.definition.children,
            Accepts::Rows | Accepts::Cells | Accepts::Items
        ) {
            let mut errors = Vec::new();
            crate::resolve::validate_structure(&mut r, &call.definition.id.name, &mut errors);
            if let Some(error) = errors.first() {
                call.invalid = Some(error.message.clone());
            }
            for error in errors {
                self.report(error.span, error.message);
            }
        }

        r.pending = Some(Box::new(PendingEvaluation::Call(call)));
        r
    }
}
fn contains_block(r: &RExpr) -> bool {
    crate::shape::is_block_element(r)
        || r.ctor == Ctor::Node && r.children.iter().any(contains_block)
}

fn resolved_ctor(definition: &FunctionDef) -> Ctor {
    if definition.id.module == "notist" {
        return Ctor::from_name(&definition.id.name).unwrap();
    }
    let accepts = match definition.children {
        Accepts::Any => match definition.children_type.as_ref().and_then(Type::level) {
            Some(Level::Inline) => Accepts::Inline,
            _ => Accepts::Block,
        },
        a => a,
    };
    Ctor::Extension(ExtensionCtor {
        id: definition.id.clone(),
        accepts,
        level: definition.returns.level().unwrap_or(Level::Inline),
    })
}

pub(super) fn known_fields<'a>(
    definition: &FunctionDef,
    arguments: impl Iterator<Item = (&'a str, &'a TypedExpr)>,
) -> Dict {
    arguments
        .filter_map(|(name, value)| {
            let mut known = value.known.clone();
            // Forwarded options keep the caller view: some(v) stores v, none omits.
            if definition
                .parameters
                .iter()
                .any(|p| p.name == name && p.mode == ParameterMode::Optional)
                && matches!(&value.ty, Some(Type::Option(_)))
            {
                known = match known {
                    Some(ComptimeValue::Data(Value::Some(v))) => Some(ComptimeValue::Data(*v)),
                    Some(ComptimeValue::Data(Value::None)) => None,
                    other => other,
                };
            }
            match &known {
                Some(ComptimeValue::Data(v)) => Some((name.to_owned(), v.clone())),
                _ => None,
            }
        })
        .collect()
}
