use crate::{
    builtins::Accepts,
    definitions::{
        ComptimeParameter, FunctionDef, FunctionId, FunctionImpl, NativeFunction, ParameterDef,
        ParameterMode, Type, ValueConstraint,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataFunction {
    ReadJson,
    ReadToml,
    ParseJson,
    ParseToml,
    Decode,
    Require,
    Get,
    RequireIndex,
    Index,
    Has,
    Keys,
    Len,
    Kind,
    ErrorKind,
    ErrorMessage,
    ErrorPath,
    Fail,
    Expect,
}
impl DataFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::ReadJson => "read_json",
            Self::ReadToml => "read_toml",
            Self::ParseJson => "parse_json",
            Self::ParseToml => "parse_toml",
            Self::Decode => "data_decode",
            Self::Require => "data_require",
            Self::Get => "data_get",
            Self::RequireIndex => "data_require_index",
            Self::Index => "data_index",
            Self::Has => "data_has",
            Self::Keys => "data_keys",
            Self::Len => "data_len",
            Self::Kind => "data_kind",
            Self::ErrorKind => "data_error_kind",
            Self::ErrorMessage => "data_error_message",
            Self::ErrorPath => "data_error_path",
            Self::Fail => "data_fail",
            Self::Expect => "data_expect",
        }
    }
    pub fn reads_resources(self) -> bool {
        matches!(self, Self::ReadJson | Self::ReadToml)
    }
    pub fn decodes(self) -> bool {
        matches!(
            self,
            Self::Decode | Self::Require | Self::Get | Self::RequireIndex | Self::Index
        )
    }
}
pub fn definitions() -> Vec<FunctionDef> {
    use DataFunction::*;
    let target = Type::Parameter("T".into());
    let result = |t| Type::Result(Box::new(t), Box::new(Type::DataError));
    let mut functions = Vec::new();
    for kind in [
        ReadJson,
        ReadToml,
        ParseJson,
        ParseToml,
        Decode,
        Require,
        Get,
        RequireIndex,
        Index,
        Has,
        Keys,
        Len,
        Kind,
        ErrorKind,
        ErrorMessage,
        ErrorPath,
        Fail,
        Expect,
    ] {
        let returns = match kind {
            ReadJson | ReadToml | ParseJson | ParseToml => result(Type::Data),
            Decode | Require | RequireIndex => result(target.clone()),
            Get | Index => result(Type::Option(Box::new(target.clone()))),
            Has => result(Type::Bool),
            Keys => result(Type::Array(Some(Box::new(Type::String)))),
            Len => result(Type::Int),
            Kind | ErrorKind | ErrorMessage | ErrorPath => Type::String,
            Fail => Type::Never,
            Expect => target.clone(),
        };
        let mut def = FunctionDef::new(
            FunctionId::new("notist", kind.name()),
            Accepts::Nothing,
            returns,
        );
        def.implementation = FunctionImpl::Native(NativeFunction::Data(kind));
        if kind.decodes() || kind == Expect {
            def.comptime.push(ComptimeParameter {
                name: "T".into(),
                ty: Type::Type,
                default: None,
                constraint: ValueConstraint::Any,
                span: def.span,
            });
        }
        let args = match kind {
            ReadJson | ReadToml => vec![("source", Type::Path)],
            ParseJson | ParseToml => vec![("text", Type::String)],
            Decode | Kind => vec![("data", Type::Data)],
            Require | Get | Has => vec![("object", Type::Data), ("key", Type::String)],
            RequireIndex | Index => vec![("array", Type::Data), ("index", Type::Int)],
            Keys => vec![("object", Type::Data)],
            Len => vec![("array", Type::Data)],
            ErrorKind | ErrorMessage | ErrorPath | Fail => vec![("error", Type::DataError)],
            Expect => vec![("value", result(target.clone()))],
        };
        def.parameters = args
            .into_iter()
            .map(|(n, t)| ParameterDef::new(n, t, ParameterMode::Required))
            .collect();
        functions.push(def);
    }
    functions
}
