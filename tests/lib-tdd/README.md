# lib-tdd — the RED suite for SPEC.pgRDF.LIB.v0.6.34

Executable form of the LIB/TDD spec line. Written **before** the emissions it tests
(E0–E7); cases are expected to fail — the point is that they fail **for the stated
reason**, and flip GREEN only when the emission actually lands.

## Three-state semantics (M1)

| Exit | State | Meaning |
|---|---|---|
| 0 | **GREEN** | the target behaviour is met |
| 44 | **RED** | fails today *exactly as predicted* — documents the unimplemented emission. A RED is a suite PASS. |
| other | **BROKEN** | fails for an **unstated** reason — the engine moved, or the prediction was wrong. The only failing state. |

`run.sh` exits non-zero **iff any case is BROKEN**. RED→GREEN = an emission landed
(move the ledger row below). GREEN→RED or anything→BROKEN = a regression: stop and look.

## Running

```
tests/lib-tdd/run.sh                 # all cases (container-exec defaults, same env
                                     # knobs as tests/regression/run.sh)
tests/lib-tdd/run.sh 01-e0-lock-sqlstate     # one case
PGRDF_TDD_PSQL="psql …" tests/lib-tdd/run.sh # any bench via full psql command
```

No bench route is embedded in the suite. Cases create only `urn:tdd:*` graphs and drop
them on exit; all are re-runnable and safe to run concurrently with other work.

## Case catalog

State = the ledger as of the E0/T-NULL cut (branch `lib-0-6-34-emissions`).
Every RED asserts its *specific* current failure (exact SQLSTATE, exact hash, exact
delta), so a stale prediction surfaces as BROKEN, never as a silent pass.

| Case | Obligation (spec id) | What it does | Covered by (in-engine) | State |
|---|---|---|---|---|
| `01-e0-lock-sqlstate` | C1-1/E0 | locks a graph, provokes `clear_graph`, asserts SQLSTATE `55P03` | `lock.rs` gate via `crate::refuse`; `#[pg_test] lock_refusal_carries_lock_not_available` (enum assert) | **GREEN** |
| `02-e0-algebra-sqlstate` | C1-1/E0 | runs a SERVICE query, asserts `0A000` | `executor.rs` both algebra catch-alls; `#[pg_test] unsupported_algebra_carries_feature_not_supported` | **GREEN** |
| `03-e0-validate-mode-sqlstate` | C1-1/E0 | `validate(mode=>'nonsense')`, asserts `22023` | `shacl.rs` mode gate; `#[pg_test] validate_unknown_mode_carries_invalid_parameter_value` + pre-existing message-pinned test | **GREEN** |
| `04-e0-truncation-fail-closed-sqlstate` | C1-1/E0 | 70-hop chain, `on_path_truncation='error'`, asserts `54000` | `executor.rs` Error arm; `#[pg_test] truncation_error_mode_carries_program_limit_exceeded` + message-pinned sibling | **GREEN** |
| `05-tnull-digest-absent-refuses` | T-NULL-1 (L9) | `graph_digest(<absent id>)`, asserts `42704` — never sha256("") | `canon.rs` registry check; `#[pg_test] graph_digest_absent_graph_refuses_undefined_object` | **GREEN** |
| `06-tnull-empty-vs-absent` | T-NULL-2 (L9) | empty graph digests (an answer), absent refuses — distinguishable | same gate; `#[pg_test] graph_digest_empty_graph_still_digests` | **GREEN** |
| `07-c2-delta-cross-session-pollution` | C2-2/E2 | session A truncates between session B's two counter reads; B's delta=1 having run nothing; probes for a per-call figure | — (E2 unimplemented; the same race flakes `property_path_one_or_more_depth_guard_bumps_stat` under parallel `cargo test`) | RED |
| `08-k6-fd1-collision-pair` | K6-1/E6 | loads the 8-triple pair (bnode 4-cycle vs two 2-cycles, non-isomorphic, fd1-colliding); expects `structural_digest` to exist AND collide. Tripwire: if a future fd1 *separates* the pair, the case goes BROKEN — that algorithm would not be fleet-fd1 | — (E6 fd1 half unimplemented) | RED |
| `09-k6-rdfc-separates-pair` | K6-2 (RET) | same pair; `graph_digest` must separate it conclusively, inside the complexity budget | `canon.rs` RDFC-1.0 (shipped 0.6.32) | **GREEN** |
| `10-e1-inventory-surface` | C3-1/E1-1 | probes candidate inventory-verb names; the day one exists it auto-upgrades to a Q1 parity check against `_pgrdf_graphs` | — (E1 unimplemented) | RED |
| `11-e5-surface-queryable` | E5-1 | probes `pgrdf.surface()`; when present, asserts it covers every `pg_proc` export | — (E5 emission unimplemented) | RED |
| `12-e5-manifest-coverage` | E5-2/K9 | regenerates the live surface (`gen-surface.sh`, unit = `(proname, identity_args)`) and diffs both directions against `classification.tsv` | the manifest itself (DRAFT — operator review owed) | **GREEN** |
| `13-k11-lock-cure-works` | K11-1 (RET) | provokes the lock refusal, extracts the cure it names, EXECUTES that cure, asserts the condition resolves | `lock.rs` message contract (`unlock_graph(id, reason)`) | **GREEN** |
| `14-tmsg-no-debug-dump` | T-MSG-1 | asserts the algebra refusal names the construct (SERVICE) and carries no `NamedNode { … }` debris | `algebra_construct_name()` in `executor.rs` | **GREEN** |
| `15-bench-identity-triple` | T-BENCH-TRIPLE (RET) | `version == extversion`, `build_id` populated, not `-dirty` — the stale-`.so` catcher (caught a pre-tag artifact on the compose bench's first boot) | `version()`/`build_id()`/catalog | RED while iterating (dirty tree — the honest state); GREEN on any clean-tree delivery |

## Coverage map — what tests what

Each landed emission is covered **twice, on purpose**:

- **In-engine** (`#[pg_test]`, runs in `just test-native` / `just test`): asserts the
  `PgSqlErrorCode` **by enum** via `PgTryBuilder` — the mechanism. Five such negative
  controls exist (cases 01–06's gates), plus the pre-existing message-pinned tests,
  which still pass because every reclassified message is byte-identical (K2).
- **On-the-wire** (this suite, through psql): asserts the five-char SQLSTATE a real
  client receives — the emission. This is the plane the LIB spec is *about*; SPI-side
  green with wire-side red would mean the code never left the backend.

All reclassified gates route through one door: `crate::refuse(code, msg)` in
`src/lib.rs` — ERROR-level report on the identical panic/unwind path as `pgrx::error!`
(pgrx converts ERROR reports into Rust panics), `#[track_caller]` so LOCATION still
names the gate site. Genuine invariant breaks stay `panic!` = `XX000`, deliberately.

## Support files

```
surface/gen-surface.sh      live surface dump from pg_proc (name, args, kind, vol, strict)
surface/classification.tsv  DRAFT stable/internal/spike classification, all 54 exports
gates/gen-gates.sh          mechanical census of deliberate panic-gates
gates/gates.tsv             the snapshot: 56 rows = the E0 sweep worklist (4 done)
```
