//! Built-in `notist` module declarations, prelude functions, and receiver methods.

use crate::ast::{Span, Type, WhereExpr, WhereExprKind};
use crate::check::{Binding, CheckedDecl, DeclId, FunctionSig, InstanceKey, ModuleExpr, ModuleId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinFn {
    // Inline constructors
    Text,
    Strong,
    Emph,
    Strike,
    Underline,
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
    List,
    Table,
    Div,
    Section,
    Blocks,
    // Data host & control
    ReadJson,
    ReadText,
    Panic,
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

impl DeclId {
    pub(crate) const NOTIST_TEXT: Self = Self { module: ModuleId::NOTIST, index: 0 };
    pub(crate) const NOTIST_STRONG: Self = Self { module: ModuleId::NOTIST, index: 1 };
    pub(crate) const NOTIST_EMPH: Self = Self { module: ModuleId::NOTIST, index: 2 };
    pub(crate) const NOTIST_STRIKE: Self = Self { module: ModuleId::NOTIST, index: 3 };
    pub(crate) const NOTIST_UNDERLINE: Self = Self { module: ModuleId::NOTIST, index: 4 };
    pub(crate) const NOTIST_RAW: Self = Self { module: ModuleId::NOTIST, index: 5 };
    pub(crate) const NOTIST_MATH: Self = Self { module: ModuleId::NOTIST, index: 6 };
    pub(crate) const NOTIST_LINK: Self = Self { module: ModuleId::NOTIST, index: 7 };
    pub(crate) const NOTIST_IMAGE: Self = Self { module: ModuleId::NOTIST, index: 8 };
    pub(crate) const NOTIST_INLINES: Self = Self { module: ModuleId::NOTIST, index: 9 };
    pub(crate) const NOTIST_PARA: Self = Self { module: ModuleId::NOTIST, index: 10 };
    pub(crate) const NOTIST_HEADING: Self = Self { module: ModuleId::NOTIST, index: 11 };
    pub(crate) const NOTIST_CALLOUT: Self = Self { module: ModuleId::NOTIST, index: 12 };
    pub(crate) const NOTIST_CODE_BLOCK: Self = Self { module: ModuleId::NOTIST, index: 13 };
    pub(crate) const NOTIST_DISPLAY_MATH: Self = Self { module: ModuleId::NOTIST, index: 14 };
    pub(crate) const NOTIST_QUOTE: Self = Self { module: ModuleId::NOTIST, index: 15 };
    pub(crate) const NOTIST_DIVIDER: Self = Self { module: ModuleId::NOTIST, index: 16 };
    pub(crate) const NOTIST_LIST: Self = Self { module: ModuleId::NOTIST, index: 17 };
    pub(crate) const NOTIST_TABLE: Self = Self { module: ModuleId::NOTIST, index: 18 };
    pub(crate) const NOTIST_DIV: Self = Self { module: ModuleId::NOTIST, index: 19 };
    pub(crate) const NOTIST_SECTION: Self = Self { module: ModuleId::NOTIST, index: 20 };
    pub(crate) const NOTIST_BLOCKS: Self = Self { module: ModuleId::NOTIST, index: 21 };
    pub(crate) const NOTIST_READ_JSON: Self = Self { module: ModuleId::NOTIST, index: 22 };
    pub(crate) const NOTIST_READ_TEXT: Self = Self { module: ModuleId::NOTIST, index: 23 };
    pub(crate) const NOTIST_PANIC: Self = Self { module: ModuleId::NOTIST, index: 24 };
}

pub(crate) struct NotistBuiltins {
    pub(crate) names: Vec<&'static str>,
    pub(crate) display_signatures: Vec<String>,
    pub(crate) signatures: Vec<FunctionSig>,
    pub(crate) exports: HashMap<String, Binding>,
    pub(crate) decls: Vec<CheckedDecl>,
}

pub(crate) fn notist_builtins() -> &'static NotistBuiltins {
    static BUILTINS: OnceLock<NotistBuiltins> = OnceLock::new();
    BUILTINS.get_or_init(build_notist_builtins)
}

/// Fast-path instruction selection when a `ModuleId::NOTIST` declaration has NOT
/// been replaced. Note that `display_math` delegates to `notist.math<Block>`, so
/// callers must only use its direct builtin when `NOTIST_MATH` is also unreplaced.
pub(crate) fn notist_direct_builtin(index: usize, type_args: &[Type]) -> Option<BuiltinFn> {
    Some(match index {
        0 => BuiltinFn::Text,
        1 => BuiltinFn::Strong,
        2 => BuiltinFn::Emph,
        3 => BuiltinFn::Strike,
        4 => BuiltinFn::Underline,
        5 => BuiltinFn::Raw,
        6 => match type_args.first()? {
            Type::Inline => BuiltinFn::Math,
            Type::Block => BuiltinFn::DisplayMath,
            _ => return None,
        },
        7 => BuiltinFn::Link,
        8 => BuiltinFn::Image,
        9 => BuiltinFn::Inlines,
        10 => BuiltinFn::Para,
        11 => BuiltinFn::Heading,
        12 => BuiltinFn::Callout,
        13 => BuiltinFn::CodeBlock,
        14 => BuiltinFn::DisplayMath,
        15 => BuiltinFn::Quote,
        16 => BuiltinFn::Divider,
        17 => BuiltinFn::List,
        18 => BuiltinFn::Table,
        19 => BuiltinFn::Div,
        20 => BuiltinFn::Section,
        21 => BuiltinFn::Blocks,
        22 => BuiltinFn::ReadJson,
        23 => BuiltinFn::ReadText,
        24 => BuiltinFn::Panic,
        _ => return None,
    })
}

fn build_notist_builtins() -> NotistBuiltins {
    use Type::*;

    struct Spec {
        id: DeclId,
        name: &'static str,
        display_sig: &'static str,
        type_params: Vec<std::string::String>,
        type_param_defaults: Vec<Option<Type>>,
        params: Vec<Type>,
        returns: Type,
        where_clause: Option<WhereExpr>,
        body: ModuleExpr,
    }

    let mono_body = |builtin: BuiltinFn, arity: usize| -> ModuleExpr {
        let args = (0..arity).map(ModuleExpr::Local).collect();
        ModuleExpr::Call(
            Box::new(ModuleExpr::Builtin(builtin, Span::DUMMY)),
            args,
            Span::DUMMY,
        )
    };

    let math_where = WhereExpr::new(
        WhereExprKind::Or(
            Box::new(WhereExpr::new(
                WhereExprKind::TypeEq(Param("C".into()), Inline),
                Span::DUMMY,
            )),
            Box::new(WhereExpr::new(
                WhereExprKind::TypeEq(Param("C".into()), Block),
                Span::DUMMY,
            )),
        ),
        Span::DUMMY,
    );

    let math_body = ModuleExpr::TypeMatch {
        param_index: 0,
        arms: vec![
            (Inline, mono_body(BuiltinFn::Math, 1)),
            (Block, mono_body(BuiltinFn::DisplayMath, 1)),
        ],
        wildcard: Box::new(ModuleExpr::CompileError {
            message: "notist.math requires Inline or Block".into(),
            span: Span::DUMMY,
        }),
    };

    let display_math_body = ModuleExpr::Call(
        Box::new(ModuleExpr::Function(
            InstanceKey {
                decl: DeclId::NOTIST_MATH,
                type_args: vec![Block],
            },
            Span::DUMMY,
        )),
        vec![ModuleExpr::Local(0)],
        Span::DUMMY,
    );

    let specs = vec![
        Spec {
            id: DeclId::NOTIST_TEXT,
            name: "text",
            display_sig: "fn text(text: String) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Text, 1),
        },
        Spec {
            id: DeclId::NOTIST_STRONG,
            name: "strong",
            display_sig: "fn strong(body: Inline) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Inline],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Strong, 1),
        },
        Spec {
            id: DeclId::NOTIST_EMPH,
            name: "emph",
            display_sig: "fn emph(body: Inline) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Inline],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Emph, 1),
        },
        Spec {
            id: DeclId::NOTIST_STRIKE,
            name: "strike",
            display_sig: "fn strike(body: Inline) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Inline],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Strike, 1),
        },
        Spec {
            id: DeclId::NOTIST_UNDERLINE,
            name: "underline",
            display_sig: "fn underline(body: Inline) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Inline],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Underline, 1),
        },
        Spec {
            id: DeclId::NOTIST_RAW,
            name: "raw",
            display_sig: "fn raw(text: String) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Raw, 1),
        },
        Spec {
            id: DeclId::NOTIST_MATH,
            name: "math",
            display_sig: "fn math<C: Type = Inline>(text: String) -> C\nwhere C == Inline || C == Block",
            type_params: vec!["C".into()],
            type_param_defaults: vec![Some(Inline)],
            params: vec![String],
            returns: Param("C".into()),
            where_clause: Some(math_where),
            body: math_body,
        },
        Spec {
            id: DeclId::NOTIST_LINK,
            name: "link",
            display_sig: "fn link(target: String, label: Inline) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String, Inline],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Link, 2),
        },
        Spec {
            id: DeclId::NOTIST_IMAGE,
            name: "image",
            display_sig: "fn image(target: String, alt: String) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String, String],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Image, 2),
        },
        Spec {
            id: DeclId::NOTIST_INLINES,
            name: "inlines",
            display_sig: "fn inlines(items: List<Inline>) -> Inline",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![List(Box::new(Inline))],
            returns: Inline,
            where_clause: None,
            body: mono_body(BuiltinFn::Inlines, 1),
        },
        Spec {
            id: DeclId::NOTIST_PARA,
            name: "para",
            display_sig: "fn para(body: Inline) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Inline],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Para, 1),
        },
        Spec {
            id: DeclId::NOTIST_HEADING,
            name: "heading",
            display_sig: "fn heading(level: Int, title: Inline) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Int, Inline],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Heading, 2),
        },
        Spec {
            id: DeclId::NOTIST_CALLOUT,
            name: "callout",
            display_sig: "fn callout(kind: String, body: Block) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String, Block],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Callout, 2),
        },
        Spec {
            id: DeclId::NOTIST_CODE_BLOCK,
            name: "code_block",
            display_sig: "fn code_block(lang: String, text: String) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String, String],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::CodeBlock, 2),
        },
        Spec {
            id: DeclId::NOTIST_DISPLAY_MATH,
            name: "display_math",
            display_sig: "fn display_math(text: String) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: Block,
            where_clause: None,
            body: display_math_body,
        },
        Spec {
            id: DeclId::NOTIST_QUOTE,
            name: "quote",
            display_sig: "fn quote(body: Block) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Block],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Quote, 1),
        },
        Spec {
            id: DeclId::NOTIST_DIVIDER,
            name: "divider",
            display_sig: "fn divider() -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Divider, 0),
        },
        Spec {
            id: DeclId::NOTIST_LIST,
            name: "list",
            display_sig: "fn list(ordered: Bool, start: Int, items: List<Block>) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Bool, Int, List(Box::new(Block))],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::List, 3),
        },
        Spec {
            id: DeclId::NOTIST_TABLE,
            name: "table",
            display_sig: "fn table(headers: List<Inline>, rows: List<List<Inline>>) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![
                List(Box::new(Inline)),
                List(Box::new(List(Box::new(Inline)))),
            ],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Table, 2),
        },
        Spec {
            id: DeclId::NOTIST_DIV,
            name: "div",
            display_sig: "fn div(body: Block) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Block],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Div, 1),
        },
        Spec {
            id: DeclId::NOTIST_SECTION,
            name: "section",
            display_sig: "fn section(body: Block) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![Block],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Section, 1),
        },
        Spec {
            id: DeclId::NOTIST_BLOCKS,
            name: "blocks",
            display_sig: "fn blocks(items: List<Block>) -> Block",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![List(Box::new(Block))],
            returns: Block,
            where_clause: None,
            body: mono_body(BuiltinFn::Blocks, 1),
        },
        Spec {
            id: DeclId::NOTIST_READ_JSON,
            name: "read_json",
            display_sig: "fn read_json(path: String) -> Json",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: Json,
            where_clause: None,
            body: mono_body(BuiltinFn::ReadJson, 1),
        },
        Spec {
            id: DeclId::NOTIST_READ_TEXT,
            name: "read_text",
            display_sig: "fn read_text(path: String) -> String",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: String,
            where_clause: None,
            body: mono_body(BuiltinFn::ReadText, 1),
        },
        Spec {
            id: DeclId::NOTIST_PANIC,
            name: "panic",
            display_sig: "fn panic(message: String) -> Never",
            type_params: vec![],
            type_param_defaults: vec![],
            params: vec![String],
            returns: Never,
            where_clause: None,
            body: mono_body(BuiltinFn::Panic, 1),
        },
    ];

    let mut names = Vec::with_capacity(specs.len());
    let mut display_signatures = Vec::with_capacity(specs.len());
    let mut signatures = Vec::with_capacity(specs.len());
    let mut exports = HashMap::with_capacity(specs.len());
    let mut decls = Vec::with_capacity(specs.len());

    for (expected_idx, spec) in specs.into_iter().enumerate() {
        debug_assert_eq!(spec.id.index, expected_idx);
        let _ = Arc::<str>::from(spec.name);
        names.push(spec.name);
        display_signatures.push(spec.display_sig.to_owned());
        exports.insert(spec.name.to_owned(), Binding::Function(spec.id));
        decls.push(CheckedDecl {
            name: spec.name.to_owned(),
            type_params: spec.type_params.clone(),
            where_clause: spec.where_clause.clone(),
            arity: spec.params.len(),
            body: spec.body,
        });
        signatures.push(FunctionSig {
            name: spec.name.to_owned(),
            type_params: spec.type_params,
            type_param_defaults: spec.type_param_defaults,
            parameters: spec.params,
            returns: spec.returns,
            where_clause: spec.where_clause,
        });
    }

    NotistBuiltins {
        names,
        display_signatures,
        signatures,
        exports,
        decls,
    }
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
