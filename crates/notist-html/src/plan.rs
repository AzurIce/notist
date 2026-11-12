//! Tree-free HTML preparation. Literal bytes are encoded once; only host
//! decisions (URLs and optional component shells) survive as typed insertions.
use super::*;
use notist_core::definitions::DefId;
use std::collections::BTreeMap;

pub struct HtmlPlan {
    literal: String,
    insertions: Vec<Insertion>,
}

pub struct PlanResult {
    pub html: String,
    pub diagnostics: Vec<Diagnostic>,
    pub used_components: Vec<Component>,
}

pub(super) struct Preparation<'a> {
    candidates: &'a BTreeMap<DefId, Component>,
    reference: &'a dyn Fn(&Item) -> usize,
}

pub(super) struct Insertion {
    offset: usize,
    operation: Operation,
}

pub(super) enum Operation {
    Diagnostic(Diagnostic),
    ComponentUse(DefId),
    ComponentOpen {
        id: DefId,
        bound: String,
        fallback: String,
        diagnostics: Vec<Diagnostic>,
        missing: Diagnostic,
    },
    ComponentClose {
        id: DefId,
        bound: String,
        fallback: String,
    },
    Url {
        key: usize,
        url: Url,
    },
    ResourceLabel {
        key: usize,
    },
    Embed {
        key: usize,
        url: Url,
        label: String,
    },
}

/// Only the scalars needed for URL presentation, never a Value/Item or Sources.
pub(super) struct Url {
    original: Option<String>,
    resource: bool,
    query: Option<String>,
    fragment: Option<String>,
    location: Diagnostic,
    embed: bool,
}

impl Url {
    fn prepare(state: &State<'_, '_>, item: &Item, embed: bool) -> Self {
        let mut diagnostic = state.shell();
        diagnostic.diagnostic(item, String::new());
        Self {
            original: string_field(item, "target").map(str::to_owned),
            resource: matches!(item.field("target"), Some(Value::Path(_))),
            query: string_field(item, "query").map(str::to_owned),
            fragment: string_field(item, "fragment").map(str::to_owned),
            location: diagnostic.diagnostics.pop().unwrap(),
            embed,
        }
    }

    fn resolve(&self, resolved: Option<&str>) -> Result<Option<String>, Diagnostic> {
        let issue = |message: String| {
            let mut d = self.location.clone();
            d.message.insert_str(0, &message);
            d
        };
        let base = if self.resource {
            match resolved {
                Some(url) if is_safe_url(url) => url,
                Some(_) => return Err(issue("disallowed published URL; `href` omitted".into())),
                None => {
                    let mut d = issue(format!(
                        "local {} requires a host-provided resource URL",
                        if self.embed { "embed" } else { "link" }
                    ));
                    d.code = Some(notist_core::diag::DiagnosticCode::UnpublishedResource);
                    return Err(d);
                }
            }
        } else if let Some(original) = self.original.as_deref() {
            let base = resolved.unwrap_or(original);
            if !is_safe_url(original) || !is_safe_url(base) {
                return Err(issue("disallowed URL in `target`; `href` omitted".into()));
            }
            base
        } else {
            return Ok(None);
        };
        let url = notist_core::embed::append_suffix_parts(
            base,
            self.query.as_deref(),
            self.fragment.as_deref(),
        )
        .map_err(issue)?;
        if !is_safe_url(&url) {
            return Err(issue("disallowed published URL; `href` omitted".into()));
        }
        Ok(Some(url))
    }
}

impl Renderer<'_> {
    /// Encode a tree without consulting host resources. Candidates are possible,
    /// not yet selected, component bindings. Reference keys belong to the host.
    /// Math/code hooks run now; URL and embed decisions run when finalizing.
    /// Source-map and URL/embed-hook rendering use the immediate render API.
    pub fn prepare(
        &self,
        item: &Item,
        candidates: &BTreeMap<DefId, Component>,
        reference: impl Fn(&Item) -> usize,
    ) -> Result<HtmlPlan, &'static str> {
        if self.source_map
            || self.url_resolver.is_some()
            || self.resource_resolver.is_some()
            || self.embed.is_some()
        {
            return Err(
                "HTML preparation requires unmapped output and deferred URL/embed decisions",
            );
        }
        notist_observe::measure(
            "core.render.html.prepare",
            || "Prepare HTML plan".into(),
            || {
                let mut state = State {
                    renderer: self,
                    output: String::new(),
                    diagnostics: Vec::new(),
                    source_map: Vec::new(),
                    used_components: Vec::new(),
                    preparation: Some(Preparation {
                        candidates,
                        reference: &reference,
                    }),
                    insertions: Vec::new(),
                };
                state.node(item);
                Ok(HtmlPlan {
                    literal: state.output,
                    insertions: state.insertions,
                })
            },
        )
    }
}

impl HtmlPlan {
    /// Materialize in presentation order using immutable host decisions.
    /// The optional embed hook receives a complete, safety-checked URL.
    /// The registry must preserve the preparation renderer's installed bindings
    /// and use the same authoritative signatures for selected candidates.
    /// Every reference key supplied during preparation must index `urls`.
    pub fn finish(
        self,
        registry: &HtmlRegistry,
        urls: &[Option<String>],
        embed: impl Fn(usize, &str) -> Option<String>,
    ) -> PlanResult {
        notist_observe::measure(
            "core.render.html.finish",
            || "Finalize HTML plan".into(),
            || {
                let mut result = PlanResult {
                    html: String::with_capacity(self.literal.len()),
                    diagnostics: Vec::new(),
                    used_components: Vec::new(),
                };
                let mut start = 0;
                for Insertion { offset, operation } in self.insertions {
                    result.html.push_str(&self.literal[start..offset]);
                    start = offset;
                    match operation {
                        Operation::Diagnostic(d) => result.diagnostics.push(d),
                        Operation::ComponentUse(id) => {
                            if let Some(Target::Component(component)) = registry.target(id)
                                && !result
                                    .used_components
                                    .iter()
                                    .any(|used| used.id == component.id)
                            {
                                result.used_components.push(component.clone());
                            }
                        }
                        Operation::ComponentOpen {
                            id,
                            bound,
                            fallback,
                            diagnostics,
                            missing,
                        } => {
                            if let Some(Target::Component(component)) = registry.target(id) {
                                if !result
                                    .used_components
                                    .iter()
                                    .any(|used| used.id == component.id)
                                {
                                    result.used_components.push(component.clone());
                                }
                                result.html.push_str(&bound);
                                result.diagnostics.extend(diagnostics);
                            } else {
                                result.html.push_str(&fallback);
                                result.diagnostics.push(missing);
                            }
                        }
                        Operation::ComponentClose {
                            id,
                            bound,
                            fallback,
                        } => {
                            result.html.push_str(
                                if matches!(registry.target(id), Some(Target::Component(_))) {
                                    &bound
                                } else {
                                    &fallback
                                },
                            );
                        }
                        Operation::Url { key, url } => match url.resolve(urls[key].as_deref()) {
                            Ok(Some(url)) => attributes::attribute(&mut result.html, "href", &url),
                            Ok(None) => {}
                            Err(d) => result.diagnostics.push(d),
                        },
                        Operation::ResourceLabel { key } => {
                            escape::write(
                                &mut result.html,
                                urls[key].as_deref().unwrap_or(""),
                                false,
                            );
                        }
                        Operation::Embed { key, url, label } => {
                            let resolved = url.resolve(urls[key].as_deref());
                            if let Ok(Some(url)) = &resolved
                                && let Some(html) = embed(key, url)
                            {
                                result.html.push_str(&html);
                                continue;
                            }
                            result.html.push_str("<a");
                            match resolved {
                                Ok(Some(url)) => {
                                    attributes::attribute(&mut result.html, "href", &url)
                                }
                                Ok(None) => {}
                                Err(d) => result.diagnostics.push(d),
                            }
                            result.html.push('>');
                            escape::write(&mut result.html, &label, false);
                            result.html.push_str("</a>");
                        }
                    }
                }
                result.html.push_str(&self.literal[start..]);
                result
            },
        )
    }
}

impl State<'_, '_> {
    fn shell(&self) -> State<'_, '_> {
        State {
            renderer: self.renderer,
            output: String::new(),
            diagnostics: Vec::new(),
            source_map: Vec::new(),
            used_components: Vec::new(),
            preparation: None,
            insertions: Vec::new(),
        }
    }

    pub(super) fn insert(&mut self, operation: Operation) {
        self.insertions.push(Insertion {
            offset: self.output.len(),
            operation,
        });
    }

    pub(super) fn deferred_component(&mut self, item: &Item) -> bool {
        let Some(preparation) = &self.preparation else {
            return false;
        };
        let Some(component) = item
            .function_id()
            .and_then(|id| preparation.candidates.get(&id))
        else {
            return false;
        };
        let id = component.id.def_id();
        let mut bound = self.shell();
        bound.component_open(item, component);
        let mut fallback = self.shell();
        fallback.diagnostic(
            item,
            format!("no HTML implementation for `{}`", component.id),
        );
        let tag = if is_block(item) { "div" } else { "span" };
        let wrapped = attributes::has_html_attrs(&item.attrs);
        if wrapped {
            fallback.open(tag, item, "");
            fallback.output.push('>');
        }
        let closing = format!("</{}>", component.tag);
        let operation = Operation::ComponentOpen {
            id,
            bound: bound.output,
            fallback: fallback.output,
            diagnostics: bound.diagnostics,
            missing: fallback.diagnostics.pop().unwrap(),
        };
        self.insert(operation);
        self.children(item);
        self.insert(Operation::ComponentClose {
            id,
            bound: closing,
            fallback: if wrapped {
                format!("</{tag}>")
            } else {
                String::new()
            },
        });
        true
    }

    pub(super) fn deferred_url(&mut self, item: &Item, embed: bool) -> bool {
        let Some(preparation) = &self.preparation else {
            return false;
        };
        let key = (preparation.reference)(item);
        let url = Url::prepare(self, item, embed);
        if embed {
            self.open("span", item, "notist-embed");
            self.field_title(item);
            self.output.push('>');
            let label = string_field(item, "description")
                .filter(|s| !s.is_empty())
                .or_else(|| string_field(item, "target"))
                .or_else(|| match item.field("target") {
                    Some(Value::Path(path)) => path.path().rsplit('/').next(),
                    _ => None,
                })
                .unwrap_or("")
                .to_owned();
            self.insert(Operation::Embed { key, url, label });
            self.close("span");
        } else {
            self.open("a", item, "");
            self.insert(Operation::Url { key, url });
            self.field_title(item);
            self.output.push('>');
            if item.children().is_empty() {
                if matches!(item.field("target"), Some(Value::Path(_))) {
                    self.insert(Operation::ResourceLabel { key });
                } else {
                    self.text(string_field(item, "target").unwrap_or(""));
                }
            } else {
                self.children(item);
            }
            self.close("a");
        }
        true
    }
}
