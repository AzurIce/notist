use notist_core::{
    definitions::FunctionId,
    item::{Ctor, Value},
};
use notist_pipeline::{Context, Pipeline, analyze_module, transforms::Replace};
use rowan::TextRange;

fn context(declaration: &str) -> Context {
    Context::default()
        .with_module(analyze_module("demo", declaration).unwrap())
        .unwrap()
}

#[test]
fn one_context_drives_analysis_and_transforms_without_transforming_analysis() {
    let context = context("fn math<C: Type = InlineContent>(text: String) -> C;")
        .with_transforms(&[Replace {
            from: FunctionId::new("notist", "math"),
            to: FunctionId::new("demo", "math"),
            span: TextRange::empty(0.into()),
        }])
        .unwrap();
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let analysis = pipeline.analyze(path, "$x$", &context).unwrap();
        let (inspected, _) = pipeline.inspect(path, "$x$", &context).unwrap();
        assert_eq!(analysis, inspected);
        assert!(analysis.diagnostics().is_empty());
        let original = analysis.root().clone();
        assert!(original.find(|node| node.ctor == Ctor::Math).is_some());
        let transformed = pipeline.evaluate(&analysis, &context);
        assert!(transformed.diagnostics.is_empty());
        let math = transformed
            .root
            .find(|node| {
                node.target
                    .as_ref()
                    .is_some_and(|i| i.function == FunctionId::new("demo", "math"))
            })
            .unwrap();
        assert_eq!(math.fields.get("text"), Some(&Value::Str("x".into())));
        assert_eq!(*analysis.root(), original);
    }
}

#[test]
fn one_pipeline_uses_each_explicit_context_for_names_and_shape() {
    let pipeline = Pipeline::default();
    let inline = context("fn token() -> InlineContent;");
    let block = context("fn token() -> BlockContent;");
    let source = "before #demo::token() after";
    for path in ["doc.not", "doc.md"] {
        let first = pipeline.analyze(path, source, &inline).unwrap();
        let second = pipeline.analyze(path, source, &block).unwrap();
        let missing = pipeline.analyze(path, source, &Context::default()).unwrap();
        assert!(first.diagnostics().is_empty());
        assert!(
            second
                .diagnostics()
                .iter()
                .any(|d| d.message.contains("separate blocks with a blank line"))
        );
        assert_eq!(first.root().children.len(), 1);
        assert_eq!(second.root().children.len(), 1);
        assert!(
            missing
                .diagnostics()
                .iter()
                .any(|d| { d.message == "unknown constructor or function `demo::token`" })
        );
    }
}
