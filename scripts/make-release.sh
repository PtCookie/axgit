#!/usr/bin/env bash
# Packages a release tarball for the single-binary, non-container deploy path
# (the `embed-web` Cargo feature, docs/DECISIONS.md #74/#75): builds `axgit`
# with `web/dist` baked in, then stages it together with the systemd unit,
# env-file template, and install docs into
#   release/axgit-$VERSION-$TARGET.tar.gz
#   release/SHA256SUMS
#
# Assumes `pnpm install` has already run (Jenkins' "Install dependencies"
# stage does this) — this script does not install JS/Rust dependencies
# itself, only builds and packages.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="$ROOT/release"

# Overridable so the packaging logic (staging/tar/checksums) can be exercised
# on a non-Linux dev machine, e.g. `TARGET=aarch64-apple-darwin
# ./scripts/make-release.sh` — Jenkins always uses the musl default.
TARGET="${TARGET:-x86_64-unknown-linux-musl}"

VERSION="$(awk -F'"' '/^version = /{ print $2; exit }' "$ROOT/api/Cargo.toml")"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [package] version from api/Cargo.toml" >&2
  exit 1
fi

# Jenkins sets TAG_NAME on a tag build (release trigger, docs/DECISIONS.md
# #75). A mismatch between the pushed tag and the crate version should stop
# the release rather than ship a mislabelled tarball.
if [[ -n "${TAG_NAME:-}" && "$TAG_NAME" != "v$VERSION" ]]; then
  echo "error: TAG_NAME '$TAG_NAME' does not match api/Cargo.toml version 'v$VERSION'" >&2
  exit 1
fi

# musl targets need a musl-targeting C compiler: libgit2-sys, liblzma-sys,
# zstd-sys, and bzip2-sys all compile vendored C sources via the `cc` crate,
# which otherwise reaches for the host's glibc-targeting compiler. The
# linker itself is left at rustc's default (self-contained musl crt).
if [[ "$TARGET" == *-musl ]]; then
  if ! command -v musl-gcc >/dev/null 2>&1; then
    echo "error: musl-gcc not found on PATH. Install it first, e.g.:" >&2
    echo "  sudo apt-get install musl-tools" >&2
    exit 1
  fi
  cc_var="CC_$(echo "$TARGET" | tr '-' '_')"
  export "${cc_var}=musl-gcc"
fi

if command -v rustup >/dev/null 2>&1; then
  if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "error: rust target '$TARGET' is not installed. Install it first:" >&2
    echo "  rustup target add $TARGET" >&2
    exit 1
  fi
fi

echo "==> Building frontend (pnpm --filter web build)"
(cd "$ROOT" && pnpm --filter web build)

echo "==> Building axgit $VERSION for $TARGET (--features embed-web)"
(cd "$ROOT" && cargo build --release --locked \
  --manifest-path api/Cargo.toml \
  --features embed-web \
  --target "$TARGET")

BINARY="$ROOT/api/target/$TARGET/release/axgit"
if [[ ! -x "$BINARY" ]]; then
  echo "error: expected binary not found at $BINARY" >&2
  exit 1
fi

PKG_NAME="axgit-$VERSION-$TARGET"
STAGE_DIR="$RELEASE_DIR/$PKG_NAME"

echo "==> Staging $PKG_NAME"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"
cp "$BINARY" "$STAGE_DIR/axgit"
cp "$ROOT/packaging/axgit.service" "$STAGE_DIR/"
cp "$ROOT/packaging/axgit.env.example" "$STAGE_DIR/"
cp "$ROOT/packaging/INSTALL.md" "$STAGE_DIR/"
cp "$ROOT/LICENSE" "$STAGE_DIR/"

TARBALL="$RELEASE_DIR/$PKG_NAME.tar.gz"
echo "==> Writing $TARBALL"
if tar --version 2>/dev/null | grep -q GNU; then
  # Reproducible archive: fixed ownership/order/mtime regardless of build host.
  tar -C "$RELEASE_DIR" \
    --sort=name --mtime='UTC 2026-01-01' --owner=0 --group=0 --numeric-owner \
    -czf "$TARBALL" "$PKG_NAME"
else
  # bsdtar (macOS) has no --sort/--mtime/--numeric-owner; used for local
  # verification only, not the Jenkins release artifact.
  tar -C "$RELEASE_DIR" -czf "$TARBALL" "$PKG_NAME"
fi
rm -rf "$STAGE_DIR"

echo "==> Writing checksums"
SUMS="$RELEASE_DIR/SHA256SUMS"
(
  cd "$RELEASE_DIR"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$(basename "$TARBALL")" >"$SUMS"
  else
    shasum -a 256 "$(basename "$TARBALL")" >"$SUMS"
  fi
)

echo "==> Done"
echo "  $TARBALL"
echo "  $SUMS"
