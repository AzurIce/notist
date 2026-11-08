//! Small, evaluation-local accelerator for already checked implementations.
//! Data comptime values retain CheckedProgram's bit-exact canonical-key path.
use notist_core::{
    definitions::{ComptimeValue, DefId, ResolvedInstance as FunctionInstance, Type},
    evaluation::InstanceRecord,
};
use std::{collections::BTreeMap, sync::Arc};

const MAX_ENTRIES: usize = 256;
const MAX_SPECIALIZATIONS: usize = 16;

struct Entry {
    arguments: Vec<Type>,
    record: Arc<InstanceRecord>,
}
impl Entry {
    fn matches(&self, instance: &FunctionInstance) -> bool {
        self.arguments.len() == instance.arguments.len()
            && self.arguments.iter().zip(&instance.arguments).all(
                |(expected, actual)| matches!(actual, ComptimeValue::Type(ty) if expected == ty),
            )
    }
}

/// This table belongs to one Evaluator and its immutable Context. It cannot
/// reuse records across replacements, input batches or separate evaluations.
/// Entry limits bound cache growth, not language semantics: a miss or full
/// table always falls back to the authoritative CheckedProgram.
#[derive(Default)]
pub(super) struct InstanceCache {
    functions: BTreeMap<DefId, Vec<Entry>>,
    entries: usize,
}
impl InstanceCache {
    pub(super) fn get(&self, instance: &FunctionInstance) -> Option<&Arc<InstanceRecord>> {
        self.functions
            .get(&instance.function)?
            .iter()
            .find(|entry| entry.matches(instance))
            .map(|entry| &entry.record)
    }

    pub(super) fn insert(&mut self, instance: &FunctionInstance, record: &Arc<InstanceRecord>) {
        if self.entries >= MAX_ENTRIES
            || record.checking
            || !record.diagnostics.is_empty()
            || !instance
                .arguments
                .iter()
                .all(|value| matches!(value, ComptimeValue::Type(_)))
            || self
                .functions
                .get(&instance.function)
                .is_some_and(|entries| entries.len() >= MAX_SPECIALIZATIONS)
        {
            return;
        }
        let entries = self.functions.entry(instance.function).or_default();
        if entries.iter().any(|entry| entry.matches(instance)) {
            return;
        }
        let arguments = instance
            .arguments
            .iter()
            .map(|value| match value {
                ComptimeValue::Type(ty) => ty.clone(),
                _ => unreachable!("only Type arguments enter this cache"),
            })
            .collect();
        entries.push(Entry {
            arguments,
            record: record.clone(),
        });
        self.entries += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notist_core::{
        builtins::Accepts,
        definitions::{FunctionDef, FunctionId},
    };
    // In production these names belong to Evaluator.sources. Standalone cache
    // tests retain an equivalent fixture snapshot until the test thread exits.
    thread_local! { static NAMES: std::cell::RefCell<notist_core::definition_identity::Definitions> = Default::default(); }

    fn instance(name: &str, arguments: Vec<ComptimeValue>) -> FunctionInstance {
        FunctionInstance {
            function: NAMES.with(|names| names.borrow_mut().insert(FunctionId::new("demo", name))),
            arguments,
        }
    }
    fn record() -> Arc<InstanceRecord> {
        Arc::new(InstanceRecord {
            inputs: Default::default(),
            definition: FunctionDef::new(
                FunctionId::new("demo", "view"),
                Accepts::Nothing,
                Type::String,
            ),
            body: None,
            diagnostics: Vec::new(),
            checking: false,
        })
    }

    #[test]
    fn successful_records_are_shared_by_exact_function_and_type_arguments() {
        let mut cache = InstanceCache::default();
        let key = instance("view", vec![ComptimeValue::Type(Type::String)]);
        let record = record();
        cache.insert(&key, &record);
        assert!(Arc::ptr_eq(cache.get(&key).unwrap(), &record));
        assert!(
            cache
                .get(&instance("view", vec![ComptimeValue::Type(Type::Int)]))
                .is_none()
        );
        assert!(
            cache
                .get(&instance("other", key.arguments.clone()))
                .is_none()
        );
        let mut other_module = key.clone();
        other_module.function = NAMES.with(|names| {
            names
                .borrow_mut()
                .insert(FunctionId::new("replacement", "view"))
        });
        assert!(cache.get(&other_module).is_none());
        cache.insert(&key, &record);
        assert_eq!(cache.entries, 1);
        assert!(InstanceCache::default().get(&key).is_none());
    }

    #[test]
    fn data_arguments_and_incomplete_records_keep_the_authoritative_path() {
        let mut cache = InstanceCache::default();
        let record = record();
        let reusable = instance("view", vec![ComptimeValue::Type(Type::Float)]);
        cache.insert(&reusable, &record);
        for bits in [0, (-0.0_f64).to_bits(), 0x7ff8_0000_0000_0042] {
            let key = instance(
                "view",
                vec![ComptimeValue::Data(notist_core::item::Value::Float(
                    f64::from_bits(bits),
                ))],
            );
            cache.insert(&key, &record);
            assert!(cache.get(&key).is_none());
        }
        for value in [
            notist_core::item::Value::Int(1),
            notist_core::item::Value::Str("x".into()),
        ] {
            for arguments in [
                vec![ComptimeValue::Data(value.clone())],
                vec![ComptimeValue::Type(Type::Int), ComptimeValue::Data(value)],
            ] {
                let key = instance("view", arguments);
                cache.insert(&key, &record);
                assert!(cache.get(&key).is_none());
            }
        }
        let key = instance("view", vec![]);
        let mut provisional = record.clone();
        Arc::make_mut(&mut provisional).checking = true;
        cache.insert(&key, &provisional);
        let mut failed = record.clone();
        Arc::make_mut(&mut failed)
            .diagnostics
            .push(notist_core::diag::Diagnostic::new(
                notist_core::diag::Phase::Type,
                Default::default(),
                "not reusable",
            ));
        cache.insert(&key, &failed);
        assert_eq!(cache.entries, 1);
        assert!(cache.get(&key).is_none());
        assert!(Arc::ptr_eq(cache.get(&reusable).unwrap(), &record));
    }

    #[test]
    fn cache_limits_only_make_lookup_fall_back() {
        let mut cache = InstanceCache::default();
        let record = record();
        for i in 0..MAX_ENTRIES + 1 {
            cache.insert(&instance(&format!("f{i}"), vec![]), &record);
        }
        assert_eq!(cache.entries, MAX_ENTRIES);
        assert!(cache.get(&instance("f0", vec![])).is_some());
        assert!(
            cache
                .get(&instance(&format!("f{MAX_ENTRIES}"), vec![]))
                .is_none()
        );
        let mut cache = InstanceCache::default();
        let mut ty = Type::Int;
        for _ in 0..MAX_SPECIALIZATIONS + 1 {
            cache.insert(
                &instance("view", vec![ComptimeValue::Type(ty.clone())]),
                &record,
            );
            ty = Type::Array(Some(Box::new(ty)));
        }
        assert_eq!(cache.entries, MAX_SPECIALIZATIONS);
    }
}
