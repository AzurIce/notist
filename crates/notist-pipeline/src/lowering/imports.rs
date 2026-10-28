//! Shared lowering and validation of static file-level module declarations.
use notist_core::{
    definitions::{Import, ImportKind, ImportMember},
    diag::{Diagnostic, Phase},
};
use notist_syntax::{
    code::{ast::ImportDecl, lexer::lex_module},
    tree::SyntaxKind,
};

pub fn valid_import_alias(alias: &str) -> bool {
    notist_core::definitions::valid_name(alias)
        && lex_module(alias).kind(0) == Some(SyntaxKind::Ident)
        && !matches!(
            alias,
            "notist" | "self" | "true" | "false" | "some" | "none" | "compile_error" | "Array"
        )
}

pub fn lower_imports(
    declarations: impl IntoIterator<Item = ImportDecl>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Import> {
    declarations
        .into_iter()
        .filter_map(|decl| {
            let token = decl.source()?;
            let notist_core::item::Value::Str(path) =
                super::literals::syntax_value(&rowan::NodeOrToken::Token(token), diagnostics)?
            else {
                return None;
            };
            if path.is_empty() {
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    decl.range(),
                    "import requires a nonempty path",
                ));
                return None;
            }
            let kind = if decl.is_wildcard() {
                ImportKind::Wildcard
            } else if decl.has_members() {
                ImportKind::Members(
                    decl.members()
                        .filter_map(|member| {
                            let name = member.name()?.text().to_owned();
                            let alias = member
                                .alias()
                                .map(|t| t.text().to_owned())
                                .unwrap_or_else(|| name.clone());
                            if !valid_import_alias(&alias) {
                                diagnostics.push(Diagnostic::new(
                                    Phase::Type,
                                    member.range(),
                                    format!(
                                        "invalid or reserved import name `{alias}` from `{path}`"
                                    ),
                                ));
                                return None;
                            }
                            Some(ImportMember {
                                name,
                                alias,
                                span: member.range(),
                            })
                        })
                        .collect(),
                )
            } else {
                let alias = decl
                    .alias()
                    .map(|t| t.text().to_owned())
                    .unwrap_or_else(|| {
                        std::path::Path::new(&path)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default()
                            .to_owned()
                    });
                if !valid_import_alias(&alias) {
                    diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        decl.range(),
                        format!("invalid or reserved import name `{alias}` from `{path}`"),
                    ));
                    return None;
                }
                ImportKind::Module { alias }
            };
            Some(Import {
                path,
                kind,
                span: decl.range(),
            })
        })
        .collect()
}
