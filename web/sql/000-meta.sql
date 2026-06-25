-- pgRDF v0.2 — registry database `pgrdf_meta`.
--
-- One row per project. Holds the tier (public/protected/secure), the
-- team_id (for `secure`-tier scope checks), the backing mode
-- (standalone/bridged), and the per-project database name that the
-- FastAPI sidecar should connect to.
--
-- Run once against an empty `pgrdf_meta` database:
--
--   docker exec -i pgrdf-local-pg psql -U pgrdf -d pgrdf_meta < sql/000-meta.sql

CREATE TABLE IF NOT EXISTS projects (
  slug          text PRIMARY KEY,
  -- public  : no auth required, read-only API + UI
  -- protected: any valid Bearer is enough; writes still role-gated
  -- secure  : Bearer must carry team_id matching projects.team_id
  tier          text NOT NULL CHECK (tier IN ('public', 'protected', 'secure')),
  team_id       text NOT NULL,
  -- standalone: project data lives entirely in the compute pg (pg_db_name)
  -- bridged   : project data is FDW-mapped to a managed Flexible Server;
  --             the local DB only carries extension state + foreign tables
  mode          text NOT NULL CHECK (mode IN ('standalone', 'bridged')),
  pg_db_name    text NOT NULL UNIQUE,
  title         text,
  description   text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);

-- A trigger to keep updated_at current.
CREATE OR REPLACE FUNCTION projects_touch_updated_at() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS projects_touch_updated_at_trg ON projects;
CREATE TRIGGER projects_touch_updated_at_trg
  BEFORE UPDATE ON projects
  FOR EACH ROW EXECUTE FUNCTION projects_touch_updated_at();
