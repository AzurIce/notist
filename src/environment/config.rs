//! Document extension configuration. Each replacement has a private import table.
use crate::{Diagnostic, Import, Phase, TextRange};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStage {
    BeforeCheck,
    AfterCheck,
    AfterEvaluate,
}

#[derive(Debug, Clone)]
pub enum ExtensionBinding {
    Replace {
        imports: Vec<Import>,
        from: String,
        to: String,
        span: TextRange,
    },
    Command {
        command: Vec<String>,
        stages: Vec<DocumentStage>,
        span: TextRange,
    },
}
impl ExtensionBinding {
    pub fn imports(&self) -> &[Import] {
        match self {
            Self::Replace { imports, .. } => imports,
            Self::Command { .. } => &[],
        }
    }
}
#[derive(Debug, Clone)]
pub struct Configuration {
    pub extensions: Vec<ExtensionBinding>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    extensions: Vec<toml::Spanned<Extension>>,
    #[serde(default)]
    book: Option<toml::Value>,
    #[serde(default)]
    ssg: Option<toml::Value>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Extension {
    kind: ExtensionKind,
    imports: Option<BTreeMap<String, toml::Spanned<String>>>,
    from: Option<String>,
    to: Option<String>,
    command: Option<Vec<String>>,
    stages: Option<Vec<DocumentStage>>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExtensionKind {
    Replace,
    Command,
}

pub fn parse_config(source: &str) -> Result<Configuration, Vec<Diagnostic>> {
    let config: Config = toml::from_str(source).map_err(|e| {
        vec![Diagnostic::new(
            Phase::Semantic,
            super::range(e.span().unwrap_or(0..0)),
            e.message(),
        )]
    })?;
    let _ = (config.book, config.ssg);
    let mut diagnostics = Vec::new();
    let extensions = config
        .extensions
        .into_iter()
        .map(|r| {
            let span = super::range(r.span());
            let extension = r.into_inner();
            match extension.kind {
                ExtensionKind::Replace => {
                    if extension.command.is_some() || extension.stages.is_some() {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "replace extension does not accept command or stages",
                        ));
                    }
                    if extension.from.is_none() || extension.to.is_none() {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "replace extension requires from and to",
                        ));
                    }
                    let from = extension.from.unwrap_or_default();
                    let to = extension.to.unwrap_or_default();
                    let imports = extension
                        .imports
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|(alias, path)| {
                            let span = super::range(path.span());
                            if !notist_pipeline::lowering::imports::valid_import_alias(&alias) {
                                diagnostics.push(Diagnostic::new(
                                    Phase::Semantic,
                                    span,
                                    "invalid extension import alias",
                                ));
                                return None;
                            }
                            Some(Import {
                                kind: crate::ImportKind::Module { alias },
                                path: path.into_inner(),
                                span,
                            })
                        })
                        .collect();
                    ExtensionBinding::Replace {
                        imports,
                        from,
                        to,
                        span,
                    }
                }
                ExtensionKind::Command => {
                    if extension.imports.is_some()
                        || extension.from.is_some()
                        || extension.to.is_some()
                    {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "command extension does not accept imports, from or to",
                        ));
                    }
                    let command = extension.command.unwrap_or_default();
                    let stages = extension.stages.unwrap_or_default();
                    if command.first().is_none_or(|program| program.is_empty()) {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "extension command requires a nonempty program",
                        ));
                    }
                    if stages.is_empty()
                        || stages
                            .iter()
                            .enumerate()
                            .any(|(i, stage)| stages[..i].contains(stage))
                    {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "extension stages must be nonempty and unique",
                        ));
                    }
                    ExtensionBinding::Command {
                        command,
                        stages,
                        span,
                    }
                }
            }
        })
        .collect();
    if diagnostics.is_empty() {
        Ok(Configuration { extensions })
    } else {
        Err(diagnostics)
    }
}
