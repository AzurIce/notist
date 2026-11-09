use notist::{Context, Pipeline, Type};

#[test]
fn builtin_type_names_are_shared_without_broadening_default_syntax() {
    let pipeline = Pipeline::default();
    for name in [
        "Type",
        "Never",
        "Data",
        "DataError",
        "InlineContent",
        "BlockContent",
        "Unit",
        "Bool",
        "Int",
        "Float",
        "String",
        "Path",
        "ResourceTarget",
        "Dict",
    ] {
        assert!(Type::builtin(name).is_some(), "{name}");
        let module = pipeline
            .compile_module(
                "body.not",
                "type-names",
                "names.notc",
                &format!("fn same<T: Type = {name}>() -> Bool {{ T == {name} }}"),
            )
            .unwrap();
        let context = Context::default().with_module(module).unwrap();
        assert!(pipeline.check(&context).is_empty(), "{name}");
    }
    assert_eq!(Type::builtin("Array"), Some(Type::Array(None)));
    for name in ["Option", "Result", "Unknown", "integer"] {
        assert_eq!(Type::builtin(name), None);
    }
    for default in ["Array", "Array<Int>", "Option<Int>", "Result<Int, String>"] {
        assert!(
            pipeline
                .compile_module(
                    "body.not",
                    "type-default",
                    "default.notc",
                    &format!("fn value<T: Type = {default}>() -> String {{ \"x\" }}")
                )
                .is_err(),
            "default syntax must not expand: {default}"
        );
    }
    let module = pipeline
        .compile_module(
            "body.not",
            "array-name",
            "array.notc",
            "fn same(value: Array) -> Bool { Array == Array }",
        )
        .unwrap();
    assert!(
        pipeline
            .check(&Context::default().with_module(module).unwrap())
            .is_empty()
    );
}
