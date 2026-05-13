# pg_regress-style golden tests

Each `.sql` under `sql/` is executed against a freshly-installed
extension. Output is captured and diffed against the corresponding
`.out` under `expected/`. Unexpected diffs fail CI.

## Layout

    tests/regression/
    ├── sql/
    │   └── 00-smoke.sql           (TODO Phase 1)
    └── expected/
        └── 00-smoke.out           (TODO Phase 1)

## Naming

`NN-<topic>.sql` where NN preserves run order. By convention:

- `00-09`   — smoke + schema
- `10-29`   — ingestion + dictionary
- `30-59`   — query
- `60-79`   — inference
- `80-99`   — validation

## Authoring a new regression test

1. Write `sql/<n>-<topic>.sql`.
2. Run the harness; it produces `out/<n>-<topic>.out`.
3. Inspect the output. If correct, `mv out/*.out expected/`. Commit.
4. Subsequent runs compare against the new expected.

When a test legitimately changes (e.g. you fix a bug and the output
needs to update), re-accept the same way. The diff in the PR is the
audit trail.
