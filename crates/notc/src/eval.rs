use crate::ast::{BinaryOp, UnaryOp};
use crate::check::{CheckedExpr, CheckedProgram, FunctionId};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Function(FunctionValue),
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
}

impl CheckedProgram {
    /// Execute a zero-argument entry in a fresh session.
    pub fn run(&self, entry: &str) -> Result<Value, ExecutionError> {
        let id = *self
            .function_names
            .get(entry)
            .ok_or_else(|| ExecutionError::UnknownEntry(entry.into()))?;
        let function = &self.functions[id.0];
        if function.arity != 0 {
            return Err(ExecutionError::EntryRequiresArguments(function.arity));
        }
        evaluate(self, &function.body, &mut Vec::new(), 0, 1)
    }
}

fn evaluate(
    program: &CheckedProgram,
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
        C::Let(initializer, body) => {
            let value = evaluate(program, initializer, stack, base, depth)?;
            stack.push(value);
            let result = evaluate(program, body, stack, base, depth);
            stack.pop();
            result
        }
        C::Seq(first, second) => {
            let _ = evaluate(program, first, stack, base, depth)?;
            evaluate(program, second, stack, base, depth)
        }
        C::Unary(op, operand) => {
            let value = evaluate(program, operand, stack, base, depth)?;
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
            let Value::Bool(left) = evaluate(program, left, stack, base, depth)? else {
                unreachable!("checked boolean operand")
            };
            if !left {
                return Ok(Value::Bool(false));
            }
            evaluate(program, right, stack, base, depth)
        }
        C::Binary(BinaryOp::Or, left, right) => {
            let Value::Bool(left) = evaluate(program, left, stack, base, depth)? else {
                unreachable!("checked boolean operand")
            };
            if left {
                return Ok(Value::Bool(true));
            }
            evaluate(program, right, stack, base, depth)
        }
        C::Binary(op, left, right) => {
            let left = evaluate(program, left, stack, base, depth)?;
            let right = evaluate(program, right, stack, base, depth)?;
            eval_binary(*op, left, right)
        }
        C::If(condition, yes, no) => {
            let Value::Bool(condition) = evaluate(program, condition, stack, base, depth)? else {
                unreachable!("checked condition")
            };
            evaluate(
                program,
                if condition { yes } else { no },
                stack,
                base,
                depth,
            )
        }
        C::Call(callee, arguments) => {
            const MAX_CALL_DEPTH: usize = 64;
            if depth >= MAX_CALL_DEPTH {
                return Err(ExecutionError::CallDepthExceeded);
            }
            let Value::Function(FunctionValue(id)) = evaluate(program, callee, stack, base, depth)?
            else {
                unreachable!("checked call target")
            };
            // Evaluate in the caller's frame before installing the callee frame.
            // Stage arguments separately so they cannot occupy caller let slots.
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate(program, argument, stack, base, depth)?);
            }
            let frame = stack.len();
            stack.extend(values);
            let result = evaluate(
                program,
                &program.functions[id.0].body,
                stack,
                frame,
                depth + 1,
            );
            stack.truncate(frame);
            result
        }
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

        (Op::Eq, Value::Unit, Value::Unit) => Ok(Value::Bool(true)),
        (Op::Ne, Value::Unit, Value::Unit) => Ok(Value::Bool(false)),
        (Op::Eq, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
        (Op::Ne, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a != b)),
        (Op::Eq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
        (Op::Ne, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a != b)),
        (Op::Eq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
        (Op::Ne, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a != b)),
        (Op::Eq, Value::String(a), Value::String(b)) => Ok(Value::Bool(a == b)),
        (Op::Ne, Value::String(a), Value::String(b)) => Ok(Value::Bool(a != b)),

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
