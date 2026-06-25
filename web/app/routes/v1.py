"""Stub /api/v1/* endpoints. v0.1: returns the same shape the UI's `data.jsx`
hard-codes, but driven by the FastAPI sidecar so the UI never talks to pg
directly. Real pg backing arrives in v0.2; for now the route shapes are
locked so the UI integrator can wire fetches in parallel.

Every endpoint expects a valid Bearer (enforced by the auth middleware).
Schema: returns are scoped to the current project (PROJECT_SLUG); team
scoping happens at the auth layer (the JWT must belong to the team that
owns the project, validated by request.state.user.team_id checks at
deploy time wiring).
"""
from __future__ import annotations

from fastapi import APIRouter, Body, HTTPException, Request

router = APIRouter(prefix="/api/v1")


# ---------- catalogue / inventory ----------

@router.get("/graphs")
def list_graphs(request: Request):
    return {
        "graphs": [
            {"id": 0, "name": "default",        "label": "default",         "triples": 0},
            {"id": 1, "name": "lubm:2024",      "label": "lubm:2024",       "triples": 0},
            {"id": 2, "name": "lubm:inferred",  "label": "lubm:inferred",   "triples": 0, "inferred": True},
        ],
    }


@router.get("/ontology/classes")
def list_classes(request: Request, graph: int = 0):
    return {"graph": graph, "classes": []}


@router.get("/ontology/predicates")
def list_predicates(request: Request, graph: int = 0):
    return {"graph": graph, "predicates": []}


@router.get("/shapes")
def list_shapes(request: Request, graph: int = 0):
    return {"graph": graph, "shapes": []}


@router.get("/rules")
def list_rules(request: Request, graph: int = 0):
    return {"graph": graph, "rules": []}


# ---------- queries ----------

@router.get("/queries/saved")
def saved_queries(request: Request):
    return {"queries": []}


@router.get("/queries/history")
def query_history(request: Request, limit: int = 50):
    return {"queries": []}


@router.post("/query")
def run_query(request: Request, body: dict = Body(...)):
    """POST {"sparql": "...", "graph": <id>}  →  {cols, rows, plan, logs, ms}.

    v0.1: refuses with 501. Real execution lands once pg.py is wired."""
    if "sparql" not in body:
        raise HTTPException(400, "missing required field: sparql")
    raise HTTPException(501, "SPARQL execution not yet wired in v0.1")


# ---------- validation / materialization ----------

@router.post("/validate")
def validate(request: Request, body: dict = Body(...)):
    """POST {"data_graph": <id>, "shapes_graph": <id>} → ValidationReport JSONB."""
    raise HTTPException(501, "SHACL validation not yet wired in v0.1")


@router.post("/materialize")
def materialize(request: Request, body: dict = Body(...)):
    """POST {"graph": <id>, "profile": "rdfs"|"owl2rl"} → {derived, ms}."""
    raise HTTPException(501, "materialization not yet wired in v0.1")


# ---------- ontology sync (LLD §4) ----------

@router.post("/ontology/sync")
def ontology_sync(request: Request, body: dict = Body(default={})):
    """Manually trigger the controller procs that regenerate AGE labels +
    JSONB CHECK constraints from the current ontology. Normally fires via
    statement-level trigger on ontology_triples; this endpoint is for
    debugging + initial bootstrap."""
    raise HTTPException(501, "ontology sync not yet wired in v0.1")
