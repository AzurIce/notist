//! Package registration lowers bodies without executing them. Each concrete
//! instance is checked later, with known comptime arguments and unknown inputs.
use crate::lowering::{code::expression, content::ContentLowering, literals::syntax_value};
use notist_core::{
    builtins::Accepts,
    definitions::{
        ComptimeParameter, ComptimeValue, DefinitionModule, FunctionDef, FunctionId, ParameterDef,
        ParameterMode, Type, validate_module,
    },
    diag::{Diagnostic, Phase},
    expr::Expr,
};
use notist_syntax::code::ast::{FunctionDecl, Module, Parameter, TypeRef};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodeProgram {
    pub(crate) bodies: BTreeMap<FunctionId, CodeBody>,
    pub(crate) sources: BTreeMap<String, String>,
    pub(crate) modules: BTreeMap<String, String>,
}
impl CodeProgram {
    pub fn sources(&self) -> &BTreeMap<String, String> {
        &self.sources
    }
    pub fn source_path(&self, package: &str) -> Option<&str> {
        self.modules.get(package).map(String::as_str)
    }
    pub fn package_for_source(&self, path: &str) -> Option<&str> {
        self.modules
            .iter()
            .find_map(|(package, source)| (source == path).then_some(package.as_str()))
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeBody {
    pub expression: Option<Expr>,
    pub diagnostics: Vec<Diagnostic>,
    pub source: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CodeModule {
    pub definitions: DefinitionModule,
    pub program: CodeProgram,
}

impl CodeModule {
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.program
            .bodies
            .values()
            .flat_map(|b| b.diagnostics.iter())
    }
}

/// Compile an in-memory native-Markup Code module. Body diagnostics are kept
/// with the implementation, so unrelated functions remain available.
pub fn analyze_module(package: &str, source: &str) -> Result<CodeModule, Vec<Diagnostic>> {
    compile_module(
        package,
        &format!("{package}/lib.notc"),
        source,
        &crate::frontend::markup::NotistFrontend,
    )
}

pub(crate) fn compile_module(
    package: &str,
    path: &str,
    source: &str,
    frontend: &dyn ContentLowering,
) -> Result<CodeModule, Vec<Diagnostic>> {
    let parsed = notist_syntax::code::parse_module(source, frontend);
    let ast = Module::cast(parsed.syntax()).unwrap();
    let mut diagnostics = Vec::new();
    let mut definitions = DefinitionModule::new(package);
    definitions.span = ast.range();
    let mut program = CodeProgram::default();
    program.sources.insert(path.into(), source.into());
    program.modules.insert(package.into(), path.into());
    let mut assigned = vec![false; parsed.diagnostics.len()];
    for function in ast.functions() {
        let body = function.body();
        let body_range = body.as_ref().map(|n| n.text_range());
        let mut body_diags = Vec::new();
        for (i, d) in parsed.diagnostics.iter().enumerate() {
            if assigned[i] {
                continue;
            }
            let mut diagnostic = Diagnostic::new(Phase::Syntax, d.span, &d.message);
            diagnostic.source = Some(path.into());
            if body_range.is_some_and(|r| r.contains_inclusive(d.span.start())) {
                body_diags.push(diagnostic);
                assigned[i] = true;
            } else if function.range().contains_range(d.span) {
                diagnostics.push(diagnostic);
                assigned[i] = true;
            }
        }
        if let Some(definition) = lower_function(package, &function, &mut diagnostics) {
            let expr = body.and_then(|n| {
                expression(
                    rowan::NodeOrToken::Node(n),
                    Some(&parsed),
                    Some(frontend),
                    &mut body_diags,
                )
            });
            for d in &mut body_diags {
                d.source = Some(path.into());
            }
            if definition.code {
                program.bodies.insert(
                    definition.id.clone(),
                    CodeBody {
                        expression: expr,
                        diagnostics: body_diags,
                        source: path.into(),
                    },
                );
            }
            definitions.functions.push(definition);
        }
    }
    for (i, d) in parsed.diagnostics.iter().enumerate() {
        if !assigned[i] {
            diagnostics.push(Diagnostic::new(Phase::Syntax, d.span, &d.message));
        }
    }
    diagnostics.extend(validate_module(&definitions));
    if diagnostics.is_empty() {
        Ok(CodeModule {
            definitions,
            program,
        })
    } else {
        Err(diagnostics)
    }
}

fn lower_type(
    t: &TypeRef,
    params: &[ComptimeParameter],
    diags: &mut Vec<Diagnostic>,
) -> Option<Type> {
    let name = t.path()?.text();
    let args: Vec<_> = t.arguments().collect();
    if name == "Array" && args.len() <= 1 {
        return Some(Type::Array(match args.first() {
            Some(t) => Some(Box::new(lower_type(t, params, diags)?)),
            None => None,
        }));
    }
    if !args.is_empty() {
        diags.push(Diagnostic::new(
            Phase::Type,
            t.range(),
            "type constructors only support Array<T>",
        ));
        return None;
    }
    Some(match name.as_str() {
        "Type" => Type::Type,
        "Unit" => Type::Unit,
        "Bool" => Type::Bool,
        "Int" => Type::Int,
        "Float" => Type::Float,
        "String" => Type::String,
        "Dict" => Type::Dict,
        "InlineContent" => Type::InlineContent,
        "BlockContent" => Type::BlockContent,
        _ if params.iter().any(|p| p.name == name && p.ty == Type::Type) => Type::Parameter(name),
        _ => {
            diags.push(Diagnostic::new(
                Phase::Type,
                t.range(),
                format!("unknown type `{name}`"),
            ));
            return None;
        }
    })
}
fn ct_default(p: &Parameter, ty: &Type, diags: &mut Vec<Diagnostic>) -> Option<ComptimeValue> {
    let el = p.default_value()?.value()?;
    if *ty == Type::Type {
        let name = el.to_string();
        let ty = match name.as_str() {
            "Type" => Type::Type,
            "InlineContent" => Type::InlineContent,
            "BlockContent" => Type::BlockContent,
            "Bool" => Type::Bool,
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Dict" => Type::Dict,
            "Unit" => Type::Unit,
            _ => {
                diags.push(Diagnostic::new(
                    Phase::Type,
                    el.text_range(),
                    "expected a type default",
                ));
                return None;
            }
        };
        Some(ComptimeValue::Type(ty))
    } else {
        syntax_value(&el, diags).map(ComptimeValue::Data)
    }
}
fn lower_function(
    package: &str,
    f: &FunctionDecl,
    diags: &mut Vec<Diagnostic>,
) -> Option<FunctionDef> {
    let mut comptime = Vec::new();
    for p in f.comptime_parameters() {
        let ty = lower_type(&p.ty()?, &comptime, diags)?;
        let default = ct_default(&p, &ty, diags);
        comptime.push(ComptimeParameter {
            constraint: Default::default(),
            name: p.name()?.text().into(),
            ty,
            default,
            span: p.range(),
        });
    }
    let returns = lower_type(&f.return_type()?, &comptime, diags)?;
    let children_type = match f.children_decl() {
        Some(c) => Some(lower_type(&c.ty()?, &comptime, diags)?),
        None => None,
    };
    let accepts = match children_type.as_ref() {
        None => Accepts::Nothing,
        Some(Type::InlineContent) => Accepts::Inline,
        Some(Type::BlockContent) => Accepts::Block,
        Some(Type::Parameter(_)) => Accepts::Any,
        _ => {
            diags.push(Diagnostic::new(
                Phase::Type,
                f.range(),
                "children must have a content type",
            ));
            return None;
        }
    };
    let mut definition =
        FunctionDef::new(FunctionId::new(package, f.name()?.text()), accepts, returns);
    definition.comptime = comptime;
    definition.children_type = children_type;
    definition.span = f.range();
    definition.code = f.body().is_some();
    for p in f.parameters() {
        let ty = lower_type(&p.ty()?, &definition.comptime, diags)?;
        let mode = if let Some(default) = p.default_value() {
            ParameterMode::Default(syntax_value(&default.value()?, diags)?)
        } else if p.is_optional() {
            if definition.code {
                diags.push(Diagnostic::new(
                    Phase::Type,
                    p.range(),
                    "Code parameters require a value or a concrete default",
                ));
            }
            ParameterMode::Optional
        } else {
            ParameterMode::Required
        };
        let mut param = ParameterDef::new(p.name()?.text(), ty, mode);
        param.span = p.range();
        definition.parameters.push(param);
    }
    Some(definition)
}
