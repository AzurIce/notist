//! Private execution/recovery frames. They are not content values or public IR.
use notist_core::{
    builtins::Level,
    definitions::FunctionInstance,
    evaluation::{ExprKind, PendingEvaluation},
    item::{ContentItem, Ctor, Dict, Node, NodeContent, Origin},
    shared::Shared,
};
use rowan::TextRange;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum EvaluationState {
    Native,
    Expanded,
    Error(String),
}

#[derive(Debug, Clone)]
pub(super) struct Frame {
    pub ctor: Ctor,
    pub pending: Option<Box<PendingEvaluation>>,
    pub instance: Option<Shared<FunctionInstance>>,
    pub target: Option<Shared<FunctionInstance>>,
    pub evaluation: Option<EvaluationState>,
    pub origin: Option<Origin>,
    pub level: Level,
    pub fields: Dict,
    pub children: Vec<Frame>,
    pub attrs: Dict,
    pub span: TextRange,
}

impl Frame {
    #[cfg(test)]
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }
    #[cfg(test)]
    pub fn text(text: String, span: TextRange) -> Self {
        Self::from_node(&Node::text(text, span))
    }
    pub fn new(ctor: Ctor, span: TextRange) -> Self {
        Self::from_node(&Node::new(ctor, span))
    }

    pub fn from_node(node: &Node) -> Self {
        let mut frame = Self {
            ctor: node.ctor(),
            pending: None,
            instance: node.instance().cloned().map(Into::into),
            target: None,
            evaluation: None,
            origin: node.origin.clone(),
            level: node.level,
            fields: node.fields(),
            children: Vec::new(),
            attrs: node.attrs.clone(),
            span: node.span,
        };
        match &node.content {
            NodeContent::Item(item) => {
                frame.children = item.children.iter().map(Self::from_node).collect();
                if let Ctor::Error(message) = &item.ctor {
                    frame.evaluation = Some(EvaluationState::Error(message.clone()));
                }
            }
            NodeContent::Expr(expr) => {
                frame.evaluation = None;
                frame.pending = Some(Box::new(match &expr.kind {
                    ExprKind::Call(call) => {
                        let mut call = (**call).clone();
                        if let Some(child) = call.children.take() {
                            // Content-call children use the exact same expression
                            // representation as other call arguments.
                            if let ExprKind::Content(nodes) = &child.kind {
                                frame.children = nodes.iter().map(Self::from_node).collect();
                            } else {
                                frame
                                    .children
                                    .push(Self::from_node(&Node::expression(*child)));
                            }
                        }
                        PendingEvaluation::Call(call)
                    }
                    ExprKind::For { .. } => PendingEvaluation::Repeat((**expr).clone()),
                    ExprKind::Failure(message) => PendingEvaluation::Failure {
                        ty: expr.ty.clone(),
                        message: message.clone(),
                    },
                    _ => PendingEvaluation::Splice((**expr).clone()),
                }));
            }
        }
        frame
    }

    /// Export a bound recipe back into the public expression/content tree.
    pub fn recipe(&self) -> Node {
        let mut node = Node::new(self.ctor.clone(), self.span);
        node.attrs = self.attrs.clone();
        node.origin = self.origin.clone();
        node.level = self.level;
        if let Some(pending) = self.pending.as_deref() {
            let (kind, ty) = match pending {
                PendingEvaluation::Call(call) => {
                    let mut call = call.clone();
                    if !self.children.is_empty() || call.definition.children_type.is_some() {
                        call.children = Some(Box::new(notist_core::evaluation::TypedExpr {
                            ty: call.definition.children_type.clone(),
                            known: None,
                            kind: ExprKind::Content(
                                self.children.iter().map(Self::recipe).collect(),
                            ),
                            span: self.span,
                            source: self
                                .origin
                                .as_ref()
                                .map(|o| o.source.to_string())
                                .unwrap_or_default(),
                            shape: Default::default(),
                        }));
                    }
                    let ty = Some(call.definition.returns.clone());
                    (ExprKind::Call(Box::new(call)), ty)
                }
                PendingEvaluation::Splice(expr) | PendingEvaluation::Repeat(expr) => {
                    node.content = NodeContent::Expr(Box::new(expr.clone()));
                    return node;
                }
                PendingEvaluation::Failure { ty, message } => {
                    (ExprKind::Failure(message.clone()), ty.clone())
                }
            };
            node.content = NodeContent::Expr(Box::new(notist_core::evaluation::TypedExpr {
                ty,
                known: None,
                kind,
                span: self.span,
                source: self
                    .origin
                    .as_ref()
                    .map(|o| o.source.to_string())
                    .unwrap_or_default(),
                shape: Default::default(),
            }));
        } else if self.evaluation == Some(EvaluationState::Expanded) {
            let kind = ExprKind::Content(self.children.iter().map(Self::recipe).collect());
            let shape = notist_core::evaluation::ContentShape::of_expr(&kind);
            node.content = NodeContent::Expr(Box::new(notist_core::evaluation::TypedExpr {
                ty: Some(if self.level == Level::Block {
                    notist_core::definitions::Type::BlockContent
                } else {
                    notist_core::definitions::Type::InlineContent
                }),
                known: None,
                kind,
                span: self.span,
                source: self
                    .origin
                    .as_ref()
                    .map(|o| o.source.to_string())
                    .unwrap_or_default(),
                shape,
            }));
        } else {
            node.content = NodeContent::Item(ContentItem {
                ctor: self.ctor.clone(),
                fields: self.fields.clone(),
                children: self.children.iter().map(Self::recipe).collect(),
                instance: self.instance.clone(),
            });
        }
        node
    }

    /// Erase execution history. Only explicit groups or nonempty annotations
    /// retain a boundary around an expanded result.
    pub fn finish(self) -> Vec<Node> {
        let error = match &self.evaluation {
            Some(EvaluationState::Error(message)) => Some(message.clone()),
            _ => None,
        };
        let expanded = self.evaluation == Some(EvaluationState::Expanded);
        let mut children = Vec::new();
        for child in self.children {
            children.extend(child.finish());
        }
        if expanded && self.attrs.is_empty() && !matches!(self.ctor, Ctor::Node | Ctor::Group) {
            return children;
        }
        let (ctor, fields, instance) = if let Some(message) = error {
            children.clear();
            (Ctor::Error(message), Dict::default(), None)
        } else if expanded {
            (Ctor::Group, Dict::default(), None)
        } else {
            let instance = self.target.or(self.instance);
            let ctor = instance
                .as_ref()
                .map(|i| {
                    if i.function.module == "notist" {
                        Ctor::from_name(&i.function.name).unwrap_or_else(|| self.ctor.clone())
                    } else {
                        Ctor::Extension(notist_core::item::ExtensionCtor {
                            id: i.function.clone(),
                            accepts: self
                                .ctor
                                .accepts()
                                .unwrap_or(notist_core::builtins::Accepts::Any),
                            level: self.level,
                        })
                    }
                })
                .unwrap_or(self.ctor);
            (ctor, self.fields, instance)
        };
        vec![Node {
            content: NodeContent::Item(ContentItem {
                ctor,
                fields,
                children,
                instance,
            }),
            attrs: self.attrs,
            origin: self.origin,
            level: self.level,
            span: self.span,
        }]
    }

    pub fn function_id(&self) -> Option<notist_core::definitions::FunctionId> {
        self.ctor.function_id()
    }
    pub fn is_evaluated_interpolation(&self) -> bool {
        self.ctor == Ctor::Interpolation
            && self.pending.is_none()
            && self.evaluation == Some(EvaluationState::Expanded)
            && self.instance.is_none()
            && self.target.is_none()
    }
    pub fn descendants(&self) -> impl Iterator<Item = &Self> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }
}
