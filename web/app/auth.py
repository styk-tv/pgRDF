"""Bearer-JWT validation against the cluster's Keycloak realm.

Mirrors the asset-gateway auth pattern: in-app PyJWT validation, JWKS cache
with refresh-on-unknown-kid, public-prefix exemption, ASGI middleware that
puts decoded claims on `request.state.user`.

We pivoted to in-app validation (not ACA Easy Auth) because Easy Auth's
`customOpenIdConnectProviders` only fully supports interactive cookie-based
auth — bearer-only mode (Return401 + token-store false) returns HTTP 500
on every Bearer, valid or not. Doing it in FastAPI gives us:

- correct behaviour on any token shape (clean 401 with a reason)
- one identity stack (the realm's JWKS)
- a validator we can swap to a different IdP per project later
"""
from __future__ import annotations

import json as _json
import os
import time
import urllib.request

import jwt
from fastapi import HTTPException, Request


KC_ISSUER = os.environ.get(
    "KC_ISSUER", "https://id.tech.games/realms/techgames"
).rstrip("/")

# Keycloak issues `aud: ["ck-web", "account"]` for the ck-web PKCE client;
# accept either so the same token works whether the audience mapper is on or not.
KC_AUDIENCES = [
    a.strip() for a in
    os.environ.get("KC_AUDIENCES", "ck-web,account").split(",")
    if a.strip()
]
KC_ALGS = [a.strip() for a in os.environ.get(
    "KC_ALGS", "RS256,EdDSA,RS384,RS512,ES256").split(",") if a.strip()]
JWKS_URL = f"{KC_ISSUER}/protocol/openid-connect/certs"

# Paths that bypass the global auth middleware.
# /p/{slug}/... does its own tier-aware auth in routes/projects.py — listing
# AND per-project endpoints both decide auth based on the project's tier,
# so the global middleware exempts the whole prefix.
PUBLIC_PREFIXES = ("/ui", "/health", "/.auth", "/p")
PUBLIC_EXACT    = {"/", "/api", "/favicon.ico", "/robots.txt"}


_JWKS_CACHE: dict | None = None
_JWKS_FETCHED_AT: float = 0.0
_JWKS_TTL = 600.0


def _fetch_jwks() -> dict:
    global _JWKS_CACHE, _JWKS_FETCHED_AT
    if _JWKS_CACHE and (time.time() - _JWKS_FETCHED_AT) < _JWKS_TTL:
        return _JWKS_CACHE
    with urllib.request.urlopen(JWKS_URL, timeout=5) as r:
        _JWKS_CACHE = _json.loads(r.read())
    _JWKS_FETCHED_AT = time.time()
    return _JWKS_CACHE


def _key_for(kid: str):
    jwks = _fetch_jwks()
    for k in jwks.get("keys", []):
        if k.get("kid") == kid:
            return jwt.algorithms.get_default_algorithms()[k["alg"]].from_jwk(_json.dumps(k))
    # Force a refresh in case keys rotated since last fetch.
    global _JWKS_CACHE, _JWKS_FETCHED_AT
    _JWKS_CACHE = None
    _JWKS_FETCHED_AT = 0
    jwks = _fetch_jwks()
    for k in jwks.get("keys", []):
        if k.get("kid") == kid:
            return jwt.algorithms.get_default_algorithms()[k["alg"]].from_jwk(_json.dumps(k))
    raise HTTPException(401, f"unknown signing key kid={kid}")


def _validate_token(token: str) -> dict:
    try:
        unverified_header = jwt.get_unverified_header(token)
    except jwt.DecodeError as e:
        raise HTTPException(401, f"malformed token: {e}")
    kid = unverified_header.get("kid")
    if not kid:
        raise HTTPException(401, "token missing kid")
    key = _key_for(kid)
    try:
        return jwt.decode(
            token, key=key, algorithms=KC_ALGS,
            audience=KC_AUDIENCES, issuer=KC_ISSUER,
            options={"require": ["exp", "iss"]},
        )
    except jwt.ExpiredSignatureError:
        raise HTTPException(401, "token expired")
    except jwt.InvalidAudienceError:
        raise HTTPException(401, f"invalid audience (need one of: {KC_AUDIENCES})")
    except jwt.InvalidIssuerError:
        raise HTTPException(401, f"invalid issuer (need: {KC_ISSUER})")
    except jwt.InvalidSignatureError:
        raise HTTPException(401, "invalid signature")
    except jwt.PyJWTError as e:
        raise HTTPException(401, f"token rejected: {e}")


def is_public(path: str) -> bool:
    if path in PUBLIC_EXACT:
        return True
    return any(path == p or path.startswith(p + "/") for p in PUBLIC_PREFIXES)


async def auth_middleware(request: Request, call_next):
    """Validate Bearer on every non-public route. Sets `request.state.user`
    to the decoded claims so handlers can read team_id, sub, etc."""
    path = request.url.path
    if request.method == "OPTIONS" or is_public(path):
        return await call_next(request)
    auth = request.headers.get("authorization") or request.headers.get("Authorization")
    if not auth or not auth.lower().startswith("bearer "):
        return _json_response(401, {"detail": "missing Bearer token"})
    token = auth.split(" ", 1)[1].strip()
    try:
        claims = _validate_token(token)
    except HTTPException as e:
        return _json_response(e.status_code, {"detail": e.detail})
    request.state.user = claims
    return await call_next(request)


def _json_response(status: int, body: dict):
    from fastapi.responses import JSONResponse
    return JSONResponse(
        status_code=status, content=body,
        headers={"WWW-Authenticate": 'Bearer realm="project-azure-pgRDF"'} if status == 401 else None,
    )


# ---------- team helpers ----------

def team_id_of(request: Request) -> str | None:
    """Extract team_id from the JWT. Layout expected:
       - `groups` claim (Keycloak Group Mapper) contains `/teams/<team-slug>`
       - OR a custom claim `team_id` set by a Protocol Mapper
    Adapt to whichever shape your realm settles on."""
    user = getattr(request.state, "user", None) or {}
    if "team_id" in user:
        return str(user["team_id"])
    for g in user.get("groups") or []:
        if g.startswith("/teams/"):
            return g.split("/teams/", 1)[1].split("/")[0]
    return None
