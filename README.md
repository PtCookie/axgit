# Axgit

Web frontend for a self-hosted Git server (git-compose stack). Replaces the Cgit + Nginx +
fcgiwrap setup.

- **api/** — Rust (axum + libgit2). Read-only JSON API, Smart HTTP clone, static file serving.
- **web/** — Astro + React + shadcn/ui. Static build.

Deployed either as a single container (see [Deployment](#deployment)) or as a single binary (see
[Single-binary build](#single-binary-build)); push is handled by the existing git-server (SSH).
The API is self-documenting at `/swagger-ui` (raw spec at `/api/v1/openapi.json`).

Docs: [Architecture](docs/ARCHITECTURE.md) · [API spec](docs/API.md) ·
[Decision history](docs/DECISIONS.md) · [Roadmap](docs/ROADMAP.md)

## Features

Full parity with cgit's own feature set (see `docs/ROADMAP.md` for the audit and the deliberate
differences), plus a few things cgit doesn't have:

- **Repository index** — section grouping, sortable columns (`?sort=`,
  `AXGIT_REPOSITORY_SORT`), a client-side name/description/owner filter (`?q=`), and an optional
  site-wide title/description/readme/logo/favicon.
- **Repository pages** — a summary page (rendered README + metadata sidebar) and a refs page
  (local/remote branches, tags with their own detail page, per-tag archive downloads, and
  "Compare" entry points).
- **Log and commits** — a branch/merge graph column, ref badges, a path filter with rename
  following, expandable commit messages, changed files/lines columns, and commit detail (diff,
  git notes, archive links).
- **Diffs** — arbitrary two-revision comparisons, unified/side-by-side/stat-only views,
  intra-line highlighting, plus raw unified diff (`/rawdiff`) and `git am`-able patch
  (`/patch`) output.
- **Tree, blob, and blame** — Shiki syntax highlighting, symlink targets, submodule
  (gitlink) links, a hex dump for binary files, rename-tracking blame, and a by-object-id page.
- **Search and stats** — in-repository search across six modes (`content`, `path`, `message`,
  `author`, `committer`, `range`), and commit-activity stats (12 buckets, period/path filters, an
  author table with an `Others (N)` rollup).
- **Serving** — Smart HTTP clone/fetch (upload-pack only — push is always `403`), five archive
  formats (`tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst`, `zip`), an Atom feed (`ref`/`path`/`all`)
  with `<head>` auto-discovery, cgit URL compatibility redirects, and a System/Light/Dark theme.

**The app is strictly read-only** — no auth, no write endpoints; every mutation happens over SSH
against the git-server directly.

## Development

```sh
pnpm install                                       # lefthook hooks register via postinstall
./scripts/make-fixtures.sh                         # 4 fixture bare repos, fixed dates
cargo run --manifest-path api/Cargo.toml -- --repo-root ./fixtures/repos   # backend
pnpm --filter web dev                               # frontend dev server (/api proxied via AXGIT_API_URL)
```

Tests and checks:

```sh
cargo test --manifest-path api/Cargo.toml
cargo clippy --manifest-path api/Cargo.toml --all-targets -- -D warnings
pnpm --filter web test          # vitest
pnpm --filter web test:e2e      # Playwright e2e
pnpm --filter web check         # eslint + prettier check
```

Changing the API means updating `docs/API.md`, `docs/openapi.json`, and
`web/src/lib/api/types.ts` together, in the same commit — see `docs/openapi.json`'s regeneration
command and `pnpm --filter web gen:types` (both in `AGENTS.md`/`CLAUDE.md`). See
[web/README.md](web/README.md) for frontend-specific commands and layout.

## Deployment

Build the single-container image (multi-stage: web build → api build → alpine runtime;
docs/DECISIONS.md #22). The build is defined in `Containerfile`; `Dockerfile` is a committed
symlink to it, so `docker build` needs no extra flag while `podman`/`buildah` (which look for
`Containerfile` first) also work unchanged:

```sh
docker build --tag axgit:latest .
```

```sh
buildah build --tag axgit:latest --file Containerfile .
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

### Single-binary build

For a bare-metal/systemd install instead of the container, `web/dist` can be baked directly into
the `axgit` executable (docs/DECISIONS.md #74) — the resulting binary plus a `git` binary on
`PATH` is a complete deployment, no `AXGIT_STATIC_DIR`/directory needed:

```sh
pnpm --filter web build
cargo build --release --manifest-path api/Cargo.toml --features embed-web
```

`AXGIT_STATIC_DIR` still overrides the embedded copy at runtime when set. To package this into an
installable release tarball (binary + systemd unit + env-file template + install docs,
docs/DECISIONS.md #75):

```sh
./scripts/make-release.sh
```

Defaults to `TARGETS="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"` (space-separated;
override via the `TARGETS` env var, or set `TARGET` for a single-target alias, e.g.
`TARGET=aarch64-apple-darwin` for local verification on macOS). Each `*-musl` target needs
`rustup target add <target>` plus a matching C compiler on `PATH`: `musl-tools` (`musl-gcc`) for a
native x86_64 build, and for the aarch64 leg either musl.cc/Homebrew's `musl-cross`
(`aarch64-linux-musl-gcc`) or `messense/macos-cross-toolchains` (`aarch64-unknown-linux-musl-gcc`)
— both cross conventions are also usable on macOS, so both musl legs can be built and verified on
a Mac dev machine (docs/DECISIONS.md #79). Produces one `release/axgit-<version>-<target>.tar.gz`
per target and a single `release/SHA256SUMS` covering all of them — see the tarball's own
`INSTALL.md` (also at [packaging/INSTALL.md](packaging/INSTALL.md)) for the systemd install steps.

### Configuration

All settings are environment variables (also available as CLI flags — `axgit --help`):

| Variable | Default | Description |
| --- | --- | --- |
| `AXGIT_REPO_ROOT` | `/srv/git` | Directory containing bare repositories (`*.git`) |
| `AXGIT_STATIC_DIR` | _(unset)_ | Astro static build (`web/dist`) to serve at `/`; set to `/app/dist` inside the image. Overrides an `embed-web`-baked build when both are present |
| `AXGIT_LISTEN` | `0.0.0.0:8080` | Socket address to listen on |
| `AXGIT_CLONE_URL_BASE` | _(unset)_ | Base URL used when displaying clone URLs on the summary page |
| `AXGIT_CACHE_SCAN_TTL` | `60` | Repository scan cache TTL, in seconds |
| `AXGIT_CACHE_RESPONSE_TTL` | `300` | Response cache TTL, in seconds (a safety net — pushes invalidate entries immediately via the HEAD/agefile validator) |
| `AXGIT_CACHE_RESPONSE_MAX_BYTES` | `33554432` (32 MiB) | Response cache capacity, in bytes |
| `AXGIT_REPOSITORY_SORT` | `name` | Default repository index sort order (`name`, `desc`, `owner`, `idle`, `section`, optionally `-`-prefixed); a request's own `?sort=` overrides it |
| `AXGIT_ROOT_TITLE` | _(unset)_ | Site-wide title, shown as the header brand and falling back to "Axgit" |
| `AXGIT_ROOT_DESC` | _(unset)_ | Site-wide description, shown on the index page |
| `AXGIT_ROOT_README` | _(unset)_ | Path to a markdown/reStructuredText/plain-text file rendered on the index page |
| `AXGIT_LOGO` | _(unset)_ | Site logo, shown beside the header brand. An `http(s)://` URL (used verbatim) or a filesystem path axgit serves itself at `GET /api/v1/site/logo` |
| `AXGIT_LOGO_LINK` | _(unset)_ | Where the logo links to; an `http(s)://` URL or a root-relative path, falling back to `/` |
| `AXGIT_FAVICON` | _(unset)_ | Site favicon, replacing axgit's own default. Same URL-or-path rule as `AXGIT_LOGO`, served at `GET /api/v1/site/favicon` |

### Repository configuration

Per-repository settings are read from each bare repo's own `config` file, `[cgit]` section (the
git-server's `git-init` script writes this format). An `[axgit]` section, if present, takes
precedence key-by-key.

| Key | Description |
| --- | --- |
| `section` | Group heading on the repository index |
| `owner` | Shown on the index and summary page |
| `desc` | Shown on the index and summary page |
| `homepage` | External homepage link (only `http://`/`https://` are honoured; anything else is treated as unset) |
| `defbranch` | Default branch for ref-less requests, when it names an existing local branch |
| `hide` | Boolean — drops the repository from the index, but it stays reachable by direct path |
| `ignore` | Boolean — the repository is unreachable entirely (every per-repo route and Smart HTTP 404) |
| `module-link`, `<path>.module-link` | Submodule link template (`%s` substituted with the gitlink's path and sha) |

`hide`/`ignore` accept the same boolean spellings git itself does (`true`/`false`, `yes`/`no`,
`on`/`off`, `1`/`0`). A submodule with no `module-link` key falls back to `.gitmodules`'s own
`url`. Last-activity timestamps come from the agefile (`info/web/last-modified`, updated by the
git-server's post-receive hook), falling back to the HEAD commit's authordate when it's missing.
