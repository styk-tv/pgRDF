//! SPARQL parser wrapper.
//!
//! Wraps `spargebra::Query::parse` and exposes the resulting algebra
//! AST to the executor. We deliberately keep this layer thin so we
//! can swap spargebra versions without touching the translator.
