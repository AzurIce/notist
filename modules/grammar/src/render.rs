use crate::{CharacterSetItem, Expr, ExprKind, Grammar, Layer, RangeLimit};
use railroad::*;
use std::{collections::HashSet, fmt::Write, str::FromStr};

type Rail = Box<dyn Node>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Light,
    Dark,
    Rust,
    Coal,
    Navy,
    Ayu,
}

impl FromStr for Theme {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            "rust" => Ok(Self::Rust),
            "coal" => Ok(Self::Coal),
            "navy" => Ok(Self::Navy),
            "ayu" => Ok(Self::Ayu),
            _ => Err(format!(
                "unknown grammar theme `{value}`; expected light, dark, rust, coal, navy or ayu"
            )),
        }
    }
}

impl Theme {
    fn stylesheet(self) -> Stylesheet {
        match self {
            Self::Light => Stylesheet::Light,
            Self::Dark => Stylesheet::Dark,
            Self::Rust => Stylesheet::Rust,
            Self::Coal => Stylesheet::Coal,
            Self::Navy => Stylesheet::Navy,
            Self::Ayu => Stylesheet::Ayu,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Render just this production, or all productions in source order.
    pub rule: Option<String>,
    pub theme: Theme,
    /// Sequences wrap between nodes when they exceed this width. A single
    /// indivisible node can still be wider; the component scrolls horizontally.
    pub max_width: i64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            rule: None,
            theme: Theme::Light,
            max_width: 900,
        }
    }
}

/// Produce standalone SVGs, one per production, with embedded CSS.
///
/// PEG decisions and zero-width operations are explicitly annotated. The
/// diagrams describe recognition structure, not an executable automaton.
pub fn render(grammar: &Grammar, options: &RenderOptions) -> Result<String, String> {
    if options.max_width < 200 {
        return Err("diagram max_width must be at least 200".into());
    }
    let rules: Vec<_> = grammar
        .rules
        .iter()
        .filter(|rule| options.rule.as_ref().is_none_or(|name| name == &rule.name))
        .collect();
    if let Some(name) = &options.rule
        && rules.is_empty()
    {
        return Err(format!("unknown grammar rule `{name}`"));
    }
    if rules.is_empty() {
        return Err("grammar has no productions to draw".into());
    }
    let names = rules.iter().map(|rule| rule.name.as_str()).collect();

    let mut output = String::new();
    for rule in rules {
        let context = Context {
            names: &names,
            max_width: options.max_width,
            layer: rule.layer,
            building: false,
        };
        let name = invocation(&rule.name, rule.parameters.as_deref());
        let layer = match rule.layer {
            Layer::Lex => "lex",
            Layer::Syntax => "syntax",
            Layer::Ast => "ast",
        };
        let title = format!(
            "{layer} {name}{}",
            if rule.is_root { " · @root" } else { "" }
        );
        let rail: Rail = Box::new(Sequence::new(vec![
            Box::new(SimpleStart) as Rail,
            context.expression(&rule.expression),
            Box::new(SimpleEnd),
        ]));
        let mut sections = vec![comment(&title), rail];
        if let Some(result) = &rule.result {
            let result_context = Context {
                building: true,
                ..context
            };
            sections.push(comment("=> result · no input consumption"));
            sections.push(result_context.expression(result));
        }
        let root: Rail = Box::new(VerticalGrid::new(sections));
        let mut diagram = Diagram::new_with_stylesheet(root, &options.theme.stylesheet());
        let geometry = diagram.compute_geometry();
        diagram
            .attr("width".into())
            .or_insert_with(|| geometry.width.to_string());
        diagram
            .attr("height".into())
            .or_insert_with(|| geometry.height.to_string());
        diagram
            .attr("id".into())
            .or_insert_with(|| rule_id(&rule.name));
        diagram.attr("role".into()).or_insert_with(|| "img".into());
        diagram
            .attr("aria-label".into())
            .or_insert_with(|| format!("Grammar for {title}"));
        diagram.add_element(svg::Element::new("title").text(&title));
        // Keep each exported SVG independent of the component's own CSS.
        diagram.add_css("svg.railroad { font-synthesis: none; } svg.railroad a:hover text { text-decoration: underline; }");
        writeln!(output, "{diagram}").unwrap();
    }
    Ok(output)
}

struct Context<'a> {
    names: &'a HashSet<&'a str>,
    layer: Layer,
    max_width: i64,
    building: bool,
}

impl Context<'_> {
    fn expression(&self, expression: &Expr) -> Rail {
        match &expression.kind {
            ExprKind::Literal(text) => Box::new(Terminal::new(visible(text))),
            ExprKind::Unicode { hex, .. } => Box::new(Terminal::new(format!("U+{hex}"))),
            ExprKind::Prose(text) => label(comment(text), "prose · specified separately"),
            ExprKind::Complement(excluded) => {
                let label_node: Rail = Box::new(VerticalGrid::new(vec![
                    comment("with the exception of"),
                    self.expression(excluded),
                ]));
                let input: Rail = Box::new(NonTerminal::new(
                    match self.layer {
                        Layer::Lex => "CHAR",
                        Layer::Syntax => "ANY",
                        Layer::Ast => "NODE",
                    }
                    .into(),
                ));
                Box::new(LabeledBox::new(input, label_node))
            }
            ExprKind::Annotated {
                expression,
                suffix,
                footnote,
            } => {
                let mut node = self.expression(expression);
                if let Some(suffix) = suffix {
                    node = label(node, suffix);
                }
                if let Some(footnote) = footnote {
                    node = label(node, &format!("footnote {footnote}"));
                }
                node
            }
            ExprKind::Reference { name, arguments } => {
                let node = self.reference(name, arguments.as_deref());
                if self.building {
                    label(node, "value / call")
                } else if self.layer == Layer::Ast {
                    label(node, "AST node pattern")
                } else {
                    node
                }
            }
            ExprKind::Node {
                name,
                arguments,
                children,
            } => {
                let content = self.expression(children);
                label(
                    content,
                    &format!(
                        "{} {}",
                        if self.building {
                            "construct"
                        } else {
                            "AST node pattern"
                        },
                        invocation(name, arguments.as_deref())
                    ),
                )
            }
            ExprKind::Computation(code) => {
                label(comment(&compact(code)), "pure computation · result only")
            }
            ExprKind::CharacterSet(items) => {
                let nodes: Vec<Rail> = items
                    .iter()
                    .map(|item| match item {
                        CharacterSetItem::Character(ch) => {
                            Box::new(Terminal::new(visible(&ch.to_string()))) as Rail
                        }
                        CharacterSetItem::Range(first, last) => Box::new(Terminal::new(format!(
                            "{}-{}",
                            visible(&first.to_string()),
                            visible(&last.to_string())
                        ))),
                        CharacterSetItem::Reference(name) => self.reference(name, None),
                    })
                    .collect();
                label(Box::new(Choice::new(nodes)), "character set")
            }
            ExprKind::Sequence(expressions) => {
                let commit = expressions
                    .iter()
                    .position(|e| matches!(e.kind, ExprKind::Commit));
                let mut nodes: Vec<_> = expressions[..commit.unwrap_or(expressions.len())]
                    .iter()
                    .map(|e| self.expression(e))
                    .collect();
                if let Some(commit) = commit {
                    let rest = self.sequence(
                        expressions[commit + 1..]
                            .iter()
                            .map(|e| self.expression(e))
                            .collect(),
                    );
                    nodes.push(label(rest, "committed · failure → Broken"));
                }
                self.sequence(nodes)
            }
            ExprKind::OrderedChoice(expressions) => {
                let alternatives: Vec<Rail> = expressions
                    .iter()
                    .enumerate()
                    .map(|(index, expression)| {
                        Box::new(Sequence::new(vec![
                            comment(&format!("{}", index + 1)),
                            self.expression(expression),
                        ])) as Rail
                    })
                    .collect();
                label(
                    Box::new(Choice::new(alternatives)),
                    if self.building {
                        "alternative results"
                    } else {
                        "ordered choice · first match"
                    },
                )
            }
            ExprKind::Repeat {
                expression,
                min,
                max,
                limit,
                count,
            } => {
                let inner = self.expression(expression);
                let greedy = if self.building { "" } else { " · greedy" };
                let numeric_max =
                    max.as_ref()
                        .and_then(|v| v.parse::<u32>().ok())
                        .map(|v| match limit {
                            RangeLimit::Closed => v,
                            RangeLimit::HalfOpen => v.saturating_sub(1),
                        });
                let numeric_min = min.parse::<u32>().ok();
                let mut node: Rail = match (numeric_min, numeric_max, max) {
                    (Some(0), Some(0), _) => label(Box::new(Empty), "0 times"),
                    (Some(1), Some(1), _) => inner,
                    (Some(0), Some(1), _) => label(
                        Box::new(Optional::new(inner)),
                        if self.building {
                            "optional result"
                        } else {
                            "optional · greedy"
                        },
                    ),
                    (Some(0), None, None) => Box::new(Optional::new(Repeat::new(
                        inner,
                        comment(if self.building {
                            "repeated result"
                        } else {
                            "greedy"
                        }),
                    ))),
                    (Some(1), None, None) => Box::new(Repeat::new(
                        inner,
                        comment(if self.building {
                            "repeated result"
                        } else {
                            "greedy"
                        }),
                    )),
                    _ => {
                        let times = if max.as_deref() == Some(min) && *limit == RangeLimit::Closed {
                            format!("exactly {min} times{greedy}")
                        } else {
                            format!(
                                "total {min}{}{} times{greedy}",
                                if *limit == RangeLimit::Closed {
                                    "..="
                                } else {
                                    ".."
                                },
                                max.as_deref().unwrap_or("")
                            )
                        };
                        let repeated: Rail = Box::new(Repeat::new(inner, comment(&times)));
                        if min == "0" || numeric_min.is_none() {
                            Box::new(Optional::new(repeated))
                        } else {
                            repeated
                        }
                    }
                };
                if let Some(count) = count {
                    node = label(node, &format!("bind repeat count {count}"));
                }
                node
            }
            ExprKind::Lookahead {
                positive,
                expression,
            } => label(
                self.expression(expression),
                if *positive {
                    "positive lookahead · zero-width"
                } else {
                    "negative lookahead · zero-width"
                },
            ),
            ExprKind::Capture { name, expression } => {
                label(self.expression(expression), &format!("capture {name}"))
            }
            ExprKind::Predicate(predicate) => {
                label(comment(&compact(predicate)), "predicate · zero-width")
            }
            ExprKind::Binding { name, value } => label(
                comment(&format!("let {name} = {}", compact(value))),
                "binding · zero-width",
            ),
            ExprKind::Within { end, expression } => label(
                self.expression(expression),
                &format!("within {} · restore limit on exit", compact(end)),
            ),
            ExprKind::Scanner {
                probe,
                name,
                arguments,
            } => {
                let action = if *probe { "probe" } else { "scan" };
                let scanner: Rail = Box::new(NonTerminal::new(format!(
                    "{action} {}",
                    invocation(name, arguments.as_deref())
                )));
                label(
                    scanner,
                    if *probe {
                        "scanner · zero-width"
                    } else {
                        "scanner · consume through result.end"
                    },
                )
            }
            ExprKind::Commit => label(Box::new(Empty), "commit · failure → Broken"),
            ExprKind::Empty => label(
                Box::new(Empty),
                if self.building {
                    "ε · empty result"
                } else {
                    "ε · zero-width"
                },
            ),
        }
    }

    fn reference(&self, name: &str, arguments: Option<&str>) -> Rail {
        let node = NonTerminal::new(invocation(name, arguments));
        if self.names.contains(name) {
            Box::new(Link::new(node, format!("#{}", rule_id(name))))
        } else {
            Box::new(node)
        }
    }

    fn sequence(&self, nodes: Vec<Rail>) -> Rail {
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut width = 0;
        for node in nodes {
            let size = node.width() + 12;
            if !row.is_empty() && width + size > self.max_width {
                rows.push(Box::new(Sequence::new(std::mem::take(&mut row))) as Rail);
                width = 0;
            }
            width += size;
            row.push(node);
        }
        if !row.is_empty() {
            rows.push(Box::new(Sequence::new(row)) as Rail);
        }
        match rows.len() {
            0 => Box::new(Empty),
            1 => rows.pop().unwrap(),
            _ => Box::new(Stack::new(rows)),
        }
    }
}

fn comment(text: &str) -> Rail {
    Box::new(Comment::new(text.to_owned()))
}
fn label(inner: Rail, text: &str) -> Rail {
    Box::new(LabeledBox::new(inner, comment(text)))
}
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn invocation(name: &str, arguments: Option<&str>) -> String {
    match arguments {
        Some(arguments) => format!("{name}({})", compact(arguments)),
        None => name.to_owned(),
    }
}
fn visible(text: &str) -> String {
    if text.is_empty() {
        return "\"\"".into();
    }
    text.chars()
        .map(|ch| match ch {
            '\n' => "\\n".into(),
            '\r' => "\\r".into(),
            '\t' => "\\t".into(),
            ' ' => "␠".into(),
            c if c.is_control() => format!("U+{:04X}", c as u32),
            c => c.to_string(),
        })
        .collect()
}
fn rule_id(name: &str) -> String {
    let mut id = String::from("grammar-");
    for byte in name.bytes() {
        write!(id, "{byte:02x}").unwrap();
    }
    id
}
