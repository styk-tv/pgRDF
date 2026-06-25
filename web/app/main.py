"""FastAPI app for project-azure-pgRDF.

Wires:
  - auth middleware (Bearer JWT against id.tech.games/realms/techgames)
  - /health and /api (public)
  - /api/v1/* (protected stubs — return shapes the UI expects)
  - /ui/ (static SPA — React 18 + Babel-standalone, no build step)
"""
from __future__ import annotations

from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse
from fastapi.staticfiles import StaticFiles

from .auth import auth_middleware
from .config import STATIC_DIR, UI_SUBDIR
from .registry import registry
from .routes import api as api_meta, health as health_route, projects as projects_route, v1


app = FastAPI(
    title="project-azure-pgRDF",
    version="0.2.0",
    docs_url=None,  # keep the Swagger UI off the public surface in v0.1
    redoc_url=None,
)


@app.on_event("startup")
async def _startup():
    n = await registry.refresh()
    import logging
    logging.getLogger("startup").info("registry: loaded %d project(s)", n)


@app.on_event("shutdown")
async def _shutdown():
    await registry.close()

# CORS — for SPA dev against a separate origin. In prod the SPA is same-origin
# under /ui/, but local-dev with a vite proxy benefits from permissive CORS.
app.add_middleware(
    CORSMiddleware,
    allow_origins=["http://localhost:5173", "http://localhost:3000"],
    allow_credentials=False,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Dev: never cache the /ui SPA assets so .jsx edits show on reload.
# (In cloud we'll flip this to immutable + content-hash filenames.)
@app.middleware("http")
async def _no_cache_ui(request, call_next):
    resp = await call_next(request)
    if request.url.path.startswith("/ui"):
        resp.headers["Cache-Control"] = "no-store, must-revalidate"
    return resp


# Bearer middleware — runs AFTER CORS so OPTIONS preflights bypass it.
app.middleware("http")(auth_middleware)

# Public routes
app.include_router(health_route.router)
app.include_router(api_meta.router)

# Project-scoped routes — own tier-aware auth, bypass global middleware
app.include_router(projects_route.router)

# Protected v1 (auth enforced by middleware)  — legacy global surface, not
# per-project; kept for back-compat with the v0.1 UI stubs.
app.include_router(v1.router)


# Static SPA mounted at /ui/ — never at / (avoids the asset-gateway-style
# root 404 confusion the user flagged). Visiting / 302s to /ui/.
ui_dir = Path(STATIC_DIR) / UI_SUBDIR
if ui_dir.is_dir():
    app.mount("/ui", StaticFiles(directory=str(ui_dir), html=True), name="ui")


@app.get("/")
def root_redirect():
    return RedirectResponse(url="/ui/", status_code=302)
