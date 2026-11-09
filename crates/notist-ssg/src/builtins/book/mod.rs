//! SUMMARY interpretation and reading metadata. Rendering belongs to site themes.
mod layout;
mod routes;
mod structure;
mod summary;

use crate::{
    Error, Page,
    extensions::{CompileInput, SiteContext, SiteExtension, TemplateInput},
    mounts::Mounts,
};
pub use layout::{Navigation, PageLink};
use notist::{DiagnosticCode, SourceDiagnostic, Vault, resources::ResourceKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
pub use structure::NavigationKind;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BookConfig {
    /// Relative to the site content root. None discovers SUMMARY.not or SUMMARY.md.
    pub summary: Option<PathBuf>,
    /// Restrict publication to SUMMARY entries; otherwise it only controls navigation.
    pub select: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub config: BookConfig,
    entries: Option<Vec<structure::Entry>>,
}

impl Book {
    pub fn new(config: BookConfig) -> Self {
        Self {
            config,
            entries: None,
        }
    }
}

impl SiteExtension for Book {
    fn name(&self) -> &str {
        "builtins.book"
    }

    fn before_compile(
        &mut self,
        context: &SiteContext<'_>,
        input: &mut CompileInput,
    ) -> Result<(), Error> {
        let content = context.resources.resolve(&context.config.content);
        let mounts = Mounts::new(context.resources, context.config)?;
        let summary = if let Some(path) = &self.config.summary {
            if !crate::extensions::valid_path(path) {
                return Err(Error::Message(
                    "book summary must be a content-relative path".into(),
                ));
            }
            Some(content.join(path))
        } else {
            let mut found = None;
            for name in summary::FILE_NAMES {
                let path = content.join(name);
                if input.documents.contains_key(&PathBuf::from(name))
                    || context.resources.kind(&path)? == Some(ResourceKind::File)
                {
                    if found.is_some() {
                        return Err(Error::Message(
                            "both SUMMARY.not and SUMMARY.md exist; keep one book structure file"
                                .into(),
                        ));
                    }
                    found = Some(path);
                }
            }
            found
        };
        let summary_source = summary
            .as_ref()
            .and_then(|path| input.documents.remove(path.strip_prefix(&content).unwrap()));
        for name in summary::FILE_NAMES {
            input.documents.remove(&PathBuf::from(name));
        }
        let entries = if let Some(path) = summary {
            let source = summary_source
                .map(Ok)
                .unwrap_or_else(|| context.resources.source(&path))?;
            // SUMMARY is parsed as data, never evaluated or transformed by document extensions.
            let mut vault = Vault::new(context.resources);
            let analysis = vault.analyze(&path, &source)?;
            let mut errors: Vec<_> = analysis
                .diagnostics()
                .iter()
                .filter(|d| d.code != Some(DiagnosticCode::OutsideVault))
                .cloned()
                .collect();
            let sources: BTreeSet<_> = input
                .documents
                .keys()
                .map(|p| mounts.resolve(&content.join(p)))
                .collect();
            let plan = summary::interpret(analysis.root()).and_then(|entries| {
                summary::resolve(&entries, &path, &sources, &context.resources, &mounts)
            });
            let plan = match plan {
                Ok(plan) => Some(plan),
                Err(diagnostics) => {
                    errors.extend(diagnostics);
                    None
                }
            };
            if !errors.is_empty() {
                return Err(Error::Diagnostics(
                    errors
                        .into_iter()
                        .map(|d| SourceDiagnostic::locate(&path, &source, d))
                        .collect(),
                ));
            }
            let plan = plan.unwrap();
            if self.config.select {
                input
                    .documents
                    .retain(|p, _| plan.includes(&mounts.resolve(&content.join(p)), &mounts));
            }
            let routes = input
                .documents
                .keys()
                .map(|p| {
                    let mut route = crate::Route::plan(
                        content.join(p),
                        &content,
                        context.config.directory_urls,
                    )?;
                    route.source = mounts.resolve(&route.source);
                    Ok(route)
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Some(plan.expand(&routes::Routes::new(routes).map_err(Error::Message)?))
        } else {
            None
        };
        if input.documents.is_empty() {
            return Err(Error::Message("book contains no selected documents".into()));
        }
        self.entries = entries;
        Ok(())
    }

    fn before_template(
        &mut self,
        context: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        let pages = input.pages;
        if input.data.contains_key("book") {
            return Err(Error::Message(
                "site.data.book is reserved by the book extension".into(),
            ));
        }
        let entries = &self.entries;
        fn validate(entries: &[structure::Entry], pages: &[Page]) -> Result<(), Error> {
            for entry in entries {
                if let Some(source) = &entry.source {
                    if !pages.iter().any(|p| &p.route.source == source) {
                        return Err(Error::Message(format!(
                            "book navigation references removed page `{}`",
                            source.display()
                        )));
                    }
                }
                validate(&entry.children, pages)?;
            }
            Ok(())
        }
        if let Some(entries) = entries {
            validate(entries, pages)?;
        }
        let layout = match entries {
            Some(entries) => layout::Layout::declared(entries, pages),
            None => layout::Layout::new(&context.config.title, pages),
        };
        let mut metadata = BTreeMap::new();
        for page in pages {
            let current = &page.route.output;
            let (previous, next) = layout.adjacent(current, pages);
            let breadcrumbs = layout.breadcrumbs(current);
            let ancestors: Vec<_> = breadcrumbs
                .iter()
                .filter(|node| !node.current)
                .map(|node| node.id)
                .collect();
            metadata.insert(
                page.route.path.to_string_lossy().into_owned(),
                serde_json::json!({
                    "home": layout.home_url(current),
                    "ancestors": ancestors,
                    "breadcrumbs": breadcrumbs,
                    "previous": previous,
                    "next": next,
                }),
            );
        }
        input.data.insert("book".into(), serde_json::json!({
            "structure": entries,
            "navigation": layout.navigation(std::path::Path::new("__navigation__")),
            "order": layout.order(),
            "pages": metadata,
        }));
        Ok(())
    }
}
