use super::*;
use notist_core::evaluation::{ArgumentTarget, NativeParameter};
use notist_core::shared::DictBuilder;
use smallvec::SmallVec;

/// Owned call-local values; omission is represented by absence, not Value::None.
pub(super) struct ArgumentValues<'a> {
    call: &'a Invocation,
    values: SmallVec<[(&'a ArgumentTarget, Runtime); 2]>,
}
impl<'a> ArgumentValues<'a> {
    pub(super) fn evaluate<C: EvaluationContext + ?Sized>(
        e: &mut Evaluator<'_, C>,
        call: &'a Invocation,
        scope: &Scope,
    ) -> Result<Self, Failure> {
        let mut values = SmallVec::new();
        for argument in call.checked_arguments() {
            let expression = &argument.expression;
            let parameter = match argument.target {
                ArgumentTarget::Parameter(index) => Some(&call.definition().parameters[index]),
                ArgumentTarget::Extra(_) => None,
            };
            let value = e.expr(expression, scope)?;
            let value = if parameter.is_some_and(|p| p.mode == ParameterMode::Optional)
                && matches!(expression.ty, Some(Type::Option(_)))
            {
                match value {
                    Runtime::Value(ComptimeValue::Data(Value::Some(value))) => {
                        Runtime::Value(ComptimeValue::Data(*value))
                    }
                    Runtime::Value(ComptimeValue::Data(Value::None)) => continue,
                    _ => {
                        return Err(format!(
                            "invalid option for `{}.{}`",
                            call.definition().id,
                            call.argument_name(&argument.target)
                                .expect("invocation-bound target")
                        )
                        .into());
                    }
                }
            } else {
                value
            };
            if let Some(parameter) = parameter
                && let Runtime::Value(ComptimeValue::Data(value)) = &value
                && !parameter.accepts(value)
            {
                return Err(format!(
                    "invalid value for `{}.{}`",
                    call.definition().id,
                    parameter.name
                )
                .into());
            }
            values.push((&argument.target, value));
        }
        Ok(Self { call, values })
    }
    pub(super) fn native(&self, record: &InstanceRecord, role: NativeParameter) -> Option<&Value> {
        let index = record.native_parameter(role)?;
        self.values
            .iter()
            .rev()
            .find_map(|(target, value)| match (target, value) {
                (ArgumentTarget::Parameter(i), Runtime::Value(ComptimeValue::Data(value)))
                    if *i == index =>
                {
                    Some(value)
                }
                _ => None,
            })
    }
    pub(super) fn into_fields(self) -> DictBuilder {
        let mut fields = DictBuilder::default();
        for (target, value) in self.values {
            if let Runtime::Value(ComptimeValue::Data(value)) = value {
                fields.insert(
                    self.call
                        .argument_name(target)
                        .expect("invocation-bound target")
                        .clone(),
                    value,
                );
            }
        }
        fields
    }
    fn take_native(&mut self, record: &InstanceRecord, role: NativeParameter) -> Option<Value> {
        let index = record.native_parameter(role)?;
        let position = self.values.iter().rposition(|(target, value)|
            matches!((target, value), (ArgumentTarget::Parameter(i), Runtime::Value(ComptimeValue::Data(_))) if *i == index))?;
        let (_, Runtime::Value(ComptimeValue::Data(value))) = self.values.remove(position) else {
            unreachable!()
        };
        Some(value)
    }
    pub(super) fn into_scope(
        self,
        record: &InstanceRecord,
        children: &Content,
    ) -> Result<Scope, Failure> {
        let mut inputs = Scope::new();
        for (target, value) in self.values {
            let ArgumentTarget::Parameter(index) = *target else {
                return Err("missing checked parameter binding".into());
            };
            let id = record
                .inputs
                .parameters
                .get(index)
                .copied()
                .flatten()
                .ok_or("missing checked parameter binding")?;
            let value = if self.call.definition().parameters[index].mode == ParameterMode::Optional
            {
                match value {
                    Runtime::Value(ComptimeValue::Data(v)) => {
                        Runtime::Value(ComptimeValue::Data(Value::Some(Box::new(v))))
                    }
                    other => other,
                }
            } else {
                value
            };
            inputs.insert(id, value);
        }
        for (index, parameter) in self.call.definition().parameters.iter().enumerate() {
            let id = record
                .inputs
                .parameters
                .get(index)
                .copied()
                .flatten()
                .ok_or("missing checked parameter binding")?;
            if parameter.mode == ParameterMode::Optional && !inputs.contains_key(&id) {
                inputs.insert(id, Runtime::Value(ComptimeValue::Data(Value::None)));
            }
        }
        if let Some(ty) = &self.call.definition().children_type {
            let id = record
                .inputs
                .children
                .ok_or("missing checked children binding")?;
            inputs.insert(id, Runtime::Content(ty.clone(), children.clone()));
        }
        Ok(inputs)
    }
}

pub(super) struct NativeArguments<'a, 'call> {
    pub(super) values: &'a mut ArgumentValues<'call>,
    pub(super) record: &'a InstanceRecord,
}
impl NativeArguments<'_, '_> {
    pub(super) fn get(&self, role: NativeParameter) -> Option<&Value> {
        self.values.native(self.record, role)
    }
    pub(super) fn take(&mut self, role: NativeParameter) -> Option<Value> {
        self.values.take_native(self.record, role)
    }
}
