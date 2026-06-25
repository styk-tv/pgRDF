"""Runtime configuration — read from env vars, with sensible defaults for local-dev."""
from __future__ import annotations

import os

# ---------- compute pg (the in-container pg+pgRDF+AGE) ----------
PG_HOST = os.environ.get("PG_HOST", "localhost")
PG_PORT = int(os.environ.get("PG_PORT", "5432"))
PG_DB   = os.environ.get("PG_DB",   "postgres")
PG_USER = os.environ.get("PG_USER", "postgres")
PG_PASS = os.environ.get("PG_PASS", "postgres")

# ---------- project identity ----------
# Each ACA deployment is one project; injected at deploy time by new-project.sh.
PROJECT_SLUG = os.environ.get("PROJECT_SLUG", "local-dev")
TEAM_ID      = os.environ.get("TEAM_ID",      "local-dev-team")

# ---------- static SPA ----------
# Default resolves to the repo's app/static; override via env in container.
_DEFAULT_STATIC = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "app", "static")
if not os.path.isdir(_DEFAULT_STATIC):
    _DEFAULT_STATIC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "static")
STATIC_DIR = os.environ.get("STATIC_DIR", _DEFAULT_STATIC)
UI_SUBDIR  = "ui"   # served at /ui/

# ---------- auth (consumed by app.auth) ----------
KC_ISSUER     = os.environ.get("KC_ISSUER",     "https://id.tech.games/realms/techgames")
KC_AUDIENCES  = os.environ.get("KC_AUDIENCES",  "ck-web,account")
KC_CLIENT_ID  = os.environ.get("KC_CLIENT_ID",  "ck-web")
