//! Authoritative resolved signature templates. Named declarations are input and
//! presentation metadata, never keys for inference or substitution.
use crate::{
    binding::{BindingId, TypeParamId},
    definitions::{ComptimeValue, DefId, DefinitionModule, FunctionDef, Type},
};
use std::{collections::BTreeMap, fmt, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedType {
    Concrete(Type),
    Parameter(TypeParamId),
    Array(Option<Box<Self>>),
    Option(Box<Self>),
    Result(Box<Self>, Box<Self>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComptimeId {
    Type(TypeParamId),
    Value(BindingId),
}
#[derive(Clone, Debug)]
pub struct ResolvedComptimeParameter {
    pub id: ComptimeId,
    pub name: String,
    pub ty: ResolvedType,
}
#[derive(Clone, Debug)]
pub struct ResolvedSignature {
    declaration: SignatureDeclaration,
    comptime: Vec<ResolvedComptimeParameter>,
    parameters: Vec<ResolvedType>,
    returns: ResolvedType,
    children_type: Option<ResolvedType>,
    type_parameters: BTreeMap<TypeParamId, usize>,
}
#[derive(Clone, Debug)]
enum SignatureDeclaration {
    Module(Arc<DefinitionModule>, usize),
    Owned(Arc<FunctionDef>),
}
impl SignatureDeclaration {
    fn get(&self) -> &FunctionDef {
        match self {
            Self::Module(module, index) => &module.functions[*index],
            Self::Owned(definition) => definition,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureError(pub String);
impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SignatureError {}

impl ResolvedSignature {
    pub fn new(definition: &FunctionDef) -> Result<Self, &'static str> {
        Self::from_shared(Arc::new(definition.clone()))
    }
    pub fn from_shared(definition: Arc<FunctionDef>) -> Result<Self, &'static str> {
        Self::resolve_declaration(SignatureDeclaration::Owned(definition))
    }
    pub(crate) fn from_module(
        module: Arc<DefinitionModule>,
        index: usize,
    ) -> Result<Self, &'static str> {
        if index >= module.functions.len() {
            return Err("invalid signature declaration index");
        }
        Self::resolve_declaration(SignatureDeclaration::Module(module, index))
    }
    fn resolve_declaration(declaration: SignatureDeclaration) -> Result<Self, &'static str> {
        let definition = declaration.get();
        let mut names = BTreeMap::new();
        let mut type_parameters = BTreeMap::new();
        for (index, parameter) in definition.comptime.iter().enumerate() {
            if matches!(&parameter.default, Some(ComptimeValue::Type(ty)) if !ty.concrete()) {
                return Err("type defaults require a concrete Type");
            }
            let id = if parameter.ty == Type::Type {
                let id = TypeParamId::allocate()?;
                type_parameters.insert(id, index);
                ComptimeId::Type(id)
            } else {
                ComptimeId::Value(BindingId::allocate()?)
            };
            if names.insert(parameter.name.clone(), id).is_some() {
                return Err("duplicate comptime declaration");
            }
        }
        fn resolve(
            ty: &Type,
            names: &BTreeMap<String, ComptimeId>,
        ) -> Result<ResolvedType, &'static str> {
            Ok(match ty {
                Type::Parameter(name) => match names.get(name) {
                    Some(ComptimeId::Type(id)) => ResolvedType::Parameter(*id),
                    _ => return Err("unbound signature type parameter"),
                },
                Type::Array(t) => ResolvedType::Array(
                    t.as_ref()
                        .map(|t| resolve(t, names).map(Box::new))
                        .transpose()?,
                ),
                Type::Option(t) => ResolvedType::Option(Box::new(resolve(t, names)?)),
                Type::Result(a, b) => {
                    ResolvedType::Result(Box::new(resolve(a, names)?), Box::new(resolve(b, names)?))
                }
                ty => ResolvedType::Concrete(ty.clone()),
            })
        }
        Ok(Self {
            comptime: definition
                .comptime
                .iter()
                .map(|p| {
                    Ok(ResolvedComptimeParameter {
                        id: names[&p.name],
                        name: p.name.clone(),
                        ty: resolve(&p.ty, &names)?,
                    })
                })
                .collect::<Result<_, &'static str>>()?,
            parameters: definition
                .parameters
                .iter()
                .map(|p| resolve(&p.ty, &names))
                .collect::<Result<_, _>>()?,
            returns: resolve(&definition.returns, &names)?,
            children_type: definition
                .children_type
                .as_ref()
                .map(|ty| resolve(ty, &names))
                .transpose()?,
            type_parameters,
            declaration,
        })
    }
    pub fn function(&self) -> DefId {
        self.declaration().id.def_id()
    }
    pub fn declaration(&self) -> &FunctionDef {
        self.declaration.get()
    }
    pub fn comptime(&self) -> &[ResolvedComptimeParameter] {
        &self.comptime
    }
    pub fn parameters(&self) -> &[ResolvedType] {
        &self.parameters
    }
    pub fn returns(&self) -> &ResolvedType {
        &self.returns
    }
    pub fn children_type(&self) -> Option<&ResolvedType> {
        self.children_type.as_ref()
    }
    pub fn may_return_content(&self) -> bool {
        matches!(&self.returns, ResolvedType::Parameter(_))
            || matches!(&self.returns, ResolvedType::Concrete(ty) if ty.level().is_some())
    }
    /// Alpha equivalence relates declaration positions in two owned signatures,
    /// never equal spellings or equal process-local parameter numbers.
    pub fn alpha_equivalent_type(
        &self,
        left: &ResolvedType,
        other: &Self,
        right: &ResolvedType,
    ) -> Result<bool, SignatureError> {
        self.check_type(left)?;
        other.check_type(right)?;
        fn same(
            a: &ResolvedType,
            a_sig: &ResolvedSignature,
            b: &ResolvedType,
            b_sig: &ResolvedSignature,
        ) -> bool {
            match (a, b) {
                (ResolvedType::Concrete(a), ResolvedType::Concrete(b)) => a == b,
                (ResolvedType::Parameter(a), ResolvedType::Parameter(b)) => {
                    a_sig.type_parameters[a] == b_sig.type_parameters[b]
                }
                (ResolvedType::Array(None), ResolvedType::Array(None)) => true,
                (ResolvedType::Array(Some(a)), ResolvedType::Array(Some(b)))
                | (ResolvedType::Option(a), ResolvedType::Option(b)) => same(a, a_sig, b, b_sig),
                (ResolvedType::Result(a, b), ResolvedType::Result(c, d)) => {
                    same(a, a_sig, c, b_sig) && same(b, a_sig, d, b_sig)
                }
                _ => false,
            }
        }
        Ok(same(left, self, right, other))
    }
    /// Only named argument resolution and presentation use this projection.
    pub fn comptime_named(&self, name: &str) -> Option<&ResolvedComptimeParameter> {
        self.comptime.iter().find(|p| p.name == name)
    }
    pub fn type_parameter(&self, name: &str) -> Option<TypeParamId> {
        match self.comptime_named(name)?.id {
            ComptimeId::Type(id) => Some(id),
            _ => None,
        }
    }
    fn check_type(&self, ty: &ResolvedType) -> Result<(), SignatureError> {
        match ty {
            ResolvedType::Concrete(ty) if !ty.concrete() => {
                Err(SignatureError("named template in resolved type".into()))
            }
            ResolvedType::Parameter(id) if !self.type_parameters.contains_key(id) => Err(
                SignatureError("type parameter belongs to another signature".into()),
            ),
            ResolvedType::Array(Some(t)) | ResolvedType::Option(t) => self.check_type(t),
            ResolvedType::Result(a, b) => {
                self.check_type(a)?;
                self.check_type(b)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ComptimeBindings {
    signature: Arc<ResolvedSignature>,
    values: BTreeMap<ComptimeId, ComptimeValue>,
}
impl ComptimeBindings {
    pub fn new(signature: Arc<ResolvedSignature>) -> Self {
        Self {
            signature,
            values: BTreeMap::new(),
        }
    }
    pub fn signature(&self) -> &Arc<ResolvedSignature> {
        &self.signature
    }
    pub fn get(&self, id: ComptimeId) -> Option<&ComptimeValue> {
        self.values.get(&id)
    }
    pub fn insert(
        &mut self,
        id: ComptimeId,
        value: ComptimeValue,
    ) -> Result<Option<ComptimeValue>, SignatureError> {
        let parameter = self
            .signature
            .comptime
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                SignatureError("comptime parameter belongs to another signature".into())
            })?;
        if matches!(&value, ComptimeValue::Type(ty) if !ty.concrete()) {
            return Err(SignatureError(
                "comptime Type values must be concrete".into(),
            ));
        }
        if matches!(id, ComptimeId::Type(_))
            && !matches!(&value, ComptimeValue::Type(ty) if ty.concrete())
        {
            return Err(SignatureError(format!(
                "comptime `{}` requires a concrete Type",
                parameter.name
            )));
        }
        Ok(self.values.insert(id, value))
    }
    /// None is an explicit, valid partially inferred type. Invalid identities
    /// are errors, not an unchanged named type or an ignored inference request.
    pub fn concrete(&self, ty: &ResolvedType) -> Result<Option<Type>, SignatureError> {
        self.signature.check_type(ty)?;
        self.project(ty, false)
    }
    pub fn require(&self, ty: &ResolvedType) -> Result<Type, SignatureError> {
        self.concrete(ty)?
            .ok_or_else(|| SignatureError("unbound type parameter in concrete signature".into()))
    }
    /// Explicit boundary projection for a diagnostic's partial contract only.
    pub fn display_type(&self, ty: &ResolvedType) -> Result<Type, SignatureError> {
        self.signature.check_type(ty)?;
        self.project(ty, true)?
            .ok_or_else(|| SignatureError("invalid display type projection".into()))
    }
    fn project(&self, ty: &ResolvedType, display: bool) -> Result<Option<Type>, SignatureError> {
        Ok(Some(match ty {
            ResolvedType::Concrete(ty) => ty.clone(),
            ResolvedType::Parameter(id) => match self.get(ComptimeId::Type(*id)) {
                Some(ComptimeValue::Type(ty)) if ty.concrete() => ty.clone(),
                Some(_) => {
                    return Err(SignatureError(
                        "type parameter requires a concrete Type value".into(),
                    ));
                }
                None if display => Type::Parameter(
                    self.signature.comptime[self.signature.type_parameters[id]]
                        .name
                        .clone(),
                ),
                None => return Ok(None),
            },
            ResolvedType::Array(None) => Type::Array(None),
            ResolvedType::Array(Some(t)) => {
                let Some(t) = self.project(t, display)? else {
                    return Ok(None);
                };
                Type::Array(Some(Box::new(t)))
            }
            ResolvedType::Option(t) => {
                let Some(t) = self.project(t, display)? else {
                    return Ok(None);
                };
                Type::Option(Box::new(t))
            }
            ResolvedType::Result(a, b) => {
                let (Some(a), Some(b)) = (self.project(a, display)?, self.project(b, display)?)
                else {
                    return Ok(None);
                };
                Type::Result(Box::new(a), Box::new(b))
            }
        }))
    }
    pub fn instantiate(&self) -> Result<FunctionDef, SignatureError> {
        for parameter in &self.signature.comptime {
            if self.get(parameter.id).is_none() {
                return Err(SignatureError(format!(
                    "unbound comptime parameter `{}`",
                    parameter.name
                )));
            }
        }
        let mut concrete = self.signature.declaration().clone();
        concrete.returns = self.require(&self.signature.returns)?;
        concrete.children_type = self
            .signature
            .children_type
            .as_ref()
            .map(|t| self.require(t))
            .transpose()?;
        for (parameter, template) in concrete
            .parameters
            .iter_mut()
            .zip(&self.signature.parameters)
        {
            parameter.ty = self.require(template)?;
        }
        Ok(concrete)
    }
    pub fn infer(
        &mut self,
        pattern: &ResolvedType,
        actual: &Type,
    ) -> Result<Vec<String>, SignatureError> {
        self.signature.check_type(pattern)?;
        if !actual.concrete() {
            return Err(SignatureError(
                "inference requires a concrete actual type".into(),
            ));
        }
        let mut errors = Vec::new();
        self.infer_resolved(pattern, actual, &mut errors);
        Ok(errors)
    }
    fn infer_resolved(&mut self, pattern: &ResolvedType, actual: &Type, errors: &mut Vec<String>) {
        if *actual == Type::Never {
            return;
        }
        match (pattern, actual) {
            (ResolvedType::Parameter(id), ty) => {
                let value = ComptimeValue::Type(ty.clone());
                match self.values.get(&ComptimeId::Type(*id)) {
                    Some(previous) if previous != &value => errors.push(format!(
                        "conflicting inference for `{}`",
                        self.signature.comptime[self.signature.type_parameters[id]].name
                    )),
                    None => {
                        self.values.insert(ComptimeId::Type(*id), value);
                    }
                    _ => {}
                }
            }
            (ResolvedType::Array(Some(p)), Type::Array(Some(a)))
            | (ResolvedType::Option(p), Type::Option(a)) => self.infer_resolved(p, a, errors),
            (ResolvedType::Result(p, q), Type::Result(a, b)) => {
                self.infer_resolved(p, a, errors);
                self.infer_resolved(q, b, errors);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtins::Accepts,
        definitions::{ComptimeParameter, FunctionId, ValueConstraint},
    };
    fn signature(module: &str) -> Arc<ResolvedSignature> {
        let mut d = FunctionDef::new(
            FunctionId::new(module, "generic"),
            Accepts::Nothing,
            Type::Parameter("T".into()),
        );
        d.comptime.push(ComptimeParameter {
            name: "T".into(),
            ty: Type::Type,
            default: None,
            constraint: ValueConstraint::Any,
            span: Default::default(),
        });
        Arc::new(ResolvedSignature::new(&d).unwrap())
    }
    #[test]
    fn same_spelling_in_different_signatures_is_not_the_same_parameter() {
        let a = signature("types-a");
        let b = signature("types-b");
        assert_ne!(a.type_parameter("T"), b.type_parameter("T"));
        assert!(
            a.alpha_equivalent_type(a.returns(), &b, b.returns())
                .unwrap()
        );
        assert!(
            a.alpha_equivalent_type(b.returns(), &b, b.returns())
                .is_err()
        );
        let mut values = ComptimeBindings::new(a.clone());
        assert!(values.infer(&a.returns, &Type::Int).unwrap().is_empty());
        assert_eq!(values.require(&a.returns).unwrap(), Type::Int);
        assert!(values.infer(&b.returns, &Type::Int).is_err());
        assert!(
            values
                .insert(b.comptime[0].id, ComptimeValue::Type(Type::Int))
                .is_err()
        );
        assert!(values.concrete(&b.returns).is_err());
    }
    #[test]
    fn conflicts_and_never_evidence_preserve_inference_rules() {
        let signature = signature("types-conflict");
        let mut values = ComptimeBindings::new(signature.clone());
        assert!(
            values
                .infer(&signature.returns, &Type::Never)
                .unwrap()
                .is_empty()
        );
        assert!(values.get(signature.comptime[0].id).is_none());
        assert!(values.concrete(&signature.returns).unwrap().is_none());
        assert!(values.require(&signature.returns).is_err());
        assert!(
            values
                .infer(&signature.returns, &Type::String)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            values.infer(&signature.returns, &Type::Int).unwrap(),
            ["conflicting inference for `T`"]
        );
        assert_eq!(values.require(&signature.returns).unwrap(), Type::String);
    }
    #[test]
    fn structural_templates_need_no_named_side_table_and_never_silently_skip() {
        let signature = signature("types-structural");
        let template = ResolvedType::Option(Box::new(signature.returns.clone()));
        let mut values = ComptimeBindings::new(signature);
        assert!(
            values
                .infer(&template, &Type::Option(Box::new(Type::Int)))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            values.require(&template).unwrap(),
            Type::Option(Box::new(Type::Int))
        );
        let invalid = ResolvedType::Concrete(Type::Parameter("T".into()));
        assert!(values.infer(&invalid, &Type::Int).is_err());
        assert!(values.concrete(&invalid).is_err());
        assert!(
            values
                .infer(&template, &Type::Parameter("T".into()))
                .is_err()
        );
        assert!(
            values
                .insert(
                    values.signature().comptime()[0].id,
                    ComptimeValue::Type(Type::Parameter("T".into()))
                )
                .is_err()
        );
    }
    #[test]
    fn signature_keeps_its_authoritative_declaration_snapshot() {
        let signature = signature("types-owned-declaration");
        let mut declaration = signature.declaration().clone();
        declaration.returns = Type::Bool;
        let replacement = ResolvedSignature::new(&declaration).unwrap();
        assert_eq!(replacement.function(), signature.function());
        let mut values = ComptimeBindings::new(signature.clone());
        assert!(values.instantiate().is_err());
        values
            .insert(signature.comptime[0].id, ComptimeValue::Type(Type::Int))
            .unwrap();
        assert_eq!(values.instantiate().unwrap().returns, Type::Int);
        assert_eq!(replacement.returns(), &ResolvedType::Concrete(Type::Bool));
    }
    #[test]
    fn unresolved_type_defaults_are_not_hidden_named_references() {
        let source = signature("types-invalid-default");
        let mut declaration = source.declaration().clone();
        declaration.comptime[0].default = Some(ComptimeValue::Type(Type::Parameter("T".into())));
        assert!(ResolvedSignature::new(&declaration).is_err());
        let mut module = DefinitionModule::new("types-invalid-default");
        module.functions.push(declaration);
        assert!(
            crate::definitions::validate_module(&module)
                .iter()
                .any(|error| error.message.contains("invalid comptime default"))
        );
    }
    #[test]
    fn dependent_comptime_values_cannot_smuggle_named_type_references() {
        let original = signature("types-dependent-value");
        let mut declaration = original.declaration().clone();
        declaration.comptime.push(ComptimeParameter {
            name: "value".into(),
            ty: Type::Parameter("T".into()),
            default: None,
            constraint: ValueConstraint::Any,
            span: Default::default(),
        });
        let signature = Arc::new(ResolvedSignature::new(&declaration).unwrap());
        let mut bindings = ComptimeBindings::new(signature.clone());
        bindings
            .insert(signature.comptime()[0].id, ComptimeValue::Type(Type::Type))
            .unwrap();
        assert!(matches!(signature.comptime()[1].id, ComptimeId::Value(_)));
        assert!(
            bindings
                .insert(
                    signature.comptime()[1].id,
                    ComptimeValue::Type(Type::Parameter("T".into()))
                )
                .is_err()
        );
    }
}
