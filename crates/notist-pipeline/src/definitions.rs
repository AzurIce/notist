//! Code declaration analysis: syntax views → shared core definitions.
//! No filesystem, dependency loading or plugin execution occurs here.

use notist_core::builtins::{Accepts, Level};
use notist_core::definitions::{
    DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, ReturnRule, ValueType,
    validate_module,
};
use notist_syntax::ast::{FunctionDecl, Module, TypeRef};

use notist_core::diag::{Diagnostic, Phase};
use notist_lowering::literals::syntax_value;

/// Analyze one package's `lib.notc` source into a validated definition module.
/// Any syntax, literal or definition error returns diagnostics instead of a
/// partially installable module. Use `syntax::parse_module` for recovery ASTs.
///
/// ```
/// use notist_pipeline::analyze_module;
/// use notist_core::builtins;
///
/// let module = analyze_module("widgets", "fn badge(label: String) -> InlineContent;").unwrap();
/// let mut registry = builtins::registry().clone();
/// registry.register(module).unwrap();
/// assert_eq!(registry.resolve("widgets::badge").unwrap().id.name, "badge");
/// assert!(registry.resolve("badge").is_err()); // external functions stay qualified
/// ```
pub fn analyze_module(package: &str, source: &str) -> Result<DefinitionModule, Vec<Diagnostic>> {
    let parsed = notist_syntax::parse_module(source);
    let mut diagnostics: Vec<_> = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| Diagnostic::new(Phase::Syntax, diagnostic.span, &diagnostic.message))
        .collect();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let ast = Module::cast(parsed.syntax()).expect("parse_module produces a Module");
    let mut module = DefinitionModule::new(package);
    module.span = ast.range();
    for function in ast.functions() {
        if let Some(definition) = lower_function(package, &function, &mut diagnostics) {
            module.functions.push(definition);
        }
    }
    diagnostics.extend(validate_module(&module));
    if diagnostics.is_empty() {
        Ok(module)
    } else {
        Err(diagnostics)
    }
}

fn lower_function(
    package: &str,
    function: &FunctionDecl,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<FunctionDef> {
    let name = function.name()?;
    let returns = return_rule(&function.return_type()?, diagnostics)?;
    let children = match function.children_decl() {
        Some(mount) => match content_type(&mount.ty()?, diagnostics)? {
            Level::Inline => Accepts::Inline,
            _ => Accepts::Content,
        },
        None => Accepts::Nothing,
    };
    let mut definition = FunctionDef::new(FunctionId::new(package, name.text()), children, returns);
    definition.span = function.range();
    for parameter in function.parameters() {
        let name = parameter.name()?;
        let ty = value_type(&parameter.ty()?, diagnostics)?;
        let mode = if let Some(default) = parameter.default_value() {
            let value = default.value()?;
            ParameterMode::Default(syntax_value(&value, diagnostics)?)
        } else if parameter.is_optional() {
            ParameterMode::Optional
        } else {
            ParameterMode::Required
        };
        let mut lowered = ParameterDef::new(name.text(), ty, mode);
        lowered.span = parameter.range();
        definition.parameters.push(lowered);
    }
    Some(definition)
}

fn return_rule(ty: &TypeRef, diagnostics: &mut Vec<Diagnostic>) -> Option<ReturnRule> {
    let arguments: Vec<_> = ty.arguments().collect();
    if ty.path()?.text() == "Content" && arguments.len() == 1 {
        let selector = &arguments[0];
        let name = selector.path()?.text();
        if selector.arguments().next().is_none() && notist_core::definitions::valid_name(&name) {
            return Some(ReturnRule::BlockIfTrue(name));
        }
        diagnostics.push(Diagnostic::new(
            Phase::Type,
            selector.range(),
            "Content's selector must be a boolean parameter name",
        ));
        return None;
    }
    content_type(ty, diagnostics).map(ReturnRule::Fixed)
}

fn content_type(ty: &TypeRef, diagnostics: &mut Vec<Diagnostic>) -> Option<Level> {
    let name = ty.path()?.text();
    if ty.arguments().next().is_none() {
        match name.as_str() {
            "Content" => return Some(Level::Block),
            "InlineContent" => return Some(Level::Inline),
            _ => {}
        }
    }
    diagnostics.push(Diagnostic::new(
        Phase::Type,
        ty.range(),
        "expected Content or InlineContent",
    ));
    None
}

fn value_type(ty: &TypeRef, diagnostics: &mut Vec<Diagnostic>) -> Option<ValueType> {
    let name = ty.path()?.text();
    let arguments: Vec<_> = ty.arguments().collect();
    let basic = match name.as_str() {
        "Unit" => ValueType::Unit,
        "Bool" => ValueType::Bool,
        "Int" => ValueType::Int,
        "Float" => ValueType::Float,
        "String" => ValueType::String,
        "Dict" => ValueType::Dict,
        "Array" if arguments.len() <= 1 => {
            let element = match arguments.first() {
                Some(argument) => Some(Box::new(value_type(argument, diagnostics)?)),
                None => None,
            };
            return Some(ValueType::Array(element));
        }
        "Array" => {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                ty.range(),
                "Array takes at most one element type",
            ));
            return None;
        }
        "Content" | "InlineContent" => {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                ty.range(),
                "content is mounted as children, not passed as a value parameter",
            ));
            return None;
        }
        _ => {
            diagnostics.push(Diagnostic::new(
                Phase::Type,
                ty.range(),
                format!("unknown value type `{name}`"),
            ));
            return None;
        }
    };
    if !arguments.is_empty() {
        diagnostics.push(Diagnostic::new(
            Phase::Type,
            ty.range(),
            format!("`{name}` takes no type arguments"),
        ));
        None
    } else {
        Some(basic)
    }
}
