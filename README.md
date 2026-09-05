# Axgit

Web frontend for a self-hosted Git server (git-compose stack). Replaces the Cgit + Nginx +
fcgiwrap setup.

- **api/** — Rust (axum + libgit2). Read-only JSON API, Smart HTTP clone, static file serving.
- **web/** — Astro + React + shadcn/ui. Static build.

Deployed either as a single container (see [Deployment](#deployment)) or as a single binary (see
[Single-binary build](#single-binary-build)); push is handled by the existing git-server (SSH).
The API is self-documenting at `/swagger-ui` (raw spec at `/api/v1/openapi.json`).

Docs: [Architecture](docs/ARCHITECTURE.md) · [API spec](docs/API.md) ·
[Decisions](docs/DECISIONS.md) · [Roadmap](docs/ROADMAP.md)

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

Changing the API means updating `docs/API.md`, `openapi.json`, and
`web/src/lib/api/types.ts` together, in the same commit — see `openapi.json`'s regeneration
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
[compose.yaml](compose.yaml) for an illustrative service definition
(the real change is tracked in the separate git-compose.git repository).

### Single-binary build

The default build bakes `web/dist` directly into the `axgit` executable (docs/DECISIONS.md #74,
#88) — the resulting binary plus a `git` binary on `PATH` is a complete deployment, no
`AXGIT_STATIC_DIR`/directory needed:

```sh
pnpm --filter web build
cargo build --release --manifest-path api/Cargo.toml
```

`AXGIT_STATIC_DIR` still overrides the embedded copy at runtime when set. A pure-API build with no
bundled frontend at all is `cargo build --release --manifest-path api/Cargo.toml --features
api-only` — that one does need `AXGIT_STATIC_DIR` (or nothing is served at `/`) and has no
compile-time dependency on `web/dist`. To package the default build into an installable release
tarball (binary + systemd unit + env-file template + install docs, docs/DECISIONS.md #75):

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
per target — the binary plus `LICENSE`, nothing else — and a single `release/SHA256SUMS` covering
all of them. See [systemd install](#systemd-install) for what to do with one.

### systemd install

The alternative to the container image, for a bare-metal or VM install. A release tarball is a
complete deployment on its own: the binary has the frontend baked in (docs/DECISIONS.md #74, #88),
so nothing else from the build needs to be copied. A `git` binary must still be on `PATH` — it's
used for archive downloads and Smart HTTP clone/fetch (docs/ARCHITECTURE.md's hybrid libgit2+exec
policy).

Releases ship one tarball per CPU architecture (docs/DECISIONS.md #79), and the filename is the
only place that's recorded — so before installing, check `uname -m` (`x86_64` or `aarch64`; on
Linux, `aarch64` is arm64) against the target triple in the `axgit-<version>-<target>.tar.gz` name.

**1. Install the binary**

```sh
sudo install -m 755 axgit /usr/local/bin/axgit
```

**2. Choose the service user**

Run the service as **the user that owns the repository root** (`AXGIT_REPO_ROOT`), not as a
dedicated unprivileged user with no relation to the repositories. This matters because axgit reads
bare repositories directly with libgit2 and shells out to `git`, both of which refuse to operate on
a directory owned by a different uid ("dubious ownership") unless a `safe.directory` config entry is
added. When the process uid already matches the repositories' owning uid, that check passes outright
and no `safe.directory` entry is needed anywhere — this was verified by reading libgit2's
ownership-check source directly (docs/DECISIONS.md #74). If your repositories are owned by e.g. a
`git` user, run axgit as that same user.

**3. Install the unit**

Write this to `/etc/systemd/system/axgit.service`, with `User=`/`Group=` set to the account chosen
above. The comments are the rationale for each non-obvious directive:

```ini
[Unit]
Description=Axgit — read-only web frontend for bare Git repositories
Documentation=https://git.ptcookie.net/axgit.git
After=network.target

[Service]
Type=exec
ExecStart=/usr/local/bin/axgit
Restart=on-failure
RestartSec=2

# The user that OWNS the repository root — see step 2. libgit2's/git's
# dubious-ownership check passes outright when the process uid matches the
# repositories' owning uid, so no [safe.directory] entry is needed anywhere
# (docs/DECISIONS.md #74/#75).
User=git
Group=git

# One of the two configuration surfaces — every AXGIT_* variable from
# api/src/config/ (see README.md's configuration table). The leading '-'
# makes the file optional: axgit's own defaults apply if it's absent.
#
# The other is /etc/axgit/axgit.toml, which axgit reads by itself when it
# exists (docs/DECISIONS.md #87) — no unit change needed. A variable set
# here wins over the same setting in that file, so pick one of the two per
# setting rather than splitting one across both.
EnvironmentFile=-/etc/axgit/axgit.env

# Sandboxing. The app never writes anywhere (the read-only invariant) but
# does fork/exec `git` for archive/upload-pack, so process spawning and
# network access are left open while filesystem/kernel surface is locked
# down.
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
# read-only, not "yes": libgit2 still reads $HOME/.gitconfig for the
# ownership-check config stack (docs/DECISIONS.md #74).
ProtectHome=read-only
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectKernelLogs=true
ProtectControlGroups=true
RestrictNamespaces=true
RestrictSUIDSGID=true
LockPersonality=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM

# Uncomment to bind a privileged port (e.g. :80) directly instead of sitting
# behind a reverse proxy (the default deployment shape, docs/DECISIONS.md
# #10):
# AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

**4. Configure it**

Both surfaces below are documented in [Configuration](#configuration); axgit runs on sane defaults
for anything left unset, but `AXGIT_REPO_ROOT` is required in practice (the `/srv/git` default only
makes sense inside the container image).

```sh
sudo mkdir -p /etc/axgit
sudo "$EDITOR" /etc/axgit/axgit.env    # AXGIT_REPO_ROOT=…, AXGIT_CLONE_URL_BASE=…, one per line
```

To keep the configuration in one commentable file instead of a list of environment variables, write
`/etc/axgit/axgit.toml` — axgit reads that path on its own, with no unit change needed
(docs/DECISIONS.md #87). [axgit.toml](axgit.toml) in this repository is a commented example covering
every key — copy it and replace its (development-shaped) active values. An `AXGIT_*` variable wins
over the same setting in the TOML file, so set any given value in one file or the other, not both.

**5. Start it**

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now axgit
sudo systemctl status axgit
```

axgit listens on `AXGIT_LISTEN` (default `0.0.0.0:8080`) and has no built-in TLS — put a reverse
proxy in front of it for a public deployment (docs/DECISIONS.md #10), the same assumption
`compose.yaml` makes for the container deployment.

**6. Verify**

```sh
curl -s http://127.0.0.1:8080/api/v1/repos
```

should return a JSON array (empty if `AXGIT_REPO_ROOT` has no repositories yet), and
`http://127.0.0.1:8080/` should serve the web UI — with no `AXGIT_STATIC_DIR` set, this is the
frontend baked into the binary.

**Upgrading** is just a binary swap; repository data is never touched by axgit itself:

```sh
sudo systemctl stop axgit
sudo install -m 755 axgit /usr/local/bin/axgit   # from a newer release tarball
sudo systemctl start axgit
```

### Configuration

Every setting is reachable three ways — a CLI flag (`axgit --help`), an `AXGIT_*` environment
variable, or a key in a TOML config file — and they layer in that order:

**CLI flag > environment variable > config file > built-in default.**

The config file is optional. `--config <path>` / `AXGIT_CONFIG` names one explicitly (a path that
doesn't exist is a startup error); with neither set, `/etc/axgit/axgit.toml` is read if it happens
to be there, and startup is silent if it isn't. Keys axgit doesn't recognize are logged as a
warning and ignored, so a `cgitrc` carried over from cgit — with its `scan-path`, `enable-*`,
`snapshots` keys — still boots.

```toml
# /etc/axgit/axgit.toml
repo-root       = "/srv/git"
listen          = "0.0.0.0:8080"
clone-url-base  = "https://git.example.com"
repository-sort = "-idle"

[site]
root-title = "PtCookie Git"
root-desc  = "self-hosted git"
logo       = "/srv/git/logo.svg"

[cache]
response-ttl = 300
```

Section names are organizational only; keys keep cgit's own `cgitrc` spelling wherever cgit has
one, so an existing value can be pasted straight across.

The container image sets no `ENV` of its own (docs/DECISIONS.md #88 — the frontend is baked into
the binary, so there's nothing left for `AXGIT_STATIC_DIR` to point at), so every setting below,
including it, is the config file's to set, as long as the same setting isn't also passed as an
environment variable.

| Variable | Config file key | Default | Description |
| --- | --- | --- | --- |
| `AXGIT_CONFIG` | _(n/a)_ | `/etc/axgit/axgit.toml` when it exists | TOML config file holding any of the settings below |
| `AXGIT_REPO_ROOT` | `repo-root` | `/srv/git` | Directory containing bare repositories (`*.git`) |
| `AXGIT_STATIC_DIR` | `static-dir` | _(unset)_ | Astro static build (`web/dist`) to serve at `/`, overriding the binary's own baked-in copy. Only way to serve a frontend at all with the `api-only` Cargo feature |
| `AXGIT_LISTEN` | `listen` | `0.0.0.0:8080` | Socket address to listen on |
| `AXGIT_CLONE_URL_BASE` | `clone-url-base` | _(unset)_ | Base URL used when displaying clone URLs on the summary page |
| `AXGIT_CACHE_SCAN_TTL` | `cache.scan-ttl` | `60` | Repository scan cache TTL, in seconds |
| `AXGIT_CACHE_RESPONSE_TTL` | `cache.response-ttl` | `300` | Response cache TTL, in seconds (a safety net — pushes invalidate entries immediately via the HEAD/agefile validator) |
| `AXGIT_CACHE_RESPONSE_MAX_BYTES` | `cache.response-max-bytes` | `33554432` (32 MiB) | Response cache capacity, in bytes |
| `AXGIT_REPOSITORY_SORT` | `repository-sort` | `name` | Default repository index sort order (`name`, `desc`, `owner`, `idle`, `section`, optionally `-`-prefixed); a request's own `?sort=` overrides it |
| `AXGIT_ROOT_TITLE` | `site.root-title` | _(unset)_ | Site-wide title, shown as the header brand and falling back to "Axgit" |
| `AXGIT_ROOT_DESC` | `site.root-desc` | _(unset)_ | Site-wide description, shown on the index page |
| `AXGIT_ROOT_README` | `site.root-readme` | _(unset)_ | Path to a markdown/reStructuredText/plain-text file rendered on the index page |
| `AXGIT_LOGO` | `site.logo` | _(unset)_ | Site logo, shown beside the header brand. An `http(s)://` URL (used verbatim) or a filesystem path axgit serves itself at `GET /api/v1/site/logo`; unset shows axgit's own mark (`/favicon.svg`), the same one the tab icon defaults to |
| `AXGIT_LOGO_LINK` | `site.logo-link` | _(unset)_ | Where the logo links to; an `http(s)://` URL or a root-relative path, falling back to `/` |
| `AXGIT_FAVICON` | `site.favicon` | _(unset)_ | Site favicon, replacing axgit's own default. Same URL-or-path rule as `AXGIT_LOGO`, served at `GET /api/v1/site/favicon` |

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
