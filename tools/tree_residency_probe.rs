//! Diagnostic snapshot helper, never linked into the ordinary timing binary.
use super::Document;
use std::collections::{BTreeMap, HashSet};

pub(super) fn observe(documents: &[Document]) {
    let mut nodes = 0usize;
    let mut child_capacity = 0usize;
    let mut field_uses = BTreeMap::<usize, usize>::new();
    let mut attr_uses = BTreeMap::<usize, usize>::new();
    let mut unique_dicts = BTreeMap::<usize, usize>::new();
    let mut seen = HashSet::new();
    let mut entries_capacity = 0usize;
    let mut index_entries = 0usize;
    let mut storage_bytes = 0usize;
    let mut dict_capacities = BTreeMap::<String, usize>::new();
    let mut dict_keys = BTreeMap::<String, usize>::new();
    let mut instances = HashSet::new();
    let mut instance_uses = 0usize;
    let mut instance_string_capacity = 0usize;
    let mut instance_argument_capacity_bytes = 0usize;
    let mut field_texts = HashSet::new();
    let mut field_text_bytes = 0usize;
    for document in documents {
        for node in document.root.descendants() {
            nodes += 1;
            child_capacity += node.as_item().map_or(0, |item| item.children.capacity());
            instance_uses += usize::from(node.instance().is_some());
            if let Some(instance) = node.instance() {
                let id = instance as *const notist::FunctionInstance as usize;
                if instances.insert(id) {
                    instance_string_capacity += instance.function.module.capacity() + instance.function.name.capacity();
                    instance_argument_capacity_bytes += instance.arguments.capacity() * std::mem::size_of::<notist::ComptimeValue>();
                }
            }
            let fields = node.fields();
            for (dict, uses) in [(&fields, &mut field_uses), (&node.attrs, &mut attr_uses)] {
                let (id, len, capacity, index_len, bytes) = dict.residency_probe();
                *uses.entry(len).or_default() += 1;
                if seen.insert(id) {
                    *dict_capacities.entry(format!("{len}/{capacity}")).or_default() += 1;
                    *unique_dicts.entry(len).or_default() += 1;
                    entries_capacity += capacity;
                    index_entries += index_len;
                    storage_bytes += bytes;
                    for (key, value) in dict.iter() {
                        *dict_keys.entry(key.to_owned()).or_default() += 1;
                        if let notist::Value::Str(text) = value {
                            let key = (text.as_str().as_ptr() as usize, text.as_str().len());
                            if field_texts.insert(key) {
                                field_text_bytes += key.1;
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("[tree-residency] {}", serde_json::json!({
        "documents": documents.len(), "nodes": nodes,
        "item_size": std::mem::size_of::<notist::Item>(),
        "child_capacity": child_capacity,
        "child_item_capacity_bytes": child_capacity * std::mem::size_of::<notist::Item>(),
        "field_uses_by_length": field_uses,
        "attr_uses_by_length": attr_uses,
        "unique_dicts_by_length": unique_dicts,
        "dict_storage_bytes_excluding_allocator_headers": storage_bytes,
        "dict_entries_capacity_bytes": entries_capacity * std::mem::size_of::<(notist::Text, notist::Value)>(),
        "dict_index_entries": index_entries,
        "unique_dicts_by_length_and_capacity": dict_capacities,
        "dict_key_occurrences_by_unique_dict": dict_keys,
        "instance_uses": instance_uses,
        "unique_instances": instances.len(),
        "unique_instance_value_bytes": instances.len() * std::mem::size_of::<notist::FunctionInstance>(),
        "instance_string_capacity": instance_string_capacity,
        "instance_argument_capacity_bytes": instance_argument_capacity_bytes,
        "unique_direct_field_texts": field_texts.len(),
        "direct_field_text_payload_bytes": field_text_bytes,
        "note": "Snapshot counts and Rust capacities, not allocator-exact bytes or primary RSS. Observer HashSet adds its own memory."
    }));
}
