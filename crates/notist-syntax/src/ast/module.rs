//! Typed views of Code declarations. Values and types remain syntax here;
//! semantic lowering owns type resolution, literal conversion and validation.

use super::{NodeOrToken, Path, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

macro_rules! node {
    ($name:ident) => {
        #[derive(Debug, Clone)]
        pub struct $name(pub(crate) SyntaxNode);

        impl $name {
            pub fn cast(node: SyntaxNode) -> Option<Self> {
                (node.kind() == SyntaxKind::$name).then_some(Self(node))
            }

            pub fn range(&self) -> TextRange {
                self.0.text_range()
            }

            pub fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

node!(Module);
node!(FunctionDecl);
node!(ParameterList);
node!(Parameter);
node!(ChildrenDecl);
node!(TypeRef);
node!(DefaultValue);

fn ident(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|el| el.into_token())
        .find(|token| token.kind() == SyntaxKind::Ident)
}

impl Module {
    /// Includes recoverable declarations; inspect parse diagnostics before
    /// installing definitions into an analysis environment.
    pub fn functions(&self) -> impl Iterator<Item = FunctionDecl> + '_ {
        self.0.children().filter_map(FunctionDecl::cast)
    }
}

impl FunctionDecl {
    pub fn name(&self) -> Option<SyntaxToken> {
        ident(&self.0)
    }

    pub fn parameter_list(&self) -> Option<ParameterList> {
        self.0.children().find_map(ParameterList::cast)
    }

    pub fn parameters(&self) -> impl Iterator<Item = Parameter> + '_ {
        self.0
            .children()
            .filter_map(ParameterList::cast)
            .flat_map(|list| list.0.children().filter_map(Parameter::cast))
    }

    pub fn children_decl(&self) -> Option<ChildrenDecl> {
        self.0.children().find_map(ChildrenDecl::cast)
    }

    pub fn return_type(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }
}

impl ParameterList {
    pub fn parameters(&self) -> impl Iterator<Item = Parameter> + '_ {
        self.0.children().filter_map(Parameter::cast)
    }
}

impl Parameter {
    pub fn name(&self) -> Option<SyntaxToken> {
        ident(&self.0)
    }

    pub fn is_optional(&self) -> bool {
        self.0
            .children_with_tokens()
            .any(|el| el.kind() == SyntaxKind::Question)
    }

    pub fn ty(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }

    pub fn default_value(&self) -> Option<DefaultValue> {
        self.0.children().find_map(DefaultValue::cast)
    }
}

impl ChildrenDecl {
    pub fn ty(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }
}

impl TypeRef {
    pub fn path(&self) -> Option<Path> {
        self.0.children().find_map(Path::cast)
    }

    pub fn arguments(&self) -> impl Iterator<Item = TypeRef> + '_ {
        self.0.children().filter_map(TypeRef::cast)
    }
}

impl DefaultValue {
    /// The lossless literal element, not a decoded semantic Value.
    pub fn value(&self) -> Option<NodeOrToken<SyntaxNode, SyntaxToken>> {
        self.0.children_with_tokens().find(|el| {
            !matches!(
                el.kind(),
                SyntaxKind::Eq
                    | SyntaxKind::Whitespace
                    | SyntaxKind::Newline
                    | SyntaxKind::LineComment
                    | SyntaxKind::BlockComment
            )
        })
    }
}
