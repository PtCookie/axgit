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
