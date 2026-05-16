-- 115-aggregate-over-union.sql
--
-- Phase F group F2 (slices 30-27, LLD v0.4 §11) — aggregates over
-- UNION. The v0.3 limitation: `SELECT (COUNT(?x) AS ?n) WHERE {
-- {…} UNION {…} }` panicked ("aggregates on top of UNION not
-- supported yet"). F2 ships a derived-table refactor: each UNION
-- branch becomes a sub-SELECT projecting the aggregate / GROUP BY
-- variables' dict ids into the F1 `vK` derived-column pool; the
-- branches `UNION ALL` into `(<union>) qU`; the EXISTING aggregate
-- translator runs over `qU` unchanged. SPARQL UNION is multiset
-- union of solution sequences (the v0.3 UNION path already uses
-- `UNION ALL`); the aggregate runs over those rows.
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   H. COUNT(?x) over a 2-branch UNION, no GROUP BY.
--   I. SUM / AVG over UNION with GROUP BY a union var.
--   J. COUNT(DISTINCT ?x) over UNION; COUNT(*) over UNION.
--   K. type-aware MIN/MAX + GROUP_CONCAT + SAMPLE over UNION.
--   L. HAVING over the aggregate-of-union.
--   M. 3-branch UNION; a branch containing a property path (?x p+ ?y).
--   N. Under GRAPH scoping; inherited by pgrdf.construct.
--   O. LLD §11 acceptance: sparql_parse no longer flags
--      aggregates-over-UNION (gap-8 retired in 80-unsupported);
--      DESCRIBE still listed (F3); OPTIONAL/VALUES still absent (F1).

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- All primary fixture lives in the DEFAULT graph (id 0) so the
-- union counts are deterministic (an unscoped BGP scans ALL graphs;
-- the named-graph rows below are reachable only via GRAPH scoping).
--   Category "books":  i1 price 10, i2 price 12, i3 price 18
--   Category "tools":  t1 price 5,  t2 price 25
--   chain: c1 ex:next c2 ; c2 ex:next c3            (for the M `+` path)
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
     ex:i1 ex:cat "books" ; ex:price 10 .
     ex:i2 ex:cat "books" ; ex:price 12 .
     ex:i3 ex:cat "books" ; ex:price 18 .
     ex:t1 ex:cat "tools" ; ex:price 5 .
     ex:t2 ex:cat "tools" ; ex:price 25 .
     ex:c1 ex:next ex:c2 .
     ex:c2 ex:next ex:c3 .',
    0);
  PERFORM pgrdf.add_graph('http://example.com/g/a');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:ga ex:tag "x" .
     ex:gb ex:tag "y" .
     ex:gc ex:tag "x" .',
    pgrdf.graph_id('http://example.com/g/a'));
END $$;

-- H. COUNT(?p) over a 2-branch UNION, no GROUP BY. Branch 1 binds
-- ?p to books prices (3 rows: 10,12,18); branch 2 to tools prices
-- (2 rows: 5,25). UNION ALL = 5 rows → COUNT = 5.
SELECT (sparql->>'n')::int AS h_count_union
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(?p) AS ?n) WHERE {
       { ?x ex:cat "books" . ?x ex:price ?p }
       UNION
       { ?x ex:cat "tools" . ?x ex:price ?p } }'
  ) AS sparql;

-- I. SUM and AVG over UNION with GROUP BY the union var ?c (the
-- category). Branch1 = books rows (?c="books"): 10+12+18=40, n=3,
-- avg=40/3. Branch2 = tools (?c="tools"): 5+25=30, n=2, avg=15.
-- Two groups. Check the books SUM and the tools AVG.
SELECT (sparql->>'s')::int AS i_sum_books
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (SUM(?p) AS ?s) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c'
  ) AS sparql
  WHERE sparql->>'c' = 'books';
SELECT round((sparql->>'a')::numeric, 2) AS i_avg_tools
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (AVG(?p) AS ?a) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c'
  ) AS sparql
  WHERE sparql->>'c' = 'tools';
-- I also: number of GROUP BY groups = 2.
SELECT count(*)::int AS i_group_count
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (SUM(?p) AS ?s) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c'
  ) AS sparql;

-- J. COUNT(DISTINCT ?c) over UNION — ?c takes "books" (3x) and
-- "tools" (2x) → 5 rows, 2 distinct → 2. And COUNT(*) over the
-- same union → 5.
SELECT (sparql->>'d')::int AS j_count_distinct
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(DISTINCT ?c) AS ?d) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }'
  ) AS sparql;
SELECT (sparql->>'n')::int AS j_count_star
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(*) AS ?n) WHERE {
       { ?x ex:cat "books" . ?x ex:price ?p }
       UNION
       { ?x ex:cat "tools" . ?x ex:price ?p } }'
  ) AS sparql;

-- K. type-aware MIN/MAX + GROUP_CONCAT + SAMPLE over UNION. All 5
-- prices in one union: 10,12,18 ∪ 5,25. MIN=5 (numeric order, not
-- lexicographic "10"<"12"<"18"<"25"<"5"), MAX=25. GROUP_CONCAT of
-- the category strings is order-undefined; instead assert its
-- length-equivalent via COUNT and check MIN/MAX numerically. SAMPLE
-- returns one of the values deterministically (our MIN surrogate).
SELECT (sparql->>'mn')::int AS k_min_price
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (MIN(?p) AS ?mn) WHERE {
       { ?x ex:cat "books" . ?x ex:price ?p }
       UNION
       { ?x ex:cat "tools" . ?x ex:price ?p } }'
  ) AS sparql;
SELECT (sparql->>'mx')::int AS k_max_price
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (MAX(?p) AS ?mx) WHERE {
       { ?x ex:cat "books" . ?x ex:price ?p }
       UNION
       { ?x ex:cat "tools" . ?x ex:price ?p } }'
  ) AS sparql;
-- GROUP_CONCAT over the union of category strings; SEPARATOR ",".
-- The 5-row union has 3×"books" + 2×"tools"; sorted-char check via
-- length: "books,books,books,tools,tools" = 29 chars. (Order within
-- string_agg is unspecified by SPARQL but our planner is stable;
-- pin the LENGTH which is order-invariant.)
SELECT length(sparql->>'g') AS k_groupconcat_len
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (GROUP_CONCAT(?c ; SEPARATOR = ",") AS ?g) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }'
  ) AS sparql;
-- SAMPLE returns a single value present in the union; assert it is
-- one of the two categories.
SELECT ((sparql->>'sv') IN ('books','tools')) AS k_sample_member
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (SAMPLE(?c) AS ?sv) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }'
  ) AS sparql;

-- L. HAVING over the aggregate-of-union. GROUP BY ?c, keep groups
-- whose COUNT(?p) > 2. books has 3 (>2 → kept); tools has 2 (not
-- >2 → dropped). → 1 surviving group, ?c="books".
SELECT count(*)::int AS l_having_groups
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (COUNT(?p) AS ?n) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c HAVING(COUNT(?p) > 2)'
  );
SELECT (sparql->>'c')::text AS l_having_group
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (COUNT(?p) AS ?n) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c HAVING(COUNT(?p) > 2)'
  ) AS sparql;

-- M. 3-branch UNION; one branch contains a `+` property path.
-- Branch1: books prices (3 rows). Branch2: tools prices (2 rows).
-- Branch3: ?x ex:next+ ?y — transitive closure of c1→c2→c3:
--   c1→c2, c1→c3, c2→c3 = 3 solution rows.
-- COUNT(*) over the 3-branch union = 3 + 2 + 3 = 8.
SELECT (sparql->>'n')::int AS m_three_branch_path
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(*) AS ?n) WHERE {
       { ?x ex:cat "books" . ?x ex:price ?p }
       UNION
       { ?x ex:cat "tools" . ?x ex:price ?p }
       UNION
       { ?x ex:next+ ?y } }'
  ) AS sparql;

-- N. aggregate-over-UNION under GRAPH scoping. Named graph g/a has
-- ga tag "x", gb tag "y", gc tag "x". Union of {tag "x"} and
-- {tag "y"} branches, both scoped to g/a. COUNT = 2 (x: ga,gc) +
-- 1 (y: gb) = 3.
SELECT (sparql->>'n')::int AS n_graph_scoped_count
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(?s) AS ?n) WHERE {
       GRAPH <http://example.com/g/a> {
         { ?s ex:tag "x" } UNION { ?s ex:tag "y" } } }'
  ) AS sparql;

-- N note: CONSTRUCT cannot carry an aggregate at all (LLD §6.2 —
-- a `Group` in a CONSTRUCT WHERE is rejected by
-- `reject_construct_modifiers`), so "aggregate-over-UNION inherited
-- by pgrdf.construct" is vacuous by spec. The meaningful F2
-- inheritance for the SELECT surface is the GRAPH-scoped
-- aggregate-over-UNION above (`n_graph_scoped_count`). Plain
-- CONSTRUCT-over-UNION (no aggregate) is a separate, still-unshipped
-- construct-path surface tracked independently of F2.

-- O. LLD §11 acceptance: pgrdf.sparql_parse no longer flags
-- aggregates-over-UNION in `unsupported_algebra` (the parser always
-- walked Group/Union; the gate was the executor panic, retired in
-- 80-unsupported-shapes gap-8). Lock the empty array. DESCRIBE
-- still reports supported=false (F3). OPTIONAL/VALUES not regressed
-- (F1) — empty unsupported for an OPTIONAL+VALUES query too.
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT (COUNT(?p) AS ?n) WHERE {
     { ?x ex:cat "books" . ?x ex:price ?p }
     UNION { ?x ex:cat "tools" . ?x ex:price ?p } }'
)->'unsupported_algebra' AS o_aggunion_unsupported;
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/i1>')->>'supported'
  AS o_describe_still_unsupported;
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?x ?y WHERE { ?x ex:cat ?y
     OPTIONAL { ?x ex:price ?p } } VALUES (?y) { ("books") }'
)->'unsupported_algebra' AS o_f1_not_regressed;

ROLLBACK;
