use super::*;
use std::ops::Deref;

/// Source identity and presentation metadata, never an incomplete output node.
#[derive(Debug, Clone)]
pub(super) struct NodeMetadata {
    pub(super) attrs: Dict,
    pub(super) origin: Option<Origin>,
    pub(super) level: Level,
    pub(super) span: rowan::TextRange,
    pub(super) ctor: Ctor,
}
impl From<&Node> for NodeMetadata {
    fn from(node: &Node) -> Self {
        Self {
            attrs: node.attrs.clone(),
            origin: node.origin.clone(),
            level: node.level,
            span: node.span,
            ctor: node.ctor(),
        }
    }
}
impl NodeMetadata {
    pub(super) fn node(&self, content: ContentItem) -> Node {
        Node {
            content: NodeContent::Item(content),
            attrs: self.attrs.clone(),
            origin: self.origin.clone(),
            level: self.level,
            span: self.span,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Content {
    pub(super) pieces: Pieces,
}
#[derive(Debug, Clone)]
pub(super) struct Counted<T> {
    value: T,
    nodes: usize,
}
#[derive(Debug, Clone, Default)]
pub(super) enum Pieces {
    #[default]
    Empty,
    One(Shared<Counted<Piece>>),
    Many(Shared<Counted<Vec<Piece>>>),
}
impl Deref for Pieces {
    type Target = [Piece];
    fn deref(&self) -> &[Piece] {
        match self {
            Self::Empty => &[],
            Self::One(piece) => std::slice::from_ref(&piece.value),
            Self::Many(pieces) => &pieces.value,
        }
    }
}
impl Content {
    pub(super) fn from_pieces(mut pieces: smallvec::SmallVec<[Piece; 1]>, nodes: usize) -> Self {
        Self {
            pieces: match pieces.len() {
                0 => Pieces::Empty,
                1 => Pieces::One(
                    Counted {
                        value: pieces.pop().unwrap(),
                        nodes,
                    }
                    .into(),
                ),
                _ => Pieces::Many(
                    Counted {
                        value: pieces.into_vec(),
                        nodes,
                    }
                    .into(),
                ),
            },
        }
    }
    pub(super) fn nodes(&self) -> usize {
        match &self.pieces {
            Pieces::Empty => 0,
            Pieces::One(piece) => piece.nodes,
            Pieces::Many(pieces) => pieces.nodes,
        }
    }
    pub(super) fn one(piece: Piece, nodes: usize) -> Self {
        Self {
            pieces: Pieces::One(
                Counted {
                    value: piece,
                    nodes,
                }
                .into(),
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Piece {
    pub(super) source: NodeMetadata,
    pub(super) kind: Construction,
}
#[derive(Debug, Clone)]
pub(super) struct ElementSpec {
    pub(super) ctor: Ctor,
    pub(super) fields: Dict,
    pub(super) instance: Option<Shared<FunctionInstance>>,
}
#[derive(Debug, Clone)]
pub(super) enum Construction {
    Ready {
        item: ContentItem,
        nodes: usize,
    },
    Element {
        spec: ElementSpec,
        children: Content,
    },
    Call {
        call: Shared<Invocation>,
        environment: Scope,
        children: Content,
    },
    Repeat {
        expression: Shared<TypedExpr>,
        environment: Scope,
    },
    Splice(Content),
    Error,
}

pub(super) struct ElementDraft<'a> {
    pub(super) source: &'a NodeMetadata,
    pub(super) spec: ElementSpec,
    pub(super) children: Content,
}
pub(super) enum CallOutcome<'a> {
    Value(ComptimeValue),
    Content(Type, Content),
    Element(ElementDraft<'a>),
}
impl<'a> From<Runtime> for CallOutcome<'a> {
    fn from(value: Runtime) -> Self {
        match value {
            Runtime::Value(v) => Self::Value(v),
            Runtime::Content(t, c) => Self::Content(t, c),
        }
    }
}
impl CallOutcome<'_> {
    pub(super) fn into_runtime(self, returns: &Type) -> Runtime {
        match self {
            Self::Value(v) => Runtime::Value(v),
            Self::Content(t, c) => Runtime::Content(t, c),
            Self::Element(draft) => {
                let nodes = 1 + draft.children.nodes();
                Runtime::Content(
                    returns.clone(),
                    Content::one(
                        Piece {
                            source: draft.source.clone(),
                            kind: Construction::Element {
                                spec: draft.spec,
                                children: draft.children,
                            },
                        },
                        nodes,
                    ),
                )
            }
        }
    }
}
