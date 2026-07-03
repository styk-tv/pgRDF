//! Bag-equivalent result comparison with blank-node isomorphism.
//!
//! `engine` rows come from pgRDF (via run.sh); `oracle` rows come from
//! this binary's `eval`. The relation is asymmetric on purpose: only
//! the oracle side carries explicit blank-node markers (`"_:label"` in
//! flat rows, `{"type": "bnode"}` in structured rows) because the
//! engine's flat SELECT output renders blank nodes as bare labels.

use serde_json::Value;
use std::collections::HashMap;

/// Blank-node bijection: oracle label -> engine value, plus the
/// inverse image to enforce injectivity. Cloned per matching attempt
/// so backtracking never needs rollback.
#[derive(Clone, Default)]
struct BnodeMap {
    fwd: HashMap<String, String>,
    inv: HashMap<String, String>,
}

impl BnodeMap {
    fn bind(&mut self, label: &str, value: &str) -> bool {
        match self.fwd.get(label) {
            Some(bound) => bound == value,
            None => {
                if self.inv.contains_key(value) {
                    return false; // two labels may not share one image
                }
                self.fwd.insert(label.to_string(), value.to_string());
                self.inv.insert(value.to_string(), label.to_string());
                true
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Match,
    Diverge { detail: String },
}

/// Compare two result bags. Row order is irrelevant on both sides;
/// multiplicity matters (bag, not set, semantics).
pub fn compare(engine: &[Value], oracle: &[Value]) -> Verdict {
    if engine.len() != oracle.len() {
        return Verdict::Diverge {
            detail: format!(
                "row count: engine {} vs oracle {}",
                engine.len(),
                oracle.len()
            ),
        };
    }
    let mut used = vec![false; engine.len()];
    if assign(0, oracle, engine, &mut used, &BnodeMap::default()) {
        Verdict::Match
    } else {
        Verdict::Diverge {
            detail: "no bag-equivalent row assignment exists (up to bnode isomorphism)".to_string(),
        }
    }
}

/// Backtracking perfect matching: give every oracle row a distinct
/// engine row it agrees with, under one globally consistent blank-node
/// bijection. Fixture result sets are small, so exponential worst case
/// is irrelevant here.
fn assign(i: usize, oracle: &[Value], engine: &[Value], used: &mut [bool], map: &BnodeMap) -> bool {
    if i == oracle.len() {
        return true;
    }
    for j in 0..engine.len() {
        if used[j] {
            continue;
        }
        let mut attempt = map.clone();
        if row_match(&engine[j], &oracle[i], &mut attempt) {
            used[j] = true;
            if assign(i + 1, oracle, engine, used, &attempt) {
                return true;
            }
            used[j] = false;
        }
    }
    false
}

fn row_match(engine: &Value, oracle: &Value, map: &mut BnodeMap) -> bool {
    match (engine, oracle) {
        (Value::Object(e), Value::Object(o)) => {
            if e.len() != o.len() {
                return false;
            }
            o.iter().all(|(key, ov)| match e.get(key) {
                Some(ev) => term_match(ev, ov, map),
                None => false,
            })
        }
        _ => engine == oracle,
    }
}

fn term_match(engine: &Value, oracle: &Value, map: &mut BnodeMap) -> bool {
    match (engine, oracle) {
        (Value::String(es), Value::String(os)) => {
            if let Some(label) = os.strip_prefix("_:") {
                map.bind(label, es)
            } else {
                es == os || numeric_eq(es, os)
            }
        }
        // Structured term objects (CONSTRUCT/DESCRIBE rows):
        // {"type": "iri"|"literal"|"bnode", "value": …,
        //  "datatype"?: …, "language"?: …}
        (Value::Object(e), Value::Object(o)) => {
            let (Some(et), Some(ot)) = (str_field(e, "type"), str_field(o, "type")) else {
                return engine == oracle;
            };
            if et != ot {
                return false;
            }
            if et == "bnode" {
                return match (str_field(e, "value"), str_field(o, "value")) {
                    (Some(ev), Some(ov)) => map.bind(ov, ev),
                    _ => false,
                };
            }
            if e.len() != o.len() {
                return false;
            }
            o.iter().all(|(key, ov)| match (e.get(key), ov) {
                // BCP 47 language tags are case-insensitive.
                (Some(Value::String(ev)), Value::String(ovs)) if key == "language" => {
                    ev.eq_ignore_ascii_case(ovs)
                }
                (Some(ev), _) => ev == ov,
                (None, _) => false,
            })
        }
        _ => engine == oracle,
    }
}

fn str_field<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

/// Lexical-form-insensitive numeric equality: both sides must parse
/// fully as finite numbers. Guards against "01a"-style near-numbers.
fn numeric_eq(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.is_finite() && y.is_finite() && x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rows(v: Vec<Value>) -> Vec<Value> {
        v
    }

    #[test]
    fn identical_flat_rows_match() {
        let e = rows(vec![json!({"s": "http://ex.com/a", "n": "Alice"})]);
        let o = rows(vec![json!({"s": "http://ex.com/a", "n": "Alice"})]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn bag_order_insensitive() {
        let e = rows(vec![json!({"n": "Alice"}), json!({"n": "Bob"})]);
        let o = rows(vec![json!({"n": "Bob"}), json!({"n": "Alice"})]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn multiplicity_mismatch_diverges() {
        // Same set, different bag: {A, A, B} vs {A, B, B}.
        let e = rows(vec![
            json!({"n": "A"}),
            json!({"n": "A"}),
            json!({"n": "B"}),
        ]);
        let o = rows(vec![
            json!({"n": "A"}),
            json!({"n": "B"}),
            json!({"n": "B"}),
        ]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn row_count_mismatch_diverges() {
        let e = rows(vec![json!({"n": "A"})]);
        let o = rows(vec![json!({"n": "A"}), json!({"n": "B"})]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn differing_value_diverges() {
        let e = rows(vec![json!({"n": "Alice"})]);
        let o = rows(vec![json!({"n": "Bob"})]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn differing_key_sets_diverge() {
        let e = rows(vec![json!({"n": "Alice"})]);
        let o = rows(vec![json!({"m": "Alice"})]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn numeric_lexical_forms_equivalent() {
        // Engine derives values through Postgres numeric, the oracle
        // through XSD arithmetic — lexical forms may differ while the
        // number is the same.
        let e = rows(vec![json!({"a": "01", "b": "2.50", "c": "1e0"})]);
        let o = rows(vec![json!({"a": "1", "b": "2.5", "c": "1"})]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn numeric_equivalence_requires_full_parse() {
        // "01a" is not a number; must not match "1a" numerically.
        let e = rows(vec![json!({"a": "01a"})]);
        let o = rows(vec![json!({"a": "1a"})]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn different_numbers_diverge() {
        let e = rows(vec![json!({"a": "1"})]);
        let o = rows(vec![json!({"a": "1.5"})]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn bnode_bijection_flat_consistent() {
        // Oracle marks blank nodes "_:label"; engine renders bare
        // labels. Same oracle label must map to the same engine value
        // across ALL rows.
        let o = rows(vec![
            json!({"x": "_:a", "y": "1"}),
            json!({"x": "_:a", "y": "2"}),
        ]);
        let e = rows(vec![
            json!({"x": "b1_1", "y": "1"}),
            json!({"x": "b1_1", "y": "2"}),
        ]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn bnode_bijection_flat_inconsistent_diverges() {
        let o = rows(vec![
            json!({"x": "_:a", "y": "1"}),
            json!({"x": "_:a", "y": "2"}),
        ]);
        let e = rows(vec![
            json!({"x": "b1_1", "y": "1"}),
            json!({"x": "b1_2", "y": "2"}),
        ]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn bnode_mapping_is_injective() {
        // Two distinct oracle labels may not collapse onto one engine
        // value.
        let o = rows(vec![
            json!({"x": "_:a", "y": "1"}),
            json!({"x": "_:b", "y": "2"}),
        ]);
        let e = rows(vec![
            json!({"x": "b1_1", "y": "1"}),
            json!({"x": "b1_1", "y": "2"}),
        ]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn structured_bnode_bijection() {
        // CONSTRUCT rows carry structured terms; bnodes are explicit
        // via "type", labels differ between engine and oracle.
        let o = rows(vec![json!({
            "subject": {"type": "bnode", "value": "a"},
            "predicate": {"type": "iri", "value": "http://ex.com/p"},
            "object": {"type": "literal", "value": "x",
                        "datatype": "http://www.w3.org/2001/XMLSchema#string"},
        })]);
        let e = rows(vec![json!({
            "subject": {"type": "bnode", "value": "b1_1"},
            "predicate": {"type": "iri", "value": "http://ex.com/p"},
            "object": {"type": "literal", "value": "x",
                        "datatype": "http://www.w3.org/2001/XMLSchema#string"},
        })]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn structured_lang_tag_case_insensitive() {
        // BCP 47 tags are case-insensitive; engines may differ.
        let o = rows(vec![json!({
            "object": {"type": "literal", "value": "Le Widget",
                        "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                        "language": "FR"},
        })]);
        let e = rows(vec![json!({
            "object": {"type": "literal", "value": "Le Widget",
                        "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                        "language": "fr"},
        })]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }

    #[test]
    fn structured_term_type_mismatch_diverges() {
        let o = rows(vec![json!({
            "object": {"type": "iri", "value": "http://ex.com/x"},
        })]);
        let e = rows(vec![json!({
            "object": {"type": "literal", "value": "http://ex.com/x",
                        "datatype": "http://www.w3.org/2001/XMLSchema#string"},
        })]);
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn structured_bnode_bijection_is_global_across_positions() {
        // The same oracle bnode appearing as object in one row and
        // subject in another must map to ONE engine label.
        let o = rows(vec![
            json!({
                "subject": {"type": "iri", "value": "http://ex.com/s"},
                "predicate": {"type": "iri", "value": "http://ex.com/p"},
                "object": {"type": "bnode", "value": "n"},
            }),
            json!({
                "subject": {"type": "bnode", "value": "n"},
                "predicate": {"type": "iri", "value": "http://ex.com/q"},
                "object": {"type": "iri", "value": "http://ex.com/o"},
            }),
        ]);
        let e = rows(vec![
            json!({
                "subject": {"type": "iri", "value": "http://ex.com/s"},
                "predicate": {"type": "iri", "value": "http://ex.com/p"},
                "object": {"type": "bnode", "value": "b9"},
            }),
            json!({
                "subject": {"type": "bnode", "value": "b7"},
                "predicate": {"type": "iri", "value": "http://ex.com/q"},
                "object": {"type": "iri", "value": "http://ex.com/o"},
            }),
        ]);
        // b9 != b7: the shared oracle node cannot map consistently.
        assert!(matches!(compare(&e, &o), Verdict::Diverge { .. }));
    }

    #[test]
    fn bnode_bijection_two_labels_swap() {
        // Isomorphism, not label equality: the assignment that works
        // requires trying row orders — {_:a->b2, _:b->b1}.
        let o = rows(vec![
            json!({"x": "_:a", "y": "_:b"}),
            json!({"x": "_:b", "y": "_:a"}),
        ]);
        let e = rows(vec![
            json!({"x": "b2", "y": "b1"}),
            json!({"x": "b1", "y": "b2"}),
        ]);
        assert_eq!(compare(&e, &o), Verdict::Match);
    }
}
