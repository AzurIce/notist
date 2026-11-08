//! Shared document IR, declaration contracts and diagnostics. No source or resource IO.
pub mod annotation;
pub mod builtins;
pub mod data;
pub mod definition_identity;
pub mod definitions;
pub mod diag;
pub mod dump;
pub mod embed;
pub mod evaluation;
pub mod expr;
pub mod extensions;
pub mod frontend;
mod identity;
pub mod index;
pub mod inputs;
pub mod item;
pub mod registry;
pub mod resource;
pub mod shared;
pub mod source;
pub mod source_chain;
#[cfg(feature = "serde")]
pub mod source_wire;
