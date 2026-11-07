use notist::{Context, Ctor, Item, Pipeline, Source, SourceNamespace, Sources};

#[test]
fn source_snapshots_and_escaped_diagnostics_keep_the_consumed_revision() {
    let pipeline = Pipeline::default();
    let context = Context::default();
    let old = pipeline
        .analyze("edit.not", "#missing[old]", &context)
        .unwrap();
    let new = pipeline.analyze("edit.not", "new", &context).unwrap();
    assert_eq!(old.source().id(), new.source().id());
    let diagnostic = old.diagnostics()[0].clone();
    let (tree, _, sources) = old.into_parts();
    drop(context);
    assert!(sources.validate_node(&tree).is_ok());
    assert_eq!(
        sources.text(new.source().id()).unwrap().as_ref(),
        "#missing[old]"
    );
    let presented = notist::SourceDiagnostic::locate(
        std::path::Path::new("wrong.not"),
        "wrong text",
        diagnostic,
    );
    assert_eq!(presented.source, "#missing[old]");
    assert_eq!(presented.path, std::path::Path::new("edit.not"));
    let mut combined = sources.clone();
    assert!(combined.merge(new.sources()).is_err());
    assert_eq!(combined, sources);
}

#[test]
fn contextual_wire_preserves_names_and_rejects_missing_or_ambiguous_owners() {
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("wire.not", "hello", &Context::default())
        .unwrap();
    let wire = serde_json::to_value(analysis.sources().wire(analysis.root())).unwrap();
    assert_eq!(wire["origin"]["source"], "wire.not");
    assert!(serde_json::to_value(Sources::new().wire(analysis.root())).is_err());
    assert!(Sources::new().decode::<Item>(wire.clone()).is_err());
    let imported = Sources::import::<Item>(wire.clone(), SourceNamespace::named("remote")).unwrap();
    assert!(imported.sources.validate_node(&imported.value).is_ok());
    let origin = imported.origin.as_ref().unwrap();
    assert!(imported.sources.text(origin.source).is_err());
    assert_eq!(
        imported
            .sources
            .source(origin.source)
            .unwrap()
            .namespace()
            .name(),
        "remote"
    );
    assert_eq!(
        serde_json::to_value(imported.sources.wire(&imported.value)).unwrap(),
        wire
    );
    let mut ambiguous = analysis.sources().clone();
    ambiguous
        .insert(
            Source::in_namespace(
                SourceNamespace::named("other"),
                "wire.not",
                Some("other".into()),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(serde_json::to_value(ambiguous.wire(analysis.root())).is_err());
    assert!(ambiguous.decode::<Item>(wire).is_err());
    assert!(Source::metadata("").is_err());
}

#[test]
fn source_maps_pin_owners_and_missing_sources_cannot_escape_as_valid_maps() {
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("map.not", "hello", &Context::default())
        .unwrap();
    let result = notist_html::Renderer::new()
        .with_sources(analysis.sources())
        .with_source_map()
        .render_with_diagnostics(analysis.root());
    let id = analysis.source().id();
    drop(analysis);
    assert!(!result.source_map.is_empty());
    assert_eq!(result.sources.text(id).unwrap().as_ref(), "hello");
    let mut node = Item::new(Ctor::Paragraph, Default::default());
    node.origin = Some(notist::Origin {
        source: id,
        start: 0,
        end: 5,
        expansion: Default::default(),
    });
    let rejected = notist_html::Renderer::new()
        .with_source_map()
        .render_with_diagnostics(&node);
    assert_eq!(rejected.diagnostics.len(), 1);
    assert!(rejected.source_map.is_empty());
}

#[test]
fn cached_specializations_do_not_capture_the_first_callers_revision() {
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module(
            "doc.not",
            "view",
            "view.notc",
            "fn show() -> InlineContent { [shared] }",
        )
        .unwrap();
    let context = Context::default().with_module(module).unwrap();
    let old = pipeline
        .analyze("same.not", "#self.show()", &context)
        .unwrap();
    let new = pipeline
        .analyze("same.not", "prefix #self.show()", &context)
        .unwrap();
    for analysis in [&old, &new] {
        let result = pipeline.evaluate(analysis, &context);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result
                .sources
                .validate_program(analysis.checked_program())
                .is_ok()
        );
        assert_eq!(
            result.sources.text(analysis.source().id()).unwrap(),
            analysis.source().text().unwrap()
        );
    }
}

#[test]
fn extensions_cannot_publish_locations_outside_their_snapshot() {
    #[derive(Debug)]
    struct Foreign(Source);
    impl notist::extensions::DocumentExtension for Foreign {
        fn name(&self) -> &str {
            "foreign-source"
        }
        fn after_evaluate(
            &self,
            _: &notist::extensions::DocumentContext<'_>,
            document: &mut Item,
        ) -> notist::extensions::ExtensionResult {
            document.origin.as_mut().unwrap().source = self.0.id();
            Ok(())
        }
    }
    let context = Context::default()
        .with_extension(Foreign(Source::new("foreign.not", "other").unwrap()))
        .unwrap();
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("extension.not", "hello", &context)
        .unwrap();
    let result = pipeline.evaluate(&analysis, &context);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.root.origin, analysis.root().origin);
    assert!(result.sources.validate_node(&result.root).is_ok());
}
