//! Shared lowering and validation of static file-level module declarations.
use notist_core::{
    definitions::Import,
    diag::{Diagnostic, Phase},
};
use notist_syntax::{
    code::{ast::ImportDecl, lexer::lex_module},
    tree::SyntaxKind,
};
use std::collections::BTreeSet;

pub fn valid_import_alias(alias: &str) -> bool {
    notist_core::definitions::valid_name(alias)
        && lex_module(alias).kind(0) == Some(SyntaxKind::Ident)
        && !matches!(alias, "notist" | "self" | "true" | "false")
}

pub fn lower_imports(
    declarations: impl IntoIterator<Item = ImportDecl>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Import> {
    let mut aliases = BTreeSet::new();
    declarations
        .into_iter()
        .filter_map(|decl| {
            let token = decl.source()?;
            let notist_core::item::Value::Str(path) =
                super::literals::syntax_value(&rowan::NodeOrToken::Token(token), diagnostics)?
            else {
                return None;
            };
            let alias = decl
                .alias()
                .map(|t| t.text().to_string())
                .unwrap_or_else(|| {
                    std::path::Path::new(&path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_owned()
                });
            if path.is_empty() || !valid_import_alias(&alias) || !aliases.insert(alias.clone()) {
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    decl.range(),
                    "import requires a nonempty path and a valid, unique module alias",
                ));
                return None;
            }
            Some(Import {
                path,
                alias,
                span: decl.range(),
            })
        })
        .collect()
}
