# 23-min-max-numeric

W3C SPARQL 1.1 §17.4 — `MIN` / `MAX` use the value's **natural
order** — numeric for `xsd:numeric` literals, lexicographic
otherwise.

The fixture's four `xsd:integer` literals sort lexicographically
as `"10" < "100" < "2" < "20"` — completely wrong if the aggregate
naïvely uses the textual lexical_value. pgRDF's translator detects
the numeric datatype path and aggregates over the NUMERIC cast,
giving the correct numeric extreme.

Hand-computed expected output:

```jsonl
{"lo": "2", "hi": "100"}
```

Implementation: `translate_aggregate` for `MIN`/`MAX` emits
`COALESCE(MIN(numeric)::text, MIN(lex))` — when any row has a
numeric datatype the numeric MIN wins; for pure-string groups
the numeric MIN is NULL and COALESCE falls back to lex MIN.
Mixed-type groups (both numeric and string literals) prefer the
numeric MIN; the SPARQL spec leaves mixed-type ordering
implementation-defined.
