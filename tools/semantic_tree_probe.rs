//! Full-tree canonical evidence. Diagnostic copies only; no timing conclusions.
use super::Document;
use notist::{Analysis, Diagnostic, Evaluation, Item, ResourcePath};
use serde::Serialize;
use std::{collections::BTreeSet, io::Write, path::Path};

fn frame<T: Serialize>(kind: &str, path: &Path, value: &T) {
    let stderr = std::io::stderr();
    let mut out = std::io::BufWriter::new(stderr.lock());
    out.write_all(b"[semantic-page] ").unwrap();
    serde_json::to_writer(&mut out, &(kind, path)).unwrap();
    out.write_all(b"\n[semantic-json] ").unwrap();
    serde_json::to_writer(&mut out, value).unwrap();
    out.write_all(b"\n").unwrap();
    out.flush().unwrap();
}

pub(super) fn evaluation(path: &Path, analysis: &Analysis, evaluation: &Evaluation) {
    if std::env::var_os("NOTIST_SEMANTIC_ONLY_OBSERVED").is_some() {
        return;
    }
    #[derive(Serialize)]
    struct Evidence<'a> {
        analysis_root: &'a Item,
        analysis_diagnostics: &'a [Diagnostic],
        analysis_sources: Vec<(&'a str, &'a str)>,
        evaluation_root: &'a Item,
        evaluation_diagnostics: &'a [Diagnostic],
        evaluation_sources: Vec<(&'a str, &'a str)>,
        evaluation_dependencies: &'a BTreeSet<ResourcePath>,
    }
    frame("evaluation", path, &Evidence {
        analysis_root: analysis.root(),
        analysis_diagnostics: analysis.diagnostics(),
        analysis_sources: analysis.sources().iter().map(|(k, v)| (k.as_str(), v.as_ref())).collect(),
        evaluation_root: &evaluation.root,
        evaluation_diagnostics: &evaluation.diagnostics,
        evaluation_sources: evaluation.sources.iter().map(|(k, v)| (k.as_str(), v.as_ref())).collect(),
        evaluation_dependencies: &evaluation.dependencies,
    });
}

pub(super) fn prepared(document: &Document) {
    if std::env::var_os("NOTIST_SEMANTIC_ONLY_OBSERVED").is_some() {
        return;
    }
    // Includes SSG heading IDs/anchors, attributes, route and the complete tree.
    frame("prepared", &document.page.route.path,
          &(&document.root, &document.page, &document.ids));
}
