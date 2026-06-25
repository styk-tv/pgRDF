"""Public liveness probe. Returns a small JSON; no auth required."""
from __future__ import annotations

from datetime import datetime, timezone

from fastapi import APIRouter

from ..config import PROJECT_SLUG

router = APIRouter()


@router.get("/health")
def health():
    return {
        "ok": True,
        "ts": datetime.now(timezone.utc).isoformat(timespec="milliseconds"),
        "project": PROJECT_SLUG,
    }
