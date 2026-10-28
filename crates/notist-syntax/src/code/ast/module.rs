//! Typed views of Code declarations. Values and types remain syntax here;
//! semantic lowering owns type resolution, literal conversion and validation.

use super::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};

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
node!(ImportDecl);
node!(ImportMember);
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

impl Module {
    pub fn imports(&self) -> impl Iterator<Item = ImportDecl> + '_ {
        self.0.children().filter_map(ImportDecl::cast)
    }
}
impl ImportDecl {
    pub fn source(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::Str)
    }
    pub fn alias(&self) -> Option<SyntaxToken> {
        ident(&self.0)
    }
    pub fn has_members(&self) -> bool {
        self.0
            .children_with_tokens()
            .any(|e| e.kind() == SyntaxKind::Colon)
    }
    pub fn is_wildcard(&self) -> bool {
        self.0
            .children_with_tokens()
            .any(|e| e.kind() == SyntaxKind::Star)
    }
    pub fn members(&self) -> impl Iterator<Item = ImportMember> + '_ {
        self.0.children().filter_map(ImportMember::cast)
    }
}

impl ImportMember {
    pub fn name(&self) -> Option<SyntaxToken> {
        ident(&self.0)
    }
    pub fn alias(&self) -> Option<SyntaxToken> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::Ident)
            .nth(1)
    }
}

impl FunctionDecl {
    pub fn comptime_parameters(&self) -> impl Iterator<Item = Parameter> + '_ {
        self.0
            .children()
            .filter(|n| n.kind() == SyntaxKind::ComptimeParameters)
            .flat_map(|n| n.children().filter_map(Parameter::cast))
    }
    pub fn body(&self) -> Option<SyntaxNode> {
        self.0
            .children()
            .find(|n| n.kind() == SyntaxKind::BlockExpr)
    }

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
    pub fn name(&self) -> Option<SyntaxNode> {
        self.0
            .children()
            .find(|n| matches!(n.kind(), SyntaxKind::Path | SyntaxKind::MemberExpr))
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
