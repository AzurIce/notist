use std::collections::HashMap;

/// An item can be represented as a function call of `name(params)`
pub struct Item {
    name: String,
    params: Dict,
}

pub struct Dict {
    inner: HashMap<String, Value>,
}

pub enum Value {
    String(String),
    Int(i32),
    Float(f64),
    Bool(bool),
    Dict(Dict),
    Item(Item),
    Unit,
}

pub struct Node {
    attr: Dict,
}
