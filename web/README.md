# project-azure-pgRDF

A scale-to-zero pgRDF deployment, packaged as a Marketplace SaaS offer (Microsoft ISV Success partnership).

> **Status: v0.1 scaffolding.** Folder layout + UI files unpacked + FastAPI shell + auth copied from asset-gateway. Not yet runnable end-to-end. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the picture.

## Topology

One project = one isolated tenant:

```
                            Marketplace subscribe → CLI provisioner
                                          │
                  ┌───────────────────────┴─────────────────────────┐
                  │                                                 │
        ┌─────────▼──────────┐                       ┌──────────────▼─────────────┐
        │  ACA Container App │                       │ Azure Files (Premium SMB)  │
        │  (scale-to-zero)   │ ◀── private 5432 ──▶  │  CMK = team's KEK in KV    │
        │  postgres + pgRDF  │                       │  /var/lib/postgresql/data  │
        │  + AGE + FDW       │                       └────────────────────────────┘
        │  + FastAPI sidecar │
        └─────────┬──────────┘
                  │ ingress (public HTTPS)
                  ▼
       https://<project>.nicedune-XXX.westeurope.azurecontainerapps.io/
                  │
       ┌──────────┴──────────────────┐
       │ /ui/  → React Console SPA   │  (browser-side React 18 + Babel-standalone)
       │ /api/v1/* → REST API        │  (Bearer JWT via id.tech.games/realms/techgames)
       │ /health → public            │
       └─────────────────────────────┘
```

- **TCP (5432) is never publicly exposed.** All access flows through the FastAPI sidecar.
- **Bearer JWT** is the only auth perimeter. Cookie-based Easy Auth is OFF.
- **Cryptographic team isolation:** each team in Keycloak has its own KEK in Key Vault; each project's Azure Files share is encrypted with that team's KEK. Different teams' projects cannot mount each other's volumes even with the same managed identity.

## Local-dev (planned, not yet wired)

```bash
docker-compose up        # pg-with-pgRDF + fastapi + fake-keycloak
open http://localhost:8000/ui/
```

## Deploy a new project (planned)

```bash
./infra/new-project.sh <project-slug> <team-id>
# Provisions ACA app + Azure Files share + KEK ref. Prints the project URL.
```

## What's in the box

| Path | Purpose |
|---|---|
| [`LLD_ Ontology-Driven Architecture on Azure.md`](LLD_ Ontology-Driven Architecture on Azure.md) | Low-Level Design (the picture, the FDW model, the controller procs) |
| [`app/`](app/) | FastAPI sidecar — auth middleware, routes, static SPA serving |
| [`app/static/ui/`](app/static/ui/) | The pgRDF Console (React 18 + Babel-standalone, no build step) |
| [`sql/`](sql/) | One-time init SQL: extensions, FDW, ontology tables, controller procs |
| [`infra/`](infra/) | Bicep templates + provisioning scripts |
| [`tools/`](tools/) | `build.sh`, `deploy.sh`, `seed.sh` |
| [`tests/`](tests/) | smoke + cold-start tests |
| [`docs/`](docs/) | architecture, marketplace offer draft, onboarding |
