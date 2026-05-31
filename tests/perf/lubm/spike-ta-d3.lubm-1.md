# TA-D3 spike — batched dict resolution (2-pass)

**Status:** spike completed v0.5.27. Validates theory (-17% e2e,
-30% dict, 250× fewer SPI calls). Decision deferred to TA-D1
after TA-D2 (shmem pre-warm) lands.

## What this spike implements

A new ingest path `pgrdf.parse_turtle_dict_batched(content,
graph_id, base_iri, dict_batch_size)` (also `load_turtle_dict_batched`
for server-side path source) that replaces the per-term SPI
roundtrip in `put_term_full` with **bulk dict resolution**:

1. **Parse pass** — walk the rio Turtle iterator once,
   materializing every Triple into `Vec<oxrdf::Triple>` and
   collecting unique `(term_type, lexical, datatype_id, language)`
   tuples into a HashSet.
2. **Datatype-IRI pre-resolve** — datatype IRIs need a dict id
   themselves before literals can be keyed. One bulk-resolve pass
   handles them first.
3. **Bulk resolve remaining terms** — chunks of `dict_batch_size`
   (default 500) go through new `dict::put_terms_batch(terms)` →
   2 SPI calls per chunk: bulk `INSERT … ON CONFLICT DO NOTHING`
   then bulk `JOIN _pgrdf_dictionary` to read back ids in input
   order via `WITH ORDINALITY`.
4. **Insert pass** — walk materialized triples, look up s/p/o
   ids from the now-hot HashMap, build s/p/o arrays, `flush_batch`
   per `BATCH_SIZE` (existing prepared INSERT path; identical to
   baseline).

## Side-by-side numbers (LUBM-1 / pg17 / Apple-Silicon / Colima)

Measured against the same `compose/extensions/`-mounted
`pgrdf.so` build on 2026-05-31. Same docker host. Same ingest
file. Different graph per pass (to avoid quad-row dedup).

| metric              | baseline (parse_turtle) | spike (parse_turtle_dict_batched) | delta |
|---------------------|-------------------------|------------------------------------|-------|
| triples             | 100,573                 | 100,573                            | ✅ identical |
| elapsed_ms          | 1,452.36                | 1,200.04                           | **-17.4%** |
| dict_ms             | 1,059.27                | 738.54                             | **-30.2%** |
| parse_ms            | 73.69                   | 85.48                              | +16.0% |
| insert_ms           | 311.61                  | 376.02                             | +20.7% |
| **dict_db_calls**   | **26,473**              | **106**                            | **-99.6% (250× reduction)** |
| dict_cache_hits     | 308,325                 | 275,247                            | — (metric semantics differ between paths; see notes) |
| shmem_cache_hits    | 0                       | 0                                  | — (TA-D2 covers warming) |
| quad_batches        | 101                     | 101                                | ✅ identical |

## Interpretation

**The theory validates.** Batching dict resolution into 2 SPI
calls per chunk eliminates the per-term roundtrip cost — SPI
calls drop from 26,473 to 106 (a 250× reduction). The 30% dict
phase speedup translates to a 17% total ingest speedup at
LUBM-1 scale.

**But the win is smaller than Phase-0 predicted.** Phase-0
estimated 60% total savings on the theory that dict_ms could
drop to near-zero. Actual: dict_ms stayed at 739ms (-30%, not
-90%+). Three sources of residual cost in the 2-pass design:

1. **Phase-1 parse_ms increased** (+16%) — the parse step now
   ALSO walks every triple to collect unique term keys (HashSet
   inserts + String clones).
2. **Phase-3 insert_ms increased** (+21%) — the second walk over
   the materialized triples does HashMap lookups + owned String
   key construction for s/p/o per triple, which the baseline
   single-pass avoided.
3. **Bulk dict resolve still does work** — PostgreSQL's UNNEST +
   JOIN with 4 columns × 500 rows per batch has real cost; not
   zero. dict_db_calls=106 = 53 chunks × 2 SPI calls.

## Implications for TA-D1 + TA-9 decision

TA-D3 is a **net win** (-17% e2e at LUBM-1). The production
landing (TA-7 winner) should NOT just lift this 2-pass shape —
it should:

- **One-pass version** — accumulate uncached terms into a
  deferred queue; flush the queue + the corresponding quad batch
  together when either fills. Avoids the 2-pass overhead.
- **Reuse HashMap allocations** across iterations (current spike
  recreates DictKey tuples).
- **Investigate residual dict_ms** — 739ms for 106 SPI calls is
  ~7ms per call (much higher than the single-row 42µs). The
  bulk-INSERT-ON-CONFLICT + bulk-JOIN approach has measurable PG
  cost that may be tunable.

## Companion: TA-D2 (next spike)

TA-D3 measured shmem_cache_hits=0 on a cold-cache run. TA-D2
spike pre-warms the shmem cache from `_pgrdf_dictionary` at
extension boot. Expected effect: dict_db_calls drops further
(repeat ingests over the same corpus stop hitting SPI).
Combined with TA-D3, the dict phase could approach the
sub-100ms range.

## Companion: TA-11 (heap_multi_insert) + TA-10 (COPY BINARY)

Targets the `insert_ms` phase (376ms in this spike, 312ms in
baseline). Independent of TA-D3 — could combine. TA-9 decision
considers all four.
