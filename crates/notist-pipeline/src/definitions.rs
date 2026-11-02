//! Module registration lowers Code bodies and embedded Markup to IR₁.
//! Each concrete instance runs the shared Markup backend later, with known
//! comptime arguments and typed ordinary inputs.
use crate::code::literal::resolve_literal;
use crate::lowering::{code::expression, content::ContentLowering, literals::literal_expression};
use crate::modules::{LinkedModule, ModuleScope};
use notist_core::{
    builtins::Accepts,
    definitions::{
        ComptimeParameter, ComptimeValue, DefinitionModule, FunctionDef, FunctionId, Import,
        ParameterDef, ParameterMode, Type, validate_module,
    },
    diag::{Diagnostic, Phase, Sources},
    expr::Expr,
};
use notist_syntax::code::ast::{FunctionDecl, Module, Parameter, TypeRef};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodeProgram {
    modules: Arc<BTreeMap<String, Arc<LinkedModule>>>,
    scopes: Arc<BTreeMap<String, Arc<ModuleScope>>>,
}
impl CodeProgram {
    /// Collect shared source texts at the output boundary, without copying bodies.
    pub fn sources(&self) -> Sources {
        self.modules
            .values()
            .map(|module| {
                let code = module.code();
                (code.source_path().to_owned(), code.source().clone())
            })
            .collect()
    }
    pub fn source_path(&self, module: &str) -> Option<&str> {
        Some(self.module(module)?.code().source_path())
    }
    pub fn module(&self, module: &str) -> Option<&Arc<LinkedModule>> {
        self.modules.get(module)
    }
    pub fn scope(&self, module: &str) -> Option<&Arc<ModuleScope>> {
        self.scopes
            .get(module)
            .or_else(|| self.module(module).map(|m| m.scope()))
    }
    pub(crate) fn body(&self, function: &FunctionId) -> Option<&CodeBody> {
        self.module(&function.module)?.code().body(&function.name)
    }
    pub(crate) fn install(&mut self, module: Arc<LinkedModule>) {
        Arc::make_mut(&mut self.modules).insert(module.code().id().to_owned(), module);
    }
    pub(crate) fn with_scope(&mut self, module: String, scope: Arc<ModuleScope>) {
        Arc::make_mut(&mut self.scopes).insert(module, scope);
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeBody {
    pub expression: Option<Expr>,
    pub diagnostics: Vec<Diagnostic>,
    pub source: String,
    pub resource_root: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CodeModule {
    pub definitions: Arc<DefinitionModule>,
    pub imports: Arc<[Import]>,
    bodies: Arc<BTreeMap<String, CodeBody>>,
    source_path: Arc<str>,
    source: Arc<str>,
}

impl CodeModule {
    pub fn id(&self) -> &str {
        &self.definitions.module
    }
    pub fn source_path(&self) -> &str {
        &self.source_path
    }
    pub fn source(&self) -> &Arc<str> {
        &self.source
    }
    pub(crate) fn body(&self, name: &str) -> Option<&CodeBody> {
        self.bodies.get(name)
    }
    pub(crate) fn bodies(&self) -> impl Iterator<Item = &CodeBody> {
        self.bodies.values()
    }
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.bodies.values().flat_map(|b| b.diagnostics.iter())
    }
}

pub(crate) fn compile_module(
    module: &str,
    path: &str,
    source: &str,
    resource_root: &str,
    frontend: &dyn ContentLowering,
) -> Result<CodeModule, Vec<Diagnostic>> {
    let parsed = notist_syntax::code::parse_module(source, frontend);
    let ast = Module::cast(parsed.syntax()).unwrap();
    let mut diagnostics = Vec::new();
    let mut definitions = DefinitionModule::new(module);
    definitions.span = ast.range();
    let mut bodies = BTreeMap::new();
    let imports = crate::lowering::imports::lower_imports(ast.imports(), &mut diagnostics);
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
        if let Some(definition) =
            lower_function(module, &function, resource_root, path, &mut diagnostics)
        {
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
            if definition.implementation.is_code() {
                bodies.insert(
                    definition.id.name.clone(),
                    CodeBody {
                        expression: expr,
                        diagnostics: body_diags,
                        source: path.into(),
                        resource_root: resource_root.into(),
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
            definitions: Arc::new(definitions),
            bodies: Arc::new(bodies),
            imports: imports.into(),
            source_path: path.into(),
            source: source.into(),
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
    let name = t.name()?.text().to_string();
    let args: Vec<_> = t.arguments().collect();
    if name == "Array" && args.len() <= 1 {
        return Some(Type::Array(match args.first() {
            Some(t) => Some(Box::new(lower_type(t, params, diags)?)),
            None => None,
        }));
    }
    if name == "Option" && args.len() == 1 {
        return Some(Type::Option(Box::new(lower_type(&args[0], params, diags)?)));
    }
    if name == "Result" && args.len() == 2 {
        return Some(Type::Result(
            Box::new(lower_type(&args[0], params, diags)?),
            Box::new(lower_type(&args[1], params, diags)?),
        ));
    }
    if matches!(name.as_str(), "Option" | "Result") {
        diags.push(Diagnostic::new(
            Phase::Type,
            t.range(),
            format!(
                "{name} requires {} type arguments",
                if name == "Result" { 2 } else { 1 }
            ),
        ));
        return None;
    }
    if !args.is_empty() {
        diags.push(Diagnostic::new(
            Phase::Type,
            t.range(),
            "type constructors only support Array<T>, Option<T> and Result<T, E>",
        ));
        return None;
    }
    Some(match name.as_str() {
        "Type" => Type::Type,
        "Never" => Type::Never,
        "Data" => Type::Data,
        "DataError" => Type::DataError,
        "Unit" => Type::Unit,
        "Bool" => Type::Bool,
        "Int" => Type::Int,
        "Float" => Type::Float,
        "String" => Type::String,
        "Path" => Type::Path,
        "ResourceTarget" => Type::ResourceTarget,
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
fn ct_default(
    p: &Parameter,
    ty: &Type,
    root: &str,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<ComptimeValue> {
    let el = p.default_value()?.value()?;
    if *ty == Type::Type {
        let name = el.to_string();
        let ty = match name.as_str() {
            "Type" => Type::Type,
            "Never" => Type::Never,
            "Data" => Type::Data,
            "DataError" => Type::DataError,
            "InlineContent" => Type::InlineContent,
            "BlockContent" => Type::BlockContent,
            "Bool" => Type::Bool,
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Path" => Type::Path,
            "ResourceTarget" => Type::ResourceTarget,
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
        default_value(&el, root, source, diags).map(ComptimeValue::Data)
    }
}
fn default_value(
    el: &rowan::NodeOrToken<notist_syntax::tree::SyntaxNode, notist_syntax::tree::SyntaxToken>,
    root: &str,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<notist_core::item::Value> {
    let expr = literal_expression(el, diags)?;
    match resolve_literal(&expr, root, source) {
        Ok(value) => Some(value),
        Err(error) => {
            diags.push(error);
            None
        }
    }
}
fn lower_function(
    module: &str,
    f: &FunctionDecl,
    resource_root: &str,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<FunctionDef> {
    let mut comptime = Vec::new();
    for p in f.comptime_parameters() {
        let ty = lower_type(&p.ty()?, &comptime, diags)?;
        let default = ct_default(&p, &ty, resource_root, source, diags);
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
        FunctionDef::new(FunctionId::new(module, f.name()?.text()), accepts, returns);
    definition.comptime = comptime;
    definition.children_type = children_type;
    definition.span = f.range();
    if f.body().is_some() {
        definition.implementation = notist_core::definitions::FunctionImpl::Code;
    }
    for p in f.parameters() {
        let ty = lower_type(&p.ty()?, &definition.comptime, diags)?;
        let mode = if let Some(default) = p.default_value() {
            ParameterMode::Default(default_value(
                &default.value()?,
                resource_root,
                source,
                diags,
            )?)
        } else if p.is_optional() {
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
