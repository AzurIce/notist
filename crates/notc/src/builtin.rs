//! Built-in content constructors, data helpers, and methods exposed in the standard prelude.

use crate::ast::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinFn {
    // Inline constructors
    Text,
    Strong,
    Emph,
    Raw,
    Math,
    Link,
    Image,
    Inlines,
    // Block constructors
    Para,
    Heading,
    Callout,
    CodeBlock,
    DisplayMath,
    Quote,
    Divider,
    Table,
    Div,
    Section,
    Blocks,
    // Data host
    ReadJson,
    // Scalar methods
    IntToString,
    FloatToString,
    BoolToString,
    // List methods
    ListLen,
    // Json methods
    JsonGet,
    JsonAt,
    JsonAsString,
    JsonAsInt,
    JsonAsBool,
    JsonAsList,
    JsonLen,
    JsonIsNull,
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
        "math" => (BuiltinFn::Math, vec![String], Inline),
        "link" => (BuiltinFn::Link, vec![String, Inline], Inline),
        "image" => (BuiltinFn::Image, vec![String, String], Inline),
        "inlines" => (BuiltinFn::Inlines, vec![List(Box::new(Inline))], Inline),
        "para" => (BuiltinFn::Para, vec![Inline], Block),
        "heading" => (BuiltinFn::Heading, vec![Int, Inline], Block),
        "callout" => (BuiltinFn::Callout, vec![String, Block], Block),
        "code_block" => (BuiltinFn::CodeBlock, vec![String, String], Block),
        "display_math" => (BuiltinFn::DisplayMath, vec![String], Block),
        "quote" => (BuiltinFn::Quote, vec![Block], Block),
        "divider" => (BuiltinFn::Divider, vec![], Block),
        "table" => (
            BuiltinFn::Table,
            vec![
                List(Box::new(Inline)),
                List(Box::new(List(Box::new(Inline)))),
            ],
            Block,
        ),
        "div" => (BuiltinFn::Div, vec![Block], Block),
        "section" => (BuiltinFn::Section, vec![Block], Block),
        "blocks" => (BuiltinFn::Blocks, vec![List(Box::new(Block))], Block),
        "read_json" => (BuiltinFn::ReadJson, vec![String], Json),
        _ => return None,
    })
}

pub(crate) fn all_prelude_functions() -> Vec<(&'static str, BuiltinFn, Vec<Type>, Type)> {
    const NAMES: &[&str] = &[
        "text",
        "strong",
        "emph",
        "raw",
        "math",
        "link",
        "image",
        "inlines",
        "para",
        "heading",
        "callout",
        "code_block",
        "display_math",
        "quote",
        "divider",
        "table",
        "div",
        "section",
        "blocks",
        "read_json",
    ];
    NAMES
        .iter()
        .filter_map(|&name| {
            let (b, params, ret) = lookup_prelude_function(name)?;
            Some((name, b, params, ret))
        })
        .collect()
}

pub(crate) fn lookup_builtin_method(
    receiver: &Type,
    name: &str,
) -> Option<(BuiltinFn, Vec<Type>, Type)> {
    use Type::*;
    match (receiver, name) {
        (Int, "to_string") => Some((BuiltinFn::IntToString, vec![], String)),
        (Float, "to_string") => Some((BuiltinFn::FloatToString, vec![], String)),
        (Bool, "to_string") => Some((BuiltinFn::BoolToString, vec![], String)),
        (List(_), "len") => Some((BuiltinFn::ListLen, vec![], Int)),
        (Json, "get") => Some((BuiltinFn::JsonGet, vec![String], Json)),
        (Json, "at") => Some((BuiltinFn::JsonAt, vec![Int], Json)),
        (Json, "as_string") => Some((BuiltinFn::JsonAsString, vec![String], String)),
        (Json, "as_int") => Some((BuiltinFn::JsonAsInt, vec![Int], Int)),
        (Json, "as_bool") => Some((BuiltinFn::JsonAsBool, vec![Bool], Bool)),
        (Json, "as_list") => Some((BuiltinFn::JsonAsList, vec![], List(Box::new(Json)))),
        (Json, "len") => Some((BuiltinFn::JsonLen, vec![], Int)),
        (Json, "is_null") => Some((BuiltinFn::JsonIsNull, vec![], Bool)),
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

pub(crate) fn builtin_methods_for_type(
    receiver: &Type,
) -> Vec<(&'static str, BuiltinFn, Vec<Type>, Type)> {
    let names: &[&str] = match receiver {
        Type::Int | Type::Float | Type::Bool => &["to_string"],
        Type::List(_) => &["len"],
        Type::Json => &[
            "get",
            "at",
            "as_string",
            "as_int",
            "as_bool",
            "as_list",
            "len",
            "is_null",
        ],
        Type::Inline => &["plain_text", "with_attr", "attr_or", "has_attr", "rewrite"],
        Type::Block => &[
            "plain_text",
            "sectionize",
            "heading_count",
            "headings_summary",
            "with_attr",
            "attr_or",
            "has_attr",
            "rewrite_inlines",
            "rewrite_blocks",
        ],
        _ => &[],
    };
    names
        .iter()
        .filter_map(|&name| {
            let (b, params, ret) = lookup_builtin_method(receiver, name)?;
            Some((name, b, params, ret))
        })
        .collect()
}
