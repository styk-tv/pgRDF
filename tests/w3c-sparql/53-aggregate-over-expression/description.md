# 53 — aggregate over an expression (issue #50)

SPARQL 1.1 §18.2.4.1 allows an arbitrary expression as the aggregate
argument: `SUM(?price * ?qty)` per customer. Locks the expression
lowering through the numeric lane inside a grouped aggregate.
W3C-oracle-eligible: standard SPARQL 1.1, deterministic integer
arithmetic (no division, so no numeric-scale surprises in the
lexical form).
