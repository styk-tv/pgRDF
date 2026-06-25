#!/usr/bin/env python3
"""Provision a new pgRDF project as a database inside the compute pg cluster.

Connects as the cluster superuser (the postgres user from POSTGRES_USER /
POSTGRES_PASSWORD), CREATEs the project database, installs pgRDF + (optional)
AGE + postgres_fdw, then upserts a row in pgrdf_meta.projects.

Usage:
    python3 tools/provision.py <slug> \\
        --tier public|protected|secure \\
        --mode standalone|bridged \\
        --team <team-id> \\
        [--title "..."] \\
        [--description "..."]

For local-dev (compose.local.yml stack), env vars default to compose's
PG_USER/PG_PASS. Override with the usual PG_* env or --pg-host etc.

Idempotent: re-provisioning the same slug updates the row; if the
database already exists, it skips the create + extension steps.
"""
from __future__ import annotations

import argparse
import asyncio
import os
import re
import sys
from pathlib import Path

import asyncpg

REPO_ROOT = Path(__file__).resolve().parent.parent
SQL_DIR   = REPO_ROOT / "sql"

META_DB = "pgrdf_meta"


def slug_to_db_name(slug: str) -> str:
    """`pgrdf_<sanitized>` — lower, underscores only, ≤63 chars."""
    s = re.sub(r"[^a-z0-9_]+", "_", slug.lower())
    s = re.sub(r"_+", "_", s).strip("_")
    s = "pgrdf_" + s
    if len(s) > 63:
        raise SystemExit(f"slug too long: db name '{s}' exceeds 63 chars")
    return s


def is_valid_slug(slug: str) -> bool:
    return bool(re.fullmatch(r"[a-z][a-z0-9_-]{0,40}", slug))


async def ensure_meta_db(superuser_pool):
    """Create the registry database + table if missing."""
    async with superuser_pool.acquire() as conn:
        exists = await conn.fetchval(
            "SELECT 1 FROM pg_database WHERE datname = $1", META_DB
        )
        if not exists:
            print(f"  creating registry database: {META_DB}")
            await conn.execute(f'CREATE DATABASE "{META_DB}"')

    # Connect to meta DB to install the schema.
    cfg = superuser_pool._connect_kwargs  # asyncpg internal — fine for our use
    meta_conn = await asyncpg.connect(**{**cfg, "database": META_DB})
    try:
        sql = (SQL_DIR / "000-meta.sql").read_text()
        # asyncpg can't execute multi-statement via execute() with DDL safely;
        # split on the explicit blank line + ";\n" boundaries.
        await meta_conn.execute(sql)
        print(f"  registry schema ensured in {META_DB}")
    finally:
        await meta_conn.close()


async def create_project_db(superuser_pool, db_name: str) -> bool:
    """Create the database if it doesn't already exist. Returns True if created."""
    async with superuser_pool.acquire() as conn:
        exists = await conn.fetchval(
            "SELECT 1 FROM pg_database WHERE datname = $1", db_name
        )
        if exists:
            print(f"  database {db_name} already exists — skipping CREATE DATABASE")
            return False
        print(f"  creating database {db_name}")
        await conn.execute(f'CREATE DATABASE "{db_name}"')
        return True


async def install_extensions(connect_kwargs, db_name: str):
    """Run sql/001-project-init.sql inside the new database."""
    conn = await asyncpg.connect(**{**connect_kwargs, "database": db_name})
    try:
        sql = (SQL_DIR / "001-project-init.sql").read_text()
        # The init script ends with SELECT pgrdf.version() — that's a select
        # not a DDL, so it returns a row. asyncpg.execute() tolerates that.
        await conn.execute(sql)
        v = await conn.fetchval("SELECT pgrdf.version()")
        print(f"  pgrdf extension installed in {db_name}: version={v}")
    finally:
        await conn.close()


async def upsert_project_row(connect_kwargs, cfg: dict):
    conn = await asyncpg.connect(**{**connect_kwargs, "database": META_DB})
    try:
        await conn.execute(
            """
            INSERT INTO projects (slug, tier, team_id, mode, pg_db_name, title, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (slug) DO UPDATE
              SET tier        = EXCLUDED.tier,
                  team_id     = EXCLUDED.team_id,
                  mode        = EXCLUDED.mode,
                  pg_db_name  = EXCLUDED.pg_db_name,
                  title       = EXCLUDED.title,
                  description = EXCLUDED.description
            """,
            cfg["slug"], cfg["tier"], cfg["team_id"], cfg["mode"],
            cfg["pg_db_name"], cfg.get("title"), cfg.get("description"),
        )
        print(f"  registry row upserted: {cfg['slug']}")
    finally:
        await conn.close()


async def amain(args):
    if not is_valid_slug(args.slug):
        sys.stderr.write(
            f"ERROR: invalid slug {args.slug!r} — must match [a-z][a-z0-9_-]{{0,40}}\n"
        )
        return 2

    db_name = slug_to_db_name(args.slug)

    connect_kwargs = dict(
        host=args.pg_host, port=args.pg_port,
        user=args.pg_user, password=args.pg_pass,
    )

    print(f"provisioning project {args.slug!r}")
    print(f"  pg cluster: {args.pg_host}:{args.pg_port} as {args.pg_user}")
    print(f"  tier: {args.tier}  mode: {args.mode}  team: {args.team}")
    print(f"  db_name: {db_name}")

    # All admin work goes through a tiny pool against the bootstrap DB
    # (typically the cluster's default DB, e.g. 'pgrdf' in compose, 'postgres'
    # in cloud) — asyncpg needs SOME database for the initial connection.
    bootstrap_pool = await asyncpg.create_pool(
        **{**connect_kwargs, "database": args.bootstrap_db},
        min_size=1, max_size=2,
    )
    try:
        await ensure_meta_db(bootstrap_pool)
        created = await create_project_db(bootstrap_pool, db_name)
        if created or args.reinstall_extensions:
            await install_extensions(connect_kwargs, db_name)
        else:
            print(f"  skipping extension install (--reinstall-extensions to force)")

        await upsert_project_row(connect_kwargs, {
            "slug":        args.slug,
            "tier":        args.tier,
            "team_id":     args.team,
            "mode":        args.mode,
            "pg_db_name":  db_name,
            "title":       args.title,
            "description": args.description,
        })
    finally:
        await bootstrap_pool.close()

    print(f"\n✓ project {args.slug!r} ready.")
    print(f"  URL pattern: /p/{args.slug}/info")
    print(f"  URL pattern: /p/{args.slug}/pgrdf-version")
    print(f"  URL pattern: /p/{args.slug}/graphs")
    return 0


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("slug", help="project slug; will become db pgrdf_<slug>")
    p.add_argument("--tier", required=True, choices=["public", "protected", "secure"])
    p.add_argument("--mode", required=True, choices=["standalone", "bridged"])
    p.add_argument("--team", required=True, help="team id (for secure tier scope checks)")
    p.add_argument("--title", default=None)
    p.add_argument("--description", default=None)
    p.add_argument("--reinstall-extensions", action="store_true",
                   help="run sql/001-project-init.sql even if the database already exists")
    p.add_argument("--pg-host",   default=os.environ.get("PG_HOST", "127.0.0.1"))
    p.add_argument("--pg-port",   default=int(os.environ.get("PG_PORT", "5433")), type=int)
    p.add_argument("--pg-user",   default=os.environ.get("PG_USER", "pgrdf"))
    p.add_argument("--pg-pass",   default=os.environ.get("PG_PASS", "pgrdf"))
    p.add_argument("--bootstrap-db", default=os.environ.get("BOOTSTRAP_DB", "pgrdf"),
                   help="the DB to connect to first; needs CREATE DATABASE privilege")
    args = p.parse_args()
    sys.exit(asyncio.run(amain(args)))


if __name__ == "__main__":
    main()
