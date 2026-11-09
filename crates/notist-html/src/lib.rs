//! Render Notist's final [`Item`] tree into an HTML fragment.
//!
//! The default renderer consumes `notist-core` without syntax dependencies.
//! The optional `highlight` feature adds syntax-aware code rendering. Analyze
//! source with a frontend first, then pass the final tree to [`render`].
//!
//! ```
//! use notist_core::item::{Ctor, Item, Value};
//! let text = Item::new(Ctor::Text, Default::default())
//!     .with_field("text", Value::Str("Hello <world>".into()));
//! assert_eq!(notist_html::render(&text), "Hello &lt;world&gt;");
//! ```
//!
//! [`Renderer`] allows applications to supply trusted HTML for the contents
//! of math, embed and block code nodes. The renderer retains the outer element
//! and attributes; returning `None` selects the default rendering.

use notist_core::diag::{Diagnostic, Phase};
use notist_core::item::{Ctor, Item, Value};

mod attributes;
pub mod components;
mod escape;
#[cfg(feature = "highlight")]
pub mod highlight;
mod url;
pub use components::{BindingError, Component, HtmlRegistry, ModuleLocator, Target};

pub use escape::{escape_attribute, escape_text};
pub use url::is_safe_url;

/// Optional CSS for draft, staged and outdate status annotations.
pub const STATUS_CSS: &str = include_str!("status.css");

type Hook<'a> = Box<dyn Fn(&Item) -> Option<String> + 'a>;
type UrlResolver<'a> = Box<dyn Fn(&Item, &str) -> Option<String> + 'a>;
type ResourceResolver<'a> =
    Box<dyn Fn(&Item, &notist_core::resource::ResourcePath) -> Option<String> + 'a>;

/// An HTML fragment and problems encountered while rendering it.
///
/// These diagnostics supplement source analysis: for example, a disallowed
/// URL or an attributed group on an HTML structural axis. They carry the
/// originating item's source range and use the semantic phase.
#[derive(Debug)]
pub struct RenderResult {
    pub html: String,
    pub diagnostics: Vec<Diagnostic>,
    /// Empty unless `Renderer::with_source_map` is enabled.
    pub source_map: Vec<SourceMapping>,
    pub used_components: Vec<Component>,
    /// Keeps source-map locations valid independently of the input analysis.
    pub sources: notist_core::diag::Sources,
}

/// A mapping to an element actually emitted by this render. IDs are local to
/// this result; ranges are UTF-8 byte offsets in the analyzed source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMapping {
    pub node_id: usize,
    pub range: std::ops::Range<usize>,
    pub kind: SourceMappingKind,
    pub origin: Option<notist_core::item::Origin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMappingKind {
    Block,
    Inline,
    Container,
}

/// Render using the defaults. Use [`Renderer::render_with_diagnostics`] to
/// retain rendering diagnostics in addition to the HTML.
pub fn render(item: &Item) -> String {
    Renderer::default().render(item)
}

// Fence info strings can carry comma- or whitespace-separated attributes such
// as `rust,ignore,no_run`. Only the leading language controls presentation.
fn code_language(info: &str) -> Option<&str> {
    info.trim_start()
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .next()
        .filter(|language| !language.is_empty())
}

/// Configurable rendering of final IR. Hooks may borrow application state.
pub struct Renderer<'a> {
    embed: Option<Hook<'a>>,
    math: Option<Hook<'a>>,
    code: Option<Hook<'a>>,
    source_map: bool,
    registry: HtmlRegistry,
    url_resolver: Option<UrlResolver<'a>>,
    resource_resolver: Option<ResourceResolver<'a>>,
    sources: notist_core::diag::Sources,
}
impl Default for Renderer<'_> {
    fn default() -> Self {
        Self::from_registry(HtmlRegistry::default())
    }
}

impl<'a> Renderer<'a> {
    pub fn with_sources(mut self, sources: &notist_core::diag::Sources) -> Self {
        self.sources = sources.clone().with_definitions(self.registry.identities());
        self
    }
    /// Resolve link and URL-string embed targets using host data, without IO. `None`
    /// keeps the source URL. Both source and resolved URLs are checked for safety.
    pub fn with_url_resolver(
        mut self,
        resolve: impl Fn(&Item, &str) -> Option<String> + 'a,
    ) -> Self {
        self.url_resolver = Some(Box::new(resolve));
        self
    }
    /// Select a public URL for a Path link or embed. No path stringification
    /// or URL fallback occurs when a resource has no host binding. The returned
    /// base URL is safety-checked; query/fragment fields are added afterward.
    pub fn with_resource_resolver(
        mut self,
        resolve: impl Fn(&Item, &notist_core::resource::ResourcePath) -> Option<String> + 'a,
    ) -> Self {
        self.resource_resolver = Some(Box::new(resolve));
        self
    }
    pub fn with_registry(mut self, registry: HtmlRegistry) -> Self {
        self.sources.definitions_mut().merge(registry.identities());
        self.registry = registry;
        self
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Render with an already bound registry without constructing and discarding
    /// the default native registry first.
    pub fn from_registry(registry: HtmlRegistry) -> Self {
        let sources = notist_core::diag::Sources::new().with_definitions(registry.identities());
        Self {
            embed: None,
            math: None,
            code: None,
            source_map: false,
            registry,
            url_resolver: None,
            resource_resolver: None,
            sources,
        }
    }

    /// Emit reserved `data-notist-node` identifiers and source ranges. Default
    /// output is unchanged. Transparent and structural groups stay flattened;
    /// empty-range generated nodes have no identifier and fall back to parents.
    pub fn with_source_map(mut self) -> Self {
        self.source_map = true;
        self
    }

    /// Supply the inner HTML of an embed's `span.notist-embed`.
    ///
    /// The callback receives the full item, including the Path or URL target,
    /// query, fragment, description, title and attributes. Local Paths must use
    /// a host's explicit resource mapping, never path stringification.
    /// Return `None` to use a resource link. Returned
    /// HTML is trusted application output and is not escaped or sanitized;
    /// use [`escape_attribute`], [`escape_text`] and [`is_safe_url`] when
    /// incorporating source fields.
    pub fn with_embed_renderer(mut self, render: impl Fn(&Item) -> Option<String> + 'a) -> Self {
        self.embed = Some(Box::new(render));
        self
    }

    /// Supply the inner HTML of math's `span.notist-math` (inline) or
    /// `div.notist-math` (block).
    ///
    /// Returning `None` displays escaped formula text. As with the embed
    /// callback, returned HTML is trusted application output.
    pub fn with_math_renderer(mut self, render: impl Fn(&Item) -> Option<String> + 'a) -> Self {
        self.math = Some(Box::new(render));
        self
    }

    /// Supply trusted HTML for the text inside a block code element, such as
    /// escaped syntax highlighting spans. The renderer owns the pre/code shell.
    /// Returning `None` retains the ordinary escaped text.
    pub fn with_code_renderer(mut self, render: impl Fn(&Item) -> Option<String> + 'a) -> Self {
        self.code = Some(Box::new(render));
        self
    }

    pub fn render(&self, item: &Item) -> String {
        self.render_with_diagnostics(item).html
    }

    pub fn render_with_diagnostics(&self, item: &Item) -> RenderResult {
        let mut state = State {
            renderer: self,
            output: String::new(),
            diagnostics: Vec::new(),
            source_map: Vec::new(),
            used_components: Vec::new(),
        };
        if self.source_map
            && let Err(error) = self.sources.validate_node(item)
        {
            return RenderResult {
                html: String::new(),
                diagnostics: vec![Diagnostic::new(
                    Phase::Semantic,
                    item.span,
                    error.to_string(),
                )],
                source_map: Vec::new(),
                used_components: Vec::new(),
                sources: self.sources.clone(),
            };
        }
        state.node(item);
        RenderResult {
            html: state.output,
            diagnostics: state.diagnostics,
            source_map: state.source_map,
            used_components: state.used_components,
            sources: self.sources.clone(),
        }
    }
}

struct State<'r, 'a> {
    renderer: &'r Renderer<'a>,
    output: String,
    diagnostics: Vec<Diagnostic>,
    source_map: Vec<SourceMapping>,
    used_components: Vec<Component>,
}

impl State<'_, '_> {
    fn node(&mut self, item: &Item) {
        for (key, value) in item.attrs.iter() {
            if value.contains_data_views() {
                self.diagnostic(item, format!("Data or DataError in attribute `{key}` cannot be exported as HTML; decode selected ordinary values first"));
            }
            if value.contains_path() {
                self.diagnostic(item, format!("Path in attribute `{key}` cannot be exported as HTML; publish the resource explicitly"));
            }
        }
        if let Ctor::Error(message) = item.ctor() {
            let tag = if is_block(item) { "div" } else { "span" };
            self.open(tag, item, "notist-error");
            self.attr("role", "alert");
            self.output.push('>');
            self.text(&message);
            self.close(tag);
            return;
        }
        if item.ctor() == Ctor::Group {
            self.transparent(item, if is_block(item) { "div" } else { "span" });
            return;
        }

        if let Some(id) = item.function_id()
            && let Some(target) = self.renderer.registry.target(id)
        {
            match target {
                Target::Component(component) => {
                    self.component(item, component);
                    return;
                }
                Target::Native(ctor) => {
                    self.native(item, ctor);
                    return;
                }
            }
        }
        if let Some(target) = item.instance() {
            let name = self
                .renderer
                .sources
                .definitions()
                .function(target.function)
                .or_else(|_| {
                    self.renderer
                        .registry
                        .identities()
                        .function(target.function)
                });
            let message = match name {
                Ok(name) if name.module == "notist" => {
                    self.native(item, &item.ctor());
                    return;
                }
                Ok(name) => format!("no HTML implementation for `{name}`"),
                Err(error) => error.to_string(),
            };
            self.diagnostic(item, message);
            self.transparent(item, if is_block(item) { "div" } else { "span" });
            return;
        }
        self.native(item, &item.ctor());
    }

    fn component(&mut self, item: &Item, component: &Component) {
        if !self
            .used_components
            .iter()
            .any(|used| used.id == component.id)
        {
            self.used_components.push(component.clone());
        }
        self.open(&component.tag, item, "");
        self.attr("notist-protocol", "2");
        self.attr("notist-function", &component.id.to_string());
        self.attr(
            "notist-content",
            if is_block(item) { "block" } else { "inline" },
        );
        if let Some(instance) = item.instance() {
            if component.signature.comptime().len() != instance.arguments.len() {
                self.diagnostic(item, "component comptime arity mismatch".into());
            }
            for (parameter, value) in component
                .signature
                .comptime()
                .iter()
                .zip(&instance.arguments)
            {
                let encoded = match value {
                    notist_core::definitions::ComptimeValue::Type(t) => format!("{t}"),
                    notist_core::definitions::ComptimeValue::Data(v) => {
                        match components::encode_parameter(v) {
                            Ok(value) => value,
                            Err(message) => {
                                self.diagnostic(item, message.into());
                                continue;
                            }
                        }
                    }
                };
                self.attr(
                    &format!(
                        "notist-comptime-{}",
                        components::encode_name(&parameter.name)
                    ),
                    &encoded,
                );
            }
        }
        let mut bindings =
            notist_core::signature::ComptimeBindings::new(component.signature.clone());
        if let Some(instance) = item.instance() {
            for (parameter, value) in component
                .signature
                .comptime()
                .iter()
                .zip(&instance.arguments)
            {
                if let Err(error) = bindings.insert(parameter.id, value.clone()) {
                    self.diagnostic(item, error.to_string());
                }
            }
        }
        for (parameter, template) in component
            .definition
            .parameters
            .iter()
            .zip(component.signature.parameters())
        {
            if let Some(value) = item.field(&parameter.name) {
                let valid = match bindings.require(template) {
                    Ok(ty) => ty.accepts(value),
                    Err(error) => {
                        self.diagnostic(item, error.to_string());
                        false
                    }
                };
                if valid {
                    match components::encode_parameter(value) {
                        Ok(encoded) => {
                            self.attr(&components::parameter_attribute(&parameter.name), &encoded)
                        }
                        Err(message) => self.diagnostic(
                            item,
                            format!("component parameter `{}`: {message}", parameter.name),
                        ),
                    }
                } else {
                    self.diagnostic(
                        item,
                        format!("invalid component parameter `{}` omitted", parameter.name),
                    );
                }
            }
        }
        self.output.push('>');
        self.children(item);
        self.close(&component.tag);
    }

    fn native(&mut self, item: &Item, ctor: &Ctor) {
        match ctor {
            Ctor::Doc => self.transparent(item, "div"),
            Ctor::Node => self.transparent(item, if is_block(item) { "div" } else { "span" }),
            Ctor::Text => {
                if attributes::has_html_attrs(&item.attrs)
                    || (self.renderer.source_map && !item.span.is_empty())
                {
                    self.open("span", item, "");
                    self.output.push('>');
                    self.text(string_field(item, "text").unwrap_or(""));
                    self.close("span");
                } else {
                    self.text(string_field(item, "text").unwrap_or(""));
                }
            }
            Ctor::Paragraph => {
                // An opaque inline extension can accept Content as light DOM.
                // A p would be implicitly closed by its descendant block tags,
                // so retain this IR paragraph as a flow container in that case.
                if item.children().iter().any(has_block_content) {
                    self.container("div", item, "notist-paragraph");
                } else {
                    self.container("p", item, "");
                }
            }
            Ctor::Heading => {
                let level = int_field(item, "level").filter(|n| *n > 0).unwrap_or(1);
                if level <= 6 {
                    self.container(
                        ["h1", "h2", "h3", "h4", "h5", "h6"][(level - 1) as usize],
                        item,
                        "",
                    );
                } else {
                    self.open("div", item, "notist-heading");
                    self.attr("role", "heading");
                    self.attr("aria-level", &level.to_string());
                    self.output.push('>');
                    self.children(item);
                    self.close("div");
                }
            }
            Ctor::Section => self.container("section", item, ""),
            Ctor::Strong => self.container("strong", item, ""),
            Ctor::Emph => self.container("em", item, ""),
            Ctor::Strike => self.container("del", item, ""),
            Ctor::Underline => {
                self.open("span", item, "");
                self.attr("style", "text-decoration: underline");
                self.output.push('>');
                self.children(item);
                self.close("span");
            }
            Ctor::Divider => {
                self.open("hr", item, "");
                self.output.push('>');
            }
            Ctor::RawInline => {
                let block = is_block(item);
                let class = string_field(item, "lang")
                    .and_then(code_language)
                    .map(|lang| format!("language-{lang}"))
                    .unwrap_or_default();
                if block {
                    self.open("pre", item, "");
                    self.output.push('>');
                    self.output.push_str("<code");
                    if !class.is_empty() {
                        self.attr("class", &class);
                    }
                } else {
                    self.open("code", item, &class);
                }
                self.output.push('>');
                if let Some(html) = block
                    .then(|| self.renderer.code.as_ref().and_then(|f| f(item)))
                    .flatten()
                {
                    self.output.push_str(&html);
                } else {
                    self.text(string_field(item, "text").unwrap_or(""));
                }
                self.close("code");
                if block {
                    self.close("pre");
                }
            }
            Ctor::Link => self.link(item),
            Ctor::Embed => {
                self.open("span", item, "notist-embed");
                self.field_title(item);
                self.output.push('>');
                if let Some(html) = self.renderer.embed.as_ref().and_then(|f| f(item)) {
                    self.output.push_str(&html);
                } else {
                    self.output.push_str("<a");
                    self.url(item, "target", "href", true);
                    self.output.push('>');
                    self.text(
                        string_field(item, "description")
                            .filter(|s| !s.is_empty())
                            .or_else(|| string_field(item, "target"))
                            .or_else(|| match item.field("target") {
                                Some(Value::Path(path)) => path.path().rsplit('/').next(),
                                _ => None,
                            })
                            .unwrap_or(""),
                    );
                    self.close("a");
                }
                self.close("span");
            }
            Ctor::Math => {
                let tag = if is_block(item) { "div" } else { "span" };
                self.open(tag, item, "notist-math");
                self.output.push('>');
                if let Some(html) = self.renderer.math.as_ref().and_then(|f| f(item)) {
                    self.output.push_str(&html);
                } else {
                    self.text(string_field(item, "text").unwrap_or(""));
                }
                self.close(tag);
            }
            Ctor::Callout => {
                let kind = string_field(item, "kind");
                let quote = kind == Some("quote");
                let tag = if quote { "blockquote" } else { "div" };
                self.open(tag, item, if quote { "" } else { "notist-callout" });
                if let Some(kind) = kind {
                    self.attr("data-notist-kind", kind);
                }
                self.output.push('>');
                self.children(item);
                self.close(tag);
            }
            Ctor::List => {
                let ordered = bool_field(item, "ordered");
                let tag = if ordered { "ol" } else { "ul" };
                self.open(tag, item, "");
                if ordered && let Some(start) = int_field(item, "start").filter(|n| *n != 1) {
                    self.attr("start", &start.to_string());
                }
                self.output.push('>');
                for child in self.structural(item.children(), &Ctor::ListItem, "list") {
                    if child.ctor() == Ctor::ListItem && !is_error(child) {
                        self.node(child);
                    } else {
                        self.output.push_str("<li>");
                        self.node(child);
                        self.close("li");
                    }
                }
                self.close(tag);
            }
            Ctor::ListItem => self.container("li", item, ""),
            Ctor::Table => self.table(item),
            Ctor::TableRow => self.row(item, &[]),
            Ctor::TableCell => self.cell(item, false, None),
            Ctor::Extension(function) => {
                self.diagnostic(
                    item,
                    self.renderer
                        .sources
                        .definitions()
                        .function(function.id)
                        .map(|name| format!("no HTML implementation for `{name}`"))
                        .unwrap_or_else(|error| error.to_string()),
                );
                self.container(
                    if is_block(item) { "div" } else { "span" },
                    item,
                    "notist-extension",
                );
            }
            Ctor::Group | Ctor::Error(_) => {
                self.transparent(item, if is_block(item) { "div" } else { "span" })
            }
            Ctor::Interpolation => {
                self.diagnostic(item, "content interpolation requires evaluation".into());
                self.container(
                    if is_block(item) { "div" } else { "span" },
                    item,
                    "notist-pending",
                );
            }
            Ctor::Custom(name) => {
                self.diagnostic(
                    item,
                    format!("unknown constructor `{name}` rendered as a container"),
                );
                let tag = if is_block(item) { "div" } else { "span" };
                self.open(tag, item, "notist-custom");
                self.attr("data-notist-constructor", name);
                self.output.push('>');
                self.children(item);
                self.close(tag);
            }
        }
    }

    // open intentionally leaves '>' pending for semantic attributes.
    fn open(&mut self, tag: &str, item: &Item, class: &str) {
        self.output.push('<');
        self.output.push_str(tag);
        attributes::write(&mut self.output, &item.attrs, class);
        if self.renderer.source_map && !item.span.is_empty() && item.ctor() != Ctor::Doc {
            let node_id = self.source_map.len();
            let kind = if matches!(item.ctor(), Ctor::Heading | Ctor::Paragraph) {
                SourceMappingKind::Block
            } else {
                match tag {
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "li" | "tr" | "hr" => {
                        SourceMappingKind::Block
                    }
                    "div" | "section" | "blockquote" | "ul" | "ol" | "table" => {
                        SourceMappingKind::Container
                    }
                    _ if tag.contains('-') && is_block(item) => SourceMappingKind::Container,
                    _ => SourceMappingKind::Inline,
                }
            };
            self.source_map.push(SourceMapping {
                node_id,
                origin: item.origin.clone(),
                range: usize::from(item.span.start())..usize::from(item.span.end()),
                kind,
            });
            self.attr("data-notist-node", &node_id.to_string());
        }
    }

    fn close(&mut self, tag: &str) {
        self.output.push_str("</");
        self.output.push_str(tag);
        self.output.push('>');
    }

    fn container(&mut self, tag: &str, item: &Item, class: &str) {
        self.open(tag, item, class);
        self.output.push('>');
        self.children(item);
        self.close(tag);
    }

    fn transparent(&mut self, item: &Item, tag: &str) {
        if attributes::has_html_attrs(&item.attrs) {
            self.container(tag, item, "");
        } else {
            self.children(item);
        }
    }

    fn children(&mut self, item: &Item) {
        for child in item.children() {
            self.node(child);
        }
    }

    fn text(&mut self, text: &str) {
        escape::write(&mut self.output, text, false);
    }

    fn attr(&mut self, key: &str, value: &str) {
        attributes::attribute(&mut self.output, key, value);
    }

    fn field_title(&mut self, item: &Item) {
        if !attributes::has_title(&item.attrs)
            && let Some(title) = string_field(item, "title")
        {
            self.attr("title", title);
        }
    }

    fn url(&mut self, item: &Item, field: &str, attribute: &str, embed: bool) {
        if let Some(Value::Path(path)) = item.field(field) {
            let resolved = self
                .renderer
                .resource_resolver
                .as_ref()
                .and_then(|resolve| resolve(item, path));
            match resolved {
                Some(url) if is_safe_url(&url) => {
                    match notist_core::embed::append_suffix(&url, &item.fields()) {
                        Ok(url) if is_safe_url(&url) => self.attr(attribute, &url),
                        Ok(_) => self.diagnostic(
                            item,
                            format!("disallowed published URL; `{attribute}` omitted"),
                        ),
                        Err(message) => self.diagnostic(item, message),
                    }
                }
                Some(_) => self.diagnostic(
                    item,
                    format!("disallowed published URL; `{attribute}` omitted"),
                ),
                None => {
                    self.diagnostic(
                        item,
                        format!(
                            "local {} requires a host-provided resource URL",
                            if embed { "embed" } else { "link" }
                        ),
                    );
                    self.diagnostics.last_mut().unwrap().code =
                        Some(notist_core::diag::DiagnosticCode::UnpublishedResource);
                }
            }
            return;
        }
        if let Some(url) = string_field(item, field) {
            let resolved = is_safe_url(url)
                .then(|| {
                    self.renderer
                        .url_resolver
                        .as_ref()
                        .and_then(|resolve| resolve(item, url))
                })
                .flatten();
            let target = resolved.as_deref().unwrap_or(url);
            if is_safe_url(url) && is_safe_url(target) {
                {
                    match notist_core::embed::append_suffix(target, &item.fields()) {
                        Ok(target) if is_safe_url(&target) => self.attr(attribute, &target),
                        Ok(_) => self.diagnostic(
                            item,
                            format!("disallowed published URL; `{attribute}` omitted"),
                        ),
                        Err(message) => self.diagnostic(item, message),
                    }
                }
            } else {
                self.diagnostic(
                    item,
                    format!("disallowed URL in `{field}`; `{attribute}` omitted"),
                );
            }
        }
    }

    fn link(&mut self, item: &Item) {
        self.open("a", item, "");
        self.url(item, "target", "href", false);
        self.field_title(item);
        self.output.push('>');
        if item.children().is_empty() {
            match item.field("target") {
                Some(Value::Path(path)) => {
                    let label = self
                        .renderer
                        .resource_resolver
                        .as_ref()
                        .and_then(|resolve| resolve(item, path));
                    self.text(label.as_deref().unwrap_or(""));
                }
                _ => self.text(string_field(item, "target").unwrap_or("")),
            }
        } else {
            self.children(item);
        }
        self.close("a");
    }

    fn table(&mut self, item: &Item) {
        let align = match item.field("align") {
            Some(Value::Array(values)) => &values[..],
            _ => &[],
        };
        let rows = self.structural(item.children(), &Ctor::TableRow, "table");
        let headers = rows
            .iter()
            .take_while(|row| {
                row.ctor() == Ctor::TableRow && !is_error(row) && bool_field(row, "header")
            })
            .count();
        self.open("table", item, "");
        self.output.push('>');
        if headers > 0 {
            self.output.push_str("<thead>");
            for row in &rows[..headers] {
                self.row(row, align);
            }
            self.close("thead");
        }
        if headers < rows.len() {
            self.output.push_str("<tbody>");
            for row in &rows[headers..] {
                if row.ctor() == Ctor::TableRow && !is_error(row) {
                    self.row(row, align);
                } else {
                    self.output.push_str("<tr><td>");
                    self.node(row);
                    self.output.push_str("</td></tr>");
                }
            }
            self.close("tbody");
        }
        self.close("table");
    }

    fn row(&mut self, item: &Item, align: &[Value]) {
        self.open("tr", item, "");
        self.output.push('>');
        let header = bool_field(item, "header");
        for (column, child) in self
            .structural(item.children(), &Ctor::TableCell, "row")
            .into_iter()
            .enumerate()
        {
            if child.ctor() == Ctor::TableCell && !is_error(child) {
                self.cell(child, header, align.get(column));
            } else {
                self.output.push_str("<td>");
                self.node(child);
                self.close("td");
            }
        }
        self.close("tr");
    }

    fn cell(&mut self, item: &Item, header: bool, align: Option<&Value>) {
        let tag = if header { "th" } else { "td" };
        self.open(tag, item, "");
        if header {
            self.attr("scope", "col");
        }
        if let Some(Value::Str(align)) = align
            && matches!(align.as_str(), "left" | "center" | "right")
        {
            self.attr("style", &format!("text-align: {align}"));
        }
        self.output.push('>');
        self.children(item);
        self.close(tag);
    }

    fn structural<'n>(
        &mut self,
        children: &'n [Item],
        expected: &Ctor,
        parent: &str,
    ) -> Vec<&'n Item> {
        let mut result = Vec::new();
        for child in children {
            if is_error(child) {
                result.push(child);
            } else if child.ctor() == Ctor::Node || child.ctor() == Ctor::Group {
                if attributes::has_html_attrs(&child.attrs) {
                    let group = if child.ctor() == Ctor::Node {
                        "node"
                    } else {
                        "content group"
                    };
                    self.diagnostic(child, format!("{group} attributes omitted inside `{parent}`: HTML has no wrapper for this structural sequence"));
                }
                result.extend(self.structural(child.children(), expected, parent));
            } else {
                if child.ctor() != *expected {
                    self.diagnostic(
                        child,
                        format!(
                            "unexpected `{}` inside `{parent}`; rendered in a recovery container",
                            child
                                .ctor()
                                .name(self.renderer.sources.definitions())
                                .map(|name| name.into_owned())
                                .unwrap_or_else(|error| error.to_string())
                        ),
                    );
                }
                result.push(child);
            }
        }
        result
    }

    fn diagnostic(&mut self, item: &Item, message: String) {
        let mut d = Diagnostic::new(Phase::Semantic, item.span, message);
        if let Some(origin) = &item.origin {
            match self.renderer.sources.source(origin.source) {
                Ok(source) => d.source = Some(source.clone()),
                Err(error) if origin.source != notist_core::source::SourceId::DETACHED => {
                    d.message
                        .push_str(&format!(" (source location unavailable: {error})"));
                    d.span = Default::default();
                }
                Err(_) => {}
            }
        }
        self.diagnostics.push(d);
    }
}

fn string_field<'n>(item: &'n Item, key: &str) -> Option<&'n str> {
    match item.field(key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

fn int_field(item: &Item, key: &str) -> Option<i64> {
    match item.field(key) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

fn bool_field(item: &Item, key: &str) -> bool {
    matches!(item.field(key), Some(Value::Bool(true)))
}

fn is_block(item: &Item) -> bool {
    item.level == notist_core::builtins::Level::Block
}

fn is_error(item: &Item) -> bool {
    item.is_error()
}

fn has_block_content(item: &Item) -> bool {
    is_block(item) || item.children().iter().any(has_block_content)
}
