/// What a content function's children mount accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Accepts {
    /// Only inline content (hugging `[..]`).
    Inline,
    /// A block content forest; no category conversion is performed.
    Block,
    /// Unconstrained.
    Any,
    /// A structural sequence of table rows, without paragraph promotion.
    Rows,
    /// A structural sequence of table cells, without paragraph promotion.
    Cells,
    /// A structural sequence of list items, without paragraph promotion.
    Items,
    /// No children at all (the payload lives in fields).
    Nothing,
}

/// The level of the element a content function produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Level {
    /// A block-level element; interrupts inline runs.
    Block,
    /// An inline element; flows inside paragraphs.
    Inline,
    /// Transparent: inherits the flavor of its children (`node`).
    Inherit,
}

/// A builtin constructor's signature: mount acceptance and produced level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtorSignature {
    pub accepts: Accepts,
    pub level: Level,
}

/// The builtin constructor table, keyed by source name.
pub const BUILTINS: &[(&str, CtorSignature)] = &[
    (
        "paragraph",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Block,
        },
    ),
    (
        "heading",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Block,
        },
    ),
    (
        "list",
        CtorSignature {
            accepts: Accepts::Items,
            level: Level::Block,
        },
    ),
    (
        "item",
        CtorSignature {
            accepts: Accepts::Block,
            level: Level::Block,
        },
    ),
    (
        "callout",
        CtorSignature {
            accepts: Accepts::Block,
            level: Level::Block,
        },
    ),
    (
        "divider",
        CtorSignature {
            accepts: Accepts::Nothing,
            level: Level::Block,
        },
    ),
    (
        "table",
        CtorSignature {
            accepts: Accepts::Rows,
            level: Level::Block,
        },
    ),
    (
        "row",
        CtorSignature {
            accepts: Accepts::Cells,
            level: Level::Block,
        },
    ),
    (
        "cell",
        CtorSignature {
            accepts: Accepts::Block,
            level: Level::Block,
        },
    ),
    (
        "strong",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Inline,
        },
    ),
    (
        "emph",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Inline,
        },
    ),
    (
        "strike",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Inline,
        },
    ),
    (
        "underline",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Inline,
        },
    ),
    (
        "link",
        CtorSignature {
            accepts: Accepts::Inline,
            level: Level::Inline,
        },
    ),
    (
        "embed",
        CtorSignature {
            accepts: Accepts::Nothing,
            level: Level::Inline,
        },
    ),
    (
        "text",
        CtorSignature {
            accepts: Accepts::Nothing,
            level: Level::Inline,
        },
    ),
    (
        "raw",
        CtorSignature {
            accepts: Accepts::Nothing,
            level: Level::Inline,
        },
    ),
    (
        "math",
        CtorSignature {
            accepts: Accepts::Nothing,
            level: Level::Inline,
        },
    ),
    (
        "node",
        CtorSignature {
            accepts: Accepts::Any,
            level: Level::Inherit,
        },
    ),
    (
        "section",
        CtorSignature {
            accepts: Accepts::Block,
            level: Level::Block,
        },
    ),
];

/// The signature of a builtin constructor by source name (`None` for custom).
pub fn builtin_signature(name: &str) -> Option<CtorSignature> {
    crate::item::Ctor::from_name(name)?;
    let definition = registry().function("notist", name)?;
    Some(CtorSignature {
        accepts: definition.children,
        level: definition.returns.level().unwrap_or(Level::Inherit),
    })
}

/// All builtin constructor names.
pub fn builtin_ctors() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|(n, _)| *n)
}

/// Native definitions use the same model and validation as source declarations.
pub fn definitions() -> crate::definitions::DefinitionModule {
    use crate::definitions::{
        ComptimeParameter, DefinitionModule, FunctionDef, FunctionId, ParameterDef,
        ParameterMode as Mode, Type as Ty, ValueConstraint,
    };
    use crate::item::Value;

    let mut module = DefinitionModule::new("notist");
    for &(name, signature) in BUILTINS {
        let returns = match name {
            "raw" | "math" => Ty::Parameter("C".into()),
            "node" => Ty::Parameter("C".into()),
            _ => {
                if signature.level == Level::Block {
                    Ty::BlockContent
                } else {
                    Ty::InlineContent
                }
            }
        };
        let mut function =
            FunctionDef::new(FunctionId::new("notist", name), signature.accepts, returns);
        if matches!(
            name,
            "raw" | "math" | "node" | "callout" | "item" | "cell" | "section"
        ) {
            function.comptime.push(ComptimeParameter {
                name: "C".into(),
                constraint: ValueConstraint::Any,
                ty: Ty::Type,
                default: Some(crate::definitions::ComptimeValue::Type(
                    if matches!(name, "raw" | "math" | "node") {
                        Ty::InlineContent
                    } else {
                        Ty::BlockContent
                    },
                )),
                span: rowan::TextRange::empty(0.into()),
            });
            if !matches!(name, "raw" | "math") {
                function.children_type = Some(Ty::Parameter("C".into()));
                function.children = Accepts::Any;
            }
        }
        if name == "link" {
            let span = rowan::TextRange::empty(0.into());
            function.comptime.push(ComptimeParameter {
                name: "T".into(),
                ty: Ty::Type,
                constraint: ValueConstraint::Any,
                default: None,
                span,
            });
            let eq = |ty: &str| crate::expr::Expr::Binary {
                op: "==".into(),
                left: Box::new(crate::expr::Expr::reference("T", span)),
                right: Box::new(crate::expr::Expr::reference(ty, span)),
                span,
            };
            function.where_clause = Some(crate::expr::Expr::Binary {
                op: "||".into(),
                left: Box::new(eq("Path")),
                right: Box::new(eq("String")),
                span,
            });
        }
        let mut parameter = |name: &str, ty, mode, positional| {
            let mut parameter = ParameterDef::new(name, ty, mode);
            parameter.positional = positional;
            function.parameters.push(parameter);
        };
        match name {
            "text" => parameter("text", Ty::String, Mode::Required, false),
            "math" => {
                parameter("text", Ty::String, Mode::Required, true);
            }
            "raw" => {
                parameter("text", Ty::String, Mode::Required, true);
                parameter("lang", Ty::String, Mode::Optional, false);
            }
            "link" => {
                parameter("target", Ty::Parameter("T".into()), Mode::Required, true);
                parameter("title", Ty::String, Mode::Optional, false);
                parameter("query", Ty::String, Mode::Optional, false);
                parameter("fragment", Ty::String, Mode::Optional, false);
            }
            "embed" => {
                parameter("target", Ty::ResourceTarget, Mode::Required, true);
                parameter("description", Ty::String, Mode::Optional, false);
                parameter("title", Ty::String, Mode::Optional, false);
                parameter("query", Ty::String, Mode::Optional, false);
                parameter("fragment", Ty::String, Mode::Optional, false);
            }
            "heading" => {
                function.comptime.push(ComptimeParameter {
                    name: "level".into(),
                    constraint: ValueConstraint::PositiveInt,
                    ty: Ty::Int,
                    default: Some(crate::definitions::ComptimeValue::Data(Value::Int(1))),
                    span: rowan::TextRange::empty(0.into()),
                });
            }
            "list" => {
                parameter(
                    "ordered",
                    Ty::Bool,
                    Mode::Default(Value::Bool(false)),
                    false,
                );
                parameter("start", Ty::Int, Mode::Default(Value::Int(1)), false);
            }
            "callout" => parameter("kind", Ty::String, Mode::Optional, false),
            "table" => {
                parameter(
                    "align",
                    Ty::Array(Some(Box::new(Ty::String))),
                    Mode::Optional,
                    false,
                );
                function.parameters[0].constraint = ValueConstraint::ArrayStrings(
                    ["none", "left", "center", "right"]
                        .map(str::to_string)
                        .into(),
                );
                function.parameters[0].expectation = Some("alignment array".into());
            }
            "row" => parameter("header", Ty::Bool, Mode::Optional, false),
            _ => {}
        }
        module.functions.push(function);
    }
    let mut read = FunctionDef::new(
        FunctionId::new("notist", "read_text"),
        Accepts::Nothing,
        Ty::String,
    );
    read.implementation =
        crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::ReadText);
    read.parameters
        .push(ParameterDef::new("source", Ty::Path, Mode::Required));
    module.functions.push(read);
    let mut panic = FunctionDef::new(
        FunctionId::new("notist", "panic"),
        Accepts::Nothing,
        Ty::Never,
    );
    panic.implementation =
        crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::Panic);
    panic
        .parameters
        .push(ParameterDef::new("message", Ty::String, Mode::Required));
    module.functions.push(panic);
    module.functions.extend(crate::data::definitions());
    let mut string = FunctionDef::new(
        FunctionId::new("notist", "str"),
        Accepts::Nothing,
        Ty::String,
    );
    string.implementation =
        crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::Str);
    string.comptime.push(ComptimeParameter {
        name: "T".into(),
        ty: Ty::Type,
        default: None,
        constraint: ValueConstraint::Any,
        span: string.span,
    });
    string.parameters.push(ParameterDef::new(
        "value",
        Ty::Parameter("T".into()),
        Mode::Required,
    ));
    module.functions.push(string);
    module
}

/// Immutable standard signatures, installed through the shared atomic registry.
pub fn registry() -> &'static crate::registry::Registry {
    use crate::definitions::{
        FunctionDef, FunctionId, ParameterDef, ParameterMode as Mode, Type as Ty,
    };
    static REGISTRY: std::sync::OnceLock<crate::registry::Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = crate::registry::Registry::new();
        registry
            .install(definitions(), true)
            .expect("native content definitions must be valid");
        let mut array = crate::definitions::DefinitionModule::new("notist.array");
        let mut len = crate::definitions::FunctionDef::new(
            crate::definitions::FunctionId::new("notist.array", "len"),
            Accepts::Nothing,
            crate::definitions::Type::Int,
        );
        len.implementation =
            crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::ArrayLen);
        len.parameters.push(crate::definitions::ParameterDef::new(
            "self",
            crate::definitions::Type::Array(None),
            crate::definitions::ParameterMode::Required,
        ));
        array.functions.push(len);
        registry
            .install(array, true)
            .expect("native array methods must be valid");
        let mut path = crate::definitions::DefinitionModule::new("notist.path");
        let mut join = FunctionDef::new(
            FunctionId::new("notist.path", "join"),
            Accepts::Nothing,
            Ty::Path,
        );
        join.implementation =
            crate::definitions::FunctionImpl::Native(crate::definitions::NativeFunction::PathJoin);
        join.parameters
            .push(ParameterDef::new("self", Ty::Path, Mode::Required));
        let mut relative = ParameterDef::new("relative", Ty::String, Mode::Required);
        relative.positional = true;
        join.parameters.push(relative);
        path.functions.push(join);
        registry
            .install(path, true)
            .expect("native path methods must be valid");
        registry
    })
}
