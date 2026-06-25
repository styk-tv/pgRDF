"""Public API metadata. Lists the v1 endpoints, no auth required."""
from __future__ import annotations

from fastapi import APIRouter

from ..config import PROJECT_SLUG

router = APIRouter()


@router.get("/api")
def api_meta():
    return {
        "name": "project-azure-pgRDF",
        "version": "0.1.0",
        "project": PROJECT_SLUG,
        "endpoints": [
            "/health",
            "/api",
            "/ui",
            "/api/v1/graphs",
            "/api/v1/ontology/classes",
            "/api/v1/ontology/predicates",
            "/api/v1/shapes",
            "/api/v1/rules",
            "/api/v1/queries/saved",
            "/api/v1/queries/history",
            "/api/v1/query",
            "/api/v1/validate",
            "/api/v1/materialize",
        ],
    }
