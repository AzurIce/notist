//! Module registration lowers Code bodies and embedded Markup to IR₁.
//! Each concrete instance runs the shared Markup backend later, with known
//! comptime arguments and typed ordinary inputs.
use crate::code::literal::resolve_literal;
use crate::frontend::PipelineFrontend;
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
use std::hash::{Hash, Hasher};
use std::{collections::BTreeMap, sync::Arc};

struct ContentCompiler<'a> {
    key: notist_core::frontend::FrontendCacheKey,
    frontend: &'a dyn ContentLowering,
}

impl Hash for ContentCompiler<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

pub(crate) fn compile_module(
    module: &str,
    path: &str,
    source: &str,
    resource_root: &str,
    frontend: &dyn PipelineFrontend,
) -> Result<CodeModule, Vec<Diagnostic>> {
    let timing =
        crate::progress::Span::stage("core.module.cache", || format!("Module cache {module}"));
    let probe = crate::progress::timings::CacheProbe::new("core.module.compile.work");
    let enabled = frontend.cache_key().is_some() && cache_enabled();
    let result = match frontend.cache_key() {
        Some(key) => compile_cached(
            module,
            path,
            source,
            resource_root,
            ContentCompiler { key, frontend },
        ),
        None => compile_uncached(module, path, source, resource_root, frontend),
    };
    timing.attribute("cache", || {
        if enabled { probe.outcome() } else { "bypass" }.into()
    });
    if result.is_ok() {
        timing.finish();
    }
    result
}

fn cache_enabled() -> bool {
    #[cfg(feature = "profiling")]
    {
        crate::profiling::compile_enabled()
    }
    #[cfg(not(feature = "profiling"))]
    {
        true
    }
}

#[comemo::memoize(enabled = cache_enabled())]
fn compile_cached(
    module: &str,
    path: &str,
    source: &str,
    resource_root: &str,
    compiler: ContentCompiler<'_>,
) -> Result<CodeModule, Vec<Diagnostic>> {
    compile_uncached(module, path, source, resource_root, compiler.frontend)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodeProgram {
    modules: Arc<BTreeMap<notist_core::definitions::ModuleId, Arc<LinkedModule>>>,
    scopes: Arc<BTreeMap<notist_core::definitions::ModuleId, Arc<ModuleScope>>>,
    identities: notist_core::definition_identity::Definitions,
    sources: Sources,
}
impl CodeProgram {
    /// Collect shared source texts at the output boundary, without copying bodies.
    pub fn sources(&self) -> Sources {
        self.sources.clone()
    }
    pub fn source_path(&self, module: &str) -> Option<&str> {
        Some(self.module(module)?.code().source_path())
    }
    pub fn module(&self, module: &str) -> Option<&Arc<LinkedModule>> {
        self.module_by_id(self.identities.module_id(module)?)
    }
    pub fn module_by_id(
        &self,
        module: notist_core::definitions::ModuleId,
    ) -> Option<&Arc<LinkedModule>> {
        self.modules.get(&module)
    }
    pub fn scope(&self, module: &str) -> Option<&Arc<ModuleScope>> {
        self.scope_by_id(self.identities.module_id(module)?)
    }
    pub fn scope_by_id(
        &self,
        module: notist_core::definitions::ModuleId,
    ) -> Option<&Arc<ModuleScope>> {
        self.scopes
            .get(&module)
            .or_else(|| self.module_by_id(module).map(|m| m.scope()))
    }
    pub(crate) fn body_by_id(
        &self,
        function: notist_core::definitions::DefId,
    ) -> Option<&CodeBody> {
        let module = self.identities.function(function).ok()?.module_id();
        self.module_by_id(module)?.code().body_by_id(function)
    }
    pub(crate) fn install(
        &mut self,
        module: Arc<LinkedModule>,
        identities: &notist_core::definition_identity::Definitions,
        registered: &notist_core::definition_identity::Definitions,
    ) -> Result<(), notist_core::source::SourceError> {
        self.sources.insert(module.code().source_record().clone())?;
        self.identities = identities.clone();
        *self.sources.definitions_mut() = registered.clone();
        Arc::make_mut(&mut self.modules).insert(module.code().module_id(), module);
        Ok(())
    }
    pub(crate) fn with_scope(
        &mut self,
        id: notist_core::definitions::ModuleId,
        scope: Arc<ModuleScope>,
        identities: &notist_core::definition_identity::Definitions,
    ) {
        self.identities = identities.clone();
        Arc::make_mut(&mut self.scopes).insert(id, scope);
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeBody {
    // Source declaration metadata, retained independently of mutable public
    // signature snapshots so COW edits cannot orphan the body's index names.
    pub definition: FunctionId,
    pub expression: Option<Expr>,
    pub diagnostics: Vec<Diagnostic>,
    pub source: notist_core::source::Source,
    pub resource_root: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CodeModule {
    pub definitions: Arc<DefinitionModule>,
    pub imports: Arc<[Import]>,
    bodies: Arc<BTreeMap<notist_core::definitions::DefId, CodeBody>>,
    source: notist_core::source::Source,
}

impl CodeModule {
    pub(crate) fn linked_snapshot(code: Arc<Self>) -> Arc<Self> {
        if code
            .bodies
            .values()
            .all(|body| body.definition.module_id() == code.module_id())
        {
            return code;
        }
        let mut snapshot = (*code).clone();
        let bodies = code
            .bodies
            .iter()
            .map(|(id, body)| {
                if let Some(definition) = code
                    .definitions
                    .functions
                    .iter()
                    .find(|definition| definition.id.name == body.definition.name)
                {
                    let mut body = body.clone();
                    body.definition = definition.id.clone();
                    (definition.id.def_id(), body)
                } else {
                    (*id, body.clone())
                }
            })
            .collect();
        snapshot.bodies = Arc::new(bodies);
        Arc::new(snapshot)
    }
    pub fn module_id(&self) -> notist_core::definitions::ModuleId {
        self.definitions.module.id()
    }
    pub fn id(&self) -> &str {
        &self.definitions.module
    }
    pub fn source_path(&self) -> &str {
        self.source.name()
    }
    pub fn source(&self) -> &Arc<str> {
        self.source
            .text()
            .expect("compiled modules retain their input text")
    }
    pub fn source_record(&self) -> &notist_core::source::Source {
        &self.source
    }
    pub(crate) fn body_by_id(&self, id: notist_core::definitions::DefId) -> Option<&CodeBody> {
        self.bodies.get(&id)
    }
    pub(crate) fn bodies(&self) -> impl Iterator<Item = &CodeBody> {
        let mut bodies: Vec<_> = self.bodies.values().collect();
        bodies.sort_by(|a, b| a.definition.name.cmp(&b.definition.name));
        bodies.into_iter()
    }
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.bodies().flat_map(|b| b.diagnostics.iter())
    }
}

fn compile_uncached(
    module: &str,
    path: &str,
    source: &str,
    resource_root: &str,
    frontend: &dyn ContentLowering,
) -> Result<CodeModule, Vec<Diagnostic>> {
    crate::progress::run_stage(
        "core.module.compile.work",
        || format!("Compile module body {module}"),
        || {
            #[cfg(feature = "profiling")]
            let _timer = crate::profiling::enter(crate::profiling::Stage::CompileWork);
            let origin = notist_core::source::Source::in_namespace(
                notist_core::source::SourceNamespace::named(resource_root),
                path,
                Some(source.into()),
            )
            .map_err(|error| {
                vec![Diagnostic::new(
                    Phase::Type,
                    Default::default(),
                    error.to_string(),
                )]
            })?;
            let parsed = crate::progress::measure(
                "core.parse",
                || "Parse Code module".into(),
                || notist_syntax::code::parse_module(source, frontend),
            );
            let lowering =
                crate::progress::Span::stage("core.lower", || "Lower Code module".into());
            let ast = Module::cast(parsed.syntax()).unwrap();
            let mut diagnostics = Vec::new();
            let identity =
                notist_core::definitions::ModuleRef::try_new(module).map_err(|error| {
                    let mut diagnostic =
                        Diagnostic::new(Phase::Type, ast.range(), error.to_string());
                    diagnostic.source = Some(origin.clone());
                    vec![diagnostic]
                })?;
            let mut definitions = DefinitionModule {
                module: identity,
                functions: Vec::new(),
                span: ast.range(),
            };
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
                    diagnostic.source = Some(origin.clone());
                    if body_range.is_some_and(|r| r.contains_inclusive(d.span.start())) {
                        body_diags.push(diagnostic);
                        assigned[i] = true;
                    } else if function.range().contains_range(d.span) {
                        diagnostics.push(diagnostic);
                        assigned[i] = true;
                    }
                }
                if let Some(definition) =
                    lower_function(module, &function, resource_root, &origin, &mut diagnostics)
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
                        d.source = Some(origin.clone());
                    }
                    if definition.implementation.is_code() {
                        bodies.insert(
                            definition.id.def_id(),
                            CodeBody {
                                definition: definition.id.clone(),
                                expression: expr,
                                diagnostics: body_diags,
                                source: origin.clone(),
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
            lowering.finish();
            if diagnostics.is_empty() {
                Ok(CodeModule {
                    definitions: Arc::new(definitions),
                    bodies: Arc::new(bodies),
                    imports: imports.into(),
                    source: origin,
                })
            } else {
                for diagnostic in &mut diagnostics {
                    diagnostic.source.get_or_insert_with(|| origin.clone());
                }
                Err(diagnostics)
            }
        },
    )
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
    if let Some(ty) = Type::builtin(&name) {
        Some(ty)
    } else if params.iter().any(|p| p.name == name && p.ty == Type::Type) {
        Some(Type::Parameter(name))
    } else {
        diags.push(Diagnostic::new(
            Phase::Type,
            t.range(),
            format!("unknown type `{name}`"),
        ));
        None
    }
}
fn ct_default(
    p: &Parameter,
    ty: &Type,
    root: &str,
    source: &notist_core::source::Source,
    diags: &mut Vec<Diagnostic>,
) -> Option<ComptimeValue> {
    let el = p.default_value()?.value()?;
    if *ty == Type::Type {
        let name = el.to_string();
        // Preserve the declaration-default grammar: erased Array is available
        // in annotations/expressions, but is not a supported type default.
        let Some(ty) = Type::builtin(&name).filter(|ty| !matches!(ty, Type::Array(_))) else {
            diags.push(Diagnostic::new(
                Phase::Type,
                el.text_range(),
                "expected a type default",
            ));
            return None;
        };
        Some(ComptimeValue::Type(ty))
    } else {
        default_value(&el, root, source, diags).map(ComptimeValue::Data)
    }
}
fn default_value(
    el: &rowan::NodeOrToken<notist_syntax::tree::SyntaxNode, notist_syntax::tree::SyntaxToken>,
    root: &str,
    source: &notist_core::source::Source,
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
    source: &notist_core::source::Source,
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
    let identity = match FunctionId::try_new(module, f.name()?.text()) {
        Ok(identity) => identity,
        Err(error) => {
            diags.push(Diagnostic::new(Phase::Type, f.range(), error.to_string()));
            return None;
        }
    };
    let mut definition = FunctionDef::new(identity, accepts, returns);
    definition.comptime = comptime;
    if let Some(clause) = f.where_clause() {
        definition.where_clause = expression(clause.condition()?, None, None, diags);
        if definition.where_clause.is_none() {
            diags.push(Diagnostic::new(
                Phase::Type,
                clause.range(),
                "invalid where expression",
            ));
        }
    }
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
