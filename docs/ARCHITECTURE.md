# Axgit Architecture

## Background

Replaces git-web (Cgit + Nginx + fcgiwrap) in the existing git-compose stack. Requirements
gathered from analyzing Cgit:

- Cgit is a C CGI program linked against git's internal libraries, executed via
  nginx → fcgiwrap → cgit.cgi, with a disk cache (`cache-root`) offsetting CGI execution cost.
- Per-repository metadata is stored in the bare repo's `config`, under the `[cgit]` section, and
  the last-activity timestamp is stored in the agefile (`info/web/last-modified`, updated by the
  post-receive hook). **Axgit reads these same two sources as-is.**
- Clone traffic was handled by `git-http-backend`, not Cgit (upload-pack only, push is SSH-only).
- Screens to replace: index (repo list), summary, log, tree, blob/plain, commit/diff, refs,
  blame, stats (deferred), snapshot, Atom feed.

## Overall layout

```
Browser ──→ Axgit container (single)
              ├─ /              → Astro static build output (web/dist)
              ├─ /api/v1/*      → axum JSON API ──→ git2 / git exec ──→ /srv/git (ro)
              └─ /{repo}.git/*  → Smart HTTP (git upload-pack --stateless-rpc)
git push ──→ SSH 2222 → git-server container (unchanged, existing)
```

- Replaces the existing `git-web` service in git-compose with this image. Mounts the
  `git-repository` volume `:ro`.
- TLS is terminated by a separate reverse proxy container added to the git-compose stack (the
  certbot volume moves there too). Axgit serves HTTP only (DECISIONS.md #10).

## Backend (api/)

### Stack

- **axum** + tokio. See DECISIONS.md for the framework choice rationale.
- **git2** (libgit2 bindings): refs, tree, blob, commit lookups, diff, blame.
- **git binary exec**: `git archive` (snapshots), `git upload-pack --stateless-rpc` (Smart HTTP),
  and an escape hatch for operations confirmed to be a bottleneck on large repos. Not trying to
  solve everything with libgit2.

### Repository scanning

- On startup and periodically, scans `AXGIT_REPO_ROOT` (default `/srv/git`) for `*.git`
  directories (equivalent to cgit's `scan-path`).
- Parses `section`/`name`/`owner`/`desc` from each repo's config, checking `[axgit]` then `[cgit]`
  in order.
- Scan results are kept as an in-memory list, rescanned when the TTL (default 60s) elapses
  (equivalent to cgit's `cache-scanrc-ttl`).

### Caching

A two-layer improvement on cgit's disk TTL cache:

1. **Server response cache** — in-memory LRU (`moka`, `cache.rs`). Key: `(repo, endpoint,
   normalized params)`.
   - Validator: the repo's **HEAD sha + agefile mtime** (`repo/meta.rs::Validator`). On a cache
     hit, the entry is discarded if the validator differs — unlike cgit's plain TTL, this reflects
     a push immediately while still letting a quiet repo reuse its entry until the TTL expires.
   - Looking up the validator itself (reading HEAD + stat'ing the agefile) is cheap, so it's done
     on every request.
   - Responses whose key includes a commit sha (commit detail, diff, sha-based tree/blob) are
     immutable, so they skip validation and go straight to the LRU — this is the only path where
     a cache hit avoids opening the repository at all.
   - Capacity is tracked by body bytes (`AXGIT_CACHE_RESPONSE_MAX_BYTES`), with a 1 MiB cap per
     entry body (so a single huge diff can't evict the whole cache). The TTL
     (`AXGIT_CACHE_RESPONSE_TTL`) acts as a staleness ceiling for changes the validator can't see
     (e.g. manual config edits).
   - Exclusions: the repo list is a scan snapshot, so it keeps using `ScanCache` (a single-value
     TTL) as-is; raw is large binary data and archive is streamed, so neither is cached.
2. **Client-side cache** — sha-bearing URLs get `immutable`; everything else gets `ETag`
   (validator-based) + `no-cache` + 304.

The caching layer lives in a shared handler helper (`handlers/mod.rs::cached_response`), not tower
middleware: whether a response is immutable is only known after ref resolution, params
normalization and content-type differ per endpoint, and error responses must never be cached.

Note: git2's `Repository` isn't `Sync`. The cache stores only serialized responses; `Repository`
is opened within request scope.

### Smart HTTP

- Advertise: prepends a pkt-line service header to the output of
  `git upload-pack --stateless-rpc --advertise-refs {repo}`.
- Data: streams the request body into `git upload-pack --stateless-rpc {repo}`'s stdin, and its
  stdout back as the response. Needs to handle decompressing a gzip'd request body
  (`Content-Encoding: gzip`).
- receive-pack is never exposed in any form (403).

### Test strategy

- `api/tests/` builds fixture bare repos with the git CLI in a tempdir (commits/tags/submodules
  included), then runs integration tests against the axum router. Snapshot/clone are verified via
  actual round-trips using `git clone http://…`.

## Frontend (web/)

- **Astro static** + React islands + shadcn/ui + Tailwind. Astro pages are prerendered shells —
  page chrome (heading, tab nav, `<title>`) is static HTML; only the data regions are client-fetched
  React islands (`client:only="react"`, with a static `slot="fallback"` skeleton).
- Route layout — each is a real file under `web/src/pages/` (✅ implemented, others planned):
  - ✅ `/` repository list (grouped by section, equivalent to cgit's index)
  - ✅ `/{repo}/` summary · ✅ `/{repo}/refs`
  - planned: `/{repo}/log` · `/{repo}/tree/[...path]` · `/{repo}/blob/[...path]` ·
    `/{repo}/commit/{sha}` · `/{repo}/blame/[...path]`
  - ref selection is unified via the `?ref=` URL query
- Per-repository pages can't be enumerated at build time (the repo list is per-deployment), so
  `src/pages/[repo]/*.astro` is prerendered once under a reserved placeholder param and the server
  maps request path *shapes* onto the matching shell (`api/src/shell.rs`, docs/DECISIONS.md #17).
  There is no client-side router; the only client-side URL parsing left is recovering the real
  repository name from `location` for the data islands and for one `is:inline` script that fills in
  the heading/tab links/title before first paint.
- Code highlighting: **Shiki, client-side**, with lazy-loaded language grammars. Highlighting is
  skipped above a size threshold for large files. (Astro's built-in Shiki/markdown is build-time
  only, so it can't be used for runtime-fetched data.)
- README rendering: **react-markdown + remark-gfm + rehype-sanitize** (markdown only; rst/plain
  are shown as `<pre>`).
- Avatars: generated locally with **DiceBear**, seeded from a hash of the committer's email
  address (no external requests). Commit message linkification uses regex-based linkify.
- vitest browser mode (`@vitest/browser-playwright` + `vitest-browser-react`) + Playwright e2e.
  The API client is tested with fetch mocking, components with fixture JSON.

## Build/deploy

- Multi-stage Dockerfile: ① `pnpm --filter web build` in node:alpine → ② `cargo build --release`
  in rust:alpine → ③ alpine runtime: git binary + api binary + web/dist.
  Stage ① COPYs the root `package.json`/`pnpm-workspace.yaml`/`pnpm-lock.yaml` +
  `web/package.json` first so `pnpm install --frozen-lockfile` lands in its own cached layer,
  before copying the rest of the source.
- Runtime image packages needed: `git` (for exec), `ca-certificates`. cgit filter dependencies
  like Python/pygments/groff aren't needed at all.
- Configuration is via environment variables: `AXGIT_REPO_ROOT`, `AXGIT_STATIC_DIR`,
  `AXGIT_LISTEN` (default `0.0.0.0:8080`), `AXGIT_CLONE_URL_BASE` (for displaying clone URLs),
  `AXGIT_CACHE_SCAN_TTL` (repo scan TTL, default 60s), `AXGIT_CACHE_RESPONSE_TTL` (response cache
  TTL, default 300s), `AXGIT_CACHE_RESPONSE_MAX_BYTES` (response cache capacity, default 32 MiB).
- Logs go to stdout/stderr as JSON (`tracing` + `tracing-subscriber`) — collected by the stack's
  fluentd logging driver.
