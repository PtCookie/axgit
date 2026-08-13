# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

**Axgit** is a web frontend that replaces Cgit in a self-hosted Git server (the
[git-compose](https://git.ptcookie.net/git-compose.git) stack). It's a monorepo managing two components:

- **api/** — Rust backend (axum + git2/libgit2). Reads bare repositories to serve a JSON API,
  serves Smart HTTP clone (`git-upload-pack`), and also serves the Astro static build output.
- **web/** — Astro + React + shadcn/ui frontend. Static build, fetches the API client-side.

**Deployment is a single container**: a multi-stage Dockerfile builds web → builds api → produces
one runtime image. It replaces the existing git-compose stack's `git-web` service with this image;
nginx/fcgiwrap/CGI are not used.

**Continuing across sessions**: `docs/ROADMAP.md` is the single source of truth for what's been
completed so far and what to implement next. When starting a new session, or when asked something
like "what should I do next?", read this file first. When finishing a piece of work, update the
"Done" section and replace the "Next up" section with the next target, in the same commit.

## Core invariants

- **The web app is strictly read-only.** Writes (push, repository creation) happen only via SSH
  to the git-server container. Do not add mutation endpoints to the API. There is no auth/authz
  logic either.
- **Repositories are bare repos under `/srv/git`**, mounted read-only into the container
  (configured via `AXGIT_REPO_ROOT`).
- **Repository metadata is read from each repo's `config` file, `[cgit]` section**
  (`section`, `name`, `owner`, `desc`). The git-server's `git-init` script writes this format, so
  **do not break compatibility**. An `[axgit]` section, if present, takes precedence.
- **Last-activity timestamps come from the agefile** (`info/web/last-modified`), updated by
  git-server's post-receive hook. Falls back to the HEAD commit's authordate if the agefile is
  missing.
- Smart HTTP supports **upload-pack (fetch/clone) only**. `git-receive-pack` requests are rejected
  with 403.

## Commands

```sh
# Initial setup (from the workspace root)
pnpm install                # all JS dependencies (root pnpm-workspace.yaml bundles web as a package)
lefthook install            # register git hooks

# Frontend (web/) — run from root with --filter web
pnpm --filter web dev       # Astro dev server (API proxied via AXGIT_API_URL)
pnpm --filter web build     # static build → web/dist/
pnpm --filter web test      # vitest
pnpm --filter web check     # eslint + prettier check (individually: lint / format)

# Backend (api/)
cargo build --manifest-path api/Cargo.toml
cargo test --manifest-path api/Cargo.toml
cargo clippy --manifest-path api/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path api/Cargo.toml

# Regenerate the OpenAPI spec (docs/openapi.json) — run in any commit that changes the API
AXGIT_UPDATE_OPENAPI=1 cargo test --manifest-path api/Cargo.toml --test openapi_test
pnpm --filter web gen:types # then regenerate web types (openapi-typescript)

# Generate fixture repositories (4 bare repos, fixed dates for reproducibility)
./scripts/make-fixtures.sh

# Local integrated run (api serves web/dist)
cargo run --manifest-path api/Cargo.toml -- --repo-root ./fixtures/repos --static-dir ./web/dist

# Single-binary build: bakes web/dist into the axgit binary (docs/DECISIONS.md #74),
# no --static-dir needed at runtime (AXGIT_STATIC_DIR still overrides it when set)
pnpm --filter web build
cargo build --release --manifest-path api/Cargo.toml --features embed-web

# Container build
docker build --tag axgit:latest .
```

## Architecture

See `docs/ARCHITECTURE.md` for detailed design, `docs/API.md` for the API contract,
`docs/DECISIONS.md` for the decision history, and `docs/ROADMAP.md` for implementation order and
progress.

**When changing the API, update all three of the following in the same commit**:

1. `docs/API.md` — the **normative document** for the contract. Semantic rules the spec can't
   express (limits, ref matching, `null` conditions) live here.
2. `docs/openapi.json` — the spec **generated** from utoipa annotations (do not edit directly, use
   the regeneration command above). Adding an endpoint means updating `#[utoipa::path]`,
   `paths(...)` in `api/src/openapi.rs`, and `EXPECTED_OPERATIONS` in `tests/openapi_test.rs` for
   the tests to pass.
3. `web/src/lib/api/types.ts` — TS types **generated** from openapi.json (do not edit directly).

### Backend notes

- git2's `Repository` isn't `Sync`, so **open it per request** (open cost is low). Don't put a
  `Repository` in a global cache.
- To read string values from `Repository::config()`, take a `.snapshot()` first (live config has
  limited value lookup).
- `api/` follows a `lib.rs` + thin `main.rs` structure: integration tests (`api/tests/`) import
  `build_router` directly and test the router via `tower::ServiceExt::oneshot`, without binding a
  port.
- Heavy operations are handled via git binary exec: archives use `git archive`, Smart HTTP uses
  `git http-backend` (CGI-style spawn) or `git upload-pack --stateless-rpc`. Don't try to
  reimplement everything with libgit2 (Gitea uses the same hybrid pattern).
- **Caching follows a cgit-style approach, improved**: response cache keyed by
  (repo, endpoint, params) with TTL, but instead of a plain TTL, the repo's HEAD/agefile mtime is
  used as a validator so changes are invalidated immediately after a push. Client-side caching
  uses ETag (based on commit sha). Details in `docs/ARCHITECTURE.md#caching`.

### Frontend notes

- Astro is pinned to **static mode**. Don't add an SSR adapter (that's a deployment-shape change
  that needs discussion first).
- Routing is Astro file-based (`web/src/pages/`). Per-repository pages can't be enumerated at
  build time, so `src/pages/[repo]/*.astro` is prerendered once under a reserved placeholder param
  (`__repo__`) and the server maps request path shapes onto the matching shell (`api/src/shell.rs`,
  docs/DECISIONS.md #17). **Adding a route means updating three places together**:
  `web/src/pages/`, `web/src/lib/shell.ts::shellFor`, and `api/src/shell.rs::shell_for`.
- Navigation uses Astro's `<ClientRouter />` (docs/DECISIONS.md #24) — same-origin link clicks swap
  `<body>` client-side instead of a full page load, with a short fade on `<main>`. **Nothing is
  animated by the View Transition API**: every `::view-transition-*(root)` animation is off in
  `global.css` and no element carries a `view-transition-name`, because naming one made Firefox
  scale the page vertically for the whole navigation (docs/DECISIONS.md #32). The fade is a plain
  CSS animation on `<main>` — don't reintroduce `transition:animate`. The header is
  `transition:persist`ed; `<main>` and everything inside it is not, so every data island always
  remounts fresh against the new URL rather than receiving props on a live instance. Anything that
  touches the repository shell (name, `<title>`, tab hrefs) has to re-run on
  `astro:after-swap`, not just on initial load — see `window.__axgit.fillRepoShell` in
  `web/src/layouts/Layout.astro`. `data-astro-rerun` alone is insufficient for that: Astro's
  re-run scripts execute after the transition's DOM update, i.e. after paint.
- Dynamic data is fetched from React islands. They stay `client:only="react"` with a static
  `slot="fallback"` skeleton — don't switch to `client:load` without re-reading DECISIONS.md #17
  (the shell is built under a placeholder param, so a hydrated island would receive it as a prop).
- Code highlighting uses Shiki, lazy-loaded client-side (dynamic import per language).
  READMEs use react-markdown + rehype-sanitize; avatars are generated locally with DiceBear
  (no external requests, DECISIONS.md #11).
- UI components use shadcn/ui, generated into `web/src/components/ui/`. Generated files may be
  modified (vendored approach).

## Conventions

- **Commit messages**: Conventional Commits in English (`feat:`, `fix:`, `build:`, `docs:`,
  `refactor:`, `test:`). Use a scope when useful, e.g. `feat(api):`, `fix(web):`.
- **Rust**: edition 2024, default `cargo fmt` settings, zero clippy warnings (`-D warnings`).
  Errors use `thiserror` (library code) + `anyhow` (bin entry points).
- **TypeScript**: strict mode. API response types come from `web/src/lib/api/types.ts` —
  generated by `pnpm gen:types` from `docs/openapi.json`, so don't edit it directly.
- **Testing**: api builds fixture repos with the git CLI in a tempdir for integration tests
  (`api/tests/`). git CLI invocations block host config with `GIT_CONFIG_GLOBAL=/dev/null`
  `GIT_CONFIG_SYSTEM=/dev/null` and pin `GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` for determinism.
  web uses vitest browser mode (`@vitest/browser-playwright` + `vitest-browser-react`,
  `web/tests/`) + Playwright e2e (`web/e2e/`). Pre-commit hooks are handled by lefthook
  (`lefthook.yml`).
- Docs, code comments, commit messages, and user-facing UI strings are written in English.

## Repository layout

```
axgit/
  api/                # Rust crate (axum + git2)
    src/
      main.rs         # entry point (thin), lib.rs declares modules
      routes.rs       # router setup + Swagger UI mount
      repo/           # repository scanning, metadata, git2 reads (response structs live here too)
      handlers/        # HTTP handlers (1:1 with API.md, #[utoipa::path] annotations)
      openapi.rs      # #[derive(OpenApi)] — the spec's path/tag listing
      cache.rs        # response cache
      smart_http.rs   # git-upload-pack proxy
    tests/            # fixture-repo-based integration tests (+ openapi_test.rs snapshot)
  web/                # Astro + React + shadcn/ui (pnpm workspace package, name: web)
    src/
      pages/          # Astro routes
      layouts/        # shared layouts
      components/     # React islands, ui/ (shadcn)
      lib/
        api/          # fetch client + types.ts (generated file)
        format/       # display formatting utils (dates, etc.)
    tests/            # vitest (browser mode)
    e2e/              # Playwright
  docs/               # ARCHITECTURE.md, API.md, DECISIONS.md, ROADMAP.md, openapi.json
  packaging/          # systemd unit, env-file template, install docs (single-binary release)
  scripts/            # make-fixtures.sh, make-release.sh
  lefthook.yml
  Dockerfile          # web build → api build → runtime (single image)
```
