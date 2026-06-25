"""Per-project registry + lazy asyncpg pool per project database.

Loaded once at FastAPI startup; reads project rows from `pgrdf_meta.projects`
and exposes:

  - `Registry.get(slug)` → ProjectCfg | None
  - `Registry.pool(slug)` → asyncpg.Pool (lazy-create per slug)
  - `Registry.refresh()` → re-read registry from pgrdf_meta

A "project" is one row in `pgrdf_meta.projects`, backed by a database in the
same pg cluster (`pgrdf_<slug>` by convention), pre-loaded with the pgRDF
extension.
"""
from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass

import asyncpg

from .config import PG_DB, PG_HOST, PG_PASS, PG_PORT, PG_USER

log = logging.getLogger("registry")

META_DB = "pgrdf_meta"


@dataclass
class ProjectCfg:
    slug:        str
    tier:        str   # public | protected | secure
    team_id:     str
    mode:        str   # standalone | bridged
    pg_db_name:  str
    title:       str | None = None
    description: str | None = None


class Registry:
    def __init__(self):
        self._projects: dict[str, ProjectCfg] = {}
        self._pools:    dict[str, asyncpg.Pool] = {}
        self._lock = asyncio.Lock()

    async def refresh(self) -> int:
        """Re-read pgrdf_meta.projects. Returns the number of projects loaded.
        Tolerates a missing pgrdf_meta database — leaves the registry empty
        and logs a warning so the API still responds (with no projects)."""
        async with self._lock:
            try:
                conn = await asyncpg.connect(
                    host=PG_HOST, port=PG_PORT, user=PG_USER, password=PG_PASS,
                    database=META_DB,
                )
            except (asyncpg.InvalidCatalogNameError, OSError) as e:
                log.warning("pgrdf_meta unavailable (%s) — registry stays empty", e)
                self._projects.clear()
                return 0
            try:
                rows = await conn.fetch(
                    "SELECT slug, tier, team_id, mode, pg_db_name, title, description "
                    "FROM projects"
                )
            except asyncpg.UndefinedTableError:
                log.warning("pgrdf_meta.projects table does not exist — registry stays empty")
                return 0
            finally:
                await conn.close()
            self._projects = {
                r["slug"]: ProjectCfg(**dict(r)) for r in rows
            }
            return len(self._projects)

    def get(self, slug: str) -> ProjectCfg | None:
        return self._projects.get(slug)

    def all(self) -> list[ProjectCfg]:
        return list(self._projects.values())

    async def pool(self, slug: str) -> asyncpg.Pool:
        """Lazy-create the asyncpg pool for this project's database.
        Caller is responsible for ensuring the slug exists in the registry."""
        if slug in self._pools:
            return self._pools[slug]
        cfg = self.get(slug)
        if cfg is None:
            raise KeyError(f"unknown project: {slug}")
        async with self._lock:
            if slug in self._pools:
                return self._pools[slug]
            pool = await asyncpg.create_pool(
                host=PG_HOST, port=PG_PORT, user=PG_USER, password=PG_PASS,
                database=cfg.pg_db_name,
                min_size=0, max_size=4,
                # 60s idle then closed — projects scale-to-zero gracefully.
                max_inactive_connection_lifetime=60.0,
            )
            self._pools[slug] = pool
            return pool

    async def close(self):
        for pool in self._pools.values():
            await pool.close()
        self._pools.clear()


registry = Registry()
