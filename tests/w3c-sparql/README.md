# W3C SPARQL 1.1 conformance harness

This directory hosts the runner that drives the W3C SPARQL 1.1 test
suite against pgRDF. The suite itself lives at https://github.com/w3c/rdf-tests
and is pulled in as a git submodule once the runner is in.

## Layout (target)

    tests/w3c-sparql/
    ├── README.md                  (this file)
    ├── manifest_runner.rs         (Phase 2)
    └── fixtures/                  (submodule: w3c/rdf-tests, sparse-checked to sparql/sparql11/)

## Manifest format

W3C tests use RDF/Turtle manifests (`manifest.ttl`) declaring
`mf:Manifest` with `mf:entries` lists. Each entry references:

- `mf:action` → the SPARQL query file
- `mf:result` → the expected SPARQL Results XML / JSON
- `qt:data`   → input RDF graph (Turtle/N-Triples)

The runner parses the manifest, iterates entries, executes the query
through `pgrdf.sparql()`, and diffs results against expected.

## Coverage gates per phase

See [docs/10-roadmap.md](../../docs/10-roadmap.md):
- Phase 2: ≥ 30% pass
- Phase 3: ≥ 70% pass
- Phase 4: ≥ 95% pass

## Running locally (once the runner ships)

    git submodule update --init tests/w3c-sparql/fixtures
    cargo run -p pgrdf-w3c-sparql -- tests/w3c-sparql/fixtures/sparql11/manifest.ttl
