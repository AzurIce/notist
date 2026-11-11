//! Bounded ownership sharing for output descriptors, not implementations or results.
use notist_core::{
    definitions::{ComptimeValue, DefId, ResolvedInstance, Type},
    item::Value,
    shared::Shared,
};
use std::{borrow::Cow, collections::BTreeMap};

const MAX_ENTRIES: usize = 256;
const MAX_SPECIALIZATIONS: usize = 16;
const MAX_ARGUMENTS: usize = 16;
const MAX_TYPE_NODES: usize = 64;

/// One evaluator owns this table alongside its Sources/Definitions snapshot.
/// Only the selected, post-mapping constructor target enters it. Sharing never
/// bypasses argument evaluation, specialization, diagnostics or resource limits.
#[derive(Default)]
pub(super) struct DescriptorCache {
    functions: BTreeMap<DefId, Vec<Shared<ResolvedInstance>>>,
    entries: usize,
}
impl DescriptorCache {
    pub(super) fn share(
        &mut self,
        instance: Cow<'_, ResolvedInstance>,
    ) -> Shared<ResolvedInstance> {
        if !reusable(&instance) {
            return instance.into_owned().into();
        }
        if let Some(entries) = self.functions.get(&instance.function) {
            if let Some(existing) = entries
                .iter()
                .find(|entry| same_arguments(&entry.arguments, &instance.arguments))
            {
                return existing.clone();
            }
            if entries.len() >= MAX_SPECIALIZATIONS {
                return instance.into_owned().into();
            }
        }
        if self.entries >= MAX_ENTRIES {
            return instance.into_owned().into();
        }
        let instance: Shared<ResolvedInstance> = instance.into_owned().into();
        self.functions
            .entry(instance.function)
            .or_default()
            .push(instance.clone());
        self.entries += 1;
        instance
    }
}

fn reusable(instance: &ResolvedInstance) -> bool {
    // Bound retained signature size and comparison work as well as entry count.
    // Variable-size data and unresolved type parameters keep the original path.
    fn bounded_type(ty: &Type, remaining: &mut usize) -> bool {
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        match ty {
            Type::Array(Some(ty)) | Type::Option(ty) => bounded_type(ty, remaining),
            Type::Result(ok, err) => bounded_type(ok, remaining) && bounded_type(err, remaining),
            Type::Parameter(_) => false,
            _ => true,
        }
    }
    let mut remaining = MAX_TYPE_NODES;
    instance.arguments.len() <= MAX_ARGUMENTS
        && instance.arguments.iter().all(|argument| match argument {
            ComptimeValue::Type(ty) => bounded_type(ty, &mut remaining),
            ComptimeValue::Data(
                Value::Unit | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::None,
            ) => true,
            _ => false,
        })
}

fn same_arguments(left: &[ComptimeValue], right: &[ComptimeValue]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            match (left, right) {
                (ComptimeValue::Type(left), ComptimeValue::Type(right)) => left == right,
                (ComptimeValue::Data(left), ComptimeValue::Data(right)) => match (left, right) {
                    (Value::Unit, Value::Unit) | (Value::None, Value::None) => true,
                    (Value::Bool(left), Value::Bool(right)) => left == right,
                    (Value::Int(left), Value::Int(right)) => left == right,
                    // Storage equivalence is not language equality: retain signed
                    // zero and NaN payloads, without making NaN equal to itself.
                    (Value::Float(left), Value::Float(right)) => left.to_bits() == right.to_bits(),
                    _ => false,
                },
                _ => false,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notist_core::{definition_identity::Definitions, definitions::FunctionId};

    fn instance(
        names: &mut Definitions,
        name: &str,
        arguments: Vec<ComptimeValue>,
    ) -> ResolvedInstance {
        ResolvedInstance {
            function: names.insert(FunctionId::new("descriptor-cache", name)),
            arguments,
        }
    }

    #[test]
    fn exact_targets_and_type_arguments_share_but_other_evaluations_do_not() {
        let mut names = Definitions::default();
        let mut cache = DescriptorCache::default();
        for arguments in [
            vec![],
            vec![ComptimeValue::Type(Type::String)],
            vec![ComptimeValue::Type(Type::Result(
                Box::new(Type::Array(Some(Box::new(Type::Int)))),
                Box::new(Type::Option(Box::new(Type::Bool))),
            ))],
            vec![ComptimeValue::Data(Value::Int(2))],
        ] {
            let target = instance(&mut names, "view", arguments);
            let first = cache.share(Cow::Borrowed(&target));
            let again = cache.share(Cow::Owned(target.clone()));
            assert!(first.shares_storage(&again));
            assert_eq!(*first, target);
            let other = instance(&mut names, "other", target.arguments.clone());
            assert!(!first.shares_storage(&cache.share(Cow::Borrowed(&other))));
            let independent = DescriptorCache::default().share(Cow::Borrowed(&target));
            assert!(!first.shares_storage(&independent));
        }
        assert_eq!(cache.entries, 8);
    }

    #[test]
    fn scalar_storage_equivalence_preserves_float_bits_and_value_equality() {
        let mut names = Definitions::default();
        let mut cache = DescriptorCache::default();
        let values = [
            Value::Unit,
            Value::None,
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(0),
            Value::Int(1),
            Value::Float(0.0),
            Value::Float(-0.0),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
            Value::Float(f64::from_bits(0x7ff8_0000_0000_0043)),
        ];
        let mut descriptors: Vec<Shared<ResolvedInstance>> = Vec::new();
        for value in values {
            let target = instance(
                &mut names,
                "scalar",
                vec![ComptimeValue::Data(value.clone())],
            );
            let descriptor = cache.share(Cow::Borrowed(&target));
            let again = cache.share(Cow::Borrowed(&target));
            assert!(descriptor.shares_storage(&again));
            assert!(
                descriptors
                    .iter()
                    .all(|old| !old.shares_storage(&descriptor))
            );
            assert_eq!(descriptor == again, target == target);
            if let Value::Float(expected) = value {
                let ComptimeValue::Data(Value::Float(actual)) = &descriptor.arguments[0] else {
                    panic!("float descriptor")
                };
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            descriptors.push(descriptor);
        }
        let reordered = instance(
            &mut names,
            "scalar",
            vec![
                ComptimeValue::Data(Value::Int(1)),
                ComptimeValue::Type(Type::Int),
            ],
        );
        let mut other = reordered.clone();
        other.arguments.reverse();
        assert!(
            !cache
                .share(Cow::Borrowed(&reordered))
                .shares_storage(&cache.share(Cow::Borrowed(&other)))
        );
    }

    #[test]
    fn unsupported_or_large_signatures_keep_the_uncached_path() {
        let mut names = Definitions::default();
        let mut cache = DescriptorCache::default();
        let mut deep = Type::Int;
        for _ in 0..MAX_TYPE_NODES {
            deep = Type::Option(Box::new(deep));
        }
        let arguments = [
            vec![ComptimeValue::Type(Type::Parameter("T".into()))],
            vec![ComptimeValue::Type(deep)],
            vec![ComptimeValue::Type(Type::Int); MAX_ARGUMENTS + 1],
            vec![ComptimeValue::Data(Value::Str("large data".into()))],
            vec![ComptimeValue::Data(Value::Path(
                notist_core::resource::ResourcePath::resolve("/", "/doc.not", "asset").unwrap(),
            ))],
            vec![ComptimeValue::Data(Value::Array(
                vec![Value::Float(-0.0)].into(),
            ))],
            vec![ComptimeValue::Data(Value::Dict(Default::default()))],
            vec![ComptimeValue::Data(Value::Some(Box::new(Value::Float(
                f64::NAN,
            ))))],
            vec![ComptimeValue::Data(Value::Ok(Box::new(Value::Int(1))))],
            vec![ComptimeValue::Data(Value::Err(Box::new(Value::Int(1))))],
        ];
        for arguments in arguments {
            let target = instance(&mut names, "fallback", arguments);
            let first = cache.share(Cow::Borrowed(&target));
            let second = cache.share(Cow::Borrowed(&target));
            assert!(!first.shares_storage(&second));
            assert_eq!(first.arguments[0].key(), target.arguments[0].key());
        }
        assert_eq!(cache.entries, 0);
        assert!(cache.functions.is_empty());
    }

    #[test]
    fn full_tables_fall_back_without_evicting_existing_descriptors() {
        let mut names = Definitions::default();
        let mut cache = DescriptorCache::default();
        for n in 0..MAX_ENTRIES {
            cache.share(Cow::Owned(instance(&mut names, &format!("f{n}"), vec![])));
        }
        let retained = instance(&mut names, "f0", vec![]);
        let first = cache.share(Cow::Borrowed(&retained));
        let again = cache.share(Cow::Borrowed(&retained));
        assert!(first.shares_storage(&again));
        let overflow = instance(&mut names, "overflow", vec![]);
        let first = cache.share(Cow::Borrowed(&overflow));
        let again = cache.share(Cow::Borrowed(&overflow));
        assert_eq!(first, again);
        assert!(!first.shares_storage(&again));
        assert_eq!(cache.entries, MAX_ENTRIES);
        assert_eq!(cache.functions.len(), MAX_ENTRIES);

        let mut cache = DescriptorCache::default();
        for n in 0..MAX_SPECIALIZATIONS {
            cache.share(Cow::Owned(instance(
                &mut names,
                "view",
                vec![ComptimeValue::Data(Value::Int(n as i64))],
            )));
        }
        let overflow = instance(
            &mut names,
            "view",
            vec![ComptimeValue::Data(Value::Int(-1))],
        );
        assert!(
            !cache
                .share(Cow::Borrowed(&overflow))
                .shares_storage(&cache.share(Cow::Borrowed(&overflow)))
        );
        assert_eq!(cache.entries, MAX_SPECIALIZATIONS);
        let different_function = instance(&mut names, "other", vec![]);
        assert!(
            cache
                .share(Cow::Borrowed(&different_function))
                .shares_storage(&cache.share(Cow::Borrowed(&different_function)))
        );
    }

    #[test]
    fn editing_a_shared_descriptor_detaches_from_the_cache_and_other_outputs() {
        let mut names = Definitions::default();
        let mut cache = DescriptorCache::default();
        let target = instance(&mut names, "view", vec![ComptimeValue::Type(Type::Int)]);
        let original = cache.share(Cow::Borrowed(&target));
        let mut edited = cache.share(Cow::Borrowed(&target));
        edited.arguments[0] = ComptimeValue::Type(Type::Float);
        assert!(!edited.shares_storage(&original));
        assert_eq!(*original, target);
        assert!(
            cache
                .share(Cow::Borrowed(&target))
                .shares_storage(&original)
        );
    }
}
