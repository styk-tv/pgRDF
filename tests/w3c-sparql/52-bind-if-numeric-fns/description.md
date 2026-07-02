# 52 — IF + numeric functions in projection expressions (issue #51 tier 1)

SPARQL 1.1 §17.4.1.2 `IF(cond, then, else)` and §17.4.4 numeric
functions (`ROUND`, `ABS`) as projection expressions. The `gizmo`
row locks the XPath `fn:round` half-toward-positive-infinity rule:
`ROUND(-3.5)` is `-3`, where a naive Postgres `round()` lowering
(half-away-from-zero) would produce `-4`. W3C-oracle-eligible:
standard SPARQL 1.1 only, deterministic values.
