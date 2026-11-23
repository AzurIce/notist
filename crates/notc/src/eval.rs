use crate::ast::{BinaryOp, Span, UnaryOp};
use crate::builtin::BuiltinFn;
use crate::check::{CheckedExpr, CheckedProgram, FunctionId};
use crate::content::{
    AttrValue, Attrs, Block, BlockKind, CustomNode, CustomValue, Inline, InlineKind,
};
use crate::json::{DataHost, JsonValue, NoDataHost};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Inline(Inline),
    Block(Block),
    Json(JsonValue),
    List(Arc<[Value]>),
    Function(FunctionValue),
    Builtin(BuiltinFn, Span),
}

/// An opaque function reference local to the program that produced it.
#[derive(Debug, Clone, Copy)]
pub struct FunctionValue(FunctionId);

#[derive(Debug, PartialEq, Eq)]
pub enum ExecutionError {
    UnknownEntry(String),
    EntryRequiresArguments(usize),
    CallDepthExceeded,
    IntegerOverflow,
    DivisionByZero,
    HostError(String),
}

struct EvalCtx<'a> {
    program: &'a CheckedProgram,
    doc_path: &'a str,
    host: &'a dyn DataHost,
}

impl CheckedProgram {
    /// Execute a zero-argument entry in a fresh session without an external data host.
    pub fn run(&self, entry: &str) -> Result<Value, ExecutionError> {
        self.run_with_host(entry, "", &NoDataHost)
    }

    /// Execute a zero-argument entry with a read-only external `DataHost`.
    pub fn run_with_host(
        &self,
        entry: &str,
        doc_path: &str,
        host: &dyn DataHost,
    ) -> Result<Value, ExecutionError> {
        let id = *self
            .function_names
            .get(entry)
            .ok_or_else(|| ExecutionError::UnknownEntry(entry.into()))?;
        let function = &self.functions[id.0];
        if function.arity != 0 {
            return Err(ExecutionError::EntryRequiresArguments(function.arity));
        }
        let ctx = EvalCtx {
            program: self,
            doc_path,
            host,
        };
        evaluate(&ctx, &function.body, &mut Vec::new(), 0, 1)
    }
}

fn evaluate(
    ctx: &EvalCtx<'_>,
    expr: &CheckedExpr,
    stack: &mut Vec<Value>,
    base: usize,
    depth: usize,
) -> Result<Value, ExecutionError> {
    use CheckedExpr as C;
    match expr {
        C::Unit => Ok(Value::Unit),
        C::Bool(b) => Ok(Value::Bool(*b)),
        C::Int(n) => Ok(Value::Int(*n)),
        C::Float(f) => Ok(Value::Float(*f)),
        C::String(s) => Ok(Value::String(Arc::clone(s))),
        C::Local(slot) => Ok(stack[base + slot].clone()),
        C::Function(id) => Ok(Value::Function(FunctionValue(*id))),
        C::Builtin(builtin, span) => Ok(Value::Builtin(*builtin, *span)),
        C::InlineFragment(items, attrs, span) => {
            let mut inlines = Vec::with_capacity(items.len());
            for item in items {
                let Value::Inline(inline) = evaluate(ctx, item, stack, base, depth)? else {
                    unreachable!("checked inline fragment item")
                };
                inlines.push(inline);
            }
            Ok(Value::Inline(Inline::fragment(
                inlines,
                attrs.clone(),
                *span,
            )))
        }
        C::BlockFragment(items, attrs, span) => {
            let mut blocks = Vec::with_capacity(items.len());
            for item in items {
                let Value::Block(block) = evaluate(ctx, item, stack, base, depth)? else {
                    unreachable!("checked block fragment item")
                };
                blocks.push(block);
            }
            Ok(Value::Block(Block::fragment(blocks, attrs.clone(), *span)))
        }
        C::BlockList(ordered, items, attrs, span) => {
            let mut blocks = Vec::with_capacity(items.len());
            for item in items {
                let Value::Block(block) = evaluate(ctx, item, stack, base, depth)? else {
                    unreachable!("checked block list item")
                };
                blocks.push(block);
            }
            Ok(Value::Block(Block::new(
                BlockKind::List {
                    ordered: *ordered,
                    items: Arc::from(blocks),
                },
                attrs.clone(),
                *span,
            )))
        }
        C::BlockTable {
            headers,
            rows,
            attrs,
            span,
        } => {
            let mut eval_headers = Vec::with_capacity(headers.len());
            for cell in headers {
                let Value::Inline(inl) = evaluate(ctx, cell, stack, base, depth)? else {
                    unreachable!("checked table header cell")
                };
                eval_headers.push(inl);
            }
            let mut eval_rows = Vec::with_capacity(rows.len());
            for row in rows {
                let mut eval_row = Vec::with_capacity(row.len());
                for cell in row {
                    let Value::Inline(inl) = evaluate(ctx, cell, stack, base, depth)? else {
                        unreachable!("checked table cell")
                    };
                    eval_row.push(inl);
                }
                eval_rows.push(Arc::from(eval_row));
            }
            Ok(Value::Block(Block::new(
                BlockKind::Table {
                    headers: Arc::from(eval_headers),
                    rows: Arc::from(eval_rows),
                },
                attrs.clone(),
                *span,
            )))
        }
        C::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(evaluate(ctx, item, stack, base, depth)?);
            }
            Ok(Value::List(Arc::from(out)))
        }
        C::For(iterable, body) => {
            let Value::List(items) = evaluate(ctx, iterable, stack, base, depth)? else {
                unreachable!("checked for iterable")
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                stack.push(item.clone());
                let val = evaluate(ctx, body, stack, base, depth);
                stack.pop();
                out.push(val?);
            }
            Ok(Value::List(Arc::from(out)))
        }
        C::CustomNode {
            is_block,
            module,
            name,
            props,
            span,
        } => {
            let mut eval_props = Vec::with_capacity(props.len());
            for (k, expr) in props {
                let val = match evaluate(ctx, expr, stack, base, depth)? {
                    Value::Unit => CustomValue::Unit,
                    Value::Bool(b) => CustomValue::Bool(b),
                    Value::Int(n) => CustomValue::Int(n),
                    Value::Float(f) => CustomValue::Float(f),
                    Value::String(s) => CustomValue::String(s),
                    Value::Inline(i) => CustomValue::Inline(i),
                    Value::Block(b) => CustomValue::Block(b),
                    Value::Json(j) => CustomValue::String(Arc::from(j.to_string())),
                    Value::List(_) | Value::Function(_) | Value::Builtin(_, _) => {
                        unreachable!("checked extern prop")
                    }
                };
                eval_props.push((Arc::clone(k), val));
            }
            let custom = CustomNode {
                module: Arc::clone(module),
                name: Arc::clone(name),
                props: Arc::from(eval_props),
            };
            if *is_block {
                Ok(Value::Block(Block::new(
                    BlockKind::Custom(custom),
                    Attrs::empty(),
                    *span,
                )))
            } else {
                Ok(Value::Inline(Inline::new(
                    InlineKind::Custom(custom),
                    Attrs::empty(),
                    *span,
                )))
            }
        }
        C::AttachAttrs(inner, attrs) => match evaluate(ctx, inner, stack, base, depth)? {
            Value::Inline(inline) => Ok(Value::Inline(inline.with_attrs(attrs.clone()))),
            Value::Block(block) => Ok(Value::Block(block.with_attrs(attrs.clone()))),
            _ => unreachable!("checked attribute target"),
        },
        C::Let(initializer, body) => {
            let value = evaluate(ctx, initializer, stack, base, depth)?;
            stack.push(value);
            let result = evaluate(ctx, body, stack, base, depth);
            stack.pop();
            result
        }
        C::Seq(first, second) => {
            let _ = evaluate(ctx, first, stack, base, depth)?;
            evaluate(ctx, second, stack, base, depth)
        }
        C::Unary(op, operand) => {
            let value = evaluate(ctx, operand, stack, base, depth)?;
            match (op, value) {
                (UnaryOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                (UnaryOp::Neg, Value::Int(n)) => n
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or(ExecutionError::IntegerOverflow),
                (UnaryOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
                _ => unreachable!("checked unary operand"),
            }
        }
        C::Binary(BinaryOp::And, left, right) => {
            let Value::Bool(left) = evaluate(ctx, left, stack, base, depth)? else {
                unreachable!("checked boolean operand")
            };
            if !left {
                return Ok(Value::Bool(false));
            }
            evaluate(ctx, right, stack, base, depth)
        }
        C::Binary(BinaryOp::Or, left, right) => {
            let Value::Bool(left) = evaluate(ctx, left, stack, base, depth)? else {
                unreachable!("checked boolean operand")
            };
            if left {
                return Ok(Value::Bool(true));
            }
            evaluate(ctx, right, stack, base, depth)
        }
        C::Binary(op, left, right) => {
            let left = evaluate(ctx, left, stack, base, depth)?;
            let right = evaluate(ctx, right, stack, base, depth)?;
            eval_binary(*op, left, right)
        }
        C::If(condition, yes, no) => {
            let Value::Bool(condition) = evaluate(ctx, condition, stack, base, depth)? else {
                unreachable!("checked condition")
            };
            evaluate(ctx, if condition { yes } else { no }, stack, base, depth)
        }
        C::Call(callee, arguments) => {
            let callee_value = evaluate(ctx, callee, stack, base, depth)?;
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate(ctx, argument, stack, base, depth)?);
            }
            call_value(ctx, callee_value, values, stack, depth)
        }
    }
}

fn call_value(
    ctx: &EvalCtx<'_>,
    callee: Value,
    values: Vec<Value>,
    stack: &mut Vec<Value>,
    depth: usize,
) -> Result<Value, ExecutionError> {
    const MAX_CALL_DEPTH: usize = 64;
    if depth >= MAX_CALL_DEPTH {
        return Err(ExecutionError::CallDepthExceeded);
    }
    match callee {
        Value::Function(FunctionValue(id)) => {
            let frame = stack.len();
            stack.extend(values);
            let result = evaluate(
                ctx,
                &ctx.program.functions[id.0].body,
                stack,
                frame,
                depth + 1,
            );
            stack.truncate(frame);
            result
        }
        Value::Builtin(builtin, span) => eval_builtin(ctx, builtin, span, values, stack, depth),
        _ => unreachable!("checked call target"),
    }
}

fn attr_to_arc_str(value: &AttrValue) -> Arc<str> {
    match value {
        AttrValue::String(s) => Arc::clone(s),
        AttrValue::Bool(b) => Arc::from(b.to_string()),
        AttrValue::Int(n) => Arc::from(n.to_string()),
        AttrValue::Float(f) => Arc::from(f.to_string()),
    }
}

fn eval_builtin(
    ctx: &EvalCtx<'_>,
    builtin: BuiltinFn,
    span: Span,
    values: Vec<Value>,
    stack: &mut Vec<Value>,
    depth: usize,
) -> Result<Value, ExecutionError> {
    let mut args = values.into_iter();
    let take_string = |args: &mut std::vec::IntoIter<Value>| -> Arc<str> {
        match args.next() {
            Some(Value::String(s)) => s,
            _ => unreachable!("checked string builtin argument"),
        }
    };
    let take_int = |args: &mut std::vec::IntoIter<Value>| -> i64 {
        match args.next() {
            Some(Value::Int(n)) => n,
            _ => unreachable!("checked int builtin argument"),
        }
    };
    let take_float = |args: &mut std::vec::IntoIter<Value>| -> f64 {
        match args.next() {
            Some(Value::Float(f)) => f,
            _ => unreachable!("checked float builtin argument"),
        }
    };
    let take_bool = |args: &mut std::vec::IntoIter<Value>| -> bool {
        match args.next() {
            Some(Value::Bool(b)) => b,
            _ => unreachable!("checked bool builtin argument"),
        }
    };
    let take_inline = |args: &mut std::vec::IntoIter<Value>| -> Inline {
        match args.next() {
            Some(Value::Inline(inline)) => inline,
            _ => unreachable!("checked inline builtin argument"),
        }
    };
    let take_block = |args: &mut std::vec::IntoIter<Value>| -> Block {
        match args.next() {
            Some(Value::Block(block)) => block,
            _ => unreachable!("checked block builtin argument"),
        }
    };
    let take_list = |args: &mut std::vec::IntoIter<Value>| -> Arc<[Value]> {
        match args.next() {
            Some(Value::List(items)) => items,
            _ => unreachable!("checked list builtin argument"),
        }
    };
    let take_json = |args: &mut std::vec::IntoIter<Value>| -> JsonValue {
        match args.next() {
            Some(Value::Json(j)) => j,
            _ => unreachable!("checked json builtin argument"),
        }
    };

    Ok(match builtin {
        BuiltinFn::Text => {
            let text = take_string(&mut args);
            Value::Inline(Inline::text(text, span))
        }
        BuiltinFn::Strong => {
            let inner = take_inline(&mut args);
            Value::Inline(Inline::new(InlineKind::Strong(inner), Attrs::empty(), span))
        }
        BuiltinFn::Emph => {
            let inner = take_inline(&mut args);
            Value::Inline(Inline::new(InlineKind::Emph(inner), Attrs::empty(), span))
        }
        BuiltinFn::Raw => {
            let code = take_string(&mut args);
            Value::Inline(Inline::new(InlineKind::Raw(code), Attrs::empty(), span))
        }
        BuiltinFn::Math => {
            let expr = take_string(&mut args);
            Value::Inline(Inline::new(InlineKind::Math(expr), Attrs::empty(), span))
        }
        BuiltinFn::Link => {
            let target = take_string(&mut args);
            let label = take_inline(&mut args);
            Value::Inline(Inline::new(
                InlineKind::Link { target, label },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::Image => {
            let target = take_string(&mut args);
            let alt = take_string(&mut args);
            Value::Inline(Inline::new(
                InlineKind::Image { target, alt },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::Inlines => {
            let list = take_list(&mut args);
            let items = list
                .iter()
                .map(|v| match v {
                    Value::Inline(i) => i.clone(),
                    _ => unreachable!("checked List<Inline>"),
                })
                .collect();
            Value::Inline(Inline::fragment(items, Attrs::empty(), span))
        }
        BuiltinFn::Para => {
            let inner = take_inline(&mut args);
            Value::Block(Block::new(BlockKind::Paragraph(inner), Attrs::empty(), span))
        }
        BuiltinFn::Heading => {
            let level = take_int(&mut args).clamp(1, 6) as u8;
            let title = take_inline(&mut args);
            Value::Block(Block::new(
                BlockKind::Heading { level, title },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::Callout => {
            let kind = take_string(&mut args);
            let body = take_block(&mut args);
            Value::Block(Block::new(
                BlockKind::Callout { kind, body },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::CodeBlock => {
            let lang = take_string(&mut args);
            let text = take_string(&mut args);
            Value::Block(Block::new(
                BlockKind::CodeBlock { lang, text },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::DisplayMath => {
            let expr = take_string(&mut args);
            Value::Block(Block::new(BlockKind::Math(expr), Attrs::empty(), span))
        }
        BuiltinFn::Quote => {
            let body = take_block(&mut args);
            Value::Block(Block::new(BlockKind::Quote(body), Attrs::empty(), span))
        }
        BuiltinFn::Divider => Value::Block(Block::new(BlockKind::Divider, Attrs::empty(), span)),
        BuiltinFn::Table => {
            let raw_headers = take_list(&mut args);
            let raw_rows = take_list(&mut args);
            let headers: Vec<Inline> = raw_headers
                .iter()
                .map(|v| match v {
                    Value::Inline(i) => i.clone(),
                    _ => unreachable!("checked List<Inline>"),
                })
                .collect();
            let rows: Vec<Arc<[Inline]>> = raw_rows
                .iter()
                .map(|row_val| match row_val {
                    Value::List(cells) => Arc::from(
                        cells
                            .iter()
                            .map(|c| match c {
                                Value::Inline(i) => i.clone(),
                                _ => unreachable!("checked List<List<Inline>>"),
                            })
                            .collect::<Vec<_>>(),
                    ),
                    _ => unreachable!("checked List<List<Inline>>"),
                })
                .collect();
            Value::Block(Block::new(
                BlockKind::Table {
                    headers: Arc::from(headers),
                    rows: Arc::from(rows),
                },
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::Div => {
            let body = take_block(&mut args);
            Value::Block(Block::new(BlockKind::Div(body), Attrs::empty(), span))
        }
        BuiltinFn::Section => {
            let body = take_block(&mut args);
            Value::Block(Block::new(
                BlockKind::SectionBox(body),
                Attrs::empty(),
                span,
            ))
        }
        BuiltinFn::Blocks => {
            let list = take_list(&mut args);
            let items = list
                .iter()
                .map(|v| match v {
                    Value::Block(b) => b.clone(),
                    _ => unreachable!("checked List<Block>"),
                })
                .collect();
            Value::Block(Block::fragment(items, Attrs::empty(), span))
        }
        BuiltinFn::ReadJson => {
            let path = take_string(&mut args);
            let json = ctx
                .host
                .read_json(ctx.doc_path, &path)
                .map_err(ExecutionError::HostError)?;
            Value::Json(json)
        }
        BuiltinFn::IntToString => Value::String(Arc::from(take_int(&mut args).to_string())),
        BuiltinFn::FloatToString => Value::String(Arc::from(take_float(&mut args).to_string())),
        BuiltinFn::BoolToString => Value::String(Arc::from(take_bool(&mut args).to_string())),
        BuiltinFn::ListLen => Value::Int(take_list(&mut args).len() as i64),
        BuiltinFn::JsonGet => {
            let json = take_json(&mut args);
            let key = take_string(&mut args);
            Value::Json(json.get(&key))
        }
        BuiltinFn::JsonAt => {
            let json = take_json(&mut args);
            let index = take_int(&mut args);
            Value::Json(json.at(index))
        }
        BuiltinFn::JsonAsString => {
            let json = take_json(&mut args);
            let default = take_string(&mut args);
            Value::String(json.as_string(&default))
        }
        BuiltinFn::JsonAsInt => {
            let json = take_json(&mut args);
            let default = take_int(&mut args);
            Value::Int(json.as_int(default))
        }
        BuiltinFn::JsonAsBool => {
            let json = take_json(&mut args);
            let default = take_bool(&mut args);
            Value::Bool(json.as_bool(default))
        }
        BuiltinFn::JsonAsList => {
            let json = take_json(&mut args);
            let list = json.as_list();
            let values: Vec<Value> = list.iter().cloned().map(Value::Json).collect();
            Value::List(Arc::from(values))
        }
        BuiltinFn::JsonLen => {
            let json = take_json(&mut args);
            Value::Int(json.len())
        }
        BuiltinFn::JsonIsNull => {
            let json = take_json(&mut args);
            Value::Bool(json.is_null())
        }
        BuiltinFn::InlinePlainText => {
            let inline = take_inline(&mut args);
            Value::String(Arc::from(inline.plain_text()))
        }
        BuiltinFn::InlineWithAttr => {
            let inline = take_inline(&mut args);
            let key = take_string(&mut args);
            let value = take_string(&mut args);
            Value::Inline(inline.with_attr(key, AttrValue::String(value)))
        }
        BuiltinFn::InlineAttrOr => {
            let inline = take_inline(&mut args);
            let key = take_string(&mut args);
            let fallback = take_string(&mut args);
            let found = inline
                .node()
                .attrs
                .get(&key)
                .map(attr_to_arc_str)
                .unwrap_or(fallback);
            Value::String(found)
        }
        BuiltinFn::InlineHasAttr => {
            let inline = take_inline(&mut args);
            let key = take_string(&mut args);
            Value::Bool(inline.node().attrs.get(&key).is_some())
        }
        BuiltinFn::InlineRewrite => {
            let inline = take_inline(&mut args);
            let func = args.next().expect("checked rewrite callback");
            let rewritten = inline.rewrite_bottom_up(&mut |node| {
                match call_value(ctx, func.clone(), vec![Value::Inline(node)], stack, depth + 1)? {
                    Value::Inline(out) => Ok(out),
                    _ => unreachable!("checked inline rewrite return"),
                }
            })?;
            Value::Inline(rewritten)
        }
        BuiltinFn::BlockPlainText => {
            let block = take_block(&mut args);
            Value::String(Arc::from(block.plain_text()))
        }
        BuiltinFn::BlockSectionize => {
            let block = take_block(&mut args);
            Value::Block(block.sectionize())
        }
        BuiltinFn::BlockHeadingCount => {
            let block = take_block(&mut args);
            Value::Int(block.headings().len() as i64)
        }
        BuiltinFn::BlockHeadingsSummary => {
            let block = take_block(&mut args);
            let summary = block
                .headings()
                .into_iter()
                .map(|(level, title)| format!("h{level}: {title}"))
                .collect::<Vec<_>>()
                .join(" | ");
            Value::String(Arc::from(summary))
        }
        BuiltinFn::BlockWithAttr => {
            let block = take_block(&mut args);
            let key = take_string(&mut args);
            let value = take_string(&mut args);
            Value::Block(block.with_attr(key, AttrValue::String(value)))
        }
        BuiltinFn::BlockAttrOr => {
            let block = take_block(&mut args);
            let key = take_string(&mut args);
            let fallback = take_string(&mut args);
            let found = block
                .node()
                .attrs
                .get(&key)
                .map(attr_to_arc_str)
                .unwrap_or(fallback);
            Value::String(found)
        }
        BuiltinFn::BlockHasAttr => {
            let block = take_block(&mut args);
            let key = take_string(&mut args);
            Value::Bool(block.node().attrs.get(&key).is_some())
        }
        BuiltinFn::BlockRewriteInlines => {
            let block = take_block(&mut args);
            let func = args.next().expect("checked rewrite callback");
            let rewritten = block.rewrite_inlines(&mut |node| {
                match call_value(ctx, func.clone(), vec![Value::Inline(node)], stack, depth + 1)? {
                    Value::Inline(out) => Ok(out),
                    _ => unreachable!("checked inline rewrite return"),
                }
            })?;
            Value::Block(rewritten)
        }
        BuiltinFn::BlockRewriteBlocks => {
            let block = take_block(&mut args);
            let func = args.next().expect("checked rewrite callback");
            let rewritten = block.rewrite_blocks(&mut |node| {
                match call_value(ctx, func.clone(), vec![Value::Block(node)], stack, depth + 1)? {
                    Value::Block(out) => Ok(out),
                    _ => unreachable!("checked block rewrite return"),
                }
            })?;
            Value::Block(rewritten)
        }
    })
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Inline(x), Value::Inline(y)) => x == y,
        (Value::Block(x), Value::Block(y)) => x == y,
        (Value::Json(x), Value::Json(y)) => x == y,
        (Value::List(xs), Value::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| value_eq(x, y))
        }
        _ => false,
    }
}

fn eval_binary(op: BinaryOp, left: Value, right: Value) -> Result<Value, ExecutionError> {
    use BinaryOp as Op;
    match (op, left, right) {
        (Op::Add, Value::Int(a), Value::Int(b)) => a
            .checked_add(b)
            .map(Value::Int)
            .ok_or(ExecutionError::IntegerOverflow),
        (Op::Sub, Value::Int(a), Value::Int(b)) => a
            .checked_sub(b)
            .map(Value::Int)
            .ok_or(ExecutionError::IntegerOverflow),
        (Op::Mul, Value::Int(a), Value::Int(b)) => a
            .checked_mul(b)
            .map(Value::Int)
            .ok_or(ExecutionError::IntegerOverflow),
        (Op::Div, Value::Int(_), Value::Int(0)) => Err(ExecutionError::DivisionByZero),
        (Op::Div, Value::Int(a), Value::Int(b)) => a
            .checked_div(b)
            .map(Value::Int)
            .ok_or(ExecutionError::IntegerOverflow),
        (Op::Rem, Value::Int(_), Value::Int(0)) => Err(ExecutionError::DivisionByZero),
        (Op::Rem, Value::Int(a), Value::Int(b)) => a
            .checked_rem(b)
            .map(Value::Int)
            .ok_or(ExecutionError::IntegerOverflow),

        (Op::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Op::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (Op::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (Op::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (Op::Rem, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),

        (Op::Add, Value::String(a), Value::String(b)) => {
            if a.is_empty() {
                return Ok(Value::String(b));
            }
            if b.is_empty() {
                return Ok(Value::String(a));
            }
            let mut joined = String::with_capacity(a.len() + b.len());
            joined.push_str(&a);
            joined.push_str(&b);
            Ok(Value::String(Arc::from(joined)))
        }
        (Op::Add, Value::Inline(a), Value::Inline(b)) => {
            let span = a.node().span;
            Ok(Value::Inline(Inline::concat(a, b, span)))
        }
        (Op::Add, Value::Block(a), Value::Block(b)) => {
            let span = a.node().span;
            Ok(Value::Block(Block::concat(a, b, span)))
        }
        (Op::Add, Value::List(a), Value::List(b)) => {
            if a.is_empty() {
                return Ok(Value::List(b));
            }
            if b.is_empty() {
                return Ok(Value::List(a));
            }
            let mut joined = Vec::with_capacity(a.len() + b.len());
            joined.extend(a.iter().cloned());
            joined.extend(b.iter().cloned());
            Ok(Value::List(Arc::from(joined)))
        }

        (Op::Eq, a, b) => Ok(Value::Bool(value_eq(&a, &b))),
        (Op::Ne, a, b) => Ok(Value::Bool(!value_eq(&a, &b))),

        (Op::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Op::Le, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (Op::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (Op::Ge, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),

        (Op::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (Op::Le, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
        (Op::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (Op::Ge, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),

        (Op::Lt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a < b)),
        (Op::Le, Value::String(a), Value::String(b)) => Ok(Value::Bool(a <= b)),
        (Op::Gt, Value::String(a), Value::String(b)) => Ok(Value::Bool(a > b)),
        (Op::Ge, Value::String(a), Value::String(b)) => Ok(Value::Bool(a >= b)),

        _ => unreachable!("checked binary operands"),
    }
}
