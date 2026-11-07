//! Structured-data implementations use the same native call/result machinery.
use crate::EvaluationContext;
use crate::eval::*;
use notist_core::data::{Data, DataError, DataErrorKind, DataFailure, DataFormat, DataFunction};

impl<C: EvaluationContext + ?Sized> Evaluator<'_, C> {
    pub(super) fn data_native(
        &mut self,
        kind: DataFunction,
        call: &Invocation,
        fields: &Dict,
    ) -> Result<Runtime, Failure> {
        use DataFunction::*;
        let mut limits = self.limits.data;
        limits.value_bytes = limits.value_bytes.min(self.limits.value_bytes);
        let target = || match call.instance.arguments.first() {
            Some(ComptimeValue::Type(t)) => Ok(t),
            _ => Err(Failure::at("data operation requires a target Type", call)),
        };
        let view = |name| match fields.get(name) {
            Some(Value::Data(data)) => Ok(data),
            _ => Err(Failure::at("data operation requires Data", call)),
        };
        let error = || match fields.get("error") {
            Some(Value::DataError(error)) => Ok(error),
            _ => Err(Failure::at("data operation requires DataError", call)),
        };
        let key = || match fields.get("key") {
            Some(Value::Str(key)) => Ok(key.as_str()),
            _ => Err(Failure::at("data key must be String", call)),
        };
        let index = || match fields.get("index") {
            Some(Value::Int(index)) => Ok(*index),
            _ => Err(Failure::at("data index must be Int", call)),
        };
        if kind.decodes() && !notist_core::data::decode_target(target()?) {
            return data_result(
                Err(DataError::new(
                    DataErrorKind::UnsupportedTarget,
                    "unsupported Data decode target",
                )
                .into()),
                call,
            );
        }
        let decode =
            |data: &Data, t: &Type| match self.inputs.and_then(EvaluationInputs::data_cache) {
                Some(cache) => cache.decode(data, t, limits),
                None => data.decode(t, limits),
            };
        let result = match kind {
            ReadJson | ReadToml => return self.read_data(kind, call, fields),
            ParseJson | ParseToml => {
                let Some(Value::Str(text)) = fields.get("text") else {
                    return Err(Failure::at("data parsing requires String", call));
                };
                limits.working_bytes = self.data_retention.remaining(limits.working_bytes);
                notist_core::data::parse(
                    if kind == ParseJson {
                        DataFormat::Json
                    } else {
                        DataFormat::Toml
                    },
                    text.as_bytes(),
                    limits,
                )
                .map(Value::Data)
            }
            Decode => decode(view("data")?, target()?),
            Require => {
                let t = target()?;
                view("object")?.require(key()?).and_then(|d| decode(&d, t))
            }
            Get => {
                let t = target()?;
                view("object")?.get(key()?).and_then(|d| match d {
                    None => Ok(Value::None),
                    Some(d) => decode(&d, t).map(|v| Value::Some(Box::new(v))),
                })
            }
            RequireIndex => {
                let t = target()?;
                view("array")?
                    .require_index(index()?)
                    .and_then(|d| decode(&d, t))
            }
            Index => {
                let t = target()?;
                view("array")?.index(index()?).and_then(|d| match d {
                    None => Ok(Value::None),
                    Some(d) => decode(&d, t).map(|v| Value::Some(Box::new(v))),
                })
            }
            Has => view("object")?
                .get(key()?)
                .map(|v| Value::Bool(v.is_some())),
            Keys => view("object")?.keys().map(Value::Array),
            Len => view("array")?.len().map(|n| Value::Int(n as i64)),
            Kind => return Ok(data_runtime(Value::Str(view("data")?.kind().into()))),
            ErrorKind => return Ok(data_runtime(Value::Str(error()?.kind().as_str().into()))),
            ErrorMessage => return Ok(data_runtime(Value::Str(error()?.message().into()))),
            ErrorPath => return Ok(data_runtime(Value::Str(error()?.path().into()))),
            Fail => return Err(self.fail_data(error()?, call)),
            Expect => {
                return match fields.get("value") {
                    Some(Value::Ok(value)) => Ok(data_runtime((**value).clone())),
                    Some(Value::Err(error)) => match error.as_ref() {
                        Value::DataError(error) => Err(self.fail_data(error, call)),
                        _ => Err(Failure::at("data_expect requires DataError", call)),
                    },
                    _ => Err(Failure::at("data_expect requires Result", call)),
                };
            }
        };
        self.retain_data_result(&result, call)?;
        data_result(result, call)
    }
    fn read_data(
        &mut self,
        kind: DataFunction,
        call: &Invocation,
        fields: &Dict,
    ) -> Result<Runtime, Failure> {
        if self.mode == EvaluationMode::Comptime {
            return Err(Failure::at(
                "resource reading is not permitted during comptime evaluation",
                call,
            ));
        }
        if call.site != CallSite::Code {
            return Err(Failure::at(
                format!("{} can only be called from notc Code", kind.name()),
                call,
            ));
        }
        self.requests = self.requests.saturating_add(1);
        if self.requests > self.limits.read_requests {
            return Err(Failure::at("resource request limit exceeded", call));
        }
        let Some(Value::Path(path)) = fields.get("source") else {
            return Err(Failure::at("data reading requires a Path", call));
        };
        self.dependencies.insert(path.clone());
        let bytes = match self
            .inputs
            .map(|i| i.file(path))
            .unwrap_or(InputRead::Failed(InputError::Unavailable))
        {
            InputRead::Ready(bytes) => bytes,
            InputRead::Pending => {
                self.needed.insert(path.clone());
                return Err(Failure::at("resource input is not prepared", call));
            }
            InputRead::Failed(error) => {
                let kind = match error {
                    InputError::NotFound => DataErrorKind::NotFound,
                    InputError::AccessDenied => DataErrorKind::AccessDenied,
                    InputError::NotFile => DataErrorKind::NotFile,
                    InputError::Io(_) => DataErrorKind::Io,
                    _ => return Err(Failure::at(format!("{}: {error}", kind.name()), call)),
                };
                return data_result(Err(DataError::new(kind, error.to_string()).into()), call);
            }
        };
        let format = if kind == DataFunction::ReadJson {
            DataFormat::Json
        } else {
            DataFormat::Toml
        };
        let mut limits = self.limits.data;
        limits.value_bytes = limits.value_bytes.min(self.limits.value_bytes);
        let parsed = match self.inputs.and_then(EvaluationInputs::data_cache) {
            Some(cache) => cache.parse(format, bytes, limits),
            None => {
                limits.working_bytes = self.data_retention.remaining(limits.working_bytes);
                notist_core::data::parse(format, &bytes, limits)
            }
        };
        let location = SourceLocation {
            source: call.source,
            start: call.span.start().into(),
            end: call.span.end().into(),
        };
        let parsed = match parsed {
            Ok(data) => Ok(Value::Data(data.bind_source(path.logical_path(), location))),
            Err(DataFailure::Error(error)) => {
                Err(error.bind_source(path.logical_path(), location).into())
            }
            Err(limit) => Err(limit),
        };
        self.retain_data_result(&parsed, call)?;
        data_result(parsed, call)
    }
    fn retain_data_result(
        &mut self,
        result: &Result<Value, DataFailure>,
        call: &Invocation,
    ) -> Result<(), Failure> {
        match result {
            Ok(value) => self
                .data_retention
                .observe(value, self.limits.data.working_bytes),
            Err(DataFailure::Error(error)) => self.data_retention.observe(
                &Value::DataError(error.clone()),
                self.limits.data.working_bytes,
            ),
            Err(DataFailure::Limit(_)) => Ok(()),
        }
        .map_err(|e| Failure::at(e.to_string(), call))
    }
    fn fail_data(&mut self, error: &DataError, call: &Invocation) -> Failure {
        let mut failure = Failure::at(error.to_string(), call);
        if let Some((name, text, span, read)) = error.diagnostic_source() {
            self.sources.insert(name.into(), text);
            failure.location = Some(SourceLocation {
                source: name.into(),
                start: span.start as u32,
                end: span.end as u32,
            });
            failure.expansion.push(read.clone());
        }
        failure
    }
}
fn data_runtime(value: Value) -> Runtime {
    Runtime::Value(ComptimeValue::Data(value))
}
fn data_result(value: Result<Value, DataFailure>, call: &Invocation) -> Result<Runtime, Failure> {
    match value {
        Ok(value) => Ok(data_runtime(Value::Ok(Box::new(value)))),
        Err(DataFailure::Error(error)) => {
            Ok(data_runtime(Value::Err(Box::new(Value::DataError(error)))))
        }
        Err(DataFailure::Limit(message)) => Err(Failure::at(message, call)),
    }
}
