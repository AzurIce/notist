//! Output configuration with a private import table for each transform.
use crate::{Diagnostic, Import, Phase, TextRange};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct TransformBinding {
    pub imports: Vec<Import>,
    pub from: String,
    pub to: String,
    pub span: TextRange,
}
#[derive(Debug, Clone)]
pub struct Configuration {
    pub transforms: Vec<TransformBinding>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    transforms: Vec<toml::Spanned<Rule>>,
    #[serde(default)]
    site: Option<toml::Value>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    kind: RuleKind,
    #[serde(default)]
    imports: BTreeMap<String, toml::Spanned<String>>,
    from: String,
    to: String,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleKind {
    Replace,
}

pub fn parse_config(source: &str) -> Result<Configuration, Vec<Diagnostic>> {
    let config: Config = toml::from_str(source).map_err(|e| {
        vec![Diagnostic::new(
            Phase::Semantic,
            super::range(e.span().unwrap_or(0..0)),
            e.message(),
        )]
    })?;
    let _ = config.site;
    let mut diagnostics = Vec::new();
    let transforms = config
        .transforms
        .into_iter()
        .map(|r| {
            let span = super::range(r.span());
            let Rule {
                kind: RuleKind::Replace,
                imports,
                from,
                to,
            } = r.into_inner();
            let imports = imports
                .into_iter()
                .filter_map(|(alias, path)| {
                    let span = super::range(path.span());
                    if !notist_pipeline::lowering::imports::valid_import_alias(&alias) {
                        diagnostics.push(Diagnostic::new(
                            Phase::Semantic,
                            span,
                            "invalid transform import alias",
                        ));
                        return None;
                    }
                    Some(Import {
                        alias,
                        path: path.into_inner(),
                        span,
                    })
                })
                .collect();
            TransformBinding {
                imports,
                from,
                to,
                span,
            }
        })
        .collect();
    if diagnostics.is_empty() {
        Ok(Configuration { transforms })
    } else {
        Err(diagnostics)
    }
}
