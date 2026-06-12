# pgRDF — Roadmap (retired)

> **This standalone roadmap is retired** (last content was current to v0.5.1).
> It drifted from the shipped state and is superseded by the live sources below.
> Retained as a pointer so existing links don't break.

**Current status & capability** → [`README.md`](README.md) — the status row,
the LUBM-100 milestone, and the per-engine surface, kept current at every cut.

**Per-release record** → [`CHANGELOG.md`](CHANGELOG.md) — the running, dated
log; the head of every release.

**Engineering history (phase + slice tracking)** →
[`docs/10-roadmap.md`](docs/10-roadmap.md).

**Contracts** → [`specs/SPEC.pgRDF.LLD.v0.x.md`](specs/) and the forward look
[`specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md`](specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md).

**Benchmark methodology** →
[`specs/SPEC.pgRDF.BENCH.v0.6.0.md`](specs/SPEC.pgRDF.BENCH.v0.6.0.md).

As of v0.6.x: pgRDF passes the **full LUBM-100 benchmark** (13.9M triples, all
14 queries ≤ 5 s across plain and OWL-RL-materialized profiles, zero tuning).
See the README milestone for the headline and the BENCH spec for how it's run.
