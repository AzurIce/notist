//! Built-in content constructors and methods exposed in the standard prelude.

use crate::ast::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinFn {
    // Inline constructors
    Text,
    Strong,
    Emph,
    Raw,
    Link,
    // Block constructors
    Para,
    Heading,
    Callout,
    CodeBlock,
    // Inline methods
    InlinePlainText,
    InlineWithAttr,
    InlineAttrOr,
    InlineHasAttr,
    InlineRewrite,
    // Block methods
    BlockPlainText,
    BlockSectionize,
    BlockHeadingCount,
    BlockHeadingsSummary,
    BlockWithAttr,
    BlockAttrOr,
    BlockHasAttr,
    BlockRewriteInlines,
    BlockRewriteBlocks,
}

pub(crate) fn lookup_prelude_function(name: &str) -> Option<(BuiltinFn, Vec<Type>, Type)> {
    use Type::*;
    Some(match name {
        "text" => (BuiltinFn::Text, vec![String], Inline),
        "strong" => (BuiltinFn::Strong, vec![Inline], Inline),
        "emph" => (BuiltinFn::Emph, vec![Inline], Inline),
        "raw" => (BuiltinFn::Raw, vec![String], Inline),
        "link" => (BuiltinFn::Link, vec![String, Inline], Inline),
        "para" => (BuiltinFn::Para, vec![Inline], Block),
        "heading" => (BuiltinFn::Heading, vec![Int, Inline], Block),
        "callout" => (BuiltinFn::Callout, vec![String, Block], Block),
        "code_block" => (BuiltinFn::CodeBlock, vec![String, String], Block),
        _ => return None,
    })
}

pub(crate) fn lookup_builtin_method(
    receiver: &Type,
    name: &str,
) -> Option<(BuiltinFn, Vec<Type>, Type)> {
    use Type::*;
    match (receiver, name) {
        (Inline, "plain_text") => Some((BuiltinFn::InlinePlainText, vec![], String)),
        (Inline, "with_attr") => Some((BuiltinFn::InlineWithAttr, vec![String, String], Inline)),
        (Inline, "attr_or") => Some((BuiltinFn::InlineAttrOr, vec![String, String], String)),
        (Inline, "has_attr") => Some((BuiltinFn::InlineHasAttr, vec![String], Bool)),
        (Inline, "rewrite") => Some((
            BuiltinFn::InlineRewrite,
            vec![Function(vec![Inline], Box::new(Inline))],
            Inline,
        )),
        (Block, "plain_text") => Some((BuiltinFn::BlockPlainText, vec![], String)),
        (Block, "sectionize") => Some((BuiltinFn::BlockSectionize, vec![], Block)),
        (Block, "heading_count") => Some((BuiltinFn::BlockHeadingCount, vec![], Int)),
        (Block, "headings_summary") => Some((BuiltinFn::BlockHeadingsSummary, vec![], String)),
        (Block, "with_attr") => Some((BuiltinFn::BlockWithAttr, vec![String, String], Block)),
        (Block, "attr_or") => Some((BuiltinFn::BlockAttrOr, vec![String, String], String)),
        (Block, "has_attr") => Some((BuiltinFn::BlockHasAttr, vec![String], Bool)),
        (Block, "rewrite_inlines") => Some((
            BuiltinFn::BlockRewriteInlines,
            vec![Function(vec![Inline], Box::new(Inline))],
            Block,
        )),
        (Block, "rewrite_blocks") => Some((
            BuiltinFn::BlockRewriteBlocks,
            vec![Function(vec![Block], Box::new(Block))],
            Block,
        )),
        _ => None,
    }
}
