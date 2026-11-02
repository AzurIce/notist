//! Provenance-free selected-subtree snapshots: a bounded prefix token stream.
//! Counts describe structure, not internal arena ids or a resource capability.
use super::*;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, SeqAccess, Visitor},
    ser::SerializeSeq,
};

#[derive(Serialize, Deserialize)]
enum Token {
    Null,
    Bool(bool),
    Int(i64),
    Float(#[serde(with = "crate::item::float_serde")] f64),
    String(Text),
    Array(usize),
    Object(#[serde(deserialize_with = "object_keys")] Vec<Text>),
}
fn object_keys<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Text>, D::Error> {
    struct Keys;
    impl<'de> Visitor<'de> for Keys {
        type Value = Vec<Text>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("bounded object keys")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let limits = DataLimits::default();
            let mut keys = Vec::new();
            let mut bytes = 0usize;
            while let Some(key) = seq.next_element::<Text>()? {
                bytes = bytes.saturating_add(key.len()).saturating_add(8);
                if keys.len() >= limits.nodes || bytes > limits.value_bytes {
                    return Err(A::Error::custom("data wire object limit exceeded"));
                }
                keys.push(key);
            }
            Ok(keys)
        }
    }
    deserializer.deserialize_seq(Keys)
}
impl Serialize for Data {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.check_limits(DataLimits::default())
            .map_err(serde::ser::Error::custom)?;
        fn write<S: SerializeSeq>(data: &Data, seq: &mut S) -> Result<(), S::Error> {
            let token = match &data.current().kind {
                NodeKind::Null => Token::Null,
                NodeKind::Bool(v) => Token::Bool(*v),
                NodeKind::Int(v) => Token::Int(*v),
                NodeKind::Float(v) => Token::Float(*v),
                NodeKind::String(v) => Token::String(v.clone()),
                NodeKind::Array(v) => Token::Array(v.len()),
                NodeKind::Object { entries, .. } => {
                    Token::Object(entries.iter().map(|(k, _)| k.clone()).collect())
                }
            };
            seq.serialize_element(&token)?;
            match &data.current().kind {
                NodeKind::Array(v) => {
                    for n in v {
                        write(&data.child(*n), seq)?;
                    }
                }
                NodeKind::Object { entries, .. } => {
                    for (_, n) in entries {
                        write(&data.child(*n), seq)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut seq = serializer.serialize_seq(Some(self.node_count()))?;
        write(self, &mut seq)?;
        seq.end()
    }
}
impl<'de> Deserialize<'de> for Data {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Snapshot;
        impl<'de> Visitor<'de> for Snapshot {
            type Value = Data;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded Data subtree token stream")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Data, A::Error> {
                let limits = DataLimits::default();
                let mut tokens = Vec::new();
                let mut bytes = 0usize;
                while let Some(token) = seq.next_element::<Token>()? {
                    if tokens.len() >= limits.nodes {
                        return Err(A::Error::custom("data wire node limit exceeded"));
                    }
                    bytes = bytes.saturating_add(match &token {
                        Token::String(s) => s.len(),
                        Token::Object(keys) => {
                            if keys.len() > limits.nodes {
                                return Err(A::Error::custom("data wire object limit exceeded"));
                            }
                            keys.iter()
                                .fold(0usize, |n, k| n.saturating_add(k.len() + 8))
                        }
                        Token::Array(n) => {
                            if *n > limits.nodes {
                                return Err(A::Error::custom("data wire array limit exceeded"));
                            }
                            n.saturating_mul(8)
                        }
                        _ => 8,
                    });
                    if bytes > limits.value_bytes {
                        return Err(A::Error::custom("data wire size limit exceeded"));
                    }
                    tokens.push(token);
                }
                fn build(
                    tokens: &mut std::vec::IntoIter<Token>,
                    builder: &mut Builder,
                    depth: usize,
                ) -> Result<usize, DataFailure> {
                    if depth >= builder.limits.depth {
                        return Err(DataFailure::Limit("data wire depth limit exceeded"));
                    }
                    let bad =
                        || DataError::new(DataErrorKind::Syntax, "invalid Data subtree snapshot");
                    let kind = match tokens.next().ok_or_else(bad)? {
                        Token::Null => NodeKind::Null,
                        Token::Bool(v) => NodeKind::Bool(v),
                        Token::Int(v) => NodeKind::Int(v),
                        Token::Float(v) if v.is_finite() => NodeKind::Float(v),
                        Token::Float(_) => return Err(bad().into()),
                        Token::String(v) => NodeKind::String(v),
                        Token::Array(n) => NodeKind::Array(
                            (0..n)
                                .map(|_| build(tokens, builder, depth + 1))
                                .collect::<Result<_, _>>()?,
                        ),
                        Token::Object(keys) => {
                            let mut entries = Vec::new();
                            let mut index = BTreeMap::new();
                            for key in keys {
                                if index.contains_key(&key) {
                                    return Err(bad().into());
                                }
                                let n = build(tokens, builder, depth + 1)?;
                                index.insert(key.clone(), entries.len());
                                entries.push((key, n));
                            }
                            NodeKind::Object { entries, index }
                        }
                    };
                    builder.add(kind, None)
                }
                let mut tokens = tokens.into_iter();
                let mut builder = Builder {
                    nodes: Vec::new(),
                    text: None,
                    limits,
                    bytes: 0,
                };
                let root = build(&mut tokens, &mut builder, 0).map_err(A::Error::custom)?;
                if tokens.next().is_some() {
                    return Err(A::Error::custom("trailing Data snapshot nodes"));
                }
                Ok(builder.finish(root))
            }
        }
        deserializer.deserialize_seq(Snapshot)
    }
}

#[derive(Serialize, Deserialize)]
struct ErrorWire {
    kind: DataErrorKind,
    message: Text,
    path: Text,
    expected: Option<Text>,
    actual: Option<Text>,
}
impl Serialize for DataError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.logical_size() > DataLimits::default().value_bytes {
            return Err(serde::ser::Error::custom(
                "data error wire size limit exceeded",
            ));
        }
        ErrorWire {
            kind: self.kind(),
            message: self.detail.message.clone(),
            path: self.detail.path.clone(),
            expected: self.detail.expected.clone(),
            actual: self.detail.actual.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for DataError {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ErrorWire::deserialize(deserializer)?;
        let error = Self {
            detail: Arc::new(ErrorDetail {
                kind: wire.kind,
                message: wire.message,
                path: wire.path,
                expected: wire.expected,
                actual: wire.actual,
                span: None,
            }),
            text: None,
            source: None,
        };
        if error.logical_size() > DataLimits::default().value_bytes {
            return Err(D::Error::custom("data error wire size limit exceeded"));
        }
        Ok(error)
    }
}
