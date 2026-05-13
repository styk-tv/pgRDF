//! Wrapper around the `reasonable` OWL 2 RL reasoner.
//!
//! Flow: stream rows from `_pgrdf_quads` (graph_id filter) into the
//! reasoner, collect inferred triples, COPY BINARY back into the
//! quad table with `is_inferred = TRUE`.
//!
//! Scope note: `reasonable` implements OWL 2 RL only. OWL 2 EL/QL
//! and Datalog beyond RL are out of scope for v0.2.
