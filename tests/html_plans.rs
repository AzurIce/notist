use notist::{Ctor, Item, MemoryResources, ResourcePath, Sources, Value, Vault};
use notist_html::{HtmlRegistry, Renderer};
use std::collections::{BTreeMap, HashMap};

fn keys(root: &Item) -> HashMap<*const Item, usize> {
    notist::embed::references(root)
        .enumerate()
        .map(|(i, node)| (node as *const Item, i))
        .collect()
}

fn compare(root: &Item, sources: &Sources, urls: Vec<Option<String>>) {
    let keys = keys(root);
    let expected = Renderer::default()
        .with_sources(sources)
        .with_url_resolver(|node, _| urls[keys[&(node as *const Item)]].clone())
        .with_resource_resolver(|node, _| urls[keys[&(node as *const Item)]].clone())
        .render_with_diagnostics(root);
    let actual = Renderer::default()
        .with_sources(sources)
        .prepare(root, &BTreeMap::new(), |node| keys[&(node as *const Item)])
        .unwrap()
        .finish(&HtmlRegistry::default(), &urls, |_, _| None);
    assert_eq!(actual.html, expected.html);
    assert_eq!(actual.diagnostics, expected.diagnostics);
    assert!(actual.used_components.is_empty());
}

#[test]
fn prepared_urls_match_immediate_safety_suffixes_labels_and_diagnostics() {
    let resource = ResourcePath::resolve("/site", "/site/page.not", "a?b#c.svg").unwrap();
    for ctor in [Ctor::Link, Ctor::Embed] {
        for target in [
            Value::Str("safe?q=x#part".into()),
            Value::Str("javascript:alert(1)".into()),
            Value::Path(resource.clone()),
        ] {
            for resolved in [
                None,
                Some(""),
                Some("../a%3Fb.svg"),
                Some("bad?query=yes"),
                Some("javascript:bad"),
            ] {
                for (query, fragment) in [
                    (None, None),
                    (Some("v=1"), Some("a b%")),
                    (Some("bad#query"), None),
                ] {
                    let mut root = Item::new(ctor.clone(), Default::default())
                        .with_field("target", target.clone());
                    if let Some(query) = query {
                        root = root.with_field("query", Value::Str(query.into()));
                    }
                    if let Some(fragment) = fragment {
                        root = root.with_field("fragment", Value::Str(fragment.into()));
                    }
                    root.attrs.insert("title", Value::Str("<&\"".into()));
                    compare(
                        &root,
                        &Default::default(),
                        vec![resolved.map(str::to_owned)],
                    );
                }
            }
        }
    }
}

#[test]
fn component_selection_is_deferred_and_keeps_nested_fallback_diagnostic_order() {
    for available in [false, true] {
        let mut files = MemoryResources::new("/site");
        files.insert("view.notc", "fn panel<T: Type>(value: T)[children: BlockContent] -> BlockContent; fn leaf(label: String) -> InlineContent;");
        if available {
            files.insert("panel.js", "export default class extends HTMLElement {}");
            files.insert(
                "leaf/index.js",
                "export default class extends HTMLElement {}",
            );
        }
        let mut vault = Vault::new(files);
        let text = "#import \"view.notc\" as view;\n@(class: \"outer\")\n#view.panel<String>(\"<&\")[\n#view.leaf(\"nested\") [link](other.not)\n]";
        let (analysis, handle) = vault
            .analyze_document_with_resource_root("page.not", text, "/site")
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let output = vault.evaluate_document(&handle, &analysis).unwrap();
        let bindings = handle.html_bindings(&output.root);
        let candidates = bindings.candidates();
        let keys = keys(&output.root);
        let plan = Renderer::default()
            .with_sources(&output.sources)
            .prepare(&output.root, &candidates, |node| {
                keys[&(node as *const Item)]
            })
            .unwrap();
        let urls = vec![Some("other.html".into())];
        let registry = vault.resolve_html_bindings(&bindings).unwrap();
        let expected = Renderer::from_registry(registry.clone())
            .with_sources(&output.sources)
            .with_url_resolver(|_, _| urls[0].clone())
            .render_with_diagnostics(&output.root);
        // Finalization cannot accidentally fall back to the original IR/sources.
        drop(output);
        drop(analysis);
        drop(handle);
        drop(vault);
        let actual = plan.finish(&registry, &urls, |_, _| None);
        assert_eq!(actual.html, expected.html);
        assert_eq!(actual.diagnostics, expected.diagnostics);
        assert_eq!(
            actual
                .used_components
                .iter()
                .map(|c| &c.id)
                .collect::<Vec<_>>(),
            expected
                .used_components
                .iter()
                .map(|c| &c.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(actual.used_components.len(), if available { 2 } else { 0 });
    }
}

#[test]
fn preparation_shares_native_layout_and_reports_only_visible_diagnostics() {
    let pipeline = notist::Pipeline::default();
    let context = notist::Context::new(notist::builtins::registry().clone());
    for text in [
        "= Heading\n\nText *strong* $x < y$ and `code`.\n\n- One\n- Two\n\n---",
        "| Name | Value |\n| :--- | ---: |\n| [a](a.not) | **b** |",
        "@(class: \"styled\")\n#table[ #table-row[ #table-cell[one] #table-cell[two] ] ]",
        "#unknown[hidden [link](hidden.not)]\n\nVisible [link](visible.not)",
    ] {
        let analysis = pipeline.analyze("page.not", text, &context).unwrap();
        let count = keys(analysis.root()).len();
        compare(analysis.root(), analysis.sources(), vec![None; count]);
    }
}

#[test]
fn distinct_occurrences_do_not_collapse_to_their_shared_origin_or_target() {
    let link =
        Item::new(Ctor::Link, Default::default()).with_field("target", Value::Str("same".into()));
    let root = Item::new(Ctor::Doc, Default::default()).with_children(vec![link.clone(), link]);
    let keys = keys(&root);
    let result = Renderer::default()
        .prepare(&root, &BTreeMap::new(), |n| keys[&(n as *const Item)])
        .unwrap()
        .finish(
            &HtmlRegistry::default(),
            &[Some("first".into()), Some("second".into())],
            |_, _| None,
        );
    assert_eq!(
        result.html,
        "<a href=\"first\">same</a><a href=\"second\">same</a>"
    );
}

#[test]
fn source_map_or_immediate_resolvers_cannot_be_silently_ignored() {
    let root = Item::new(Ctor::Doc, Default::default());
    for renderer in [
        Renderer::default().with_source_map(),
        Renderer::default().with_url_resolver(|_, _| None),
        Renderer::default().with_embed_renderer(|_| None),
    ] {
        assert!(renderer.prepare(&root, &BTreeMap::new(), |_| 0).is_err());
    }
}

#[test]
fn deferred_bindings_cannot_cross_vault_or_configuration_boundaries() {
    let mut a = Vault::new(MemoryResources::new("/site"));
    let b = Vault::new(MemoryResources::new("/site"));
    let (analysis, handle) = a
        .analyze_document_with_resource_root("a.not", "text", "/site")
        .unwrap();
    let output = a.evaluate_document(&handle, &analysis).unwrap();
    let bindings = handle.html_bindings(&output.root);
    assert!(a.resolve_html_bindings(&bindings).is_ok());
    assert!(b.resolve_html_bindings(&bindings).is_err());
}

#[test]
fn missing_source_locations_remain_explicit_in_deferred_url_diagnostics() {
    let source = notist::Source::new("missing.not", "test source").unwrap();
    let mut root = Item::new(Ctor::Link, Default::default())
        .with_field("target", Value::Str("javascript:bad".into()));
    root.origin = Some(notist::Origin {
        source: source.id(),
        start: 0,
        end: 4,
        expansion: Default::default(),
    });
    compare(&root, &Default::default(), vec![None]);
}
