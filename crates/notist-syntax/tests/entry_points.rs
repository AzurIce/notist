use notist_syntax::code::{ast, parse_module};
use notist_syntax::markup::parse_document;

#[test]
fn document_and_module_use_distinct_roots_and_dispatch() {
    let src = "fn foo() -> BlockContent;";
    let document = parse_document(src);
    let code = parse_module(src, &notist_syntax::markup::MarkupContentParser);
    assert!(notist_syntax::markup::ast::Document::cast(document.syntax()).is_some());
    assert!(ast::Module::cast(document.syntax()).is_none());
    assert!(ast::Module::cast(code.syntax()).is_some());
    assert!(notist_syntax::markup::ast::Document::cast(code.syntax()).is_none());
}
