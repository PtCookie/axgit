# syntax=docker/dockerfile:1
#
# Multi-stage build: web (Astro static build) -> api (Rust binary) -> runtime.
# See docs/ARCHITECTURE.md#Build/deploy and docs/DECISIONS.md #22 for the design rationale.
#
# Base images are pinned to their minor version for build reproducibility (docs/DECISIONS.md
# #22) — bump these deliberately in their own commit, not implicitly via a floating tag.
ARG NODE_IMAGE=node:24.11-alpine3.22
ARG RUST_IMAGE=rust:1.97-alpine3.22
ARG RUNTIME_IMAGE=alpine:3.22

# ---------------------------------------------------------------------------
# Stage: web — pnpm --filter web build -> web/dist
# ---------------------------------------------------------------------------
FROM ${NODE_IMAGE} AS web

# The root package.json pins packageManager to an exact pnpm version; corepack
# reads that field instead of fetching whatever pnpm is latest.
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable

WORKDIR /app

# Copy manifests only first so `pnpm install --frozen-lockfile` lands in its
# own cached layer, before the rest of the source invalidates it.
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY web/package.json web/package.json
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile

COPY web/ web/
COPY docs/openapi.json docs/openapi.json
RUN pnpm --filter web build

# ---------------------------------------------------------------------------
# Stage: api — cargo build --release -> /axgit
# ---------------------------------------------------------------------------
FROM ${RUST_IMAGE} AS api

# musl-dev: libgit2-sys's build.rs falls back to building vendored libgit2
# (via `cc`) since alpine has no system libgit2 to find via pkg-config — this
# also statically links libgit2/zlib into the final musl binary. The same `cc`
# toolchain also builds liblzma-sys's and zstd-sys's vendored C sources for
# the archive endpoint's xz/zstd encoders (docs/DECISIONS.md #54) — no
# separate package is needed for those.
RUN apk add --no-cache musl-dev

WORKDIR /app/api

# Manifests only first, so dependency fetch/compilation is cached separately
# from application code changes.
COPY api/Cargo.toml api/Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo fetch --locked

COPY api/ .
# Cache mounts aren't persisted into the image layer, so the binary must be
# copied out to a normal path within the same RUN that builds it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/api/target \
    touch src/main.rs && \
    cargo build --release --locked --offline && \
    cp target/release/axgit /axgit

# ---------------------------------------------------------------------------
# Stage: runtime — alpine + git binary + axgit binary + web/dist
# ---------------------------------------------------------------------------
FROM ${RUNTIME_IMAGE} AS runtime

LABEL org.opencontainers.image.title="axgit" \
      org.opencontainers.image.description="Read-only web frontend for bare Git repositories" \
      org.opencontainers.image.source="https://git.ptcookie.net/axgit.git" \
      org.opencontainers.image.licenses="MIT"

# `git` is required for archive/upload-pack exec (docs/ARCHITECTURE.md's
# hybrid libgit2+exec policy). ca-certificates covers TLS trust roots if a
# future outbound call needs it; tzdata is intentionally omitted — jiff only
# uses UTC/fixed offsets recorded in git commits, never the system tzdb.
RUN apk add --no-cache git ca-certificates

# `/srv/git` is a read-only mount owned by the git-server container (a
# different uid), which trips git's/libgit2's "dubious ownership" ownership
# check. Trusting every directory here is scoped to this single-purpose,
# read-only container (docs/DECISIONS.md #22) — this config applies to both
# the `git` exec calls and libgit2, which both read the system gitconfig.
RUN printf '[safe]\n\tdirectory = *\n' > /etc/gitconfig

# The app never needs write access anywhere (CLAUDE.md's read-only
# invariant), so it runs as a dedicated, home-less, login-less user.
RUN adduser -D -H -u 10001 axgit

COPY --from=api /axgit /usr/local/bin/axgit
COPY --from=web /app/web/dist /app/dist

ENV AXGIT_REPO_ROOT=/srv/git \
    AXGIT_STATIC_DIR=/app/dist \
    AXGIT_LISTEN=0.0.0.0:8080

EXPOSE 8080
USER axgit

# No dedicated health endpoint exists; the repo list is cheap (backed by
# ScanCache) and always available once the server is up.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/api/v1/repos > /dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/axgit"]
