//! The ownership boundary between evaluated IR and whole-site finalization.
use crate::{
    Error, Page, Route,
    document::{Document, diagnostic},
};
use notist::{
    Diagnostic, DocumentHandle, Evaluation, Item, Phase, ResourcePath, SourceDiagnostic, Value,
};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

pub(crate) enum Target {
    Resource(ResourcePath),
    Url(String),
    None,
}

pub(crate) struct Reference {
    pub target: Target,
    pub embed: bool,
    pub origin: Result<PathBuf, String>,
    pub fragment: Option<String>,
    pub description: String,
    location: Diagnostic,
    source: notist::Source,
}

impl Reference {
    pub fn issue(&self, message: String) -> SourceDiagnostic {
        let mut d = self.location.clone();
        d.message.insert_str(0, &message);
        diagnostic(
            Path::new(self.source.name()),
            self.source.text().map(|s| s.as_ref()).unwrap_or(""),
            d,
        )
    }

    fn new(document: &Document, node: &Item, source: &notist::Source) -> Self {
        let span = node.origin.as_ref().map_or(node.span, |o| {
            notist::TextRange::new(o.start.into(), o.end.into())
        });
        let mut d = Diagnostic::new(Phase::Semantic, span, String::new());
        if let Some(origin) = &node.origin {
            d = d.at_source(&document.sources, origin.source);
        }
        Self {
            target: match node.field("target") {
                Some(Value::Path(path)) => Target::Resource(path.clone()),
                Some(Value::Str(url)) => Target::Url(url.to_string()),
                _ => Target::None,
            },
            embed: notist::embed::is_embed(node),
            origin: node
                .origin
                .as_ref()
                .map(|o| document.sources.name(o.source).map(PathBuf::from))
                .transpose()
                .map(|p| p.unwrap_or_else(|| document.page.route.source.clone()))
                .map_err(|e| e.to_string()),
            fragment: notist::embed::string(&node.fields(), "fragment").map(str::to_owned),
            description: notist::embed::string(&node.fields(), "description")
                .unwrap_or("")
                .to_owned(),
            location: d,
            source: source.clone(),
        }
    }
}

pub(crate) struct PreparedPage {
    pub page: Page,
    pub ids: BTreeSet<String>,
    pub references: Vec<Reference>,
    pub html: notist_html::HtmlPlan,
    pub bindings: notist::vault::HtmlBindings,
    pub source: notist::Source,
}

impl PreparedPage {
    /// Consumes the only evaluated tree. Returned values cannot retain IR or a
    /// Sources aggregate; source-bearing diagnostics keep only their own text.
    pub fn new(
        route: Route,
        text: &str,
        source: notist::Source,
        evaluation: Evaluation,
        handle: &DocumentHandle,
        site_title: &str,
    ) -> Result<(Self, Vec<SourceDiagnostic>), Error> {
        let errors = evaluation
            .diagnostics
            .into_iter()
            .map(|d| diagnostic(&route.source, text, d))
            .collect();
        let mut document = Document::new(route, evaluation.root, text, evaluation.sources)?;
        if document.page.route.url == "/"
            && document.page.headings.is_empty()
            && !document.page.attrs.contains_key("title")
        {
            document.page.title = site_title.into();
        }
        let mut keys = HashMap::new();
        let references = notist::embed::references(&document.root)
            .enumerate()
            .map(|(i, node)| {
                keys.insert(node as *const Item, i);
                Reference::new(&document, node, &source)
            })
            .collect();
        let bindings = handle.html_bindings(&document.root);
        let html = notist_html::Renderer::default()
            .with_sources(&document.sources)
            .with_code_renderer(notist_html::highlight::code)
            .prepare(&document.root, &bindings.candidates(), |node| {
                keys[&(node as *const Item)]
            })
            .map_err(|e| Error::Message(e.into()))?;
        Ok((
            Self {
                page: document.page,
                ids: document.ids,
                references,
                html,
                bindings,
                source,
            },
            errors,
        ))
    }

    pub fn source_text(&self) -> &str {
        self.source.text().map(|s| s.as_ref()).unwrap_or("")
    }
}

pub(crate) fn image(description: &str, url: &str) -> Option<String> {
    if !notist_html::is_safe_url(url) {
        return None;
    }
    let extension = Path::new(url.split(['?', '#']).next()?)
        .extension()?
        .to_str()?;
    if ![
        "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "apng", "bmp", "ico",
    ]
    .iter()
    .any(|e| extension.eq_ignore_ascii_case(e))
    {
        return None;
    }
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        notist_html::escape_attribute(url),
        notist_html::escape_attribute(description)
    ))
}
