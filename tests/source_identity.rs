use notist::{Context, Pipeline, SourceId};

#[test]
fn same_identity_keeps_old_and_new_source_snapshots_separate() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    let old = pipeline
        .analyze("/project/main.not", "old text", &context)
        .unwrap();
    let new = pipeline
        .analyze("/project/main.not", "new text", &context)
        .unwrap();
    let id = SourceId::intern("/project/main.not");
    assert_eq!(old.root().origin.as_ref().unwrap().source, id);
    assert_eq!(new.root().origin.as_ref().unwrap().source, id);
    let old_output = pipeline.evaluate(&old, &context);
    let new_output = pipeline.evaluate(&new, &context);
    drop(old);
    drop(new);
    assert_eq!(old_output.sources[&id].as_ref(), "old text");
    assert_eq!(new_output.sources[&id].as_ref(), "new text");
    assert_eq!(old_output.sources[id.as_str()].as_ref(), "old text");
}

#[test]
fn source_maps_keep_lexical_order_not_registration_order() {
    let z = SourceId::intern("/order/z.not");
    let a = SourceId::intern("/order/a.not");
    let sources = notist::Sources::from([(z, "z".into()), (a, "a".into())]);
    assert_eq!(
        sources.keys().map(|id| id.as_str()).collect::<Vec<_>>(),
        ["/order/a.not", "/order/z.not"]
    );
    assert!(sources.contains_key("/order/a.not"));
}

#[cfg(feature = "serde")]
#[test]
fn tree_and_diagnostic_wire_keep_names_after_source_owner_is_dropped() {
    let analysis = Pipeline::default()
        .analyze("/project/wire.not", "hello", &Context::default())
        .unwrap();
    let (root, _) = analysis.into_parts();
    let wire = serde_json::to_value(&root).unwrap();
    assert_eq!(wire["origin"]["source"], "/project/wire.not");
    let restored: notist::Node = serde_json::from_value(wire).unwrap();
    assert_eq!(restored, root);
    let mut diagnostic = notist::Diagnostic::new(notist::Phase::Type, root.span, "test");
    diagnostic.source = Some(root.origin.as_ref().unwrap().source);
    let wire = serde_json::to_value(&diagnostic).unwrap();
    assert_eq!(wire["source"], "/project/wire.not");
    assert_eq!(
        serde_json::from_value::<notist::Diagnostic>(wire).unwrap(),
        diagnostic
    );
}
