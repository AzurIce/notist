use crate::check::{CheckedExpr, CheckedProgram, FunctionId};

#[derive(Debug, Clone, Copy)]
pub enum Value {
    Int(i64),
    Bool(bool),
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
        C::Int(n) => Ok(Value::Int(*n)),
        C::Bool(b) => Ok(Value::Bool(*b)),
        C::Local(slot) => Ok(stack[base + slot]),
        C::Function(id) => Ok(Value::Function(FunctionValue(*id))),
        C::Let(initializer, body) => {
            let value = evaluate(program, initializer, stack, base, depth)?;
            stack.push(value);
            let result = evaluate(program, body, stack, base, depth);
            stack.pop();
            result
        }
        C::Add(left, right) => {
            let Value::Int(left) = evaluate(program, left, stack, base, depth)? else {
                unreachable!("checked integer operand")
            };
            let Value::Int(right) = evaluate(program, right, stack, base, depth)? else {
                unreachable!("checked integer operand")
            };
            left.checked_add(right)
                .map(Value::Int)
                .ok_or(ExecutionError::IntegerOverflow)
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
