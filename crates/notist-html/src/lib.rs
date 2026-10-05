//! Render Notist's final [`Item`] tree into an HTML fragment.
//!
//! This crate depends only on `notist-core`. Analyze source with a frontend
//! first, then pass the final document or a subtree to [`render`].
//!
//! ```
//! use notist_core::item::{Ctor, Item, Value};
//! let text = Item::new(Ctor::Text, Default::default())
//!     .with_field("text", Value::Str("Hello <world>".into()));
//! assert_eq!(notist_html::render(&text), "Hello &lt;world&gt;");
//! ```
//!
//! [`Renderer`] allows applications to supply trusted HTML for the contents
//! of math and embed nodes. The renderer retains the node's outer element
//! and attributes; returning `None` selects the default rendering.

use notist_core::diag::{Diagnostic, Phase};
use notist_core::item::{Ctor, Item, Value};

mod attributes;
pub mod components;
mod escape;
mod url;
pub use components::{BindingError, Component, HtmlRegistry, ModuleLocator, Target};

pub use escape::{escape_attribute, escape_text};
pub use url::is_safe_url;

type Hook<'a> = Box<dyn Fn(&Item) -> Option<String> + 'a>;
type UrlResolver<'a> = Box<dyn Fn(&Item, &str) -> Option<String> + 'a>;

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
}

/// A mapping to an element actually emitted by this render. IDs are local to
/// this result; ranges are UTF-8 byte offsets in the analyzed source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMapping {
    pub node_id: usize,
    pub range: std::ops::Range<usize>,
    pub kind: SourceMappingKind,
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

/// Configurable rendering of final IR. Hooks may borrow application state.
#[derive(Default)]
pub struct Renderer<'a> {
    embed: Option<Hook<'a>>,
    math: Option<Hook<'a>>,
    source_map: bool,
    registry: HtmlRegistry,
    url_resolver: Option<UrlResolver<'a>>,
}

impl<'a> Renderer<'a> {
    /// Resolve link/embed targets using explicit host data, without IO. `None`
    /// keeps the source URL. Both source and resolved URLs are checked for safety.
    pub fn with_url_resolver(
        mut self,
        resolve: impl Fn(&Item, &str) -> Option<String> + 'a,
    ) -> Self {
        self.url_resolver = Some(Box::new(resolve));
        self
    }
    pub fn with_registry(mut self, registry: HtmlRegistry) -> Self {
        self.registry = registry;
        self
    }
    pub fn new() -> Self {
        Self::default()
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
    /// The callback receives the full item, including target, description,
    /// title and attributes. Return `None` to use a resource link. Returned
    /// HTML is trusted application output and is not escaped or sanitized;
    /// use [`escape_attribute`], [`escape_text`] and [`is_safe_url`] when
    /// incorporating source fields.
    pub fn with_embed_renderer(mut self, render: impl Fn(&Item) -> Option<String> + 'a) -> Self {
        self.embed = Some(Box::new(render));
        self
    }

    /// Supply the inner HTML of a math node's `span.notist-math`.
    ///
    /// Returning `None` displays escaped formula text. As with the embed
    /// callback, returned HTML is trusted application output.
    pub fn with_math_renderer(mut self, render: impl Fn(&Item) -> Option<String> + 'a) -> Self {
        self.math = Some(Box::new(render));
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
        state.node(item);
        RenderResult {
            html: state.output,
            diagnostics: state.diagnostics,
            source_map: state.source_map,
            used_components: state.used_components,
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
        if let Some(id) = item.ctor.function_id()
            && let Some(target) = self.renderer.registry.get(&id)
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
        self.native(item, &item.ctor);
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
        self.attr("notist-protocol", "1");
        for parameter in &component.definition.parameters {
            if let Some(value) = item.fields.get(&parameter.name) {
                if parameter.ty.accepts(value) {
                    self.attr(
                        &components::parameter_attribute(&parameter.name),
                        &components::encode_parameter(value),
                    );
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
            Ctor::Group => self.transparent(item, if is_block(item) { "div" } else { "span" }),
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
                if item.children.iter().any(has_block_content) {
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
            Ctor::Divider => {
                self.open("hr", item, "");
                self.output.push('>');
            }
            Ctor::RawInline => {
                let block = bool_field(item, "block");
                let class = string_field(item, "lang")
                    .and_then(|s| s.split_whitespace().next())
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
                self.text(string_field(item, "text").unwrap_or(""));
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
                    self.url(item, "target", "href");
                    self.output.push('>');
                    self.text(
                        string_field(item, "description")
                            .filter(|s| !s.is_empty())
                            .or_else(|| string_field(item, "target"))
                            .unwrap_or(""),
                    );
                    self.close("a");
                }
                self.close("span");
            }
            Ctor::Math => {
                self.open("span", item, "notist-math");
                self.output.push('>');
                if let Some(html) = self.renderer.math.as_ref().and_then(|f| f(item)) {
                    self.output.push_str(&html);
                } else {
                    self.text(string_field(item, "text").unwrap_or(""));
                }
                self.close("span");
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
                for child in self.structural(&item.children, &Ctor::ListItem, "list") {
                    if child.ctor == Ctor::ListItem {
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
                    format!("no HTML implementation for `{}`", function.id),
                );
                self.container(
                    if is_block(item) { "div" } else { "span" },
                    item,
                    "notist-extension",
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
        if self.renderer.source_map && !item.span.is_empty() && item.ctor != Ctor::Doc {
            let node_id = self.source_map.len();
            let kind = if matches!(item.ctor, Ctor::Heading | Ctor::Paragraph) {
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
        for child in &item.children {
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

    fn url(&mut self, item: &Item, field: &str, attribute: &str) {
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
                self.attr(attribute, target);
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
        self.url(item, "target", "href");
        self.field_title(item);
        self.output.push('>');
        if item.children.is_empty() {
            self.text(string_field(item, "target").unwrap_or(""));
        } else {
            self.children(item);
        }
        self.close("a");
    }

    fn table(&mut self, item: &Item) {
        let align = match item.fields.get("align") {
            Some(Value::Array(values)) => values.as_slice(),
            _ => &[],
        };
        let rows = self.structural(&item.children, &Ctor::TableRow, "table");
        let headers = rows
            .iter()
            .take_while(|row| row.ctor == Ctor::TableRow && bool_field(row, "header"))
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
                if row.ctor == Ctor::TableRow {
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
            .structural(&item.children, &Ctor::TableCell, "row")
            .into_iter()
            .enumerate()
        {
            if child.ctor == Ctor::TableCell {
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
            if child.ctor == Ctor::Group {
                if attributes::has_html_attrs(&child.attrs) {
                    self.diagnostic(child, format!("group attributes omitted inside `{parent}`: HTML has no wrapper for this structural sequence"));
                }
                result.extend(self.structural(&child.children, expected, parent));
            } else {
                if child.ctor != *expected {
                    self.diagnostic(
                        child,
                        format!(
                            "unexpected `{}` inside `{parent}`; rendered in a recovery container",
                            child.ctor.name()
                        ),
                    );
                }
                result.push(child);
            }
        }
        result
    }

    fn diagnostic(&mut self, item: &Item, message: String) {
        self.diagnostics
            .push(Diagnostic::new(Phase::Semantic, item.span, message));
    }
}

fn string_field<'n>(item: &'n Item, key: &str) -> Option<&'n str> {
    match item.fields.get(key) {
        Some(Value::Str(value)) => Some(value),
        _ => None,
    }
}

fn int_field(item: &Item, key: &str) -> Option<i64> {
    match item.fields.get(key) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

fn bool_field(item: &Item, key: &str) -> bool {
    matches!(item.fields.get(key), Some(Value::Bool(true)))
}

fn is_block(item: &Item) -> bool {
    item.level == notist_core::builtins::Level::Block
}

fn has_block_content(item: &Item) -> bool {
    is_block(item) || item.children.iter().any(has_block_content)
}
