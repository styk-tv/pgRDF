# tests/perf/lubm/generator — containerised UBA generator

The Lehigh SWAT UBA (Univ-Bench Artificial) data generator, sealed
in a docker image so it never touches the host JRE. Image name
`pgrdf-lubm-generator:latest` per the workstation `pgrdf-` prefix
discipline.

## Build (one time, ~1–2 min on first build, cached after)

```bash
just lubm-build
```

(Equivalent direct invocation: `docker build -t pgrdf-lubm-generator:latest tests/perf/lubm/generator/`.)

## Run

```bash
# Generate LUBM-10 into the docker named volume `pgrdf-lubm-data`.
just lubm-gen 10
```

Equivalent direct invocation:

```bash
docker run --rm \
  -v pgrdf-lubm-data:/data \
  pgrdf-lubm-generator:latest \
  10
```

Output:

```
/data/lubm-10/
  ├── raw/  → University0_0.owl, University0_1.owl, … (UBA output)
  ├── nt/   → lubm-10.nt        (concatenated N-Triples, ~1.3 M lines)
  └── ttl/  → lubm-10.ttl       (compact Turtle form)
```

## Why a container?

- The user's workstation runs zero host Java by policy. UBA is a
  Java tool; this image is the only way it runs here.
- The Dockerfile pins the UBA tarball URL + sha256, the
  `eclipse-temurin:17-jre-jammy` base, and the raptor2-utils
  conversion path. Image is reproducible from the tracked source.
- Output lives in a docker named volume (`pgrdf-lubm-data`) — not
  on the host filesystem — so cleanup is `docker volume rm` and
  there's no accidental git pollution.

## Why raptor (rapper) for conversion?

UBA emits RDF/XML serialised with the `.owl` file extension. We
convert to N-Triples (line-stable; trivial for `pgrdf.parse_turtle`
to ingest) and Turtle (compact, useful for human review). `rapper`
from the raptor2-utils Debian package is the standard CLI for
this; pulling oxigraph/oxrdfio into the generator image is
overkill for a one-shot conversion.

## Upstream provenance

- UBA source: https://swat.cse.lehigh.edu/projects/lubm/
- Paper: Y. Guo, Z. Pan, J. Heflin. "LUBM: A Benchmark for OWL
  Knowledge Base Systems." *Journal of Web Semantics* 3(2-3),
  2005.

## Out of scope here

- LUBM-100, LUBM-1000 — same image, higher `univ_count` argument.
  Localhost-bound regardless; no hosted-runner planning per the
  [[lubm-localhost-only]] memory.
- Cross-engine timing (vs. Apache Jena TDB, Apache AGE) — separate
  follow-up; this directory only generates and ingests the
  dataset.
