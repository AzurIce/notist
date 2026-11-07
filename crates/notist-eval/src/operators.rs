//! Scalar operations shared by compile-time and runtime evaluation.
use notist_core::{definitions::ComptimeValue, item::Value};
pub fn unary(op: &str, value: &ComptimeValue) -> Result<ComptimeValue, String> {
    let value = match (op, value) {
        ("!", ComptimeValue::Data(Value::Bool(v))) => Value::Bool(!v),
        ("-", ComptimeValue::Data(Value::Int(v))) => {
            Value::Int(v.checked_neg().ok_or("integer overflow")?)
        }
        ("-", ComptimeValue::Data(Value::Float(v))) => Value::Float(-v),
        _ => return Err("invalid unary operator".into()),
    };
    Ok(ComptimeValue::Data(value))
}
pub fn binary(op: &str, a: &ComptimeValue, b: &ComptimeValue) -> Result<ComptimeValue, String> {
    use ComptimeValue::{Data as D, Type as T};
    let boolean = |v| Ok(D(Value::Bool(v)));
    if let (T(a), T(b)) = (a, b) {
        return match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Type only supports equality".into()),
        };
    }
    match (a, b) {
        (D(Value::Int(a)), D(Value::Int(b))) => {
            let value = match op {
                "+" => a.checked_add(*b),
                "-" => a.checked_sub(*b),
                "*" => a.checked_mul(*b),
                "/" => a.checked_div(*b),
                "%" => a.checked_rem(*b),
                "==" => return boolean(a == b),
                "!=" => return boolean(a != b),
                "<" => return boolean(a < b),
                "<=" => return boolean(a <= b),
                ">" => return boolean(a > b),
                ">=" => return boolean(a >= b),
                _ => return Err("invalid integer operator".into()),
            };
            Ok(D(Value::Int(
                value.ok_or("integer overflow or division by zero")?,
            )))
        }
        (D(Value::Float(a)), D(Value::Float(b))) => Ok(D(match op {
            "+" => Value::Float(a + b),
            "-" => Value::Float(a - b),
            "*" => Value::Float(a * b),
            "/" => Value::Float(a / b),
            "%" => Value::Float(a % b),
            "==" => Value::Bool(a == b),
            "!=" => Value::Bool(a != b),
            "<" => Value::Bool(a < b),
            "<=" => Value::Bool(a <= b),
            ">" => Value::Bool(a > b),
            ">=" => Value::Bool(a >= b),
            _ => return Err("invalid float operator".into()),
        })),
        (D(Value::Str(a)), D(Value::Str(b))) => match op {
            "+" => {
                if a.len().saturating_add(b.len()) > 16 * 1024 * 1024 {
                    return Err("value size limit exceeded".into());
                }
                Ok(D(Value::Str(format!("{a}{b}").into())))
            }
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            "<" => boolean(a < b),
            "<=" => boolean(a <= b),
            ">" => boolean(a > b),
            ">=" => boolean(a >= b),
            _ => Err("invalid string operator".into()),
        },
        (D(Value::Bool(a)), D(Value::Bool(b))) => match op {
            "&&" => boolean(*a && *b),
            "||" => boolean(*a || *b),
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("invalid Bool operator".into()),
        },
        (D(Value::Path(a)), D(Value::Path(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Path only supports equality".into()),
        },
        (D(Value::Some(a)), D(Value::Some(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::None), D(Value::None)) => match op {
            "==" => boolean(true),
            "!=" => boolean(false),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::Some(_)), D(Value::None)) | (D(Value::None), D(Value::Some(_))) => match op {
            "==" => boolean(false),
            "!=" => boolean(true),
            _ => Err("Option only supports equality".into()),
        },
        (D(Value::Ok(a)), D(Value::Ok(b))) | (D(Value::Err(a)), D(Value::Err(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Result only supports equality".into()),
        },
        (D(Value::Ok(_)), D(Value::Err(_))) | (D(Value::Err(_)), D(Value::Ok(_))) => match op {
            "==" => boolean(false),
            "!=" => boolean(true),
            _ => Err("Result only supports equality".into()),
        },
        (D(Value::Data(a)), D(Value::Data(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("Data only supports equality".into()),
        },
        (D(Value::DataError(a)), D(Value::DataError(b))) => match op {
            "==" => boolean(a == b),
            "!=" => boolean(a != b),
            _ => Err("DataError only supports equality".into()),
        },
        _ => Err("operator requires matching scalar operands".into()),
    }
}
