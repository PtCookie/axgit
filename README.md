# Axgit

Web frontend for a self-hosted Git server (git-compose stack). Replaces the Cgit + Nginx +
fcgiwrap setup.

- **api/** — Rust (axum + libgit2). Read-only JSON API, Smart HTTP clone, static file serving.
- **web/** — Astro + React + shadcn/ui. Static build.

Deployed as a single container; push is handled by the existing git-server (SSH).

Docs: [Architecture](docs/ARCHITECTURE.md) · [API spec](docs/API.md) · [Decision history](docs/DECISIONS.md)

## Development

```sh
pnpm install && lefthook install
pnpm --filter web dev                        # frontend dev server
cargo run --manifest-path api/Cargo.toml     # backend
```

## Deployment

Build the single-container image (multi-stage: web build → api build → alpine runtime;
docs/DECISIONS.md #22):

```sh
docker build --tag axgit:latest .
```

The build produces an image for the host's own architecture. If the deployment target differs
(e.g. building on Apple Silicon for an amd64 server), pass `--platform`:

```sh
docker build --platform linux/amd64 --tag axgit:latest .
```

Run it against a repository root (`--repo-root`/`AXGIT_REPO_ROOT`, mounted read-only — axgit never
writes to a repository):

```sh
./scripts/make-fixtures.sh   # or point at a real /srv/git
docker run --rm -p 8080:8080 -v "$PWD/fixtures/repos:/srv/git:ro" axgit:latest
```

In the actual git-compose stack, this image replaces the `git-web` service — see
[docs/compose.example.yaml](docs/compose.example.yaml) for an illustrative service definition
(the real change is tracked in the separate git-compose.git repository).

### Configuration

All settings are environment variables (also available as CLI flags — `axgit --help`):

| Variable | Default | Description |
| --- | --- | --- |
| `AXGIT_REPO_ROOT` | `/srv/git` | Directory containing bare repositories (`*.git`) |
| `AXGIT_STATIC_DIR` | _(unset)_ | Astro static build (`web/dist`) to serve at `/`; set to `/app/dist` inside the image |
| `AXGIT_LISTEN` | `0.0.0.0:8080` | Socket address to listen on |
| `AXGIT_CLONE_URL_BASE` | _(unset)_ | Base URL used when displaying clone URLs on the summary page |
| `AXGIT_CACHE_SCAN_TTL` | `60` | Repository scan cache TTL, in seconds |
| `AXGIT_CACHE_RESPONSE_TTL` | `300` | Response cache TTL, in seconds (a safety net — pushes invalidate entries immediately via the HEAD/agefile validator) |
| `AXGIT_CACHE_RESPONSE_MAX_BYTES` | `33554432` (32 MiB) | Response cache capacity, in bytes |
| `AXGIT_REPOSITORY_SORT` | `name` | Default repository index sort order (`name`, `desc`, `owner`, `idle`, `section`, optionally `-`-prefixed); a request's own `?sort=` overrides it |
