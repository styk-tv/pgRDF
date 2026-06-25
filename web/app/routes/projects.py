"""Project-scoped routes mounted at /p/{slug}/...

Tier semantics (read from the registry per request):
  - public    : no Bearer required; only GET endpoints exposed
  - protected : Bearer required (any valid token)
  - secure    : Bearer required AND request.state.user.team_id matches
                projects.team_id

Public listing (GET /p) is itself public — useful as a discovery surface so
the marketplace front-page can list every public project on this cluster.
"""
from __future__ import annotations

import json
import logging

from fastapi import APIRouter, Body, HTTPException, Request

from ..auth import _validate_token, team_id_of
from ..registry import ProjectCfg, registry

log = logging.getLogger("routes.projects")

router = APIRouter()

# Common namespace → short prefix, for display-friendly IRIs in the Console.
PREFIXES = {
    "rdf":  "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "rdfs": "http://www.w3.org/2000/01/rdf-schema#",
    "owl":  "http://www.w3.org/2002/07/owl#",
    "xsd":  "http://www.w3.org/2001/XMLSchema#",
    "sh":   "http://www.w3.org/ns/shacl#",
    "foaf": "http://xmlns.com/foaf/0.1/",
    "ub":   "http://swat.cse.lehigh.edu/onto/univ-bench.owl#",
    "inst": "http://example.org/lubm/",
}


def _shorten(iri: str) -> str:
    for pfx, ns in PREFIXES.items():
        if iri.startswith(ns):
            return f"{pfx}:{iri[len(ns):]}"
    return iri


async def _sparql(conn, query: str) -> list[dict]:
    """Run pgrdf.sparql and return decoded JSON rows. pgRDF returns one
    JSON object per row in a column called `sparql`."""
    rows = await conn.fetch("SELECT pgrdf.sparql($1) AS r", query)
    out = []
    for r in rows:
        v = r["r"]
        out.append(json.loads(v) if isinstance(v, str) else v)
    return out


# ---------- discovery (always public) ----------

@router.get("/p")
def list_projects():
    """Discovery surface — returns every project's public-facing metadata.
    Tier == 'public' projects are included; protected/secure projects appear
    in this list too but with limited fields (no team_id). Used by the
    marketplace front-page."""
    out = []
    for p in registry.all():
        out.append({
            "slug":  p.slug,
            "tier":  p.tier,
            "mode":  p.mode,
            "title": p.title,
            "description": p.description,
            # team_id is intentionally NOT included here — only visible
            # through /p/{slug}/info once auth has cleared the tier gate.
        })
    return {"count": len(out), "projects": out}


# ---------- per-project gate ----------

async def _resolve_project(request: Request, slug: str) -> ProjectCfg:
    """Resolve a project's config and enforce tier-specific *authentication*.

    Authorization for read-vs-write is left to each endpoint (a public
    project still serves SPARQL SELECT over POST — the query endpoint
    blocks UPDATE keywords itself).

      public    : no Bearer required
      protected : valid Bearer required (any token)
      secure    : valid Bearer + team_id claim == project.team_id
    """
    cfg = registry.get(slug)
    if cfg is None:
        raise HTTPException(404, f"unknown project: {slug}")

    if cfg.tier == "public":
        return cfg

    # protected + secure require a valid Bearer.
    auth = request.headers.get("authorization") or request.headers.get("Authorization")
    if not auth or not auth.lower().startswith("bearer "):
        raise HTTPException(401, f"{cfg.tier} project requires Bearer token")
    token = auth.split(" ", 1)[1].strip()
    claims = _validate_token(token)
    request.state.user = claims

    if cfg.tier == "secure":
        tid = team_id_of(request)
        if tid != cfg.team_id:
            raise HTTPException(403, f"team_id mismatch (token has {tid!r}, project requires {cfg.team_id!r})")

    return cfg


# ---------- per-project endpoints ----------

@router.get("/p/{slug}/info")
async def project_info(request: Request, slug: str):
    cfg = await _resolve_project(request, slug)
    return {
        "slug":  cfg.slug,
        "tier":  cfg.tier,
        "mode":  cfg.mode,
        "title": cfg.title,
        "description": cfg.description,
        "pg_db_name": cfg.pg_db_name,
    }


@router.get("/p/{slug}/graphs")
async def project_graphs(request: Request, slug: str):
    cfg = await _resolve_project(request, slug)
    pool = await registry.pool(slug)
    async with pool.acquire() as conn:
        # pgRDF stores per-graph row counts in pgrdf._pgrdf_quads partitions.
        # For v0.2 stage 1 we just enumerate partitions — the count_quads()
        # helper lands when we wire it through.
        try:
            rows = await conn.fetch(
                """
                SELECT
                  inhrelid::regclass::text AS partition,
                  pg_get_partkeydef(parent.oid) AS parent_def
                FROM pg_inherits
                JOIN pg_class parent ON parent.oid = pg_inherits.inhparent
                WHERE parent.relname = '_pgrdf_quads'
                ORDER BY partition
                """
            )
        except Exception as e:
            log.warning("graphs lookup failed for %s: %s", slug, e)
            rows = []
    return {
        "project": cfg.slug,
        "graphs": [{"partition": r["partition"]} for r in rows],
    }


@router.get("/p/{slug}/pgrdf-version")
async def project_pgrdf_version(request: Request, slug: str):
    cfg = await _resolve_project(request, slug)
    pool = await registry.pool(slug)
    async with pool.acquire() as conn:
        v = await conn.fetchval("SELECT pgrdf.version()")
    return {"project": cfg.slug, "pgrdf_version": v}


@router.get("/p/{slug}/api/snapshot")
async def project_snapshot(request: Request, slug: str):
    """The shape the Console's window.PGRDF_DATA expects, but derived from
    LIVE pgRDF queries against this project's database. The UI merges this
    over its data.jsx fixtures so unfilled keys (Q_NODES, etc.) keep working."""
    cfg = await _resolve_project(request, slug)
    pool = await registry.pool(slug)
    async with pool.acquire() as conn:
        pgrdf_version = await conn.fetchval("SELECT pgrdf.version()")

        # GRAPHS — from _pgrdf_graphs + per-graph quad count
        graph_rows = await conn.fetch(
            "SELECT graph_id, iri FROM pgrdf._pgrdf_graphs ORDER BY graph_id"
        )
        graphs = []
        for g in graph_rows:
            n = await conn.fetchval("SELECT pgrdf.count_quads($1)", g["graph_id"])
            graphs.append({
                "id":      g["graph_id"],
                "name":    g["iri"],
                "label":   _shorten(g["iri"]),
                "triples": int(n or 0),
            })

        # CLASSES — instances per rdf:type + subClassOf lattice
        cls = await _sparql(conn,
            "SELECT ?class (COUNT(?s) AS ?n) WHERE { ?s a ?class } "
            "GROUP BY ?class ORDER BY DESC(?n)")
        sub = await _sparql(conn,
            "SELECT ?c ?super WHERE { ?c "
            "<http://www.w3.org/2000/01/rdf-schema#subClassOf> ?super }")
        sub_map = {r["c"]: r["super"] for r in sub if r.get("c")}
        classes = [{
            "iri":       _shorten(r["class"]),
            "instances": int(r.get("n", 0)),
            "subOf":     _shorten(sub_map[r["class"]]) if r["class"] in sub_map else None,
            "color":     "class",
        } for r in cls if r.get("class")]

        # PREDICATES — usage count per predicate
        preds = await _sparql(conn,
            "SELECT ?p (COUNT(*) AS ?n) WHERE { ?s ?p ?o } "
            "GROUP BY ?p ORDER BY DESC(?n)")
        predicates = [{
            "iri":   _shorten(r["p"]),
            "uses":  int(r.get("n", 0)),
            "domain": None, "range": None,
            "group": _shorten(r["p"]).split(":")[0] if ":" in _shorten(r["p"]) else "",
        } for r in preds if r.get("p")]

    total = sum(g["triples"] for g in graphs)
    return {
        "live": True,
        "project": cfg.slug,
        "pgrdf_version": pgrdf_version,
        "PREFIXES": PREFIXES,
        "GRAPHS": graphs,
        "CLASSES": classes,
        "PREDICATES": predicates,
        "SHAPES": [],
        "RULES": [],
        "SAVED": [],
        "HISTORY": [],
        "dbStats": {
            "triples": str(total),
            "inferred": "0",
            "dict": "—",
            "cache": "—",
        },
    }


@router.post("/p/{slug}/api/query")
async def project_query(request: Request, slug: str, body: dict = Body(...)):
    """Run a SPARQL SELECT against this project. Body: {"sparql": "..."}.

    The UI never builds SQL — it sends SPARQL text; pgRDF parses + plans it.
    No string interpolation into SQL: the query text is bound as a parameter
    to pgrdf.sparql($1)."""
    cfg = await _resolve_project(request, slug)
    sparql = (body or {}).get("sparql", "").strip()
    if not sparql:
        raise HTTPException(400, "missing required field: sparql")
    low = sparql.lower()
    # Public + protected: read-only SPARQL only. Block UPDATE forms.
    if cfg.tier in ("public", "protected") and any(
        kw in low for kw in ("insert ", "delete ", "drop ", "clear ", "load ", "create ")
    ):
        raise HTTPException(403, f"{cfg.tier} tier is read-only (SELECT/ASK/CONSTRUCT only)")
    pool = await registry.pool(slug)
    try:
        async with pool.acquire() as conn:
            rows = await _sparql(conn, sparql)
    except Exception as e:
        raise HTTPException(400, f"query failed: {str(e)[:300]}")
    cols = list(rows[0].keys()) if rows else []
    return {
        "project": cfg.slug,
        "cols": [{"v": c, "label": f"?{c}"} for c in cols],
        "rows": [[r.get(c) for c in cols] for r in rows],
        "count": len(rows),
    }
