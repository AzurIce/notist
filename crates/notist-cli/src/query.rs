//! Output envelope for the query command.
use notist::{Ctor, Item};

pub fn render_json(sources: &notist::Sources, matches: &[&Item]) -> String {
    let results = matches
        .iter()
        .map(|item| {
            let mut data = notist::json::item(item);
            let name = item.ctor.name();
            data["ctor"] = serde_json::json!(if matches!(item.ctor, Ctor::Extension(_)) {
                name.into_owned()
            } else {
                name.to_lowercase()
            });
            let range = usize::from(item.span.start())..usize::from(item.span.end());
            let source = item.origin.as_ref().and_then(|o| sources.get(&o.source));
            data["text"] = serde_json::json!(source.and_then(|s| s.get(range)).unwrap_or(""));
            data
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&results).expect("valid query output")
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_output_retains_source_and_metadata() {
        let source = "@(id: \"x\", tags: (\"a\",))#emph[目标]\n\n*粗*\n";
        let (item, diagnostics) = notist::Pipeline::default()
            .analyze("test.not", source, &notist::Context::default())
            .unwrap()
            .into_parts();
        assert!(diagnostics.is_empty());
        let matches = notist::query::select(&item, "id:x");
        let data: serde_json::Value = serde_json::from_str(&super::render_json(
            &std::collections::BTreeMap::from([("test.not".into(), source.into())]),
            &matches,
        ))
        .unwrap();
        assert_eq!(data[0]["ctor"], "emph");
        assert_eq!(data[0]["attrs"]["tags"], serde_json::json!(["a"]));
        assert!(data[0]["text"].as_str().unwrap().contains("目标"));
    }
    #[test]
    fn output_serialization_accepts_deep_ir_without_reparsing_json() {
        let mut item = notist::Item::new(notist::Ctor::Text, Default::default());
        for _ in 0..160 {
            item =
                notist::Item::new(notist::Ctor::Node, Default::default()).with_children(vec![item]);
        }
        let output = super::render_json(&std::collections::BTreeMap::new(), &[&item]);
        assert_eq!(output.matches("\"children\":").count(), 161);
        assert!(output.contains("\"ctor\":\"node\""));
    }
}
